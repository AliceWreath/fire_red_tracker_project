//! Per-run statistics JSON: run/route stats, badge splits, catch-attempt
//! log, difficulty score, area time breakdown, shiny stats, and the SQL
//! query page backend.

use super::*;

// ---------------------------------------------------------------------------
// Full database dump — used by the DB viewer web page
// ---------------------------------------------------------------------------

/// Deletes every record from all tables in the database.
///
/// Opens its own connection (like `dump_all`) so the live tracker connections
/// are not blocked. Deletes child tables before parent to satisfy foreign keys,
/// then resets the active_run_id meta key.
///
/// Returns `Ok(())` on success or an error string on failure.
/// Executes arbitrary SQL via a fresh connection and returns the results as JSON.
///
/// SELECT queries return `{ "columns": [...], "rows": [{col: val, ...}, ...] }`.
/// Non-SELECT statements return `{ "columns": [], "rows": [], "rows_affected": N }`.
/// Errors return `{ "error": "..." }`.
pub fn run_sql(conn_str: &str, sql: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let messages = match client.simple_query(sql) {
        Ok(m) => m,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let mut rows_affected: Option<u64> = None;
    for msg in messages {
        match msg {
            postgres::SimpleQueryMessage::Row(row) => {
                if columns.is_empty() {
                    columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                }
                let obj: serde_json::Map<String, serde_json::Value> = row
                    .columns()
                    .iter()
                    .enumerate()
                    .map(|(i, col)| {
                        let val = row
                            .get(i)
                            .map(|s| serde_json::Value::String(s.to_string()))
                            .unwrap_or(serde_json::Value::Null);
                        (col.name().to_string(), val)
                    })
                    .collect();
                rows.push(serde_json::Value::Object(obj));
            }
            postgres::SimpleQueryMessage::CommandComplete(n) => {
                rows_affected = Some(n);
            }
            _ => {}
        }
    }
    serde_json::json!({ "columns": columns, "rows": rows, "rows_affected": rows_affected })
}

/// Returns per-run statistics for the given run ID as JSON.
///
/// Opens its own connection (like `dump_all`) so live tracker connections are not blocked.
pub fn run_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let run_row = match client.query_opt(
        "SELECT player_name, started_at, ended_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return serde_json::json!({ "error": "Run not found" }),
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let player_name: String = run_row.get(0);
    let started_at: i64 = run_row.get(1);
    let ended_at: Option<i64> = run_row.get(2);

    let now = unix_now() as i64;
    let duration_secs = ended_at.unwrap_or(now) - started_at;
    let playtime = format!("{}h {}m", duration_secs / 3600, (duration_secs % 3600) / 60);

    let enc_rows = client
        .query(
            "SELECT map_group, map_name, species_name, level, caught, is_shiny, encountered_at
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default();

    let total_encounters = enc_rows.len();
    let total_caught: usize = enc_rows.iter().filter(|r| r.get::<_, bool>(4)).count();
    let catch_rate = if total_encounters > 0 {
        (total_caught as f64 / total_encounters as f64 * 100.0).round()
    } else {
        0.0
    };

    let zone_stats: Vec<serde_json::Value> = enc_rows
        .iter()
        .map(|row| {
            let mg = row.get::<_, i32>(0) as u8;
            let mn = row.get::<_, i32>(1) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "area":          area,
                "species_name":  row.get::<_, String>(2),
                "level":         row.get::<_, i32>(3),
                "caught":        row.get::<_, bool>(4),
                "is_shiny":      row.get::<_, bool>(5),
                "encountered_at": format_timestamp(row.get::<_, i64>(6) as u64),
            })
        })
        .collect();

    let dead_rows = client
        .query(
            "SELECT level, species_name, met_location, died_at, is_soul_link_death
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default();

    let total_deaths = dead_rows.len();
    let avg_death_level = if total_deaths > 0 {
        let total: i64 = dead_rows.iter().map(|r| r.get::<_, i32>(0) as i64).sum();
        (total as f64 / total_deaths as f64).round()
    } else {
        0.0
    };

    let deaths: Vec<serde_json::Value> = dead_rows
        .iter()
        .map(|row| {
            let met_loc = row.get::<_, i32>(2) as u8;
            let raw = fire_red_location_names::location_name(met_loc);
            let location = if raw.is_empty() {
                format!("loc {}", met_loc)
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "level":        row.get::<_, i32>(0),
                "species_name": row.get::<_, String>(1),
                "location":     location,
                "died_at":      format_timestamp(row.get::<_, i64>(3) as u64),
                "soul_link":    row.get::<_, bool>(4),
            })
        })
        .collect();

    serde_json::json!({
        "run_id":          run_id,
        "player_name":     player_name,
        "started_at":      format_timestamp(started_at as u64),
        "ended_at":        ended_at.map(|t| format_timestamp(t as u64)),
        "playtime":        playtime,
        "total_encounters": total_encounters,
        "total_caught":    total_caught,
        "catch_rate_pct":  catch_rate,
        "total_deaths":    total_deaths,
        "avg_death_level": avg_death_level,
        "zone_stats":      zone_stats,
        "deaths":          deaths,
    })
}

