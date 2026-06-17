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
- **Type coverage panel** — displayed below the party list, shows three sets of Gen III type IDs for the living team: team types (blue), shared weaknesses (red), and offensive coverage gaps (grey). Types are read directly from the ROM base-stats table and computed using the full Gen III effectiveness table (17 types, eighths arithmetic, mono-type guard).

### Encounter tracking
- **Encounters panel** — shows the wild Pokémon available in the current map area, split by encounter type (grass, water/fishing, Rock Smash). Updates when the player moves to a new map.
- **First-encounter recording** — records the first wild Pokémon encountered per area per run (Nuzlocke rule). Encounters and deaths are not recorded until the player has obtained enough Pokéballs (default: 5, configurable via `run_start_balls` in `config.toml`); once that threshold is crossed the latch stays set for the remainder of the run. Duplicate species are skipped. Catches are detected automatically when the Pokémon joins the party.
- **Dupes clause** — optional Nuzlocke variant rule, configured via `dupes_clause` in `config.toml`. Three modes are available:
  - `"off"` *(default)* — standard Nuzlocke, first encounter per area, no species check.
  - `"per_player"` — per-player: a new encounter is skipped if *this player* has already caught the species anywhere in the current run.
  - `"shared"` — shared / cross-player: a new encounter is skipped if *any player* in the shared run has caught the species. Designed for Soul Link and co-op runs — one catch covers the whole group.
  
  Old boolean values (`true`/`false`) are still accepted and map to `"shared"` / `"off"` respectively for backward compatibility.
- **`allow_species_repeats`** — set `allow_species_repeats = true` in `config.toml` (or toggle in the setup wizard / Settings panel) to skip the global "already seen this species in the run" check. Each area still allows only one encounter entry, and the dupes clause still applies independently. Useful for randomized ROMs or variants where the same species legitimately appears on multiple routes.
- **`run_start_balls`** — optional integer (default `5`). Sets how many Pokéballs are required before the run-start latch triggers. Increase if your starter gift delays picking up the first balls.
- **`preset`** — optional shorthand that sets `dupes_clause` and `allow_species_repeats` together. Applied at load time; individual fields still win if set afterward in code.
  - `"standard"` — `dupes_clause = "off"`, `allow_species_repeats = false` (default behaviour)
  - `"hardcore"` — `dupes_clause = "per_player"`, `allow_species_repeats = false`
  - `"randomizer"` — `dupes_clause = "off"`, `allow_species_repeats = true`
  - `"soul_link"` — `dupes_clause = "shared"`, `allow_species_repeats = false`
- **Shiny detection** — the Gen III shiny formula (`p_high ^ p_low ^ id_high ^ id_low < 8`) is evaluated when an encounter is recorded. Shiny encounters are flagged in the database and trigger a shiny alert toast.
- **Route completion board** — a grid showing every Nuzlocke-relevant zone colour-coded as caught (green), failed/fled (red), or not yet visited (grey), grouped by region. Available at `/:index/routes`.

### Alerts overlay
A dedicated transparent OBS source (`/:index/alerts`) that shows timed toast notifications for all run-critical events:

- **Zone-entry** — fires when entering a wild area with no first encounter yet this run. Includes the zone name, a note that no encounter has been recorded, what was caught or fled in that zone on the most recently completed run, and a level-cap warning if any encounter in the zone matches or exceeds the next gym's cap.
- **Faint** — fires when a party member's HP reaches zero. Shows nickname, species, level, and nature.
- **Shiny encounter** — fires when a new encounter is recorded with the shiny flag set. Shows species, level, and zone name. Stays visible for 10 seconds.
- **Party wipe / blackout** — fires when every party member is dead simultaneously. The tracker detects the wipe and calls `end_run()` automatically; the overlay displays the "PARTY WIPED" banner in response to the resulting run-state change.
- **Injection command** — fires when an injection API endpoint is successfully executed. Shows a green "INJECTION" toast with a description of the command (e.g. "Gave 1× Rare Candy", "Made party[0] shiny"). Multiple rapid injections are queued and shown sequentially, 4 seconds each with a 0.4-second gap between toasts.

### OBS clip trigger

The tracker can automatically save the OBS replay buffer and/or switch OBS scenes on key events. Configure the optional `[obs]` section in `~/.config/fire_red_tracker/config.toml`:

```toml
[obs]
host           = "localhost"   # OBS WebSocket host (default: localhost)
port           = 4455          # OBS WebSocket port (default: 4455)
password       = "secret"      # omit if OBS authentication is disabled
clip_on_death  = true          # save replay buffer when a party member faints
clip_on_shiny  = true          # save replay buffer on shiny encounter
clip_on_wipe   = true          # save replay buffer on party wipe
clip_on_badge  = true          # save replay buffer when a gym badge is earned
scene_on_death = "Death Cam"   # switch to this OBS scene on death (optional)
scene_on_wipe  = "End Screen"  # switch to this OBS scene on wipe (optional)
scene_on_shiny = "Shiny Alert" # switch to this OBS scene on shiny encounter (optional)
scene_on_badge = "Badge Jingle"# switch to this OBS scene when a badge is earned (optional)
scene_on_catch = "Catch Cam"   # switch to this OBS scene when a Pokémon is caught (optional)
```

Clip flags default to `false`; scene fields default to absent (no switch). The `[obs]` section is omitted from the config file entirely when nothing is configured. Scene names must match OBS scene names exactly (case-sensitive).

Clips and scene switches are fired on the same background thread as webhooks. The tracker connects to OBS via plain TCP WebSocket (OBS WebSocket v5 protocol), authenticates with SHA-256 if a password is set, and sends a `SaveReplayBuffer` or `SetCurrentProgramScene` request. OBS must have **Replay Buffer** enabled and running (Tools → Replay Buffer → Start) for clip triggers to work. Connection errors are printed as warnings and do not interrupt the game-polling loop.

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
| `badge` | A gym badge flag transitions from 0 → 1 in SaveBlock1 | `badge_url` | `badge_template` |
| `nickname_change` | A party Pokémon's in-game nickname differs from the value stored in the database | `nickname_url` | `nickname_template` |
| `friendship_threshold` | A party member's friendship byte crosses 220 (evolution / happiness threshold) | *(no URL config)* | *(no URL config)* |

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
| `{badge.name}` | Badge name (e.g. `Boulder Badge`) | Only meaningful for `badge` events; empty string for all others |
| `{pokemon.old_name}` | Previous nickname before the rename | Only meaningful for `nickname_change` events |
| `{pokemon.new_name}` | New nickname after the rename | Only meaningful for `nickname_change` events |

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

#### Webhook HMAC signing

Set the optional `hmac_secret` key in `[webhooks]` to enable request signing. When set, every outbound POST includes an `X-Hub-Signature-256` header containing `sha256=<hex>` where `<hex>` is the HMAC-SHA256 of the raw request body keyed with the secret. Receiving services should verify this header before processing the payload.

```toml
[webhooks]
hmac_secret    = "mysecret"
death_url      = "https://example.com/webhook"
```

### Twitch chat bot

The aggregator can respond to viewer chat commands in a Twitch channel. Configure the optional `[twitch]` section in `~/.config/fire_red_aggregator/config.toml`:

```toml
[twitch]
channel = "mychannel"           # channel name without #
nick    = "my_bot_account"      # Twitch username for the bot account
token   = "oauth:xxxxxxxxxx"    # OAuth token — get one at twitchapps.com/tmi
# slot  = 0                     # which tracker slot to read (default: 0)
```

Supported viewer commands:

| Command      | Response                                                                         |
|--------------|----------------------------------------------------------------------------------|
| `!party`     | Current party members: nickname, species, level, HP                              |
| `!deaths`    | Death count and the five most recent deaths (requires DB)                        |
| `!shinies`   | Shiny count and last shiny name (requires DB)                                    |
| `!status`    | One-liner: `"Player — HP/MaxHP — Zone"`                                          |
| `!moves`     | Lead Pokémon's move set with current PP for each slot                            |
| `!ivs`       | Lead Pokémon's IVs for all six stats                                             |
| `!badges`    | Badge count and names earned so far (Boulder through Earth)                      |
| `!bag`       | Items and Pokéballs currently in the player's bag                                |
| `!map`       | Player's current map / location name                                             |
| `!encounter` | Current route's encounter table — species and percentage rates                   |
| `!luck`      | Shiny count / total encounters with expected-rate comparison (requires DB)       |
| `!timer`     | Elapsed HH:MM:SS from run start (requires DB)                                   |
| `!box`       | PC box Pokémon count with species list (requires DB)                             |
| `!run`       | Current run ID, caught count, and death count (requires DB)                      |

The bot reconnects automatically on disconnect, with exponential backoff from 2 seconds up to 60 seconds. The OAuth token is stored in plaintext in the TOML config — keep this file private. The `[twitch]` section works in both headless (`--ws-port`) and GUI modes.

#### Delivery mechanics

### Twitch Helix API integration

The tracker can call the Twitch Helix API to drop VOD markers, open polls, and create/resolve predictions on key events. Configure the optional `[twitch_helix]` section in `~/.config/fire_red_tracker/config.toml`:

```toml
[twitch_helix]
client_id       = "xxxx"           # Twitch application Client-ID
token           = "oauth:xxxx"     # OAuth token — get one at twitchapps.com/tmi (strips "oauth:" prefix automatically)
broadcaster_id  = "123456789"      # your numeric Twitch user ID

marker_on_death  = true    # drop a VOD marker when a party member faints
marker_on_shiny  = true    # drop a VOD marker on a shiny encounter
marker_on_badge  = true    # drop a VOD marker when a gym badge is earned

clip_on_death  = true      # create a Twitch clip when a party member faints
clip_on_shiny  = true      # create a Twitch clip on a shiny encounter
clip_on_badge  = true      # create a Twitch clip when a gym badge is earned

poll_on_legendary        = true   # auto-open a poll on legendary encounters ("Catch the Mewtwo?")
poll_duration_secs       = 60     # how long the poll stays open (default: 60)

prediction_on_legendary  = true   # open a Helix prediction on legendary encounters
prediction_window_secs   = 120    # prediction window before auto-cancel (default: 120)
```

Required OAuth scopes: `channel:manage:broadcast` (markers), `channel:manage:polls` (polls), `channel:manage:predictions` (predictions).

Helix calls are dispatched on a background thread and never block the game-polling loop. Predictions are resolved automatically: "Yes" (caught) when the Pokémon joins the party, "No" (escaped) when the party wipes. Legendaries checked: Articuno (144), Zapdos (145), Moltres (146), Mewtwo (150), Mew (151), Raikou (243), Entei (244), Suicune (245), Lugia (249), Ho-Oh (250), Celebi (251).

### Discord live status embed

The aggregator can maintain a pinned Discord message that shows the current party state for all connected trackers. Configure the optional `[discord_live_embed]` section in `~/.config/fire_red_aggregator/config.toml`:

```toml
[discord_live_embed]
bot_token            = "Bot xxxx"       # Discord bot token (prefix "Bot " is optional)
channel_id           = 123456789012345678
message_id           = 987654321098765432  # ID of the existing pinned message to edit
update_interval_secs = 30               # how often to push an update (minimum: 10)
```

The bot edits the pinned message with an embed showing each player's badge count and current party. Requires `Send Messages` + `Read Message History` permissions in the target channel. Create the initial message manually; the bot only edits, never creates.

### Discord run threads

The aggregator can open a dedicated Discord thread for each new Nuzlocke run and post badge milestone messages as the run progresses. Configure the optional `[discord_run_thread]` section:

```toml
[discord_run_thread]
bot_token  = "Bot xxxx"
channel_id = 123456789012345678   # forum or text channel where threads are created
```

A new thread named `"Run #N — PlayerName"` is created when a new run ID is detected. Subsequent badge events post milestone replies (e.g. "🏅 Badge #3 earned!") into the thread. Requires `Create Public Threads` permission.

### YouTube Live chat bot

The aggregator can poll a YouTube Live Chat and respond to viewer commands. Configure the optional `[youtube_chat]` section:

```toml
[youtube_chat]
api_key      = "AIza..."        # YouTube Data API v3 key
broadcast_id = "dQw4w9WgXcQ"   # YouTube video ID for the live broadcast
slot         = 0                # which tracker slot to read (default: 0)
poll_secs    = 5                # polling interval (default: 5)
```

