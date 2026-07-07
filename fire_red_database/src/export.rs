//! Export and import: run JSON/CSV exports, full-DB snapshots for
//! scheduled backups, run import, events/timeline JSON, and Pokepaste.

use super::*;

pub fn clear_all_records(conn_str: &str) -> Result<(), String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    client
        .batch_execute(
            "
        DELETE FROM encounters;
        DELETE FROM caught_pokemon;
        DELETE FROM dead_pokemon;
        DELETE FROM runs;
        DELETE FROM meta WHERE key = 'active_run_id';
    ",
        )
        .map_err(|e| format!("Clear failed: {e}"))
}

/// Returns a JSON export of a single run: metadata, caught, dead, and encounter lists.
///
/// Opens its own connection so the live tracker connection is not blocked.
pub fn export_run(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rid = run_id as i32;

    let run_row = match client.query_opt(
        "SELECT id, player_name, started_at, ended_at FROM runs WHERE id = $1",
        &[&rid],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return serde_json::json!({ "error": format!("Run {run_id} not found") }),
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };

    let caught_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, location_name, caught_at, player_name, personality, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM caught_pokemon WHERE run_id = $1 ORDER BY caught_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run caught query failed for run {run_id}: {e}");
            vec![]
        });

    let dead_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, died_at, player_name, is_soul_link_death, personality, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run dead query failed for run {run_id}: {e}");
            vec![]
        });

    let enc_rows = client
        .query(
            "SELECT species_name, level, map_group, map_name, caught, is_shiny, \
                encountered_at, player_name \
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run encounters query failed for run {run_id}: {e}");
            vec![]
        });

    serde_json::json!({
        "run": {
            "id":          run_row.get::<_, i32>(0),
            "player_name": run_row.get::<_, String>(1),
            "started_at":  format_timestamp(run_row.get::<_, i64>(2) as u64),
            "ended_at":    run_row.get::<_, Option<i64>>(3).map(|t| format_timestamp(t as u64)),
        },
        "caught": caught_rows.iter().map(|r| serde_json::json!({
            "nickname":      r.get::<_, String>(0),
            "species_name":  r.get::<_, String>(1),
            "level":         r.get::<_, i32>(2),
            "nature":        r.get::<_, String>(3),
            "is_shiny":      r.get::<_, bool>(4),
            "gender":        r.get::<_, i32>(5),
            "met_location":  r.get::<_, i32>(6),
            "location_name": r.get::<_, String>(7),
            "caught_at":     format_timestamp(r.get::<_, i64>(8) as u64),
            "player_name":   r.get::<_, String>(9),
            "personality":   r.get::<_, i64>(10),
            "iv_hp":         r.get::<_, i32>(11),
            "iv_atk":        r.get::<_, i32>(12),
            "iv_def":        r.get::<_, i32>(13),
            "iv_spe":        r.get::<_, i32>(14),
            "iv_spa":        r.get::<_, i32>(15),
            "iv_spd":        r.get::<_, i32>(16),
            "ev_hp":         r.get::<_, i32>(17),
            "ev_atk":        r.get::<_, i32>(18),
            "ev_def":        r.get::<_, i32>(19),
            "ev_spe":        r.get::<_, i32>(20),
            "ev_spa":        r.get::<_, i32>(21),
            "ev_spd":        r.get::<_, i32>(22),
        })).collect::<Vec<_>>(),
        "dead": dead_rows.iter().map(|r| serde_json::json!({
            "nickname":          r.get::<_, String>(0),
            "species_name":      r.get::<_, String>(1),
            "level":             r.get::<_, i32>(2),
            "nature":            r.get::<_, String>(3),
            "is_shiny":          r.get::<_, bool>(4),
            "gender":            r.get::<_, i32>(5),
            "met_location":      r.get::<_, i32>(6),
            "died_at":           format_timestamp(r.get::<_, i64>(7) as u64),
            "player_name":       r.get::<_, String>(8),
            "is_soul_link_death": r.get::<_, bool>(9),
            "personality":       r.get::<_, i64>(10),
            "iv_hp":             r.get::<_, i32>(11),
            "iv_atk":            r.get::<_, i32>(12),
            "iv_def":            r.get::<_, i32>(13),
            "iv_spe":            r.get::<_, i32>(14),
            "iv_spa":            r.get::<_, i32>(15),
            "iv_spd":            r.get::<_, i32>(16),
            "ev_hp":             r.get::<_, i32>(17),
            "ev_atk":            r.get::<_, i32>(18),
            "ev_def":            r.get::<_, i32>(19),
            "ev_spe":            r.get::<_, i32>(20),
            "ev_spa":            r.get::<_, i32>(21),
            "ev_spd":            r.get::<_, i32>(22),
        })).collect::<Vec<_>>(),
        "encounters": enc_rows.iter().map(|r| serde_json::json!({
            "species_name":   r.get::<_, String>(0),
            "level":          r.get::<_, i32>(1),
            "map_group":      r.get::<_, i32>(2),
            "map_name":       r.get::<_, i32>(3),
            "caught":         r.get::<_, bool>(4),
            "is_shiny":       r.get::<_, bool>(5),
            "encountered_at": format_timestamp(r.get::<_, i64>(6) as u64),
            "player_name":    r.get::<_, String>(7),
        })).collect::<Vec<_>>(),
    })
}

