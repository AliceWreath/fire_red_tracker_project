//! Run-scoped analytics/history endpoints (/api/run/:id/...).

use super::*;

/// `GET /api/run/:id/stats` — per-run statistics JSON.
pub(crate) async fn api_run_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::run_stats(&conn, run_id)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/route_stats` — per-route catch-rate statistics JSON.
pub(crate) async fn api_run_route_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::route_stats(&conn, run_id)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/export` — full run export.
///
/// - Without query params (or `?format=json`): returns the full run as JSON
///   (metadata, caught, dead, encounters).
/// - `?format=csv`: returns three CSV sections (caught, dead, encounters) joined
///   by blank lines. Content-Type is `text/csv`.
pub(crate) async fn api_run_export(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response();
        }
    };
    if params.get("format").map(|s| s.as_str()) == Some("csv") {
        let result =
            tokio::task::spawn_blocking(move || fire_red_database::export_run_csv(&conn, run_id))
                .await;
        match result {
            Ok(Ok(csv)) => (
                [
                    ("content-type", "text/csv"),
                    (
                        "content-disposition",
                        &format!("attachment; filename=\"run_{run_id}.csv\""),
                    ),
                ],
                csv,
            )
                .into_response(),
            Ok(Err(e)) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
            Err(_) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Task panicked",
            )
                .into_response(),
        }
    } else {
        let result =
            tokio::task::spawn_blocking(move || fire_red_database::export_run(&conn, run_id)).await;
        axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
            .into_response()
    }
}