Supported commands match the Twitch IRC bot: `!party`, `!deaths`, `!shinies`, `!status`. Responses are posted back to the live chat using the same API key (requires `youtube.force-ssl` OAuth scope if posting; read-only polling requires only an API key). The bot polls for new messages on a background thread with no impact on the game-polling loop.

---

The tracker starts a single long-lived background thread at startup that owns a `reqwest::blocking::Client` with a 5-second timeout. When `fire_event` is called from the game-polling loop it:

1. Looks up the URL and template for that event type.
2. If a template is configured, renders it by substituting all placeholders in a single pass over the string. For `wipe` events the five pokemon placeholders are replaced with empty strings.
3. Enqueues a `(url, body)` pair to the background thread via an `mpsc` channel and returns immediately — the polling loop is never blocked.
4. The background thread drains the channel and POSTs each payload. If a POST fails (network error, timeout, non-2xx response) the error is printed to stderr and delivery is not retried.

### Run management
- **Badge tracker** — displays obtained badges as coloured dots and shows the next gym leader's name, city, and highest level.
- **Badge events** — each newly earned badge is recorded in the event log and fires an optional webhook / OBS clip. A sentinel-based boot guard (`None` initial mask) prevents replaying already-earned badges on tracker startup and after a wipe/run-reset; the first call after any reset silently adopts the current badge state as the new baseline.
- **Nickname-change tracking** — when a Pokémon's in-game nickname changes, both the old and new names are written to the event log (`old_nickname` column) and an optional webhook is fired. If a transient DB error occurs during the read phase, the UPDATE is still attempted to keep the stored nickname in sync. The `old_nickname` field is included in all `/api/run/:id/events` and `/api/timeline` responses.
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
aggregator --no-injections       # disable all injection endpoints (give_item, change_species, set_ivs, heal_party, etc.)
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
| `http://localhost:PORT/0/party?plain-view` | Player 1's party — minimalist layout (sprites, nicknames, levels, HP only; no sidebar panels). Combine with `?plain-view&theme=X` for a themed variant. |
| `http://localhost:PORT/party/mobile` | Mobile-friendly party viewer — touch-optimised card layout with sprites, HP bars, badge dot row, and status badges. Works for all connected slots; slot is selected on the page. |
| `http://localhost:PORT/0/encounters` | Player 1's area encounters (or DB encounter log if no active run) |
| `http://localhost:PORT/0/dead` | Player 1's dead Pokémon log (requires `--db`) |
| `http://localhost:PORT/0/caught` | Player 1's caught Pokémon log (requires `--db`) |
| `http://localhost:PORT/0/box` | Player 1's PC box contents (all 14 boxes, sprite + IVs) |
| `http://localhost:PORT/1/party` | Player 2's party / run summary |
| `http://localhost:PORT/1/encounters` | Player 2's encounters / DB log |
| `http://localhost:PORT/1/dead` | Player 2's dead Pokémon log |
| `http://localhost:PORT/1/caught` | Player 2's caught Pokémon log |
| `http://localhost:PORT/1/box` | Player 2's PC box contents |
| `http://localhost:PORT/0/routes` | Player 1's route completion board — all Nuzlocke zones colour-coded caught / failed / unvisited, with encountered species shown inline on each zone card |
| `http://localhost:PORT/1/routes` | Player 2's route completion board |
| `http://localhost:PORT/0/alerts` | Player 1's alerts overlay — transparent OBS source for zone, death, shiny, wipe, and injection-command toasts |
| `http://localhost:PORT/1/alerts` | Player 2's alerts overlay |
| `http://localhost:PORT/0/deaths` | Player 1's death counter — large red number, ideal as a compact OBS Browser Source widget |
| `http://localhost:PORT/1/deaths` | Player 2's death counter |
| `http://localhost:PORT/0/encounter_count` | Player 1's encounter counter — total encounters this run with caught/missed breakdown |
| `http://localhost:PORT/1/encounter_count` | Player 2's encounter counter |
| `http://localhost:PORT/0/hp` | Player 1's party HP bars — sprite, nickname, level, status badge, HP text, and animated colour-coded bar (green >50 %, yellow >25 %, red ≤25 %) |
| `http://localhost:PORT/1/hp` | Player 2's party HP bars |
| `http://localhost:PORT/0/badges` | Player 1's badge progress — eight dots (gold = earned, dark = not yet) with count label |
| `http://localhost:PORT/1/badges` | Player 2's badge progress |
| `http://localhost:PORT/0/nextgym` | Player 1's next gym panel — upcoming leader name, city, level cap, type badge, and party members highlighted when they have a type advantage |
| `http://localhost:PORT/1/nextgym` | Player 2's next gym panel |
| `http://localhost:PORT/0/items` | Player 1's bag contents — tabbed view by pocket (Items, Key Items, Poké Balls, TMs/HMs) with quantities |
| `http://localhost:PORT/1/items` | Player 2's bag contents |
| `http://localhost:PORT/0/moves` | Player 1's party moves — transparent OBS overlay showing each Pokémon's four moves in a 2×2 grid with PP and type colour |
| `http://localhost:PORT/1/moves` | Player 2's party moves overlay |
| `http://localhost:PORT/timeline` | Run timeline — chronological event log (catches, deaths, badges, zone entries) for the active run as a vertical card list |
| `http://localhost:PORT/run/:id/timeline` | Run timeline for a specific run by ID |
| `http://localhost:PORT/trainers` | Trainer battle log — table of every trainer battle recorded for the active run, with trainer name, location, and timestamp |
| `http://localhost:PORT/run/:id/trainers` | Trainer battle log for a specific run by ID |
| `http://localhost:PORT/species` | Cross-run species survival stats — all species ever caught across all runs with caught count, death count, and survival rate bar |
| `http://localhost:PORT/history` | Run history — all past runs with expandable catch / death / encounter logs |
| `http://localhost:PORT/shiny` | Shiny odds tracker — encounter count since last shiny encounter, last shiny detail card, full encounter list since last shiny |
| `http://localhost:PORT/memorial` | Memorial grid — dead Pokémon from the active run as sprite cards with nickname, species, level, and death date |
| `http://localhost:PORT/run/:id/memorial` | Memorial grid for a specific run by ID |
| `http://localhost:PORT/run/:id/stats` | Per-run statistics — playtime, catch rate by zone, zone encounter log table, death log |
| `http://localhost:PORT/soullink` | Soul Link health overview — OBS Browser Source showing all active soul-link pairs side-by-side with sprites, HP bars, and live dead/alive state |
| `http://localhost:PORT/soullink/manage` | Soul Link override manager — set and clear manual pairings that take precedence over automatic met-location pairing (requires `--db`) |
| `http://localhost:PORT/:index/types` | Type coverage dashboard for one player — party type badges, per-type defensive exposure chart, next gym leader with their primary type highlighted, and Elite 4 progress track |
| `http://localhost:PORT/alerts` | Slot 0 alerts overlay — shorthand for `/0/alerts`; append `?slot=N` to target a different player |
| `http://localhost:PORT/about` | Version info and quick reference — tracker version, available pages, themes, and REST API summary |
| `http://localhost:PORT/join` | Direct mode join page — allows a remote player to connect their RetroArch instance to the aggregator without pre-configuration. Only available when direct mode is enabled (`--direct` flag or `direct_mode = true` in config). |
| `http://localhost:PORT/0/dex` | Player 1's Pokédex caught-species counter — species count for the active run, updated every 30 seconds |
| `http://localhost:PORT/1/dex` | Player 2's Pokédex caught-species counter |
| `http://localhost:PORT/0/typechart` | Gen III 17×17 type effectiveness chart — static overlay, no WebSocket needed |
| `http://localhost:PORT/1/typechart` | Same type chart for player 2's browser source slot |

The per-player pages can all be added as separate Browser Sources in OBS and positioned independently. The alerts overlay is fully transparent when idle — nothing appears until an event fires.

