//! WebSocket endpoint: per-client state push loop.

use super::*;

pub(crate) async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let show = params.get("show").cloned();
    let user_id = user.id;
    ws.on_upgrade(move |socket| {
        handle_socket(socket, state.tx.subscribe(), state.live_slots, show, user_id)
    })
}

/// Strips fields from a slot-array JSON string that the given `show` view does
/// not render, reducing per-tick payload size for narrow views.
pub(crate) fn filter_slots_json(json: &str, show: &str) -> String {
    let strip: &[&str] = match show {
        "box" => &[
            "party",
            "encounters",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        "dead" => &["encounters", "box_pokemon", "caught", "prev_run_encounters"],
        "caught" => &["encounters", "box_pokemon", "dead", "prev_run_encounters"],
        "memorial" => &[
            "encounters",
            "box_pokemon",
            "caught",
            "prev_run_encounters",
            "db_encounters",
        ],
        "soullink" => &[
            "encounters",
            "box_pokemon",
            "db_encounters",
            "prev_run_encounters",
        ],
        // types page only needs party (with type fields), badge state, and next_gym.
        "types" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // deaths overlay only needs the dead list and run_summary.
        "deaths" => &[
            "party",
            "encounters",
            "box_pokemon",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // counter overlay only needs db_encounters for counts.
        "counter" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "prev_run_encounters",
        ],
        // hp overlay only needs party (hp/status) and badges.
        "hp" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // badges overlay only needs badges.
        "badges" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // nextgym overlay needs party (types) + next_gym + badges.
        "nextgym" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // encounter_table overlay only needs encounters (with species_name/rate).
        "encounter_table" => &[
            "party",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // money overlay only needs the money field.
        "money" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // playtime overlay needs play_time_* and run_summary (for wall-clock).
        "playtime" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // goals overlay only needs goals list.
        "goals" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // damage_calc overlay only needs damage_panel (+ connected).
        "damage_calc" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // vs_leader overlay needs next_gym + leader_party + party types.
        "vs_leader" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        _ => return json.to_owned(),
    };
    let Ok(mut slots) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return json.to_owned();
    };
    for slot in &mut slots {
        if let Some(obj) = slot.as_object_mut() {
            for key in strip {
                obj.remove(*key);
            }
        }
    }
    serde_json::to_string(&slots).unwrap_or_else(|_| json.to_owned())
}

pub(crate) async fn handle_socket(
    socket: axum::extract::ws::WebSocket,
    mut rx: watch::Receiver<String>,
    live_slots: SharedSlots,
    show: Option<String>,
    user_id: u32,
) {
    let is_owner = user_id == 1;

    // Fetch accessible run IDs at connect time; refreshed every 30 s in the
    // broadcast loop so invite changes take effect without reconnecting.
    let mut accessible: HashSet<u32> =
        tokio::task::spawn_blocking(move || fire_red_database::get_accessible_run_ids(user_id))
            .await
            .unwrap_or(Ok(HashSet::new()))
            .unwrap_or_default();
    let mut accessible_refreshed = std::time::Instant::now();

    // Filter helper: takes an explicit accessible set so the closure can be
    // called with the periodically-refreshed set rather than a captured snapshot.
    let filter_json = |raw: &str, acc: &HashSet<u32>| -> String {
        let arr: serde_json::Value =
            serde_json::from_str(raw).unwrap_or(serde_json::Value::Array(vec![]));
        let user_slots: Vec<serde_json::Value> = arr
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|s| match s.get("active_run_id").and_then(|v| v.as_u64()) {
                None if is_owner => s,
                None => serde_json::Value::Null,
                Some(rid) if acc.contains(&(rid as u32)) => s,
                _ => serde_json::Value::Null,
            })
            .collect();
        let filtered =
            serde_json::to_string(&serde_json::Value::Array(user_slots)).unwrap_or_default();
        match &show {
            Some(s) => filter_slots_json(&filtered, s),
            None => filtered,
        }
    };

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Send current state immediately so the browser isn't blank on connect.
    {
        let current = rx.borrow_and_update().clone();
        if !current.is_empty() {
            let msg = filter_json(&current, &accessible);
            if ws_tx
                .send(axum::extract::ws::Message::Text(msg))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // Forward incoming browser commands only to slots accessible by this user.
    // Reuse the initial accessible snapshot — no second DB round-trip needed.
    let live_slots_cmd = live_slots.clone();
    let accessible_cmd = accessible.clone();
    tokio::spawn(async move {
        while let Some(Ok(axum::extract::ws::Message::Text(text))) = ws_rx.next().await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                let msg = match val["cmd"].as_str().unwrap_or("") {
                    "end_run" => Some(ClientMessage::EndRun),
                    "new_run" => Some(ClientMessage::NewRun),
                    _ => None,
                };
                if let Some(msg) = msg {
                    let slots = live_slots_cmd.lock_or_recover().clone();
                    for slot in &slots {
                        let run_id = slot.db.as_ref().and_then(|db| db.get_run_id());
                        let allowed = match run_id {
                            None => true,
                            Some(rid) => accessible_cmd.contains(&rid),
                        };
                        if allowed {
                            slot.command_queue.lock_or_recover().push_back(msg.clone());
                        }
                    }
                }
            }
        }
    });

    // Push state updates whenever the broadcast channel changes.
    loop {
        if rx.changed().await.is_err() {
            break;
        }
        // Refresh the accessible-run set every 30 s so invite changes
        // mid-session take effect without disconnecting the client.
        if accessible_refreshed.elapsed() >= std::time::Duration::from_secs(30) {
            accessible = tokio::task::spawn_blocking(move || fire_red_database::get_accessible_run_ids(user_id))
                .await
                .unwrap_or(Ok(HashSet::new()))
                .unwrap_or_default();
            accessible_refreshed = std::time::Instant::now();
        }
        let raw = rx.borrow_and_update().clone();
        let msg = filter_json(&raw, &accessible);
        if ws_tx
            .send(axum::extract::ws::Message::Text(msg))
            .await
            .is_err()
        {
            break;
        }
    }
}
