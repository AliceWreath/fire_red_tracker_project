# Fire Red Tracker

A real-time Pokémon FireRed party and encounter monitor built in Rust. It reads live game state from a running RetroArch instance, displays the player's current party and wild encounter table in a native GUI, and supports multi-player Soul Link / Nuzlocke runs through a networked aggregator mode.

---

## What it does

- **Party panel** — shows each Pokémon's sprite (shiny-aware), nickname, level, HP (colour-coded), experience, and caught location in real time.
- **Encounters panel** — shows the wild Pokémon available in the current area, split by type: grass, water/fishing, and Rock Smash.
- **Soul Link detection** — in aggregator mode, Pokémon caught in the same location across two or more players' games are automatically linked and labelled.
- **Clean ROM mode** — pass `--clean` to also display ability data (only reliable on unmodified ROMs).

---

## Modes

### Standalone
Runs locally. Reads the ROM and polls RetroArch on the same machine. Displays the GUI.

```
tracker firered.gba
tracker firered.gba --clean
```

### Server
Like standalone but also listens for remote client connections over TCP. Runs headless (no GUI). Streams party state and sprite data to any connected clients.

```
tracker firered.gba --server
tracker firered.gba --server 7878
```

### Client
Connects to a running server. Does not need the ROM — all data including sprites is received over the network. Displays the GUI.

```
tracker --client
tracker --client 192.168.1.10 7878
```

Default host is `127.0.0.1`, default port is `7878`.

### Aggregator
A separate binary for Soul Link / co-op runs. Connects to multiple tracker servers simultaneously and renders each player's data in a side-by-side column layout.

```
aggregator localhost:7878 localhost:7879
aggregator 192.168.1.10:7878 192.168.1.11:7878
```

The window width scales with the number of players. Soul Link matches (Pokémon sharing the same caught location across players) are highlighted in purple automatically.

---

## Soul Link / Nuzlocke context

In a **Nuzlocke** run, the player may only catch the first Pokémon encountered in each new area, and any Pokémon that faints is considered dead and must be released. The encounter panel makes it easy to see at a glance which Pokémon are available before stepping into grass.

A **Soul Link** is a Nuzlocke variant played with a partner: each player's catches are paired with their partner's catch from the same route. If one linked Pokémon faints, both must be released. The aggregator's Soul Link detection automates the pairing by comparing `met_location` values across all connected players' parties, removing the need to manually track which Pokémon are linked.

> **Limitation:** Soul Link matching uses `met_location` as the pairing key. This is reliable on standard FireRed but may produce false positives on heavily modified ROMs where multiple areas share a location ID.

---

## Architecture

### Workspace crates

| Crate | Role |
|---|---|
| `fire_red_loop` | Central coordinator. Owns the main map-polling loop, starts party/box monitors, and exposes the public API used by the GUI and network layers. |
| `fire_red_party_monitor` | Reads and decrypts the player's party from RetroArch memory. Owns `Party`, `Pokemon`, `BoxPokemon`, and all encrypted substructure types. Runs its own background poll loop. |
| `fire_red_box_monitor` | Reads all 14 PC boxes (420 slots) from EWRAM on a slow cycle. Maintains a deduplicated species cache and detects newly caught Pokémon. |
| `fire_red_image_data` | Extracts and decodes Pokémon front sprites from the ROM: pointer resolution → LZ77 decompression → 4bpp tile decode → BGR555 palette → RGBA image. |
| `fire_red_pokemon_data` | Wild encounter table types (`WildPokemonHeader`, `WildPokemonInfo`, `WildPokemon`). Parses encounter data from ROM and provides both safe Rust and FFI-compatible representations. |
| `fire_red_get_values` | Low-level byte parsing utilities. Three families: `get_*` for RetroArch hex-token buffers (LE), `read_*` for raw byte slices (LE), `read_*_raw` for raw byte slices (BE). |
| `fire_red_states` | Shared types and length-prefixed bincode TCP message protocol: `GameState`, `ServerMessage`, `ClientMessage`, `SpriteData`, `Mode`. Used by both server and client sides. |
| `fire_red_retroarch_interfacing` | Sends `READ_CORE_MEMORY` commands to RetroArch over UDP and parses the whitespace-tokenised responses. Owns the global shared `UdpSocket`. |
| `fire_red_rom_buffer` | Global ROM buffer. Loaded once from disk via `fill_rom` and shared as a `&'static [u8]` across all crates for the process lifetime. |
| `fire_red_scanner` | Scans the ROM binary with heuristic validation to locate the `WildMonHeader` table offset, which varies between ROM revisions. |
| `fire_red_text` | Decodes FireRed's custom GBA text encoding into UTF-8. Builds and caches the full Pokémon name table from ROM at startup. |
| `fire_red_pokemon_name_buffer` | Global Pokémon name repository. Initialised once from the decoded name table and shared as a `&'static [String]`. |

