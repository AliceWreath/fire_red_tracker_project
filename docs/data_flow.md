# Data Flow

## End-to-End: RetroArch → Overlay

```
RetroArch (GBA core)
  │
  │  UDP :55355
  │  Command:  "READ_CORE_MEMORY 0x02000000 4096"
  │  Response: "READ_CORE_MEMORY 0x02000000 de ad be ef ..."
  │
  ▼
fire_red_memory::read_chunk()
  • Sends UDP command; retries up to MAX_RETRIES=5 (backoff: 50ms × retry)
  • Validates response header token and address match
  • Rejects chunks with malformed hex
  • EWRAM (256 KiB, 64 chunks of 4096 bytes) and IWRAM (32 KiB, 8 chunks)
    read concurrently on separate threads; each region stored independently
    as soon as it completes (IWRAM ~16 ms, EWRAM ~64 ms at 16 concurrent)
  • Sliding-window semaphore keeps exactly MAX_CONCURRENT_CHUNKS=16 in
    flight at all times — new chunk dispatched the moment any slot frees
  │
  │  ArcSwap<Vec<u8>>  (lock-free snapshot swap)
  │
  ▼
Game-polling thread (100 ms)
  │
  ├─ map_state_from_ewram()
  │    reads EWRAM[0x02031DBC - 0x02000000 .. +2]
  │    → FireRedState { map_group_id, map_name_id }
  │
  ├─ fill_party_list()
  │    reads EWRAM[0x02024029]       → party_size (0–6)
  │    reads EWRAM[0x02024284 .. +600] → 6 × Pokemon structs
  │    → Arc<Mutex<Vec<Pokemon>>>
  │
  ├─ get_wild_enemy_pokemon()
  │    reads EWRAM[0x0202402C .. +100] → gEnemyParty[0]
  │    → Option<Pokemon> (None if personality == 0)
  │
  ├─ get_area_pokemon_id_for_state()
  │    looks up WildMonHeader cache by (map_group, map_name)
  │    → WildPokemonHeader  (land/water/rock/fish encounter lists)
  │    → Arc<Mutex<WildPokemonHeader>>
  │
  ├─ EncounterTracker::tick(…, run_start_balls: u32)
  │    if !run_tracking_active:
  │      has_pokeballs_threshold(run_start_balls)? → set run_tracking_active = true
  │      else → return (pre-ball phase, nothing recorded)
  │    compares enemy personality to last_enemy_personality
  │    if changed → new battle detected
  │      is_wild? (compare enemy ot_id to party lead ot_id)
  │      has_encounter(map)? → no-op (already recorded this area)
  │      allow_species_repeats = false?
  │        species_encountered()? → no-op (already recorded this species this run)
  │      dupes_clause check (DupesClauseMode, always applied):
  │        Off       → no extra check
  │        PerPlayer → species_caught_by_self()? → no-op (this player already caught it)
  │        Shared    → species_caught_any()?      → no-op (any player already caught it)
  │      record_encounter(enc) → INSERT INTO encounters
  │      set tracked_personality = enemy.personality
  │    scan party for tracked_personality
  │      found → set_encounter_caught(map) → UPDATE encounters SET caught=TRUE
  │
  ├─ check_for_dead_pokemon()
  │    for each party slot where current HP = 0 and was alive:
  │      mark_dead(pokemon) → INSERT INTO dead_pokemon
  │
  ├─ check_for_new_pokemon()
  │    for each box slot not seen before:
  │      mark_caught(pokemon) → INSERT INTO caught_pokemon
  │      if nickname non-empty: update_caught_nickname(personality, nickname)
  │        SELECT old nickname; if differs → UPDATE + return Some(old)
  │        on SELECT error → best-effort UPDATE, return None
  │        Some(old) → record_event(NicknameChange) + fire_event(webhook)
  │
  ├─ check_for_new_badges(last_badge_mask)  [sentinel None = uninitialized]
  │    read_badge_state() → BadgeState
  │    build current_mask (8 bits, LSB = Boulder Badge)
  │    last_mask == None → silently adopt current_mask (boot guard)
  │    newly_earned = current_mask & !last_mask
  │    for each newly-earned badge:
  │      record_event(Badge) → INSERT INTO events
  │      fire_event(WebhookEvent::Badge) → optional POST + OBS clip
  │    last_badge_mask reset to None on: wipe detected, game unload,
  │      run change (thread_run_changed)
  │
  └─ check_for_run_over() → wipe detected
       enc_tracker.mark_wipe()
       thread_wipe_signal.store(true)
       last_badge_mask = None

  │
  │  Arc<Mutex<Vec<Pokemon>>> + Arc<Mutex<WildPokemonHeader>>
  │
  ▼
handle_client()  [tracker, Connected mode]
  Writer loop (100 ms):
    builds GameState {
      party, encounters, player_name, badge_state
    }
    serialises → bincode → 4-byte-BE-length + body
    sends over TCP

  Reader thread:
    receives ClientMessage (bincode, length-prefixed)
    │
    ├─ RequestTextures(species_ids)
    │    for each species: build_sprite_data(rom, id, shiny)
    │      decompress LZ77 → RGBA pixels
    │      zlib-compress → SpriteData.pixels
    │    sends ServerMessage::Textures(Vec<SpriteData>)
    │
    ├─ EndRun
    │    fire_red_database::end_run()
    │      UPDATE runs SET ended_at = now WHERE id = ?
    │      delete_meta("active_run_id")
    │    run_changed.store(true, Release)
    │    sends ServerMessage::RunChanged(None)
    │
    └─ NewRun
         fire_red_database::new_run("Unknown")
           INSERT INTO runs → returns id
           set_meta("active_run_id", id)
         run_changed.store(true, Release)
         sends ServerMessage::RunChanged(Some(id))

  │
  │  TCP
  │
  ▼
handle_tracker_connection()  [aggregator]
  Reader loop:
    receives ServerMessage::State(gs)
      → *slot.state.lock() = Some(gs)
      → *slot.label.lock() = gs.player_name

    receives ServerMessage::Textures(sprites)
      for each sprite:
        decompress pixels (zlib → RGBA)
        encode_png → Vec<u8>
        slot.sprite_cache.insert((species, shiny), png)
        push PendingTexture to slot.pending_textures

    receives ServerMessage::RunChanged(_)
      → slot.run_changed.store(true, Release)
         (triggers DbReader.mark_dirty() on next broadcast tick)

  Writer thread (50 ms):
    drains slot.command_queue
      → sends ClientMessage::EndRun or NewRun to tracker
    drains slot.texture_request_queue
      → dedup species ids → sends ClientMessage::RequestTextures

  │
  │  SharedSlots  (in-process, Arc<Mutex<...>>)
  │
  ▼
BroadcastLoop (100 ms)  [aggregator, web mode]
  For each slot:
    snapshot state = slot.state.lock().clone()
    snapshot label = slot.label.lock().clone()
    run_changed? → db.mark_dirty()
    db.sync_player(player_name) → resolves run_id from DB
    collect sprites for party + encounters
      missing species → push to slot.texture_request_queue

    build SlotDto {
      label, connected, db_connected, active_run_id,
      run_summary (if run ended),
      db_encounters (if run ended),
      badges, next_gym,
      party: [MemberDto { sprite as data URI, stats, soul_link_partner? }],
      encounters: [EncounterGroupDto],
      dead: [DeadMonDto],
      caught: [CaughtMonDto],
    }

  serde_json::to_string(all_slots)
  if JSON changed → broadcast to all WS clients

  │
  │  WebSocket
  │
  ▼
overlay.html / focused.html  (browser / OBS browser source)
  JSON parsed → render()
  Sprites are data:image/png;base64,... URIs embedded in JSON
    → no separate HTTP requests needed
  Run controls only visible when ?manage in URL
  sendCmd("end_run" / "new_run") → WS message
    → WebState::handle_socket() → push to ALL slot command_queues
```

