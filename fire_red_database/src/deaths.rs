//! Death tracking, HP closest-call tracking, and per-Pokemon HP history.

use super::*;

// ---------------------------------------------------------------------------
// Public API — death tracking
// ---------------------------------------------------------------------------

/// Records a Pokemon as permanently dead in the active run.
///
/// Returns `true` if the row was newly inserted; `false` if there is no active
/// run, the DB write failed, or the record already existed (ON CONFLICT).
/// Callers should only fire downstream events (webhooks, etc.) on `true`.
/// Returns `Ok(true)` when the row was newly inserted, `Ok(false)` when there
/// is no active run (caller should skip the death event silently), and
/// `Err(e)` on a database error — the caller should log the error and skip.
pub fn mark_dead(pokemon: DeadPokemon) -> Result<bool, postgres::Error> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(false),
    };
    let player = pg_safe(&state.effective_player_name());
    let ot_name = pg_safe(&pokemon.ot_name);
    let nickname = pg_safe(&pokemon.nickname);
    let spec_name = pg_safe(&pokemon.species_name);
    let ability_name = pg_safe(&pokemon.ability_name);
    let area_name = pg_safe(&pokemon.area_name);
    let n = state.client.execute(
        "INSERT INTO dead_pokemon (
            run_id, player_name, personality, ot_id, ot_name, nickname,
            species, species_name, is_shiny, nature,
            level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
            move1, move2, move3, move4,
            pp1, pp2, pp3, pp4,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
            held_item, ability, ability_name, friendship, met_location, died_at, gender,
            is_soul_link_death, killed_by_species, killed_by_move, area_name
        ) VALUES (
            $1,  $2,  $3,  $4,  $5,  $6,  $7,  $8,  $9,  $10, $11,
            $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22,
            $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33,
            $34, $35, $36, $37, $38, $39, $40, $41, $42, $43, $44, $45,
            $46, $47, $48, $49
        ) ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
            &player, // $2  = player_name
            &(pokemon.personality as i64),
            &(pokemon.ot_id as i64),
            &ot_name,
            &nickname,
            &(pokemon.species as i32),
            &spec_name,
            &pokemon.is_shiny,
            &pokemon.nature,
            &(pokemon.level as i32),
            &(pokemon.experience as i64),
            &(pokemon.max_hp as i32),
            &(pokemon.attack as i32),
            &(pokemon.defense as i32),
            &(pokemon.speed as i32),
            &(pokemon.sp_attack as i32),
            &(pokemon.sp_defense as i32),
            &(pokemon.moves[0] as i32),
            &(pokemon.moves[1] as i32),
            &(pokemon.moves[2] as i32),
            &(pokemon.moves[3] as i32),
            &(pokemon.pp[0] as i32),
            &(pokemon.pp[1] as i32),
            &(pokemon.pp[2] as i32),
            &(pokemon.pp[3] as i32),
            &(pokemon.ivs.hp as i32),
            &(pokemon.ivs.attack as i32),
            &(pokemon.ivs.defense as i32),
            &(pokemon.ivs.speed as i32),
            &(pokemon.ivs.sp_attack as i32),
            &(pokemon.ivs.sp_defense as i32),
            &(pokemon.evs.hp as i32),
            &(pokemon.evs.attack as i32),
            &(pokemon.evs.defense as i32),
            &(pokemon.evs.speed as i32),
            &(pokemon.evs.sp_attack as i32),
            &(pokemon.evs.sp_defense as i32),
            &(pokemon.held_item as i32),
            &(pokemon.ability as i32),
            &ability_name,
            &(pokemon.friendship as i32),
            &(pokemon.met_location as i32),
            &(pokemon.died_at as i64),
            &(pokemon.gender as i32),
            &pokemon.is_soul_link_death,
            &pokemon.killed_by_species,
            &pokemon.killed_by_move,
            &area_name, // $49
        ],
    )?;
    // execute() returns the number of rows affected. ON CONFLICT DO NOTHING yields 0,
    // meaning the record already exists — return false so callers don't re-fire events.
    Ok(n > 0)
}

