//! Encounter tracking, catch attempts, area visits, species lookups, and
//! per-personality soul-link overrides.

use super::*;

// ---------------------------------------------------------------------------
// Public API — encounter tracking
// ---------------------------------------------------------------------------

/// Records the first wild encounter in an area for the current player.
///
/// Subsequent encounters in the same area by the same player are silently
/// ignored (Nuzlocke rule). Returns `true` if this was a new encounter.
/// Records a wild encounter. Returns `Ok(true)` when the row was newly inserted
/// (first encounter for this area), `Ok(false)` when the encounter already
/// exists or there is no active run, and `Err` on a DB error.
pub fn record_encounter(encounter: Encounter) -> Result<bool, postgres::Error> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(false),
    };
    let player = pg_safe(&state.effective_player_name());
    let spec_name = pg_safe(&encounter.species_name);
    let rows = state.client.execute(
        "INSERT INTO encounters (
            run_id, player_name, map_group, map_name, species, species_name, level, caught, encountered_at, is_shiny
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, $8, $9)
         ON CONFLICT (run_id, player_name, map_group, map_name) DO NOTHING",
        &[
            &(active as i32),
            &player,
            &(encounter.map_group as i32),
            &(encounter.map_name as i32),
            &(encounter.species as i32),
            &spec_name,
            &(encounter.level as i32),
            &(encounter.encountered_at as i64),
            &encounter.is_shiny,
        ],
    )?;
    Ok(rows == 1)
}

/// Marks the current player's encounter for this area as successfully caught.
pub fn set_encounter_caught(map_group: u8, map_name: u8) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return,
    };
    let player = state.effective_player_name();
    if let Err(e) = state.client.execute(
        "UPDATE encounters SET caught = TRUE
         WHERE run_id = $1 AND player_name = $2 AND map_group = $3 AND map_name = $4",
        &[
            &(active as i32),
            &player,
            &(map_group as i32),
            &(map_name as i32),
        ],
    ) {
        tracing::warn!("set_encounter_caught: DB error: {}", e);
    }
}

/// Records the outcome of a tracked wild encounter (first-per-area Nuzlocke slot).
///
/// Called by the encounter tracker when the encounter resolves — either a catch
/// or the next battle personality replacing the current one (fled/fainted).
/// Silently no-ops when there is no active run.
pub fn record_catch_attempt(
    species_name: &str,
    area: &str,
    balls_thrown: u32,
    caught: bool,
    encountered_at: u64,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return,
    };
    let player = pg_safe(&state.effective_player_name());
    let spec = pg_safe(species_name);
    let area_s = pg_safe(area);
    if let Err(e) = state.client.execute(
        "INSERT INTO catch_attempts
             (run_id, player_name, species_name, area, balls_thrown, caught, encountered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &(active as i32),
            &player,
            &spec,
            &area_s,
            &(balls_thrown as i32),
            &caught,
            &(encountered_at as i64),
        ],
    ) {
        tracing::warn!("record_catch_attempt: DB error: {e}");
    }
}

/// Records the start of a new area visit.  Returns the row `id` so the caller
/// can later close it with [`close_area_visit`].  Returns `None` when there is
/// no active run or the insert fails.
pub fn open_area_visit(map_group: u8, map_name: u8, area_name: &str, entered_at: u64) -> Option<i64> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let active = state.effective_run_id()? as i32;
    let player = pg_safe(&state.effective_player_name());
    let area_s = pg_safe(area_name);
    state
        .client
        .query_one(
            "INSERT INTO area_visits (run_id, player_name, map_group, map_name, area_name, entered_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id",
            &[
                &active,
                &player,
                &(map_group as i32),
                &(map_name as i32),
                &area_s,
                &(entered_at as i64),
            ],
        )
        .ok()
        .map(|row| row.get::<_, i32>(0) as i64)
}

/// Closes an open area visit by setting `exited_at`.  Silently ignores errors.
pub fn close_area_visit(visit_id: i64, exited_at: u64) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    if let Err(e) = state.client.execute(
        "UPDATE area_visits SET exited_at = $1 WHERE id = $2",
        &[&(exited_at as i64), &(visit_id as i32)],
    ) {
        tracing::warn!("close_area_visit: DB error: {e}");
    }
}

