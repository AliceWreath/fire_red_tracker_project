# Fire Red Tracker

A real-time Pokémon FireRed Nuzlocke and Soul Link tracker built in Rust. It reads live game state from a running RetroArch instance via the mGBA core, tracks first encounters, deaths, catches, and shiny encounters across runs, feeds a set of OBS Browser Source overlays through a WebSocket aggregator server, and fires configurable HTTP webhooks on key events for Discord notifications, stream alerts, and other external integrations.

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

### Party panel
Shows each Pokémon's sprite (shiny-aware), nickname, level, nature, HP (colour-coded), experience, ability, held item, growth rate, stat EVs, and IVs in real time.

- **Level cap indicator** — the level value turns orange-red when a Pokémon is at or above the next gym leader's highest Pokémon level.
- **Status conditions** — SLP / PSN / BRN / FRZ / PAR / TOX badges appear inline next to the level, each with a distinct colour, read directly from the Gen III status bitmask.
- **XP to next level** — shows the exact experience needed to reach the next level, computed using the Pokémon's growth rate (all six Gen III curves: Fast, Medium Fast, Medium Slow, Slow, Erratic, Fluctuating).

### Encounter tracking
- **Encounters panel** — shows the wild Pokémon available in the current map area, split by encounter type (grass, water/fishing, Rock Smash). Updates when the player moves to a new map.
- **First-encounter recording** — records the first wild Pokémon encountered per area per run (Nuzlocke rule). Encounters and deaths are not recorded until the player has obtained 5 or more Pokéballs; once that threshold is crossed the latch stays set for the remainder of the run. Duplicate species are skipped. Catches are detected automatically when the Pokémon joins the party.
- **Shiny detection** — the Gen III shiny formula (`p_high ^ p_low ^ id_high ^ id_low < 8`) is evaluated when an encounter is recorded. Shiny encounters are flagged in the database and trigger a shiny alert toast.
- **Route completion board** — a grid showing every Nuzlocke-relevant zone colour-coded as caught (green), failed/fled (red), or not yet visited (grey), grouped by region. Available at `/:index/routes`.

### Alerts overlay
A dedicated transparent OBS source (`/:index/alerts`) that shows timed toast notifications for all run-critical events:

- **Zone-entry** — fires when entering a wild area with no first encounter yet this run. Includes the zone name, a note that no encounter has been recorded, what was caught or fled in that zone on the most recently completed run, and a level-cap warning if any encounter in the zone matches or exceeds the next gym's cap.
- **Faint** — fires when a party member's HP reaches zero. Shows nickname, species, level, and nature.
- **Shiny encounter** — fires when a new encounter is recorded with the shiny flag set. Shows species, level, and zone name. Stays visible for 10 seconds.
- **Party wipe / blackout** — fires when every party member is dead simultaneously. The tracker detects the wipe and calls `end_run()` automatically; the overlay displays the "PARTY WIPED" banner in response to the resulting run-state change.

### Webhooks

The tracker can POST to a user-configured URL whenever a key event occurs. Each event type has its own independent optional URL; any combination can be enabled. Configuration is done via the setup dialog or the in-app ⚙ Settings panel. All keys live under `[webhooks]` in `~/.config/fire_red_tracker/config.toml`. The section is omitted from the file entirely when nothing is configured.

POSTs are fire-and-forget: dispatched on a dedicated background thread with a 5-second timeout so the game-polling loop is never blocked. Failures are printed to stderr.

#### Events

| Event | Trigger | URL key | Template key |
|---|---|---|---|
| `death` | A party member's HP reaches zero and the death is written to the database | `death_url` | `death_template` |
| `catch` | A new Pokémon joins the party (caught, gifted, or traded in) | `catch_url` | `catch_template` |
| `shiny` | A shiny wild Pokémon's personality is detected, before any catch attempt | `shiny_url` | `shiny_template` |
| `wipe` | Every party member is dead and the run ends | `wipe_url` | `wipe_template` |

#### Default payload format

When no template is configured for an event, every POST is `Content-Type: application/json`:

```json
{
  "event":     "death",
  "player":    "Alice",
  "timestamp": 1748989234,
  "pokemon": {
    "nickname": "Bulbasaur",
    "species":  "Bulbasaur",
    "level":    14,
    "shiny":    false,
    "nature":   "Jolly"
  }
}
```

Notes on specific events:
- `wipe` — the `pokemon` field is absent entirely; only `event`, `player`, and `timestamp` are present.
- `shiny` — the `nickname` field is always an empty string because the wild Pokémon has not yet been named at the moment of detection.
- `catch` — includes `"shiny": true` when the caught Pokémon is shiny. A separate `shiny` webhook fires earlier at the moment of wild encounter, before any catch attempt.

#### Custom payload templates

Each event supports an optional `*_template` config key. When set, the template string is rendered using simple `{placeholder}` substitution and the result is POSTed verbatim as the request body (`Content-Type: application/json`). The default JSON schema is not used.

**Available placeholders:**

| Placeholder | Value | Notes |
|---|---|---|
| `{event}` | `death`, `catch`, `shiny`, or `wipe` | |
| `{player}` | Player name from config | |
| `{timestamp}` | Unix timestamp in seconds | |
| `{pokemon.nickname}` | Pokémon's in-game nickname | Empty string for `wipe` events |
| `{pokemon.species}` | Species name | Empty string for `wipe` events |
| `{pokemon.level}` | Level as a plain integer string | Empty string for `wipe` events |
| `{pokemon.shiny}` | `true` or `false` | Empty string for `wipe` events |
| `{pokemon.nature}` | Nature name | Empty string for `wipe` events |

Templates are per-event and independent. You can use a template for `death` and leave `catch` using the default JSON — they do not need to match.

The template string is used as the complete POST body after substitution. The caller is responsible for the resulting content being valid for the receiving service. For Discord webhooks (which require `Content-Type: application/json` with a JSON body), write the template as JSON:

```
{"content": "{player} just lost **{pokemon.nickname}** (Lv.{pokemon.level} {pokemon.species})!"}
```

For services that accept a plain string body, omit the outer JSON wrapper. Either way the `Content-Type` header is `application/json`.