/// `GET /api/run/:id/route_odds` — encountered and unencountered wild areas for a run.
///
/// Returns `encountered` (routes already visited with species and catch info)
/// and `unencountered` (all known FireRed wild areas not yet recorded).
pub(crate) async fn api_run_route_odds(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::route_odds_json(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/webhook_log` — webhook delivery receipt log for a run.
pub(crate) async fn api_run_webhook_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::get_webhook_log_json(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/soul_link/overrides` — list all manual soul-link overrides for a run.
pub(crate) async fn api_run_soul_link_overrides(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::soul_link_overrides_json(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `POST /api/run/:id/soul_link/override` — set a manual soul-link pairing.
///
/// Body: `{ "personality": <u32>, "partner_personality": <u32> }`.
/// Replaces any existing override for the same `personality`.
pub(crate) async fn api_set_soul_link_override(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let personality = match body["personality"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
    {
        Some(v) => v,
        None => {
            return axum::Json(serde_json::json!({ "error": "Missing or invalid 'personality'" }));
        }
    };
    let partner_personality = match body["partner_personality"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
    {
        Some(v) => v,
        None => {
            return axum::Json(
                serde_json::json!({ "error": "Missing or invalid 'partner_personality'" }),
            );
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::set_soul_link_override_by_run(
            &conn,
            run_id,
            personality,
            partner_personality,
        )
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `DELETE /api/run/:id/soul_link/override/:personality` — remove a manual override.
pub(crate) async fn api_clear_soul_link_override(
    State(state): State<WebState>,
    Path((run_id, personality)): Path<(u32, u64)>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let p = match u32::try_from(personality) {
        Ok(v) => v,
        Err(_) => return axum::Json(serde_json::json!({ "error": "personality out of range" })),
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::clear_soul_link_override_by_run(&conn, run_id, p)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/species/stats` — cross-run per-species survival statistics JSON.
pub(crate) async fn api_species_stats(State(state): State<WebState>) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || fire_red_database::species_stats(&conn)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/trainers` — trainer battle log JSON for a run.
pub(crate) async fn api_run_trainers(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_trainer_defeats_json(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/shiny` — shiny odds statistics JSON for a run.
pub(crate) async fn api_shiny_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::shiny_stats(&conn, run_id)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/timeline` — chronological event log for the **active** run.
///
/// Includes both a Unix integer timestamp (`occurred_at`) and a human-readable
/// `occurred_at_human` string.
///
/// Status codes:
/// - `200 OK`                  — timeline returned successfully.
/// - `404 Not Found`           — no run is currently active.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
pub(crate) async fn api_active_timeline(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let uid = user.id;
    let result =
        tokio::task::spawn_blocking(move || {
            fire_red_database::active_run_timeline_for_user_json(&conn, uid)
        })
            .await
            .unwrap_or_else(|_| {
                Err(fire_red_database::EventsError::QueryFailed(
                    "Task panicked".into(),
                ))
            });

    match result {
        Ok(body) => (StatusCode::OK, axum::Json(body)).into_response(),
        Err(fire_red_database::EventsError::NoActiveRun) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "no active run" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/run/:id/events` — chronological event log for a run.
///
/// Status codes:
/// - `200 OK`                  — events returned.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
pub(crate) async fn api_run_events(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::list_events_json(&conn, run_id))
            .await
            .unwrap_or_else(|_| {
                Err(fire_red_database::EventsError::QueryFailed(
                    "Task panicked".into(),
                ))
            });

    match result {
        Ok(body) => (StatusCode::OK, axum::Json(body)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/runs` — summary list of runs accessible to the authenticated user.
pub(crate) async fn api_runs(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::list_runs_for_user_json(&conn, uid)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `POST /api/run/import` — import a run from the JSON format produced by `/api/run/:id/export`.
///
/// Creates a new run with a fresh id and re-inserts caught, dead, and encounter records.
/// The imported run is linked to the authenticated user. Returns `{ "run_id": <new_id> }`.
pub(crate) async fn api_run_import(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let result = tokio::task::spawn_blocking(move || {
        let val = fire_red_database::import_run(&conn, &body);
        if let Some(run_id) = val.get("run_id").and_then(|v| v.as_u64()).map(|v| v as u32) {
            let _ = fire_red_database::link_run_to_user(run_id, uid);
        }
        val
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/runs/compare?ids=1,2,3` — side-by-side stats for multiple runs.
///
/// Query param `ids` is a comma-separated list of run IDs (max 20).
pub(crate) async fn api_runs_compare(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let ids_str = match params.get("ids") {
        Some(s) => s.clone(),
        None => return axum::Json(serde_json::json!({ "error": "Missing 'ids' query parameter" })),
    };
    let requested: Vec<u32> = ids_str
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .take(20)
        .collect();
    if requested.is_empty() {
        return axum::Json(serde_json::json!({ "error": "No valid run IDs provided" }));
    }
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    let run_ids: Vec<u32> = requested.into_iter().filter(|id| accessible.contains(id)).collect();
    if run_ids.is_empty() {
        return axum::Json(serde_json::json!({ "error": "No accessible run IDs provided" }));
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::run_comparison(&conn, &run_ids)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/luck` — luck/RNG analysis for a single run.
///
/// Returns shiny rate vs expected (1/8192), per-area encounter list.
pub(crate) async fn api_run_luck(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::run_luck_stats(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/catch_rate?species=X&hp=Y&max_hp=Z&status=W&ball=B`
///
/// Computes the Gen III catch probability using the ROM's species catch rate.
///
/// - `species` — species ID (1–386)
/// - `hp` — current HP
/// - `max_hp` — max HP
/// - `status` — `none` | `sleep` | `freeze` | `paralyze` | `poison` | `burn`
///   (default: `none`)
/// - `ball` — `pokeball` | `greatball` | `ultraball` | `masterball` |
///   `safariball` | `netball` | `nestball` | `repeatball` |
///   `timerball` | `diveball` | `premierball` (default: `pokeball`)
pub(crate) async fn api_catch_rate(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let parse_u16 = |key: &str| -> Option<u16> {
        params.get(key)?.parse::<u16>().ok()
    };
    let species = match parse_u16("species") {
        Some(s) if s > 0 && s <= MAX_NATIONAL_DEX_FIRERED => s,
        _ => {
            return axum::Json(
                serde_json::json!({ "error": "species must be 1–386" }),
            );
        }
    };
    let hp = match parse_u16("hp") {
        Some(v) => v,
        None => return axum::Json(serde_json::json!({ "error": "hp required" })),
    };
    let max_hp = match parse_u16("max_hp") {
        Some(v) if v > 0 => v,
        _ => return axum::Json(serde_json::json!({ "error": "max_hp must be > 0" })),
    };

    let status = params.get("status").map(|s| s.as_str()).unwrap_or("none");
    let (status_num, status_label): (u32, &str) = match status {
        "sleep" | "freeze" => (15, status),
        "paralyze" | "poison" | "burn" => (12, status),
        _ => (10, "none"),
    };

    let ball = params.get("ball").map(|s| s.as_str()).unwrap_or("pokeball");
    let (ball_num, ball_label): (u32, &str) = match ball {
        "masterball"  => (255, "masterball"),
        "ultraball"   => (20, "ultraball"),   // 2.0 × 10
        "greatball"   => (15, "greatball"),   // 1.5 × 10
        "safariball"  => (15, "safariball"),
        "netball"     => (30, "netball"),     // 3.0 × 10
        "nestball" => {
            // (41 - level) / 10, minimum 1×; only beneficial below level 31.
            let mult = if let Some(lv) = parse_u16("level") {
                ((41u32.saturating_sub(lv as u32)) / 10).max(1)
            } else {
                1
            };
            (mult * 10, "nestball")
        }
        "repeatball" => {
            // 3× only if the species is already registered in the player's Pokédex.
            let already_caught = params.get("has_caught").map(|v| v == "true").unwrap_or(false);
            if already_caught { (30, "repeatball") } else { (10, "repeatball") }
        }
        "timerball"   => (40, "timerball"),   // max 4.0 × 10
        "diveball"    => (35, "diveball"),    // 3.5 × 10
        "premierball" => (10, "premierball"),
        _             => (10, "pokeball"),
    };

    // Look up catch rate from ROM base stats (28 bytes/entry, catch_rate at byte 8).
    const BASE_STATS_SIZE: usize = 28;
    const CATCH_RATE_OFFSET: usize = 8;
    let catch_rate = if let Some(rom) = fire_red_rom_buffer::try_get_rom() {
        let addrs = fire_red_rom_buffer::get_rom_addresses();
        let off = addrs.base_stats_addr + species as usize * BASE_STATS_SIZE + CATCH_RATE_OFFSET;
        rom.get(off).copied().unwrap_or(45)
    } else {
        45 // fallback: average catch rate if ROM not loaded
    };

    // Gen III modified catch rate:
    //   a = floor((3*M - 2*H) * rate * ball_num/10) / (3*M) * status_num/10
    // where M=max_hp, H=hp. We use u64 to avoid overflow.
    let m = max_hp as u64;
    let h = hp.min(max_hp) as u64;
    let numer = (3 * m - 2 * h) * (catch_rate as u64) * (ball_num as u64);
    let denom = 3 * m * 10;
    let a_raw = numer / denom;
    let a = (a_raw * status_num as u64 / 10).min(255);

    let guaranteed = a >= 255 || ball_num >= 255 * 10;
    let catch_probability_pct = if guaranteed {
        100.0f64
    } else {
        // b = floor(65536 / (255/a)^0.25)
        let b = (65536.0 / (255.0 / a as f64).powf(0.25)) as u64;
        let b = b.min(65535) as f64;
        // P = (b/65536)^4
        let p = (b / 65536.0).powi(4);
        (p * 10000.0).round() / 100.0
    };

    axum::Json(serde_json::json!({
        "species": species,
        "catch_rate": catch_rate,
        "hp": hp,
        "max_hp": max_hp,
        "status": status_label,
        "status_bonus": status_num as f64 / 10.0,
        "ball": ball_label,
        "ball_bonus": ball_num as f64 / 10.0,
        "modified_catch_rate": a,
        "guaranteed": guaranteed,
        "catch_probability_pct": catch_probability_pct,
    }))
}

/// `GET /api/run/:id/closest_calls` — Pokémon that came closest to fainting.
///
/// Returns up to 50 entries ordered by lowest HP/max_HP ratio ever observed.
pub(crate) async fn api_run_closest_calls(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::closest_calls(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/pokemon/:personality/hp_history` — full HP timeline for one Pokémon.
///
/// Returns every HP change observed while the Pokémon was in the active party,
/// ordered oldest-first.
pub(crate) async fn api_run_pokemon_hp_history(
    State(state): State<WebState>,
    Path((run_id, personality)): Path<(u32, u32)>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_hp_history(&conn, run_id, personality)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/enemy_hp_log` — enemy Pokémon HP at start and end of each encounter.
///
/// Groups by enemy personality. Each entry shows initial HP, final HP, and
/// total damage dealt. Species name is inferred from the nearest first-encounter record.
pub(crate) async fn api_run_enemy_hp_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::get_enemy_hp_log(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/battle_damage` — per-battle damage summary.
///
/// Groups damage events (HP decreases) across all party Pokémon into battles
/// using a 120-second gap threshold. Returns each battle's time window, which
/// Pokémon were involved, and how much damage each took.
pub(crate) async fn api_run_battle_damage(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_battle_damage_log(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/report` — self-contained HTML recap of a run (stats,
/// badge splits, deaths, roster, luck, difficulty). Save-and-share friendly:
/// no external resources, styled inline. Returns `404` when the run is not
/// found. Access is scoped by the usual `/api/run/:id` middleware.
pub(crate) async fn api_run_report(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::response::Response {
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return (StatusCode::SERVICE_UNAVAILABLE, "No database configured").into_response()
        }
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::run_report_html(&conn, run_id))
            .await
            .unwrap_or_else(|_| Err("Task panicked".to_string()));
    match result {
        Ok(html) => Html(html).into_response(),
        Err(e) if e.contains("not found") => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `GET /api/run/:id/summary` — Markdown text recap for a completed (or in-progress) run.
///
/// Append `?format=text` to receive `text/plain` (Markdown source directly); omit it to
/// receive `{ "markdown": "..." }` JSON. Returns `404` when the run is not found.
pub(crate) async fn api_run_summary(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::response::AppendHeaders([("content-type", "text/plain")]),
                "No database configured".to_string(),
            )
                .into_response()
        }
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::run_summary_markdown(&conn, run_id))
            .await
            .unwrap_or_else(|_| Err("Task panicked".to_string()));

    match result {
        Err(e) if e.contains("not found") => (
            StatusCode::NOT_FOUND,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            e,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            e,
        )
            .into_response(),
        Ok(md) => {
            if params.get("format").map(|s| s.as_str()) == Some("text") {
                (
                    StatusCode::OK,
                    axum::response::AppendHeaders([("content-type", "text/plain; charset=utf-8")]),
                    md,
                )
                    .into_response()
            } else {
                axum::Json(serde_json::json!({ "markdown": md })).into_response()
            }
        }
    }
}

/// `PATCH /api/run/:id/event/:event_id/note` — set or replace a free-text
/// annotation on an event log entry.
///
/// Request body: `{ "note": "some text" }`.
/// Passing an empty string clears the annotation without deleting the event.
///
/// Status codes:
/// - `200 OK`                  — note saved.
/// - `400 Bad Request`         — body missing or `note` field not a string.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
pub(crate) async fn api_set_event_note(
    State(state): State<WebState>,
    Path((_run_id, event_id)): Path<(u32, i32)>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let Some(note) = body.get("note").and_then(|v| v.as_str()).map(|s| s.to_string()) else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "Missing or invalid 'note' field" })),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::set_event_note(&conn, event_id, &note))
            .await
            .unwrap_or_else(|_| Err("Task panicked".into()));
    match result {
        Ok(()) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// `DELETE /api/run/:id/event/:event_id/note` — clear the annotation on an
/// event log entry (equivalent to PATCH with `"note": ""`).
pub(crate) async fn api_clear_event_note(
    State(state): State<WebState>,
    Path((_run_id, event_id)): Path<(u32, i32)>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::set_event_note(&conn, event_id, ""))
            .await
            .unwrap_or_else(|_| Err("Task panicked".into()));
    match result {
        Ok(()) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// `GET /api/run/:id/pokepaste` — export the run's Pokémon in Pokepaste format.
///
/// Returns `text/plain` with living party members first (`# Living Party`) and
/// fallen members second (`# Fallen`). Move data is only available for fallen
/// members (the surviving-party snapshot is captured at catch time, before moves
/// are trained). Ideal for sharing party state on [Pokémon Showdown](https://pokepast.es/).
///
/// Status codes:
/// - `200 OK`                  — Pokepaste text returned.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
pub(crate) async fn api_run_pokepaste(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            "No database configured".to_string(),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::pokepaste_export(&conn, run_id))
            .await
            .unwrap_or_else(|_| Err("Task panicked".into()));
    match result {
        Ok(text) => (
            StatusCode::OK,
            axum::response::AppendHeaders([("content-type", "text/plain; charset=utf-8")]),
            text,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            e,
        )
            .into_response(),
    }
}

/// `GET /api/run/:id/splits` — badge split times for a run.
///
/// Returns the wall-clock timestamp, elapsed seconds from run start, and
/// seconds since the previous badge for each of the up to 8 gym badges (plus
/// the game-clear event if recorded).
pub(crate) async fn api_run_splits(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::badge_splits(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// `GET /api/run/:id/catch_log` — catch attempt log for a run.
///
/// Each Nuzlocke first-encounter attempt (per area) is recorded with the
/// species name, area, total Pokéballs thrown, and whether the catch succeeded.
/// Summary totals (`total_balls_thrown`, `most_balls_in_one_encounter`) are
/// included at the top level.
pub(crate) async fn api_run_catch_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::catch_attempt_log(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// `GET /api/run/:id/difficulty` — composite difficulty score for a run.
///
/// Returns a 0–100 score derived from death ratio (40 %), HP danger (30 %),
/// catch miss rate (20 %), and trainer battle load (10 %), plus the raw
/// component values and input counts used to compute them.
pub(crate) async fn api_run_difficulty(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::difficulty_score(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// `GET /api/run/:id/area_times` — per-area time breakdown for a run.
///
/// Groups `area_visits` rows by area name and sums the total seconds spent in
/// each area, sorted by time descending. Open visits (player currently in that
/// area) use the current time as the exit. Each entry also includes a
/// human-readable `formatted` string and the visit count.
pub(crate) async fn api_run_area_times(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::area_time_breakdown(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_run_death_map(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::death_map(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_run_level_curve(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::level_curve(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_run_move_usage(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::move_usage(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_run_friendship(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::friendship_history(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// `GET /api/run/:id/goals` — list all goals for a run.
pub(crate) async fn api_list_run_goals(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let rid = run_id;
    let can = tokio::task::spawn_blocking(move || {
        fire_red_database::user_can_access_run(rid, uid)
    })
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);
    if !can {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let goals = tokio::task::spawn_blocking(move || {
        fire_red_database::list_goals_for_run(&conn, run_id)
    })
    .await
    .unwrap_or_default();
    let goals_json: Vec<_> = goals.into_iter().map(|g| serde_json::json!({
        "id": g.id, "text": g.text, "completed": g.completed
    })).collect();
    axum::Json(serde_json::json!({ "goals": goals_json }))
}

/// `POST /api/goal` — create a new run goal.
///
/// Body: `{"run_id": <u32>, "text": "<string>"}`
pub(crate) async fn api_post_goal(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let run_id = match body["run_id"].as_u64() {
        Some(id) => id as u32,
        None => return axum::Json(serde_json::json!({ "error": "missing run_id" })),
    };
    let text = match body["text"].as_str() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return axum::Json(serde_json::json!({ "error": "missing or empty text" })),
    };
    let uid = user.id;
    let can = tokio::task::spawn_blocking(move || {
        fire_red_database::user_can_access_run(run_id, uid)
    })
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);
    if !can {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::create_goal(&conn, run_id, &text)
    })
    .await;
    match result {
        Ok(Some(id)) => axum::Json(serde_json::json!({ "id": id })),
        _ => axum::Json(serde_json::json!({ "error": "failed to create goal" })),
    }
}

/// `PATCH /api/goal/:id/complete` — mark a goal as completed.
pub(crate) async fn api_complete_goal(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(goal_id): Path<i32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let conn_clone = conn.clone();
    let gid = goal_id;
    let run_id = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_id_for_goal(&conn_clone, gid)
    })
    .await
    .unwrap_or(None);
    // Fail-closed: if the goal doesn't exist or the lookup failed, deny the
    // operation rather than skipping the ownership check.
    let Some(rid) = run_id else {
        return axum::Json(serde_json::json!({ "error": "goal not found" }));
    };
    let can = tokio::task::spawn_blocking(move || {
        fire_red_database::user_can_access_run(rid, uid)
    })
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);
    if !can {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::complete_goal(&conn, goal_id)
    })
    .await;
    match result {
        Ok(true) => axum::Json(serde_json::json!({ "ok": true })),
        _ => axum::Json(serde_json::json!({ "error": "goal not found or update failed" })),
    }
}

/// `DELETE /api/goal/:id` — delete a goal.
pub(crate) async fn api_delete_goal(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(goal_id): Path<i32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let conn_clone = conn.clone();
    let gid = goal_id;
    let run_id = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_id_for_goal(&conn_clone, gid)
    })
    .await
    .unwrap_or(None);
    // Fail-closed: if the goal doesn't exist or the lookup failed, deny the
    // operation rather than skipping the ownership check.
    let Some(rid) = run_id else {
        return axum::Json(serde_json::json!({ "error": "goal not found" }));
    };
    let can = tokio::task::spawn_blocking(move || {
        fire_red_database::user_can_access_run(rid, uid)
    })
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);
    if !can {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::delete_goal(&conn, goal_id)
    })
    .await;
    match result {
        Ok(true) => axum::Json(serde_json::json!({ "ok": true })),
        _ => axum::Json(serde_json::json!({ "error": "goal not found" })),
    }
}

/// `PATCH /api/goal/:id` — set the completed flag to any value.
///
/// Body: `{"completed": <bool>}`  — use `true` to complete, `false` to un-complete.
pub(crate) async fn api_set_goal_completed(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(goal_id): Path<i32>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let completed = match body["completed"].as_bool() {
        Some(v) => v,
        None => return axum::Json(serde_json::json!({ "error": "missing completed bool" })),
    };
    let conn = require_db!(state);
    let uid = user.id;
    let conn_clone = conn.clone();
    let gid = goal_id;
    let run_id = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_id_for_goal(&conn_clone, gid)
    })
    .await
    .unwrap_or(None);
    let Some(rid) = run_id else {
        return axum::Json(serde_json::json!({ "error": "goal not found" }));
    };
    let can = tokio::task::spawn_blocking(move || {
        fire_red_database::user_can_access_run(rid, uid)
    })
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);
    if !can {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::set_goal_completed(&conn, goal_id, completed)
    })
    .await;
    match result {
        Ok(true) => axum::Json(serde_json::json!({ "ok": true })),
        _ => axum::Json(serde_json::json!({ "error": "goal not found or update failed" })),
    }
}

// ---------------------------------------------------------------------------
// New analytics handlers (v0.9.54)
// ---------------------------------------------------------------------------

pub(crate) async fn api_run_type_matchups(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::type_matchup_heatmap(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_run_ghost_compare(
    State(state): State<WebState>,
    Path((run_id, ghost_id)): Path<(u32, u32)>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::ghost_run_comparison(&conn, run_id, ghost_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_slot_shiny_pressure(
    State(state): State<WebState>,
    Path(slot_index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let run_id = {
        let slots = state.live_slots.lock_or_recover();
        let Some(slot) = slots.get(slot_index) else {
            return axum::Json(serde_json::json!({ "error": "Slot index out of range" }));
        };
        slot.db.as_ref().and_then(|db| db.active_run_id())
    };
    let Some(run_id) = run_id else {
        return axum::Json(serde_json::json!({ "error": "No active run for this slot" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::shiny_pressure(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_run_status_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_status_log(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

pub(crate) async fn api_run_dex(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::dex_count(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// POST /api/run/:id/share — mint a 24-hour read-only share token for this run.
pub(crate) async fn api_create_share(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    if state.db_conn.is_none() {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    }
    let token = tokio::task::spawn_blocking(move || {
        fire_red_database::create_share_token(run_id, 86400)
    })
    .await
    .unwrap_or(None);
    match token {
        Some(t) => axum::Json(serde_json::json!({ "token": t, "ttl_secs": 86400 })),
        None => axum::Json(serde_json::json!({ "error": "Failed to create share token" })),
    }
}

/// GET /share/:token/state — return read-only run stats for the token's run.
pub(crate) async fn api_share_state(
    State(state): State<WebState>,
    Path(token): Path<String>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let run_id = {
        let conn2 = conn.clone();
        tokio::task::spawn_blocking(move || fire_red_database::resolve_share_token(&conn2, &token))
            .await
            .unwrap_or(None)
    };
    let Some(run_id) = run_id else {
        return axum::Json(serde_json::json!({ "error": "Invalid or expired share token" }));
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::run_stats(&conn, run_id))
        .await
        .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// POST /api/config/reload — re-parse the aggregator config file and validate it.
///
/// Returns `{ "ok": true, "path": "..." }` on success or `{ "error": "..." }` on
/// parse failure. Requires `config_path` to be populated (set from `--config` CLI arg).
/// Useful for verifying edits before a full restart.
pub(crate) async fn api_config_reload(
    State(state): State<WebState>,
) -> axum::Json<serde_json::Value> {
    let Some(path) = state.config_path else {
        return axum::Json(serde_json::json!({ "error": "No config path available (run with --config)" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(&*path)
            .map_err(|e| format!("Cannot read config file: {e}"))?;
        let cfg: crate::config::AggregatorConfig = toml::from_str(&text)
            .map_err(|e| format!("TOML parse error: {e}"))?;
        Ok::<_, String>(serde_json::json!({
            "ok": true,
            "path": *path,
            "db": cfg.db.is_some(),
            "ws_port": cfg.ws_port,
            "twitch": cfg.twitch.is_some(),
            "discord_slash": cfg.discord_slash.is_some(),
        }))
    })
    .await
    .unwrap_or_else(|_| Err("Task panicked".into()));
    axum::Json(result.unwrap_or_else(|e| serde_json::json!({ "error": e })))
}