/// Returns per-route catch statistics for the given run ID as JSON.
///
/// Each entry in `zones` covers one (map_group, map_name) pair and includes
/// the encounter count, catch count, and catch-rate percentage. Opens its own
/// connection so live tracker connections are not blocked.
pub fn route_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT map_group, map_name,
                COUNT(*) AS total,
                SUM(CASE WHEN caught THEN 1 ELSE 0 END) AS caught_count
         FROM encounters
         WHERE run_id = $1
         GROUP BY map_group, map_name
         ORDER BY map_group, map_name",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let zones: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mg: u8 = row.get::<_, i32>(0) as u8;
            let mn: u8 = row.get::<_, i32>(1) as u8;
            let total: i64 = row.get(2);
            let caught: i64 = row.get(3);
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            let catch_rate = if total > 0 {
                (caught as f64 / total as f64 * 100.0).round()
            } else {
                0.0
            };
            serde_json::json!({
                "map_group":      mg,
                "map_name":       mn,
                "area":           area,
                "total":          total,
                "caught":         caught,
                "catch_rate_pct": catch_rate,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "zones": zones })
}

/// Returns badge split times for the given run as JSON.
///
/// Each entry in `splits` has `badge_name`, `earned_at` (formatted timestamp),
/// `elapsed_secs` (seconds since run started), and `split_secs` (seconds since
/// the previous badge, or since run start for the first badge).
pub fn badge_splits(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let started_at: i64 = match client.query_opt(
        "SELECT started_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ) {
        Ok(Some(r)) => r.get(0),
        Ok(None) => return serde_json::json!({ "error": "Run not found" }),
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let rows = match client.query(
        "SELECT species_name, occurred_at
         FROM events
         WHERE run_id = $1 AND event_type = 'badge'
         ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let mut prev_ts = started_at;
    let splits: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let badge_name: String = row.get(0);
            let occurred_at: i64 = row.get(1);
            let elapsed = (occurred_at - started_at).max(0);
            let split = (occurred_at - prev_ts).max(0);
            prev_ts = occurred_at;
            serde_json::json!({
                "badge_name":   badge_name,
                "earned_at":    format_timestamp(occurred_at as u64),
                "elapsed_secs": elapsed,
                "split_secs":   split,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "started_at": format_timestamp(started_at as u64), "splits": splits })
}

/// Returns catch-attempt log for the given run as JSON.
///
/// Each entry covers one wild encounter (Nuzlocke first-per-area only) and
/// includes `species_name`, `area`, `balls_thrown`, `caught`, and
/// `encountered_at`.  Summary totals are included at the top level.
pub fn catch_attempt_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT player_name, species_name, area, balls_thrown, caught, encountered_at
         FROM catch_attempts
         WHERE run_id = $1
         ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let mut total_balls: i64 = 0;
    let mut max_balls: i32 = 0;
    let mut worst_encounter = String::new();

    let attempts: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let player_name: String = row.get(0);
            let species_name: String = row.get(1);
            let area: String = row.get(2);
            let balls_thrown: i32 = row.get(3);
            let caught: bool = row.get(4);
            let encountered_at: i64 = row.get(5);
            total_balls += balls_thrown as i64;
            if balls_thrown > max_balls {
                max_balls = balls_thrown;
                worst_encounter = format!("{} ({})", species_name, area);
            }
            serde_json::json!({
                "player_name":    player_name,
                "species_name":   species_name,
                "area":           area,
                "balls_thrown":   balls_thrown,
                "caught":         caught,
                "encountered_at": format_timestamp(encountered_at as u64),
            })
        })
        .collect();

    serde_json::json!({
        "run_id":          run_id,
        "total_balls_thrown": total_balls,
        "most_balls_in_one_encounter": max_balls,
        "hardest_encounter": worst_encounter,
        "attempts":        attempts,
    })
}

