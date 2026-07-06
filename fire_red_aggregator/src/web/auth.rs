//! User auth helpers, handlers, and access-control middleware.

use super::*;

// ---------------------------------------------------------------------------
// User auth helpers + handlers
// ---------------------------------------------------------------------------

/// Extract a bearer token from `Authorization: Bearer <token>`,
/// `X-Session-Token: <token>`, or the `frt_token` cookie.
/// Returns `None` if none is present.
pub(crate) fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("Authorization")
        && let Ok(s) = v.to_str()
        && let Some(tok) = s.strip_prefix("Bearer ") {
            return Some(tok.trim().to_string());
    }
    if let Some(v) = headers.get("X-Session-Token")
        && let Ok(s) = v.to_str() {
            return Some(s.trim().to_string());
    }
    // Cookie fallback — works for same-origin page loads and WS upgrades.
    if let Some(v) = headers.get(header::COOKIE)
        && let Ok(s) = v.to_str() {
        for part in s.split(';') {
            if let Some(val) = part.trim().strip_prefix("frt_token=") {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

/// Extract a `?token=<value>` query parameter from a URI.
/// Used by OBS browser sources that embed the token in the URL.
pub(crate) fn extract_query_token(uri: &axum::http::Uri) -> Option<String> {
    let q = uri.query()?;
    for pair in q.split('&') {
        if let Some(val) = pair.strip_prefix("token=")
            && !val.is_empty()
        {
            return Some(val.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Auth / access-control middleware
// ---------------------------------------------------------------------------

/// Routes that do not require a valid session.
pub(crate) fn is_public_route(path: &str, method: &axum::http::Method) -> bool {
    // /static/overlay.js: shared page runtime; <script src> requests carry no
    // token, and the file contains only code, never data.
    matches!(path, "/" | "/register" | "/interactions" | "/api/webhook/donation" | "/api/catch_rate" | "/static/overlay.js")
        || path == "/api/login"
        || path.starts_with("/share/")
        // POST /api/users = register endpoint
        || (path == "/api/users" && method == axum::http::Method::POST)
}

/// Global authentication middleware — validates the session on every
/// non-public route and injects [`User`] into request extensions.
/// Unauthenticated page requests are redirected to `/`; API/WS requests
/// receive `401 Unauthorized`.
pub(crate) async fn auth_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path   = request.uri().path().to_string();
    let method = request.method().clone();

    if is_public_route(&path, &method) {
        return next.run(request).await;
    }

    let token = extract_query_token(request.uri())
        .or_else(|| extract_bearer(request.headers()));

    let user: Option<User> = if let Some(tok) = token {
        tokio::task::spawn_blocking(move || fire_red_database::validate_session(&tok))
            .await
            .unwrap_or(Ok(None))
            .unwrap_or(None)
    } else {
        None
    };

    match user {
        Some(u) => {
            let mut req = request;
            req.extensions_mut().insert(u);
            next.run(req).await
        }
        None => {
            if path.starts_with("/api/") || path == "/ws" {
                axum::response::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(r#"{"error":"authentication required"}"#))
                    .unwrap()
            } else {
                axum::response::Redirect::to("/").into_response()
            }
        }
    }
}

/// Per-run access middleware — checks `user_can_access_run` for any path
/// that looks like `/api/run/<numeric-id>/…`.
///
/// Exceptions: invite-flow paths where the user doesn't yet have access
/// (`/invite/accept`, `/invite/decline`, `/invite/request`).
pub(crate) async fn run_access_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Public routes pass through auth_middleware without a User extension — skip.
    let Some(user) = request.extensions().get::<User>().cloned() else {
        return next.run(request).await;
    };

    let path = request.uri().path();

    // Invite-flow routes: caller may not have access yet.
    if path.ends_with("/invite/accept")
        || path.ends_with("/invite/decline")
        || (path.ends_with("/invite/request") && request.method() == axum::http::Method::POST)
    {
        return next.run(request).await;
    }

    // Extract numeric run_id from /api/run/<id>/…
    let run_id: Option<u32> = path
        .strip_prefix("/api/run/")
        .and_then(|s| s.split('/').next())
        .and_then(|s| s.parse().ok());

    if let Some(rid) = run_id {
        let uid = user.id;
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);

        if !can {
            return axum::response::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(r#"{"error":"access denied"}"#))
                .unwrap();
        }
    }

    next.run(request).await
}

/// Rewrites the numeric index of `/<n>/<rest>` page routes (party, hp,
/// routes, deaths, etc.) and `/api/slot/<n>[/<rest>]` API routes from a
/// display slot (as assigned via the /overlay corner button — 1-indexed, so
/// URL index `n` means pinned slot `n + 1`) to whichever physical live-slot
/// index currently holds that pin, if any.
///
/// Falls back to treating `<n>` as a raw physical slot index unchanged when
/// no live slot is pinned to it, so URLs/OBS scenes/Stream Deck buttons built
/// before any slot was ever assigned keep working exactly as before. Runs
/// before `slot_access_middleware`, so ownership checks there see the
/// already-resolved physical index.
pub(crate) async fn slot_display_index_middleware(
    State(state): State<WebState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let uri = request.uri().clone();
    let path = uri.path();

    // Either "/<n>/<rest>" (page routes) or "/api/slot/<n>[/<rest>]" (API routes).
    let parsed = if let Some(rest) = path.strip_prefix("/api/slot/") {
        let (num_str, tail) = match rest.find('/') {
            Some(slash) => (&rest[..slash], rest[slash..].to_string()),
            None => (rest, String::new()),
        };
        num_str
            .parse::<usize>()
            .ok()
            .and_then(|requested| {
                u8::try_from(requested + 1)
                    .ok()
                    .map(|wanted| ("/api/slot/", requested, wanted, tail))
            })
    } else {
        path.strip_prefix('/').and_then(|rest| {
            let slash = rest.find('/')?;
            let requested: usize = rest[..slash].parse().ok()?;
            let wanted = u8::try_from(requested + 1).ok()?;
            Some(("/", requested, wanted, rest[slash..].to_string()))
        })
    };

    if let Some((prefix, requested, wanted, tail)) = parsed {
        let slots = state.live_slots.clone();
        let resolved = tokio::task::spawn_blocking(move || {
            let slots = slots.lock_or_recover();
            slots.iter().position(|slot| {
                let label = slot.label.lock_or_recover().clone();
                slot.db.as_ref().and_then(|db| db.query_player_slot_index(&label)) == Some(wanted)
            })
        })
        .await
        .unwrap_or(None);

        if let Some(resolved) = resolved
            && resolved != requested
        {
            let new_path = format!("{prefix}{resolved}{tail}");
            let rebuilt = match uri.query() {
                Some(q) => format!("{new_path}?{q}"),
                None => new_path,
            };
            if let Ok(new_uri) = rebuilt.parse::<axum::http::Uri>() {
                *request.uri_mut() = new_uri;
            }
        }
    }

    next.run(request).await
}

/// Per-slot access middleware — for all requests to `/api/slot/<idx>/…`,
/// verifies the authenticated user has access to that slot's run.
pub(crate) async fn slot_access_middleware(
    State(state): State<WebState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Public routes pass through auth_middleware without a User extension — skip.
    let Some(user) = request.extensions().get::<User>().cloned() else {
        return next.run(request).await;
    };

    let path = request.uri().path();
    let slot_idx: Option<usize> = path
        .strip_prefix("/api/slot/")
        .and_then(|s| s.split('/').next())
        .and_then(|s| s.parse().ok());

    if let Some(idx) = slot_idx {
        let run_id = {
            let lock = state.live_slots.lock_or_recover();
            lock.get(idx).and_then(|s| s.db.as_ref().and_then(|db| db.get_run_id()))
        };

        if let Some(rid) = run_id {
            let uid = user.id;
            let can = tokio::task::spawn_blocking(move || {
                fire_red_database::user_can_access_run(rid, uid)
            })
            .await
            .unwrap_or(Ok(false))
            .unwrap_or(false);

            if !can {
                return axum::response::Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(r#"{"error":"access denied"}"#))
                    .unwrap();
            }
        }
    }

    next.run(request).await
}

/// Filter a JSON slot-array string for `user_id`.
///
/// Array positions are **preserved** — inaccessible slots are replaced with
/// `null` rather than removed.  This ensures that overlay URLs such as
/// `/1/alerts` (which index into the array by position) still work correctly
/// when multiple users share the same server.
///
/// A slot with no `active_run_id` is nulled out for regular users.
/// Only the server owner (user ID 1) sees unlinked slots so they can
/// diagnose connection issues without exposing another player's live state.
pub(crate) async fn filter_slots_for_user(json: &str, user_id: u32) -> String {
    let arr: serde_json::Value =
        serde_json::from_str(json).unwrap_or(serde_json::Value::Array(vec![]));
    let slots = match arr.as_array() {
        Some(s) => s.clone(),
        None => return "[]".to_string(),
    };

    let accessible: HashSet<u32> =
        tokio::task::spawn_blocking(move || fire_red_database::get_accessible_run_ids(user_id))
            .await
            .unwrap_or(Ok(HashSet::new()))
            .unwrap_or_default();

    // Replace inaccessible slots with null (preserve position).
    // Unlinked slots (no active_run_id) are visible to the server owner only.
    let is_owner = user_id == 1;
    let filtered: Vec<serde_json::Value> = slots
        .into_iter()
        .map(|slot| {
            match slot.get("active_run_id").and_then(|v| v.as_u64()) {
                None if is_owner => slot,                               // unlinked → owner only
                None => serde_json::Value::Null,                       // unlinked → hidden
                Some(rid) if accessible.contains(&(rid as u32)) => slot, // owned → keep
                _ => serde_json::Value::Null,                          // forbidden → null
            }
        })
        .collect();

    serde_json::to_string(&serde_json::Value::Array(filtered))
        .unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Deserialize)]
pub(crate) struct RegisterBody {
    username: String,
    password: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct LoginBody {
    username: String,
    password: String,
}

/// `POST /api/users` — register a new user account.
///
/// Body: `{ "username": "...", "password": "..." }` (password ≥ 8 chars)
/// Returns: `{ "id": N, "username": "..." }` or `{ "error": "..." }` with
/// `409 Conflict` when the username is already taken.
pub(crate) async fn api_register_user(
    axum::Json(body): axum::Json<RegisterBody>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::create_user(&body.username, &body.password)
    }).await;
    match result {
        Ok(Ok(user)) => (
            StatusCode::CREATED,
            axum::Json(serde_json::json!({
                "id": user.id,
                "username": user.username,
                "created_at": user.created_at,
            })),
        ),
        Ok(Err(e)) if e.contains("already taken") => (
            StatusCode::CONFLICT,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "Task panicked" })),
        ),
    }
}

/// `GET /api/users` — list all registered users (server-owner only).
///
/// Restricted to user ID 1 (the first registered account = server owner).
/// Returns: `[{ "id": N, "username": "...", "created_at": N }, ...]`
pub(crate) async fn api_list_users(Extension(user): Extension<User>) -> impl IntoResponse {
    if user.id != 1 {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "only the server owner can list users" })),
        );
    }
    let result = tokio::task::spawn_blocking(fire_red_database::list_users).await;
    match result {
        Ok(Ok(users)) => {
            let arr: Vec<_> = users.iter().map(|u| serde_json::json!({
                "id": u.id,
                "username": u.username,
                "created_at": u.created_at,
            })).collect();
            (StatusCode::OK, axum::Json(serde_json::json!(arr)))
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "Task panicked" })),
        ),
    }
}

