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
  fire_red_memory          ── UDP snapshots of EWRAM/IWRAM
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