/// Returns a JSON snapshot of every run in the database, for scheduled
/// backups. Each element of `runs` has the same shape as [`export_run`]'s
/// output, so any single run can be restored through the existing
/// `/api/run/import` path.
///
/// Opens its own connection so the live tracker connection is not blocked.
pub fn export_all_runs(conn_str: &str) -> Result<serde_json::Value, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let ids: Vec<u32> = client
        .query("SELECT id FROM runs ORDER BY id", &[])
        .map_err(|e| format!("Query failed: {e}"))?
        .iter()
        .map(|r| r.get::<_, i32>(0) as u32)
        .collect();
    drop(client);

    let runs: Vec<serde_json::Value> = ids.iter().map(|&id| export_run(conn_str, id)).collect();
    Ok(serde_json::json!({
        "format": "fire_red_tracker_backup",
        "version": 1,
        "created_at": unix_now(),
        "run_count": runs.len(),
        "runs": runs,
    }))
}

/// Returns a CSV export of a single run: three sections separated by blank lines.
///
/// Sections: `caught`, `dead`, `encounters`. Each section has a header row.
/// Opens its own connection so the live tracker is not blocked.
pub fn export_run_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let mut out = String::new();

    // Caught Pokémon
    out.push_str(
        "section,player_name,nickname,species_name,level,nature,is_shiny,gender,\
met_location,location_name,iv_hp,iv_atk,iv_def,iv_spe,iv_spa,iv_spd,\
ev_hp,ev_atk,ev_def,ev_spe,ev_spa,ev_spd,caught_at\n",
    );
    let caught_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, location_name, caught_at, player_name, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM caught_pokemon WHERE run_id = $1 ORDER BY caught_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run_csv caught query failed for run {run_id}: {e}");
            vec![]
        });
    for r in &caught_rows {
        out.push_str(&format!(
            "caught,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(9)), // player_name
            csv_field(r.get::<_, String>(0)), // nickname
            csv_field(r.get::<_, String>(1)), // species_name
            r.get::<_, i32>(2),               // level
            csv_field(r.get::<_, String>(3)), // nature
            r.get::<_, bool>(4),              // is_shiny
            r.get::<_, i32>(5),               // gender
            r.get::<_, i32>(6),               // met_location
            csv_field(r.get::<_, String>(7)), // location_name
            r.get::<_, i32>(10),              // iv_hp
            r.get::<_, i32>(11),              // iv_atk
            r.get::<_, i32>(12),              // iv_def
            r.get::<_, i32>(13),              // iv_spe
            r.get::<_, i32>(14),              // iv_spa
            r.get::<_, i32>(15),              // iv_spd
            r.get::<_, i32>(16),              // ev_hp
            r.get::<_, i32>(17),              // ev_atk
            r.get::<_, i32>(18),              // ev_def
            r.get::<_, i32>(19),              // ev_spe
            r.get::<_, i32>(20),              // ev_spa
            r.get::<_, i32>(21),              // ev_spd
            csv_field(format_timestamp(r.get::<_, i64>(8) as u64)), // caught_at
        ));
    }

    out.push('\n');

    // Dead Pokémon
    out.push_str(
        "section,player_name,nickname,species_name,level,nature,is_shiny,gender,\
met_location,soul_link_death,iv_hp,iv_atk,iv_def,iv_spe,iv_spa,iv_spd,\
ev_hp,ev_atk,ev_def,ev_spe,ev_spa,ev_spd,died_at\n",
    );
    let dead_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, died_at, player_name, is_soul_link_death, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run_csv dead query failed for run {run_id}: {e}");
            vec![]
        });
    for r in &dead_rows {
        out.push_str(&format!(
            "dead,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(8)), // player_name
            csv_field(r.get::<_, String>(0)), // nickname
            csv_field(r.get::<_, String>(1)), // species_name
            r.get::<_, i32>(2),               // level
            csv_field(r.get::<_, String>(3)), // nature
            r.get::<_, bool>(4),              // is_shiny
            r.get::<_, i32>(5),               // gender
            r.get::<_, i32>(6),               // met_location
            r.get::<_, bool>(9),              // soul_link_death
            r.get::<_, i32>(10),              // iv_hp
            r.get::<_, i32>(11),              // iv_atk
            r.get::<_, i32>(12),              // iv_def
            r.get::<_, i32>(13),              // iv_spe
            r.get::<_, i32>(14),              // iv_spa
            r.get::<_, i32>(15),              // iv_spd
            r.get::<_, i32>(16),              // ev_hp
            r.get::<_, i32>(17),              // ev_atk
            r.get::<_, i32>(18),              // ev_def
            r.get::<_, i32>(19),              // ev_spe
            r.get::<_, i32>(20),              // ev_spa
            r.get::<_, i32>(21),              // ev_spd
            csv_field(format_timestamp(r.get::<_, i64>(7) as u64)), // died_at
        ));
    }

    out.push('\n');

    // Encounters
    out.push_str("section,player_name,species_name,level,map_group,map_name,caught,is_shiny,encountered_at\n");
    let enc_rows = client
        .query(
            "SELECT species_name, level, map_group, map_name, caught, is_shiny, \
                encountered_at, player_name \
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run_csv encounters query failed for run {run_id}: {e}");
            vec![]
        });
    for r in &enc_rows {
        out.push_str(&format!(
            "encounter,{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(7)),
            csv_field(r.get::<_, String>(0)),
            r.get::<_, i32>(1),
            r.get::<_, i32>(2),
            r.get::<_, i32>(3),
            r.get::<_, bool>(4),
            r.get::<_, bool>(5),
            csv_field(format_timestamp(r.get::<_, i64>(6) as u64)),
        ));
    }

    Ok(out)
}

