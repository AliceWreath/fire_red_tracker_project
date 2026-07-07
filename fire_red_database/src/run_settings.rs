//! Injection presets, per-run rules, and per-player display slot pins.

use super::*;

// ---------------------------------------------------------------------------
// Presets (v17)
// ---------------------------------------------------------------------------

/// Save or replace a named party preset. `config_json` should be a JSON array
/// of `ClientMessage`-compatible command objects (the caller serialises it).
pub fn save_preset(conn_str: &str, name: &str, config_json: &str) -> Result<(), String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let now = unix_now() as i64;
    client
        .execute(
            "INSERT INTO presets (name, config, created_at) VALUES ($1, $2, $3)
             ON CONFLICT (name) DO UPDATE SET config = EXCLUDED.config, created_at = EXCLUDED.created_at",
            &[&name, &config_json, &now],
        )
        .map_err(|e| format!("Failed to save preset: {e}"))?;
    Ok(())
}

/// Return all presets as `{ "presets": [ { "name": ..., "commands": [...], "created_at": ... } ] }`.
pub fn list_presets(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query("SELECT name, config, created_at FROM presets ORDER BY name", &[]) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let presets: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let config: String = r.get(1);
            let created_at: i64 = r.get(2);
            let commands: serde_json::Value =
                serde_json::from_str(&config).unwrap_or(serde_json::Value::Array(vec![]));
            serde_json::json!({
                "name": name,
                "commands": commands,
                "created_at": format_timestamp(created_at as u64),
            })
        })
        .collect();
    serde_json::json!({ "presets": presets })
}

/// Fetch the command list for a single preset by name.
/// Returns `None` if the preset does not exist.
pub fn get_preset(conn_str: &str, name: &str) -> Option<serde_json::Value> {
    let mut client = Client::connect(conn_str, NoTls).ok()?;
    let row = client
        .query_opt("SELECT config FROM presets WHERE name = $1", &[&name])
        .ok()??;
    let config: String = row.get(0);
    serde_json::from_str(&config).ok()
}

/// Delete a preset by name. Returns `true` if a row was removed.
pub fn delete_preset(conn_str: &str, name: &str) -> bool {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .execute("DELETE FROM presets WHERE name = $1", &[&name])
        .map(|n| n > 0)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Run rules (v18)
// ---------------------------------------------------------------------------

/// Return the challenge-rule flags for a run.
/// Inserts a default all-false row on first access.
pub fn get_run_rules(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rid = run_id as i32;
    let row = client
        .query_opt(
            "SELECT duplicate_clause, species_clause, gift_clause, shiny_clause, updated_at
             FROM run_rules WHERE run_id = $1",
            &[&rid],
        )
        .unwrap_or(None);
    match row {
        Some(r) => serde_json::json!({
            "run_id": run_id,
            "duplicate_clause": r.get::<_, bool>(0),
            "species_clause":   r.get::<_, bool>(1),
            "gift_clause":      r.get::<_, bool>(2),
            "shiny_clause":     r.get::<_, bool>(3),
            "updated_at":       format_timestamp(r.get::<_, i64>(4) as u64),
        }),
        None => serde_json::json!({
            "run_id": run_id,
            "duplicate_clause": false,
            "species_clause":   false,
            "gift_clause":      false,
            "shiny_clause":     false,
            "updated_at":       null,
        }),
    }
}

/// Upsert the challenge-rule flags for a run. Only fields present in `patch`
/// are changed; others keep their current value.
pub fn set_run_rules(conn_str: &str, run_id: u32, patch: &serde_json::Value) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rid = run_id as i32;
    let now = unix_now() as i64;

    // Read existing or default.
    let existing = client
        .query_opt(
            "SELECT duplicate_clause, species_clause, gift_clause, shiny_clause
             FROM run_rules WHERE run_id = $1",
            &[&rid],
        )
        .unwrap_or(None);
    let (mut dup, mut spc, mut gift, mut shiny) = match existing {
        Some(r) => (
            r.get::<_, bool>(0),
            r.get::<_, bool>(1),
            r.get::<_, bool>(2),
            r.get::<_, bool>(3),
        ),
        None => (false, false, false, false),
    };

    if let Some(v) = patch.get("duplicate_clause").and_then(|v| v.as_bool()) { dup = v; }
    if let Some(v) = patch.get("species_clause").and_then(|v| v.as_bool())   { spc = v; }
    if let Some(v) = patch.get("gift_clause").and_then(|v| v.as_bool())      { gift = v; }
    if let Some(v) = patch.get("shiny_clause").and_then(|v| v.as_bool())     { shiny = v; }

    if let Err(e) = client.execute(
        "INSERT INTO run_rules (run_id, duplicate_clause, species_clause, gift_clause, shiny_clause, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (run_id) DO UPDATE
         SET duplicate_clause = EXCLUDED.duplicate_clause,
             species_clause   = EXCLUDED.species_clause,
             gift_clause      = EXCLUDED.gift_clause,
             shiny_clause     = EXCLUDED.shiny_clause,
             updated_at       = EXCLUDED.updated_at",
        &[&rid, &dup, &spc, &gift, &shiny, &now],
    ) {
        return serde_json::json!({ "error": format!("Failed to upsert run_rules: {e}") });
    }

    serde_json::json!({
        "run_id": run_id,
        "duplicate_clause": dup,
        "species_clause":   spc,
        "gift_clause":      gift,
        "shiny_clause":     shiny,
        "updated_at":       format_timestamp(now as u64),
    })
}

