//! Aggregator-facing JSON: webhook delivery log, per-run soul-link
//! overrides, route odds, full DB dumps, species stats, run summary.

use super::*;

// ---------------------------------------------------------------------------
// Webhook delivery log
// ---------------------------------------------------------------------------

/// Returns the `run_id` of the currently active run, or `None` if there is no
/// active run or the database has not been initialized (tracker process only).
pub fn get_active_run_id() -> Option<u32> {
    db()?.lock_or_recover().run_id
}

/// Records the final outcome of a webhook delivery attempt.
///
/// Silently no-ops when the database is not initialized (e.g. in tests or the
/// aggregator process — this function should only be called from the tracker).
pub fn record_webhook_delivery(
    run_id: Option<u32>,
    event_type: &str,
    url: &str,
    success: bool,
    attempts: u32,
    payload: &str,
) {
    let Some(db) = db() else {
        return;
    };
    let mut state = db.lock_or_recover();
    let fired_at = unix_now() as i64;
    if let Err(e) = state.client.execute(
        "INSERT INTO webhook_log (run_id, event_type, url, success, attempts, payload, fired_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &run_id.map(|id| id as i32),
            &event_type,
            &url,
            &success,
            &(attempts as i32),
            &payload,
            &fired_at,
        ],
    ) {
        tracing::warn!("Failed to record webhook delivery: {e}");
    }
}

/// Returns a JSON array of webhook delivery log entries for the given run.
///
/// Opens its own connection; intended for the aggregator's API endpoint.
pub fn get_webhook_log_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT event_type, url, success, attempts, payload, fired_at
         FROM webhook_log WHERE run_id = $1 ORDER BY fired_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "event_type": row.get::<_, String>(0),
                "url":        row.get::<_, String>(1),
                "success":    row.get::<_, bool>(2),
                "attempts":   row.get::<_, i32>(3),
                "payload":    row.get::<_, String>(4),
                "fired_at":   row.get::<_, i64>(5),
                "fired_at_human": format_timestamp(row.get::<_, i64>(5) as u64),
            })
        })
        .collect();
    serde_json::json!({ "run_id": run_id, "webhook_log": entries })
}

// ---------------------------------------------------------------------------
// Soul-link override management — connection-string variants for the aggregator
// ---------------------------------------------------------------------------

/// Returns all soul-link overrides for `run_id` as JSON.
///
/// Used by `GET /api/run/:id/soul_link/overrides`.
pub fn soul_link_overrides_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, partner_personality, created_at
         FROM soul_link_overrides WHERE run_id = $1 ORDER BY created_at",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let overrides: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "personality":         row.get::<_, i64>(0) as u64,
                "partner_personality": row.get::<_, i64>(1) as u64,
                "created_at":          row.get::<_, i64>(2),
            })
        })
        .collect();
    serde_json::json!({ "run_id": run_id, "overrides": overrides })
}

/// Upserts a soul-link override for `run_id`: `personality` ↔ `partner_personality`.
///
/// Used by `POST /api/run/:id/soul_link/override`.
pub fn set_soul_link_override_by_run(
    conn_str: &str,
    run_id: u32,
    personality: u32,
    partner_personality: u32,
) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let p = personality as i64;
    let pp = partner_personality as i64;
    let now = unix_now() as i64;
    let rid = run_id as i32;
    match client.execute(
        "INSERT INTO soul_link_overrides (run_id, personality, partner_personality, created_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (run_id, personality)
         DO UPDATE SET partner_personality = EXCLUDED.partner_personality,
                       created_at          = EXCLUDED.created_at",
        &[&rid, &p, &pp, &now],
    ) {
        Ok(_) => serde_json::json!({ "ok": true }),
        Err(e) => serde_json::json!({ "error": format!("DB error: {e}") }),
    }
}