/// Returns a composite difficulty score (0–100) for the given run, plus the
/// component breakdown used to compute it.
///
/// Components:
/// - `death_ratio`  (40 %) — deaths / (deaths + survivors) × 100
/// - `hp_danger`    (30 %) — avg "danger fraction" (1 − min_hp/max_hp) × 100
/// - `catch_miss`   (20 %) — (total_encounters − caught) / total_encounters × 100
/// - `trainer_load` (10 %) — min(trainer_count / 80, 1.0) × 100
pub fn difficulty_score(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rid = run_id as i32;

    let death_count: i64 = client
        .query_one("SELECT COUNT(*) FROM dead_pokemon WHERE run_id = $1", &[&rid])
        .map(|r| r.get(0))
        .unwrap_or(0);

    let survivor_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon cp
             WHERE cp.run_id = $1
               AND NOT EXISTS (
                   SELECT 1 FROM dead_pokemon dp
                   WHERE dp.run_id = $1 AND dp.personality = cp.personality
               )",
            &[&rid],
        )
        .map(|r| r.get(0))
        .unwrap_or(0);

    let total_pokemon = death_count + survivor_count;
    let death_ratio = if total_pokemon > 0 {
        death_count as f64 / total_pokemon as f64 * 100.0
    } else {
        0.0
    };

    // HP danger: average of (1 - min_hp/max_hp) for all mons with recorded HP
    let hp_rows = client
        .query(
            "SELECT min_hp_seen_hp, min_hp_seen_max_hp
             FROM caught_pokemon
             WHERE run_id = $1
               AND min_hp_seen_hp IS NOT NULL
               AND min_hp_seen_max_hp > 0",
            &[&rid],
        )
        .unwrap_or_default();

    let hp_danger = if hp_rows.is_empty() {
        0.0
    } else {
        let sum: f64 = hp_rows
            .iter()
            .map(|r| {
                let hp: i16 = r.get(0);
                let max_hp: i16 = r.get(1);
                1.0 - (hp as f64 / max_hp as f64)
            })
            .sum();
        sum / hp_rows.len() as f64 * 100.0
    };

    let enc_row = client
        .query_one(
            "SELECT COUNT(*), SUM(CASE WHEN caught THEN 1 ELSE 0 END)
             FROM encounters WHERE run_id = $1",
            &[&rid],
        )
        .ok();
    let (total_enc, total_caught): (i64, i64) = enc_row
        .as_ref()
        .map(|r| (r.get(0), r.get::<_, Option<i64>>(1).unwrap_or(0)))
        .unwrap_or((0, 0));

    let catch_miss = if total_enc > 0 {
        (total_enc - total_caught) as f64 / total_enc as f64 * 100.0
    } else {
        0.0
    };

    let trainer_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM trainer_battles WHERE run_id = $1",
            &[&rid],
        )
        .map(|r| r.get(0))
        .unwrap_or(0);

    let trainer_load = (trainer_count as f64 / 80.0).min(1.0) * 100.0;

    let score = (0.40 * death_ratio + 0.30 * hp_danger + 0.20 * catch_miss + 0.10 * trainer_load)
        .clamp(0.0, 100.0);

    serde_json::json!({
        "run_id":        run_id,
        "difficulty":    (score * 10.0).round() / 10.0,
        "components": {
            "death_ratio_pct":   (death_ratio  * 10.0).round() / 10.0,
            "hp_danger_pct":     (hp_danger    * 10.0).round() / 10.0,
            "catch_miss_pct":    (catch_miss   * 10.0).round() / 10.0,
            "trainer_load_pct":  (trainer_load * 10.0).round() / 10.0,
        },
        "raw": {
            "deaths":         death_count,
            "survivors":      survivor_count,
            "total_encounters": total_enc,
            "total_caught":   total_caught,
            "trainer_battles": trainer_count,
        }
    })
}