/// `POST /api/login` — authenticate and get a session token.
///
/// Body: `{ "username": "...", "password": "..." }`
/// Returns: `{ "token": "...", "user": { "id": N, "username": "..." } }` and
/// sets an `HttpOnly` `frt_token` cookie so browser page-loads are authenticated
/// automatically. Returns `401` on bad credentials.
pub(crate) async fn api_login(
    axum::Json(body): axum::Json<LoginBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let username_for_log = body.username.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<(fire_red_database::User, String)>, String> {
        let user = fire_red_database::authenticate_user(&body.username, &body.password)?;
        match user {
            Some(u) => {
                let token = fire_red_database::create_session(u.id)?;
                Ok(Some((u, token)))
            }
            None => Ok(None),
        }
    }).await;
    match result {
        Ok(Ok(Some((user, token)))) => {
            let cookie = format!(
                "frt_token={}; HttpOnly; Path=/; SameSite=Lax; Max-Age=2592000",
                token
            );
            (
                StatusCode::OK,
                [(header::SET_COOKIE, cookie)],
                axum::Json(serde_json::json!({
                    "token": token,
                    "user": { "id": user.id, "username": user.username },
                })),
            ).into_response()
        }
        Ok(Ok(None)) => {
            tracing::warn!(username = %username_for_log, "POST /api/login → 401");
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "invalid username or password" })),
            ).into_response()
        }
        Ok(Err(e)) => {
            tracing::error!(username = %username_for_log, error = %e, "POST /api/login → 500 (DB error)");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": e })),
            ).into_response()
        }
        Err(_) => {
            tracing::error!(username = %username_for_log, "POST /api/login → 500 (task panicked)");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "Task panicked" })),
            ).into_response()
        }
    }
}