/// Deletes the soul-link override for `personality` in `run_id`.
///
/// Used by `DELETE /api/run/:id/soul_link/override/:personality`.
pub fn clear_soul_link_override_by_run(
    conn_str: &str,
    run_id: u32,
    personality: u32,
) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let p = personality as i64;
    let rid = run_id as i32;
    match client.execute(
        "DELETE FROM soul_link_overrides WHERE run_id = $1 AND personality = $2",
        &[&rid, &p],
    ) {
        Ok(n) => serde_json::json!({ "ok": true, "deleted": n }),
        Err(e) => serde_json::json!({ "error": format!("DB error: {e}") }),
    }
}

// ---------------------------------------------------------------------------
// Route odds — unencountered areas
// ---------------------------------------------------------------------------

/// Returns encountered and unencountered wild areas for the given run as JSON.
///
/// `encountered` — routes already visited (species, level, caught flag).
/// `unencountered` — all known FireRed wild areas not yet recorded for the run.
///
/// Opens its own connection; intended for the aggregator's API endpoint.
pub fn route_odds_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    // Load all encounter rows for this run.
    let rows = match client.query(
        "SELECT player_name, map_group, map_name, species, species_name, level, caught, is_shiny, encountered_at
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r)  => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    // Build set of (map_group, map_name) pairs that have been encountered,
    // respecting the dungeon-floor grouping (multi-floor dungeons share a
    // Nuzlocke slot via the dungeon_floors() canonical floor list).
    use std::collections::HashSet;
    let mut seen_canonical: HashSet<(u8, u8)> = HashSet::new();
    for row in &rows {
        let mg = row.get::<_, i32>(1) as u8; // col 1 = map_group
        let mn = row.get::<_, i32>(2) as u8; // col 2 = map_name
        let floors = fire_red_location_names::dungeon_floors(mg, mn);
        if floors.is_empty() {
            seen_canonical.insert((mg, mn));
        } else {
            for &(fg, fn_) in floors {
                seen_canonical.insert((fg, fn_));
            }
        }
    }

    let encountered: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mg = row.get::<_, i32>(1) as u8;
            let mn = row.get::<_, i32>(2) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{mg}:{mn}")
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "player_name":    row.get::<_, String>(0),
                "map_group":      mg,
                "map_name":       mn,
                "area":           area,
                "species":        row.get::<_, i32>(3),
                "species_name":   row.get::<_, String>(4),
                "level":          row.get::<_, i32>(5),
                "caught":         row.get::<_, bool>(6),
                "is_shiny":       row.get::<_, bool>(7),
                "encountered_at": format_timestamp(row.get::<_, i64>(8) as u64),
            })
        })
        .collect();

    // Unencountered: all known wild areas minus those in seen_canonical.
    let unencountered: Vec<serde_json::Value> = fire_red_location_names::all_wild_areas()
        .iter()
        .filter(|&&(mg, mn, _)| !seen_canonical.contains(&(mg, mn)))
        .map(|&(mg, mn, area)| {
            serde_json::json!({
                "map_group": mg,
                "map_name":  mn,
                "area":      area,
            })
        })
        .collect();

    serde_json::json!({
        "run_id":        run_id,
        "encountered":   encountered,
        "unencountered": unencountered,
    })
}

// ---------------------------------------------------------------------------
// Full DB dump
// ---------------------------------------------------------------------------

/// Opens a fresh connection and returns a JSON snapshot of every table.
///
/// Intended for the `/db.json` endpoint; opens its own connection so the live
/// tracker connections are not blocked. Returns a JSON error object on failure.
pub fn dump_all(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let runs = dump_runs(&mut client);
    let caught = dump_caught(&mut client);
    let dead = dump_dead(&mut client);
    let encounters = dump_encounters(&mut client);

    serde_json::json!({ "runs": runs, "caught": caught, "dead": dead, "encounters": encounters })
}

/// Like `dump_all` but restricted to runs accessible to `user_id` (owned or accepted invite).
pub fn dump_for_user(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let ids: Vec<i32> = match client.query(
        "SELECT r.id FROM runs r
         WHERE r.user_id = $1
            OR EXISTS (
                SELECT 1 FROM run_invites ri
                WHERE ri.run_id = r.id AND ri.invited_user = $1 AND ri.status = 'accepted'
            )",
        &[&(user_id as i32)],
    ) {
        Ok(rows) => rows.iter().map(|r| r.get::<_, i32>(0)).collect(),
        Err(e) => return serde_json::json!({ "error": format!("Access query failed: {e}") }),
    };

    let runs = dump_runs_for(&mut client, &ids);
    let caught = dump_caught_for(&mut client, &ids);
    let dead = dump_dead_for(&mut client, &ids);
    let encounters = dump_encounters_for(&mut client, &ids);

    serde_json::json!({ "runs": runs, "caught": caught, "dead": dead, "encounters": encounters })
}

