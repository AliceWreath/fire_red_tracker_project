# Fire Red Tracker

A real-time Pokémon FireRed party and encounter monitor built in Rust. It reads live game state from a running RetroArch instance using the mGBA core, displays the player's current party and wild encounter table in a native GUI, and supports multi-player Soul Link / Nuzlocke runs through a networked aggregator.

---

## Documentation

| Document | Contents |
|----------|----------|
| [docs/architecture.md](docs/architecture.md) | Crate dependency graph, system architecture diagram, thread inventory |
| [docs/data_flow.md](docs/data_flow.md) | End-to-end data flow (RetroArch → tracker → aggregator → overlay), DB write/read paths, sprite pipeline, run-change flow |
| [docs/memory_map.md](docs/memory_map.md) | GBA EWRAM/IWRAM/ROM address map, party and wild-encounter struct layouts |
| [docs/ffi.md](docs/ffi.md) | C FFI entry points, ownership and lifetime diagrams for `fire_red_loop`, `fire_red_badge`, and `fire_red_pokemon_data` FFI allocations |

---

## What it does

- **Party panel** — shows each Pokémon's sprite (shiny-aware), nickname, level, HP (colour-coded), experience, caught location, and badge progress in real time.
- **Encounters panel** — shows the wild Pokémon available in the current area, split by type: grass, water/fishing, and Rock Smash. Updates automatically when the player moves to a new map.
- **Badge tracker** — displays obtained badges as coloured dots and shows the next gym leader's name, city, and highest Pokémon level.
- **First-encounter tracking** — records the first wild Pokémon encountered in each area (Nuzlocke rule). Catches are flagged automatically when the Pokémon appears in the player's party.
- **Reset detection** — clears stale party, encounter, and badge data when a soft reset or title screen is detected.
- **Soul Link detection** — in aggregator mode, Pokémon caught in the same location across two or more players' games are automatically linked and labelled in purple.
- **Clean ROM mode** — pass `--clean` to also display ability data (only reliable on unmodified ROMs).

---

## Modes

### Standalone
Runs locally. Reads the ROM and polls RetroArch on the same machine. Displays a local GUI.

```
tracker firered.gba
tracker firered.gba --clean
```

### Connected
Like standalone but connects to a running aggregator and streams game state to it. Runs headless (no local GUI). The aggregator is the display surface.

```
tracker firered.gba connect
tracker firered.gba connect --host 192.168.1.10 --port 7878
```

Default host is `127.0.0.1`, default port is `7878`. The tracker reconnects automatically if the aggregator restarts.

### Aggregator
A separate binary for Soul Link / co-op runs. **Listens** for incoming tracker connections — no addresses need to be configured in advance. Each tracker dials out to the aggregator when started in connected mode.

```
aggregator
aggregator --listen-port 7878
```

The window width scales as trackers connect. Soul Link matches (Pokémon sharing the same caught location across players) are highlighted automatically. If a tracker disconnects, its slot shows as disconnected and its last known data is preserved; when it reconnects it reuses the same slot.

#### Database integration

Pass a PostgreSQL connection string with `--db` to enable persistent Nuzlocke tracking.

```
aggregator --db postgresql://user:pass@host/nuzlocke
```

When a database is connected the aggregator:

- Tracks the active **Nuzlocke run** (start time, player name, end time). Multiple runs are stored; the most recent active run is used.
- Records every **first encounter** per map area: species, level, whether it was caught or fled.
- Records every **caught Pokémon** (species, nickname, IVs, met location, timestamp).
- Records every **death** (full stats snapshot, timestamp).
- Automatically propagates **Soul Link deaths** — when one partner faints, the other is immediately marked dead in the database.
- Shows a green **● Run #N** indicator in each player column; shows amber **● No active run** when the run has been ended.
- When no run is active, replaces the live party panel with a **run summary** (start/end time, death/catch counts) and shows the recorded first-encounter list in place of the live encounter table.

The database must already exist (`CREATE DATABASE nuzlocke;`). The schema is created automatically on first run.

#### Run management

The aggregator web overlay (see below) has **End Run** and **New Run** buttons in every player column. Both buttons apply to **all connected trackers simultaneously** — you don't need to manage each player separately.