// ---------------------------------------------------------------------------
// Per-player slot index (display column order) for a run
// ---------------------------------------------------------------------------
//
// A soul-link/co-op run can have several physical connections sharing one
// `runs` row (see `DbReader::sync_player`'s "all connected trackers share a
// single run" note), each tagged by its own `player_name`. So the pinned
// display column has to be keyed by (run_id, player_name), not just run_id —
// otherwise pinning one player's column edits the same row the other player
// reads from. `slot_index` is 1-indexed (1 = leftmost column) throughout.

/// Returns every pinned (player_name, slot_index) pair recorded for a run.
pub fn get_run_player_slots(conn_str: &str, run_id: u32) -> Vec<(String, u8)> {
    let Ok(mut client) = Client::connect(conn_str, NoTls) else { return vec![] };
    let Ok(rows) = client.query(
        "SELECT player_name, slot_index FROM run_player_slots WHERE run_id = $1",
        &[&(run_id as i32)],
    ) else { return vec![] };
    rows.iter()
        .filter_map(|row| {
            let idx: i32 = row.get(1);
            u8::try_from(idx).ok().map(|idx| (row.get::<_, String>(0), idx))
        })
        .collect()
}

/// Set (or clear) the display-column index for one player within a run.
///
/// `owner_id` must match the run's `user_id`; returns an error string otherwise.
/// Pass `slot_index = None` to remove the pin (falls back to auto-ordering).
pub fn set_run_player_slot_index(
    conn_str: &str,
    run_id: u32,
    owner_id: u32,
    player_name: &str,
    slot_index: Option<u8>,
) -> Result<(), String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let row = client
        .query_opt("SELECT user_id FROM runs WHERE id = $1", &[&rid])
        .map_err(|e| format!("DB query failed: {e}"))?
        .ok_or_else(|| format!("run {run_id} not found"))?;
    let stored_owner: Option<i32> = row.get(0);
    if stored_owner != Some(owner_id as i32) {
        return Err("only the run owner can set the slot index".to_string());
    }

    match slot_index {
        Some(idx) => {
            client.execute(
                "INSERT INTO run_player_slots (run_id, player_name, slot_index)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (run_id, player_name) DO UPDATE SET slot_index = EXCLUDED.slot_index",
                &[&rid, &player_name, &(idx as i32)],
            ).map_err(|e| format!("DB update failed: {e}"))?;
        }
        None => {
            client.execute(
                "DELETE FROM run_player_slots WHERE run_id = $1 AND player_name = $2",
                &[&rid, &player_name],
            ).map_err(|e| format!("DB update failed: {e}"))?;
        }
    }
    Ok(())
}