fn dump_runs(client: &mut Client) -> serde_json::Value {
    dump_runs_for(client, &[])
}

fn dump_runs_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("r.id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT r.id, r.player_name, r.started_at, r.ended_at,
                COUNT(DISTINCT d.personality) AS deaths,
                COUNT(DISTINCT c.personality) AS catches,
                COUNT(DISTINCT e.id) AS encounters
         FROM runs r
         LEFT JOIN dead_pokemon d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         LEFT JOIN encounters e ON e.run_id = r.id
         WHERE {filter}
         GROUP BY r.id ORDER BY r.id");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
                let ended: Option<i64> = row.get(3);
                serde_json::json!({
                    "id":         row.get::<_, i32>(0),
                    "player":     row.get::<_, String>(1),
                    "started":    format_timestamp(row.get::<_, i64>(2) as u64),
                    "ended":      ended.map(|t| format_timestamp(t as u64)),
                    "deaths":     row.get::<_, i64>(4),
                    "catches":    row.get::<_, i64>(5),
                    "encounters": row.get::<_, i64>(6),
                })
            })
            .collect(),
    )
}

fn dump_caught(client: &mut Client) -> serde_json::Value {
    dump_caught_for(client, &[])
}

fn dump_caught_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("run_id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT run_id, player_name, nickname,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, nature, is_shiny,
                location_name,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                caught_at, gender
         FROM caught_pokemon WHERE {filter} ORDER BY caught_at ASC");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
                serde_json::json!({
                    "run_id":    row.get::<_, i32>(0),
                    "player":    row.get::<_, String>(1),
                    "nickname":  row.get::<_, String>(2),
                    "species":   row.get::<_, String>(3),
                    "level":     row.get::<_, i32>(4),
                    "nature":    row.get::<_, String>(5),
                    "shiny":     row.get::<_, bool>(6),
                    "location":  row.get::<_, String>(7),
                    "ivs":       format!("{}/{}/{}/{}/{}/{}",
                        row.get::<_, i32>(8),  row.get::<_, i32>(9),  row.get::<_, i32>(10),
                        row.get::<_, i32>(11), row.get::<_, i32>(12), row.get::<_, i32>(13)),
                    "evs":       format!("{}/{}/{}/{}/{}/{}",
                        row.get::<_, i32>(14), row.get::<_, i32>(15), row.get::<_, i32>(16),
                        row.get::<_, i32>(17), row.get::<_, i32>(18), row.get::<_, i32>(19)),
                    "caught_at": format_timestamp(row.get::<_, i64>(20) as u64),
                    "gender":    row.get::<_, i32>(21),
                })
            })
            .collect(),
    )
}

fn dump_dead(client: &mut Client) -> serde_json::Value {
    dump_dead_for(client, &[])
}

