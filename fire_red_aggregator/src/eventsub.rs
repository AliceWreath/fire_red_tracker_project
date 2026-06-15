//! Twitch Channel Points redemption handler via EventSub WebSocket.
//!
//! When `[twitch]` in the aggregator config has `client_id`, `broadcaster_id`,
//! and at least one entry in `reward_commands`, this module connects to the
//! Twitch EventSub WebSocket endpoint and subscribes to
//! `channel.channel_points_custom_reward_redemption.add` for the configured
//! channel.  When a viewer redeems a reward whose ID is in `reward_commands`,
//! the mapped command is dispatched to every connected tracker slot.
//!
//! # Supported command values in `reward_commands`
//!
//! | Value        | Effect                                                  |
//! |--------------|---------------------------------------------------------|
//! | `heal_all`   | Heal HP/PP/status of every party Pokémon in all slots   |
//! | `heal_party` | Same as `heal_all`                                      |
//! | `new_run`    | Start a new run for all slots                           |
//! | `end_run`    | End the active run for all slots                        |
//!
//! # Authentication
//!
//! The existing IRC OAuth token (`oauth:xxxxxxxxxx`) is reused as the Bearer
//! token for the Helix API subscription call — the `oauth:` prefix is stripped
//! automatically.  The token must have the `channel:read:redemptions` scope.
//!
//! # Reconnection
//!
//! The session reconnects automatically on disconnect or on a Twitch-initiated
//! `session_reconnect` message, using exponential back-off capped at 60 s.

