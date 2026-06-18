//! Interactive terminal configuration editor for the aggregator.
//!
//! Invoked with `--config-editor-cli`.  Covers every field exposed by the
//! egui setup window and adds the optional integration sections it omits
//! (Twitch, Discord, YouTube, LiveSplit).

use crate::config::{
    AggregatorConfig, AggregatorTestOverrides, DiscordLiveEmbedConfig, DiscordRunThreadConfig,
    DiscordSlashConfig, TwitchConfig, YouTubeChatConfig, save_config,
};
use fire_red_game_loop::config::DupesClauseMode;
use std::io::{self, Write};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Low-level I/O helpers
// ---------------------------------------------------------------------------

fn flush() {
    let _ = io::stdout().flush();
}

fn read_line() -> String {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap_or_default();
    buf.trim_end_matches('\n').trim_end_matches('\r').to_string()
}

fn section(title: &str) {
    let pad = 56usize.saturating_sub(title.len() + 5);
    println!();
    println!("  ── {title} {}", "─".repeat(pad));
}

// ---------------------------------------------------------------------------
// Typed prompt helpers
// ---------------------------------------------------------------------------

fn prompt_str(label: &str, current: &str, hint: &str) -> String {
    if hint.is_empty() {
        print!("  {label} [{current}]: ");
    } else {
        print!("  {label} [{current}]\n    ({hint}): ");
    }
    flush();
    let input = read_line();
    if input.is_empty() { current.to_string() } else { input }
}

fn prompt_opt_str(label: &str, current: Option<&str>, hint: &str) -> Option<String> {
    let display = current.unwrap_or("(none)");
    if hint.is_empty() {
        print!("  {label} [{display}] (Enter=keep, 'clear'=remove): ");
    } else {
        print!("  {label} [{display}]\n    ({hint}, 'clear'=remove): ");
    }
    flush();
    let input = read_line();
    match input.as_str() {
        "" => current.map(String::from),
        "clear" => None,
        v => Some(v.to_string()),
    }
}

fn prompt_u16(label: &str, current: u16) -> u16 {
    loop {
        print!("  {label} [{current}]: ");
        flush();
        let input = read_line();
        if input.is_empty() {
            return current;
        }
        match input.parse::<u16>() {
            Ok(n) if n > 0 => return n,
            _ => println!("    ✗  Enter a port number 1–65535."),
        }
    }
}

fn prompt_opt_u16(label: &str, current: Option<u16>) -> Option<u16> {
    loop {
        let display = current.map(|n| n.to_string()).unwrap_or_else(|| "(none)".to_string());
        print!("  {label} [{display}] (Enter=keep, 'clear'=remove): ");
        flush();
        let input = read_line();
        match input.as_str() {
            "" => return current,
            "clear" => return None,
            v => match v.parse::<u16>() {
                Ok(n) if n > 0 => return Some(n),
                _ => println!("    ✗  Enter a port number 1–65535 or 'clear'."),
            },
        }
    }
}

fn prompt_opt_u64(label: &str, current: Option<u64>) -> Option<u64> {
    loop {
        let display = current.map(|n| n.to_string()).unwrap_or_else(|| "(none)".to_string());
        print!("  {label} [{display}] (Enter=keep, 'clear'=remove): ");
        flush();
        let input = read_line();
        match input.as_str() {
            "" => return current,
            "clear" => return None,
            v => match v.parse::<u64>() {
                Ok(n) => return Some(n),
                Err(_) => println!("    ✗  Enter a number or 'clear'."),
            },
        }
    }
}

fn prompt_u64_clamped(label: &str, current: u64, min: u64, max: u64, default: u64) -> u64 {
    loop {
        print!("  {label} [{current}] ({min}–{max}, blank={default}): ");
        flush();
        let input = read_line();
        if input.is_empty() {
            return default;
        }
        match input.parse::<u64>() {
            Ok(n) if (min..=max).contains(&n) => return n,
            Ok(n) => println!("    ✗  Must be {min}–{max} (got {n})."),
            Err(_) => println!("    ✗  Enter a number between {min} and {max}."),
        }
    }
}