/// Returns time spent per map area for the given run, sorted by total seconds
/// descending.  Open visits (player still there) use the current time as the
/// exit.
pub fn area_time_breakdown(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let now = unix_now() as i64;
    let rows = match client.query(
        "SELECT area_name, map_group, map_name,
                SUM(COALESCE(exited_at, $2) - entered_at) AS total_secs,
                COUNT(*) AS visits
         FROM area_visits
         WHERE run_id = $1
         GROUP BY area_name, map_group, map_name
         ORDER BY total_secs DESC",
        &[&(run_id as i32), &now],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let areas: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let area_name: String = row.get(0);
            let map_group: i32 = row.get(1);
            let map_name: i32 = row.get(2);
            let total_secs: i64 = row.get(3);
            let visits: i64 = row.get(4);
            let hours = total_secs / 3600;
            let mins = (total_secs % 3600) / 60;
            let secs = total_secs % 60;
            serde_json::json!({
                "area_name":   area_name,
                "map_group":   map_group,
                "map_name":    map_name,
                "total_secs":  total_secs,
                "formatted":   format!("{}h {:02}m {:02}s", hours, mins, secs),
                "visits":      visits,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "areas": areas })
}

/// Returns shiny encounter statistics for the given run ID as JSON.
///
/// Counts total encounters, total shinies, and encounters since the last shiny.
/// Opens its own connection so live tracker connections are not blocked.
pub fn shiny_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = client
        .query(
            "SELECT species_name, encountered_at, is_shiny, map_group, map_name, level, caught
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default();

    let total_encounters = rows.len();
    let total_shinies: usize = rows.iter().filter(|r| r.get::<_, bool>(2)).count();

    let last_shiny_idx = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.get::<_, bool>(2))
        .map(|(i, _)| i)
        .last();

    let (encounters_since_shiny, last_shiny) = match last_shiny_idx {
        Some(idx) => {
            let sr = &rows[idx];
            let mg = sr.get::<_, i32>(3) as u8;
            let mn = sr.get::<_, i32>(4) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            let shiny = serde_json::json!({
                "species_name":  sr.get::<_, String>(0),
                "encountered_at": format_timestamp(sr.get::<_, i64>(1) as u64),
                "area":          area,
                "level":         sr.get::<_, i32>(5),
                "caught":        sr.get::<_, bool>(6),
            });
            (total_encounters - idx - 1, Some(shiny))
        }
        None => (total_encounters, None),
    };

    let recent_start = last_shiny_idx.map(|i| i + 1).unwrap_or(0);
    let since_last_shiny: Vec<serde_json::Value> = rows[recent_start..]
        .iter()
        .map(|row| {
            let mg = row.get::<_, i32>(3) as u8;
            let mn = row.get::<_, i32>(4) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "species_name":  row.get::<_, String>(0),
                "encountered_at": format_timestamp(row.get::<_, i64>(1) as u64),
                "area":          area,
                "level":         row.get::<_, i32>(5),
                "caught":        row.get::<_, bool>(6),
            })
        })
        .collect();

    serde_json::json!({
        "run_id":                   run_id,
        "total_encounters":         total_encounters,
        "total_shinies":            total_shinies,
        "encounters_since_last_shiny": encounters_since_shiny,
        "last_shiny":               last_shiny,
        "since_last_shiny":         since_last_shiny,
    })
}