/// Returns `true` if a Pokémon with this species ID exists in the `caught_pokemon`
/// table for the active run under any player.
///
/// Used to enforce the dupes clause: when enabled, a new encounter is skipped if
/// the species was already caught at any point in the current run, regardless of
/// which area it was encountered in.
pub fn species_caught_any(species: u16) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon WHERE run_id = $1 AND species = $2",
            &[&(active as i32), &(species as i32)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns `true` if a Pokémon with this species ID exists in the `caught_pokemon`
/// table for the active run under the **current player only**.
///
/// Used to enforce the per-player dupes clause: a new encounter is skipped if
/// this player has already caught the species at any point in the current run.
pub fn species_caught_by_self(species: u16) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.effective_player_name();
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon \
             WHERE run_id = $1 AND player_name = $2 AND species = $3",
            &[&(active as i32), &player, &(species as i32)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns `true` if this species has already been recorded as a first encounter
/// anywhere in the active run for the current player.
pub fn species_encountered(species: u16) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.effective_player_name();
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM encounters
             WHERE run_id = $1 AND player_name = $2 AND species = $3",
            &[&(active as i32), &player, &(species as i32)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns `true` if any encounters have been recorded for the active run.
/// Used at startup to seed the pre-ball latch: if the run already has
/// encounters the player must have had balls at some point this run.
pub fn has_any_encounters() -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    state
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM encounters WHERE run_id = $1)",
            &[&(active as i32)],
        )
        .map(|row| row.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Returns `true` if an encounter has already been recorded for **any** of the
/// given `(map_group, map_name)` pairs by the current player in the active run.
///
/// Pass the slice returned by `fire_red_location_names::dungeon_floors` to
/// check whether any floor of a multi-floor dungeon is already claimed.
/// Returns `false` immediately for an empty slice.
pub fn has_encounter_for_any_floor(floors: &[(u8, u8)]) -> bool {
    if floors.is_empty() {
        return false;
    }
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.effective_player_name();

    // Build a single EXISTS query with one OR-clause per floor so we only
    // issue one round-trip instead of N while holding the DB mutex.
    // N is always small (≤5 for multi-floor dungeons), so dynamic query
    // construction is safe and the query planner handles it fine.
    use std::fmt::Write as _;
    let mut cond = String::new();
    let floor_pairs: Vec<(i32, i32)> = floors
        .iter()
        .map(|&(mg, mn)| (mg as i32, mn as i32))
        .collect();
    for i in 0..floor_pairs.len() {
        if i > 0 {
            cond.push_str(" OR ");
        }
        write!(
            &mut cond,
            "(map_group=${} AND map_name=${})",
            3 + i * 2,
            4 + i * 2
        )
        .unwrap();
    }
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM encounters \
         WHERE run_id = $1 AND player_name = $2 AND ({cond}))"
    );
    let active_i32 = active as i32;
    let mut params: Vec<&(dyn postgres::types::ToSql + Sync)> = vec![&active_i32, &player];
    for (mg, mn) in &floor_pairs {
        params.push(mg);
        params.push(mn);
    }
    state
        .client
        .query_one(&sql, &params)
        .map(|row| row.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Returns `true` if an encounter has already been recorded for this area by the current player.
pub fn has_encounter(map_group: u8, map_name: u8) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.effective_player_name();
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM encounters
             WHERE run_id = $1 AND player_name = $2 AND map_group = $3 AND map_name = $4",
            &[
                &(active as i32),
                &player,
                &(map_group as i32),
                &(map_name as i32),
            ],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns all encounters for the active run, ordered by time.
pub fn list_encounters() -> Vec<Encounter> {
    let Some(db) = db() else { return vec![] };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return vec![],
    };
    let player = state.effective_player_name();
    state.client
        .query(
            "SELECT player_name, map_group, map_name, species, species_name, level, caught, encountered_at, is_shiny
             FROM encounters
             WHERE run_id = $1 AND player_name = $2
             ORDER BY encountered_at ASC",
            &[&(active as i32), &player],
        )
        .unwrap_or_default()
        .iter()
        .map(|row| Encounter {
            player_name:    row.get(0),
            map_group:      row.get::<_, i32>(1) as u8,
            map_name:       row.get::<_, i32>(2) as u8,
            species:        row.get::<_, i32>(3) as u16,
            species_name:   row.get(4),
            level:          row.get::<_, i32>(5) as u8,
            caught:         row.get(6),
            encountered_at: row.get::<_, i64>(7) as u64,
            is_shiny:       row.get(8),
        })
        .collect()
}

/// Upserts a soul-link override for the active run: `personality` will be
/// linked to `partner_personality` regardless of met_location.
///
/// Replaces any existing override for the same personality in this run.
pub fn set_soul_link_override(personality: u32, partner_personality: u32) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => {
            tracing::warn!("set_soul_link_override: no active run");
            return;
        }
    };
    let p = personality as i64;
    let pp = partner_personality as i64;
    let now = unix_now() as i64;
    let run_i32 = active as i32;
    if let Err(e) = state.client.execute(
        "INSERT INTO soul_link_overrides (run_id, personality, partner_personality, created_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (run_id, personality)
         DO UPDATE SET partner_personality = EXCLUDED.partner_personality,
                       created_at          = EXCLUDED.created_at",
        &[&run_i32, &p, &pp, &now],
    ) {
        tracing::warn!("set_soul_link_override: DB error: {e}");
    }
}

/// Removes the soul-link override for `personality` in the active run.
///
/// After this call the automatic met_location / receipt-order pairing resumes.
pub fn clear_soul_link_override(personality: u32) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return,
    };
    let p = personality as i64;
    let run_i32 = active as i32;
    if let Err(e) = state.client.execute(
        "DELETE FROM soul_link_overrides WHERE run_id = $1 AND personality = $2",
        &[&run_i32, &p],
    ) {
        tracing::warn!("clear_soul_link_override: DB error: {e}");
    }
}