- **End Run** — marks the current run as ended (sets `ended_at`). The tracker stops recording deaths, catches, and encounters. The web display switches to the run summary / history view.
- **New Run** — creates a fresh run on every connected tracker and makes it active. Recording resumes immediately.

A confirmation dialog is shown for both actions. The "End Run" button is disabled when no run is active.

#### WebSocket overlay mode

Pass `--ws-port PORT` to run the aggregator as a headless HTTP + WebSocket server instead of opening a window. This is designed for OBS Browser Source overlays.

```
aggregator --db postgresql://... --ws-port 9090
```

In OBS, add a **Browser Source** pointing to `http://localhost:9090`. The overlay updates in real time over WebSocket (up to 10 times per second when state changes, zero bandwidth when idle).

The following pages are available:

| URL | Content |
|---|---|
| `http://localhost:PORT/` | Full overlay — all players side by side |
| `http://localhost:PORT/0/party` | Player 1's party (or run summary if no active run) |
| `http://localhost:PORT/0/encounters` | Player 1's area encounters (or DB encounter log if no active run) |
| `http://localhost:PORT/0/dead` | Player 1's dead Pokémon log (requires `--db`) |
| `http://localhost:PORT/0/caught` | Player 1's caught Pokémon log (requires `--db`) |
| `http://localhost:PORT/1/party` | Player 2's party / run summary |
| `http://localhost:PORT/1/encounters` | Player 2's encounters / DB log |
| `http://localhost:PORT/1/dead` | Player 2's dead Pokémon log |
| `http://localhost:PORT/1/caught` | Player 2's caught Pokémon log |

The full overlay and per-player pages can all be added as separate Browser Sources in OBS and positioned independently.

> **ROM paths with spaces** can be quoted: `tracker "My ROMs/fire red.gba"`

---

## Soul Link / Nuzlocke context

In a **Nuzlocke** run, the player may only catch the first Pokémon encountered in each new area, and any Pokémon that faints is considered dead and must be released. The encounter panel makes it easy to see at a glance which Pokémon are available before stepping into grass. The first-encounter tracker automatically records the area's encounter and updates it to "caught" when the Pokémon appears in the party.

A **Soul Link** is a Nuzlocke variant played with a partner: each player's catches are paired with their partner's catch from the same route. If one linked Pokémon faints, both must be released. The aggregator automates this in two layers:

1. **Instant visual detection** — compares `met_location` across all connected players' live party data every 100 ms. The moment a Pokémon's HP reaches zero, its partner is shown as dead immediately, before the tracker has written anything to the database.

2. **Database propagation** (requires `--db`) — once the tracker writes a death record, the aggregator cross-references it against the caught table and writes a corresponding soul-link death record for the partner. This persists across sessions.

> **Limitation:** Soul Link matching uses `met_location` as the pairing key. This is reliable on standard FireRed but may produce false positives on heavily modified ROMs where multiple areas share a location ID.

---

## Architecture

### Workspace crates

