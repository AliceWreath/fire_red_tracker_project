//! Optional Twitch IRC chat bot.
//!
//! When a `[twitch]` section is present in the aggregator config, this module
//! spawns a background thread that connects to `irc.chat.twitch.tv:6667` and
//! responds to viewer commands with live run state.
//!
//! # Supported commands
//!
//! | Command    | Response                                                       |
//! |------------|----------------------------------------------------------------|
//! | `!party`   | Current party members with species, level, and HP              |
//! | `!deaths`  | Death count and most-recent deaths                             |
//! | `!shinies` | Shiny encounter list for the active run (requires `--db`)      |
//! | `!status`  | One-liner: `"Player — HP/MaxHP — Zone"`                        |
//! | `!moves`   | Lead Pokémon's current move set with PP                        |
//! | `!ivs`     | Lead Pokémon's individual values                               |
//! | `!badges`  | Badge count and names earned so far                            |
//! | `!bag`     | Items pocket and ball pocket contents                          |
//! | `!map`     | Player's current location                                      |
//!
//! The bot reconnects automatically on disconnect, with exponential backoff
//! capped at 60 seconds. Connection errors are logged as warnings and never
//! crash the aggregator.

use crate::client::SharedSlots;
use crate::config::TwitchConfig;
use fire_red_states::LockOrRecover;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Spawn the Twitch IRC bot as a daemon thread. Returns immediately.
pub fn spawn(config: TwitchConfig, slots: SharedSlots, db_conn: Option<String>) {
    let config = std::sync::Arc::new(config);
    let spawn_result = std::thread::Builder::new()
        .name("twitch-irc".into())
        .spawn(move || {
            let mut delay = Duration::from_secs(2);
            loop {
                match run_session(&config, &slots, db_conn.as_deref()) {
                    Ok(()) => {
                        tracing::info!("Twitch IRC session ended cleanly; reconnecting.");
                        delay = Duration::from_secs(2);
                    }
                    Err(e) => {
                        tracing::warn!("Twitch IRC error: {e}; reconnecting in {:?}", delay);
                        std::thread::sleep(delay);
                        delay = (delay * 2).min(Duration::from_secs(60));
                    }
                }
            }
        });
    if let Err(e) = spawn_result {
        tracing::error!("Failed to spawn Twitch IRC thread: {e}");
    }
}

// ---------------------------------------------------------------------------
// Session loop
// ---------------------------------------------------------------------------

