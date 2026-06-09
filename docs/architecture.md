# Architecture

## Crate Dependency Graph

```
fire_red_tracker          fire_red_aggregator
     │                          │
     ├── fire_red_states ────────┤  (shared protocol types)
     ├── fire_red_loop           ├── fire_red_states
     │   ├── fire_red_memory     ├── fire_red_database
     │   ├── fire_red_scanner    └── fire_red_party_monitor
     │   ├── fire_red_pokemon_data
     │   ├── fire_red_party_monitor
     │   ├── fire_red_box_monitor
     │   ├── fire_red_badge
     │   ├── fire_red_text
     │   ├── fire_red_map_data
     │   └── fire_red_retroarch_interfacing
     ├── fire_red_database
     ├── fire_red_image_data
     └── fire_red_trainer_data

Support crates (no external deps):
  fire_red_memory          ── sliding-window UDP snapshots of EWRAM/IWRAM
  fire_red_retroarch_interfacing ── UDP socket helpers
  fire_red_scanner         ── ROM header scan
  fire_red_pokemon_data    ── ROM wild-encounter tables + FFI
  fire_red_badge           ── Badge flags + FFI
  fire_red_text            ── GBA character encoding
  fire_red_image_data      ── LZ77 sprite decompression
  fire_red_map_data        ── Gym progression table
  fire_red_rom_buffer      ── Global ROM byte slice
  fire_red_pokemon_name_buffer ── Cached Pokémon name list
  fire_red_trainer_data    ── Trainer name / ID helpers
  fire_red_party_monitor   ── Party Pokemon struct
  fire_red_get_values      ── Misc EWRAM value helpers
```

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  RetroArch  (UDP :55355)                                            │
│  responds to READ_CORE_MEMORY commands                              │
└──────────┬──────────────────────────────────────────────────────────┘
           │ UDP (127.0.0.1)
           ▼