### Binaries

| Binary | Description |
|---|---|
| `tracker` | Standalone / server / client — all three modes in one binary, selected by CLI flags. |
| `aggregator` | Multi-player Soul Link viewer. Connects to N tracker servers and renders one column per player. |

### Key external dependencies

| Crate | Purpose |
|---|---|
| `eframe` / `egui` | Native GUI framework and immediate-mode UI rendering |
| `image` | `ImageBuffer<Rgba<u8>>` used for decoded sprite data |
| `flate2` | zlib compression/decompression for sprite data sent over TCP |
| `bincode` | Binary serialisation of `GameState`, `SpriteData`, and message enums |
| `serde` / `serde_big_array` | Derive macros for serialisable types; `BigArray` for fixed-size array fields in encrypted substructures |
| `arc-swap` | Lock-free `Arc` swapping for the party monitor's hot-path reads (`ArcSwap<Party>`) |
| `colored` | Terminal colour output for the server-mode startup banner |
| `ctrlc` | Ctrl-C signal handler for clean server shutdown |
| `libc` | `size_t`, `c_uchar`, `c_uint`, etc. for `#[repr(C)]` FFI structs |

---

### Thread model

#### Standalone / Server

```
main thread  (GUI or headless park)
│
├── game-polling thread       poll RetroArch every 333 ms, update FireRedState
├── party-monitor thread      read party on size-change + force-refresh every 1 s
├── box-monitor thread        read all 14 PC boxes every 5 s
│
└── [server mode] TCP listener thread
        └── per-client thread  (one spawned per accepted connection)
                ├── writer loop    push GameState snapshot every 100 ms
                └── reader thread  handle RequestTextures → reply with SpriteData
```

#### Client / Aggregator

```
main thread  (GUI)
│
└── [per server] client thread   outer reconnect loop (retry every 3 s)
        ├── writer thread   drain texture_request_queue every 50 ms
        └── reader loop     receive State + Textures, update shared Arcs
```

All inter-thread data flows through `Arc<Mutex<_>>` or `ArcSwap`. The GUI never holds a mutex during rendering — state is snapshotted at the start of each frame and all locks are released before drawing begins.

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

### Sprite pipeline

Sprites are decoded on first use and cached for the process lifetime:

1. Follow the two-level ROM pointer table (`FRONT_SPRITE_TABLE_PTR` at ROM offset `0x128`) to find the compressed sprite blob. Each table entry is 8 bytes wide; the sprite pointer occupies the first 4 bytes.
2. Decompress with GBA LZ77 (BIOS type `0x10`): 4-byte header (`0x10` + 24-bit LE decompressed size) followed by groups of 8 tokens controlled by a flag byte (bit 0 = literal, bit 1 = back-reference).
3. Decode 8×8-pixel tiles from 4bpp format (low nibble = left pixel, high nibble = right pixel) into a flat palette-index array. All sprites are 8×8 tiles = 64×64 pixels.
4. Resolve the 16-colour BGR555 palette to RGBA8. Each 5-bit channel is expanded to 8 bits with `(v5 << 3) | (v5 >> 2)`. Palette index 0 is always transparent (alpha 0).
5. Compress the raw RGBA pixels with zlib before sending over TCP (server → client).
6. Decompress on the client and upload to the GPU via `egui::Context::load_texture`.

