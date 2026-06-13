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
    ├─ NewRun
    │    fire_red_database::new_run("Unknown")
    │      INSERT INTO runs → returns id
    │      set_meta("active_run_id", id)
    │    run_changed.store(true, Release)
    │    sends ServerMessage::RunChanged(Some(id))
    │
    ├─ GiveItem { item_id, quantity }
    │    game::give_item(ewram, item_id, quantity)
    │      locate items pocket via SaveBlock1 ptr + security key
    │      compute_give_item_write() → (addr, encoded_bytes)
    │    WRITE_CORE_MEMORY addr bytes → RetroArch UDP :55355
    │
    ├─ TakeItem { item_id, quantity }  → game::take_item() → UDP
    ├─ MakeShiny { party_position }    → game::make_shiny() → UDP
    ├─ ChangeSpecies { party_position, new_species }
    │                                   → game::change_species() → UDP
    ├─ ChangeAbility { party_position, ability_slot }
    │                                   → game::change_ability() → UDP
    └─ ChangeGender { party_position, target_gender }
                                        → game::change_gender() → UDP

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
      → sends ClientMessage::EndRun / NewRun / GiveItem / TakeItem /
               MakeShiny / ChangeSpecies / ChangeAbility / ChangeGender
               to tracker over TCP
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

## Bug Fixes (v0.8.96)

```
1. export_run IV/EV round-trip data loss (FIXED)
   ─────────────────────────────────────────────
   Problem: export_run() (JSON) SELECT omitted all 12 iv_*/ev_* columns.
            import_run() hard-coded literal 0 for all 12 IV/EV bind slots.
            Every export→import round-trip silently zeroed all stat data.

   Fix: export_run() caught and dead SELECTs now include all 12 IV/EV columns
        (indices 11-22 in both queries); JSON objects expose:
          iv_hp, iv_atk, iv_def, iv_spe, iv_spa, iv_spd
          ev_hp, ev_atk, ev_def, ev_spe, ev_spa, ev_spd
        import_run() reads these keys from the JSON body (falling back to 0 for
        pre-fix exports) and passes them as $13-$24 bind params for caught and
        $15-$26 for dead instead of the hard-coded SQL 0 literals.

2. webhook::init() orphaned sender on spawn failure (FIXED)
   ─────────────────────────────────────────────────────────
   Problem: STATE.set(WebhookState { tx, … }) ran unconditionally before the
            thread spawn result was checked. If spawn returned Err, rx was
            dropped but tx lived in STATE forever. All subsequent fire_event()
            calls sent to the disconnected channel, discarding events silently
            via let _ = state.tx.send(…).

   Fix: spawn is attempted before STATE is set. On Err, both tx and rx are
        dropped and STATE is never populated, so fire_event() no-ops cleanly
        (STATE.get() returns None). On Ok, STATE.set() is called. A fast-path
        STATE.get().is_some() guard replaces the old set().is_err() guard.

3. export_run_csv silent error swallow (FIXED)
   ────────────────────────────────────────────
   Problem: All three DB queries inside export_run_csv() used .unwrap_or_default(),
            returning an empty section with no log entry on any DB failure. This
            was inconsistent with the v0.8.95 fix applied to DbReader methods.

   Fix: All three now use .unwrap_or_else(|e| { tracing::warn!(…); vec![] })
        matching the pattern used elsewhere in the file.

4. import_run encounters INSERT silent drop (FIXED)
   ─────────────────────────────────────────────────
   Problem: The encounters INSERT loop used let _ = client.execute(…), silently
            swallowing both Ok(0) collisions and Err DB errors. The caught and
            dead loops were updated to warn in v0.8.95 but encounters was missed.
            Also: encounters had no ON CONFLICT clause, so duplicates could
            produce a unique-constraint error that was silently dropped.

   Fix: Encounters INSERT now has ON CONFLICT DO NOTHING and uses a match block
        with tracing::warn! on Ok(0) and Err, consistent with caught/dead.
```

## DB Reader Error Handling (v0.8.95)

```
DbReader query methods — changed from silent empty fallback to logged warning:

  list_dead_with_records()  .query(...)
  list_encounters()         .query(...)     → .unwrap_or_else(|e| {
  list_prev_run_encounters().query(...)          tracing::warn!("... DB query failed: {e}");
                                                 vec![]
                                            })

Previously: .unwrap_or_default()   — silent empty Vec on ANY error
Now:        .unwrap_or_else(warn)  — empty Vec + logged error

import_run() personality collision — changed from silent drop to logged warning:
  match client.execute("INSERT ... ON CONFLICT DO NOTHING", ...) {
      Ok(0) => tracing::warn!("personality 0x... already exists; skipped"),
      Ok(_) => {}                   // inserted normally
      Err(e) => tracing::warn!("failed to insert: {e}"),
  }
```