/// Returns a summary JSON array of every run: id, player_name, started_at,
/// ended_at, deaths, catches, and encounter count.
///
/// Opens its own connection so the live tracker is not blocked.
pub fn list_all_runs_json(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT r.id, r.player_name, r.started_at, r.ended_at,
                COUNT(DISTINCT d.personality)  AS deaths,
                COUNT(DISTINCT c.personality)  AS catches,
                COUNT(DISTINCT e.id)           AS encounters
         FROM runs r
         LEFT JOIN dead_pokemon  d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         LEFT JOIN encounters     e ON e.run_id = r.id
         GROUP BY r.id
         ORDER BY r.id DESC",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let runs: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let started: i64 = row.get(2);
            let ended: Option<i64> = row.get(3);
            serde_json::json!({
                "id":          row.get::<_, i32>(0),
                "player_name": row.get::<_, String>(1),
                "started_at":  format_timestamp(started as u64),
                "ended_at":    ended.map(|t| format_timestamp(t as u64)),
                "deaths":      row.get::<_, i64>(4),
                "catches":     row.get::<_, i64>(5),
                "encounters":  row.get::<_, i64>(6),
            })
        })
        .collect();
    serde_json::json!({ "runs": runs })
}

/// Returns all runs owned by `user_id` in the same shape as [`list_all_runs_json`].
pub fn list_runs_for_user_json(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT r.id, r.player_name, r.started_at, r.ended_at,
                COUNT(DISTINCT d.personality)  AS deaths,
                COUNT(DISTINCT c.personality)  AS catches,
                COUNT(DISTINCT e.id)           AS encounters,
                (r.user_id = $1)               AS is_owner
         FROM runs r
         LEFT JOIN dead_pokemon   d  ON d.run_id = r.id
         LEFT JOIN caught_pokemon c  ON c.run_id = r.id
         LEFT JOIN encounters     e  ON e.run_id = r.id
         WHERE r.user_id = $1
            OR EXISTS (
                SELECT 1 FROM run_invites ri
                WHERE ri.run_id = r.id
                  AND ri.invited_user = $1
                  AND ri.status = 'accepted'
            )
         GROUP BY r.id, r.user_id
         ORDER BY r.id DESC",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let runs: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let started: i64 = row.get(2);
            let ended: Option<i64> = row.get(3);
            let is_owner: bool = row.get(7);
            serde_json::json!({
                "id":          row.get::<_, i32>(0),
                "player_name": row.get::<_, String>(1),
                "started_at":  format_timestamp(started as u64),
                "ended_at":    ended.map(|t| format_timestamp(t as u64)),
                "deaths":      row.get::<_, i64>(4),
                "catches":     row.get::<_, i64>(5),
                "encounters":  row.get::<_, i64>(6),
                "is_owner":    is_owner,
            })
        })
        .collect();
    serde_json::json!({ "runs": runs })
}