┌──────────────────────────────────────────────────────┐
│  fire_red_tracker                                     │
│                                                       │
│  ┌──────────────┐    ┌───────────────────────────┐   │
│  │ Memory thread│───▶│ EWRAM/IWRAM snapshots      │   │
│  │  (100 ms)    │    │ ArcSwap<Vec<u8>>           │   │
│  └──────────────┘    └──────────────┬────────────┘   │
│                                     │ read by all     │
│  ┌────────────────────────────────┐ │                 │
│  │ Game-polling thread  (100 ms)  │◀┘                 │
│  │  • map state                   │                   │
│  │  • party (Arc<Mutex<Vec<..>>>  │                   │
│  │  • wild encounters             │                   │
│  │  • EncounterTracker::tick()    │                   │
│  │  • check_for_dead/new_pokemon  │                   │
│  └────────────┬───────────────────┘                   │
│               │ Arc-shared                            │
│  ┌────────────▼──────────────────────┐               │
│  │ Network thread (Connected mode)   │               │
│  │  reconnect loop → handle_client() │               │
│  │  ┌────────────────────────────┐   │               │
│  │  │ Reader thread              │   │               │
│  │  │  • receives ClientMessage  │   │               │
│  │  │  • EndRun / NewRun → DB    │   │               │
│  │  │  • RequestTextures → build │   │               │
│  │  └────────────────────────────┘   │               │
│  │  Writer loop (100 ms)             │               │
│  │  • sends ServerMessage::State     │               │
│  └────────────┬──────────────────────┘               │
└───────────────┼──────────────────────────────────────┘
                │ TCP (bincode, length-prefixed)
                ▼
┌──────────────────────────────────────────────────────┐
│  fire_red_aggregator                                  │
│                                                       │
│  ┌──────────────────────────────┐                    │
│  │ TCP listener thread          │                    │
│  │  for each accepted stream:   │                    │
│  │   reuse/create MonitorSlot   │                    │
│  │   spawn tracker thread       │                    │
│  └──────────────────────────────┘                    │
│                                                       │
│  SharedSlots = Arc<Mutex<Vec<Arc<MonitorSlot>>>>      │
│                                                       │
│  ┌─────────────────────────────────────────────┐     │
│  │  Per-tracker thread (one per connection)    │     │
│  │  handle_tracker_connection()                │     │
│  │  ┌─────────────────────────────────────┐   │     │
│  │  │ Writer thread  (50 ms)              │   │     │
│  │  │  • drains command_queue             │   │     │
│  │  │    (EndRun / NewRun → tracker)      │   │     │
│  │  │  • drains texture_request_queue     │   │     │
│  │  └─────────────────────────────────────┘   │     │
│  │  Reader loop                                │     │
│  │  • State → MonitorSlot.state               │     │
│  │  • Textures → pending_textures / cache     │     │
│  │  • RunChanged → run_changed flag           │     │
│  └─────────────────────────────────────────────┘     │
│                                                       │
│  ┌─────────────────┐   ┌────────────────────────┐   │
│  │  egui GUI       │   │  axum WebSocket server  │   │
│  │  (GUI mode)     │   │  (headless / OBS mode)  │   │
│  │  reads slots    │   │  BroadcastLoop (100ms)  │   │
│  │  each frame     │   │  → JSON to all clients  │   │
│  └─────────────────┘   └────────────┬───────────┘   │
└───────────────────────────────────── ┼───────────────┘
                                       │ WebSocket
                                       ▼
                              Browser / OBS overlay
                         (overlay.html / focused.html)
```

## Modes

### Tracker modes

| Mode | Description |
|------|-------------|
| `Standalone` | Full egui window; no network; DB optional |
| `Connected { host, port }` | No GUI; streams state to aggregator over TCP |

### Aggregator modes

| Mode | Description |
|------|-------------|
| GUI | Native egui window; renders all connected players |
| Headless / WebSocket | `ws_port` set in config; serves HTTP + WebSocket overlay |

Both modes can be running simultaneously — the aggregator detects whether `ws_port` is set and takes the appropriate path.

## Thread Inventory

| Process | Thread | Purpose | Period |
|---------|--------|---------|--------|
| tracker | memory | UDP snapshot of EWRAM + IWRAM | 100 ms |
| tracker | box monitor | Reads all 420 box slots from EWRAM | 250 ms |
| tracker | game-polling | Map, party, encounters, encounter detection, badge events | 100 ms |
| tracker | network (connected) | Reconnect loop; `handle_client` reader + writer | on connect |
| aggregator | TCP listener | Accepts incoming tracker connections | blocking |
| aggregator | per-tracker | `handle_tracker_connection` writer + reader | on connect |
| aggregator | broadcast | Drains sprites, builds JSON, WS broadcast | 100 ms |

## Database Schema Notes

### `events` table

Columns: `id`, `run_id`, `player_name`, `event_type`, `species_name`, `nickname`, `old_nickname`, `level`, `occurred_at`.

`old_nickname` is populated only for `nickname_change` events (holds the name that was overwritten); it is an empty string for all other event types.

### Badge boot guard

The game-polling thread tracks badge state with a `last_badge_mask: Option<u8>`. `None` means "uninitialized". `check_for_new_badges` silently adopts the current badge state on the first call with `None`, preventing false-positive events on tracker startup and after a wipe or run change. The mask is reset to `None` on: game unload, wipe detected, and `thread_run_changed` signal.

## Design Notes

### `LockOrRecover`

`pub trait LockOrRecover<T>` lives in `fire_red_states` and is the single shared implementation of poison-recovering mutex locking. All crates that need it import `fire_red_states::LockOrRecover` — do **not** define a local copy.

### Wild encounter pointer field naming

`WildHeaderRom` in `fire_red_pokemon_data` uses `land_mon_encounters_rom_ptr` (note: corrected from the historical typo `enounters`). The public `EncounterHeader` field remains `land_mon_encounters`.

### Memory read concurrency (`fire_red_memory`)

EWRAM (256 KiB, 64 × 4 KiB chunks) and IWRAM (32 KiB, 8 × 4 KiB chunks) are
read on separate region threads.  Each thread uses a **sliding window**:

- A `Mutex<usize> + Condvar` counting semaphore is initialised to
  `MAX_CONCURRENT_CHUNKS = 16`.
- A chunk thread **acquires** a slot before it starts and **releases** it before
  sending its result, so the dispatch loop can immediately pick up the next
  chunk without waiting for the channel `recv`.
- Results arrive via `mpsc::channel`; after all chunks are dispatched the
  dispatch-side sender is dropped so `rx` drains to completion.
- Each region stores its result as soon as it finishes — IWRAM (~1 round-trip,
  ~16 ms) does not block on EWRAM (~4 round-trips, ~64 ms).

This eliminates the head-of-line stall in the old strict-batch design, where a
single slow or retrying chunk held up the entire next batch.

### Error visibility

- Sprite decompression failures in the aggregator's `decompress_pixels` are logged at `WARN` level via `tracing::warn!`.
- Database dump task panics in `serve_db_json` are logged at `ERROR` level via `tracing::error!`.
- `eframe::run_native` errors in the aggregator are printed to stderr.

### Wild encounter header scanner

`looks_like_header` in `fire_red_scanner` requires **all four** encounter table pointers to be either zero or a valid GBA ROM address. A zero pointer is valid and means "no encounters of that type on this map". Partial validity (some zero, some non-ROM) would indicate corrupted or misidentified data.

### `allow_species_repeats`

When `TrackerConfig::allow_species_repeats` is `true`, `EncounterTracker::tick()` skips the `species_encountered` check ("have we recorded this species anywhere in the run?"). The per-area deduplication and the dupes clause both still apply normally. The effect is that the same species can be recorded as a first encounter on multiple different routes — useful for randomized ROMs or Nuzlocke variants that don't restrict by species history.

### Bot summary endpoint

`GET /api/bot/:index` returns a plain-text string: `"<player> — <hp>/<max_hp> HP — <zone>"`. It reads from `SharedSlots` directly (same source as `/api/slot/:index`) but formats the result as a single human-readable line instead of JSON, making it easy to consume from a Twitch chat bot or stream overlay widget.

### Run comparison page (`/compare`)

`GET /compare` serves `compare.html`, a self-contained JavaScript page. It fetches the run list from `/api/runs` and loads per-run stats from `/api/run/:id/stats` on demand. Stats are cached in-page to avoid repeat round-trips. Numeric comparisons highlight the better value green and worse value red using a `higherIsBetter` flag per metric.