## New HTTP Endpoints (v0.8.94)

```
GET /api/run/:id/route_odds
    queries encounters WHERE run_id=$1 → builds seen canonical-floor set
    compares against fire_red_location_names::all_wild_areas() (static list)
    returns { run_id, encountered: [...], unencountered: [...] }
    encountered entries: player_name, map_group, map_name, area, species (numeric dex),
                         species_name, level, caught, is_shiny, encountered_at
    unencountered entries: map_group, map_name, area

GET /api/run/:id/webhook_log
    queries webhook_log WHERE run_id=$1 ORDER BY fired_at ASC
    returns { run_id, webhook_log: [{ event_type, url, success, attempts,
                                      payload, fired_at, fired_at_human }] }
    populated by fire_red_tracker/webhook.rs worker after each delivery attempt
```

## Webhook Delivery Log (v0.8.94)

```
Tracker process — webhook.rs worker thread
    │
    for each WorkerTask::Webhook { url, body, event_type, run_id }:
    │
    ├── captures run_id from fire_red_database::get_active_run_id()
    │     at fire_event() call time (before enqueuing)
    │
    ├── serializes payload: PostBody::Raw → clone; PostBody::Json → serde_json
    │
    ├── retry loop (max 3 attempts, exponential backoff 1s / 2s)
    │
    └── fire_red_database::record_webhook_delivery(
              run_id, event_type, url, success, attempts, payload)
          DB.lock() → INSERT INTO webhook_log (...)
```

## DB Write Functions — Error Handling (v0.8.94)

```
mark_dead(DeadPokemon)       → Result<bool, postgres::Error>
    Ok(true)  = newly inserted
    Ok(false) = no active run
    Err(e)    = DB error (logged at call site via tracing::error!)

record_encounter(Encounter)  → Result<bool, postgres::Error>
    Ok(true)  = first encounter for this area (new row)
    Ok(false) = duplicate or no active run
    Err(e)    = DB error

record_event(EventKind)      → Result<(), postgres::Error>
    Ok(())    = success or no active run
    Err(e)    = DB error
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

## Injection Commands Flow (v0.9.x)

```
HTTP POST /api/slot/:index/give_item  (or take_item / make_shiny / etc.)
    │
    web handler validates body fields
    if state.allow_injections == false → 403 Forbidden (early return)
    push ClientMessage::GiveItem { item_id, quantity }
      to slot.command_queue  (Arc<Mutex<VecDeque<ClientMessage>>>)
    return 200 OK
    │
    │  (next aggregator writer thread tick, 50 ms)
    ▼
handle_tracker_connection() writer thread
    drain slot.command_queue
    serialise ClientMessage::GiveItem → bincode → length-prefixed TCP frame
    send to tracker
    │
    │  TCP
    ▼
handle_client() tracker reader thread
    deserialise ClientMessage::GiveItem { item_id, quantity }
    game::give_item(ewram_snapshot, item_id, quantity)
      read SaveBlock1 ptr from IWRAM
      locate items pocket base address
      derive XOR security key (pocket oracle or SaveBlock2 scan)
      compute_give_item_write() → Vec<(addr, encoded_4_bytes)>
    for each (addr, bytes):
      "WRITE_CORE_MEMORY 0x{addr:08X} {hex_bytes}" → UDP :55355 → RetroArch
    │
    ▼ (next memory thread tick, 100 ms)
    fill_party_list() reads updated EWRAM
    ServerMessage::State pushed to aggregator → WS broadcast → overlay updated

Injection event toast path:
    game::give_item() returns InjectionEvent { at, kind, label }
    MonitorSlot.injection_events.lock().push_back(event)
    BroadcastLoop::tick() drains injection_events
      → SlotDto.injection_events (cleared each broadcast cycle)
    alerts.html handleState():
      for ev of slot.injection_events:
        if _seenInjectionAts.has(ev.at) → skip (dedup)
        _seenInjectionAts.add(ev.at)
        showInjectToast(ev.label)
          → queued 4s sequential toasts with 0.4s gap
```
