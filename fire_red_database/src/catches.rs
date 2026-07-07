//! Catch tracking, the `EventKind` audit-trail event type, and trainer
//! defeat records.

use super::*;

// ---------------------------------------------------------------------------
// Public API — catch tracking
// ---------------------------------------------------------------------------

/// A notable gameplay event to persist in the `events` table.
///
/// Events are a supplementary audit trail alongside the `dead_pokemon`,
/// `caught_pokemon`, and `encounters` tables. They are append-only and ordered
/// by `occurred_at`, making them suitable for streaming or timeline displays.
pub enum EventKind<'a> {
    /// A Pokémon was caught and added to the party.
    Catch {
        species_name: &'a str,
        nickname: &'a str,
        level: u8,
    },
    /// A Pokémon fainted from direct in-game damage.
    Death {
        species_name: &'a str,
        nickname: &'a str,
        level: u8,
    },
    /// A Pokémon was killed by the Soul Link rule.
    SoulLinkDeath {
        species_name: &'a str,
        nickname: &'a str,
        level: u8,
    },
    /// A shiny Pokémon appeared in the wild.
    Shiny { species_name: &'a str, level: u8 },
    /// The party was wiped, ending the run.
    Wipe,
    /// A gym badge (or E4 win) was earned.
    Badge { badge_name: &'a str },
    /// A caught Pokémon's nickname was changed in-game.
    NicknameChange {
        species_name: &'a str,
        old_name: &'a str,
        new_name: &'a str,
    },
}

impl<'a> EventKind<'a> {
    /// Extracts `(event_type, species_name, nickname, old_nickname, level)` for a DB INSERT.
    pub(crate) fn row_parts(&self) -> (&'static str, &'a str, &'a str, &'a str, i32) {
        match self {
            EventKind::Catch {
                species_name,
                nickname,
                level,
            } => ("catch", species_name, nickname, "", *level as i32),
            EventKind::Death {
                species_name,
                nickname,
                level,
            } => ("death", species_name, nickname, "", *level as i32),
            EventKind::SoulLinkDeath {
                species_name,
                nickname,
                level,
            } => ("soul_link_death", species_name, nickname, "", *level as i32),
            EventKind::Shiny {
                species_name,
                level,
            } => ("shiny", species_name, "", "", *level as i32),
            EventKind::Wipe => ("wipe", "", "", "", 0),
            EventKind::Badge { badge_name } => ("badge", badge_name, "", "", 0),
            EventKind::NicknameChange {
                species_name,
                old_name,
                new_name,
            } => ("nickname_change", species_name, new_name, old_name, 0),
        }
    }
}

/// Appends a row to the `events` table for the active run.
///
/// No-op if no run is currently active. Returns `true` when the event was
/// successfully persisted, `false` on any failure or missing run.
/// Appends a row to the `events` table. Returns `Ok(())` on success,
/// `Ok(())` with a no-op when there is no active run, and `Err` on a DB error.
pub fn record_event(event: EventKind<'_>) -> Result<(), postgres::Error> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    let run_id = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(()),
    };
    let player = state.effective_player_name();
    let occurred_at = unix_now() as i64;
    let (event_type, species_name, nickname, old_nickname, level) = event.row_parts();
    state.client.execute(
        "INSERT INTO events (run_id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        &[
            &(run_id as i32),
            &player,
            &event_type,
            &species_name,
            &nickname,
            &old_nickname,
            &level,
            &occurred_at,
        ],
    )?;
    Ok(())
}

/// Records a defeated trainer in the `trainer_battles` table for the active run.
///
/// No-op if no run is currently active or if this flag has already been recorded
/// (`ON CONFLICT DO NOTHING` on `(run_id, player_name, flag_index)`).
/// Returns `Ok(true)` when a new row was inserted.
pub fn record_trainer_defeat(defeat: TrainerDefeat) -> Result<bool, postgres::Error> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let run_id = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(false),
    };
    let player = pg_safe(&state.effective_player_name());
    let n = state.client.execute(
        "INSERT INTO trainer_battles (run_id, player_name, flag_index, trainer_name, location, defeated_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (run_id, player_name, flag_index) DO NOTHING",
        &[
            &(run_id as i32),
            &player,
            &(defeat.flag_index as i32),
            &defeat.trainer_name,
            &defeat.location,
            &(defeat.defeated_at as i64),
        ],
    )?;
    Ok(n > 0)
}

/// Returns all trainer defeats for a run as a JSON array, ordered by time.
///
/// Each entry has: `player_name`, `flag_index`, `trainer_name`, `location`,
/// `defeated_at` (unix seconds), `defeated_at_human` (formatted string).
pub fn get_trainer_defeats_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let Ok(mut client) = postgres::Client::connect(conn_str, NoTls) else {
        return serde_json::json!({ "error": "database connection failed" });
    };
    let rows = match client.query(
        "SELECT player_name, flag_index, trainer_name, location, defeated_at
         FROM trainer_battles WHERE run_id = $1 ORDER BY defeated_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": e.to_string() }),
    };
    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let defeated_at = row.get::<_, i64>(4) as u64;
            serde_json::json!({
                "player_name":     row.get::<_, String>(0),
                "flag_index":      row.get::<_, i32>(1),
                "trainer_name":    row.get::<_, String>(2),
                "location":        row.get::<_, String>(3),
                "defeated_at":     defeated_at,
                "defeated_at_human": format_timestamp(defeated_at),
            })
        })
        .collect();
    serde_json::json!(entries)
}

