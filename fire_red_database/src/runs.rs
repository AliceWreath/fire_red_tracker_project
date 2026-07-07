//! Run lifecycle: create, resume, end, list; player-name updates.

use super::*;

// ---------------------------------------------------------------------------
// Public API — run management
// ---------------------------------------------------------------------------

/// Creates a fresh run, sets it as active in this process, and returns its ID.
pub fn new_run(player_name: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Ok(0) };
    let mut state = db.lock_or_recover();
    let row = state
        .client
        .query_one(
            "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
            &[&player_name, &(unix_now() as i64)],
        )
        .map_err(|e| format!("Failed to insert run: {e}"))?;
    let id = row.get::<_, i32>(0) as u32;
    state.run_id = Some(id);
    set_meta(&mut state.client, "active_run_id", &id.to_string());
    Ok(id)
}

/// Switches the active run for this process to an existing run by ID.
///
/// Returns `Ok(false)` if no run with that ID exists.
pub fn resume_run(id: u32) -> Result<bool, String> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let exists = state
        .client
        .query_opt("SELECT 1 FROM runs WHERE id = $1", &[&(id as i32)])
        .map_err(|e| format!("Failed to query runs: {e}"))?
        .is_some();
    if exists {
        // Update only the persisted metadata (used by --list-runs and tracker
        // startup). Do NOT touch state.run_id — mutating the global would
        // silently redirect writes from all tracker-TCP game-loop threads to
        // this run ID until they restart.
        set_meta(&mut state.client, "active_run_id", &id.to_string());
    }
    Ok(exists)
}

/// Returns the active run ID for this process, falling back to the most
/// recently created run. Creates a new run if none exist.
pub fn get_or_create_run(player_name: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Ok(0) };
    let mut state = db.lock_or_recover();

    // Already selected in this session — keep it.
    if let Some(id) = state.run_id {
        return Ok(id);
    }

    // Fall back to the most recently created run — all trackers share one run.
    if let Some(row) = state
        .client
        .query_opt("SELECT id FROM runs ORDER BY id DESC LIMIT 1", &[])
        .map_err(|e| format!("Failed to query runs: {e}"))?
    {
        let id = row.get::<_, i32>(0) as u32;
        state.run_id = Some(id);
        set_meta(&mut state.client, "active_run_id", &id.to_string());
        return Ok(id);
    }

    // No runs at all — create one.
    let row = state
        .client
        .query_one(
            "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
            &[&player_name, &(unix_now() as i64)],
        )
        .map_err(|e| format!("Failed to insert run: {e}"))?;
    let id = row.get::<_, i32>(0) as u32;
    state.run_id = Some(id);
    set_meta(&mut state.client, "active_run_id", &id.to_string());
    Ok(id)
}

/// Updates the player name once it is known from the game.
///
/// Stores the name in-process for tagging all subsequent DB writes, and updates
/// the run row if it still holds the placeholder 'Unknown'.
///
/// This sets the *global* fallback only. In direct mode, where multiple
/// game-loop threads may share one process (e.g. two players in a soul-link
/// run), each thread must call [`set_thread_player_name`] instead so its
/// writes are tagged with its own name rather than racing with other threads
/// over this global value.
pub fn set_player_name(name: &str) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    state.current_player = name.to_string();
    if let Some(id) = state.effective_run_id()
        && let Err(e) = state.client.execute(
            "UPDATE runs SET player_name = $1 WHERE id = $2 AND player_name = 'Unknown'",
            &[&name, &(id as i32)],
        )
    {
        tracing::warn!("Failed to update player name: {e}");
    }
}

/// Returns the run ID active in this process (or the last-written one from
/// the meta table, which is useful for the `--list-runs` display before
/// a run has been selected in the current session).
pub fn active_run_id() -> Option<u32> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    state
        .run_id
        .or_else(|| get_meta(&mut state.client, "active_run_id").and_then(|v| v.parse().ok()))
}

/// Returns `(player_name, started_at)` for the given run ID using the global DB connection.
pub fn get_run_info(run_id: u32) -> Option<(String, u64)> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT player_name, started_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).ok()??;
    Some((row.get(0), row.get::<_, i64>(1) as u64))
}

/// Ends the active run by recording its end timestamp and clearing the
/// in-process run ID. Subsequent writes (deaths, encounters, catches)
/// will be silently dropped until a new run is started.
///
/// Returns the ID of the run that was ended, or `None` if no run was active.
pub fn end_run() -> Option<u32> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let id = state.run_id.take()?;
    if let Err(e) = state.client.execute(
        "UPDATE runs SET ended_at = $1 WHERE id = $2",
        &[&(unix_now() as i64), &(id as i32)],
    ) {
        tracing::warn!("Failed to record run end time: {e}");
    }
    delete_meta(&mut state.client, "active_run_id");
    Some(id)
}

/// End a specific run by ID, verifying the caller owns it.
/// Returns `Err` if the DB is not initialised, the run doesn't exist, the
/// caller doesn't own it, or the run is already ended.
pub fn end_run_by_id(run_id: u32, user_id: u32) -> Result<(), String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT user_id, ended_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| "run not found".to_string())?;
    let owner_id: Option<i32> = row.get(0);
    // Allow only if the run's owner matches the caller.
    // Ownerless runs (created before the auth system) require server owner (user 1).
    let caller = user_id as i32;
    let allowed = match owner_id {
        Some(oid) => oid == caller,
        None => caller == 1,
    };
    if !allowed {
        return Err("you do not own this run".to_string());
    }
    let already_ended: Option<i64> = row.get(1);
    if already_ended.is_some() {
        return Err("run is already ended".to_string());
    }
    state.client.execute(
        "UPDATE runs SET ended_at = $1 WHERE id = $2",
        &[&(unix_now() as i64), &(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Returns a summary of every run: `(id, player_name, started_at, dead_count)`.
pub fn list_runs() -> Result<Vec<(u32, String, u64, usize)>, String> {
    let Some(db) = db() else { return Ok(vec![]) };
    let mut state = db.lock_or_recover();
    let rows = state
        .client
        .query(
            "SELECT r.id, r.player_name, r.started_at, COUNT(d.personality)
             FROM runs r
             LEFT JOIN dead_pokemon d ON d.run_id = r.id
             GROUP BY r.id
             ORDER BY r.id",
            &[],
        )
        .map_err(|e| format!("Failed to query runs: {e}"))?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get::<_, i32>(0) as u32,
                row.get(1),
                row.get::<_, i64>(2) as u64,
                row.get::<_, i64>(3) as usize,
            )
        })
        .collect())
}