/// Imports a run from the JSON format produced by [`export_run`].
///
/// Creates a new `runs` row and re-inserts every caught, dead, and encounter
/// record from the export. The original run id is **not** preserved — a new
/// id is assigned so there are no conflicts with existing data.
///
/// Original `personality` values, timestamps (`caught_at`, `died_at`,
/// `encountered_at`), `is_soul_link_death`, and the run's `started_at`/`ended_at`
/// are all preserved from the export JSON. Exports produced before these fields
/// were added fall back to safe defaults (synthetic personalities, import time).
///
/// Returns `{ "run_id": <new_id> }` on success or `{ "error": "..." }` on failure.
pub fn import_run(conn_str: &str, body: &serde_json::Value) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let run_obj = match body.get("run") {
        Some(v) => v,
        None => return serde_json::json!({ "error": "missing 'run' field" }),
    };

    let player_name = run_obj
        .get("player_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Imported");

    // Preserve original run timestamps; fall back to now if absent (old exports).
    let now = unix_now() as i64;
    let started_at: i64 = run_obj
        .get("started_at")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp)
        .map(|t| t as i64)
        .unwrap_or(now);
    let ended_at: Option<i64> = run_obj
        .get("ended_at")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp)
        .map(|t| t as i64);

    let new_id: i32 = match client.query_one(
        "INSERT INTO runs (player_name, started_at, ended_at) VALUES ($1, $2, $3) RETURNING id",
        &[&player_name, &started_at, &ended_at],
    ) {
        Ok(row) => row.get(0),
        Err(e) => return serde_json::json!({ "error": format!("Failed to create run: {e}") }),
    };

    // Re-insert encounters.
    if let Some(encounters) = body.get("encounters").and_then(|v| v.as_array()) {
        for enc in encounters {
            let species_name = enc
                .get("species_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let level = enc.get("level").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let map_group = enc.get("map_group").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let map_name = enc.get("map_name").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let caught = enc.get("caught").and_then(|v| v.as_bool()).unwrap_or(false);
            let is_shiny = enc
                .get("is_shiny")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let enc_player = enc
                .get("player_name")
                .and_then(|v| v.as_str())
                .unwrap_or(player_name);
            let encountered_at: i64 = enc
                .get("encountered_at")
                .and_then(|v| v.as_str())
                .and_then(parse_timestamp)
                .map(|t| t as i64)
                .unwrap_or(now);
            match client.execute(
                "INSERT INTO encounters (run_id, player_name, species_name, level, \
                                        map_group, map_name, caught, is_shiny, encountered_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT DO NOTHING",
                &[
                    &new_id,
                    &enc_player,
                    &species_name,
                    &level,
                    &map_group,
                    &map_name,
                    &caught,
                    &is_shiny,
                    &encountered_at,
                ],
            ) {
                Ok(0) => tracing::warn!(
                    "import_run: encounter ({species_name}, map {map_group}/{map_name}, \
                     player {enc_player}) already exists in run {new_id}; skipped"
                ),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("import_run: failed to insert encounter ({species_name}): {e}")
                }
            }
        }
    }

    // Re-insert caught.
    if let Some(caught_list) = body.get("caught").and_then(|v| v.as_array()) {
        for (idx, c) in caught_list.iter().enumerate() {
            let nickname = c.get("nickname").and_then(|v| v.as_str()).unwrap_or("");
            let species_name = c.get("species_name").and_then(|v| v.as_str()).unwrap_or("");
            let level = c.get("level").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let nature = c.get("nature").and_then(|v| v.as_str()).unwrap_or("");
            let is_shiny = c.get("is_shiny").and_then(|v| v.as_bool()).unwrap_or(false);
            let gender = c.get("gender").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let met_location = c.get("met_location").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let location_name = c
                .get("location_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let c_player = c
                .get("player_name")
                .and_then(|v| v.as_str())
                .unwrap_or(player_name);
            // Use original personality; fall back to a synthetic unique value for
            // exports produced before this field was added.
            let personality: i64 = c
                .get("personality")
                .and_then(|v| v.as_i64())
                .unwrap_or(new_id as i64 * 10_000 + idx as i64 + 1);
            let caught_at: i64 = c
                .get("caught_at")
                .and_then(|v| v.as_str())
                .and_then(parse_timestamp)
                .map(|t| t as i64)
                .unwrap_or(now);
            let iv_hp: i32 = c.get("iv_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_atk: i32 = c.get("iv_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_def: i32 = c.get("iv_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spe: i32 = c.get("iv_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spa: i32 = c.get("iv_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spd: i32 = c.get("iv_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_hp: i32 = c.get("ev_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_atk: i32 = c.get("ev_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_def: i32 = c.get("ev_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spe: i32 = c.get("ev_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spa: i32 = c.get("ev_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spd: i32 = c.get("ev_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            match client.execute(
                "INSERT INTO caught_pokemon (run_id, player_name, personality, ot_id, \
                                            nickname, species, species_name, is_shiny, \
                                            nature, level, met_location, location_name, \
                                            iv_hp, iv_attack, iv_defense, iv_speed, \
                                            iv_sp_attack, iv_sp_defense, \
                                            ev_hp, ev_attack, ev_defense, ev_speed, \
                                            ev_sp_attack, ev_sp_defense, \
                                            caught_at, gender) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12, \
                         $13,$14,$15,$16,$17,$18, $19,$20,$21,$22,$23,$24, $25,$26) \
                 ON CONFLICT (run_id, personality) DO NOTHING",
                &[
                    &new_id,
                    &c_player,
                    &personality,
                    &0i64,
                    &nickname,
                    &0i32,
                    &species_name,
                    &is_shiny,
                    &nature,
                    &level,
                    &met_location,
                    &location_name,
                    &iv_hp,
                    &iv_atk,
                    &iv_def,
                    &iv_spe,
                    &iv_spa,
                    &iv_spd,
                    &ev_hp,
                    &ev_atk,
                    &ev_def,
                    &ev_spe,
                    &ev_spa,
                    &ev_spd,
                    &caught_at,
                    &gender,
                ],
            ) {
                Ok(0) => tracing::warn!(
                    "import_run: caught personality 0x{personality:08X} ({species_name}) already \
                     exists in run {new_id}; skipped — possible duplicate import"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    "import_run: failed to insert caught personality 0x{personality:08X}: {e}"
                ),
            }
        }
    }

    // Re-insert dead.
    if let Some(dead_list) = body.get("dead").and_then(|v| v.as_array()) {
        for (idx, d) in dead_list.iter().enumerate() {
            let nickname = d.get("nickname").and_then(|v| v.as_str()).unwrap_or("");
            let species_name = d.get("species_name").and_then(|v| v.as_str()).unwrap_or("");
            let level = d.get("level").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let nature = d.get("nature").and_then(|v| v.as_str()).unwrap_or("");
            let is_shiny = d.get("is_shiny").and_then(|v| v.as_bool()).unwrap_or(false);
            let gender = d.get("gender").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let met_location = d.get("met_location").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let d_player = d
                .get("player_name")
                .and_then(|v| v.as_str())
                .unwrap_or(player_name);
            // Accept "is_soul_link_death" (current) or "soul_link" (old exports).
            let is_soul_link_death = d
                .get("is_soul_link_death")
                .or_else(|| d.get("soul_link"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let personality: i64 = d
                .get("personality")
                .and_then(|v| v.as_i64())
                .unwrap_or(new_id as i64 * 10_000 + 5_000 + idx as i64 + 1);
            let died_at: i64 = d
                .get("died_at")
                .and_then(|v| v.as_str())
                .and_then(parse_timestamp)
                .map(|t| t as i64)
                .unwrap_or(now);
            let iv_hp: i32 = d.get("iv_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_atk: i32 = d.get("iv_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_def: i32 = d.get("iv_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spe: i32 = d.get("iv_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spa: i32 = d.get("iv_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spd: i32 = d.get("iv_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_hp: i32 = d.get("ev_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_atk: i32 = d.get("ev_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_def: i32 = d.get("ev_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spe: i32 = d.get("ev_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spa: i32 = d.get("ev_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spd: i32 = d.get("ev_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            match client.execute(
                "INSERT INTO dead_pokemon (run_id, player_name, personality, ot_id, \
                                          nickname, species, species_name, is_shiny, \
                                          nature, level, met_location, died_at, \
                                          gender, max_hp, is_soul_link_death, \
                                          experience, attack, defense, speed, sp_attack, sp_defense, \
                                          move1, move2, move3, move4, pp1, pp2, pp3, pp4, \
                                          iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                                          ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense, \
                                          held_item, ability, ability_name, friendship, ot_name) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,0,$14, \
                         0,0,0,0,0,0, 0,0,0,0,0,0,0,0, \
                         $15,$16,$17,$18,$19,$20, $21,$22,$23,$24,$25,$26, 0,0,'',0,'') \
                 ON CONFLICT (run_id, personality) DO NOTHING",
                &[&new_id, &d_player, &personality, &0i64,
                  &nickname, &0i32, &species_name, &is_shiny,
                  &nature, &level, &met_location, &died_at, &gender,
                  &is_soul_link_death,
                  &iv_hp, &iv_atk, &iv_def, &iv_spe, &iv_spa, &iv_spd,
                  &ev_hp, &ev_atk, &ev_def, &ev_spe, &ev_spa, &ev_spd],
            ) {
                Ok(0) => tracing::warn!(
                    "import_run: dead personality 0x{personality:08X} ({species_name}) already \
                     exists in run {new_id}; skipped — possible duplicate import"),
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    "import_run: failed to insert dead personality 0x{personality:08X}: {e}"),
            }
        }
    }

    serde_json::json!({ "run_id": new_id })
}

/// Typed error returned by [`list_events_json`] and [`active_run_timeline_json`].
///
/// HTTP handlers match on variants to assign status codes without string-matching
/// on error text embedded in a JSON body.
#[derive(Debug)]
pub enum EventsError {
    /// No run is currently marked active in the `meta` table.
    NoActiveRun,
    /// The PostgreSQL connection could not be opened.
    ConnectionFailed(String),
    /// A database query failed.
    QueryFailed(String),
}

impl std::fmt::Display for EventsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventsError::NoActiveRun => f.write_str("no active run"),
            EventsError::ConnectionFailed(e) => write!(f, "DB connection failed: {e}"),
            EventsError::QueryFailed(e) => write!(f, "Query failed: {e}"),
        }
    }
}

/// Returns a JSON array of events for the given run ID, ordered by time.
///
/// Opens its own connection so the live tracker is not blocked.
pub fn list_events_json(conn_str: &str, run_id: u32) -> Result<serde_json::Value, EventsError> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| EventsError::ConnectionFailed(e.to_string()))?;
    let rows = client.query(
        "SELECT id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at, note
         FROM events WHERE run_id = $1 ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ).map_err(|e| EventsError::QueryFailed(e.to_string()))?;
    let events: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "id":            row.get::<_, i32>(0),
                "player_name":   row.get::<_, String>(1),
                "event_type":    row.get::<_, String>(2),
                "species_name":  row.get::<_, String>(3),
                "nickname":      row.get::<_, String>(4),
                "old_nickname":  row.get::<_, String>(5),
                "level":         row.get::<_, i32>(6),
                "occurred_at":   format_timestamp(row.get::<_, i64>(7) as u64),
                "note":          row.get::<_, String>(8),
            })
        })
        .collect();
    Ok(serde_json::json!({ "run_id": run_id, "events": events }))
}

/// Returns the chronological event timeline for the **currently active** run.
///
/// Opens its own connection and reads `active_run_id` from the `meta` table
/// directly — this avoids the global [`DB`] singleton, which is only
/// initialised in the tracker process. Calling the previous `active_run_id()`
/// helper from the aggregator process would panic immediately.
///
/// Includes both `occurred_at` as a Unix integer and a human-readable
/// `occurred_at_human` string. Returns [`EventsError`] for the typed result.
pub fn active_run_timeline_json(conn_str: &str) -> Result<serde_json::Value, EventsError> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| EventsError::ConnectionFailed(e.to_string()))?;
    let run_id: u32 = get_meta(&mut client, "active_run_id")
        .and_then(|v| v.parse().ok())
        .ok_or(EventsError::NoActiveRun)?;
    let rows = client.query(
        "SELECT id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at, note
         FROM events WHERE run_id = $1 ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ).map_err(|e| EventsError::QueryFailed(e.to_string()))?;
    let events: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let ts = row.get::<_, i64>(7) as u64;
            serde_json::json!({
                "id":                row.get::<_, i32>(0),
                "player_name":       row.get::<_, String>(1),
                "event_type":        row.get::<_, String>(2),
                "species_name":      row.get::<_, String>(3),
                "nickname":          row.get::<_, String>(4),
                "old_nickname":      row.get::<_, String>(5),
                "level":             row.get::<_, i32>(6),
                "occurred_at":       ts,
                "occurred_at_human": format_timestamp(ts),
                "note":              row.get::<_, String>(8),
            })
        })
        .collect();
    Ok(serde_json::json!({ "run_id": run_id, "events": events }))
}

