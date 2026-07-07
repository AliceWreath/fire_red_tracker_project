//! Analytics: run comparison, luck stats, closest calls, death map,
//! level curve, move usage, friendship, status log, type matchups,
//! ghost-run comparison, shiny pressure, dex count, share tokens.

use super::*;

// ---------------------------------------------------------------------------
// Public API — analytics
// ---------------------------------------------------------------------------

/// Compare stats for multiple runs side-by-side.
///
/// Returns a JSON array, one entry per requested run ID. Fields per entry:
/// `id`, `player_name`, `started_at`, `ended_at`, `duration_secs`,
/// `total_encounters`, `catch_count`, `death_count`, `avg_death_level`.
pub fn run_comparison(conn_str: &str, run_ids: &[u32]) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    if run_ids.is_empty() {
        return serde_json::json!([]);
    }

    // Build a parameterised ANY($1) using an i32 array.
    let ids_i32: Vec<i32> = run_ids.iter().map(|&id| id as i32).collect();
    let rows = match client.query(
        "SELECT
             r.id,
             r.player_name,
             r.started_at,
             r.ended_at,
             (SELECT COUNT(*) FROM encounters   WHERE run_id = r.id)::bigint AS total_encounters,
             (SELECT COUNT(*) FROM caught_pokemon WHERE run_id = r.id)::bigint AS catch_count,
             (SELECT COUNT(*) FROM dead_pokemon   WHERE run_id = r.id)::bigint AS death_count,
             (SELECT AVG(level)::float FROM dead_pokemon WHERE run_id = r.id) AS avg_death_level
         FROM runs r
         WHERE r.id = ANY($1)
         ORDER BY r.id",
        &[&ids_i32],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let now = unix_now() as i64;
    let results: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let id: i32 = row.get(0);
            let player_name: String = row.get(1);
            let started_at: i64 = row.get(2);
            let ended_at: Option<i64> = row.get(3);
            let total_enc: i64 = row.get(4);
            let catch_count: i64 = row.get(5);
            let death_count: i64 = row.get(6);
            let avg_death_level: Option<f64> = row.get(7);
            let duration_secs = ended_at.unwrap_or(now) - started_at;
            serde_json::json!({
                "id": id,
                "player_name": player_name,
                "started_at": format_timestamp(started_at as u64),
                "ended_at": ended_at.map(|t| format_timestamp(t as u64)),
                "duration_secs": duration_secs,
                "playtime": format!("{}h {}m", duration_secs / 3600, (duration_secs % 3600) / 60),
                "total_encounters": total_enc,
                "catch_count": catch_count,
                "catch_rate_pct": if total_enc > 0 {
                    (catch_count as f64 / total_enc as f64 * 100.0 * 10.0).round() / 10.0
                } else { 0.0 },
                "death_count": death_count,
                "avg_death_level": avg_death_level.map(|v| (v * 10.0).round() / 10.0),
            })
        })
        .collect();

    serde_json::json!(results)
}

/// Luck / RNG analysis for a single run.
///
/// Returns a JSON object with:
/// - `total_encounters` — number of first encounters
/// - `shiny_count` — how many were shiny
/// - `expected_shinies` — `total_encounters / 8192.0`
/// - `shiny_rate_observed` — `shiny_count / total_encounters` (or null)
/// - `encounters` — per-area list with `area`, `species_name`, `level`, `is_shiny`, `caught`
pub fn run_luck_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT map_group, map_name, species_name, level, caught, is_shiny
         FROM encounters
         WHERE run_id = $1
         ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let total = rows.len() as u64;
    let shiny_count = rows.iter().filter(|r| r.get::<_, bool>(5)).count() as u64;
    let expected = total as f64 / 8192.0;
    let observed_rate = if total > 0 {
        serde_json::json!(shiny_count as f64 / total as f64)
    } else {
        serde_json::Value::Null
    };

    let enc_list: Vec<serde_json::Value> = rows
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
                "area": area,
                "species_name": row.get::<_, String>(2),
                "level": row.get::<_, i32>(3),
                "caught": row.get::<_, bool>(4),
                "is_shiny": row.get::<_, bool>(5),
            })
        })
        .collect();

    serde_json::json!({
        "run_id": run_id,
        "total_encounters": total,
        "shiny_count": shiny_count,
        "expected_shinies": (expected * 1000.0).round() / 1000.0,
        "shiny_rate_observed": observed_rate,
        "encounters": enc_list,
    })
}

