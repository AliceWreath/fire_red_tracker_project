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
     │   ├── fire_red_location_names
     │   └── fire_red_retroarch_interfacing
     ├── fire_red_database
     │   └── fire_red_location_names
     ├── fire_red_image_data
     ├── fire_red_trainer_data
     └── fire_red_location_names

Support crates (no external deps):
  fire_red_memory          ── sliding-window UDP snapshots of EWRAM/IWRAM
  fire_red_retroarch_interfacing ── UDP socket helpers
  fire_red_scanner         ── ROM header scan
  fire_red_pokemon_data    ── ROM wild-encounter tables + FFI
  fire_red_badge           ── Badge flags + FFI
  fire_red_text            ── GBA character encoding
  fire_red_image_data      ── LZ77 sprite decompression
  fire_red_map_data        ── Gym progression table
  fire_red_location_names  ── Map-area name lookup; `map_area_name` for encounter zones, `location_name` for met-location bytes, `all_wild_areas` for route-odds coverage
  fire_red_rom_buffer      ── Global ROM byte slice
  fire_red_pokemon_name_buffer ── Cached Pokémon name list
  fire_red_trainer_data    ── Trainer name / ID helpers
  fire_red_party_monitor   ── Party Pokemon struct; `species_type_static` ROM-free type lookup
  fire_red_get_values      ── Misc EWRAM value helpers