/// Like [`active_run_timeline_json`] but returns `Err(NoActiveRun)` if the
/// active run is not accessible to `user_id`.
pub fn active_run_timeline_for_user_json(
    conn_str: &str,
    user_id: u32,
) -> Result<serde_json::Value, EventsError> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| EventsError::ConnectionFailed(e.to_string()))?;
    let run_id: u32 = get_meta(&mut client, "active_run_id")
        .and_then(|v| v.parse().ok())
        .ok_or(EventsError::NoActiveRun)?;
    // Check access via global DB handle.
    let accessible = user_can_access_run(run_id, user_id)
        .unwrap_or(false);
    if !accessible {
        return Err(EventsError::NoActiveRun);
    }
    let rows = client.query(
        "SELECT id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at, note
         FROM events WHERE run_id = $1 ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ).map_err(|e| EventsError::QueryFailed(e.to_string()))?;
    let events: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let ts = row.get::<_, i64>(7) as u64;
            serde_json::json!({
                "id":                row.get::<_, i32>(0),
                "player_name":       row.get::<_, String>(1),
                "event_type":        row.get::<_, String>(2),
                "species_name":      row.get::<_, String>(3),
                "nickname":          row.get::<_, String>(4),
                "old_nickname":      row.get::<_, String>(5),
                "level":             row.get::<_, i32>(6),
                "occurred_at":       ts,
                "occurred_at_human": format_timestamp(ts),
                "note":              row.get::<_, String>(8),
            })
        })
        .collect();
    Ok(serde_json::json!({ "run_id": run_id, "events": events }))
}