| Crate | Role |
|---|---|
| `fire_red_loop` | Central coordinator. Owns the main map-polling loop, starts party/box/trainer monitors, and exposes the public API used by the GUI and network layers. |
| `fire_red_memory` | Maintains full EWRAM and IWRAM snapshots by reading from RetroArch in parallel chunks every 500 ms. All other crates read from these snapshots rather than issuing individual UDP requests. |
| `fire_red_party_monitor` | Reads and decrypts the player's party from the EWRAM snapshot. Owns `Party`, `Pokemon`, `BoxPokemon`, and all encrypted substructure types. Runs its own background poll loop. |
| `fire_red_box_monitor` | Reads all 14 PC boxes (420 slots) from the EWRAM snapshot on a slow cycle. Maintains a deduplicated species cache and detects newly caught Pokémon. |
| `fire_red_badge` | Reads badge flags from SaveBlock1 via the EWRAM/IWRAM snapshots. Exposes `BadgeState`, next-gym info, and a C-ABI FFI surface. |
| `fire_red_trainer_data` | Reads trainer name, rival name, gender, trainer ID, and play time from SaveBlock2 via the EWRAM snapshot. |
| `fire_red_image_data` | Extracts and decodes Pokémon front sprites from the ROM: pointer resolution → LZ77 decompression → 4bpp tile decode → BGR555 palette → RGBA image. |
| `fire_red_pokemon_data` | Wild encounter table types (`WildPokemonHeader`, `WildPokemonInfo`, `WildPokemon`). Parses encounter data from ROM and provides both safe Rust and FFI-compatible representations. |
| `fire_red_get_values` | Low-level byte parsing utilities. Three families: `get_*` for RetroArch hex-token buffers (LE), `read_*` for raw byte slices (LE), `read_*_raw` for raw byte slices (BE). |
| `fire_red_states` | Shared types and length-prefixed bincode TCP message protocol: `GameState`, `ServerMessage`, `ClientMessage`, `SpriteData`, `Mode`. Used by both tracker and aggregator. |
| `fire_red_database` | PostgreSQL persistence layer. Manages runs, encounters, caught Pokémon, and deaths. Provides both a write API (used by the tracker process) and a read-only `DbReader` (used by the aggregator). |
| `fire_red_retroarch_interfacing` | Sends `READ_CORE_MEMORY` commands to RetroArch over UDP and parses the whitespace-tokenised responses. |
| `fire_red_rom_buffer` | Global ROM buffer. Loaded once from disk via `fill_rom` and shared as a `&'static [u8]` across all crates for the process lifetime. |
| `fire_red_scanner` | Scans the ROM binary with heuristic validation to locate the `WildMonHeader` table offset, which varies between ROM revisions. |
| `fire_red_text` | Decodes FireRed's custom GBA text encoding into UTF-8. Builds and caches the full Pokémon name table from ROM at startup. |
| `fire_red_pokemon_name_buffer` | Global Pokémon name repository. Initialised once from the decoded name table and shared as a `&'static [String]`. |

### Binaries

| Binary | Description |
|---|---|
| `tracker` | Standalone or connected mode. Reads live game state from RetroArch. In connected mode, dials out to the aggregator and streams state to it headlessly. |
| `aggregator` | Multi-player Soul Link viewer. Listens for incoming tracker connections; no addresses need to be pre-configured. Optional `--db` flag enables PostgreSQL-backed death/catch/encounter tracking and Soul Link propagation. Optional `--ws-port` flag switches to a headless WebSocket overlay server for OBS. |

### Tracker source layout

| Module | Responsibility |
|---|---|
| `main.rs` | Entry point, thread spawning, mode dispatch |
| `cli.rs` | `Cli` and `Command` structs (clap definitions) |
| `config.rs` | Config file load/save, first-run setup dialog |
| `encounter.rs` | `EncounterTracker` — wild battle detection via personality change, catch detection via party membership |
| `game.rs` | `is_shiny`, `fill_party_list`, `map_state_from_ewram`, `game_is_loaded` |
| `textures.rs` | `PendingTexture`, sprite compression, `build_sprite_data` |
| `gui.rs` | `WindowInfo`, `eframe::App` impl, party panel, encounters viewport |
| `server.rs` | Aggregator connection handler — manages the bidirectional push stream over an established TCP connection |

### Key external dependencies

| Crate | Purpose |
|---|---|
| `eframe` / `egui` | Native GUI framework and immediate-mode UI rendering |
| `clap` | CLI argument parsing with derive macros |
| `image` | `ImageBuffer<Rgba<u8>>` used for decoded sprite data |
| `flate2` | zlib compression/decompression for sprite data sent over TCP |
| `bincode` | Binary serialisation of `GameState`, `SpriteData`, and message enums |
| `serde` / `serde_big_array` | Derive macros for serialisable types; `BigArray` for fixed-size array fields in encrypted substructures |
| `arc-swap` | Lock-free `Arc` swapping for the party monitor's hot-path reads (`ArcSwap<Party>`) |
| `colored` | Terminal colour output for the connected-mode startup banner |
| `ctrlc` | Ctrl-C signal handler for clean shutdown in connected mode |
| `postgres` | Synchronous PostgreSQL client used by the tracker and aggregator for Nuzlocke persistence |
| `axum` | HTTP + WebSocket server for the aggregator's OBS overlay mode (`--ws-port`) |
| `tokio` | Async runtime backing the axum WebSocket server |
| `serde_json` | JSON serialisation of game state pushed to WebSocket clients |
| `futures-util` | Stream/sink utilities for bidirectional axum WebSocket handling |

---

### Memory access model

