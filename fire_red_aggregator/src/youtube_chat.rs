//! YouTube Live chat bot.
//!
//! Polls the YouTube Data API v3 `liveChatMessages.list` endpoint every
//! `poll_secs` seconds and responds to `!party`, `!deaths`, `!shinies`, and
//! `!status` commands, mirroring the Twitch IRC bot feature set.
//!
//! # Configuration (`[youtube_chat]` in aggregator TOML)
//!
//! ```toml
//! [youtube_chat]
//! api_key      = "AIza..."
//! broadcast_id = "dQw4w9WgXcQ"
//! slot         = 0
//! poll_secs    = 15
//! ```
//!
//! The bot uses an API key (not OAuth) and can only read and post messages if
//! the key is also authorized for `liveChatMessages.insert`. For read-only
//! response logging, an OAuth token with `youtube.force-ssl` scope is needed
//! for posting; if not available the response is logged but not sent.

use crate::client::SharedSlots;
use crate::config::YouTubeChatConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const YT_API: &str = "https://www.googleapis.com/youtube/v3";

/// Spawn the YouTube Live chat polling thread. Returns immediately.
///
/// `user_id`: when `Some`, commands are read from that user's first active slot.
/// `stop`: set to `true` to exit the polling loop.
pub fn spawn(config: YouTubeChatConfig, slots: SharedSlots, db_conn: Option<String>, user_id: Option<u32>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        run_loop(config, slots, db_conn, user_id, stop);
    });
}

fn run_loop(config: YouTubeChatConfig, slots: SharedSlots, db_conn: Option<String>, user_id: Option<u32>, stop: Arc<AtomicBool>) {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    // Step 1: resolve the liveChatId from the broadcast ID.
    let live_chat_id = match fetch_live_chat_id(&client, &config.api_key, &config.broadcast_id) {
        Some(id) => id,
        None => {
            tracing::warn!("YouTube chat: could not resolve liveChatId for broadcast {}", config.broadcast_id);
            return;
        }
    };
    tracing::info!("YouTube chat: connected to liveChatId={}", live_chat_id);

    let poll_interval = Duration::from_secs(config.poll_secs.max(5));
    let mut next_page_token: Option<String> = None;

    loop {
        if stop.load(Ordering::Relaxed) { return; }
        std::thread::sleep(poll_interval);

        let slot_idx = crate::twitch::resolve_slot(config.slot, &slots, user_id);
        let (messages, token) = fetch_chat_messages(&client, &config.api_key, &live_chat_id, next_page_token.as_deref());
        next_page_token = token;

        for (author, text) in &messages {
            let cmd = text.trim();
            let response = match cmd {
                "!party"   => Some(crate::twitch::cmd_party(slot_idx, &slots)),
                "!deaths"  => Some(crate::twitch::cmd_deaths(slot_idx, &slots)),
                "!shinies" => Some(crate::twitch::cmd_shinies(slot_idx, &slots, db_conn.as_deref())),
                "!status"  => Some(crate::twitch::cmd_status(slot_idx, &slots)),
                _ => None,
            };
            if let Some(resp) = response {
                tracing::info!("YouTube chat [{author}]: {cmd} → {resp}");
                post_chat_message(&client, &config.api_key, &live_chat_id, &resp);
            }
        }
    }
}

fn fetch_live_chat_id(
    client: &reqwest::blocking::Client,
    api_key: &str,
    broadcast_id: &str,
) -> Option<String> {
    let url = format!(
        "{YT_API}/videos?part=liveStreamingDetails&id={broadcast_id}&key={api_key}"
    );
    let r = client.get(&url).send().ok()?;
    let j: serde_json::Value = r.json().ok()?;
    j["items"][0]["liveStreamingDetails"]["activeLiveChatId"]
        .as_str()
        .map(|s| s.to_string())
}

fn fetch_chat_messages(
    client: &reqwest::blocking::Client,
    api_key: &str,
    live_chat_id: &str,
    page_token: Option<&str>,
) -> (Vec<(String, String)>, Option<String>) {
    let mut url = format!(
        "{YT_API}/liveChat/messages?liveChatId={live_chat_id}&part=snippet,authorDetails&key={api_key}&maxResults=200"
    );
    if let Some(token) = page_token {
        url.push_str("&pageToken=");
        url.push_str(token);
    }
    let r = match client.get(&url).send() {
        Ok(r) => r,
        Err(e) => { tracing::warn!("YouTube chat poll failed: {e}"); return (vec![], None); }
    };
    let j: serde_json::Value = match r.json() {
        Ok(j) => j,
        Err(e) => { tracing::warn!("YouTube chat JSON parse failed: {e}"); return (vec![], None); }
    };

    let next_token = j["nextPageToken"].as_str().map(|s| s.to_string());
    let messages: Vec<(String, String)> = j["items"]
        .as_array()
        .map(|items| {
            items.iter().filter_map(|item| {
                let author = item["authorDetails"]["displayName"].as_str()?;
                let text   = item["snippet"]["displayMessage"].as_str()?;
                Some((author.to_string(), text.to_string()))
            }).collect()
        })
        .unwrap_or_default();

    (messages, next_token)
}

fn post_chat_message(
    client: &reqwest::blocking::Client,
    api_key: &str,
    live_chat_id: &str,
    text: &str,
) {
    let url = format!("{YT_API}/liveChat/messages?part=snippet&key={api_key}");
    let body = serde_json::json!({
        "snippet": {
            "liveChatId": live_chat_id,
            "type": "textMessageEvent",
            "textMessageDetails": { "messageText": text },
        }
    });
    let result = client.post(&url).json(&body).send();
    match result {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => tracing::warn!("YouTube chat post HTTP {}: {}", r.status(), r.text().unwrap_or_default()),
        Err(e) => tracing::warn!("YouTube chat post failed: {e}"),
    }
}