/// Sets (or clears) the free-text note on an event log entry identified by its
/// `event_id`. Passing an empty string effectively removes the annotation.
///
/// Returns `Ok(())` on success, `Err(message)` if the connection or query fails.
pub fn set_event_note(conn_str: &str, event_id: i32, note: &str) -> Result<(), String> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    client
        .execute(
            "UPDATE events SET note = $1 WHERE id = $2",
            &[&note, &event_id],
        )
        .map_err(|e| format!("Query failed: {e}"))?;
    Ok(())
}

/// Exports the living and fallen Pokémon for `run_id` in
/// [Pokémon Showdown Pokepaste](https://pokepast.es/) format.
///
/// Living party members (caught but not dead) appear first in a `# Living Party`
/// block. Because only the snapshot-at-catch is stored for survivors, move lines
/// are omitted. Fallen members appear in a `# Fallen` block with full moveset,
/// ability, and held-item data.
pub fn pokepaste_export(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let living = client.query(
        "SELECT nickname, species_name, is_shiny, nature, level,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                gender
         FROM caught_pokemon
         WHERE run_id = $1
           AND personality NOT IN (SELECT personality FROM dead_pokemon WHERE run_id = $1)
         ORDER BY caught_at",
        &[&rid],
    ).map_err(|e| format!("Query failed: {e}"))?;

    let dead = client.query(
        "SELECT nickname, species_name, is_shiny, nature, level,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                move1, move2, move3, move4, ability_name, held_item, gender
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
        &[&rid],
    ).map_err(|e| format!("Query failed: {e}"))?;

    let mut out = String::new();

    if !living.is_empty() {
        out.push_str("# Living Party\n\n");
        for row in &living {
            pokepaste_entry_no_moves(&mut out, row);
        }
    }

    if !dead.is_empty() {
        if !living.is_empty() {
            out.push('\n');
        }
        out.push_str("# Fallen\n\n");
        for row in &dead {
            pokepaste_entry_with_moves(&mut out, row);
        }
    }

    Ok(out)
}