```

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  RetroArch  (UDP :55355)                                            │
│  responds to READ_CORE_MEMORY / WRITE_CORE_MEMORY commands          │
└──────────┬──────────────────────────────────────────────────────────┘
           │ UDP (one connection per slot)
           ▼
┌──────────────────────────────────────────────────────┐
│  fire_red_tracker  (standalone binary, optional)      │
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
│  └────────────────────────────────┘                   │
│  ┌────────────────────────────────┐                   │
│  │ Box-monitor thread  (5 s)      │                   │
│  └────────────────────────────────┘                   │
│  ┌────────────────────────────────┐                   │
│  │ Webhook / OBS thread           │                   │
│  │  drains event channel;         │                   │
│  │  fires HTTP POSTs + OBS clips  │                   │
│  └────────────────────────────────┘                   │
└──────────────────────────────────────────────────────┘

  (Aggregator connects directly to RetroArch — no tracker binary required)

┌──────────────────────────────────────────────────────┐
│  fire_red_aggregator                                  │
│                                                       │
│  Per-slot game loop (one per RetroArch host):         │
│  start_loop_ctx() — isolated MemoryContext/           │
│                      PartyContext/TrainerContext       │
│  ┌────────────────────────────────────────────┐      │
│  │  Memory thread      EWRAM + IWRAM (100 ms) │      │
│  │  Game-polling thread                100 ms │      │
│  │  Box-monitor thread                   5 s  │      │
│  │  Trainer-data thread                 15 s  │      │
│  │  Webhook/OBS thread         event-driven   │      │
│  └────────────────────────────────────────────┘      │
│                                                       │
│  SharedSlots = Arc<Mutex<Vec<Arc<MonitorSlot>>>>      │
│                                                       │
│  Injection commands:                                  │
│  HTTP handler → mpsc → game loop thread               │
│  → game::* fn → WRITE_CORE_MEMORY UDP → RetroArch    │
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
| `Standalone` | Full egui window; reads ROM + polls RetroArch directly; DB optional |

### Aggregator modes

| Mode | Description |
|------|-------------|
| GUI | Native egui window; renders all connected players |
| Headless / WebSocket | `ws_port` set in config; serves HTTP + WebSocket overlay |

Both modes can be running simultaneously — the aggregator detects whether `ws_port` is set and takes the appropriate path.

The aggregator always runs in **direct mode** — it polls each configured RetroArch host directly over UDP. Players can be pre-configured via `retroarch_hosts` in the config, or they can connect on demand via the `/join` page when `direct_mode = true`.

## Thread Inventory

| Process | Thread | Purpose | Period |
|---------|--------|---------|--------|
| tracker | memory | UDP snapshot of EWRAM + IWRAM | 100 ms |
| tracker | box monitor | Reads all 420 box slots from EWRAM | 5 s |
| tracker | trainer-data | Reads trainer name / play time | 15 s |
| tracker | game-polling | Map, party, encounters, encounter detection, badge/status events | 100 ms |
| tracker | webhook worker | Drains webhook channel; fires HTTP POSTs with retry + HMAC signing | event-driven |
| tracker | Twitch Helix worker | Markers, polls, predictions, clip creation (optional) | event-driven |
| aggregator | per-slot memory | UDP snapshot of EWRAM + IWRAM (one thread per RetroArch host) | 100 ms |
| aggregator | per-slot box monitor | Reads PC box contents for one slot | 5 s |
| aggregator | per-slot game-polling | Map, party, encounters, events for one slot | 100 ms |
| aggregator | per-slot webhook worker | Event HTTP POSTs for one slot (optional) | event-driven |
| aggregator | broadcast | Drains sprites, builds JSON, WS broadcast | 100 ms |
| aggregator | Twitch IRC bot | IRC connection; dispatches `!party`/`!deaths`/`!shinies`/`!status`/`!moves`/`!ivs`/`!badges`/`!bag`/`!map`/`!encounter`/`!luck`/`!timer`/`!box`/`!run` (optional) | event-driven |
| aggregator | Channel Points EventSub | Twitch EventSub WebSocket; redeems mapped to run commands (optional) | event-driven |
| aggregator | Discord live embed | Edits pinned Discord message with party/badge state (optional) | configurable interval |
| aggregator | Discord run thread | Creates per-run threads; posts badge milestones (optional) | 5 s poll |
| aggregator | YouTube chat bot | Polls YouTube Live Chat; responds to viewer commands (optional) | configurable poll |

## Database Schema Notes

### Table inventory (schema v27+)

| Table | Key columns | Notes |
|---|---|---|
| `runs` | `id`, `player_name`, `started_at`, `ended_at` | One row per Nuzlocke run |
| `dead_pokemon` | `run_id`, `player_name`, `personality`, `species_name`, `nickname`, `level`, `died_at`, `area_name`, IVs/EVs/stats | `ON CONFLICT (run_id, personality) DO NOTHING`; `area_name` added v19 |
| `caught_pokemon` | `run_id`, `player_name`, `personality`, `species_name`, `nickname`, `level`, `met_location`, `caught_at`, IVs, `min_hp_seen_hp`, `min_hp_seen_max_hp` | `ON CONFLICT (run_id, personality) DO NOTHING`; HP-danger columns added v12 |
| `encounters` | `run_id`, `player_name`, `map_group`, `map_name`, `species_name`, `level`, `caught`, `is_shiny`, `encountered_at` | First encounter per area; `ON CONFLICT DO NOTHING` |
| `events` | `id`, `run_id`, `player_name`, `event_type`, `species_name`, `nickname`, `old_nickname`, `level`, `occurred_at`, `note` | `old_nickname` for `nickname_change`; `note` annotation column added v14 |
| `webhook_log` | `run_id`, `event_type`, `url`, `success`, `attempts`, `payload`, `fired_at` | Written by webhook worker after every delivery attempt; indexed on `run_id` (v8) |
| `soul_link_overrides` | `run_id`, `personality`, `partner_personality`, `created_at` | Manual soul-link pairings; cleared when a new run starts (v9) |
| `trainer_battles` | `run_id`, `player_name`, `flag_index`, `trainer_name`, `location`, `defeated_at` | UNIQUE on `(run_id, player_name, flag_index)`; flags 0x100–0x3DF (v11) |
| `catch_attempts` | `run_id`, `player_name`, `map_group`, `map_name`, `species_name`, `balls_thrown`, `caught`, `attempted_at` | One row per encounter resolution (v15) |
| `area_visits` | `run_id`, `player_name`, `map_group`, `map_name`, `entered_at`, `exited_at` | Opened on map transition, closed on exit; used by `/api/run/:id/area_times` (v15) |
| `party_snapshots` | `run_id`, `player_name`, `badge_index`, `badge_name`, `occurred_at`, `avg_level` | One row per badge earned; used by level-curve endpoint (v20) |
| `move_uses` | `run_id`, `player_name`, `personality`, `move_slot`, `move_name`, `use_count` | UNIQUE on `(run_id, player_name, personality, move_slot)`; upserted on PP delta (v21) |
| `friendship_log` | `run_id`, `player_name`, `personality`, `friendship`, `logged_at` | Appended when friendship byte changes; threshold event at 220 (v22) |
| `status_events` | `run_id`, `player_name`, `personality`, `nickname`, `status_name`, `event_type`, `occurred_at` | `event_type` is `onset` or `clear`; indexed on `run_id` (v23) |
| `meta` | `key`, `value` | Key-value store; `active_run_id` is the primary key; share tokens stored as `share:<token>` → `<run_id>:<expires_unix>` |
| `users` | `id`, `username`, `password_hash` | One row per registered account; password is bcrypt-hashed (v24) |
| `sessions` | `token`, `user_id`, `created_at`, `expires_at` | Active login sessions; 64-byte hex token set as `frt_token` HttpOnly cookie (v24) |
| `run_invites` | `run_id`, `invited_by`, `invited_user`, `is_request`, `status`, `created_at`, `responded_at` | Run access invites and access requests (v25) |
| `user_integrations` | `user_id`, `kind`, `config JSON`, `updated_at` | Per-user integration configs (Twitch/YouTube/Discord/OBS). PRIMARY KEY `(user_id, kind)` (v27) |

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