All live game data is read from two in-memory snapshots maintained by `fire_red_memory`:

- **EWRAM snapshot** (256 KiB) — refreshed every 500 ms by reading from RetroArch in parallel 4 KiB chunks over UDP, then assembled in address order.
- **IWRAM snapshot** (32 KiB) — refreshed on the same cycle.

Every other crate reads from these snapshots rather than issuing its own UDP requests. This eliminates hundreds of individual network round-trips per second and makes the read pattern predictable regardless of how many subsystems are running.

The map state is read directly from the EWRAM snapshot (at `0x02031DBC`) rather than through the `STATE` mutex in `fire_red_loop`. This avoids the cumulative lag of two polling intervals (~833 ms) and the race condition where `STATE` contains `(0, 0)` before the map thread has ticked for the first time.

---

### Wild encounter tracking

The tracker monitors `gEnemyParty[0]` at `0x0202402C` to detect wild battles. FireRed never clears this slot between battles — it is overwritten at the *start* of each new battle by `CreateWildMon`. Detection therefore uses personality *change* rather than presence/absence:

- When the personality value at `gEnemyParty[0]` changes, a new wild battle has started.
- Wild Pokémon receive the player's OT ID (via `CreateWildMon` → `OT_ID_PLAYER_ID`). Comparing the enemy's `ot_id` against the lead party member's `ot_id` distinguishes wild encounters from trainer battles.
- Only the **first encounter per map area** is recorded (Nuzlocke rule). Subsequent encounters in the same area are silently ignored.
- **Catch detection** watches the player's party for the exact personality value of the tracked wild Pokémon. No timer is needed — the next battle (personality change) implicitly closes any unresolved encounter as "failed/fled".

---

### Reset detection

The tracker detects soft resets and title screens by checking three signals on every poll tick:

1. The SaveBlock1 pointer at `0x03005008` in IWRAM points into valid EWRAM.
2. The party size byte at `0x02024029` is in the range 0–6.
3. The map group/name bytes at `0x02031DBC` are non-zero.

If any check fails, the shared party, encounter, and badge data are cleared and the encounter tracker is reset.

---

### Thread model

#### Tracker — standalone

```
main thread  (GUI)
│
├── memory thread             refresh EWRAM + IWRAM snapshots every 500 ms
├── game-polling thread       read map/party/encounters from snapshots every 100 ms
│                             EncounterTracker runs here (personality-change detection)
├── party-monitor thread      read party on size-change + force-refresh every 5 s
├── box-monitor thread        read all 14 PC boxes every 5 s
└── trainer-data thread       read trainer name / play time every 15 s
```

#### Tracker — connected

```
main thread  (headless, parked until Ctrl-C)
│
├── game-polling thread       (same as standalone)
│
└── network thread            outer reconnect loop (retry every 5 s on disconnect)
        └── on each connection: handle_client(stream, ...)
                ├── writer loop    push GameState snapshot to aggregator every 100 ms
                └── reader thread  receive ClientMessage (RequestTextures / EndRun / NewRun)
```

#### Aggregator

```
main thread  (GUI window or headless WebSocket server)
│
├── TCP listener thread       accept incoming tracker connections
│       └── per-tracker thread   handle_tracker_connection(stream, slot_arcs)
│               ├── writer thread    drain texture_request_queue + command_queue every 50 ms
│               └── reader loop      receive State + Textures + RunChanged, update shared Arcs
│
└── [ws-port mode] broadcast thread   BroadcastLoop::tick() every 100 ms → push JSON to WebSocket clients
```

Slots are created (or reused from a disconnected slot) when a tracker connects. `SharedSlots = Arc<Mutex<Vec<Arc<MonitorSlot>>>>` is shared between the listener, `BroadcastLoop`, and the WebSocket handler. The GUI and BroadcastLoop snapshot the slot list each frame/tick so the mutex is held only briefly.

All inter-thread data flows through `Arc<Mutex<_>>` or `AtomicBool`. The GUI never holds a mutex during rendering — state is snapshotted at the start of each frame.

---

### Network protocol

All TCP messages use a simple length-prefixed bincode frame:

```
[4-byte big-endian length][bincode-encoded message body]
```

The tracker always sends `ServerMessage` and receives `ClientMessage`, regardless of which side initiated the TCP connection.