All overlay pages reconnect automatically if the aggregator restarts. The WebSocket client uses **exponential backoff**: first retry after 1 s, then 2 s, 4 s, 8 s, …, capped at 30 s. The delay resets to 1 s on every successful connection. OBS Browser Sources survive aggregator restarts without needing a manual refresh.

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
| `/ws` | `GET` (WS upgrade) | WebSocket stream. Sends the full state JSON immediately on connect, then pushes updates whenever state changes (~10×/s while playing, zero bandwidth when idle). Accepts `{ "cmd": "end_run" }` and `{ "cmd": "new_run" }` text frames from the client. Append `?show=<mode>` to strip unused payload fields (see below) |
| `/api/state` | `GET` | Full current state as a JSON array of slot objects — the same payload the WebSocket pushes. Each slot contains party, encounters, dead, caught, box, badges, run summary, and encounter-zone fields |
| `/api/slot/:index` | `GET` | Single slot object by zero-based index. Returns `404` if the index is out of range |
| `/api/command/end_run` | `POST` | Broadcasts `EndRun` to all connected tracker slots. Returns plain text: `"Command 'end_run' sent to N slot(s)"` |
| `/api/command/new_run` | `POST` | Broadcasts `NewRun` to all connected tracker slots. Returns plain text: `"Command 'new_run' sent to N slot(s)"` |
| `/api/command/heal_all` | `POST` | Heals HP/PP/status for all party Pokémon across all connected slots. Suitable as a Stream Deck button (no body required) |
| `/api/slot/:index/command/heal_party` | `POST` | Heals HP/PP/status for all party Pokémon for a single slot. Suitable as a Stream Deck button (no body required) |
| `/api/db/query` | `POST` | Runs arbitrary SQL against the database. Request body: `{ "sql": "SELECT ..." }`. Response: `{ "columns": ["col1", ...], "rows": [{ "col1": "val", ... }, ...], "rows_affected": N }` or `{ "error": "..." }` on failure. All values are returned as strings. Requires `--db` |
| `/api/run/:id/stats` | `GET` | Per-run statistics for run `id`. Returns `{ playtime_secs, zones_entered, caught, catch_rate, deaths, avg_death_level, zone_stats: [...], deaths: [...] }`. Requires `--db` |
| `/api/run/:id/route_stats` | `GET` | Per-route catch statistics for run `id`. Returns `{ run_id, zones: [{ map_group, map_name, area, total, caught, catch_rate_pct }] }`. Requires `--db` |
| `/api/run/:id/route_odds` | `GET` | Encounter coverage for run `id`. Returns `{ encountered: [...], unencountered: [...] }` — `encountered` has species/catch info per visited route; `unencountered` lists all known FireRed wild areas not yet recorded. Requires `--db` |
| `/api/run/:id/webhook_log` | `GET` | Webhook delivery receipts for run `id`. Returns `{ run_id, webhook_log: [{ event_type, url, success, attempts, payload, fired_at, fired_at_human }] }`. Requires `--db` |
| `/api/run/:id/soul_link/overrides` | `GET` | All manual soul-link overrides for run `id`. Returns `{ run_id, overrides: [{ personality, partner_personality, created_at }] }`. Requires `--db` |
| `/api/run/:id/soul_link/override` | `POST` | Set a manual soul-link pairing. Body: `{ "personality": <u32>, "partner_personality": <u32> }`. Replaces any existing override for the same personality. Requires `--db` |
| `/api/run/:id/soul_link/override/:personality` | `DELETE` | Remove the manual soul-link override for the given personality in run `id`. Requires `--db` |
| `/api/run/:id/shiny` | `GET` | Shiny encounter statistics for run `id`. Returns `{ total_shinies, encounters_since_last_shiny, last_shiny: {...}, since_last_shiny: [...] }`. Requires `--db` |
| `/api/run/:id/export` | `GET` | Full run export. Without query params: returns the complete run as JSON (metadata + caught + dead + encounters). With `?format=csv`: returns the same data as three CSV sections (caught, dead, encounters) in a single file with `Content-Disposition: attachment`. Requires `--db` |
| `/api/run/:id/summary` | `GET` | Markdown text recap. Includes a summary table (zones, caught, deaths, shinies), full death log, and encounter log with caught/shiny flags. Append `?format=text` for `text/plain`; omit for `{ "markdown": "..." }` JSON. Requires `--db` |
| `/api/run/:id/events` | `GET` | Chronological event log for a run. Returns `{ run_id, events: [{ player_name, event_type, species_name, nickname, old_nickname, level, occurred_at }, ...] }`. `old_nickname` is populated for `nickname_change` events and empty for all others. Event types: `catch`, `death`, `soul_link_death`, `shiny`, `wipe`, `badge`, `nickname_change`. Requires `--db` |
| `/api/timeline` | `GET` | Chronological event log for the currently active run. Convenience alias for `/api/run/:id/events` on the active run. Each event includes both `occurred_at` (Unix integer) and `occurred_at_human` (formatted string), plus `old_nickname` for `nickname_change` events. Returns `404` when no run is active, `503` when no database is configured, `500` on DB failure. Requires `--db` |
| `/db.json` | `GET` | Full database snapshot — all four tables (runs, caught, dead, encounters) formatted for the browser viewer. Requires `--db` |
| `/db/clear` | `POST` | Deletes all records from every table and removes the active-run meta key. No confirmation, no undo. Requires `--db` |
| `/api/runs` | `GET` | JSON array of all stored run summaries: `id`, `player`, `started_at`, `ended_at`, `deaths`, `catches`, `encounters`. Requires `--db` |
| `/api/run/import` | `POST` | Import a previously-exported run. Accepts the JSON body produced by `/api/run/:id/export`; creates a new run and re-inserts all encounter, caught, and dead records. Returns `{ "run_id": <new_id> }`. Requires `--db` |
| `/api/slot/:index/odds` | `GET` | Wild-encounter table for the specified slot's current map area — land, water, rock-smash, and fishing slots with encounter rates. Returns `404` if the slot index is out of range or the slot is disconnected |
| `/api/slot/:index/give_item` | `POST` | **Injection.** Body: `{ "item_id": <u16>, "quantity": <u16 1–99> }`. Add `quantity` of item `item_id` to the player's bag items pocket. Writes directly into the running game's EWRAM via RetroArch's `WRITE_CORE_MEMORY` UDP command. Returns `403` when `allow_injections` is false or `--no-injections` is set |
| `/api/slot/:index/take_item` | `POST` | **Injection.** Body: `{ "item_id": <u16>, "quantity": <u16 1–99> }`. Remove up to `quantity` of item `item_id` from the bag; fully removes the stack if smaller than `quantity`. Returns `403` when injections are disabled |
| `/api/slot/:index/make_shiny` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5> }`. Patch the OT Secret ID so the Gen III shiny formula is satisfied for the party member at `party_position`. Nature, ability, gender, and IVs are preserved. Returns `403` when injections are disabled |
| `/api/slot/:index/change_species` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "new_species": <u16 1–386> }`. Rewrite the species field in the Growth substructure; personality, moves, EVs, IVs, and nature are preserved and the checksum is recalculated. Returns `403` when injections are disabled |
| `/api/slot/:index/change_ability` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "ability_slot": <u8 0 or 1> }`. Set bit 31 of the IV/egg/ability word in the Misc substructure (`0` = primary ability, `1` = secondary ability). Returns `403` when injections are disabled |
| `/api/slot/:index/change_gender` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "target_gender": <u8 0 or 1> }`. Adjust the personality low byte to satisfy `target_gender` (`0` = male, `1` = female) while preserving nature and shiny status. Returns `200` with an explanatory message for genderless or fixed-gender species. Returns `403` when injections are disabled |
| `/api/slot/:index/change_nickname` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "nickname": <string> }`. Write a new nickname (max 10 chars, GBA encoding; unmapped characters are silently dropped). The encrypted data block is untouched. Returns `403` when injections are disabled |
| `/api/slot/:index/change_held_item` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "item_id": <u16> }`. Set the held item in the Growth substructure. `item_id = 0` removes the held item. Returns `403` when injections are disabled |
| `/api/slot/:index/cure_status` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5> }`. Zero the 4-byte status word (clears burn, sleep, paralysis, poison, freeze). Returns `403` when injections are disabled |
| `/api/slot/:index/change_nature` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "nature": <u8 0–24> }`. Adjust the personality low byte so `personality % 25 == nature`, preserving gender and shiny status. Substructures are rearranged when `personality % 24` changes. Returns `403` when injections are disabled |
| `/api/slot/:index/restore_pp` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5> }`. Restore PP on all four move slots to their current maximum (base PP + PP-Up bonus). Empty slots (move_id = 0) are skipped. Returns `403` when injections are disabled |
| `/api/slot/:index/set_friendship` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "friendship": <u8 0–255> }`. Set the friendship byte in the Growth substructure (0 = min, 255 = max). Returns `403` when injections are disabled |
| `/api/slot/:index/change_move` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "slot": <u8 0–3>, "move_id": <u16> }`. Replace a move slot and set current PP to the new move's maximum. `move_id = 0` clears the slot. Returns `403` when injections are disabled |
| `/api/slot/:index/set_ivs` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>, "spd": <u8>, "spa": <u8>, "spdef": <u8> }`. Set all six IVs (each clamped to 31). Egg and ability bits in the Misc substructure are preserved. Returns `403` when injections are disabled |
| `/api/slot/:index/increase_ivs` | `POST` | **Injection.** Same body as `set_ivs`. Add each value to the corresponding IV, clamping at 31. Returns `403` when injections are disabled |
| `/api/slot/:index/set_evs` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>, "spd": <u8>, "spa": <u8>, "spdef": <u8> }`. Set all six EVs (0–255 each; 510-total cap not enforced). Contest-condition bytes are preserved. Returns `403` when injections are disabled |
| `/api/slot/:index/increase_evs` | `POST` | **Injection.** Same body as `set_evs`. Add each value to the corresponding EV, clamping at 255. Returns `403` when injections are disabled |
| `/api/slot/:index/restore_hp` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5> }`. Write the calculated max-HP value (PartyPokemon offset 88–89) to current HP (offset 86–87). No encrypted block is touched. Returns `403` when injections are disabled |
| `/api/slot/:index/heal_party` | `POST` | **Injection.** No body required. Restore HP and cure status for all six party slots in one pass (single UDP socket). Returns `403` when injections are disabled |
| `/api/slot/:index/set_pp_ups` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "pp0": <u8 0–3>, "pp1": <u8 0–3>, "pp2": <u8 0–3>, "pp3": <u8 0–3> }`. Set PP-Up bonus for all four move slots (0 = none, 3 = max). Current PP is refilled to the new max for each occupied slot. Returns `403` when injections are disabled |
| `/api/slot/:index/revive_pokemon` | `POST` | **Injection.** Body: `{ "party_position": <u8 0–5>, "personality": <u32> }`. Look up a dead Pokémon by its personality value in the current run's death records, reconstruct the full 100-byte party struct (HP=1, status=0), and write it to the specified party slot. Returns `403` when injections are disabled |
| `/api/slot/:index/undo` | `POST` | **Injection.** No body required. Revert the most recent injection command by replaying the pre-write EWRAM bytes saved before it was applied. No-ops if no undo state is available. Returns `403` when injections are disabled |
| `/api/run/:id/trainers` | `GET` | Trainer battle log for run `id`. Returns `{ run_id, trainers: [{ flag_index, trainer_name, location, defeated_at }] }`. Requires `--db` |
| `/api/runs/compare` | `GET` | Compare multiple runs side by side. Query param `?ids=1,2,3`. Returns total encounters, catch count, death count, average death level, and duration for each run. Requires `--db` |
| `/api/run/:id/luck` | `GET` | Shiny luck statistics for run `id`. Returns `{ total_encounters, shiny_count, expected_shinies, per_area: [...] }`. Requires `--db` |
| `/api/catch_rate` | `GET` | Gen III catch rate calculator (no DB required). Query params: `?species=<u16>&hp=<u16>&max_hp=<u16>&status=<none\|sleep\|freeze\|poison\|paralysis\|burn>&ball=<pokeball\|greatball\|ultraball\|...>`. Returns `{ modified_catch_rate, guaranteed, catch_probability_pct }` |
| `/api/run/:id/closest_calls` | `GET` | Up to 50 party Pokémon ordered by the lowest HP/max_HP ratio they reached during the run, closest to fainting first. Requires `--db` |
| `/api/run/:id/event/:event_id/note` | `PATCH` | Add or update a text annotation on a run event. Body: `{ "note": "..." }`. The `event_id` is the `id` field returned by `/api/run/:id/events`. Requires `--db` |
| `/api/run/:id/event/:event_id/note` | `DELETE` | Clear the annotation on a run event (sets note to `""`). Requires `--db` |
| `/api/run/:id/pokepaste` | `GET` | Export the run's surviving and fallen Pokémon as Pokepaste-formatted text. Survivors include stats, IVs, and EVs; fallen include full data with moves. Returns `text/plain`. Requires `--db` |
| `/api/run/:id/splits` | `GET` | Badge split times for run `id`. Returns each badge with `badge_name`, `earned_at`, `elapsed_secs` (from run start), and `split_secs` (since previous badge). Requires `--db` |
| `/api/run/:id/catch_log` | `GET` | Catch attempt log for run `id`. Returns `{ total_balls_thrown, most_balls_in_one_encounter, hardest_encounter, attempts: [...] }`. Each attempt records species, area, balls thrown, and whether the Pokémon was caught. Requires `--db` |
| `/api/run/:id/difficulty` | `GET` | Composite run difficulty score 0–100 for run `id`. Components: death ratio (40 %), HP danger (30 %), catch miss rate (20 %), trainer load (10 %). Returns `{ difficulty, components, raw }`. Requires `--db` |
| `/api/run/:id/area_times` | `GET` | Per-route time breakdown for run `id`. Areas sorted by total time descending, each with `formatted` duration string and `visits` count. Requires `--db` |
| `/api/run/:id/death_map` | `GET` | Deaths grouped by area for run `id`. Returns `[{ "area": "...", "count": N }]` sorted descending. Areas recorded at death time; historical records without an area show as `"Unknown"`. Requires `--db` |
| `/api/run/:id/level_curve` | `GET` | Average party level at each badge milestone for run `id`. Returns `[{ "badge_index", "badge_name", "avg_level", "levels": [...], "occurred_at" }]`. Requires `--db` |
| `/api/run/:id/move_usage` | `GET` | Move use counts per Pokémon per slot for run `id`. Returns `[{ "personality", "move_slot", "move_name", "use_count" }]` sorted by `use_count` descending. Requires `--db` |
| `/api/run/:id/friendship` | `GET` | Friendship change history for run `id` grouped by Pokémon. Returns `[{ "personality", "nickname", "species_name", "history": [{ "friendship", "logged_at" }] }]`. Requires `--db` |
| `/api/slot/:index/ev_progress` | `GET` | Live EV totals for all party Pokémon in slot `index`. Returns per-mon `{ personality, nickname, species, hp, attack, defense, speed, sp_attack, sp_defense, total, remaining, *_capped, fully_trained }`. No DB required |
| `/api/run/:id/type_matchups` | `GET` | Move-type usage heatmap for run `id` — 17-type distribution of move types recorded across all `move_uses` entries. Requires `--db` |
| `/api/run/:id/vs/:ghost_id` | `GET` | Ghost-run comparison for run `id` against `ghost_id`. Returns badge-milestone timing and average party level at each badge for both runs side-by-side. Requires `--db` |
| `/api/slot/:index/shiny_pressure` | `GET` | Cumulative shiny pressure for the active run on slot `index` — encounters-without-shiny count per area. No DB required |
| `/api/run/:id/status_log` | `GET` | Status condition event log for run `id`. Returns `[{ personality, nickname, status_name, event (onset\|clear), occurred_at }]`. Requires `--db` |
| `/api/run/:id/dex` | `GET` | Pokédex progress for run `id`. Returns `{ count, species: [{ species_id, species_name }] }` — distinct species caught during the run. Requires `--db` |
| `/api/run/:id/share` | `POST` | Create a 24-hour public share token for run `id`. Returns `{ "token": "...", "expires_at": <unix> }`. Token is stored as a `meta` key. Requires `--db` |
| `/share/:token/state` | `GET` | Resolve a share token and return run stats for the linked run. Returns `404` when the token is expired or not found. No authentication required |
| `/api/config/reload` | `POST` | Re-read and validate the aggregator config file from disk. Returns `{ "ok": true, "path": "...", "db": bool }` or an error description. No live state is hot-swapped |
| `/api/webhook/donation` | `POST` | Donation trigger bridge. Body: `{ "type": "...", "amount": <f64>, "name": "..." }`. Broadcasts a `donation` WebSocket event to all overlay clients. Append `?heal_on_donation=true` to also queue `HealParty` for all connected slots. No DB required |
| `/api/savefile` | `POST` | Parse a raw Gen III `.sav` file (raw bytes, `Content-Type: application/octet-stream`). Returns `{ "player_name": "..." }` decoded from the save header. No DB required |

##### WebSocket payload filtering (`?show=`)

Pages that only need a subset of the state can append `?show=<mode>` to the `/ws` URL. The server strips unused top-level arrays from each push, reducing bandwidth:

| `?show=` value | Arrays stripped from payload |
|---|---|
| `box` | `party`, `encounters`, `caught`, `dead`, `db_encounters`, `prev_run_encounters` |
| `dead` | `encounters`, `box_pokemon`, `caught`, `prev_run_encounters` |
| `caught` | `encounters`, `box_pokemon`, `dead`, `prev_run_encounters` |
| `memorial` | `encounters`, `box_pokemon`, `caught`, `prev_run_encounters`, `db_encounters` |
| `soullink` | `encounters`, `box_pokemon`, `db_encounters`, `prev_run_encounters` |
| `types` | `encounters`, `box_pokemon`, `dead`, `caught`, `db_encounters`, `prev_run_encounters` |
| `deaths` | `party`, `encounters`, `box_pokemon`, `caught`, `db_encounters`, `prev_run_encounters` |
| `counter` | `party`, `encounters`, `box_pokemon`, `dead`, `prev_run_encounters` |
| *(omitted or unrecognised)* | No stripping — full payload |

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
| `next_gym` | object \| null | `{ leader, city, max_level, type_id }` for the next gym; `type_id` is the leader's primary Gen III type (0–16) |
| `party` | array | Up to 6 party member objects (see below) |
| `dead` | array | Dead Pokémon records for the active run, sorted newest first |
| `caught` | array | Caught Pokémon records for the active run, sorted newest first. Each record includes `personality` (raw GBA value — used by the soul-link override manager to identify specific Pokémon) and `dead` (true if this Pokémon has a death record or is a soul-link casualty) |
| `box_pokemon` | array | All Pokémon in PC boxes |
| `db_encounters` | array | First-encounter records for the active run |
| `prev_run_encounters` | array | First-encounter records from the most recently completed run (for cross-run hints) |
| `encounters` | array | Live wild encounter tables for the current map area, grouped by type (`Land`, `Water / Fishing`, `Rock Smash`) |
| `current_map_group` | number | EWRAM map group byte for the current position |
| `current_map_name` | number | EWRAM map name byte for the current position |
| `current_zone_name` | string | Human-readable name of the current wild-encounter zone, empty when not in a wild area |
| `e4_progress` | bool[5] | Elite 4 + Champion defeat flags in order: Lorelei, Bruno, Agatha, Lance, Blue. Present only when all 8 badges are held |
| `game_cleared` | bool | True when all 8 badges and all 5 Elite 4 members (including the Champion) have been defeated |

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
| `iv_hp` … `iv_spd` | number | Individual Values for each stat (0–31), in order HP / Atk / Def / Spe / SpA / SpD |
| `moves` | string[4] | Move names for each of the four move slots (empty string for unused slots) |
| `pp` | number[4] | Current PP for each move slot |
| `type1` | number | Gen III type ID for the species' first type (0 = Normal, …, 8 = Psychic, …, 16 = Dark) |
| `type2` | number | Gen III type ID for the second type; equals `type1` for mono-type species |

> **ROM paths with spaces** can be quoted: `tracker "My ROMs/fire red.gba"`

---

## Soul Link / Nuzlocke context

In a **Nuzlocke** run, the player may only catch the first Pokémon encountered in each new area, and any Pokémon that faints is considered dead and must be released. The encounter panel shows which Pokémon are available before stepping into grass. The first-encounter tracker records the area's encounter automatically and updates it to "caught" when the Pokémon joins the party. The route completion board gives an at-a-glance view of all zones in the run: which have been completed, which were failed, and which haven't been entered yet. The alerts overlay fires toasts for zone entries, faints, shiny encounters, and party wipes so nothing goes unnoticed — even when looking away from the game.

A **Soul Link** is a Nuzlocke variant played with a partner: each player's catches are paired with their partner's catch from the same route. If one linked Pokémon faints, both must be released. The aggregator automates this in two layers:

1. **Instant visual detection** — compares `met_location` across all connected players' live party data every 100 ms. The moment a Pokémon's HP reaches zero, its partner is shown as dead immediately, before the tracker has written anything to the database.

2. **Database propagation** (requires `--db`) — once the tracker writes a death record, the aggregator cross-references it against the caught table and writes a corresponding soul-link death record for the partner. This persists across sessions.

> **Limitation:** Soul Link matching uses `met_location` as the pairing key. This is reliable on standard FireRed but may produce false positives on heavily modified ROMs where multiple areas share a location ID.

> **Gift Pokémon:** Starters, Eevee, Lapras, and other gifted Pokémon (`met_location = 0`) cannot be paired by location. Instead they are paired **by order of receipt**: Player 1's first gift (the starter) links to Player 2's first gift, the second gift to the second, and so on. If one player has received more gifts than the other, the unmatched gifts are not linked. Death propagation uses the `caught_at` timestamp to determine order; the live GUI display uses party-slot order.

**Manual overrides** — if the automatic pairing links the wrong Pokémon (e.g. on a randomizer ROM where location IDs don't uniquely identify routes), visit `/soullink/manage` in a browser tab to set custom pairings per run. An override maps one personality value to another and takes priority over both met-location and receipt-order pairing in both the live overlay and DB propagation. Overrides are stored in the `soul_link_overrides` database table (schema v9) and are cleared automatically when a new run starts. To create a symmetric link (A's death kills B *and* B's death kills A), add both directions: A → B and B → A.

---

## Architecture

### Workspace crates

| Crate | Role |
|---|---|
| `fire_red_loop` | Central coordinator. Owns the main map-polling loop, starts party/box/trainer monitors, and exposes the public API used by the GUI and network layers. |
| `fire_red_memory` | Maintains full EWRAM and IWRAM snapshots via a sliding-window UDP reader (16 concurrent chunks, ~64 ms for EWRAM, ~16 ms for IWRAM). All other crates read from these snapshots rather than issuing individual UDP requests. |
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
| `game.rs` | `fill_party_list`, `check_for_dead_pokemon`, `check_for_new_pokemon`, `check_for_run_over`, `map_state_from_ewram`, `game_is_loaded`, `has_pokeballs`, `count_pokeballs`, `read_security_key`, `scan_for_balls_pocket`, `scan_for_security_key`; injection commands: `give_item`, `take_item`, `make_shiny`, `change_species`, `change_ability`, `change_gender`, `change_nickname`, `change_held_item`, `cure_status`, `change_nature`, `restore_pp`, `set_friendship`, `change_move`, `set_ivs`, `increase_ivs`, `set_evs`, `increase_evs`, `restore_hp`, `heal_party` |
| `textures.rs` | `PendingTexture`, sprite compression, `build_sprite_data` |
| `gui.rs` | `WindowInfo`, `eframe::App` impl, party panel, encounters viewport |
| `server.rs` | Aggregator connection handler — manages the bidirectional push stream over an established TCP connection |
| `webhook.rs` | `WebhookEvent` enum, channel-backed background sender, `init` / `fire_event` — HTTP POST dispatch for death, catch, shiny, wipe, badge, nickname-change, and NuzlockeViolation events; `render_template` for `{placeholder}` substitution; OBS WebSocket v5 clip trigger (`SaveReplayBuffer`) and scene switching (`SetCurrentProgramScene`) via plain TCP `tungstenite` with SHA-256 authentication; `notify-send` desktop notifications for death/shiny/wipe events |
| `helix.rs` | Twitch Helix API integration. Background worker thread firing stream markers (`POST /streams/markers`), polls (`POST /polls`), and predictions (`POST /predictions` + `PATCH` to resolve). Public entry points: `init`, `on_death`, `on_shiny_encounter`, `on_badge`, `on_legendary_encounter`, `resolve_prediction`, `is_legendary`. Active prediction ID and outcome IDs stored in a `Mutex<Option<ActivePrediction>>` so the create → resolve lifecycle spans multiple game events |
| `type_coverage.rs` | `TypeCoverage` struct and `compute` function — given a slice of `(type1, type2)` pairs for the living party, returns team types present, types the team is collectively weak to, and types the team can hit super-effectively. Uses the full Gen III 17-type effectiveness table (eighths arithmetic). |

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
| `tungstenite` | Plain TCP WebSocket client (no TLS, no default features) used by the OBS clip trigger in `webhook.rs` to speak the OBS WebSocket v5 protocol |
| `sha2` | SHA-256 used for OBS WebSocket authentication, webhook HMAC-SHA256 signing (`X-Hub-Signature-256`), and share-token generation in `fire_red_database` |

---

### Memory access model

All live game data is read from two in-memory snapshots maintained by `fire_red_memory`:

- **EWRAM snapshot** (256 KiB, 64 × 4 KiB chunks) — read with a sliding-window semaphore keeping 16 chunks in flight simultaneously. Results arrive over an mpsc channel and are assembled in address order. At ~16 ms per round-trip, EWRAM takes approximately 4 × 16 ms = 64 ms per refresh cycle.
- **IWRAM snapshot** (32 KiB, 8 × 4 KiB chunks) — read on a separate thread in parallel with EWRAM; fits in a single window (8 < 16) so it completes in ~16 ms. Each region stores its result independently the moment it finishes — IWRAM readers are not delayed by EWRAM.

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

The tracker config supports three optional sections: `[webhooks]` for HTTP event callbacks, `[obs]` for OBS replay buffer clips, and top-level keys for ROM path, database connection, and aggregator host/port. Sections are omitted from the file entirely when nothing in them is configured.

**Startup validation** — the tracker validates the config before the main loop starts. If the ROM path is missing or not readable, or if any webhook URL does not start with `http://` or `https://`, all errors are printed together and the process exits with a non-zero status. This prevents the tracker from running with a silently broken config.