fn pokepaste_entry_no_moves(out: &mut String, row: &postgres::Row) {
    let nickname: String     = row.get(0);
    let species: String      = row.get(1);
    let shiny: bool          = row.get(2);
    let nature: String       = row.get(3);
    let level: i32           = row.get(4);
    let iv_hp: i32           = row.get(5);
    let iv_atk: i32          = row.get(6);
    let iv_def: i32          = row.get(7);
    let iv_spe: i32          = row.get(8);
    let iv_spa: i32          = row.get(9);
    let iv_spd: i32          = row.get(10);
    let ev_hp: i32           = row.get(11);
    let ev_atk: i32          = row.get(12);
    let ev_def: i32          = row.get(13);
    let ev_spe: i32          = row.get(14);
    let ev_spa: i32          = row.get(15);
    let ev_spd: i32          = row.get(16);

    let header = if nickname == species {
        species.clone()
    } else {
        format!("{nickname} ({species})")
    };
    out.push_str(&header);
    out.push('\n');
    out.push_str(&format!("Level: {level}\n"));
    if shiny {
        out.push_str("Shiny: Yes\n");
    }
    out.push_str(&format!("{nature} Nature\n"));

    let evs = pokepaste_stat_line(ev_hp, ev_atk, ev_def, ev_spe, ev_spa, ev_spd);
    if !evs.is_empty() {
        out.push_str(&format!("EVs: {evs}\n"));
    }
    let ivs = pokepaste_iv_line(iv_hp, iv_atk, iv_def, iv_spe, iv_spa, iv_spd);
    if !ivs.is_empty() {
        out.push_str(&format!("IVs: {ivs}\n"));
    }
    out.push('\n');
}

fn pokepaste_entry_with_moves(out: &mut String, row: &postgres::Row) {
    let nickname: String     = row.get(0);
    let species: String      = row.get(1);
    let shiny: bool          = row.get(2);
    let nature: String       = row.get(3);
    let level: i32           = row.get(4);
    let iv_hp: i32           = row.get(5);
    let iv_atk: i32          = row.get(6);
    let iv_def: i32          = row.get(7);
    let iv_spe: i32          = row.get(8);
    let iv_spa: i32          = row.get(9);
    let iv_spd: i32          = row.get(10);
    let ev_hp: i32           = row.get(11);
    let ev_atk: i32          = row.get(12);
    let ev_def: i32          = row.get(13);
    let ev_spe: i32          = row.get(14);
    let ev_spa: i32          = row.get(15);
    let ev_spd: i32          = row.get(16);
    let move1: i32           = row.get(17);
    let move2: i32           = row.get(18);
    let move3: i32           = row.get(19);
    let move4: i32           = row.get(20);
    let ability: String      = row.get(21);
    let held_item: i32       = row.get(22);

    let header = if nickname == species {
        species.clone()
    } else {
        format!("{nickname} ({species})")
    };
    // Item ID 0 means "no item held".
    if held_item > 0 {
        out.push_str(&format!("{header} @ Item #{held_item}\n"));
    } else {
        out.push_str(&header);
        out.push('\n');
    }
    if !ability.is_empty() {
        out.push_str(&format!("Ability: {ability}\n"));
    }
    out.push_str(&format!("Level: {level}\n"));
    if shiny {
        out.push_str("Shiny: Yes\n");
    }
    out.push_str(&format!("{nature} Nature\n"));

    let evs = pokepaste_stat_line(ev_hp, ev_atk, ev_def, ev_spe, ev_spa, ev_spd);
    if !evs.is_empty() {
        out.push_str(&format!("EVs: {evs}\n"));
    }
    let ivs = pokepaste_iv_line(iv_hp, iv_atk, iv_def, iv_spe, iv_spa, iv_spd);
    if !ivs.is_empty() {
        out.push_str(&format!("IVs: {ivs}\n"));
    }
    for mv in [move1, move2, move3, move4] {
        if mv > 0 {
            out.push_str(&format!("- {}\n", move_name(mv as u16)));
        }
    }
    out.push('\n');
}

