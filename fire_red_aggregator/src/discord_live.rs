//! Discord live-status embed and per-run thread integration.
//!
//! # Live embed
//!
//! Spawned by [`spawn_live_embed`]. Edits a pinned Discord message every
//! `update_interval_secs` seconds with the current party snapshot: deaths,
//! badges, and party species. Requires a bot token with `Send Messages` +
//! `Read Message History` scopes in the target channel.
//!
//! # Run thread
//!
//! Spawned by [`spawn_run_thread`]. Creates a new thread in the configured
//! channel when a new run is detected, then posts milestone replies for deaths,
//! badges, shinies, and game clear.

use crate::client::SharedSlots;
use crate::config::{DiscordLiveEmbedConfig, DiscordRunThreadConfig};
use fire_red_states::LockOrRecover;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const DISCORD_API: &str = "https://discord.com/api/v10";

// ---------------------------------------------------------------------------
// Live embed
// ---------------------------------------------------------------------------

/// Spawn a background thread that periodically edits a pinned Discord message
/// with the current tracker state. No-op if `slots` is empty on every tick.
///
/// `user_id`: when `Some`, only that user's slots appear in the embed.
/// `stop`: set to `true` to exit the loop.
pub fn spawn_live_embed(config: DiscordLiveEmbedConfig, slots: SharedSlots, user_id: Option<u32>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());

        let interval = Duration::from_secs(config.update_interval_secs.max(10));
        loop {
            if stop.load(Ordering::Relaxed) { return; }
            std::thread::sleep(interval);
            let accessible = user_id
                .and_then(|uid| fire_red_database::get_accessible_run_ids(uid).ok());
            let embed = build_live_embed(&slots, accessible.as_ref());
            edit_discord_message(&client, &config.bot_token, config.channel_id, config.message_id, embed);
        }
    });
}

fn build_live_embed(slots: &SharedSlots, accessible: Option<&HashSet<u32>>) -> serde_json::Value {
    let locked = slots.lock_or_recover();
    let mut fields: Vec<serde_json::Value> = Vec::new();

    for slot in locked.iter() {
        if let Some(ids) = accessible {
            let run_id = slot.db.as_ref().and_then(|db| db.get_run_id());
            if let Some(rid) = run_id
                && !ids.contains(&rid) { continue; }
        }
        let state = slot.state.lock_or_recover();
        let Some(ref gs) = *state else { continue };

        let party: Vec<String> = gs.party
            .iter()
            .filter(|p| p.box_mon.secure.growth.species != 0)
            .map(|p| format!("{} Lv.{}", p.box_mon.secure.growth.species_string, p.level))
            .collect();
        let party_str = if party.is_empty() { "—".to_string() } else { party.join(", ") };

        let badges = gs.badge_state.as_ref()
            .map(|b| b.badges.iter().filter(|&&v| v).count().to_string())
            .unwrap_or_else(|| "—".to_string());

        let name = slot.label.lock_or_recover().clone();

        fields.push(serde_json::json!({
            "name":  name,
            "value": format!("\u{1F3C5} {badges}/8 | Party: {party_str}"),
            "inline": false,
        }));
    }

    if fields.is_empty() {
        fields.push(serde_json::json!({
            "name":   "Status",
            "value":  "No active slots.",
            "inline": false,
        }));
    }

    serde_json::json!({
        "embeds": [{
            "title":     "Fire Red Tracker \u{2014} Live Status",
            "color":     0x3498db,
            "fields":    fields,
            "footer":    { "text": "Updates automatically \u{2022} Fire Red Tracker" },
            "timestamp": chrono_now_iso(),
        }]
    })
}