/// `POST /api/logout` — invalidate the current session token.
///
/// Requires `Authorization: Bearer <token>` or `X-Session-Token: <token>`.
/// Returns `200` whether or not the token existed.
pub(crate) async fn api_logout(
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let token = extract_query_token(&uri).or_else(|| extract_bearer(&headers));
    if let Some(token) = token {
        tokio::task::spawn_blocking(move || fire_red_database::delete_session(&token))
            .await
            .ok();
    }
    // Clear the frt_token cookie in the browser.
    (
        StatusCode::OK,
        [(header::SET_COOKIE, "frt_token=; HttpOnly; Path=/; SameSite=Lax; Max-Age=0")],
    )
}

/// `GET /api/me` — return the currently authenticated user.
///
/// Requires `Authorization: Bearer <token>` or `X-Session-Token: <token>`.
/// Returns `{ "id": N, "username": "..." }` or `401` if the token is missing
/// or expired.
pub(crate) async fn api_me(
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "no session token provided" })),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::validate_session(&token)
    }).await;
    match result {
        Ok(Ok(Some(user))) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "id": user.id,
                "username": user.username,
                "created_at": user.created_at,
            })),
        ),
        Ok(Ok(None)) => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "session expired or invalid" })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "Task panicked" })),
        ),
    }
}