**Structured logging** — both binaries use `tracing` for diagnostic output. Log level is set via the `RUST_LOG` environment variable (default: `info`). Examples:

```
RUST_LOG=debug ./tracker firered.gba       # verbose — logs every state change
RUST_LOG=warn  ./aggregator --ws-port 9090 # quiet  — warnings and errors only
```

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
- `http://localhost:9090/0/routes` and `http://localhost:9090/1/routes` — route completion board with species shown inline on each zone card
- `http://localhost:9090/0/alerts` and `http://localhost:9090/1/alerts` — transparent alerts overlay (add on top of everything else; invisible when idle)
- `http://localhost:9090/soullink` — Soul Link health overview (transparent OBS Browser Source showing live soul-link pairs with HP bars)
- `http://localhost:9090/shiny` — shiny odds tracker showing encounter count since the last shiny
- `http://localhost:9090/memorial` or `http://localhost:9090/run/:id/memorial` — memorial grid of dead Pokémon with sprites and nicknames
- `http://localhost:9090/run/:id/stats` — per-run statistics (catch rate, death log, playtime)

Add `http://localhost:9090/cmd` in a browser tab to manage runs — **End Run** and **New Run** buttons apply to all connected trackers simultaneously. The party wipe detector in the alerts overlay also ends the run automatically.

---

## Project status

**v0.9.54** — streaming integrations, analytics heatmaps, Pokédex overlay, share tokens, donation bridge:

- **Discord live status embed** — `[discord_live_embed]` in `~/.config/fire_red_aggregator/config.toml`. A background thread edits a pinned Discord message every N seconds with the current badge count and party for all connected trackers.
- **Discord run threads** — `[discord_run_thread]`. Creates a Discord thread per run and posts badge milestone messages (e.g. "🏅 Badge #3 earned!") as the run progresses.
- **YouTube Live chat bot** — `[youtube_chat]`. Polls the YouTube Live Chat API and responds to `!party`, `!deaths`, `!shinies`, `!status` using the same logic as the Twitch IRC bot.
- **Twitch clip triggers** — `clip_on_death`, `clip_on_shiny`, `clip_on_badge` in `[twitch_helix]`. Calls `POST /helix/clips` on the relevant event to create a VOD clip.
- **`!box` and `!run` Twitch / YouTube IRC commands** — `!box` lists PC box Pokémon count and species; `!run` returns the current run ID, caught count, and death count.
- **Webhook HMAC signing** — `hmac_secret` in `[webhooks]`. When set, every outbound webhook POST includes `X-Hub-Signature-256: sha256=<hex>` for receiver verification.
- **Donation trigger bridge** — `POST /api/webhook/donation`. Accepts `{ type, amount, name }` from a donation alert service, broadcasts a WS event to overlays, and optionally heals the party (`?heal_on_donation=true`).
- **Status condition log** — `GET /api/run/:id/status_log`. The game loop tracks SLP/PSN/BRN/FRZ/PAR/TOX onset/clear per Pokémon each tick and writes to a new `status_events` table (schema v23).
- **Type matchup heatmap** — `GET /api/run/:id/type_matchups`. Aggregates `move_uses` data into a 17-type distribution showing which move types the team relied on most.
- **Ghost-run comparison** — `GET /api/run/:id/vs/:ghost_id`. Compares badge-milestone timing and average party level curves for two runs side-by-side.
- **Cumulative shiny pressure** — `GET /api/slot/:index/shiny_pressure`. Per-area encounters-without-shiny count for the active run.
- **Pokédex REST endpoint** — `GET /api/run/:id/dex`. Returns the count and list of distinct species caught during the run.
- **Pokédex counter overlay** — `GET /:index/dex`. OBS Browser Source displaying the Pokédex caught-species count; auto-refreshes every 30 seconds via REST.
- **Type chart overlay** — `GET /:index/typechart`. Static Gen III 17×17 type effectiveness chart rendered in-browser; no WebSocket required.
- **Config hot-reload** — `POST /api/config/reload`. Validates the aggregator config file on disk and reports parsed field summary without requiring a restart.
- **Public share URL** — `POST /api/run/:id/share` issues a 24-hour token; `GET /share/:token/state` returns run stats with no authentication for sharing with viewers.
- **Savefile snapshot import** — `POST /api/savefile`. Parses a raw Gen III `.sav` file and returns the decoded player name.

---

**v0.9.53** — analytics: death heat map, level curve, move usage, friendship tracker, EV efficiency overlay:

- **`GET /api/run/:id/death_map`** — deaths grouped by the area they occurred in, sorted descending by count. Returns `[{ "area": "Route 1", "count": 3 }]`. The `area_name` field on `dead_pokemon` is populated at death time via `get_area_name()`; historical records default to an empty string shown as "Unknown".
- **`GET /api/run/:id/level_curve`** — average party level at each badge milestone. `check_for_new_badges` now logs a `party_snapshots` row whenever a badge is earned. Returns `[{ "badge_index": 0, "badge_name": "Boulder Badge", "avg_level": 14.2, "levels": [12,14,15], "occurred_at": 1748000000 }]`.
- **`GET /api/run/:id/move_usage`** — move use counts per Pokémon per slot, sorted by `use_count` descending. PP-delta detection in the main loop tick upserts a `move_uses` row when current PP drops below the previous value. Returns `[{ "personality": …, "move_slot": 0, "move_name": "Tackle", "use_count": 14 }]`.
- **`GET /api/run/:id/friendship`** — friendship change history grouped by Pokémon. The main loop appends a `friendship_log` row whenever a party member's friendship byte changes. When the value crosses 220 (evolution threshold), a `WebhookEvent::FriendshipThreshold` fires.
- **`GET /api/slot/:index/ev_progress`** — live EV totals read directly from party memory (no DB). Returns per-mon `hp`/`attack`/`defense`/`speed`/`sp_attack`/`sp_defense` EVs, `total`, `remaining` cap, per-stat `*_capped` booleans (≥ 252), and `fully_trained` (total ≥ 510).
- **Schema v19–v22** — `dead_pokemon.area_name` TEXT column (v19); `party_snapshots` table (v20); `move_uses` table with UNIQUE on `(run_id, player_name, personality, move_slot)` (v21); `friendship_log` table (v22).

---

**v0.9.24** — analytics endpoints and injection commands:

- **`POST /api/slot/:index/set_pp_ups`** — set PP-Up bonus for all four move slots (0–3 each). Current PP is refilled to the new maximum.
- **`POST /api/slot/:index/revive_pokemon`** — look up a dead Pokémon by personality, reconstruct the full 100-byte party struct (HP=1, status=0), and write it into any party slot.
- **`GET /api/runs/compare?ids=1,2,3`** — compare multiple runs side by side: total encounters, catch count, death count, average death level, and duration.
- **`GET /api/run/:id/luck`** — shiny statistics: total encounters, shiny count, expected shinies (count/8192), and per-area breakdown.
- **`GET /api/catch_rate`** — Gen III catch rate calculator (no DB needed). Accepts `?species`, `?hp`, `?max_hp`, `?status`, `?ball`. Returns modified catch rate, guaranteed flag, and probability percentage.
- **`GET /api/run/:id/closest_calls`** — up to 50 party Pokémon ranked by the lowest HP/max_HP ratio seen during the run (closest to fainting first).
- **Schema v12** — adds `min_hp_seen_hp` and `min_hp_seen_max_hp` columns to `caught_pokemon` and `dead_pokemon`. Updated on every game-loop tick for living party members.

---

**v0.9.22** — trainer battle log:

- **Trainer battle detection** — reads flags `0x100`–`0x3DF` from SaveBlock1 on every poll tick. Newly set flags (diffed against the previous snapshot) are recorded in the database with a flag index, trainer name, location, and timestamp. The detection adopts the current flag state silently on the first call after a game load or run reset (same boot-guard pattern as the badge mask).
- **`GET /api/run/:id/trainers`** — trainer battle log for a run as JSON `{ run_id, trainers: [{ flag_index, trainer_name, location, defeated_at }] }`. Also served as a rendered HTML page at `/trainers` and `/run/:id/trainers`. Requires `--db`.
- **Schema v11** — new `trainer_battles` table: `(run_id, player_name, flag_index, trainer_name, location, defeated_at)` with a UNIQUE constraint on `(run_id, player_name, flag_index)`.

---

**v0.9.20** — `heal_party` injection command:

- **`POST /api/slot/:index/heal_party`** — restore HP and cure the status condition for all six party slots in a single command. Reuses one UDP socket for the full pass; no request body needed.

---

**v0.9.19** — IV/EV injection commands and `restore_hp`:

- **`POST /api/slot/:index/set_ivs`** — set all six IVs at once (each clamped to 31). Egg and ability bits in the Misc substructure IV word are preserved.
- **`POST /api/slot/:index/increase_ivs`** — add to each IV, clamping at 31. Same body as `set_ivs`.
- **`POST /api/slot/:index/set_evs`** — set all six EVs (0–255 each; 510-total game cap not enforced). Contest-condition bytes in the Effort substructure are preserved.
- **`POST /api/slot/:index/increase_evs`** — add to each EV, clamping at 255. Same body as `set_evs`.
- **`POST /api/slot/:index/restore_hp`** — write max HP to current HP. Reads the calculated max-HP word at PartyPokemon offset 88–89 and writes it to offset 86–87; no encrypted block is touched.
- Added `move_data_addr` to `RomAddresses` (0x250C04 FireRed / 0x250BE0 LeafGreen) for ROM-based base-PP lookups.

---

**v0.9.18** — PP, friendship, and move-slot injection commands:

- **`POST /api/slot/:index/restore_pp`** — restore all four move slots to their current maximum PP (base PP + PP-Up bonus). ROM move table is used to look up base PP per move.
- **`POST /api/slot/:index/set_friendship`** — set the friendship byte in the Growth substructure (0 = min / max Frustration, 255 = max / Happiness evolutions trigger).
- **`POST /api/slot/:index/change_move`** — replace the move at a given slot (0–3) with a new move ID and set current PP to the new move's maximum. `move_id = 0` clears the slot.

---

**v0.9.17** — nickname, held-item, status, and nature injection commands:

- **`POST /api/slot/:index/change_nickname`** — write a new nickname (UTF-8 input, converted to GBA encoding; unmapped characters dropped, truncated at 10). Only the 10-byte unencrypted nickname field is written; shiny status and all encrypted data are untouched.
- **`POST /api/slot/:index/change_held_item`** — update the held-item field in the Growth substructure. `item_id = 0` removes the item.
- **`POST /api/slot/:index/cure_status`** — zero the 4-byte status word (clears burn, sleep turn counter, paralysis, poison, freeze, and Toxic stage in one write).
- **`POST /api/slot/:index/change_nature`** — adjust the personality low byte so `personality % 25 == nature` (0–24), preserving gender (for species with personality-derived gender) and shiny status. Substructures are physically rearranged when `personality % 24` changes.
- All four commands preserve shiny status; `change_nature` uses the same k16 low-byte search as `change_gender`.

---

**v0.9.16** — injection command API, alerts overlay injection toasts, `allow_injections` config/CLI flag:

- **Injection API** — six new `POST` endpoints write directly into the running game's EWRAM via RetroArch `WRITE_CORE_MEMORY`:
  - `POST /api/slot/:index/give_item` — add items to the bag items pocket (body: `{ item_id, quantity }`).
  - `POST /api/slot/:index/take_item` — remove items from the bag.
  - `POST /api/slot/:index/make_shiny` — patch OT Secret ID so the Gen III shiny formula is satisfied; preserves nature, ability, gender, and IVs.
  - `POST /api/slot/:index/change_species` — rewrite the Growth substructure species field; recalculates the checksum; preserves personality and all other data.
  - `POST /api/slot/:index/change_ability` — toggle bit 31 of the IV/egg/ability word in the Misc substructure (primary vs. secondary ability).
  - `POST /api/slot/:index/change_gender` — adjust the personality low byte to satisfy a target gender while preserving nature (`personality % 25`) and shiny status; returns `200` with an explanatory message for genderless or fixed-gender species.
  - All six commands are dispatched through the aggregator TCP connection as `ClientMessage` variants; the tracker applies them on the next EWRAM read cycle.
- **`allow_injections` config flag and `--no-injections` CLI flag** — set `allow_injections = false` in the aggregator config or pass `--no-injections` at startup to disable all six injection endpoints. All six return `403 Forbidden` when disabled. The egui Settings panel exposes a checkbox. `--no-injections` overrides `allow_injections = true` in the config file; `allow_injections = false` in the config cannot be overridden by omitting the flag.
- **Injection toasts in the alerts overlay** — `/:index/alerts` now shows a green "INJECTION" toast whenever an injection command completes. The label describes the action (e.g. "Gave 1× Rare Candy", "Made party[0] shiny"). Multiple rapid injections are queued and shown sequentially (4 s each, 0.4 s gap). Toast deduplication uses the `ev.at` unix-second timestamp from `injection_events` in `SlotDto`.

---

**v0.8.98** — type coverage overlay, E4/game-clear tracking, static species-type table:

- **`/:index/types` overlay page** — new browser-source page showing each party member's type badges, a per-type defensive exposure chart (worst incoming multiplier per attacking type across the whole party), the next gym leader / Elite 4 member with their primary type highlighted, and an Elite 4 defeat progress track once all 8 badges are held.
- **`type1` / `type2` in `MemberDto`** — Gen III type IDs (0–16) are now included in every party member object in `/api/state` and the WebSocket feed. The types page and any custom overlays can use these without a separate lookup.
- **`e4_progress` / `game_cleared` in `SlotDto`** — the slot object now includes `e4_progress: [bool; 5]` (Lorelei → Blue) and `game_cleared: bool` (true when all badges and all E4 + Champion are defeated), sourced from `BadgeState.e4` and `BadgeState.game_complete()`.
- **`type_id` in `GymDto`** — the next-gym object now includes `type_id: u8` (the leader's primary Gen III type), used by the types overlay to pre-highlight the relevant column in the defensive chart.
- **`fire_red_party_monitor::species_type_static`** — new ROM-free type lookup: a 252-entry compile-time table covering all 251 Kanto + Johto species. The aggregator uses this to populate type fields without needing to load the ROM.
- **Clippy clean** — nested `if let` chains in `build_party_dto` collapsed to `if let … && let …` per Rust 2024 style.
- **"Possible future features" updated** — removed "type-matchup warning overlay" (now implemented) and "multi-revision auto-detect" (already implemented via `detect_rom_revision` in `fire_red_rom_buffer`).

---

**v0.8.97** — correctness fixes, lock-scope improvements, and Soul Link partner override UI:

- **`mark_dead` false-positive return** — `mark_dead()` always returned `Ok(true)` even when `ON CONFLICT DO NOTHING` silently skipped the insert (the Pokémon was already recorded as dead). Now returns `Ok(n > 0)` so callers can distinguish a new record from a no-op.
- **`mark_caught` meaningful return value** — `mark_caught()` previously returned `()`. It now returns `bool` (`true` = new row inserted, `false` = already existed or error), gating `record_event(Catch)` and the catch webhook so neither fires for a duplicate catch.
- **`parse_timestamp` pre-epoch guard** — years before 1970 silently returned `Some(0)` (identical to 1970-01-01) because the `1970..year` loop is empty for `year ≤ 1969`. Added an explicit `if year < 1970 { return None; }` guard.
- **`has_encounter_for_any_floor` single round-trip** — the function previously ran N serial `SELECT EXISTS` queries (one per dungeon floor), each holding the DB mutex. Replaced with a single dynamically-built `SELECT EXISTS … OR …` query covering all floors in one round-trip.
- **Party mutex lock scope reduced** — `check_for_new_pokemon` and `check_for_dead_pokemon` held `Arc<Mutex<Vec<Pokemon>>>` across DB writes and webhook calls, blocking the game-polling thread. Both functions now snapshot the party (`.cloned().collect()`) before releasing the lock, so I/O runs without holding the mutex.
- **`wipe_signal` race fixed** — the TCP write loop used `wipe_signal.swap(false, AcqRel)` which consumed the flag before confirming delivery. A client disconnect mid-send silently lost the wipe notification. Changed to `load(Acquire)` + send + `store(false, Release)` so the flag is only cleared after successful delivery.
- **Sprite cache lock held during ROM decode** — the `RequestTextures` handler locked `sprite_cache` across `build_sprite_data` / `build_sprite_data_back`, blocking all other cache readers during potentially long ROM decompression. Changed to: check cache under a short lock, release, decode outside the lock, re-lock to insert.
- **`read_save_block1_ptr` helper extracted** — the 8-line SaveBlock1 pointer resolution sequence was duplicated across `game_is_loaded`, `count_pokeballs`, `scan_for_balls_pocket`, and `scan_for_security_key`. Extracted into a single `fn read_save_block1_ptr(iwram, ewram) -> Option<usize>` used by all four callers.
- **Soul Link partner override feature** — manual pairings that override the automatic `met_location` / receipt-order soul-link matching, for cases where the automatic pairing links the wrong Pokémon (randomizer ROMs, shared location IDs):
  - New `soul_link_overrides` table (schema v9): `(run_id, personality, partner_personality, created_at)`.
  - Global write functions `set_soul_link_override` / `clear_soul_link_override` in `fire_red_database`.
  - `DbReader::load_soul_link_overrides()` / `list_soul_link_overrides_json()` for read access.
  - Three new REST endpoints: `GET /api/run/:id/soul_link/overrides`, `POST /api/run/:id/soul_link/override`, `DELETE /api/run/:id/soul_link/override/:personality`.
  - `BroadcastLoop` caches the override map alongside the caught list; `propagate_soul_links` and `build_party_dto` consult it before falling through to the automatic pairing.
  - `/soullink/manage` web page: run selector, override table with remove buttons, add-override form, and a caught Pokémon reference grid (click a card to fill the personality field).
  - `CaughtMonDto` now includes `personality` and `dead` fields, surfaced in `/api/state` for the override manager and other API consumers.

---

**v0.8.96** — export/import IV/EV round-trip fix, webhook spawn safety, CSV error visibility, encounter import warnings:

- **`export_run` IV/EV data loss fix** — the JSON exporter (`/api/run/:id/export`) previously omitted all twelve IV and EV columns from its caught and dead Pokémon queries, and `import_run` hard-coded literal `0` for all twelve IV/EV DB slots. Any export→import round-trip silently zeroed every stat. Both sides are now fixed: `export_run` selects and emits `iv_hp … iv_spd` / `ev_hp … ev_spd`; `import_run` reads them from the JSON body (falling back to `0` for old exports that pre-date this fix).
- **`webhook::init` spawn-before-set ordering fix** — the global `STATE` (which holds the channel sender) was populated before the worker thread was confirmed alive. A spawn failure left an orphaned sender in global state; every subsequent `fire_event()` call silently discarded events via `let _ = tx.send(…)` on a disconnected channel. Fixed by attempting the spawn first and only calling `STATE.set()` on success. On failure both ends of the channel are dropped and `fire_event()` no-ops cleanly.
- **`export_run_csv` DB error visibility** — all three queries in `export_run_csv()` (caught, dead, encounters) used `.unwrap_or_default()`, returning a partial CSV with no log entry on any DB failure. Changed to `.unwrap_or_else(|e| { tracing::warn!(…); vec![] })` matching the pattern applied to `DbReader` methods in v0.8.95.
- **`import_run` encounter INSERT warnings** — the encounters insert loop used `let _ = client.execute(…)`, silently dropping both `Ok(0)` collisions and `Err` DB errors. Added `ON CONFLICT DO NOTHING` and a `match` block with `tracing::warn!` on collision and failure, consistent with the caught and dead sections updated in v0.8.95.

---

**v0.8.95** — CSV IV/EV columns, DB error visibility, import collision warnings, schema v8 index, test coverage:

- **CSV export IV/EV completeness** — `export_run_csv()` now includes all twelve IV and EV columns for both caught and dead Pokémon sections. Header and query updated; column order is `iv_hp,iv_atk,iv_def,iv_spe,iv_spa,iv_spd,ev_hp,ev_atk,ev_def,ev_spe,ev_spa,ev_spd` inserted before the timestamp. Prior CSV exports omitted this data entirely despite the DB having it.
- **`webhook_log(run_id)` index** — schema v8 adds `CREATE INDEX IF NOT EXISTS webhook_log_run_id_idx ON webhook_log(run_id)`. Without this, `/api/run/:id/webhook_log` did a full table scan on large instances.
- **Route odds `species` field** — `/api/run/:id/route_odds` encountered entries now include a numeric `species` field alongside `species_name`, so clients don't have to parse the name string to look up sprite data.
- **`DbReader::sync_player` double-lock fix** — the `run_id` mutex was acquired twice in sequence (read then write) with a gap between. Changed to a single lock scope: read old value, write new value, drop.
- **Webhook worker spawn error logged** — `webhook::init()` now uses `std::thread::Builder` and logs `tracing::error!` if the spawn fails (e.g. resource exhaustion). Previously the webhook system would silently not start.
- **`DbReader` query error visibility** — three `list_dead_with_records`, `list_encounters`, and `list_prev_run_encounters` methods changed from `.unwrap_or_default()` (silent empty on any DB error) to `.unwrap_or_else(|e| { tracing::warn!(...); vec![] })`. DB failures are now visible in the log.
- **`import_run` collision warning** — caught and dead Pokémon inserts now check the affected-row count; `Ok(0)` (personality conflict, row skipped) emits `tracing::warn!` identifying the personality and species. Previously silent data loss on duplicate import.
- **`EventKind::Badge` and `NicknameChange` tests** — four new unit tests covering `row_parts()` dispatch for the two previously-untested event variants; test count raised from 21 to 30.

---

**v0.8.94** — structured tracing, Result-returning DB writes, webhook delivery log, route coverage endpoint:

- **Structured `tracing` migration** — all `eprintln!` / `println!` diagnostic calls across 13 library crates have been replaced with structured `tracing::info!`, `tracing::warn!`, `tracing::error!`, and `tracing::debug!` macros. User-facing CLI output (update checker, `--list-runs` table, run ID lines) and `#[cfg(feature = "dev-tools")]` scan output are intentionally preserved as `println!`/`eprintln!`.
- **`mark_dead` / `record_event` / `record_encounter` return `Result`** — these three public DB functions now return `Result<bool, postgres::Error>` or `Result<(), postgres::Error>` instead of `bool`, surfacing database errors to call sites. All callers in `game.rs`, `encounter.rs`, and `app.rs` have been updated to log errors via `tracing` and continue gracefully.
- **Webhook delivery receipts** — every webhook POST outcome (success or final failure) is now recorded in a new `webhook_log` PostgreSQL table (schema v7). The background worker captures the event type, URL, serialized payload, attempt count, and success flag. A new `GET /api/run/:id/webhook_log` endpoint exposes the log as JSON for diagnostics and stream dashboards.
- **`GET /api/run/:id/route_odds`** — new endpoint returning `encountered` (routes already visited with species/catch info) and `unencountered` (all known FireRed wild areas not yet recorded for the run). Useful for seeing which Nuzlocke encounter slots are still open.
- **`fire_red_location_names::all_wild_areas()`** — new public function returning a static slice of `(map_group, map_name, area_name)` tuples for every FireRed area that can have wild encounters, used by `route_odds_json`.

---

**v0.8.93** — `fire_red_memory` sliding-window reads, independent region stores, 16 concurrent chunks:

- **Sliding-window chunk dispatch** (`fire_red_memory`) — replaced strict batch-based concurrency with a semaphore-driven sliding window (`Mutex<usize> + Condvar`). A new chunk thread is dispatched the moment any in-flight chunk finishes, keeping exactly `MAX_CONCURRENT_CHUNKS` active at all times. Previously, a single slow or retrying chunk stalled every other idle thread until the whole batch drained.
- **Independent region stores** (`fire_red_memory`) — EWRAM and IWRAM now store their results the moment each region thread finishes rather than waiting for both to complete. IWRAM (~1 round-trip, ~16 ms) is no longer delayed by EWRAM (~4 round-trips, ~64 ms).
- **`MAX_CONCURRENT_CHUNKS` raised to 16** (`fire_red_memory`) — reduces EWRAM from 8 batch rounds to 4 window rounds, cutting read time from ~128 ms to ~64 ms at nominal RetroArch latency.

---

**v0.8.92** — bug fixes, webhook backoff, configurable run-start threshold, Nuzlocke presets, per-route catch stats:

- **`MAX_CONCURRENT_CHUNKS` corrected** (`fire_red_memory`) — the constant was set to 32 despite the comment above it (and empirical testing) documenting 8 as the reliable ceiling. Fixed to 8, matching the comment. The mismatch caused RetroArch to drop responses under load, leading to stale reads and occasional retry storms.
- **Scan format fix** (`fire_red_tracker`, dev-tools only) — `scan_for_security_key`'s first loop printed `SaveBlock2+0x{:04X}` with `sb2_rel as usize`, which wraps badly for negative offsets (scan start is 0x200 bytes before SaveBlock2). Changed to `SaveBlock2{:+#06X}` with the signed `isize` value, matching the full-EWRAM fallback loop.
- **Instant arithmetic panics fixed** (`fire_red_aggregator`) — `SlotDbCache::new()` and `SlotCache::new()` initialised `last_refresh` using bare `-` subtraction on `Instant`, which panics if the process starts within 60 seconds of system boot. Both now use `.checked_sub(...).unwrap_or_else(Instant::now)`.
- **Placeholder validator dead check removed** (`fire_red_tracker`) — `find_unknown_placeholders` compared `candidate` against `"}}"` before the known-set check. Because the loop already skips `{{` continuations and only enters the branch when an opening `{` is found, `candidate` structurally can never equal `"}}"` at that point. The dead check is removed.
- **libpq key-value connection strings now pass through** (`fire_red_database`) — `initialize()` prepended `postgresql://` to any string not starting with a URI scheme, which mangled `host=localhost user=alice dbname=nuzlocke`-style strings. Strings containing `=` are now passed through unchanged, allowing both URI and key-value formats.
- **Webhook exponential backoff** (`fire_red_tracker`) — retries now wait 1 s then 2 s (was a flat 2 s for both pauses). The raw-body string is also pre-cloned before the retry loop instead of being cloned on every iteration.
- **Configurable run-start ball threshold** — `run_start_balls` (optional, default 5) can now be set in `config.toml` to change how many Pokéballs trigger the run-start latch. `encounter.rs` passes the value through to `game::has_pokeballs_threshold(n)`.
- **Nuzlocke rule presets** — a new `preset` key in `config.toml` sets `dupes_clause` and `allow_species_repeats` together: `"standard"` (off, false), `"hardcore"` (per_player, false), `"randomizer"` (off, true), `"soul_link"` (shared, false). Individual fields can still be set separately; the preset is applied at load time and does not overwrite explicit field values after that.
- **`GET /api/run/:id/route_stats`** — new endpoint returning per-area catch statistics for a completed or active run. Each zone entry includes `map_group`, `map_name`, `area` (human-readable name), `total` encounters, `caught` count, and `catch_rate_pct`.

---

**v0.8.91** — new features: randomizer mode, bot summary endpoint, run-compare page, HP bar, CSV export link:

- **`allow_species_repeats`** — new config flag (also exposed in the setup wizard and Settings panel) that skips the global "already encountered this species in the run" check. The per-area one-encounter rule and the dupes clause both still apply. The same species can now appear as a first encounter on multiple different routes.
- **`/api/bot/:index` endpoint** — plain-text one-liner returning `"<player> — <hp>/<max_hp> HP — <zone>"` for the given tracker slot. Suitable for Twitch/stream chat bots answering `!status` commands without parsing JSON.
- **`/compare` run-comparison page** — side-by-side stats for any two completed (or active) runs. Selects from a dropdown populated by `/api/runs`; pulls per-run stats from `/api/run/:id/stats`. Highlighted green/red cells indicate which run has the better value for each metric. Encounter and death logs are listed inline for each run.
- **HP bar in party overlay** — the `/:index/party` overlay now shows a colour-coded HP bar (green → yellow → red) below each party slot's HP text in both dark and light themes. Width transitions smoothly on update.
- **CSV download link in `/db`** — each row in the Runs table now has a `CSV` link that triggers a direct browser download of `/api/run/:id/export?format=csv` for that run.

---

### Possible future features

- **Overlay visual editor** — drag-and-drop config page in the web UI to position/resize overlay widgets without editing TOML.

---

**v0.9.27** — multi-player / social features: Twitch IRC chat bot, session summary export:

- **Twitch IRC chat bot** — optional `[twitch]` section in `~/.config/fire_red_aggregator/config.toml`. Fields: `channel` (without `#`), `nick` (bot account username), `token` (`oauth:xxx` — get one at twitchapps.com/tmi), optional `slot` (which tracker to read, default 0). The bot connects to `irc.chat.twitch.tv:6667` over plain TCP, authenticates, joins the channel, and responds to viewer commands: `!party` (party members with species, level, HP), `!deaths` (death count and most recent deaths), `!shinies` (shiny count and last shiny, requires DB), `!status` (one-liner zone/HP summary). Reconnects automatically on disconnect with exponential backoff. No new dependencies — plain TCP + standard IRC protocol.
- **`GET /api/run/:id/summary`** — Markdown text recap for a completed (or in-progress) run. Includes a summary table (zones visited, caught count, deaths, shinies), death log, and full encounter log with caught/missed/shiny flags. Append `?format=text` to receive `text/plain` (raw Markdown for download or clipboard); omit to receive `{ "markdown": "..." }` JSON. Returns `404` when the run is not found, `503` when no DB is configured. Requires `--db`.

---

**v0.9.33** — Twitch Helix API integration and IRC command additions:

- **Twitch stream markers** — new `[twitch_helix]` section in the tracker config. When `marker_on_death`, `marker_on_shiny`, or `marker_on_badge` is true, the tracker calls `POST /helix/streams/markers` to drop a labelled VOD marker. Dispatched on a background thread; never blocks the game-polling loop.
- **Twitch Polls on legendary encounters** — when `poll_on_legendary = true`, a Helix poll ("Catch the `<species>`?") opens automatically on Articuno / Zapdos / Moltres / Mewtwo / Mew and the Johto event legends.
- **Twitch Predictions on legendary encounters** — when `prediction_on_legendary = true`, a Helix prediction ("Will I catch the `<species>`?") opens on legendary encounter, resolves "Yes" (Caught it) when the Pokémon joins the party, and resolves "No" (Got away) on party wipe. Active prediction ID stored in a `Mutex<Option<ActivePrediction>>` to span the create → resolve lifecycle.
- **`!encounter` Twitch IRC command** — returns the current route's land encounter table with each species and slot-rate percentage. Names resolved via `fire_red_text::get_pokemon_name_by_number`.
- **`!luck` and `!timer` Twitch IRC commands** — `!luck` reports shiny count / total encounters and expected rate from DB; `!timer` reports elapsed HH:MM:SS from run start.

---

**v0.9.32** — three new OBS overlay pages completing all brainstormed overlay items:

- **`/:index/hp` overlay** — party HP bars Browser Source. Six rows: 40×40 sprite, nickname and level, status badge (SLP/TOX/PSN/BRN/FRZ/PAR), HP text, animated colour-coded bar (green >50 %, yellow >25 %, red ≤25 %). Subscribes to `?show=hp` WS filter.
- **`/:index/badges` overlay** — badge progress dot bar: eight dots (gold = earned, dark = not yet) with count label below (`X / 8`). Subscribes to `?show=badges` WS filter.
- **`/:index/nextgym` overlay** — next gym info panel: leader name, city, level cap, type badge (coloured by Gen III type). Party rows show each member's types; members with a type-advantage move get a green highlight and ✓ icon. Shows "Champion Defeated!" when `game_cleared` is true. Subscribes to `?show=nextgym` WS filter.

---

**v0.9.31** — analytics: badge splits, catch attempt log, difficulty score, per-route time breakdown:

- **Badge split times** — `GET /api/run/:id/splits`. Returns each badge with `badge_name`, `earned_at`, `elapsed_secs` (from run start), and `split_secs` (since the previous badge). Computed from the existing `events` table; no new schema needed.
- **Catch attempt log** — `GET /api/run/:id/catch_log`. `EncounterTracker` now snapshots the ball count at encounter start and records `balls_thrown` + `caught` when the encounter resolves. Includes `total_balls_thrown`, `most_balls_in_one_encounter`, and `hardest_encounter` summary fields.
- **Run difficulty score** — `GET /api/run/:id/difficulty`. Composite 0–100 score from death ratio (40 %), HP danger (30 %), catch miss rate (20 %), and trainer load (10 %). Returns the composite score, each component, and raw underlying counts.
- **Per-route time breakdown** — `GET /api/run/:id/area_times`. Main loop opens an `area_visits` row on every map transition and closes it on exit; aggregator sums time per area and returns results sorted by total time descending.
- **Schema v15** — new `catch_attempts` and `area_visits` tables.

---

**v0.9.30** — QoL tooling: event annotations, Nuzlocke violation webhook, Pokepaste export, desktop notifications:

- **Run event annotations** — `PATCH /api/run/:id/event/:event_id/note` (add/update) and `DELETE /api/run/:id/event/:event_id/note` (clear). The `note` column (schema v14) is included in all `/api/run/:id/events` and `/api/timeline` responses. The event `id` is the `SERIAL` primary key already present in the schema.
- **Nuzlocke violation webhook** — area-already-encountered, species-repeat, and dupes-clause hits all fire a `WebhookEvent::NuzlockeViolation` (URL key `nuzlocke_url`, template key `nuzlocke_template` with `{violation.message}` placeholder) in addition to the existing overlay toast.
- **Pokepaste export** — `GET /api/run/:id/pokepaste`. Returns surviving and fallen Pokémon formatted as Pokepaste text (`text/plain`). Survivors show stats, IVs, EVs; fallen include ability, held item, and moves.
- **Desktop push notifications** — `notify_on_death`, `notify_on_shiny`, `notify_on_wipe` boolean keys in `[webhooks]` config. When set, the tracker calls `notify-send` on the relevant events. Best-effort; errors logged at debug level and do not interrupt polling.

---

**v0.9.29** — Twitch chat commands, Channel Points EventSub, badge overlay toast:

- **Five new IRC commands** — `!moves` (lead Pokémon move set with current PP), `!ivs` (lead Pokémon IVs all six stats), `!badges` (count + names earned), `!bag` (items + balls pocket from ROM lookup), `!map` (current location name).
- **Channel Points EventSub** — new `eventsub.rs` module in the aggregator. Connects to `wss://eventsub.wss.twitch.tv/ws` and subscribes to channel-point redemptions via Helix API. Config fields added to `[twitch]`: `client_id`, `broadcaster_id`, `reward_commands` (map of reward-ID → command name). Supported commands: `heal_all`, `heal_party`, `new_run`, `end_run`. Requires `channel:read:redemptions` OAuth scope.
- **Badge-earned overlay toast** — `/:index/alerts` now shows a green toast when a badge flag transitions 0 → 1 (e.g. "Thunder Badge — 3/8 badges earned"). Stays visible for 8 seconds. Badge state tracked via `_prevBadges` array; resets on run start.

---

**v0.9.28** — injection undo:

- **`POST /api/slot/:index/undo`** — reverts the most recent injection command. Before every EWRAM write, the tracker saves the pre-write bytes into a thread-local `LAST_UNDO` buffer (`begin_undo()` + `push_undo(addr, bytes)`). The `undo` endpoint sends `ClientMessage::UndoLastCommand` (index 30) to replay those original bytes. Commands that require a pre-read (e.g. `cure_status`, `change_nickname`, `revive_pokemon`) now capture the old data before writing. Returns `403` when injections are disabled.

---

**v0.9.26** — overlay/streaming features: kill counter, encounter counter, OBS scene switching, Stream Deck commands:

- **`/:index/deaths` overlay** — compact death-counter Browser Source page: large red numeral on a dark transparent panel. Subscribes to `?show=deaths` WS filter so only the dead list is transferred; zero bandwidth overhead on the main overlay feed. Supports `?theme=` like all other overlay pages.
- **`/:index/encounter_count` overlay** — encounter counter Browser Source page: total encounter count with caught/missed breakdown for the active run. Subscribes to `?show=counter` WS filter (only `db_encounters` transferred).
- **OBS scene switching** — five new `Option<String>` fields in the `[obs]` config section: `scene_on_death`, `scene_on_wipe`, `scene_on_shiny`, `scene_on_badge`, `scene_on_catch`. When set, the tracker switches OBS to the named scene using `SetCurrentProgramScene` (OBS WebSocket v5) on the relevant event. Reuses the same connection/auth code as the existing replay-buffer clip trigger; errors are logged as warnings and do not interrupt polling.
- **`POST /api/command/heal_all`** — new global command that sends `HealParty` to all connected tracker slots. No request body required; suitable as a Stream Deck button.
- **`POST /api/slot/:index/command/:cmd`** — new per-slot command route for body-less Stream Deck buttons. Currently supports `heal_party` (heals HP/PP/status for the specified slot's party).

---

**v0.8.90** — code quality: dedup `LockOrRecover`, log silenced errors, fix field typo, doc/comment cleanup:

- **Removed duplicate `LockOrRecover` trait in `gui.rs`** — the trait was defined locally in `fire_red_tracker/src/gui.rs` and identically in `fire_red_states`. The local copy has been removed; `gui.rs` now imports the canonical version from `fire_red_states`.
- **Scanner comment corrected** — the comment above the four-pointer validation in `fire_red_scanner` said "At least one valid pointer" when the code (correctly) requires all four. Comment now matches the code.
- **Sprite decompression failures now logged** — `decompress_pixels` in the aggregator previously discarded zlib errors silently via `unwrap_or(0)`. It now calls `tracing::warn!` on failure so bad sprite data shows up in logs.
- **DB dump task failure now logged** — `serve_db_json` in `web.rs` previously swallowed the `JoinError` from the blocking task with `|_|`. The handler now calls `tracing::error!` before returning the fallback JSON.
- **`eframe::run_native` error surfaced** — the aggregator's `let _ = eframe::run_native(...)` now matches on `Err` and prints to stderr.
- **`land_mon_enounters_rom_ptr` → `land_mon_encounters_rom_ptr`** — the private `WildHeaderRom` field in `fire_red_pokemon_data` had a persistent typo ("enounters"). Renamed across all five use sites in the file.
- **Doc/comment typo sweep** — fixed "tokes", "teh", "signel", "decrompressed", "shinty", "nmame", "intialized", "mpa/sotred", "strucct", "falg", "vallues", "destinatino" across `fire_red_get_values`, `fire_red_image_data`, `fire_red_text`, `fire_red_rom_buffer`, `fire_red_map_data`, and `fire_red_scanner`.

---

**v0.8.89** — bug fixes: timeline endpoint, typed errors, badge sentinel, schema cleanup:

- **`/api/timeline` endpoint fixed** — the aggregator process never initialises the global DB singleton used by the previous implementation, causing a panic (HTTP 500) on every request. The function now opens its own connection and reads `active_run_id` from the `meta` table directly, eliminating the singleton dependency.
- **Typed `EventsError`** — `list_events_json` and `active_run_timeline_json` now return `Result<_, EventsError>` instead of embedding error strings in JSON. The web handlers (`api_active_timeline`, `api_run_events`) match on enum variants for `404 / 500 / 503` status codes; no more fragile string comparison against `"no active run"`.
- **`api_run_events` proper status codes** — the `/api/run/:id/events` handler previously always returned HTTP 200 even on DB failure. It now returns `500` on connection or query errors.
- **Badge sentinel `Option<u8>`** — `check_for_new_badges` parameter and return type changed from `u8` (magic `u8::MAX` sentinel) to `Option<u8>` (`None` = uninitialized). All `last_badge_mask` variables in `main.rs` updated accordingly; startup `handle_party_events` return value is now captured so a wipe at boot correctly resets the mask.
- **`CREATE TABLE events` includes `old_nickname`** — the column was previously added only via `ALTER TABLE`, so fresh-install schema and the table definition were out of sync. The column is now declared directly in `CREATE TABLE`; the `ALTER TABLE` guard is retained for backwards compatibility with pre-v6 databases.
- **Schema version bumped to 6** — `SCHEMA_VERSION` updated from `"5"` to `"6"`.
- **Test precision** — `dark_covers_psychic_in_compute` now asserts `EFFECTIVENESS[16][13] == 16` (the exact corrected cell) rather than the unrelated `EFFECTIVENESS[13][16]` (Psychic→Dark immunity).
- **`update_caught_nickname` comment corrected** — the fallback UPDATE path on a broken client is now documented honestly: the execute will also fail silently if the client is in an error state.

**v0.8.88** — level cap warnings, badge events, type coverage panel, nickname tracking, timeline API:

- **Level cap warnings** — each party member displays an orange "⚠ OVER CAP" label when its level is at or above the next gym leader's maximum Pokémon level. Works in both the standalone tracker GUI and the aggregator window / overlay.
- **Badge-earned events** — each newly earned badge fires an event log entry, an optional webhook (`badge_url` / `badge_template`), and an optional OBS replay-buffer save (`clip_on_badge`). A startup boot guard prevents replaying badges that were already earned before the tracker was launched.
- **Nickname-change tracking** — when the in-game nickname of a caught Pokémon changes, the old and new names are recorded in the event log and an optional webhook is fired (`nickname_url` / `nickname_template`). Detection is atomic: the DB query reads the old name and updates in a single round-trip, returning `Some(old_name)` only on an actual change.
- **Type coverage panel** — rendered below the live party in both GUI modes. Shows three colour-coded lists: types the team has (blue), types the team is collectively weak to (red), and types the team cannot hit super-effectively (grey). Computed from the ROM base-stats table using the full Gen III 17-type effectiveness chart.
- **`/api/timeline` endpoint** — convenience REST endpoint that returns the chronological event log for the currently active run without needing to know the run ID. Each entry includes both a Unix timestamp and a human-readable date string. Returns proper HTTP status codes: `404` for no active run, `503` for no database, `500` for query failure.
- **Clippy clean** — all new code passes `cargo clippy --all-targets` with no warnings. Uses stabilised let-chain syntax (`if let … && cond`), iterator enumeration over index loops, and `?` instead of match-on-Option-return patterns.
- **Bug fixes** — three incorrect Gen III type-effectiveness table entries corrected (Dark→Psychic was ×0, should be ×2; Flying→Ground was ×0, should be ×1; Flying→Electric was ×2, should be ×½); badge boot-guard sentinel changed from `0` to `u8::MAX` so the first badge earned on a fresh run is no longer swallowed; badge mask now resets on wipe and game-unload in addition to run-change; `update_caught_nickname` DB read errors now fall back to a best-effort UPDATE instead of silently aborting the write; `old_nickname` column added to the `events` table so rename history is fully preserved.

**v0.8.85** — full config coverage in setup wizard and settings panel:

- **Setup wizard (`--config-editor`) now covers all config fields** — previously `poll_ms`, `obs`, `clean`, `default_test`, and the `[test]` overrides were silently dropped when saving through the wizard. All fields are now editable: poll interval (20–2000 ms), dupes clause mode, clean-start toggle, OBS clip trigger (host / port / password / per-event checkboxes), test-mode toggle, and all four `[test]` override fields (DB, aggregator host/port, player number). The window is now scrollable to accommodate the extra sections.
- **In-app ⚙ Settings panel similarly expanded** — previously saving through the settings panel clobbered `clean`, `poll_ms`, `obs`, `dupes_clause`, and `[test]` overrides with their defaults. The panel now reads and writes all of these alongside the existing ROM / DB / mode / webhook fields. Also scrollable.
- **`[test]` section is now editable via GUI** — test-mode DB, aggregator host/port, and preferred-player overrides can be set from both the wizard and the settings panel. Empty fields are interpreted as "use main config value" and serialised as `None` (omitted from TOML).

**v0.8.84** — shared dupes clause and clippy fix:

- **Shared dupes clause** — `dupes_clause` in `config.toml` is now a three-way mode instead of a boolean. `"off"` (default) disables the check; `"per_player"` skips an encounter if *this* player has already caught the species; `"shared"` skips the encounter if *any* player in the shared run has caught the species, designed for Soul Link and co-op runs where one catch covers the whole group. Old boolean values are still accepted: `true` maps to `"shared"`, `false` to `"off"`.
- **`clippy::items_after_test_module` resolved** — `is_shiny` in `fire_red_states` was defined after the `base64_tests` test module; moved to before it.

**v0.8.83** — third correctness pass (aggregator + database):

- **Gift Pokémon soul-link propagation works in BroadcastLoop** — the headless WebSocket server path (`propagate_soul_links`) skipped gift Pokémon entirely (`if met_loc == 0 { continue }`), so starters and other gifts were never marked as soul-link-dead in the DB or in the live overlay when running without the egui window. Both the DB propagation loop and the live detection loop now pair gifts by receipt order (caught_at), matching the egui path.
- **`parse_timestamp` rejects day > days-in-month** — e.g. "2025-02-30" or "2025-04-31" previously produced a silently-wrong timestamp by advancing past the end of the month. The function now returns `None` for any day exceeding the actual length of that month (accounting for leap years on February).
- **Shared `sort_gifts_by_caught_at` helper** — the gift-pre-sort pattern (`filter met_location==0` → `sort_by caught_at`) was duplicated across `soul_link_kill_candidates`, `update()`, and `propagate_soul_links`. It is now a single `pub(crate)` function used in all three sites, ensuring consistent ordering.
- **Removed redundant DB guard after `mark_soul_link_dead`** — the `&& let Some(db) = &self.slots[j].db` re-check in the `Some(true)` branch was unreachable: the DB must be `Some` for `mark_soul_link_dead` to have returned `Some`. Replaced with `.expect()` so any future regression fails loudly.
- **Eliminated per-frame `CaughtPokemon` clone** — `soul_link_kill_candidates` previously accepted `&[Vec<CaughtPokemon>]`, forcing the caller to clone the entire caught list every frame. It now accepts `&[&[CaughtPokemon]]` and the caller passes slice references, avoiding the allocation.

**v0.8.82** — second bug-fix pass (aggregator + database):

- **Soul-link propagation no longer retries every frame after restart** — `mark_soul_link_dead` previously returned `false` for both "run ID not yet known" (retry) and "row already existed in DB" (skip). These are now distinguished via `Option<bool>`: `Some(true)` = newly inserted (fire event), `Some(false)` = already existed (mark propagated, no duplicate event), `None` = retry next frame. Previously, every soul-link death from a prior session would trigger one wasted DB write per frame indefinitely after an aggregator restart.
- **`parse_timestamp` rejects invalid calendar fields** — day 0, month 0 or >12, hour >23, minute/second >59 now return `None` instead of silently producing a wrong timestamp or underflowing (day=0 previously wrapped to `u32::MAX`).
- **Live soul-link gift detection pre-sorts once per slot** — in `update()`, the gifts vector for each partner slot was rebuilt and re-sorted for every `(dead_pokemon, j)` pair in the inner loop. It is now pre-sorted once per slot before the loop, matching the existing optimization in `soul_link_kill_candidates`. `gift_catch_index` (now unused) has been removed.
- **`event_type_str` test helper deduplicates against `row_parts`** — the function previously repeated the same five match arms already in `EventKind::row_parts()`. It now delegates to `row_parts().0`, eliminating the duplicate.

**v0.8.81** — bug-fix and correctness pass (aggregator + database):

- **`/api/run/import` data loss fixed** — all caught and dead Pokémon were silently discarded on import except the first, because every row was assigned the run ID as its personality value. Import now reads the original `personality` from the export JSON (preserved by `export_run` since this release); old exports without the field fall back to safe synthetic values.
- **Soul-link deaths now visible per-player** — `mark_soul_link_dead` was inserting rows without `player_name`, causing `list_dead_with_records` to never find them. The column is now populated from the caught record.
- **`is_soul_link_death` preserved on JSON export/import round-trip** — `export_run` now includes the flag in the dead-Pokémon list; `import_run` reads it under the correct key (`is_soul_link_death`, with fallback to the old `soul_link` key for existing exports).
- **Gift Pokémon soul-link pairing consistent across live UI and DB** — the live overlay was pairing gift Pokémon by party-slot order while the DB-kill path used caught-at timestamp order, which could mark the wrong partner dead. Both paths now use caught-at order.
- **No spurious soul-link events on reconnect** — `mark_soul_link_dead` now returns `true` only when a row is actually inserted (not on `ON CONFLICT DO NOTHING` or on DB error), preventing duplicate event log entries after an aggregator restart.
- **Original timestamps preserved on import** — `import_run` now parses `caught_at`, `died_at`, `encountered_at`, `started_at`, and `ended_at` from the export JSON using the new `parse_timestamp` inverse of `format_timestamp`, rather than stamping everything with the import time.
- **`record_event` dispatch deduplicated** — `EventKind::row_parts()` centralises the variant-to-columns mapping shared between the global and `DbReader` versions.
- **`require_db!` macro** consolidates the six identical DB-guard blocks across web API handlers.

**v0.8.80** — robustness pass: removed `unwrap()` panics in the webhook template renderer, party-monitor encryption helper, and bag-scan dev tool; tightened port-0 validation in the setup dialog; improved the species lookup error message in `fire_red_text`.

Personal project built for Nuzlocke and Soul Link runs. The codebase is functional but not hardened for general distribution:

- ROM scanning and all hardcoded addresses are calibrated for **FireRed USA (Rev 1)**. **LeafGreen** (`BPGE` game code) is detected automatically — runtime EWRAM/SaveBlock addresses are shared between the two games, so party, badge, and encounter data read correctly; ROM table addresses (base stats, Pokémon names, sprite pointers) are currently placeholders and will return incorrect data for LeafGreen. Other regional releases or ROM hacks will likely require address adjustments. Two scan tools are provided for this:
  - `tracker <rom> --scan-balls-pocket` — run with at least one ball in the bag to locate `BALLS_POCKET_SAVE_BLOCK_OFFSET` in `game.rs`. The scanner checks item IDs only; quantities are XOR-encrypted.
  - `tracker <rom> --scan-security-key=<QTY>` — run with exactly `QTY` balls in the bag to locate `SECURITY_KEY_OFFSET` in `game.rs`. The scanner finds all SaveBlock2-relative offsets where `raw_qty ^ offset_value == QTY` and prints the candidates for verification.
- Map area names (`map_area_name` in `fire_red_location_names`) are sourced from the FireRed/LeafGreen map groups document and cross-checked in-game for a subset of locations. All Kanto routes, major caves, and Sevii Island wild areas are covered. Individual dungeon floors (e.g. which floor of Rock Tunnel a given personality came from) have been confirmed for Diglett's Cave and inferred for the rest; verify with `READ_CORE_MEMORY 0x2031DBC 2` when entering each floor in-game.
- Ability data is read from ROM base-stat tables and is only reliable on unmodified ROMs.
- The `fire_red_map_data` crate is in progress and not yet integrated into the main loop.
- The `WildPokemonHeaderFFI` and `AreaEncountersStringArrays` FFI types are partially implemented; the C-callable interface helpers are in progress pending a stable API design.