/// Formats non-zero EVs as a Pokepaste EV line (e.g. `"252 HP / 4 Def"`).
fn pokepaste_stat_line(hp: i32, atk: i32, def: i32, spe: i32, spa: i32, spd: i32) -> String {
    let parts: Vec<String> = [
        (hp,  "HP"),
        (atk, "Atk"),
        (def, "Def"),
        (spe, "Spe"),
        (spa, "SpA"),
        (spd, "SpD"),
    ]
    .into_iter()
    .filter(|(v, _)| *v != 0)
    .map(|(v, name)| format!("{v} {name}"))
    .collect();
    parts.join(" / ")
}

/// Formats non-31 IVs as a Pokepaste IV line.
fn pokepaste_iv_line(hp: i32, atk: i32, def: i32, spe: i32, spa: i32, spd: i32) -> String {
    let parts: Vec<String> = [
        (hp,  "HP"),
        (atk, "Atk"),
        (def, "Def"),
        (spe, "Spe"),
        (spa, "SpA"),
        (spd, "SpD"),
    ]
    .into_iter()
    .filter(|(v, _)| *v != 31)
    .map(|(v, name)| format!("{v} {name}"))
    .collect();
    parts.join(" / ")
}

// ---------------------------------------------------------------------------
// Per-section CSV exports (v0.9.51)
// ---------------------------------------------------------------------------

/// `GET /api/run/:id/encounters.csv` — first encounters per area.
pub fn export_encounters_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;
    let mut out = String::from(
        "player_name,species_name,level,map_group,map_name,caught,is_shiny,encountered_at\n",
    );
    let rows = client
        .query(
            "SELECT player_name, species_name, level, map_group, map_name, caught, is_shiny, encountered_at
             FROM encounters WHERE run_id = $1 ORDER BY encountered_at",
            &[&rid],
        )
        .unwrap_or_default();
    for r in &rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(0)),
            csv_field(r.get::<_, String>(1)),
            r.get::<_, i32>(2),
            r.get::<_, i32>(3),
            r.get::<_, i32>(4),
            r.get::<_, bool>(5),
            r.get::<_, bool>(6),
            csv_field(format_timestamp(r.get::<_, i64>(7) as u64)),
        ));
    }
    Ok(out)
}

/// `GET /api/run/:id/deaths.csv` — deaths log.
pub fn export_deaths_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;
    let mut out = String::from(
        "player_name,nickname,species_name,level,nature,is_shiny,gender,\
met_location,soul_link_death,iv_hp,iv_atk,iv_def,iv_spe,iv_spa,iv_spd,\
ev_hp,ev_atk,ev_def,ev_spe,ev_spa,ev_spd,died_at\n",
    );
    let rows = client
        .query(
            "SELECT player_name, nickname, species_name, level, nature, is_shiny, gender,
                    met_location, is_soul_link_death,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                    died_at
             FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
            &[&rid],
        )
        .unwrap_or_default();
    for r in &rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(0)),
            csv_field(r.get::<_, String>(1)),
            csv_field(r.get::<_, String>(2)),
            r.get::<_, i32>(3),
            csv_field(r.get::<_, String>(4)),
            r.get::<_, bool>(5),
            r.get::<_, i32>(6),
            r.get::<_, i32>(7),
            r.get::<_, bool>(8),
            r.get::<_, i32>(9),
            r.get::<_, i32>(10),
            r.get::<_, i32>(11),
            r.get::<_, i32>(12),
            r.get::<_, i32>(13),
            r.get::<_, i32>(14),
            r.get::<_, i32>(15),
            r.get::<_, i32>(16),
            r.get::<_, i32>(17),
            r.get::<_, i32>(18),
            r.get::<_, i32>(19),
            r.get::<_, i32>(20),
            csv_field(format_timestamp(r.get::<_, i64>(21) as u64)),
        ));
    }
    Ok(out)
}

/// `GET /api/run/:id/events.csv` — full event log.
pub fn export_events_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;
    let mut out = String::from(
        "player_name,event_type,species_name,nickname,old_nickname,level,note,occurred_at\n",
    );
    let rows = client
        .query(
            "SELECT player_name, event_type, species_name, nickname, old_nickname, level, note, occurred_at
             FROM events WHERE run_id = $1 ORDER BY occurred_at",
            &[&rid],
        )
        .unwrap_or_default();
    for r in &rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(0)),
            csv_field(r.get::<_, String>(1)),
            csv_field(r.get::<_, String>(2)),
            csv_field(r.get::<_, String>(3)),
            csv_field(r.get::<_, String>(4)),
            r.get::<_, i32>(5),
            csv_field(r.get::<_, String>(6)),
            csv_field(format_timestamp(r.get::<_, i64>(7) as u64)),
        ));
    }
    Ok(out)
}