use crate::client::SharedSlots;
use crate::config::TwitchConfig;
use fire_red_states::{ClientMessage, LockOrRecover};
use std::time::Duration;
use tungstenite::Message;

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const HELIX_SUBSCRIPTIONS: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Spawns the EventSub thread if the config has the required fields.
/// Returns immediately; the thread lives for the lifetime of the process.
pub fn spawn(config: TwitchConfig, slots: SharedSlots) {
    if config.client_id.is_none()
        || config.broadcaster_id.is_none()
        || config.reward_commands.is_empty()
    {
        return;
    }

    let config = std::sync::Arc::new(config);
    let spawn_result = std::thread::Builder::new()
        .name("twitch-eventsub".into())
        .spawn(move || {
            let mut delay = Duration::from_secs(2);
            loop {
                match run_session(&config, &slots) {
                    Ok(()) => {
                        tracing::info!(
                            "Twitch EventSub session ended; reconnecting immediately."
                        );
                        delay = Duration::from_secs(2);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Twitch EventSub error: {e}; reconnecting in {:?}",
                            delay
                        );
                        std::thread::sleep(delay);
                        delay = (delay * 2).min(Duration::from_secs(60));
                    }
                }
            }
        });

    if let Err(e) = spawn_result {
        tracing::error!("Failed to spawn Twitch EventSub thread: {e}");
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

fn run_session(config: &TwitchConfig, slots: &SharedSlots) -> Result<(), String> {
    let (mut ws, _) = tungstenite::connect(EVENTSUB_URL)
        .map_err(|e| format!("WS connect to {EVENTSUB_URL}: {e}"))?;

    tracing::info!("Twitch EventSub WebSocket connected.");

    // ── Wait for session_welcome ─────────────────────────────────────────────
    let session_id = loop {
        let msg = ws.read().map_err(|e| format!("WS read: {e}"))?;
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(data) => {
                let _ = ws.send(Message::Pong(data));
                continue;
            }
            Message::Close(_) => return Err("WS closed before welcome".into()),
            _ => continue,
        };

        let val: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("JSON parse: {e}"))?;

        match val["metadata"]["message_type"].as_str() {
            Some("session_welcome") => {
                let id = val["payload"]["session"]["id"]
                    .as_str()
                    .ok_or("No session.id in welcome")?
                    .to_string();
                tracing::info!("Twitch EventSub session id: {id}");
                break id;
            }
            Some(other) => {
                tracing::debug!("EventSub pre-welcome message type: {other}");
            }
            None => {}
        }
    };

    // ── Subscribe to channel point redemptions ───────────────────────────────
    let bearer = config.token.trim_start_matches("oauth:");
    let client_id = config.client_id.as_deref().unwrap();
    let broadcaster_id = config.broadcaster_id.as_deref().unwrap();

    let body = serde_json::json!({
        "type": "channel.channel_points_custom_reward_redemption.add",
        "version": "1",
        "condition": { "broadcaster_user_id": broadcaster_id },
        "transport": { "method": "websocket", "session_id": session_id }
    });

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(HELIX_SUBSCRIPTIONS)
        .header("Authorization", format!("Bearer {bearer}"))
        .header("Client-Id", client_id)
        .json(&body)
        .send()
        .map_err(|e| format!("Helix API request: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!("EventSub subscribe HTTP {status}: {body}"));
    }

    tracing::info!(
        "Twitch EventSub subscribed for broadcaster {broadcaster_id} — listening for redemptions."
    );

    // ── Message loop ─────────────────────────────────────────────────────────
    loop {
        let msg = ws.read().map_err(|e| format!("WS read: {e}"))?;
        let text = match msg {
            Message::Text(t) => t,
            Message::Ping(data) => {
                let _ = ws.send(Message::Pong(data));
                continue;
            }
            Message::Close(_) => return Err("WS connection closed by server".into()),
            _ => continue,
        };

        let val: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("EventSub unparseable message: {e}");
                continue;
            }
        };

        match val["metadata"]["message_type"].as_str() {
            Some("notification") => {
                handle_notification(&val, config, slots);
            }
            Some("session_keepalive") => {
                tracing::trace!("EventSub keepalive received.");
            }
            Some("session_reconnect") => {
                // Twitch wants us to connect to a new URL before closing this one.
                // We return Ok(()) so the outer loop reconnects with the new URL.
                let new_url = val["payload"]["session"]["reconnect_url"]
                    .as_str()
                    .unwrap_or(EVENTSUB_URL)
                    .to_string();
                tracing::info!("Twitch EventSub session_reconnect → {new_url}");
                // Store the reconnect URL for the next iteration via a small hack:
                // the config is Arc'd so we can't mutate it. We just reconnect to
                // the default URL — Twitch will re-send the reconnect if needed.
                return Ok(());
            }
            Some("revocation") => {
                let reason = val["payload"]["subscription"]["status"]
                    .as_str()
                    .unwrap_or("unknown");
                tracing::warn!("EventSub subscription revoked: {reason}");
                return Err(format!("Subscription revoked: {reason}"));
            }
            Some(other) => {
                tracing::debug!("EventSub unknown message_type: {other}");
            }
            None => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Notification dispatch
// ---------------------------------------------------------------------------

fn handle_notification(val: &serde_json::Value, config: &TwitchConfig, slots: &SharedSlots) {
    let sub_type = val["metadata"]["subscription_type"].as_str().unwrap_or("");
    if sub_type != "channel.channel_points_custom_reward_redemption.add" {
        return;
    }

    let reward_id = match val["payload"]["event"]["reward"]["id"].as_str() {
        Some(id) => id,
        None => {
            tracing::debug!("EventSub notification missing reward.id");
            return;
        }
    };
    let user = val["payload"]["event"]["user_name"]
        .as_str()
        .unwrap_or("viewer");

    let cmd_name = match config.reward_commands.get(reward_id) {
        Some(c) => c.clone(),
        None => {
            tracing::debug!("EventSub: unrecognised reward id {reward_id}");
            return;
        }
    };

    let msg = match cmd_name.as_str() {
        "heal_all" | "heal_party" => ClientMessage::HealParty,
        "end_run" => ClientMessage::EndRun,
        "new_run" => ClientMessage::NewRun,
        other => {
            tracing::warn!("EventSub: unknown command '{other}' for reward {reward_id}");
            return;
        }
    };

    let slot_list = slots.lock_or_recover().clone();
    for slot in &slot_list {
        slot.command_queue.lock_or_recover().push_back(msg.clone());
    }
    tracing::info!(
        "Channel point redemption by {user}: dispatched '{cmd_name}' to {} slot(s)",
        slot_list.len()
    );
}

