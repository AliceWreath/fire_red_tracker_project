//! Batch injection and preset party builds.

use super::*;

// ---------------------------------------------------------------------------
// Batch injection (POST /api/batch)
// ---------------------------------------------------------------------------

/// `POST /api/batch` — apply an ordered list of injection commands in one request.
///
/// Body: a JSON array of `{ "slot": <usize>, "message": <ClientMessage> }` objects.
/// All commands are validated first, then enqueued atomically (one lock per slot).
/// Returns `{ "queued": <count> }` on success or `{ "error": "..." }` on the first
/// validation failure.
pub(crate) async fn api_batch_inject(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    if !state.allow_injections {
        return axum::Json(serde_json::json!({ "error": "injection commands are disabled" }));
    }
    let items = match body.as_array() {
        Some(a) => a,
        None => return axum::Json(serde_json::json!({ "error": "body must be a JSON array" })),
    };

    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();

    let slots = state.live_slots.lock_or_recover().clone();

    // Validate and decode every item before touching any queue.
    struct Decoded {
        slot_idx: usize,
        msg: ClientMessage,
    }
    let mut decoded: Vec<Decoded> = Vec::with_capacity(items.len());
    for (pos, item) in items.iter().enumerate() {
        let slot_idx = match item["slot"].as_u64().and_then(|v| usize::try_from(v).ok()) {
            Some(v) => v,
            None => return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: 'slot' must be a non-negative integer")
            })),
        };
        if slot_idx >= slots.len() {
            return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: slot {slot_idx} out of range")
            }));
        }
        if let Some(rid) = slots[slot_idx].db.as_ref().and_then(|db| db.get_run_id())
            && !accessible.contains(&rid)
        {
            return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: access denied for slot {slot_idx}")
            }));
        }
        let msg: ClientMessage = match serde_json::from_value(item["message"].clone()) {
            Ok(m) => m,
            Err(e) => return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: invalid message: {e}")
            })),
        };
        decoded.push(Decoded { slot_idx, msg });
    }

    // Enqueue all commands. Group by slot to minimise lock acquisitions.
    let count = decoded.len();
    for d in decoded {
        slots[d.slot_idx]
            .command_queue
            .lock_or_recover()
            .push_back(d.msg);
    }
    axum::Json(serde_json::json!({ "queued": count }))
}

// ---------------------------------------------------------------------------
// Preset party builds
// ---------------------------------------------------------------------------

/// `POST /api/preset` — save a named party preset.
///
/// Body: `{ "name": "<str>", "commands": [<ClientMessage>, ...] }`.
pub(crate) async fn api_save_preset(
    State(state): State<WebState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let name = match body["name"].as_str() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return axum::Json(serde_json::json!({ "error": "'name' must be a non-empty string" })),
    };
    let commands = match body.get("commands") {
        Some(v) => v.clone(),
        None => return axum::Json(serde_json::json!({ "error": "missing 'commands' array" })),
    };
    let config_json = commands.to_string();
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::save_preset(&conn, &name, &config_json)
    })
    .await;
    match result {
        Ok(Ok(())) => axum::Json(serde_json::json!({ "ok": true })),
        Ok(Err(e)) => axum::Json(serde_json::json!({ "error": e })),
        Err(_) => axum::Json(serde_json::json!({ "error": "Task panicked" })),
    }
}

/// `GET /api/presets` — list all saved presets.
pub(crate) async fn api_list_presets(
    State(state): State<WebState>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || fire_red_database::list_presets(&conn)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `DELETE /api/preset/:name` — delete a preset.
pub(crate) async fn api_delete_preset(
    State(state): State<WebState>,
    Path(name): Path<String>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::delete_preset(&conn, &name)).await;
    match result {
        Ok(true) => axum::Json(serde_json::json!({ "ok": true })),
        _ => axum::Json(serde_json::json!({ "error": "preset not found" })),
    }
}

/// `POST /api/preset/:name/apply` — enqueue all commands from a preset for a slot.
///
/// Body: `{ "slot": <usize> }`.
pub(crate) async fn api_apply_preset(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    if !state.allow_injections {
        return axum::Json(serde_json::json!({ "error": "injection commands are disabled" }));
    }
    let conn = require_db!(state.clone());
    let slot_idx = match body["slot"].as_u64().and_then(|v| usize::try_from(v).ok()) {
        Some(v) => v,
        None => return axum::Json(serde_json::json!({ "error": "'slot' must be a non-negative integer" })),
    };
    let slots = state.live_slots.lock_or_recover().clone();
    if slot_idx >= slots.len() {
        return axum::Json(serde_json::json!({ "error": "slot index out of range" }));
    }
    if let Some(rid) = slots[slot_idx].db.as_ref().and_then(|db| db.get_run_id()) {
        let uid = user.id;
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);
        if !can {
            return axum::Json(serde_json::json!({ "error": "access denied" }));
        }
    }
    let commands_val = match tokio::task::spawn_blocking(move || {
        fire_red_database::get_preset(&conn, &name)
    })
    .await
    {
        Ok(Some(v)) => v,
        Ok(None) => return axum::Json(serde_json::json!({ "error": "preset not found" })),
        Err(_) => return axum::Json(serde_json::json!({ "error": "Task panicked" })),
    };
    let arr = match commands_val.as_array() {
        Some(a) => a.clone(),
        None => return axum::Json(serde_json::json!({ "error": "preset 'commands' is not an array" })),
    };
    let mut count = 0usize;
    {
        let mut queue = slots[slot_idx].command_queue.lock_or_recover();
        for val in &arr {
            if let Ok(msg) = serde_json::from_value::<ClientMessage>(val.clone()) {
                queue.push_back(msg);
                count += 1;
            }
        }
    }
    axum::Json(serde_json::json!({ "queued": count }))
}