/// Returns `true` if the Pokemon with this personality is dead in the active run.
pub fn is_dead(personality: u32) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    query_is_dead(&mut state.client, active, personality)
}

/// Returns the stored `DeadPokemon` entry for this personality in the active run.
pub fn get_dead_pokemon(personality: u32) -> Option<DeadPokemon> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let active = state.effective_run_id()?;
    let row = state.client
        .query_opt(
            "SELECT
                player_name, personality, ot_id, ot_name, nickname, species, species_name, is_shiny, nature,
                level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                move1, move2, move3, move4, pp1, pp2, pp3, pp4,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                held_item, ability, ability_name, friendship, met_location, died_at, gender,
                is_soul_link_death, killed_by_species, killed_by_move,
                COALESCE(area_name, '') AS area_name
             FROM dead_pokemon
             WHERE run_id = $1 AND personality = $2",
            &[&(active as i32), &(personality as i64)],
        )
        .ok()??;

    Some(row_to_dead_pokemon(&row))
}

// ---------------------------------------------------------------------------
// Public API — HP closest-call tracking
// ---------------------------------------------------------------------------

/// Record the lowest HP ratio (current_hp / max_hp) ever seen for a party
/// Pokémon in the current run. Called every game-loop tick for each live mon.
///
/// Skips mons with `hp == 0` (already dead) or `max_hp == 0` (invalid read).
/// Uses integer cross-multiplication to avoid floating-point comparisons in SQL.
pub fn update_min_hp_seen(personality: u32, hp: u16, max_hp: u16) {
    if hp == 0 || max_hp == 0 {
        return;
    }
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let hp_i = hp as i32;
    let max_i = max_hp as i32;
    // Update only if no previous record (IS NULL) or new ratio is strictly lower.
    // Cross-multiply to compare fractions without floats:
    //   new_hp/new_max < old_hp/old_max  ⟺  new_hp*old_max < old_hp*new_max
    let _ = state.client.execute(
        "UPDATE caught_pokemon
         SET min_hp_seen_hp     = CASE
               WHEN min_hp_seen_hp IS NULL
                 OR ($3::bigint * min_hp_seen_max_hp) < (min_hp_seen_hp::bigint * $4)
               THEN $3 ELSE min_hp_seen_hp END,
             min_hp_seen_max_hp = CASE
               WHEN min_hp_seen_hp IS NULL
                 OR ($3::bigint * min_hp_seen_max_hp) < (min_hp_seen_hp::bigint * $4)
               THEN $4 ELSE min_hp_seen_max_hp END
         WHERE run_id = $1 AND personality = $2",
        &[&(run_id as i32), &(personality as i64), &hp_i, &max_i],
    );
}

// ---------------------------------------------------------------------------
// Public API — per-Pokémon HP history
// ---------------------------------------------------------------------------

/// Record a timestamped HP observation for a party Pokémon.
///
/// Call this whenever the Pokémon's HP differs from the last-recorded value.
/// Uses the shared DB connection so it is safe to call from the game loop.
pub fn record_hp_observation(personality: u32, hp: u16, max_hp: u16) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let _ = state.client.execute(
        "INSERT INTO hp_history (run_id, personality, observed_at, hp, max_hp)
         VALUES ($1, $2, $3, $4, $5)",
        &[
            &(run_id as i32),
            &(personality as i64),
            &(unix_now() as i64),
            &(hp as i32),
            &(max_hp as i32),
        ],
    );
}

/// Record an enemy Pokémon's HP at the start or end of an encounter.
///
/// `phase` should be `"initial"` (battle start) or `"final"` (battle end).
/// Uses the shared DB connection.
pub fn record_enemy_hp(personality: u32, hp: u16, max_hp: u16, phase: &str) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let _ = state.client.execute(
        "INSERT INTO enemy_hp_log (run_id, personality, observed_at, hp, max_hp, phase)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            &(run_id as i32),
            &(personality as i64),
            &(unix_now() as i64),
            &(hp as i32),
            &(max_hp as i32),
            &phase,
        ],
    );
}