Unknown placeholders (any `{...}` that doesn't match the table above) are left in the output unchanged.

#### Example TOML config

Minimal config with a Discord death alert and default JSON for everything else:

```toml
[webhooks]
death_url      = "https://discord.com/api/webhooks/your-id/your-token"
death_template = '{"content": "💀 **{player}** just lost **{pokemon.nickname}** (Lv.{pokemon.level} {pokemon.species})"}'
catch_url      = "https://discord.com/api/webhooks/your-id/your-token"
```

Full example with all four events and custom templates:

```toml
[webhooks]
death_url      = "https://discord.com/api/webhooks/your-id/your-token"
death_template = '{"content": "💀 **{player}** lost **{pokemon.nickname}** (Lv.{pokemon.level} {pokemon.species}, {pokemon.nature})"}'

catch_url      = "https://discord.com/api/webhooks/your-id/your-token"
catch_template = '{"content": "✅ **{player}** caught a **{pokemon.species}** (Lv.{pokemon.level}, {pokemon.nature}){pokemon.shiny}"}'

shiny_url      = "https://discord.com/api/webhooks/your-id/your-token"
shiny_template = '{"content": "✨ **{player}** encountered a shiny **{pokemon.species}** (Lv.{pokemon.level})!"}'

wipe_url      = "https://discord.com/api/webhooks/your-id/your-token"
wipe_template = '{"content": "☠️ **{player}**'\''s run has ended. Press F."}'
```

The `*_template` keys are optional even when the corresponding `*_url` is set — omitting a template falls back to the default JSON payload. The `[webhooks]` section is omitted from the config file entirely when all URLs and templates are unset.

#### Delivery mechanics

The tracker starts a single long-lived background thread at startup that owns a `reqwest::blocking::Client` with a 5-second timeout. When `fire_event` is called from the game-polling loop it:

1. Looks up the URL and template for that event type.
2. If a template is configured, renders it by substituting all placeholders in a single pass over the string. For `wipe` events the five pokemon placeholders are replaced with empty strings.
3. Enqueues a `(url, body)` pair to the background thread via an `mpsc` channel and returns immediately — the polling loop is never blocked.
4. The background thread drains the channel and POSTs each payload. If a POST fails (network error, timeout, non-2xx response) the error is printed to stderr and delivery is not retried.

### Run management
- **Badge tracker** — displays obtained badges as coloured dots and shows the next gym leader's name, city, and highest level.
- **Reset detection** — clears stale party, encounter, and badge data on soft reset or title screen.
- **Soul Link detection** — Pokémon caught in the same location across two or more connected players are automatically linked and shown in purple.

---

## Modes

### Standalone
Runs locally. Reads the ROM and polls RetroArch on the same machine. Displays a local GUI.

10 seconds after the window opens, a background thread checks GitHub for a newer release. If one is found the window title changes to `Tracker — v{X} available` as a passive reminder. Run `--update` to apply it.

```
tracker firered.gba
tracker firered.gba --new-run                   # start a fresh run (ignores the most recent active run)
tracker firered.gba --run-id 3                  # resume a specific run by its numeric ID
tracker firered.gba --list-runs                 # print all stored runs and exit
tracker firered.gba --scan-balls-pocket         # locate bag balls pocket offset (run with balls in bag)
tracker firered.gba --scan-security-key=<QTY>  # locate bag security key offset (run with QTY balls in bag)
tracker firered.gba --update                    # check GitHub for a newer release and self-update
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

The same background version check runs here: 10 seconds after the window opens, if a newer release is found the title bar changes to `Fire Red Aggregator — v{X} available`.

```
aggregator
aggregator --listen-port 7878
aggregator --update              # check GitHub for a newer release and self-update
```

The window width scales as trackers connect. Soul Link matches (Pokémon sharing the same caught location across players) are highlighted automatically. If a tracker disconnects, its slot shows as disconnected and its last known data is preserved; when it reconnects it reuses the same slot.

#### Database integration

Pass a PostgreSQL connection string with `--db` to enable persistent Nuzlocke tracking.

```
aggregator --db postgresql://user:pass@host/nuzlocke
```

When a database is connected the aggregator:

- Tracks the active **Nuzlocke run** (start time, player name, end time). Multiple runs are stored; the most recent active run is used.
- Records every **first encounter** per map area: species, level, shiny flag, and whether it was caught or fled.
- Records every **caught Pokémon** (species, nickname, IVs, met location, timestamp).
- Records every **death** (full stats snapshot, timestamp).
- Automatically propagates **Soul Link deaths** — when one partner faints, the other is immediately marked dead in the database.
- Shows a green **● Run #N** indicator in each player column; shows amber **● No active run** when the run has been ended.
- When no run is active, replaces the live party panel with a **run summary** (start/end time, death/catch counts) and shows the recorded first-encounter list in place of the live encounter table.

The database must already exist (`CREATE DATABASE nuzlocke;`). The schema is created automatically on first run.

#### Run management

Run commands can be issued from three places, all of which broadcast to **all connected trackers simultaneously**:

- **`/cmd` page** — dedicated command control page at `http://localhost:PORT/cmd`. Shows all connected slots with their current run IDs and party counts. Has **End Run** and **New Run** buttons that POST to the REST API below.
- **REST API** — `POST /api/command/end_run` and `POST /api/command/new_run` (see REST API section below).
- **WebSocket** — send `{ "cmd": "end_run" }` or `{ "cmd": "new_run" }` as a text frame from any connected WebSocket client.

- **End Run** — marks the current run as ended (sets `ended_at`). The tracker stops recording deaths, catches, and encounters. The web display switches to the run summary / history view.
- **New Run** — creates a fresh run on every connected tracker and makes it active. Recording resumes immediately.

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
| `http://localhost:PORT/db` | Database browser — all four tables with sort and filter (requires `--db`) |
| `http://localhost:PORT/db?manage` | Database browser with **Clear All Records** button visible |
| `http://localhost:PORT/db/query` | SQL query tool — run arbitrary SQL, see results in a sortable table |
| `http://localhost:PORT/cmd` | Command control — **End Run** / **New Run** buttons with live slot status |
| `http://localhost:PORT/0/party` | Player 1's party (or run summary if no active run) |
| `http://localhost:PORT/0/encounters` | Player 1's area encounters (or DB encounter log if no active run) |
| `http://localhost:PORT/0/dead` | Player 1's dead Pokémon log (requires `--db`) |
| `http://localhost:PORT/0/caught` | Player 1's caught Pokémon log (requires `--db`) |
| `http://localhost:PORT/0/box` | Player 1's PC box contents (all 14 boxes, sprite + IVs) |
| `http://localhost:PORT/1/party` | Player 2's party / run summary |
| `http://localhost:PORT/1/encounters` | Player 2's encounters / DB log |
| `http://localhost:PORT/1/dead` | Player 2's dead Pokémon log |
| `http://localhost:PORT/1/caught` | Player 2's caught Pokémon log |
| `http://localhost:PORT/1/box` | Player 2's PC box contents |
| `http://localhost:PORT/0/routes` | Player 1's route completion board — all Nuzlocke zones colour-coded caught / failed / unvisited |
| `http://localhost:PORT/1/routes` | Player 2's route completion board |
| `http://localhost:PORT/0/alerts` | Player 1's alerts overlay — transparent OBS source for zone, death, shiny, and wipe toasts |
| `http://localhost:PORT/1/alerts` | Player 2's alerts overlay |
| `http://localhost:PORT/history` | Run history — all past runs with expandable catch / death / encounter logs |

The per-player pages can all be added as separate Browser Sources in OBS and positioned independently. The alerts overlay is fully transparent when idle — nothing appears until an event fires.

#### Overlay themes

All overlay pages (`/`, `/:index/party`, `/:index/encounters`, `/:index/dead`, `/:index/caught`, `/:index/box`) accept a `?theme=` query parameter.

| Value | Look |
|---|---|
| *(omitted)* or `dark` | Default — dark semi-transparent panels (`rgba(0,0,0,0.88)`) with white text |
| `light` | Light grey/white panels, dark text — suits stream layouts with bright backgrounds |
| `minimal` | More transparent panels (`rgba(0,0,0,0.55)`) with softer sprite shadows — cleaner look when the game is visible beneath the overlay |

```
http://localhost:PORT/0/party?theme=light
http://localhost:PORT/0/party?theme=minimal
http://localhost:PORT/?theme=light
```

Other query parameters combine freely: `http://localhost:PORT/0/party?theme=minimal&manage`

#### REST API

All endpoints are served on the same port as the WebSocket overlay (`--ws-port`). JSON responses are `application/json`.

| Endpoint | Method | Description |
|---|---|---|
| `/ws` | `GET` (WS upgrade) | WebSocket stream. Sends the full state JSON immediately on connect, then pushes updates whenever state changes (~10×/s while playing, zero bandwidth when idle). Accepts `{ "cmd": "end_run" }` and `{ "cmd": "new_run" }` text frames from the client |
| `/api/state` | `GET` | Full current state as a JSON array of slot objects — the same payload the WebSocket pushes. Each slot contains party, encounters, dead, caught, box, badges, run summary, and encounter-zone fields |
| `/api/slot/:index` | `GET` | Single slot object by zero-based index. Returns `404` if the index is out of range |
| `/api/command/end_run` | `POST` | Broadcasts `EndRun` to all connected tracker slots. Returns plain text: `"Command 'end_run' sent to N slot(s)"` |
| `/api/command/new_run` | `POST` | Broadcasts `NewRun` to all connected tracker slots. Returns plain text: `"Command 'new_run' sent to N slot(s)"` |
| `/api/db/query` | `POST` | Runs arbitrary SQL against the database. Request body: `{ "sql": "SELECT ..." }`. Response: `{ "columns": ["col1", ...], "rows": [{ "col1": "val", ... }, ...], "rows_affected": N }` or `{ "error": "..." }` on failure. All values are returned as strings. Requires `--db` |
| `/db.json` | `GET` | Full database snapshot — all four tables (runs, caught, dead, encounters) formatted for the browser viewer. Requires `--db` |
| `/db/clear` | `POST` | Deletes all records from every table and removes the active-run meta key. No confirmation, no undo. Requires `--db` |

##### Slot object fields

Each object in the `/api/state` array (and on `/api/slot/:index`) contains:

| Field | Type | Description |
|---|---|---|
| `label` | string | Player name / IP address of the connected tracker |
| `connected` | bool | Whether a tracker is currently connected to this slot |
| `db_connected` | bool | Whether the slot has an active database connection |
| `active_run_id` | number \| null | ID of the current active run, or `null` if none |
| `run_summary` | object \| null | `{ run_id, player_name, started_at, ended_at, deaths, caught }` for the most recent run |
| `badges` | bool[8] | Badge flags in gym order (Boulder → Earth) |
| `next_gym` | object \| null | `{ leader, city, max_level }` for the next gym |
| `party` | array | Up to 6 party member objects (see below) |
| `dead` | array | Dead Pokémon records for the active run, sorted newest first |
| `caught` | array | Caught Pokémon records for the active run, sorted oldest first |
| `box_pokemon` | array | All Pokémon in PC boxes |
| `db_encounters` | array | First-encounter records for the active run |
| `prev_run_encounters` | array | First-encounter records from the most recently completed run (for cross-run hints) |
| `encounters` | array | Live wild encounter tables for the current map area, grouped by type (`Land`, `Water / Fishing`, `Rock Smash`) |
| `current_map_group` | number | EWRAM map group byte for the current position |
| `current_map_name` | number | EWRAM map name byte for the current position |
| `current_zone_name` | string | Human-readable name of the current wild-encounter zone, empty when not in a wild area |

##### Party member fields

| Field | Type | Description |
|---|---|---|
| `nickname` | string | In-game nickname (decoded from GBA text encoding) |
| `species_name` | string | Species name |
| `level` | number | Current level |
| `hp` / `max_hp` | number | Current and maximum HP |
| `exp` | number | Total experience points |
| `nature` | string | Nature name (derived from `personality % 25`) |
| `shiny` | bool | Shiny flag (Gen III formula) |
| `dead` | bool | True if the Pokémon has a death record, HP = 0, or is a soul-link kill |
| `soul_link_kill` | bool | True if the death was triggered by a soul-link partner fainting |
| `soul_link_partner` | object \| null | `{ nickname, player }` of the linked partner across another slot |
| `died_at` | string \| null | UTC timestamp of death |
| `attack` / `defense` / `speed` / `sp_attack` / `sp_defense` | number | Current stats (from death record if dead) |
| `gender` | number | `0` = male, `1` = female, `2` = genderless |
| `ability` | string | Ability name |
| `held_item` | string | Held item name |
| `held_item_id` | number | Held item ID |
| `growth_rate` | string | Growth rate name |
| `ev_hp` … `ev_spd` | number | Stat EVs (0–255) |
| `sprite` | string \| null | `data:image/png;base64,...` PNG sprite URI, or `null` while the sprite is in transit |
| `personality` | number | Raw personality value — used by overlays to detect identity changes |
| `status` | number | Gen III status bitmask: bits 0–2 = SLP turns, bit 3 = PSN, bit 4 = BRN, bit 5 = FRZ, bit 6 = PAR, bit 7 = TOX |

> **ROM paths with spaces** can be quoted: `tracker "My ROMs/fire red.gba"`

---

## Soul Link / Nuzlocke context

In a **Nuzlocke** run, the player may only catch the first Pokémon encountered in each new area, and any Pokémon that faints is considered dead and must be released. The encounter panel shows which Pokémon are available before stepping into grass. The first-encounter tracker records the area's encounter automatically and updates it to "caught" when the Pokémon joins the party. The route completion board gives an at-a-glance view of all zones in the run: which have been completed, which were failed, and which haven't been entered yet. The alerts overlay fires toasts for zone entries, faints, shiny encounters, and party wipes so nothing goes unnoticed — even when looking away from the game.

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
| `fire_red_location_names` | Human-readable location name lookup for FireRed USA Rev 1. Exposes two functions: `map_area_name(group, map)` converts live `(map_group, map_name)` pairs to display strings for the encounter panel; `location_name(loc)` converts a MAPSEC `met_location` byte to a named location for the party panel. Covers all Kanto wild areas, all Sevii Island wild areas, and all routes including the Route 21 North/South split. Constants sourced from the FireRed/LeafGreen map groups document and cross-checked in-game. |
| `fire_red_map_data` | `#[repr(C)]` structs mirroring the in-memory layout of FireRed map data (`MapHeader`, `MapLayout`, `MapEvents`, `WarpEvent`, `CoordEvent`, `BgEvent`, `MapConnections`, `MapConnection`, `ObjectEventTemplate`). Each type has a `fill_*` builder method for deserialising from RetroArch `READ_CORE_MEMORY` hex-token buffers, plus helpers for generating follow-up read commands. **In progress — not yet integrated into the main loop.** |
| `fire_red_states` | Shared types and length-prefixed bincode TCP message protocol: `GameState`, `ServerMessage`, `ClientMessage`, `SpriteData`, `Mode`. Used by both tracker and aggregator. |
| `fire_red_database` | PostgreSQL persistence layer. Manages runs, encounters (including shiny flag and caught status), caught Pokémon, and deaths. Provides a write API used by the tracker and a read-only `DbReader` used by the aggregator, which also exposes previous-run encounter data for cross-run hints. |
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
| `encounter.rs` | `EncounterTracker` — wild battle detection via personality change, balls-gate latch, duplicate species check, shiny detection (Gen III formula), catch detection via party membership |
| `game.rs` | `fill_party_list`, `check_for_dead_pokemon`, `check_for_new_pokemon`, `check_for_run_over`, `map_state_from_ewram`, `game_is_loaded`, `has_pokeballs`, `count_pokeballs`, `read_security_key`, `scan_for_balls_pocket`, `scan_for_security_key` |
| `textures.rs` | `PendingTexture`, sprite compression, `build_sprite_data` |
| `gui.rs` | `WindowInfo`, `eframe::App` impl, party panel, encounters viewport |
| `server.rs` | Aggregator connection handler — manages the bidirectional push stream over an established TCP connection |
| `webhook.rs` | `WebhookEvent` enum, channel-backed background sender, `init` / `fire_event` — HTTP POST dispatch for death, catch, shiny, and wipe events; `render_template` for `{placeholder}` substitution when a custom body template is configured |

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
| `rfd` | Native file-picker dialogs used in the first-run config setup wizard |
| `self_update` | GitHub release auto-updater; powers the `--update` flag on both binaries and the passive background version check that updates the window title when a newer release is available |
| `once_cell` | Lazy static initialisation for shared name buffers and ROM data |
| `reqwest` | Blocking HTTP client used by the tracker's webhook sender to POST event payloads; TLS via rustls |

---

### Memory access model

All live game data is read from two in-memory snapshots maintained by `fire_red_memory`:

- **EWRAM snapshot** (256 KiB) — refreshed every 100 ms by reading from RetroArch in parallel 4 KiB chunks over UDP, then assembled in address order.
- **IWRAM snapshot** (32 KiB) — refreshed on the same cycle.

Every other crate reads from these snapshots rather than issuing its own UDP requests. This eliminates hundreds of individual network round-trips per second and makes the read pattern predictable regardless of how many subsystems are running.

The map state is read directly from the EWRAM snapshot (at `0x02031DBC`) rather than through the `STATE` mutex in `fire_red_loop`. This avoids the cumulative lag of two polling intervals (~833 ms) and the race condition where `STATE` contains `(0, 0)` before the map thread has ticked for the first time.

---

### Wild encounter tracking

The tracker monitors `gEnemyParty[0]` at `0x0202402C` to detect wild battles. FireRed never clears this slot between battles — it is overwritten at the *start* of each new battle by `CreateWildMon`. Detection therefore uses personality *change* rather than presence/absence:

- When the personality value at `gEnemyParty[0]` changes, a new wild battle has started.
- Wild Pokémon receive the player's OT ID (via `CreateWildMon` → `OT_ID_PLAYER_ID`). Comparing the enemy's `ot_id` against the lead party member's `ot_id` distinguishes wild encounters from trainer battles.
- Only the **first encounter per map area** is recorded (Nuzlocke rule). Subsequent encounters in the same area are silently ignored.
- **Shiny detection** — `(p_high ^ p_low ^ id_high ^ id_low) < 8` is evaluated against the wild Pokémon's personality and OT ID at the moment of first-encounter recording. The result is stored in the `encounters` table and surfaced in the overlay.
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
├── memory thread             refresh EWRAM + IWRAM snapshots every 100 ms
├── game-polling thread       read map/party/encounters from snapshots every 100 ms
│                             EncounterTracker runs here (personality-change detection)
│                             fill_party_list called every tick; DB checks (deaths/catches)
│                             run every 1 s or immediately on party-size change
├── box-monitor thread        read all 14 PC boxes every 5 s
├── trainer-data thread       read trainer name / play time every 15 s
└── webhook thread            drains event channel; fires HTTP POSTs (5 s timeout each)
```

#### Tracker — connected

```
main thread  (headless, parked until Ctrl-C)
│
├── game-polling thread       (same as standalone)
│                             also signals wipe_signal AtomicBool on party wipe
│
└── network thread            outer reconnect loop (retry every 5 s on disconnect)
        └── on each connection: handle_client(stream, ...)
                ├── writer loop    push GameState snapshot every 100 ms;
                │                  sends RunChanged(None) when wipe_signal fires
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
| SaveBlock1 ptr | `0x03005008` | 4-byte IWRAM pointer; dereference for badge flag offset and bag pocket offsets |
| Balls pocket | `*0x03005008 + 0x0430` | 13 × 4-byte `ItemSlot` (item_id u16, quantity u16); quantities are XOR-encrypted with `security_key & 0xFFFF` (key at SaveBlock2+`0x0E4C`) |
| Security key | `0x02024298 + 0x0E4C` | u32 in SaveBlock2; lower 16 bits XOR each bag slot quantity. Use `--scan-security-key=<QTY>` to verify the offset on a different revision |
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
- `http://localhost:9090/0/party` and `http://localhost:9090/1/party` — per-player party panels (level cap highlight, status conditions, XP to next level)
- `http://localhost:9090/0/encounters` and `http://localhost:9090/1/encounters` — current area encounter table
- `http://localhost:9090/0/dead` and `http://localhost:9090/1/dead` — death logs
- `http://localhost:9090/0/routes` and `http://localhost:9090/1/routes` — route completion board
- `http://localhost:9090/0/alerts` and `http://localhost:9090/1/alerts` — transparent alerts overlay (add on top of everything else; invisible when idle)

Add `http://localhost:9090/cmd` in a browser tab to manage runs — **End Run** and **New Run** buttons apply to all connected trackers simultaneously. The party wipe detector in the alerts overlay also ends the run automatically.

---

## Project status

Personal project built for Nuzlocke and Soul Link runs. The codebase is functional but not hardened for general distribution:

- ROM scanning and all hardcoded addresses are calibrated for **FireRed USA (Rev 1)**. Other regional releases or ROM hacks will likely require address adjustments. Two scan tools are provided for this:
  - `tracker <rom> --scan-balls-pocket` — run with at least one ball in the bag to locate `BALLS_POCKET_SAVE_BLOCK_OFFSET` in `game.rs`. The scanner checks item IDs only; quantities are XOR-encrypted.
  - `tracker <rom> --scan-security-key=<QTY>` — run with exactly `QTY` balls in the bag to locate `SECURITY_KEY_OFFSET` in `game.rs`. The scanner finds all SaveBlock2-relative offsets where `raw_qty ^ offset_value == QTY` and prints the candidates for verification.
- Map area names (`map_area_name` in `fire_red_location_names`) are sourced from the FireRed/LeafGreen map groups document and cross-checked in-game for a subset of locations. All Kanto routes, major caves, and Sevii Island wild areas are covered. Individual dungeon floors (e.g. which floor of Rock Tunnel a given personality came from) have been confirmed for Diglett's Cave and inferred for the rest; verify with `READ_CORE_MEMORY 0x2031DBC 2` when entering each floor in-game.
- Ability data is read from ROM base-stat tables and is only reliable on unmodified ROMs.
- The `fire_red_map_data` crate is in progress and not yet integrated into the main loop.
- The `WildPokemonHeaderFFI` and `AreaEncountersStringArrays` FFI types are partially implemented; the C-callable interface helpers are in progress pending a stable API design.