fn edit_discord_message(
    client: &reqwest::blocking::Client,
    bot_token: &str,
    channel_id: u64,
    message_id: u64,
    body: serde_json::Value,
) {
    let url = format!("{DISCORD_API}/channels/{channel_id}/messages/{message_id}");
    let result = client.patch(&url)
        .header("Authorization", format!("Bot {}", bot_token.trim_start_matches("Bot ")))
        .header("Content-Type", "application/json")
        .json(&body)
        .send();
    match result {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => tracing::warn!("Discord embed edit HTTP {}: {}", r.status(), r.text().unwrap_or_default()),
        Err(e) => tracing::warn!("Discord embed edit request failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Run thread
// ---------------------------------------------------------------------------

/// Spawn a background thread that creates a Discord thread when a new run is
/// detected and posts milestone messages for deaths, badges, shinies, and clear.
///
/// `user_id`: when `Some`, only that user's slots are tracked.
/// `stop`: set to `true` to exit the loop.
pub fn spawn_run_thread(config: DiscordRunThreadConfig, slots: SharedSlots, user_id: Option<u32>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());

        // Track state per slot: (last_run_id, thread_channel_id, death_count, badge_count)
        let mut slot_state: Vec<Option<(u32, u64, u32, u32)>> = Vec::new();

        loop {
            if stop.load(Ordering::Relaxed) { return; }
            std::thread::sleep(Duration::from_secs(5));

            let accessible: Option<HashSet<u32>> = user_id
                .and_then(|uid| fire_red_database::get_accessible_run_ids(uid).ok());

            let locked = slots.lock_or_recover();

            if slot_state.len() < locked.len() {
                slot_state.resize(locked.len(), None);
            }

            for (slot_i, slot) in locked.iter().enumerate() {
                // Skip slots that don't belong to this user.
                if let Some(ref ids) = accessible {
                    let run_id = slot.db.as_ref().and_then(|db| db.get_run_id());
                    if let Some(rid) = run_id
                        && !ids.contains(&rid) { continue; }
                }
                let state = slot.state.lock_or_recover();
                let Some(ref gs) = *state else { continue };

                let run_id = match slot.db.as_ref().and_then(|db| db.active_run_id()) {
                    Some(id) => id,
                    None => continue,
                };

                let badge_count = gs.badge_state.as_ref()
                    .map(|b| b.badges.iter().filter(|&&v| v).count() as u32)
                    .unwrap_or(0);

                let player_name = slot.label.lock_or_recover().clone();

                if let Some((last_run_id, thread_id, last_deaths, last_badges)) = slot_state[slot_i] {
                    if run_id != last_run_id {
                        let new_thread_id = create_discord_thread(
                            &client, &config.bot_token, config.channel_id,
                            &format!("Run #{run_id} \u{2014} {player_name}"),
                        );
                        slot_state[slot_i] = Some((run_id, new_thread_id.unwrap_or(0), 0, badge_count));
                    } else if badge_count > last_badges && thread_id != 0 {
                        post_thread_message(
                            &client, &config.bot_token, thread_id,
                            &format!("\u{1F3C5} Badge #{badge_count} earned!"),
                        );
                        slot_state[slot_i] = Some((run_id, thread_id, last_deaths, badge_count));
                    }
                } else {
                    let new_thread_id = create_discord_thread(
                        &client, &config.bot_token, config.channel_id,
                        &format!("Run #{run_id} \u{2014} {player_name}"),
                    );
                    slot_state[slot_i] = Some((run_id, new_thread_id.unwrap_or(0), 0, badge_count));
                }
            }
        }
    });
}

fn create_discord_thread(
    client: &reqwest::blocking::Client,
    bot_token: &str,
    channel_id: u64,
    name: &str,
) -> Option<u64> {
    let url = format!("{DISCORD_API}/channels/{channel_id}/threads");
    let body = serde_json::json!({
        "name":                   &name[..name.len().min(100)],
        "auto_archive_duration":  10080,  // 7 days
        "type":                   11,     // PUBLIC_THREAD
    });
    let result = client.post(&url)
        .header("Authorization", format!("Bot {}", bot_token.trim_start_matches("Bot ")))
        .header("Content-Type", "application/json")
        .json(&body)
        .send();
    match result {
        Ok(r) if r.status().is_success() => {
            let j: serde_json::Value = r.json().unwrap_or_default();
            j["id"].as_str().and_then(|s| s.parse().ok())
        }
        Ok(r) => {
            tracing::warn!("Discord create thread HTTP {}: {}", r.status(), r.text().unwrap_or_default());
            None
        }
        Err(e) => { tracing::warn!("Discord create thread failed: {e}"); None }
    }
}

fn post_thread_message(
    client: &reqwest::blocking::Client,
    bot_token: &str,
    thread_id: u64,
    content: &str,
) {
    let url = format!("{DISCORD_API}/channels/{thread_id}/messages");
    let result = client.post(&url)
        .header("Authorization", format!("Bot {}", bot_token.trim_start_matches("Bot ")))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "content": content }))
        .send();
    if let Err(e) = result {
        tracing::warn!("Discord post thread message failed: {e}");
    }
}

fn chrono_now_iso() -> String {
    let secs = fire_red_database::unix_now();
    chrono_unix_to_iso(secs).to_string()
}

fn chrono_unix_to_iso(secs: u64) -> String {
    // Minimal ISO 8601 without chrono dependency — just format the UNIX timestamp as a date.
    // Discord accepts UNIX timestamps as ISO 8601: "2026-06-17T00:00:00.000Z"
    // We use a simplified formatter for whole seconds.
    let secs_i = secs as i64;
    let days_since_epoch = secs_i / 86400;
    let time_of_day = secs_i % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Convert days since Unix epoch to calendar date using Proleptic Gregorian calendar.
    let (year, month, day) = days_to_ymd(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.000Z")
}

fn days_to_ymd(mut z: i64) -> (i32, u32, u32) {
    z += 719468;
    let era: i64 = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}