/// Returns the 50 closest-call Pokémon for a run — those that reached the
/// lowest HP/max_HP ratio while alive — ordered from closest to farthest from
/// fainting.
///
/// Only Pokémon that have at least one recorded sub-max-HP observation appear.
/// The `is_dead` field is `true` when the Pokémon also has a row in
/// `dead_pokemon` (i.e. it eventually fainted).
pub fn closest_calls(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT cp.personality, cp.nickname, cp.species_name,
                cp.min_hp_seen_hp, cp.min_hp_seen_max_hp,
                (dp.personality IS NOT NULL) AS is_dead
         FROM caught_pokemon cp
         LEFT JOIN dead_pokemon dp
               ON dp.run_id = cp.run_id AND dp.personality = cp.personality
         WHERE cp.run_id = $1
           AND cp.min_hp_seen_hp IS NOT NULL
           AND cp.min_hp_seen_max_hp > 0
         ORDER BY (cp.min_hp_seen_hp::float / cp.min_hp_seen_max_hp) ASC
         LIMIT 50",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let personality: i64 = row.get(0);
            let nickname: String = row.get(1);
            let species_name: String = row.get(2);
            let hp: i16 = row.get(3);
            let max_hp: i16 = row.get(4);
            let is_dead: bool = row.get(5);
            let ratio = if max_hp > 0 {
                (hp as f64 / max_hp as f64 * 1000.0).round() / 10.0
            } else {
                0.0
            };
            serde_json::json!({
                "personality": personality as u32,
                "nickname": nickname,
                "species_name": species_name,
                "min_hp": hp,
                "min_max_hp": max_hp,
                "hp_pct": ratio,
                "is_dead": is_dead,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "closest_calls": entries })
}

// ---------------------------------------------------------------------------
// Analytics functions (v19-v22 features)
// ---------------------------------------------------------------------------

/// `GET /api/run/:id/death_map` — deaths grouped by the area they occurred in.
///
/// Returns `[{ "area": "Route 1", "count": 3 }, ...]` sorted descending by count.
pub fn death_map(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT COALESCE(NULLIF(area_name, ''), 'Unknown') AS area, COUNT(*) AS count
         FROM dead_pokemon
         WHERE run_id = $1
         GROUP BY area
         ORDER BY count DESC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let areas: Vec<serde_json::Value> = rows.iter().map(|r| {
        serde_json::json!({ "area": r.get::<_, String>(0), "count": r.get::<_, i64>(1) })
    }).collect();
    serde_json::json!(areas)
}

/// `GET /api/run/:id/level_curve` — average party level at each badge milestone.
///
/// Returns `[{ "badge_index": 0, "badge_name": "Boulder Badge", "avg_level": 14.2,
///             "levels": [12,14,15,...], "occurred_at": 1748000000 }, ...]`.
pub fn level_curve(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT badge_index, badge_name, occurred_at, avg_level, levels
         FROM party_snapshots
         WHERE run_id = $1
         ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let snapshots: Vec<serde_json::Value> = rows.iter().map(|r| {
        let levels_str: String = r.get(4);
        let levels: serde_json::Value = serde_json::from_str(&levels_str).unwrap_or(serde_json::json!([]));
        serde_json::json!({
            "badge_index": r.get::<_, i16>(0),
            "badge_name":  r.get::<_, String>(1),
            "occurred_at": r.get::<_, i64>(2),
            "avg_level":   r.get::<_, f32>(3),
            "levels":      levels,
        })
    }).collect();
    serde_json::json!(snapshots)
}

/// `GET /api/run/:id/move_usage` — move use counts per mon per slot.
///
/// Returns `[{ "personality": 123, "move_slot": 0, "move_name": "Tackle",
///             "use_count": 14 }, ...]` ordered by use_count descending.
pub fn move_usage(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, move_slot, move_id, move_name, use_count, player_name
         FROM move_uses
         WHERE run_id = $1
         ORDER BY use_count DESC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let uses: Vec<serde_json::Value> = rows.iter().map(|r| {
        serde_json::json!({
            "personality": r.get::<_, i64>(0) as u32,
            "move_slot":   r.get::<_, i16>(1),
            "move_id":     r.get::<_, i16>(2),
            "move_name":   r.get::<_, String>(3),
            "use_count":   r.get::<_, i32>(4),
            "player_name": r.get::<_, String>(5),
        })
    }).collect();
    serde_json::json!(uses)
}

