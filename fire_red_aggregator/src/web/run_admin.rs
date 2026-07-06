//! Run management, rules, player slots, CSV exports, DB admin.

use super::*;

pub(crate) async fn serve_db_json(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> axum::Json<serde_json::Value> {
    let conn = match state.db_conn {
        Some(s) => s,
        None => return axum::Json(serde_json::json!({ "error": "No database configured" })),
    };
    let token = extract_bearer(&headers).map(|s| s.to_string());
    let result = tokio::task::spawn_blocking(move || {
        if let Some(tok) = token
            && let Ok(Some(user)) = fire_red_database::validate_session(&tok)
        {
            return fire_red_database::dump_for_user(&conn, user.id);
        }
        fire_red_database::dump_all(&conn)
    }).await;
    axum::Json(result.unwrap_or_else(|e| {
        tracing::error!("db dump task failed: {e}");
        serde_json::json!({ "error": "Query failed" })
    }))
}

pub(crate) async fn clear_db(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if user.id != 1 {
        return (
            StatusCode::FORBIDDEN,
            "only the server owner can clear the database".to_string(),
        );
    }
    if params.get("confirm").map(String::as_str) != Some("true") {
        return (
            StatusCode::BAD_REQUEST,
            "Add ?confirm=true to confirm database wipe".to_string(),
        );
    }
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "No database configured".to_string(),
            );
        }
    };
    match tokio::task::spawn_blocking(move || fire_red_database::clear_all_records(&conn)).await {
        Ok(Ok(())) => (StatusCode::OK, "ok".to_string()),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Task panicked".to_string(),
        ),
    }
}

pub(crate) async fn api_db_query(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    // Owner-only: user ID 1 is the server owner.
    if user.id != 1 {
        return axum::Json(
            serde_json::json!({ "error": "Forbidden: only the server owner can execute arbitrary SQL" }),
        );
    }
    // Loopback guard retained as defence-in-depth, but note that it is bypassed
    // when the server runs behind a reverse proxy (ConnectInfo sees proxy address).
    if !addr.ip().is_loopback() {
        return axum::Json(
            serde_json::json!({ "error": "Forbidden: endpoint only available on localhost" }),
        );
    }
    let conn = require_db!(state);
    let sql = match body["sql"].as_str() {
        Some(s) => s.to_string(),
        None => return axum::Json(serde_json::json!({ "error": "Missing 'sql' field" })),
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::run_sql(&conn, &sql)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

// ---------------------------------------------------------------------------
// Challenge rules (GET/PATCH /api/run/:id/rules)
// ---------------------------------------------------------------------------

/// `GET /api/run/:id/rules` — fetch nuzlocke variant flags for a run.
pub(crate) async fn api_get_run_rules(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_rules(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `PATCH /api/run/:id/rules` — update one or more nuzlocke variant flags.
///
/// Body: any subset of `{ "duplicate_clause": bool, "species_clause": bool,
/// "gift_clause": bool, "shiny_clause": bool }`. Unspecified fields are unchanged.
pub(crate) async fn api_patch_run_rules(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::set_run_rules(&conn, run_id, &body)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

// ---------------------------------------------------------------------------
// Per-player slot index (display column order) — GET/PATCH /api/run/:id/player_slots
// ---------------------------------------------------------------------------

/// `GET /api/run/:id/player_slots` — pinned display columns for every player
/// in this run.
///
/// Response: `{ "run_id": N, "players": [{ "player_name": "Alice", "slot_index": 1 }, ...] }`.
pub(crate) async fn api_get_run_player_slots(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_player_slots(&conn, run_id)
    })
    .await
    .unwrap_or_default();
    let players: Vec<serde_json::Value> = result
        .into_iter()
        .map(|(player_name, slot_index)| {
            serde_json::json!({ "player_name": player_name, "slot_index": slot_index })
        })
        .collect();
    axum::Json(serde_json::json!({ "run_id": run_id, "players": players }))
}

#[derive(serde::Deserialize)]
pub(crate) struct PlayerSlotIndexBody {
    player_name: String,
    slot_index: Option<u8>,
}

/// `PATCH /api/run/:id/player_slots` — pin one player's column within this run.
///
/// Caller must be the run owner. Body: `{ "player_name": "Alice", "slot_index": 1 }`
/// (1 = leftmost column) or `{ "player_name": "Alice", "slot_index": null }` to
/// clear. The new ordering is reflected in the WebSocket feed within ~1 second.
pub(crate) async fn api_patch_run_player_slots(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
    axum::Json(body): axum::Json<PlayerSlotIndexBody>,
) -> (StatusCode, axum::Json<serde_json::Value>) {
    let conn = match state.db_conn {
        Some(s) => s,
        None => return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        ),
    };
    let token = match extract_bearer(&headers) {
        Some(t) => t.to_string(),
        None => return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "authentication required" })),
        ),
    };
    let player_name = body.player_name.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), (StatusCode, String)> {
        let user = fire_red_database::validate_session(&token)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;
        fire_red_database::set_run_player_slot_index(&conn, run_id, user.id, &player_name, body.slot_index)
            .map_err(|e| (StatusCode::FORBIDDEN, e))
    })
    .await;
    match result {
        Ok(Ok(())) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "run_id": run_id,
                "player_name": body.player_name,
                "slot_index": body.slot_index,
            })),
        ),
        Ok(Err((status, e))) => (status, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "Task panicked" })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Per-section CSV exports