/// Returns the full HP history for one Pokémon in a run, ordered oldest-first.
///
/// Each entry: `{ observed_at, hp, max_hp, timestamp }`.
pub fn get_hp_history(conn_str: &str, run_id: u32, personality: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT observed_at, hp, max_hp FROM hp_history
         WHERE run_id = $1 AND personality = $2
         ORDER BY observed_at ASC",
        &[&(run_id as i32), &(personality as i64)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let history: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let at: i64 = row.get(0);
            let hp: i32 = row.get(1);
            let max_hp: i32 = row.get(2);
            serde_json::json!({
                "observed_at": at,
                "timestamp": format_timestamp(at as u64),
                "hp": hp,
                "max_hp": max_hp,
            })
        })
        .collect();
    serde_json::json!({
        "run_id": run_id,
        "personality": personality,
        "history": history,
    })
}

/// Returns all enemy HP observations for a run, grouped by encounter.
///
/// Each entry: `{ personality, initial_hp, initial_max_hp, final_hp,
/// final_max_hp, damage_dealt, timestamp }`.
pub fn get_enemy_hp_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT e.personality, e.hp, e.max_hp, e.phase, e.observed_at,
                COALESCE(enc.species_name, '') AS species_name
         FROM enemy_hp_log e
         LEFT JOIN encounters enc
               ON enc.run_id = e.run_id
              AND enc.id = (
                  SELECT id FROM encounters
                  WHERE run_id = $1
                  ORDER BY ABS(encountered_at - e.observed_at)
                  LIMIT 1
              )
         WHERE e.run_id = $1
         ORDER BY e.personality, e.observed_at",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    // Group by personality → {initial, final}
    use std::collections::BTreeMap;
    struct Obs {
        hp: i32,
        max_hp: i32,
        at: i64,
        species_name: String,
    }
    let mut encounters_map: BTreeMap<i64, (Option<Obs>, Option<Obs>)> = BTreeMap::new();
    for row in &rows {
        let personality: i64 = row.get(0);
        let hp: i32 = row.get(1);
        let max_hp: i32 = row.get(2);
        let phase: String = row.get(3);
        let at: i64 = row.get(4);
        let species_name: String = row.get(5);
        let obs = Obs { hp, max_hp, at, species_name };
        let entry = encounters_map.entry(personality).or_insert((None, None));
        if phase == "initial" {
            entry.0 = Some(obs);
        } else {
            entry.1 = Some(obs);
        }
    }
    let entries: Vec<serde_json::Value> = encounters_map
        .into_iter()
        .map(|(personality, (init, fin))| {
            let species = init.as_ref().or(fin.as_ref()).map(|o| o.species_name.clone()).unwrap_or_default();
            let init_hp = init.as_ref().map(|o| o.hp).unwrap_or(0);
            let init_max = init.as_ref().map(|o| o.max_hp).unwrap_or(0);
            let fin_hp = fin.as_ref().map(|o| o.hp);
            let fin_max = fin.as_ref().map(|o| o.max_hp);
            let damage = fin_hp.map(|fh| (init_hp - fh).max(0));
            let at = init.as_ref().or(fin.as_ref()).map(|o| o.at).unwrap_or(0);
            serde_json::json!({
                "personality": personality as u32,
                "species_name": species,
                "timestamp": format_timestamp(at as u64),
                "initial_hp": init_hp,
                "initial_max_hp": init_max,
                "final_hp": fin_hp,
                "final_max_hp": fin_max,
                "damage_dealt": damage,
            })
        })
        .collect();
    serde_json::json!({ "run_id": run_id, "encounters": entries })
}