fn dump_dead_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("run_id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT run_id, player_name, nickname,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, nature, is_shiny,
                max_hp, attack, defense, speed, sp_attack, sp_defense,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                is_soul_link_death,
                died_at, gender
         FROM dead_pokemon WHERE {filter} ORDER BY died_at ASC");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
                serde_json::json!({
                    "run_id":    row.get::<_, i32>(0),
                    "player":    row.get::<_, String>(1),
                    "nickname":  row.get::<_, String>(2),
                    "species":   row.get::<_, String>(3),
                    "level":     row.get::<_, i32>(4),
                    "nature":    row.get::<_, String>(5),
                    "shiny":     row.get::<_, bool>(6),
                    "stats":     format!("{}/{}/{}/{}/{}/{}",
                        row.get::<_, i32>(7),  row.get::<_, i32>(8),  row.get::<_, i32>(9),
                        row.get::<_, i32>(10), row.get::<_, i32>(11), row.get::<_, i32>(12)),
                    "ivs":       format!("{}/{}/{}/{}/{}/{}",
                        row.get::<_, i32>(13), row.get::<_, i32>(14), row.get::<_, i32>(15),
                        row.get::<_, i32>(16), row.get::<_, i32>(17), row.get::<_, i32>(18)),
                    "evs":       format!("{}/{}/{}/{}/{}/{}",
                        row.get::<_, i32>(19), row.get::<_, i32>(20), row.get::<_, i32>(21),
                        row.get::<_, i32>(22), row.get::<_, i32>(23), row.get::<_, i32>(24)),
                    "soul_link": row.get::<_, bool>(25),
                    "died_at":   format_timestamp(row.get::<_, i64>(26) as u64),
                    "gender":    row.get::<_, i32>(27),
                })
            })
            .collect(),
    )
}

fn dump_encounters(client: &mut Client) -> serde_json::Value {
    dump_encounters_for(client, &[])
}

fn dump_encounters_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("run_id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT run_id, player_name, map_group, map_name,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, caught, encountered_at
         FROM encounters WHERE {filter} ORDER BY encountered_at ASC");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
                let group = row.get::<_, i32>(2) as u8;
                let map = row.get::<_, i32>(3) as u8;
                let name = fire_red_location_names::map_area_name(group, map);
                let area = if name.is_empty() {
                    format!("{}:{}", group, map)
                } else {
                    name.to_string()
                };
                serde_json::json!({
                    "run_id":  row.get::<_, i32>(0),
                    "player":  row.get::<_, String>(1),
                    "area":    area,
                    "species": row.get::<_, String>(4),
                    "level":   row.get::<_, i32>(5),
                    "caught":  row.get::<_, bool>(6),
                    "seen_at": format_timestamp(row.get::<_, i64>(7) as u64),
                })
            })
            .collect(),
    )
}

/// Returns cross-run per-species statistics as JSON.
///
/// For every species that has been caught or killed across all runs, returns
/// the total caught count, total death count, and a naive survival rate.
/// Results are ordered by total deaths descending so the most dangerous species
/// appear first.
pub fn species_stats(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT species_name,
                SUM(caught_count)   AS total_caught,
                SUM(dead_count)     AS total_dead
         FROM (
             SELECT species_name, 1 AS caught_count, 0 AS dead_count FROM caught_pokemon
             UNION ALL
             SELECT species_name, 0 AS caught_count, 1 AS dead_count FROM dead_pokemon
         ) t
         GROUP BY species_name
         ORDER BY total_dead DESC, total_caught DESC",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let species: String = row.get(0);
            let total_caught: i64 = row.get(1);
            let total_dead: i64 = row.get(2);
            let survival_pct = if total_caught > 0 {
                let survived = total_caught - total_dead;
                (survived.max(0) as f64 / total_caught as f64 * 100.0).round()
            } else {
                0.0
            };
            serde_json::json!({
                "species_name":   species,
                "total_caught":   total_caught,
                "total_dead":     total_dead,
                "survival_pct":   survival_pct,
            })
        })
        .collect();

    serde_json::json!({ "species": entries })
}