| Direction | Message | Contents |
|---|---|---|
| Tracker → Aggregator | `ServerMessage::State` | Full `GameState` (party + encounters + badges + trainer name), sent every 100 ms |
| Tracker → Aggregator | `ServerMessage::Textures` | `Vec<SpriteData>` (zlib-compressed RGBA + metadata) |
| Tracker → Aggregator | `ServerMessage::RunChanged(Option<u32>)` | Confirms a run change: `None` = run ended, `Some(id)` = new run ID |
| Aggregator → Tracker | `ClientMessage::RequestTextures` | `Vec<u16>` of species IDs to fetch |
| Aggregator → Tracker | `ClientMessage::EndRun` | End the active run on this tracker |
| Aggregator → Tracker | `ClientMessage::NewRun` | Start a new run on this tracker |

Maximum allowed message size is 20 MB, enforced on receive.

---

### Pokémon data structures

FireRed stores each party Pokémon as a 100-byte structure. The first 80 bytes are the boxed form (`BoxPokemon`) and the remaining 20 bytes are live battle stats (`Pokemon`). Inside `BoxPokemon`, 48 bytes of "secure" data are XOR-encrypted with `personality ^ ot_id` and split into four 12-byte substructures whose order is determined by `personality % 24`:

| Letter | Substructure | Contents |
|---|---|---|
| `G` | `GrowthSubstruct` | Species, held item, experience, PP bonuses, friendship |
| `A` | `AttackSubstruct` | Move IDs, current PP values |
| `E` | `EvConditionSubstruct` | Stat EVs, contest conditions |
| `M` | `MiscSubstruct` | Pokérus, met location, origins, IVs, egg/ability flags, ribbons |

After decryption, a checksum over the 48 secure bytes is verified against the stored `checksum` field. Slots with a failing checksum are treated as empty.

Shiny detection uses the Gen III formula: `(p_high XOR p_low XOR id_high XOR id_low) < 8`, where `p_high`/`p_low` are the upper/lower 16 bits of the personality value and `id_high`/`id_low` are the upper/lower 16 bits of the combined OT ID.

---

### Badge data

Badge flags are stored as individual bits in the flags array inside SaveBlock1. The SaveBlock1 base address is resolved at runtime by dereferencing the pointer at `0x03005008` in IWRAM.

| Flag index | Badge | Leader | City | Max level |
|---|---|---|---|---|
| `0x820` | Boulder Badge | Brock | Pewter City | 14 |
| `0x821` | Cascade Badge | Misty | Cerulean City | 21 |
| `0x822` | Thunder Badge | Lt. Surge | Vermilion City | 24 |
| `0x823` | Rainbow Badge | Erika | Celadon City | 29 |
| `0x824` | Soul Badge | Koga | Fuchsia City | 43 |
| `0x825` | Marsh Badge | Sabrina | Saffron City | 50 |
| `0x826` | Volcano Badge | Blaine | Cinnabar Island | 54 |
| `0x827` | Earth Badge | Giovanni | Viridian City | 55 |

---

### Sprite pipeline

Sprites are decoded on first use and cached for the process lifetime:

1. Follow the two-level ROM pointer table (`FRONT_SPRITE_TABLE_PTR` at ROM offset `0x128`) to find the compressed sprite blob.
2. Decompress with GBA LZ77 (BIOS type `0x10`).
3. Decode 8×8-pixel tiles from 4bpp format into a flat palette-index array. All sprites are 64×64 pixels.
4. Resolve the 16-colour BGR555 palette to RGBA8. Palette index 0 is always transparent.
5. Compress the raw RGBA pixels with zlib before sending over TCP.
6. Decompress on the aggregator and upload to the GPU via `egui::Context::load_texture` or encode to PNG for the WebSocket overlay.

Both normal and shiny variants are sent together when a species is first requested. The aggregator maintains a per-process sprite cache keyed by `(species, shiny)` so the ROM is decoded at most once per variant per session.

---

### Memory layout (FireRed USA Rev 1)

RetroArch memory reads use the `READ_CORE_MEMORY` UDP command (default port 55355). Key addresses:

| Symbol | Address | Notes |
|---|---|---|
| Party size | `0x02024029` | 1 byte, valid range 0–6 |
| Party data | `0x02024284` | Up to 6 × 100-byte `Pokemon` structs |
| Enemy party | `0x0202402C` | `gEnemyParty[0]` — wild Pokémon slot; never cleared between battles |
| PC box storage | `*0x03005010 + 0x4` | `SaveBlock3` pointer + offset; 14 × 30 × 80 bytes |
| Current map | `0x02031DBC` | 2 bytes: map group, map name |
| SaveBlock1 ptr | `0x03005008` | 4-byte IWRAM pointer; dereference for badge flag offset |
| SaveBlock2 (trainer) | `0x02024298` | 19 bytes: trainer name, gender, ID, play time |
| WildMonHeaders | scanned at startup | Offset varies; `fire_red_scanner` locates it via heuristic validation |
| Ability names | `0x24FCB0` | 13 bytes per entry |
| Base stats | `0x2547F4` | 28 bytes per entry; ability slots at +`0x16` / +`0x17` |
| Pokémon names | `0x245F5B` | GBA-encoded, `0xFF`-terminated, up to species `0x019B` |

The PC box base address is not fixed — it is resolved at runtime by reading the `SaveBlock3` pointer at `0x03005010` and adding `0x4`. Similarly, the badge flag array is located by dereferencing the `SaveBlock1` pointer at `0x03005008` and adding `0x0EE0`.

---

### Wild encounter header scanning

`fire_red_scanner` locates the `WildMonHeader` table by scanning the ROM in 4-byte-aligned increments. Each candidate offset is checked against these heuristics:

- 2-byte padding field is zero.
- Map group ≤ 50 and map number ≤ 200.
- All four encounter table pointers are either zero or fall within `[0x08000000, 0x09000000)`.

A candidate is confirmed only if scanning forward from it finds more than 50 consecutive valid headers followed by a `0xFF` sentinel byte.

---

## Building

```
cargo build --release
```

Both binaries (`tracker` and `aggregator`) are produced in `target/release/`.

---

## Running

### Prerequisites

- RetroArch must be running with the mGBA core loaded and the network command interface enabled (Settings → Network → Network Commands → On, default port 55355).
- The FireRed ROM file must be accessible on disk for tracker processes.
- PostgreSQL must be reachable if using `--db`. Create the database once before first use: `psql -c 'CREATE DATABASE nuzlocke;'`. The schema is created automatically on first run.

### Configuration

Both `tracker` and `aggregator` store their settings in a config file (`~/.config/fire_red_tracker/config.toml` and `~/.config/fire_red_aggregator/config.toml`). A setup dialog is shown on first launch to create the file. Settings can be overridden for a single run with CLI flags without modifying the saved config.

### Quick start — solo Nuzlocke

```
./aggregator --db postgresql://localhost/nuzlocke &
./tracker firered.gba connect
```

Or standalone with just the local GUI:

```
./tracker firered.gba
```

### Quick start — Soul Link with a friend

**Start the aggregator first (one machine, accessible to both players):**
```
./aggregator --db postgresql://localhost/nuzlocke
```

**Each player then starts their tracker in connected mode:**
```
./tracker firered.gba connect --host aggregator-ip --port 7878
```

The aggregator window shows both players side by side as soon as each tracker connects.

**OBS WebSocket overlay (headless aggregator):**
```
./aggregator --db postgresql://localhost/nuzlocke --ws-port 9090
```

Then in OBS add Browser Sources for whichever pages you need:
- `http://localhost:9090/` — full side-by-side overlay
- `http://localhost:9090/0/party` and `http://localhost:9090/1/party` — per-player party panels
- `http://localhost:9090/0/dead` and `http://localhost:9090/1/dead` — death logs

The web overlay also hosts **End Run** and **New Run** buttons. Clicking either button sends the command to every connected tracker simultaneously, so both players' runs are managed together.

---

## Project status

Personal project built for Nuzlocke and Soul Link runs. The codebase is functional but not hardened for general distribution:

- ROM scanning and all hardcoded addresses are calibrated for **FireRed USA (Rev 1)**. Other regional releases or ROM hacks will likely require address adjustments.
- The `--clean` ability feature reads from ROM base-stat tables and is only reliable on unmodified ROMs.
- The `WildPokemonHeaderFFI` and `AreaEncountersStringArrays` FFI types are partially implemented; the C-callable interface helpers are in progress pending a stable API design.