fn prompt_bool(label: &str, current: bool) -> bool {
    let display = if current { "y" } else { "n" };
    loop {
        print!("  {label} [{display}] (y/n): ");
        flush();
        let input = read_line();
        match input.to_lowercase().as_str() {
            "" => return current,
            "y" | "yes" | "true" | "1" => return true,
            "n" | "no" | "false" | "0" => return false,
            _ => println!("    ✗  Enter y or n."),
        }
    }
}

fn prompt_usize(label: &str, current: usize) -> usize {
    loop {
        print!("  {label} [{current}]: ");
        flush();
        let input = read_line();
        if input.is_empty() {
            return current;
        }
        match input.parse::<usize>() {
            Ok(n) => return n,
            Err(_) => println!("    ✗  Enter a non-negative integer."),
        }
    }
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

fn db_strip(raw: &str) -> String {
    raw.trim_start_matches("postgresql://")
        .trim_start_matches("postgres://")
        .to_string()
}

fn db_full(raw: &str) -> String {
    let s = raw.trim();
    if s.starts_with("postgresql://") || s.starts_with("postgres://") {
        s.to_string()
    } else {
        format!("postgresql://{s}")
    }
}

// ---------------------------------------------------------------------------
// Enum prompt
// ---------------------------------------------------------------------------

fn prompt_dupes_clause(current: DupesClauseMode) -> DupesClauseMode {
    let idx = match current {
        DupesClauseMode::Off => 1,
        DupesClauseMode::PerPlayer => 2,
        DupesClauseMode::Shared => 3,
    };
    let cur_label = match current {
        DupesClauseMode::Off => "off",
        DupesClauseMode::PerPlayer => "per_player",
        DupesClauseMode::Shared => "shared",
    };
    loop {
        println!("  Dupes clause (current: {cur_label})");
        println!("    1) Off        — standard Nuzlocke (first encounter per area)");
        println!("    2) Per Player — skip if you already caught this species this run");
        println!("    3) Shared     — skip if any player caught this species (Soul Link / co-op)");
        print!("  Choice [{idx}]: ");
        flush();
        let input = read_line();
        match input.as_str() {
            "" => return current,
            "1" => return DupesClauseMode::Off,
            "2" => return DupesClauseMode::PerPlayer,
            "3" => return DupesClauseMode::Shared,
            _ => println!("    ✗  Enter 1, 2, or 3."),
        }
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Interactive terminal configuration editor.  Invoked by `--config-editor-cli`.
pub fn run_config_editor_cli(path: &PathBuf) {
    let existing: Option<AggregatorConfig> = if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    eprintln!("Warning: could not parse existing config ({e}); starting fresh.");
                    None
                }
            },
            Err(e) => {
                eprintln!("Warning: could not read config file ({e}); starting fresh.");
                None
            }
        }
    } else {
        None
    };

    let d = existing.as_ref();

    println!();
    println!(
        "  FireRed Aggregator — {} ({})",
        if existing.is_some() { "Edit Config" } else { "First-Run Setup" },
        path.display()
    );
    println!("  Press Enter to keep the value shown in [brackets].");
    println!("  Type 'clear' for optional fields to remove them.");

    // ── Network / Ports ────────────────────────────────────────────────────

    section("Network / Ports");

    let listen_port = prompt_u16(
        "Listen port (trackers connect here)",
        d.map(|c| c.listen_port).unwrap_or(7878),
    );

    let ws_port = prompt_opt_u16(
        "WebSocket overlay port",
        d.and_then(|c| c.ws_port).or(None),
    );

    let db_raw = prompt_opt_str(
        "Database",
        d.and_then(|c| c.db.as_deref()).map(db_strip).as_deref(),
        "host/dbname  (postgresql:// prefix added automatically if omitted)",
    );
    let db = db_raw.map(|s| db_full(&s));

    // ── Behaviour ──────────────────────────────────────────────────────────

    section("Behaviour");

    let allow_injections = prompt_bool(
        "Allow injection commands (give_item, make_shiny, etc.)?",
        d.map(|c| c.allow_injections).unwrap_or(true),
    );

    let backup_dir = prompt_opt_str(
        "Backup directory",
        d.and_then(|c| c.backup_dir.as_deref()),
        "write run JSON backups here on game-clear; blank = disabled",
    );

    // ── Direct Mode ────────────────────────────────────────────────────────

    section("Direct Mode");
    println!("  The aggregator polls RetroArch directly (no tracker binary needed).");
    println!("  Enable direct_mode to also activate the /join page for on-demand connections.");

    let direct_mode = prompt_bool(
        "Enable direct mode / /join page?",
        d.map(|c| c.direct_mode).unwrap_or(false),
    );

    let rom_path = prompt_opt_str(
        "ROM path",
        d.and_then(|c| c.rom_path.as_deref()),
        "path/to/firered.gba — required for direct mode",
    );

    // Merge legacy single host into the host list for display.
    let existing_hosts: Vec<String> = {
        let mut v = d.map(|c| c.retroarch_hosts.clone()).unwrap_or_default();
        if let Some(h) = d.and_then(|c| c.retroarch_host.as_deref())
            && !v.contains(&h.to_string()) {
            v.push(h.to_string());
        }
        v
    };
    let hosts_display = existing_hosts.join(", ");

    println!();
    println!("  RetroArch hosts — enter comma- or space-separated IPs/hostnames.");
    println!("  Current: [{}]", if hosts_display.is_empty() { "(none)" } else { &hosts_display });
    print!("  Hosts (Enter=keep, 'clear'=remove all): ");
    flush();
    let hosts_input = read_line();
    let retroarch_hosts: Vec<String> = match hosts_input.as_str() {
        "" => existing_hosts,
        "clear" => Vec::new(),
        s => s
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect(),
    };

    let retroarch_port = prompt_u16(
        "RetroArch UDP port",
        d.map(|c| c.retroarch_port).unwrap_or(55355),
    );

    let poll_ms = prompt_u64_clamped(
        "Game poll interval (ms)",
        d.map(|c| c.poll_ms).unwrap_or(100),
        20,
        2000,
        100,
    );

    let dupes_clause = prompt_dupes_clause(
        d.map(|c| c.dupes_clause).unwrap_or_default(),
    );

    let allow_species_repeats = prompt_bool(
        "Randomizer mode (allow same species on multiple routes)?",
        d.map(|c| c.allow_species_repeats).unwrap_or(false),
    );

    let run_start_balls: Option<u8> = {
        let display = d.and_then(|c| c.run_start_balls).map(|n| n.to_string()).unwrap_or_else(|| "(none)".to_string());
        loop {
            print!("  Run-start Pokéballs required [{display}] (Enter=keep, 'clear'=remove): ");
            flush();
            let input = read_line();
            match input.as_str() {
                "" => break d.and_then(|c| c.run_start_balls),
                "clear" => break None,
                v => match v.parse::<u8>() {
                    Ok(n) => break Some(n),
                    Err(_) => println!("    ✗  Enter a number 0–255 or 'clear'."),
                },
            }
        }
    };

    // ── LiveSplit ──────────────────────────────────────────────────────────

    section("LiveSplit");

    let livesplit_host = prompt_opt_str(
        "LiveSplit Server host",
        d.and_then(|c| c.livesplit_host.as_deref()),
        "blank = disabled",
    );
    let livesplit_port = if livesplit_host.is_some() {
        prompt_u16(
            "LiveSplit Server port",
            d.map(|c| c.livesplit_port).unwrap_or(16834),
        )
    } else {
        d.map(|c| c.livesplit_port).unwrap_or(16834)
    };
    let livesplit_split_on_badges = prompt_bool(
        "Split on badge earned?",
        d.map(|c| c.livesplit_split_on_badges).unwrap_or(false),
    );

    // ── Twitch Bot ─────────────────────────────────────────────────────────

    section("Twitch Bot");
    println!("  Responds to !party, !deaths, !shinies, !status in chat.");

    let twitch_prev = d.and_then(|c| c.twitch.as_ref());
    let twitch_enabled = prompt_bool(
        "Enable Twitch bot?",
        twitch_prev.is_some(),
    );

    let twitch = if twitch_enabled {
        let channel = prompt_str(
            "  Channel name (without #)",
            twitch_prev.map(|t| t.channel.as_str()).unwrap_or(""),
            "",
        );
        let nick = prompt_str(
            "  Bot account username",
            twitch_prev.map(|t| t.nick.as_str()).unwrap_or(""),
            "",
        );
        let token = prompt_str(
            "  OAuth token",
            twitch_prev.map(|t| t.token.as_str()).unwrap_or(""),
            "oauth:xxxxxxxxxx — get one at twitchapps.com/tmi",
        );
        let slot = prompt_usize(
            "  Slot index to read",
            twitch_prev.map(|t| t.slot).unwrap_or(0),
        );
        let client_id = prompt_opt_str(
            "  Client ID (for Channel Points EventSub)",
            twitch_prev.and_then(|t| t.client_id.as_deref()),
            "from dev.twitch.tv — leave blank to skip EventSub",
        );
        let broadcaster_id = prompt_opt_str(
            "  Broadcaster user ID (for EventSub)",
            twitch_prev.and_then(|t| t.broadcaster_id.as_deref()),
            "numeric Twitch user ID of the channel",
        );
        Some(TwitchConfig {
            channel,
            nick,
            token,
            slot,
            client_id,
            broadcaster_id,
            // Preserve existing reward_commands mapping — not editable here.
            reward_commands: twitch_prev
                .map(|t| t.reward_commands.clone())
                .unwrap_or_default(),
        })
    } else {
        None
    };

    // ── Discord Live Embed ─────────────────────────────────────────────────

    section("Discord Live Embed");
    println!("  Keeps a pinned message in a Discord channel updated with live party info.");

    let embed_prev = d.and_then(|c| c.discord_live_embed.as_ref());
    let embed_enabled = prompt_bool(
        "Enable Discord live embed?",
        embed_prev.is_some(),
    );

    let discord_live_embed = if embed_enabled {
        let bot_token = prompt_str(
            "  Bot token",
            embed_prev.map(|e| e.bot_token.as_str()).unwrap_or(""),
            "Bot MTc…",
        );
        let channel_id = loop {
            let current = embed_prev.map(|e| e.channel_id).unwrap_or(0);
            print!("  Channel ID [{current}]: ");
            flush();
            let input = read_line();
            if input.is_empty() && current > 0 { break current; }
            match input.parse::<u64>() {
                Ok(n) if n > 0 => break n,
                _ => println!("    ✗  Enter a Discord channel ID (large number)."),
            }
        };
        let message_id = loop {
            let current = embed_prev.map(|e| e.message_id).unwrap_or(0);
            print!("  Message ID [{current}]: ");
            flush();
            let input = read_line();
            if input.is_empty() && current > 0 { break current; }
            match input.parse::<u64>() {
                Ok(n) if n > 0 => break n,
                _ => println!("    ✗  Enter the ID of the existing message to edit."),
            }
        };
        let update_interval_secs = prompt_u64_clamped(
            "  Update interval (s)",
            embed_prev.map(|e| e.update_interval_secs).unwrap_or(30),
            10,
            3600,
            30,
        );
        Some(DiscordLiveEmbedConfig { bot_token, channel_id, message_id, update_interval_secs })
    } else {
        None
    };

    // ── Discord Run Thread ─────────────────────────────────────────────────

    section("Discord Run Thread");
    println!("  Creates a thread per run and posts milestone messages (badge, death, shiny).");

    let thread_prev = d.and_then(|c| c.discord_run_thread.as_ref());
    let thread_enabled = prompt_bool(
        "Enable Discord run thread?",
        thread_prev.is_some(),
    );

    let discord_run_thread = if thread_enabled {
        let bot_token = prompt_str(
            "  Bot token",
            thread_prev.map(|t| t.bot_token.as_str()).unwrap_or(""),
            "Bot MTc…",
        );
        let channel_id = loop {
            let current = thread_prev.map(|t| t.channel_id).unwrap_or(0);
            print!("  Channel ID [{current}]: ");
            flush();
            let input = read_line();
            if input.is_empty() && current > 0 { break current; }
            match input.parse::<u64>() {
                Ok(n) if n > 0 => break n,
                _ => println!("    ✗  Enter a Discord channel ID."),
            }
        };
        Some(DiscordRunThreadConfig { bot_token, channel_id })
    } else {
        None
    };

    // ── Discord Slash Commands ─────────────────────────────────────────────

    section("Discord Slash Commands");
    println!("  Registers Application Commands; set /interactions as your endpoint URL.");

    let slash_prev = d.and_then(|c| c.discord_slash.as_ref());
    let slash_enabled = prompt_bool(
        "Enable Discord slash commands?",
        slash_prev.is_some(),
    );

    let discord_slash = if slash_enabled {
        let app_id = loop {
            let current = slash_prev.map(|s| s.app_id).unwrap_or(0);
            print!("  Application ID [{current}]: ");
            flush();
            let input = read_line();
            if input.is_empty() && current > 0 { break current; }
            match input.parse::<u64>() {
                Ok(n) if n > 0 => break n,
                _ => println!("    ✗  Enter your Discord Application ID."),
            }
        };
        let public_key = prompt_str(
            "  Ed25519 public key (hex)",
            slash_prev.map(|s| s.public_key.as_str()).unwrap_or(""),
            "from Discord dev portal → General Information",
        );
        let token = prompt_str(
            "  Bot token",
            slash_prev.map(|s| s.token.as_str()).unwrap_or(""),
            "for registering commands at startup",
        );
        let guild_id = prompt_opt_u64(
            "  Guild ID (leave blank for global commands)",
            slash_prev.and_then(|s| s.guild_id),
        );
        Some(DiscordSlashConfig { app_id, public_key, token, guild_id })
    } else {
        None
    };

    // ── YouTube Chat Bot ───────────────────────────────────────────────────

    section("YouTube Chat Bot");
    println!("  Polls YouTube Live chat; responds to !party, !deaths, !shinies, !status.");

    let yt_prev = d.and_then(|c| c.youtube_chat.as_ref());
    let yt_enabled = prompt_bool(
        "Enable YouTube chat bot?",
        yt_prev.is_some(),
    );

    let youtube_chat = if yt_enabled {
        let api_key = prompt_str(
            "  YouTube Data API v3 key",
            yt_prev.map(|y| y.api_key.as_str()).unwrap_or(""),
            "AIza…",
        );
        let broadcast_id = prompt_str(
            "  Live broadcast ID",
            yt_prev.map(|y| y.broadcast_id.as_str()).unwrap_or(""),
            "from the live broadcast URL: ?v=<broadcast_id>",
        );
        let slot = prompt_usize(
            "  Slot index to read",
            yt_prev.map(|y| y.slot).unwrap_or(0),
        );
        let poll_secs = prompt_u64_clamped(
            "  Poll interval (s)",
            yt_prev.map(|y| y.poll_secs).unwrap_or(15),
            5,
            300,
            15,
        );
        Some(YouTubeChatConfig { api_key, broadcast_id, slot, poll_secs })
    } else {
        None
    };

    // ── Test Mode ──────────────────────────────────────────────────────────

    section("Test Mode");
    println!("  Overrides applied when --test is passed (or default_test = true).");

    let default_test = prompt_bool(
        "Always run in test mode?",
        d.map(|c| c.default_test).unwrap_or(false),
    );

    let test = {
        let prev = d.and_then(|c| c.test.as_ref());
        let test_listen = prompt_opt_u16(
            "  Test listen port",
            prev.and_then(|t| t.listen_port),
        );
        let test_db_raw = prompt_opt_str(
            "  Test database",
            prev.and_then(|t| t.db.as_deref()).map(db_strip).as_deref(),
            "overrides db when --test is active",
        );
        let test_db = test_db_raw.map(|s| db_full(&s));
        let test_ws = prompt_opt_u16("  Test WebSocket port", prev.and_then(|t| t.ws_port));

        if test_listen.is_none() && test_db.is_none() && test_ws.is_none() {
            None
        } else {
            Some(AggregatorTestOverrides {
                listen_port: test_listen,
                db: test_db,
                ws_port: test_ws,
            })
        }
    };

    // ── Assemble config ────────────────────────────────────────────────────

    let cfg = AggregatorConfig {
        listen_port,
        db,
        ws_port,
        default_test,
        test,
        allow_injections,
        twitch,
        retroarch_host: None,
        retroarch_hosts,
        retroarch_port,
        rom_path,
        poll_ms,
        dupes_clause,
        allow_species_repeats,
        run_start_balls,
        direct_mode,
        backup_dir,
        livesplit_host,
        livesplit_port,
        livesplit_split_on_badges,
        discord_slash,
        discord_live_embed,
        discord_run_thread,
        youtube_chat,
    };

    // ── Confirm & save ─────────────────────────────────────────────────────

    println!();
    let save = prompt_bool(
        &format!("Save config to {}?", path.display()),
        true,
    );
    if save {
        save_config(&cfg, path);
        println!("  Config saved.");
    } else {
        println!("  Cancelled — no changes written.");
    }
}
