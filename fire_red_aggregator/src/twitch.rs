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
//! | `!encounter` | Wild encounter table for the current route                   |
//! | `!luck`    | Shiny luck stats for the active run (requires `--db`)          |
//! | `!timer`   | Elapsed real-time run duration (requires `--db`)               |
//!
//! The bot reconnects automatically on disconnect, with exponential backoff
//! capped at 60 seconds. Connection errors are logged as warnings and never
//! crash the aggregator.

use crate::client::SharedSlots;
use crate::config::TwitchConfig;
use fire_red_states::LockOrRecover;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Spawn the Twitch IRC bot as a daemon thread. Returns immediately.
///
/// `user_id`: when `Some`, bot responses are read from that user's first active slot.
/// `stop`: set to `true` to signal the thread to exit on its next reconnect cycle.
pub fn spawn(config: TwitchConfig, slots: SharedSlots, db_conn: Option<String>, user_id: Option<u32>, stop: Arc<AtomicBool>) {
    let config = Arc::new(config);
    let spawn_result = std::thread::Builder::new()
        .name("twitch-irc".into())
        .spawn(move || {
            let mut delay = Duration::from_secs(2);
            loop {
                if stop.load(Ordering::Relaxed) { return; }
                match run_session(&config, &slots, db_conn.as_deref(), user_id) {
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

/// Return the slot index for `user_id`'s first accessible active slot, or fall
/// back to `default_slot` when `user_id` is None or no accessible slot is found.
pub fn resolve_slot(default_slot: usize, slots: &SharedSlots, user_id: Option<u32>) -> usize {
    let Some(uid) = user_id else { return default_slot; };
    let accessible: HashSet<u32> =
        fire_red_database::get_accessible_run_ids(uid).unwrap_or_default();
    let locked = slots.lock_or_recover();
    locked
        .iter()
        .position(|s| {
            s.db.as_ref()
                .and_then(|db| db.get_run_id())
                .is_some_and(|rid| accessible.contains(&rid))
        })
        .unwrap_or(default_slot)
}

// ---------------------------------------------------------------------------
// Session loop
// ---------------------------------------------------------------------------

fn run_session(
    config: &TwitchConfig,
    slots: &SharedSlots,
    db_conn: Option<&str>,
    user_id: Option<u32>,
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
        let slot_idx = resolve_slot(config.slot, slots, user_id);
        if let Some(reply) = handle_privmsg(line, &channel, slot_idx, slots, db_conn) {
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
        "!party"     => Some(cmd_party(slot_idx, slots)),
        "!deaths"    => Some(cmd_deaths(slot_idx, slots)),
        "!shinies"   => Some(cmd_shinies(slot_idx, slots, db_conn)),
        "!status"    => Some(cmd_status(slot_idx, slots)),
        "!moves"     => Some(cmd_moves(slot_idx, slots)),
        "!ivs"       => Some(cmd_ivs(slot_idx, slots)),
        "!badges"    => Some(cmd_badges(slot_idx, slots)),
        "!bag"       => Some(cmd_bag(slot_idx, slots)),
        "!map"       => Some(cmd_map(slot_idx, slots)),
        "!encounter" => Some(cmd_encounter(slot_idx, slots)),
        "!luck"      => Some(cmd_luck(slot_idx, slots, db_conn)),
        "!timer"     => Some(cmd_timer(slot_idx, slots)),
        "!run"       => Some(cmd_run(slot_idx, slots)),
        _ if cmd.starts_with("!box") => Some(cmd_box(slot_idx, slots, &cmd)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

pub(crate) fn cmd_party(slot_idx: usize, slots: &SharedSlots) -> String {
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

pub(crate) fn cmd_deaths(slot_idx: usize, slots: &SharedSlots) -> String {
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

pub(crate) fn cmd_shinies(slot_idx: usize, slots: &SharedSlots, db_conn: Option<&str>) -> String {
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

pub(crate) fn cmd_status(slot_idx: usize, slots: &SharedSlots) -> String {
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

fn cmd_encounter(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let state_guard = slot.state.lock_or_recover();
    let Some(gs) = state_guard.as_ref() else {
        return "Tracker not in-game.".to_string();
    };
    let h = &gs.encounters;
    let zone = fire_red_location_names::map_area_name(h.map_group, h.map_num);

    // Build encounter entries for land (grass) encounters.
    let land = &h.land_mon_encounters;
    if land.encounter_rate == 0 {
        let zone_str = if zone.is_empty() {
            format!("{}:{}", h.map_group, h.map_num)
        } else {
            zone.to_string()
        };
        return format!("No wild encounters in {zone_str}.");
    }

    // FireRed grass slots: 12 entries with fixed encounter rates.
    // Rates (%) indexed by slot position: [20,20,10,10,10,10,5,5,4,4,1,1].
    const SLOT_RATES: [u8; 12] = [20, 20, 10, 10, 10, 10, 5, 5, 4, 4, 1, 1];

    // Deduplicate species by accumulating their rates.
    let mut rates: std::collections::HashMap<u16, u8> = std::collections::HashMap::new();
    for (i, wp) in land.wild_pokemon_list.iter().enumerate() {
        let rate = SLOT_RATES.get(i).copied().unwrap_or(0);
        *rates.entry(wp.species).or_insert(0) += rate;
    }

    // Sort by rate descending, then species ID for stability.
    let mut entries: Vec<(u16, u8)> = rates.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let parts: Vec<String> = entries
        .iter()
        .map(|(species, rate)| {
            let name = fire_red_text::get_pokemon_name_by_number(*species as usize)
                .unwrap_or_else(|_| format!("#{species}"));
            format!("{name} {}%", rate)
        })
        .collect();

    let zone_str = if zone.is_empty() {
        format!("{}:{}", h.map_group, h.map_num)
    } else {
        zone.to_string()
    };
    let result = format!("{zone_str}: {}", parts.join(", "));
    if result.len() > 450 {
        format!("{}…", &result[..449])
    } else {
        result
    }
}

fn cmd_luck(slot_idx: usize, slots: &SharedSlots, db_conn: Option<&str>) -> String {
    let Some(conn) = db_conn else {
        return "Luck stats require a database connection.".to_string();
    };
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let run_id = {
        let Some(ref db) = slot.db else {
            return "Luck stats require a database connection.".to_string();
        };
        match db.active_run_id() {
            Some(id) => id,
            None => return "No active run.".to_string(),
        }
    };
    drop(snap);

    let stats = fire_red_database::run_luck_stats(conn, run_id);
    let total   = stats["total_encounters"].as_u64().unwrap_or(0);
    let shinies = stats["shiny_count"].as_u64().unwrap_or(0);
    let expected = stats["expected_shinies"].as_f64().unwrap_or(0.0);
    if total == 0 {
        return "No encounters yet this run.".to_string();
    }
    format!(
        "Luck: {shinies} shiny / {total} encounters (expected {:.2}, rate 1/{:.0})",
        expected,
        if shinies == 0 { f64::INFINITY } else { total as f64 / shinies as f64 }
    )
}

fn cmd_timer(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let label = slot.label.lock_or_recover().clone();
    let Some(ref db) = slot.db else {
        return "Timer requires a database connection.".to_string();
    };
    let Some((_, _, started_at, ended_at, _, _)) = db.run_summary() else {
        return "No active run.".to_string();
    };
    let now = fire_red_database::unix_now();
    let elapsed = ended_at.unwrap_or(now).saturating_sub(started_at);
    let h = elapsed / 3600;
    let m = (elapsed % 3600) / 60;
    let s = elapsed % 60;
    let status = if ended_at.is_some() { " (ended)" } else { "" };
    format!("{label} run timer: {:02}:{:02}:{:02}{status}", h, m, s)
}

fn cmd_run(slot_idx: usize, slots: &SharedSlots) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    let Some(ref db) = slot.db else {
        return "Run info requires a database connection.".to_string();
    };
    let Some((run_id, player_name, started_at, ended_at, death_count, _catch_count)) =
        db.run_summary()
    else {
        return "No active run.".to_string();
    };
    let now = fire_red_database::unix_now();
    let elapsed = ended_at.unwrap_or(now).saturating_sub(started_at);
    let h = elapsed / 3600;
    let m = (elapsed % 3600) / 60;
    let s = elapsed % 60;
    let badge_count = {
        let state_guard = slot.state.lock_or_recover();
        state_guard
            .as_ref()
            .and_then(|gs| gs.badge_state.as_ref())
            .map(|b| b.badges.iter().filter(|&&v| v).count())
            .unwrap_or(0)
    };
    let ended_marker = if ended_at.is_some() { " [ended]" } else { "" };
    format!(
        "Run #{run_id} ({player_name}) — {h:02}:{m:02}:{s:02}{ended_marker} — {death_count} deaths — {badge_count}/8 badges"
    )
}

fn cmd_box(slot_idx: usize, slots: &SharedSlots, cmd: &str) -> String {
    let snap = slots.lock_or_recover().clone();
    let Some(slot) = snap.get(slot_idx) else {
        return "No tracker connected.".to_string();
    };
    // Parse box number: "!box 3" or "!box3"
    let n: usize = cmd
        .trim_start_matches("!box")
        .trim()
        .parse::<usize>()
        .unwrap_or(1)
        .saturating_sub(1); // convert 1-based to 0-based

    let box_data = slot.box_data.lock_or_recover().clone();
    let in_box: Vec<String> = box_data
        .iter()
        .filter(|e| e.box_index as usize == n)
        .map(|e| {
            let species = e.species_name.trim_matches('\0');
            let nick = e.nickname.trim_matches('\0');
            if nick.is_empty() || nick == species {
                species.to_string()
            } else {
                format!("{nick} ({species})")
            }
        })
        .collect();

    if in_box.is_empty() {
        return format!("Box {} is empty.", n + 1);
    }
    let result = format!("Box {}: {}", n + 1, in_box.join(", "));
    if result.len() > 450 {
        format!("{}…", &result[..449])
    } else {
        result
    }
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
