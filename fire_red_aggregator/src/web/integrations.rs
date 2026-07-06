//! Per-user integration management endpoints.

use super::*;

// ---------------------------------------------------------------------------
// Per-user integration management
// ---------------------------------------------------------------------------

pub(crate) async fn api_get_integrations(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::list_user_integrations(&conn, uid)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({})))
}

pub(crate) async fn api_put_integration(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(kind): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let conn = match &state.db_conn {
        Some(c) => c.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" }))),
    };
    let uid = user.id;
    let config_str = body.to_string();
    let kind2 = kind.clone();
    let conn2 = conn.clone();
    let set_result = tokio::task::spawn_blocking(move || {
        fire_red_database::set_user_integration(&conn2, uid, &kind2, &config_str)
    })
    .await;
    if let Err(e) = set_result.unwrap_or(Err("task panicked".into())) {
        return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": e })));
    }

    // Stop any existing thread for this user+kind.
    {
        let mgr = state.integration_manager.lock_or_recover();
        if let Some(stop) = mgr.get(&uid).and_then(|m| m.get(&kind)).cloned() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    // Spawn new thread based on kind.
    let slots = state.live_slots.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let spawned = spawn_integration_thread(&kind, body, slots, &conn, uid, stop2);

    if spawned {
        let mut mgr = state.integration_manager.lock_or_recover();
        mgr.entry(uid).or_default().insert(kind, stop);
    }

    (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true })))
}

pub(crate) fn spawn_integration_thread(
    kind: &str,
    config_val: serde_json::Value,
    slots: SharedSlots,
    conn: &str,
    user_id: u32,
    stop: Arc<AtomicBool>,
) -> bool {
    match kind {
        "twitch" => {
            if let Ok(cfg) = serde_json::from_value::<crate::config::TwitchConfig>(config_val) {
                crate::twitch::spawn(cfg.clone(), slots.clone(), Some(conn.to_owned()), Some(user_id), stop.clone());
                crate::eventsub::spawn(cfg, slots, Some(user_id), stop);
                true
            } else { false }
        }
        "youtube" => {
            if let Ok(cfg) = serde_json::from_value::<crate::config::YouTubeChatConfig>(config_val) {
                crate::youtube_chat::spawn(cfg, slots, Some(conn.to_owned()), Some(user_id), stop);
                true
            } else { false }
        }
        "discord_embed" => {
            if let Ok(cfg) = serde_json::from_value::<crate::config::DiscordLiveEmbedConfig>(config_val) {
                crate::discord_live::spawn_live_embed(cfg, slots, Some(user_id), stop);
                true
            } else { false }
        }
        "discord_thread" => {
            if let Ok(cfg) = serde_json::from_value::<crate::config::DiscordRunThreadConfig>(config_val) {
                crate::discord_live::spawn_run_thread(cfg, slots, Some(user_id), stop);
                true
            } else { false }
        }
        _ => false,
    }
}

pub(crate) async fn api_delete_integration(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(kind): Path<String>,
) -> impl IntoResponse {
    let conn = match &state.db_conn {
        Some(c) => c.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" }))),
    };
    let uid = user.id;
    let kind2 = kind.clone();
    tokio::task::spawn_blocking(move || {
        fire_red_database::delete_user_integration(&conn, uid, &kind2)
    })
    .await
    .ok();

    let mut mgr = state.integration_manager.lock_or_recover();
    if let Some(stop) = mgr.get(&uid).and_then(|m| m.get(&kind)).cloned() {
        stop.store(true, Ordering::Relaxed);
    }
    if let Some(user_map) = mgr.get_mut(&uid) {
        user_map.remove(&kind);
    }

    (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true })))
}