/// Generate a Markdown text recap for `run_id`.
///
/// Returns `Err(message)` when the run is not found or the DB is unreachable.
/// The caller can present the error as plain text or JSON as needed.
pub fn run_summary_markdown(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let run_row = client
        .query_opt(
            "SELECT player_name, started_at, ended_at FROM runs WHERE id = $1",
            &[&rid],
        )
        .map_err(|e| format!("Query error: {e}"))?
        .ok_or_else(|| format!("Run {run_id} not found"))?;

    let player_name: String = run_row.get(0);
    let started_at: i64 = run_row.get(1);
    let ended_at: Option<i64> = run_row.get(2);

    let now = unix_now() as i64;
    let duration_secs = ended_at.unwrap_or(now) - started_at;
    let started_str = format_timestamp(started_at as u64);
    let ended_str = ended_at
        .map(|t| format_timestamp(t as u64))
        .unwrap_or_else(|| "in progress".to_string());
    let playtime = format!("{}h {}m", duration_secs / 3600, (duration_secs % 3600) / 60);

    // Encounters
    let enc_rows = client
        .query(
            "SELECT map_group, map_name, species_name, level, caught, is_shiny, encountered_at \
             FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
            &[&rid],
        )
        .unwrap_or_default();

    let total_zones = enc_rows.len();
    let total_caught_enc: usize = enc_rows.iter().filter(|r| r.get::<_, bool>(4)).count();
    let total_shinies: usize = enc_rows.iter().filter(|r| r.get::<_, bool>(5)).count();
    let catch_pct = (total_caught_enc * 100).checked_div(total_zones).unwrap_or(0) as u32;

    // Deaths
    let dead_rows = client
        .query(
            "SELECT nickname, species_name, level, died_at, is_soul_link_death \
             FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at ASC",
            &[&rid],
        )
        .unwrap_or_default();
    let total_deaths = dead_rows.len();

    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "# FireRed Nuzlocke Run #{run_id} \u{2014} {player_name}\n\n"
    ));
    out.push_str(&format!(
        "**Started:** {started_str}  **Ended:** {ended_str}  **Playtime:** {playtime}\n\n"
    ));

    // Summary table
    out.push_str("## Run Summary\n\n");
    out.push_str("| Metric | Value |\n|--------|-------|\n");
    out.push_str(&format!("| Zones visited | {total_zones} |\n"));
    out.push_str(&format!(
        "| Caught | {total_caught_enc} / {total_zones} ({catch_pct}%) |\n"
    ));
    out.push_str(&format!("| Deaths | {total_deaths} |\n"));
    out.push_str(&format!("| Shiny encounters | {total_shinies} |\n\n"));

    // Deaths section
    if total_deaths > 0 {
        out.push_str("## \u{2620} Deaths\n\n");
        out.push_str("| # | Nickname | Species | Lv. | Date | Soul Link |\n");
        out.push_str("|---|----------|---------|-----|------|-----------|\n");
        for (i, row) in dead_rows.iter().enumerate() {
            let nickname: String = row.get(0);
            let species: String = row.get(1);
            let level: i32 = row.get(2);
            let died_at: i64 = row.get(3);
            let soul_link: bool = row.get(4);
            let date = format_timestamp(died_at as u64);
            let sl_mark = if soul_link { "yes" } else { "\u{2013}" };
            out.push_str(&format!(
                "| {} | {nickname} | {species} | {level} | {date} | {sl_mark} |\n",
                i + 1
            ));
        }
        out.push('\n');
    } else {
        out.push_str("## \u{2620} Deaths\n\nNo deaths this run!\n\n");
    }

    // Encounters section
    out.push_str("## \u{1f3af} Encounters\n\n");
    if enc_rows.is_empty() {
        out.push_str("No encounters recorded.\n\n");
    } else {
        out.push_str("| # | Zone | Species | Lv. | Caught | Shiny |\n");
        out.push_str("|---|------|---------|-----|--------|-------|\n");
        for (i, row) in enc_rows.iter().enumerate() {
            let mg = row.get::<_, i32>(0) as u8;
            let mn = row.get::<_, i32>(1) as u8;
            let raw_zone = fire_red_location_names::map_area_name(mg, mn);
            let zone = if raw_zone.is_empty() {
                format!("{mg}:{mn}")
            } else {
                raw_zone.to_string()
            };
            let species: String = row.get(2);
            let level: i32 = row.get(3);
            let caught: bool = row.get(4);
            let shiny: bool = row.get(5);
            let caught_str = if caught { "\u{2713}" } else { "\u{2717}" };
            let shiny_str = if shiny { "\u{2728}" } else { "\u{2013}" };
            out.push_str(&format!(
                "| {} | {zone} | {species} | {level} | {caught_str} | {shiny_str} |\n",
                i + 1
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "*Generated by fire_red_tracker v{}*\n",
        env!("CARGO_PKG_VERSION")
    ));

    Ok(out)
}