/// Records a Pokemon as caught in the active run.
///
/// No-op if this personality is already recorded (deduplicates on reconnect).
/// Returns `true` when a new row was inserted, `false` when the record already
/// existed (`ON CONFLICT DO NOTHING`) or no active run is set. Callers must
/// only fire downstream events (event log, webhooks) on `true`.
pub fn mark_caught(pokemon: CaughtPokemon) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = pg_safe(&state.effective_player_name());
    let nickname = pg_safe(&pokemon.nickname);
    let spec_name = pg_safe(&pokemon.species_name);
    match state.client.execute(
        "INSERT INTO caught_pokemon (
            run_id, player_name, personality, ot_id, nickname, species, species_name,
            is_shiny, nature, level, met_location,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
            caught_at, gender, location_name
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
        ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
            &player,
            &(pokemon.personality as i64),
            &(pokemon.ot_id as i64),
            &nickname,
            &(pokemon.species as i32),
            &spec_name,
            &pokemon.is_shiny,
            &pokemon.nature,
            &(pokemon.level as i32),
            &(pokemon.met_location as i32),
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
            &(pokemon.caught_at as i64),
            &(pokemon.gender as i32),
            &pokemon.location_name,
        ],
    ) {
        Ok(n) => n > 0,
        Err(e) => {
            tracing::warn!("Failed to record caught pokemon (personality={}): {e}", pokemon.personality);
            false
        }
    }
}

/// Updates the nickname of a caught Pokémon if it has changed.
///
/// No-op if the Pokémon is not registered, the run is not active, or the
/// nickname matches what is already stored.
/// Updates the in-game nickname of a caught Pokémon.
///
/// Returns `Some(old_name)` when the stored nickname differed and was updated,
/// or `None` if the name was already up to date, the Pokémon is not found, or
/// no active run is set.
pub fn update_caught_nickname(personality: u32, nickname: &str) -> Option<String> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let active = state.effective_run_id()?;
    // Read the current name first so we can return it as the "old" value.
    // On SELECT error, attempt the UPDATE anyway as a best-effort sync — but
    // note that if the client is in a broken state the UPDATE will also fail
    // silently (result is discarded). We still return None rather than panic.
    let old: Option<String> = match state.client.query_opt(
        "SELECT nickname FROM caught_pokemon
         WHERE run_id = $1 AND personality = $2 AND nickname != $3",
        &[&(active as i32), &(personality as i64), &nickname],
    ) {
        Ok(maybe_row) => maybe_row.map(|row| row.get(0)),
        Err(_) => {
            let _ = state.client.execute(
                "UPDATE caught_pokemon SET nickname = $1
                 WHERE run_id = $2 AND personality = $3 AND nickname != $1",
                &[&nickname, &(active as i32), &(personality as i64)],
            );
            return None;
        }
    };

    if let Some(old_name) = old {
        let _ = state.client.execute(
            "UPDATE caught_pokemon SET nickname = $1
             WHERE run_id = $2 AND personality = $3",
            &[&nickname, &(active as i32), &(personality as i64)],
        );
        Some(old_name)
    } else {
        None
    }
}

/// Updates the EVs of a caught Pokémon if any have changed.
///
/// No-op if the Pokémon is not registered, the run is not active, or all EVs
/// match what is already stored.
pub fn update_caught_evs(personality: u32, evs: &EVs) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return,
    };
    let _ = state.client.execute(
        "UPDATE caught_pokemon
         SET ev_hp = $1, ev_attack = $2, ev_defense = $3,
             ev_speed = $4, ev_sp_attack = $5, ev_sp_defense = $6
         WHERE run_id = $7 AND personality = $8
           AND (ev_hp != $1 OR ev_attack != $2 OR ev_defense != $3
             OR ev_speed != $4 OR ev_sp_attack != $5 OR ev_sp_defense != $6)",
        &[
            &(evs.hp as i32),
            &(evs.attack as i32),
            &(evs.defense as i32),
            &(evs.speed as i32),
            &(evs.sp_attack as i32),
            &(evs.sp_defense as i32),
            &(active as i32),
            &(personality as i64),
        ],
    );
}

/// Returns `true` if a Pokemon with this personality has been caught in the active run.
pub fn is_caught(personality: u32) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon WHERE run_id = $1 AND personality = $2",
            &[&(active as i32), &(personality as i64)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns all caught Pokemon for the active run for the current player.
pub fn list_caught() -> Vec<CaughtPokemon> {
    let Some(db) = db() else { return vec![] };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return vec![],
    };
    let player = state.effective_player_name();
    query_caught(&mut state.client, active, &player)
}