## Database Write Path

```
Game-polling thread
    │
    ├── mark_dead(DeadPokemon)
    │     DB.lock() → INSERT INTO dead_pokemon (run_id, player_name, ...)
    │     ON CONFLICT (run_id, personality) DO NOTHING
    │
    ├── mark_caught(CaughtPokemon)
    │     DB.lock() → INSERT INTO dead_pokemon (run_id, player_name, ...)
    │     ON CONFLICT (run_id, personality) DO NOTHING
    │
    ├── record_encounter(Encounter)
    │     DB.lock() → INSERT INTO encounters (run_id, player_name, map_group, map_name, ...)
    │     ON CONFLICT (run_id, player_name, map_group, map_name) DO NOTHING
    │     returns true if row was new
    │
    └── set_encounter_caught(map_group, map_name)
          DB.lock() → UPDATE encounters SET caught = TRUE
          WHERE run_id=$1 AND player_name=$2 AND map_group=$3 AND map_name=$4

Network reader thread (triggered by web command)
    │
    ├── end_run()
    │     DB.lock() → UPDATE runs SET ended_at = now() WHERE id = ?
    │     delete_meta("active_run_id")
    │
    └── new_run("Unknown")
          DB.lock() → INSERT INTO runs → get id
          set_meta("active_run_id", id)
          returns new run id
```