Both normal and shiny variants are decoded and sent together when a species is first requested so shiny detection never triggers a second network round-trip. The server maintains a per-process sprite cache keyed by `(species, shiny)` so the ROM is decoded at most once per variant per session, even with multiple clients connected.

---

### Memory layout (FireRed USA Rev 1)

RetroArch memory reads use the `READ_CORE_MEMORY` UDP command (default port 55355). Key addresses:

| Symbol | Address | Notes |
|---|---|---|
| Party size | `0x02024029` | 1 byte, valid range 0–6 |
| Party data | `0x02024284` | Up to 6 × 100-byte `Pokemon` structs |
| PC box storage | `*0x03005010 + 0x4` | `SaveBlock3` pointer + offset; 14 × 30 × 80 bytes |
| Current map | `0x02031DBC` | 4 bytes: map group at +2, map name at +3 |
| WildMonHeaders | scanned at startup | Offset varies; `fire_red_scanner` locates it via heuristic validation |
| Ability names | `0x24FCB0` | 13 bytes per entry |
| Base stats | `0x2547F4` | 28 bytes per entry; ability slots at +`0x16` / +`0x17` |
| Pokémon names | `0x245F5B` | GBA-encoded, `0xFF`-terminated, up to species `0x019B` |

The PC box base address is not fixed — it is resolved at runtime by reading the `SaveBlock3` pointer at `0x03005010` and adding `0x4`. This indirection is necessary because the storage address can shift between saves.

---

### Network protocol

All TCP messages use a simple length-prefixed bincode frame:

```
[4-byte big-endian length][bincode-encoded message body]
```

Messages are defined in `fire_red_states`:

| Direction | Message | Contents |
|---|---|---|
| Server → Client | `ServerMessage::State` | Full `GameState` (party + encounter table), sent every 100 ms |
| Server → Client | `ServerMessage::Textures` | `Vec<SpriteData>` (zlib-compressed RGBA + metadata) |
| Client → Server | `ClientMessage::RequestTextures` | `Vec<u16>` of species IDs to fetch |

Maximum allowed message size is 20 MB, enforced on receive to prevent excessive memory allocation from malformed packets.

---

### Wild encounter header scanning

`fire_red_scanner` locates the `WildMonHeader` table by scanning the ROM in 4-byte-aligned increments. Each candidate offset is checked against these heuristics:

- 2-byte padding field is zero.
- Map group ≤ 50 and map number ≤ 200.
- All four encounter table pointers are either zero or fall within `[0x08000000, 0x09000000)`.

A candidate is confirmed only if scanning forward from it finds more than 50 consecutive valid headers followed by a `0xFF` sentinel byte. This threshold is tuned for FireRed USA and may not work on other regional releases.

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
- The FireRed ROM file must be accessible on disk for standalone and server modes. Client mode does not need the ROM.

### Quick start — solo Nuzlocke

```
./tracker firered.gba
```

### Quick start — Soul Link with a friend

**Player 1 (host machine):**
```
./tracker firered.gba --server 7878
```

**Player 2 (host machine):**
```
./tracker firered.gba --server 7879
```

**Aggregator (run on either machine or a third):**
```
./aggregator player1-ip:7878 player2-ip:7879
```

Each player can also run a local `--client` instance alongside the server if they want their own GUI view in addition to the shared aggregator.

---

## Project status

Personal project built for Nuzlocke and Soul Link runs. The codebase is functional but not hardened for general distribution:

- ROM scanning and all hardcoded addresses are calibrated for **FireRed USA (Rev 1)**. Other regional releases or ROM hacks will likely require address adjustments.
- The `--clean` ability feature reads from ROM base-stat tables and is only reliable on unmodified ROMs.
- The `WildPokemonHeaderFFI` and `AreaEncountersStringArrays` FFI types are partially implemented; the C-callable interface helpers are commented out pending a stable API design.