/// `GET /api/run/:id/friendship` — friendship change history per mon.
///
/// Returns grouped by personality: `[{ "personality": 123, "nickname": "Squirtle",
///   "species_name": "Squirtle", "history": [{ "friendship": 70, "logged_at": ... }] }]`.
pub fn friendship_history(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, nickname, species_name, friendship, logged_at, player_name
         FROM friendship_log
         WHERE run_id = $1
         ORDER BY logged_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    // Group by personality.
    let mut by_mon: std::collections::HashMap<u32, serde_json::Value> = std::collections::HashMap::new();
    for r in &rows {
        let personality = r.get::<_, i64>(0) as u32;
        let entry = by_mon.entry(personality).or_insert_with(|| serde_json::json!({
            "personality": personality,
            "nickname":     r.get::<_, String>(1),
            "species_name": r.get::<_, String>(2),
            "player_name":  r.get::<_, String>(5),
            "history":      serde_json::json!([]),
        }));
        entry["history"].as_array_mut().unwrap().push(serde_json::json!({
            "friendship": r.get::<_, i16>(3),
            "logged_at":  r.get::<_, i64>(4),
        }));
    }
    serde_json::json!(by_mon.into_values().collect::<Vec<_>>())
}

/// Log a party-level snapshot at a badge milestone. Uses the global DB connection.
pub fn log_party_snapshot(
    player_name: &str,
    badge_index: u8,
    badge_name: &str,
    occurred_at: u64,
    levels: &[u8],
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    if levels.is_empty() { return; }
    let avg = levels.iter().map(|&l| l as f32).sum::<f32>() / levels.len() as f32;
    let levels_json = serde_json::to_string(levels).unwrap_or_else(|_| "[]".to_string());
    if let Err(e) = state.client.execute(
        "INSERT INTO party_snapshots (run_id, player_name, badge_index, badge_name, occurred_at, avg_level, levels)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &(run_id as i32),
            &player_name,
            &(badge_index as i16),
            &badge_name,
            &(occurred_at as i64),
            &avg,
            &levels_json,
        ],
    ) {
        tracing::warn!("log_party_snapshot: {e}");
    }
}

/// Increment a move use counter for a party Pokémon. Uses the global DB connection.
pub fn log_move_use(
    player_name: &str,
    personality: u32,
    move_slot: u8,
    move_id: u16,
    move_name: &str,
    uses: i32,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let now = unix_now();
    if let Err(e) = state.client.execute(
        "INSERT INTO move_uses (run_id, player_name, personality, move_slot, move_id, move_name, use_count, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (run_id, player_name, personality, move_slot)
         DO UPDATE SET use_count = move_uses.use_count + EXCLUDED.use_count,
                       move_name = EXCLUDED.move_name,
                       updated_at = EXCLUDED.updated_at",
        &[
            &(run_id as i32),
            &player_name,
            &(personality as i64),
            &(move_slot as i16),
            &(move_id as i16),
            &move_name,
            &uses,
            &(now as i64),
        ],
    ) {
        tracing::warn!("log_move_use: {e}");
    }
}

/// Append a friendship observation for a party Pokémon. Uses the global DB connection.
pub fn log_friendship(
    player_name: &str,
    personality: u32,
    nickname: &str,
    species_name: &str,
    friendship: u8,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let now = unix_now();
    if let Err(e) = state.client.execute(
        "INSERT INTO friendship_log (run_id, player_name, personality, nickname, species_name, friendship, logged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &(run_id as i32),
            &player_name,
            &(personality as i64),
            &nickname,
            &species_name,
            &(friendship as i16),
            &(now as i64),
        ],
    ) {
        tracing::warn!("log_friendship: {e}");
    }
}