## DB Read Path (Aggregator)

```
BroadcastLoop / AggregatorApp
    │
    └── DbReader (per-slot, read-only connection)
          │
          ├── sync_player(player_name)
          │     SELECT id FROM runs ORDER BY id DESC LIMIT 1
          │     returns true if run_id changed → forces full re-read
          │
          ├── list_dead_with_records(player_name)
          │     SELECT ... FROM dead_pokemon
          │     WHERE run_id=$1 AND player_name=$2
          │
          ├── list_caught(player_name)
          │     SELECT ... FROM caught_pokemon
          │     WHERE run_id=$1 AND player_name=$2
          │
          ├── list_encounters(player_name)
          │     SELECT ... FROM encounters
          │     WHERE run_id=$1 AND player_name=$2
          │
          └── run_summary()
                SELECT r.id, r.player_name, r.started_at, r.ended_at,
                       COUNT(dead_pokemon.*), COUNT(caught_pokemon.*)
                FROM runs r LEFT JOIN ...
                WHERE r.id = ?
```

## Sprite Pipeline

```
ROM bytes (LZ77-compressed front sprite)
    │
    fire_red_image_data::decompress_lz77()
    │
    ▼
RGBA pixel buffer (Vec<u8>, width × height × 4)
    │
    fire_red_tracker: zlib::encode → SpriteData.pixels (compressed)
    │
    ▼  TCP  (inside ServerMessage::Textures)
    │
    fire_red_aggregator (client.rs): decompress_pixels (zlib → RGBA)
    │
    ├── GUI mode:  load into egui::TextureHandle  (GPU upload)
    └── Web mode:  png::encode → Vec<u8>
                   base64::encode → String
                   embed as "data:image/png;base64,..."
                   in JSON payload  → no separate HTTP requests
```

## Run Change Flow

```
Web browser (with ?manage)
    │  WS message: { "cmd": "end_run" }
    ▼
axum WebSocket handler
    push ClientMessage::EndRun to ALL slot command_queues
    │
    ▼  (each slot's writer thread, next 50ms tick)
    sends ClientMessage::EndRun over TCP to tracker
    │
    ▼
tracker reader thread
    fire_red_database::end_run()
    run_changed.store(true, Release)
    sends ServerMessage::RunChanged(None) back
    │
    ▼
aggregator reader loop
    run_changed.store(true, Release)
    │
    ▼ (next BroadcastLoop tick)
    slot.run_changed.swap(false, AcqRel) → true
    db.mark_dirty()
    db.sync_player() → detects ended run
    build SlotDto with run_summary + db_encounters, no party
    broadcast to overlay
    │
    ▼
overlay.html
    active_run_id == null && run_summary present
    → renderRunEnded() — shows summary card + first-encounters grid
```

## New HTTP Endpoints (v0.8.91)

```
GET /api/bot/:index
    reads SharedSlots[index].state (same source as /api/slot/:index)
    returns plain text: "<player> — <hp>/<max_hp> HP — <zone>"
    suitable for Twitch chat bots; no JSON parsing needed

GET /compare
    serves compare.html (self-contained JS page)
    page fetches GET /api/runs → run list for dropdowns
    on selection fetches GET /api/run/:id/stats → per-run stats
    renders side-by-side panels; better/worse values highlighted green/red
    stats are cached in-page per run ID

GET /api/run/:id/export?format=csv   (pre-existing endpoint, now linked from /db)
    linked as a direct browser download from the CSV column in the Runs table
    on the /db page — no new server handler, just surfaced in the UI
```