fn run_session(
    config: &TwitchConfig,
    slots: &SharedSlots,
    db_conn: Option<&str>,
) -> Result<(), String> {
    let stream = TcpStream::connect("irc.chat.twitch.tv:6667")
        .map_err(|e| format!("TCP connect: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .ok();

    let mut write_half = stream.try_clone().map_err(|e| format!("clone stream: {e}"))?;
    let reader = BufReader::new(stream);

    // Authenticate and join channel.
    let channel = format!("#{}", config.channel.trim_start_matches('#'));
    irc_send(&mut write_half, &format!("PASS {}", config.token))?;
    irc_send(&mut write_half, &format!("NICK {}", config.nick))?;
    irc_send(&mut write_half, &format!("JOIN {channel}"))?;

    tracing::info!("Twitch IRC bot joined {channel} as {}.", config.nick);

    for line in reader.lines() {
        let line = line.map_err(|e| format!("read: {e}"))?;
        let line = line.trim_end_matches('\r');

        if line.starts_with("PING") {
            // PING :tmi.twitch.tv  →  PONG :tmi.twitch.tv
            let target = line.trim_start_matches("PING").trim();
            irc_send(&mut write_half, &format!("PONG {target}"))?;
            continue;
        }

        // Parse: `:nick!user@host PRIVMSG #channel :!command`
        if let Some(reply) = handle_privmsg(line, &channel, config.slot, slots, db_conn) {
            irc_send(&mut write_half, &format!("PRIVMSG {channel} :{reply}"))?;
        }
    }

    Ok(())
}

fn irc_send(w: &mut impl Write, msg: &str) -> Result<(), String> {
    write!(w, "{msg}\r\n").map_err(|e| format!("write: {e}"))
}

// ---------------------------------------------------------------------------
// Command dispatcher
// ---------------------------------------------------------------------------

fn handle_privmsg(
    line: &str,
    channel: &str,
    slot_idx: usize,
    slots: &SharedSlots,
    db_conn: Option<&str>,
) -> Option<String> {
    // Format: `:sender!u@h PRIVMSG #channel :message`
    let privmsg_marker = format!(" PRIVMSG {channel} :");
    let msg_start = line.find(&privmsg_marker)?;
    let msg = &line[msg_start + privmsg_marker.len()..];
    let cmd = msg.trim().to_lowercase();

    match cmd.as_str() {
        "!party"   => Some(cmd_party(slot_idx, slots)),
        "!deaths"  => Some(cmd_deaths(slot_idx, slots)),
        "!shinies" => Some(cmd_shinies(slot_idx, slots, db_conn)),
        "!status"  => Some(cmd_status(slot_idx, slots)),
        "!moves"   => Some(cmd_moves(slot_idx, slots)),
        "!ivs"     => Some(cmd_ivs(slot_idx, slots)),
        "!badges"  => Some(cmd_badges(slot_idx, slots)),
        "!bag"     => Some(cmd_bag(slot_idx, slots)),
        "!map"     => Some(cmd_map(slot_idx, slots)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn cmd_party(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let state_guard = slot.state.lock_or_recover();
    let Some(gs) = state_guard.as_ref() else {
        return "Tracker not in-game.".to_string();
    };
    if gs.party.is_empty() {
        return "Party is empty.".to_string();
    }
    let parts: Vec<String> = gs
        .party
        .iter()
        .filter(|p| p.box_mon.secure.growth.species > 0)
        .map(|p| {
            let nick = p.box_mon.nickname_string.trim_matches('\0').to_string();
            let species = p
                .box_mon
                .secure
                .growth
                .species_string
                .trim_matches('\0')
                .to_string();
            let name = if nick == species || nick.is_empty() {
                species.clone()
            } else {
                format!("{nick} ({species})")
            };
            format!("{name} Lv.{} {}/{} HP", p.level, p.hp, p.max_hp)
        })
        .collect();
    if parts.is_empty() {
        "Party is empty.".to_string()
    } else {
        format!("Party: {}", parts.join(" · "))
    }
}

fn cmd_deaths(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let Some(ref db) = slot.db else {
        return "Deaths require a database connection.".to_string();
    };
    let run_id = match db.active_run_id() {
        Some(id) => id,
        None => return "No active run.".to_string(),
    };
    let label = slot.label.lock_or_recover().clone();
    let dead = db.list_dead_with_records(&label);
    if dead.is_empty() {
        return format!("{label} has 0 deaths this run — SeemsGood");
    }
    let count = dead.len();
    let names: Vec<String> = {
        let mut v: Vec<_> = dead.values().collect();
        v.sort_by_key(|d| d.died_at);
        v.iter()
            .rev()
            .take(5)
            .map(|d| format!("{} (Lv.{})", d.species_name, d.level))
            .collect()
    };
    let _ = run_id;
    format!(
        "{label} deaths: {count} — {}{}",
        names.join(", "),
        if count > 5 { ", …" } else { "" }
    )
}

fn cmd_shinies(slot_idx: usize, slots: &SharedSlots, db_conn: Option<&str>) -> String {
    let Some(conn) = db_conn else {
        return "Shinies require a database connection.".to_string();
    };
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let run_id = {
        let Some(ref db) = slot.db else {
            return "Shinies require a database connection.".to_string();
        };
        match db.active_run_id() {
            Some(id) => id,
            None => return "No active run.".to_string(),
        }
    };
    drop(snap);

    let stats = fire_red_database::shiny_stats(conn, run_id);
    let shinies = stats.get("since_last_shiny").and_then(|v| v.as_array());
    let total = stats.get("total_shinies").and_then(|v| v.as_u64()).unwrap_or(0);
    if total == 0 {
        return "No shinies this run. BibleThump".to_string();
    }
    let last = stats
        .get("last_shiny")
        .and_then(|s| s.get("species_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let _ = shinies;
    format!("Shiny encounters: {total} — last shiny was {last} PogChamp")
}

fn cmd_moves(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let state_guard = slot.state.lock_or_recover();
    let Some(gs) = state_guard.as_ref() else {
        return "Tracker not in-game.".to_string();
    };
    let lead = gs.party.iter().find(|p| p.box_mon.secure.growth.species > 0);
    let Some(lead) = lead else {
        return "Party is empty.".to_string();
    };
    let nick = lead.box_mon.nickname_string.trim_matches('\0').to_string();
    let species = lead
        .box_mon
        .secure
        .growth
        .species_string
        .trim_matches('\0')
        .to_string();
    let name = if nick == species || nick.is_empty() {
        species
    } else {
        format!("{nick} ({species})")
    };
    let moves = &lead.box_mon.secure.attack.moves;
    let pp = &lead.box_mon.secure.attack.pp;
    let move_strs: Vec<String> = moves
        .iter()
        .zip(pp.iter())
        .filter(|&(&m, _)| m != 0)
        .map(|(&m, &p)| format!("{} ({}pp)", fire_red_database::move_name(m), p))
        .collect();
    if move_strs.is_empty() {
        return format!("{name} has no moves.");
    }
    format!("{name}: {}", move_strs.join(" · "))
}

fn cmd_ivs(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let state_guard = slot.state.lock_or_recover();
    let Some(gs) = state_guard.as_ref() else {
        return "Tracker not in-game.".to_string();
    };
    let lead = gs.party.iter().find(|p| p.box_mon.secure.growth.species > 0);
    let Some(lead) = lead else {
        return "Party is empty.".to_string();
    };
    let nick = lead.box_mon.nickname_string.trim_matches('\0').to_string();
    let species = lead
        .box_mon
        .secure
        .growth
        .species_string
        .trim_matches('\0')
        .to_string();
    let name = if nick == species || nick.is_empty() {
        species
    } else {
        format!("{nick} ({species})")
    };
    let iv = &lead.box_mon.secure.misc.iv_egg_ability;
    format!(
        "{name} IVs — HP:{} Atk:{} Def:{} Spe:{} SpA:{} SpD:{}",
        iv.hp_iv, iv.attack_iv, iv.defense_iv, iv.speed_iv, iv.sp_attack_iv, iv.sp_def_iv
    )
}

fn cmd_badges(slot_idx: usize, slots: &SharedSlots) -> String {
    const BADGE_NAMES: [&str; 8] = [
        "Boulder", "Cascade", "Thunder", "Rainbow", "Soul", "Marsh", "Volcano", "Earth",
    ];
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let label = slot.label.lock_or_recover().clone();
    let (count, names) = {
        let state_guard = slot.state.lock_or_recover();
        let Some(gs) = state_guard.as_ref() else {
            return format!("{label} — not in-game");
        };
        let Some(ref bs) = gs.badge_state else {
            return format!("{label} — badge data not available");
        };
        let earned: Vec<&str> = bs
            .badges
            .iter()
            .zip(BADGE_NAMES.iter())
            .filter_map(|(&has, &n)| if has { Some(n) } else { None })
            .collect();
        (earned.len(), earned.join(", "))
    };
    if count == 0 {
        return format!("{label} has no badges yet.");
    }
    format!("{label}: {count}/8 badges — {names}")
}

fn cmd_bag(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let bag_guard = slot.bag_data.lock_or_recover();
    let Some(ref bag) = *bag_guard else {
        return "Bag data not yet available.".to_string();
    };
    let item_name = |id: u16| -> String {
        if let Some(rom) = fire_red_rom_buffer::try_get_rom() {
            fire_red_party_monitor::get_item_string_from_id(rom, id)
        } else {
            format!("Item#{id}")
        }
    };
    let items: Vec<String> = bag
        .items
        .iter()
        .map(|s| format!("{} ×{}", item_name(s.item_id), s.quantity))
        .collect();
    let balls: Vec<String> = bag
        .balls
        .iter()
        .map(|s| format!("{} ×{}", item_name(s.item_id), s.quantity))
        .collect();
    let mut parts: Vec<String> = Vec::new();
    if !items.is_empty() {
        parts.push(format!("Items: {}", items.join(", ")));
    }
    if !balls.is_empty() {
        parts.push(format!("Balls: {}", balls.join(", ")));
    }
    if parts.is_empty() {
        return "Bag is empty.".to_string();
    }
    let out = parts.join(" | ");
    // Twitch chat has a ~500-character limit
    if out.len() > 450 {
        format!("{}…", &out[..449])
    } else {
        out
    }
}

fn cmd_map(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let label = slot.label.lock_or_recover().clone();
    let state_guard = slot.state.lock_or_recover();
    let Some(gs) = state_guard.as_ref() else {
        return format!("{label} — not in-game");
    };
    let zone = if !gs.zone_name.is_empty() {
        gs.zone_name.clone()
    } else {
        let raw = fire_red_location_names::map_area_name(gs.current_map_group, gs.current_map_name);
        if raw.is_empty() {
            format!("{}:{}", gs.current_map_group, gs.current_map_name)
        } else {
            raw.to_string()
        }
    };
    format!("{label} is in {zone}")
}

fn cmd_status(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let label = slot.label.lock_or_recover().clone();
    let state_guard = slot.state.lock_or_recover();
    let Some(gs) = state_guard.as_ref() else {
        return format!("{label} — not in-game");
    };
    let lead = gs
        .party
        .first()
        .filter(|p| p.box_mon.secure.growth.species > 0);
    let hp_str = lead
        .map(|p| format!("{}/{} HP", p.hp, p.max_hp))
        .unwrap_or_else(|| "no party".to_string());
    let mg = gs.current_map_group;
    let mn = gs.current_map_name;
    let raw_zone = fire_red_location_names::map_area_name(mg, mn);
    let zone_str = if raw_zone.is_empty() {
        format!("{mg}:{mn}")
    } else {
        raw_zone.to_string()
    };
    format!("{label} — {hp_str} — {zone_str}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privmsg_parse_returns_none_for_non_commands() {
        let result = handle_privmsg(
            ":viewer!u@h PRIVMSG #test :hello there",
            "#test",
            0,
            &std::sync::Arc::new(std::sync::Mutex::new(vec![])),
            None,
        );
        assert!(result.is_none());
    }

    #[test]
    fn privmsg_no_tracker_connected_for_party() {
        let slots: SharedSlots = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let result = handle_privmsg(":v!u@h PRIVMSG #test :!party", "#test", 0, &slots, None);
        assert_eq!(result.as_deref(), Some("No tracker connected."));
    }

    #[test]
    fn privmsg_no_tracker_connected_for_status() {
        let slots: SharedSlots = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let result = handle_privmsg(":v!u@h PRIVMSG #test :!status", "#test", 0, &slots, None);
        assert_eq!(result.as_deref(), Some("No tracker connected."));
    }

    #[test]
    fn privmsg_no_db_for_deaths() {
        let slots: SharedSlots = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let result = handle_privmsg(":v!u@h PRIVMSG #test :!deaths", "#test", 0, &slots, None);
        assert_eq!(result.as_deref(), Some("No tracker connected."));
    }

    #[test]
    fn privmsg_shinies_without_db_conn() {
        let slots: SharedSlots = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let result = handle_privmsg(":v!u@h PRIVMSG #test :!shinies", "#test", 0, &slots, None);
        assert_eq!(result.as_deref(), Some("Shinies require a database connection."));
    }

    #[test]
    fn privmsg_unknown_command_returns_none() {
        let slots: SharedSlots = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let result = handle_privmsg(":v!u@h PRIVMSG #test :!unknown", "#test", 0, &slots, None);
        assert!(result.is_none());
    }
}