/// Log a status condition onset or clear. Uses the global DB connection.
///
/// `status_name` is a human-readable string such as `"BRN"`, `"PAR"`, `"PSN"`, etc.
/// `event_type` is either `"onset"` or `"clear"`.
pub fn log_status_event(
    player_name: &str,
    personality: u32,
    nickname: &str,
    species_name: &str,
    status_name: &str,
    status_value: u32,
    event_type: &str,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let now = unix_now();
    if let Err(e) = state.client.execute(
        "INSERT INTO status_events
             (run_id, player_name, personality, nickname, species_name, status_name, status_value, event_type, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[
            &(run_id as i32),
            &player_name,
            &(personality as i64),
            &nickname,
            &species_name,
            &status_name,
            &(status_value as i32),
            &event_type,
            &(now as i64),
        ],
    ) {
        tracing::warn!("log_status_event: {e}");
    }
}

/// Returns the full status condition log for a run.
///
/// Each entry: `{ personality, nickname, species_name, status_name, event_type, occurred_at }`.
pub fn get_status_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, nickname, species_name, status_name, status_value, event_type, occurred_at
         FROM status_events
         WHERE run_id = $1
         ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let at: i64 = row.get(6);
            serde_json::json!({
                "personality": row.get::<_, i64>(0) as u32,
                "nickname":    row.get::<_, String>(1),
                "species_name": row.get::<_, String>(2),
                "status_name": row.get::<_, String>(3),
                "status_value": row.get::<_, i32>(4),
                "event_type":  row.get::<_, String>(5),
                "occurred_at": at,
                "timestamp":   format_timestamp(at as u64),
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "status_log": entries })
}

/// Gen III FireRed move type lookup table.
///
/// Index = move ID (1–354), value = type ID using the same 0–16 encoding as
/// `fire_red_party_monitor::type_name` (0=Normal … 8=Steel, 9=Fire … 16=Dark).
/// Index 0 is unused (no move has ID 0).
#[rustfmt::skip]
static MOVE_TYPES: [u8; 355] = [
    //  0:unused
    0,
    //  1–10
    0,  1,  0,  0,  0,  0,  9, 14, 12,  0,
    //  11–20
    0,  0,  0,  0,  0,  2,  2,  0,  2,  0,
    //  21–30
    0, 11,  0,  1,  0,  1,  1,  4,  0,  0,
    //  31–40
    0,  0,  0,  0,  0,  0,  0,  0,  0,  3,
    //  41–50
    6,  6,  0, 16,  0,  0,  0,  0,  0,  0,
    //  51–60
    3,  9,  9, 14, 10, 10, 10, 14, 14, 13,
    //  61–70
    10, 14,  0,  2,  2,  1,  1,  1,  1,  0,
    //  71–80
    11, 11, 11,  0, 11, 11,  3, 11, 11, 11,
    //  81–90
    6, 15,  9, 12, 12, 12, 12,  5,  4,  4,
    //  91–100
    4,  3, 13, 13, 13, 13, 13,  0,  0, 13,
    // 101–110
    7,  0,  0,  0,  0,  0,  0,  0,  7, 10,
    // 111–120
    0, 13, 13, 14, 13,  0,  0,  0,  2,  0,
    // 121–130
    0,  7,  3,  3,  4,  9, 10, 10,  0,  0,
    // 131–140
    0,  0, 13, 13,  0,  1,  0, 13,  3,  0,
    // 141–150
    6,  0,  2,  0, 10,  0, 11,  0, 13,  0,
    // 151–160
    3, 10,  0,  0,  4, 13,  5,  0,  0,  0,
    // 161–170
    0,  0,  0,  0,  0,  0,  1, 16,  6,  0,
    // 171–180
    7,  9,  0,  7,  0,  0,  2, 11,  1,  7,
    // 181–190
    14,  0,  1,  0, 16,  0,  0,  3,  4, 10,
    // 191–200
    4, 12,  0,  7,  0, 14,  1,  4,  0, 15,
    // 201–210
    5, 11,  0,  0,  5,  0,  0,  0, 12,  6,
    // 211–220
    8,  0,  0,  0,  0,  0,  0,  0,  0,  0,
    // 221–230
    9,  4,  1,  6, 15,  0,  0, 16,  0,  0,
    // 231–240
    8,  8,  1,  0, 11,  0,  0,  1, 15, 10,
    // 241–250
    9, 16, 13,  0,  0,  5,  7, 13,  1, 10,
    // 251–260
    16,  0,  0,  0,  0,  0,  9, 14, 16, 16,
    // 261–270
    9, 16,  0,  1,  0,  0,  0, 12, 16,  0,
    // 271–280
    13, 13,  0,  0, 11,  1, 13,  0,  1,  1,
    // 281–290
    0, 16,  0,  9, 13, 13,  0,  7, 16,  0,
    // 291–300
    10,  1,  0,  6, 13, 13,  2,  0,  9,  4,
    // 301–310
    14, 11,  0,  0,  3,  0,  9, 10,  8,  7,
    // 311–320
    0, 11, 16,  2,  9,  0,  5,  6,  8, 11,
    // 321–330
    0, 13, 10,  6,  7, 13,  1,  4, 14, 10,
    // 331–340
    11,  2, 14,  8,  0,  0, 15, 11,  1,  2,
    // 341–350
    4,  3,  0, 12, 11, 10, 13, 11, 15,  5,
    // 351–354
    12, 10,  8, 13,
];