// ---------------------------------------------------------------------------

pub(crate) fn csv_response(
    result: Result<Result<String, String>, tokio::task::JoinError>,
    run_id: u32,
    suffix: &str,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match result {
        Ok(Ok(csv)) => (
            [
                ("content-type", "text/csv".to_string()),
                (
                    "content-disposition",
                    format!("attachment; filename=\"run_{run_id}_{suffix}.csv\""),
                ),
            ],
            csv,
        )
            .into_response(),
        Ok(Err(e)) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(_) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Task panicked").into_response(),
    }
}

/// `GET /api/run/:id/encounters.csv` — first encounter per area as CSV.
pub(crate) async fn api_run_encounters_csv(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response()
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::export_encounters_csv(&conn, run_id)
    })
    .await;
    csv_response(result, run_id, "encounters")
}

/// `GET /api/run/:id/deaths.csv` — death log as CSV.
pub(crate) async fn api_run_deaths_csv(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response()
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::export_deaths_csv(&conn, run_id)
    })
    .await;
    csv_response(result, run_id, "deaths")
}

/// `GET /api/run/:id/events.csv` — event log as CSV.
pub(crate) async fn api_run_events_csv(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response()
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::export_events_csv(&conn, run_id)
    })
    .await;
    csv_response(result, run_id, "events")
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Run management endpoints
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
pub(crate) struct CreateRunBody {
    player_name: Option<String>,
}

/// `POST /api/run` — create a new run and return its ID.
///
/// Requires authentication. The run is linked to the caller's account and
/// their username is used as the player name (overriding any `player_name`
/// in the body).
pub(crate) async fn api_create_run(
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<CreateRunBody>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "authentication required" })));
    };
    let fallback_name = body.player_name.unwrap_or_else(|| "Unknown".into());

    let result = tokio::task::spawn_blocking(move || -> Result<u32, (StatusCode, String)> {
        let user = fire_red_database::validate_session(&token)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;

        let player_name = if user.username.is_empty() { fallback_name } else { user.username };
        let run_id = fire_red_database::create_run_for_slot(&player_name)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let _ = fire_red_database::link_run_to_user(run_id, user.id);
        Ok(run_id)
    }).await;

    match result {
        Ok(Ok(run_id)) => (StatusCode::CREATED, axum::Json(serde_json::json!({ "run_id": run_id }))),
        Ok(Err((status, e))) => (status, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/resume` — set an existing run as the global active run.
///
/// Requires authentication.  The caller must own the run or have an accepted
/// invite.  In direct mode each slot manages its own run context via
/// `run_id` in `POST /api/direct/connect` instead.
pub(crate) async fn api_resume_run(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "authentication required" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<u32, (StatusCode, String)> {
        let user = fire_red_database::validate_session(&token)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;

        match fire_red_database::user_can_access_run(run_id, user.id) {
            Ok(true) => {}
            Ok(false) => return Err((StatusCode::FORBIDDEN, "you do not have access to this run".into())),
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
        }
        match fire_red_database::resume_run(run_id) {
            Ok(true) => Ok(user.id),
            Ok(false) => Err((StatusCode::NOT_FOUND, format!("run #{run_id} not found"))),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
        }
    }).await;

    match result {
        Ok(Ok(user_id)) => {
            state.user_active_run.lock().unwrap().insert(user_id, run_id);
            (StatusCode::OK, axum::Json(serde_json::json!({ "run_id": run_id })))
        }
        Ok(Err((status, e))) => (status, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}