/// `GET /api/me/token` — return the caller's raw session token.
///
/// The token lives in an `HttpOnly` cookie so JavaScript can't read it directly.
/// This endpoint echoes it back as `{ "token": "…" }` so the dashboard can
/// display and copy it for use with the `?token=` OBS URL parameter.
pub(crate) async fn api_me_token(
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    match extract_bearer(&headers) {
        Some(token) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "token": token })),
        ),
        None => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "no session token" })),
        ),
    }
}

/// `GET /api/me/active_run` — return the run_id this user most recently connected to.
///
/// Returns `{ "run_id": N }` or `{ "run_id": null }` if none recorded.
pub(crate) async fn api_me_active_run(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "authentication required"})));
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::validate_session(&token)).await;
    let user = match result {
        Ok(Ok(Some(u))) => u,
        Ok(Ok(None)) => return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "session expired"}))),
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e}))),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "Task panicked"}))),
    };
    let run_id = state.user_active_run.lock().unwrap().get(&user.id).copied();
    (StatusCode::OK, axum::Json(serde_json::json!({"run_id": run_id})))
}

/// `PUT /api/me/active_run` — explicitly set the caller's active run.
///
/// Body: `{ "run_id": N }`.  Used by the page-selector dropdown so that
/// selecting a run page on the join/dashboard also updates auto-detect.
pub(crate) async fn api_me_set_active_run(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "authentication required"})));
    };
    let run_id = match body.get("run_id").and_then(|v| v.as_u64()) {
        Some(id) => id as u32,
        None => return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error": "run_id required"}))),
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::validate_session(&token)).await;
    let user = match result {
        Ok(Ok(Some(u))) => u,
        Ok(Ok(None)) => return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "session expired"}))),
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e}))),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "Task panicked"}))),
    };
    let uid = user.id;
    let can = tokio::task::spawn_blocking(move || fire_red_database::user_can_access_run(run_id, uid))
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);
    if !can {
        return (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({"error": "access denied"})));
    }
    state.user_active_run.lock().unwrap().insert(user.id, run_id);
    (StatusCode::OK, axum::Json(serde_json::json!({"ok": true})))
}

/// `GET /api/user/:id/runs` — list runs for a user (own account only).
pub(crate) async fn api_user_runs(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(user_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    if user.id != user_id {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::list_runs_for_user_json(&conn, user_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}