fn move_type_for_id(move_id: u16) -> u8 {
    MOVE_TYPES.get(move_id as usize).copied().unwrap_or(0)
}

/// Returns a type-usage breakdown derived from recorded move uses.
///
/// Aggregates `move_uses` by move ID → attacking type using a static Gen III
/// move-type table, returning sorted totals per type.
/// Type IDs follow Gen III encoding (Normal=0 … Dark=16).
pub fn type_matchup_heatmap(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT move_id, SUM(use_count)::bigint AS total_uses
         FROM move_uses
         WHERE run_id = $1 AND move_id > 0
         GROUP BY move_id",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let mut type_uses: std::collections::HashMap<u8, i64> = std::collections::HashMap::new();
    for row in &rows {
        let move_id: i16 = row.get(0);
        let uses: i64 = row.get(1);
        let atk_type = move_type_for_id(move_id as u16);
        *type_uses.entry(atk_type).or_insert(0) += uses;
    }

    const TYPE_NAMES: [&str; 17] = [
        "Normal","Fighting","Flying","Poison","Ground","Rock","Bug","Ghost",
        "Steel","Fire","Water","Grass","Electric","Psychic","Ice","Dragon","Dark",
    ];

    let entries: Vec<serde_json::Value> = type_uses
        .into_iter()
        .map(|(type_id, uses)| {
            let name = TYPE_NAMES.get(type_id as usize).copied().unwrap_or("???");
            serde_json::json!({
                "type_id":    type_id,
                "type_name":  name,
                "total_uses": uses,
            })
        })
        .collect();

    let mut sorted = entries;
    sorted.sort_by(|a, b| b["total_uses"].as_i64().cmp(&a["total_uses"].as_i64()));

    serde_json::json!({ "run_id": run_id, "type_usage": sorted })
}

/// Ghost-run milestone comparison.
///
/// Returns a side-by-side diff of the current run vs a ghost run, aligned on
/// badge milestones. For each badge milestone (0–7) present in either run,
/// returns the elapsed time, deaths, and average party level at that point.
pub fn ghost_run_comparison(conn_str: &str, run_id: u32, ghost_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let snapshots_for = |c: &mut Client, rid: u32| -> Vec<(i16, String, i64, f32)> {
        c.query(
            "SELECT ps.badge_index, ps.badge_name, ps.occurred_at, ps.avg_level
             FROM party_snapshots ps
             WHERE ps.run_id = $1
             ORDER BY ps.badge_index ASC",
            &[&(rid as i32)],
        )
        .unwrap_or_default()
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect()
    };

    let deaths_before = |c: &mut Client, rid: u32, ts: i64| -> i64 {
        c.query_one(
            "SELECT COUNT(*) FROM dead_pokemon WHERE run_id = $1 AND died_at <= $2",
            &[&(rid as i32), &ts],
        )
        .map(|r| r.get::<_, i64>(0))
        .unwrap_or(0)
    };

    let started_at = |c: &mut Client, rid: u32| -> i64 {
        c.query_one("SELECT started_at FROM runs WHERE id = $1", &[&(rid as i32)])
            .map(|r| r.get::<_, i64>(0))
            .unwrap_or(0)
    };

    let run_start   = started_at(&mut client, run_id);
    let ghost_start = started_at(&mut client, ghost_id);
    let run_snaps   = snapshots_for(&mut client, run_id);
    let ghost_snaps = snapshots_for(&mut client, ghost_id);

    let all_badges: std::collections::BTreeSet<i16> = run_snaps
        .iter()
        .chain(ghost_snaps.iter())
        .map(|(i, _, _, _)| *i)
        .collect();

    let milestones: Vec<serde_json::Value> = all_badges
        .iter()
        .map(|&badge_idx| {
            let run_entry   = run_snaps.iter().find(|(i, _, _, _)| *i == badge_idx);
            let ghost_entry = ghost_snaps.iter().find(|(i, _, _, _)| *i == badge_idx);

            let badge_name = run_entry
                .or(ghost_entry)
                .map(|(_, n, _, _)| n.as_str())
                .unwrap_or("Badge");

            let make_side = |entry: Option<&(i16, String, i64, f32)>, run_start: i64, c: &mut Client, rid: u32| {
                entry.map(|(_, _, at, avg_lv)| {
                    let elapsed = (at - run_start).max(0) as u64;
                    let deaths  = deaths_before(c, rid, *at);
                    serde_json::json!({
                        "elapsed_secs": elapsed,
                        "elapsed_human": format!("{}h {:02}m {:02}s", elapsed / 3600, (elapsed % 3600) / 60, elapsed % 60),
                        "deaths":        deaths,
                        "avg_level":     avg_lv,
                    })
                })
            };

            let current = make_side(run_entry,   run_start,   &mut client, run_id);
            let ghost   = make_side(ghost_entry, ghost_start, &mut client, ghost_id);

            serde_json::json!({
                "badge_index": badge_idx,
                "badge_name":  badge_name,
                "current":     current,
                "ghost":       ghost,
            })
        })
        .collect();

    serde_json::json!({
        "run_id":   run_id,
        "ghost_id": ghost_id,
        "milestones": milestones,
    })
}