/// Returns a battle-by-battle damage summary for a run.
///
/// Damage events (HP decreases) are grouped into battles using a 120-second
/// gap threshold — if no damage occurs for 120 s the next damage event opens
/// a new battle entry.
///
/// Each battle entry: `{ battle_index, start_at, end_at, duration_secs, mons }`.
/// Each mon entry: `{ personality, nickname, species_name, damage_taken }`.
pub fn get_battle_damage_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    // Fetch HP observations ordered by time for all party mons.
    let rows = match client.query(
        "SELECT h.personality, h.observed_at, h.hp, h.max_hp,
                COALESCE(cp.nickname, '') AS nickname,
                COALESCE(cp.species_name, '') AS species_name
         FROM hp_history h
         LEFT JOIN caught_pokemon cp
               ON cp.run_id = h.run_id AND cp.personality = h.personality
         WHERE h.run_id = $1
         ORDER BY h.personality, h.observed_at",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    // Build per-personality HP sequences, find decreases.
    use std::collections::HashMap;
    struct DamageEvent {
        personality: i64,
        at: i64,
        damage: i32,
        nickname: String,
        species_name: String,
    }

    let mut prev: HashMap<i64, (i64, i32)> = HashMap::new(); // personality → (at, hp)
    let mut mon_labels: HashMap<i64, (String, String)> = HashMap::new(); // personality → (nick, species)
    let mut damage_events: Vec<DamageEvent> = Vec::new();

    for row in &rows {
        let personality: i64 = row.get(0);
        let at: i64 = row.get(1);
        let hp: i32 = row.get(2);
        let _max_hp: i32 = row.get(3);
        let nickname: String = row.get(4);
        let species_name: String = row.get(5);
        mon_labels.entry(personality).or_insert((nickname.clone(), species_name.clone()));
        if let Some(&(_prev_at, prev_hp)) = prev.get(&personality)
            && hp < prev_hp
        {
            let (nick, spec) = mon_labels.get(&personality).cloned().unwrap_or_default();
            damage_events.push(DamageEvent {
                personality,
                at,
                damage: prev_hp - hp,
                nickname: nick,
                species_name: spec,
            });
        }
        prev.insert(personality, (at, hp));
    }

    // Sort damage events by time.
    damage_events.sort_by_key(|e| e.at);

    // Group into battles using 120-second gap threshold.
    const BATTLE_GAP_SECS: i64 = 120;
    struct Battle {
        start_at: i64,
        end_at: i64,
        mons: HashMap<i64, (i32, String, String)>, // personality → (damage, nick, species)
    }
    let mut battles: Vec<Battle> = Vec::new();
    for ev in &damage_events {
        if let Some(last) = battles.last_mut()
            && ev.at - last.end_at <= BATTLE_GAP_SECS
        {
            last.end_at = ev.at;
            let entry = last.mons.entry(ev.personality).or_insert((0, ev.nickname.clone(), ev.species_name.clone()));
            entry.0 += ev.damage;
            continue;
        }
        let mut mons = HashMap::new();
        mons.insert(ev.personality, (ev.damage, ev.nickname.clone(), ev.species_name.clone()));
        battles.push(Battle { start_at: ev.at, end_at: ev.at, mons });
    }

    let result: Vec<serde_json::Value> = battles
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut mon_list: Vec<serde_json::Value> = b.mons.iter().map(|(p, (dmg, nick, spec))| {
                serde_json::json!({
                    "personality": *p as u32,
                    "nickname": nick,
                    "species_name": spec,
                    "damage_taken": dmg,
                })
            }).collect();
            mon_list.sort_by(|a, b| b["damage_taken"].as_i64().cmp(&a["damage_taken"].as_i64()));
            let total: i32 = b.mons.values().map(|(d, _, _)| d).sum();
            serde_json::json!({
                "battle_index": i + 1,
                "start_at": b.start_at,
                "end_at": b.end_at,
                "start_timestamp": format_timestamp(b.start_at as u64),
                "end_timestamp": format_timestamp(b.end_at as u64),
                "duration_secs": b.end_at - b.start_at,
                "total_damage": total,
                "mons": mon_list,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "battles": result })
}
