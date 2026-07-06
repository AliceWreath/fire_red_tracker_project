//! Discord slash-command interactions endpoint.

use super::*;

// ---------------------------------------------------------------------------
// Discord slash-command interactions endpoint
// ---------------------------------------------------------------------------

/// Body of a Discord Interactions POST (we only need a handful of fields).
#[derive(serde::Deserialize)]
pub(crate) struct DiscordInteraction {
    #[serde(rename = "type")]
    kind: u8,
    data: Option<DiscordInteractionData>,
}

#[derive(serde::Deserialize)]
pub(crate) struct DiscordInteractionData {
    name: Option<String>,
}

/// `POST /interactions` — Discord Interactions endpoint.
///
/// Verifies the Ed25519 signature, responds to ping (type 1), and handles
/// `/party`, `/status`, `/deaths` application commands (type 2) ephemerally.
pub(crate) async fn discord_interactions(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // ── Signature verification ──────────────────────────────────────────────
    let public_key_hex = state
        .discord_slash
        .as_ref()
        .map(|c| c.public_key.as_str())
        .unwrap_or("");

    let sig_header = headers
        .get("x-signature-ed25519")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ts_header = headers
        .get("x-signature-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_discord_signature(public_key_hex, sig_header, ts_header, &body) {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "invalid signature" }))).into_response();
    }

    // ── Parse body ─────────────────────────────────────────────────────────
    let interaction: DiscordInteraction = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": "bad body" }))).into_response(),
    };

    // ── Handle ping (type 1) ───────────────────────────────────────────────
    if interaction.kind == 1 {
        return axum::Json(serde_json::json!({ "type": 1 })).into_response();
    }

    // ── Handle application command (type 2) ────────────────────────────────
    if interaction.kind == 2 {
        let cmd_name = interaction
            .data
            .as_ref()
            .and_then(|d| d.name.as_deref())
            .unwrap_or("");

        let content = {
            let slots = state.live_slots.lock_or_recover();
            build_slash_response(cmd_name, &slots)
        };

        // Ephemeral message response (type 4, flags 64)
        return axum::Json(serde_json::json!({
            "type": 4,
            "data": {
                "content": content,
                "flags": 64
            }
        })).into_response();
    }

    (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": "unknown interaction type" }))).into_response()
}

pub(crate) fn verify_discord_signature(public_key_hex: &str, signature_hex: &str, timestamp: &str, body: &[u8]) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};

    let pub_bytes = match hex_decode(public_key_hex) {
        Some(b) if b.len() == 32 => b,
        _ => return false,
    };
    let sig_bytes = match hex_decode(signature_hex) {
        Some(b) if b.len() == 64 => b,
        _ => return false,
    };

    let key = match VerifyingKey::from_bytes(pub_bytes[..32].try_into().unwrap_or(&[0u8; 32])) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig = Signature::from_bytes(sig_bytes[..64].try_into().unwrap_or(&[0u8; 64]));

    let mut message = timestamp.as_bytes().to_vec();
    message.extend_from_slice(body);

    use ed25519_dalek::Verifier;
    key.verify(&message, &sig).is_ok()
}

pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) { return None; }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

pub(crate) fn build_slash_response(cmd: &str, slots: &[Arc<crate::client::MonitorSlot>]) -> String {
    match cmd {
        "party" => {
            if slots.is_empty() {
                return "No active slots.".to_string();
            }
            let mut lines = Vec::new();
            for (i, slot) in slots.iter().enumerate() {
                let gs = slot.state.lock_or_recover();
                if let Some(gs) = gs.as_ref() {
                    let members: Vec<String> = gs.party.iter()
                        .filter(|m| m.box_mon.secure.growth.species > 0)
                        .map(|m| format!("{} Lv.{}", m.box_mon.secure.growth.species_string, m.level))
                        .collect();
                    if !members.is_empty() {
                        lines.push(format!("**Slot {}** ({}): {}", i + 1, gs.player_name, members.join(", ")));
                    }
                }
            }
            if lines.is_empty() { "No party data available.".to_string() } else { lines.join("\n") }
        }
        "status" => {
            if slots.is_empty() {
                return "No active slots.".to_string();
            }
            let slot = &slots[0];
            let gs = slot.state.lock_or_recover();
            if let Some(gs) = gs.as_ref() {
                let badges = gs.badge_state.as_ref()
                    .map(|b| b.badges.iter().filter(|&&v| v).count())
                    .unwrap_or(0);
                let zone = if gs.zone_name.is_empty() { "unknown".to_string() } else { gs.zone_name.clone() };
                format!("**{}** — {} badge(s) — currently at {}", gs.player_name, badges, zone)
            } else {
                "Tracker connected but no game data yet.".to_string()
            }
        }
        "deaths" => {
            let total: usize = slots.iter().map(|slot| {
                slot.db.as_ref()
                    .and_then(|db| db.active_run_id())
                    .map(|_| {
                        let player = slot.state.lock_or_recover()
                            .as_ref()
                            .map(|gs| gs.player_name.clone())
                            .unwrap_or_default();
                        if let Some(db) = slot.db.as_ref() {
                            db.list_dead_with_records(&player).len()
                        } else { 0 }
                    })
                    .unwrap_or(0)
            }).sum();
            format!("Total deaths across all slots: **{}**", total)
        }
        _ => format!("Unknown command: {cmd}"),
    }
}

/// Register `/party`, `/status`, `/deaths` slash commands with Discord.
/// Called once at startup when `[discord_slash]` is configured.
pub fn register_slash_commands(cfg: &crate::config::DiscordSlashConfig) {
    let commands = serde_json::json!([
        { "name": "party", "description": "Show the current party for all connected slots", "type": 1 },
        { "name": "status", "description": "Show run status (badges, location) for slot 0", "type": 1 },
        { "name": "deaths", "description": "Show total death count across all slots", "type": 1 }
    ]);

    let url = if let Some(guild_id) = cfg.guild_id {
        format!(
            "https://discord.com/api/v10/applications/{}/guilds/{}/commands",
            cfg.app_id, guild_id
        )
    } else {
        format!(
            "https://discord.com/api/v10/applications/{}/commands",
            cfg.app_id
        )
    };

    let token = cfg.token.clone();
    let commands_str = commands.to_string();
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::new();
        match client
            .put(&url)
            .header("Authorization", format!("Bot {}", token))
            .header("content-type", "application/json")
            .body(commands_str)
            .send()
        {
            Ok(r) if r.status().is_success() => {
                tracing::info!("Discord slash commands registered successfully.");
            }
            Ok(r) => {
                tracing::warn!("Discord slash command registration failed: HTTP {}", r.status());
            }
            Err(e) => {
                tracing::warn!("Discord slash command registration error: {e}");
            }
        }
    });
}