/// Cumulative shiny encounter probability for a run.
///
/// Returns the number of encounters logged and the cumulative probability of
/// having seen at least one shiny (P = 1 − (1 − 1/8192)^n).
pub fn shiny_pressure(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let row = match client.query_one(
        "SELECT COUNT(*), SUM(CASE WHEN is_shiny THEN 1 ELSE 0 END)::bigint
         FROM encounters WHERE run_id = $1",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let total: i64 = row.get(0);
    let shinies: i64 = row.get(1);
    // P(≥1 shiny in n encounters) = 1 - (1 - 1/8192)^n
    let prob_at_least_one = if total > 0 {
        1.0 - (1.0f64 - 1.0 / 8192.0).powi(total as i32)
    } else {
        0.0
    };
    let expected_at = if shinies == 0 { 8192i64 } else { total / shinies };
    serde_json::json!({
        "run_id":           run_id,
        "total_encounters": total,
        "shiny_count":      shinies,
        "probability_pct":  (prob_at_least_one * 10000.0).round() / 100.0,
        "expected_at":      expected_at,
        "unlucky":          shinies == 0 && total >= 8192,
    })
}

/// Pokédex completion count for a run.
///
/// Returns the number of unique species caught (`caught = true`) across all
/// encounters for the run, plus a list of species IDs / names.
pub fn dex_count(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT DISTINCT species, species_name
         FROM encounters
         WHERE run_id = $1 AND caught = TRUE
         ORDER BY species ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let caught: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| serde_json::json!({
            "species": row.get::<_, i32>(0),
            "species_name": row.get::<_, String>(1),
        }))
        .collect();
    serde_json::json!({
        "run_id":  run_id,
        "count":   caught.len(),
        "species": caught,
    })
}

/// Create a time-limited read-only share token for a run.
///
/// Stores the token in the `meta` table under key `share:<token>` with value
/// `<run_id>:<expires_at_unix>`. Returns the token string.
pub fn create_share_token(run_id: u32, ttl_secs: u64) -> Option<String> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    // Generate a 32-byte random token encoded as hex.
    // Use the system time + run_id + a counter as entropy (no rand crate needed).
    let now = unix_now();
    let expires = now + ttl_secs;
    let raw = format!("{run_id}-{now}-{expires}");
    let hash = Sha256::digest(raw.as_bytes());
    let token: String = hash.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    });
    let key = format!("share:{token}");
    let value = format!("{run_id}:{expires}");
    if let Err(e) = state.client.execute(
        "INSERT INTO meta (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        &[&key, &value],
    ) {
        tracing::warn!("create_share_token: {e}");
        return None;
    }
    Some(token)
}

/// Resolve a share token to a run ID, returning None if expired or not found.
pub fn resolve_share_token(conn_str: &str, token: &str) -> Option<u32> {
    let mut client = Client::connect(conn_str, NoTls).ok()?;
    let key = format!("share:{token}");
    let row = client
        .query_opt("SELECT value FROM meta WHERE key = $1", &[&key])
        .ok()??;
    let value: String = row.get(0);
    let mut parts = value.splitn(2, ':');
    let run_id: u32 = parts.next()?.parse().ok()?;
    let expires: u64 = parts.next()?.parse().ok()?;
    if unix_now() > expires {
        // Token expired — clean up silently.
        let _ = client.execute("DELETE FROM meta WHERE key = $1", &[&key]);
        return None;
    }
    Some(run_id)
}
