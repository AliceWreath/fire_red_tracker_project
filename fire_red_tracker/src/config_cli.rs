//! Interactive terminal configuration editor.
//!
//! Invoked with `--config-editor-cli`.  Mirrors every field exposed by the
//! GUI editor and adds a few it omits (LiveSplit, Discord Rich Presence,
//! username, desktop notifications).

use crate::config::{
    ConfigMode, DupesClauseMode, ObsConfig, TrackerConfig, TrackerTestOverrides,
    WebhookConfig, save_config,
};
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

/// Prompt for a required string.  Enter keeps `current`.
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

/// Prompt for an optional string.  Enter keeps current, `clear` sets to None.
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

/// Prompt for a required u16 (port).  Enter keeps `current`.
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

/// Prompt for an optional u8.  Enter keeps current, `clear` sets to None.
fn prompt_opt_u8(label: &str, current: Option<u8>) -> Option<u8> {
    loop {
        let display = current.map(|n| n.to_string()).unwrap_or_else(|| "(none)".to_string());
        print!("  {label} [{display}] (Enter=keep, 'clear'=remove): ");
        flush();
        let input = read_line();
        match input.as_str() {
            "" => return current,
            "clear" => return None,
            v => match v.parse::<u8>() {
                Ok(n) => return Some(n),
                Err(_) => println!("    ✗  Enter a number 0–255 or 'clear'."),
            },
        }
    }
}

/// Prompt for an optional u16.  Enter keeps current, `clear` sets to None.
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

/// Prompt for an optional u64.  Enter keeps current, `clear` sets to None.
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

/// Prompt for a u64 clamped to [min, max].  Enter returns `default`.
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

/// Prompt for a bool.  Enter keeps `current`.
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

/// Prompt for a player slot number (1+) or blank for auto.
fn prompt_player_number(current: Option<u8>) -> Option<u8> {
    loop {
        let display = current.map(|n| n.to_string()).unwrap_or_else(|| "(auto)".to_string());
        print!("  Player number [{display}] (1, 2, … or blank for auto): ");
        flush();
        let input = read_line();
        match input.as_str() {
            "" => return current,
            v => match v.parse::<u8>().ok().filter(|&n| n >= 1) {
                Some(n) => return Some(n),
                None => println!("    ✗  Enter a number ≥ 1 or leave blank."),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Enum prompts
// ---------------------------------------------------------------------------

fn prompt_mode(current: &ConfigMode) -> ConfigMode {
    let idx = match current {
        ConfigMode::Standalone => 1,
        ConfigMode::Connected => 2,
    };
    loop {
        let cur_label = match current {
            ConfigMode::Standalone => "standalone",
            ConfigMode::Connected => "connected",
        };
        println!("  Mode (current: {cur_label})");
        println!("    1) Standalone — tracker runs independently");
        println!("    2) Connected  — connects to an aggregator over TCP");
        print!("  Choice [{idx}]: ");
        flush();
        let input = read_line();
        match input.as_str() {
            "" => return current.clone(),
            "1" => return ConfigMode::Standalone,
            "2" => return ConfigMode::Connected,
            _ => println!("    ✗  Enter 1 or 2."),
        }
    }
}

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
// Main entry point
// ---------------------------------------------------------------------------

/// Interactive terminal configuration editor.  Invoked by `--config-editor-cli`.
pub fn run_config_editor_cli(path: &PathBuf) {
    let existing: Option<TrackerConfig> = if path.exists() {
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

    let d = existing.as_ref(); // shorthand for "default / existing"

    println!();
    println!(
        "  FireRed Tracker — {} ({})",
        if existing.is_some() { "Edit Config" } else { "First-Run Setup" },
        path.display()
    );
    println!("  Press Enter to keep the value shown in [brackets].");
    println!("  Type 'clear' for optional fields to remove them.");

    // ── ROM / Database ─────────────────────────────────────────────────────

    section("ROM / Database");

    let rom = prompt_str(
        "ROM path",
        d.map(|c| c.rom.as_str()).unwrap_or(""),
        "path/to/firered.gba",
    );
    let db_raw = prompt_str(
        "Database",
        &d.map(|c| db_strip(&c.db))
            .unwrap_or_else(|| "localhost/nuzlocke".to_string()),
        "host/dbname  (postgresql:// prefix added automatically if omitted)",
    );
    let clean = prompt_bool(
        "Clean start (wipe DB on next launch)?",
        d.map(|c| c.clean).unwrap_or(false),
    );

    // ── Connection Mode ────────────────────────────────────────────────────

    section("Connection Mode");

    let mode = prompt_mode(d.map(|c| &c.mode).unwrap_or(&ConfigMode::Standalone));

    let (aggregator_host, aggregator_port, preferred_player) = if mode == ConfigMode::Connected {
        let host = prompt_str(
            "Aggregator host",
            d.map(|c| c.aggregator_host.as_str()).unwrap_or("127.0.0.1"),
            "",
        );
        let port = prompt_u16(
            "Aggregator port",
            d.map(|c| c.aggregator_port).unwrap_or(7878),
        );
        let player = prompt_player_number(d.and_then(|c| c.preferred_player));
        (host, port, player)
    } else {
        (
            d.map(|c| c.aggregator_host.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            d.map(|c| c.aggregator_port).unwrap_or(7878),
            d.and_then(|c| c.preferred_player),
        )
    };

    // ── Run Settings ───────────────────────────────────────────────────────

    section("Run Settings");

    let poll_ms = prompt_u64_clamped(
        "Poll interval (ms)",
        d.map(|c| c.poll_ms).unwrap_or(100),
        20,
        2000,
        100,
    );
    let dupes_clause =
        prompt_dupes_clause(d.map(|c| c.dupes_clause).unwrap_or(DupesClauseMode::Off));
    let allow_species_repeats = prompt_bool(
        "Randomizer mode (allow same species on multiple routes)?",
        d.map(|c| c.allow_species_repeats).unwrap_or(false),
    );
    let run_start_balls = prompt_opt_u8(
        "Run-start Pokéballs required",
        d.and_then(|c| c.run_start_balls),
    );
    let username = prompt_opt_str(
        "Account username",
        d.and_then(|c| c.username.as_deref()),
        "links runs to a user account on the aggregator",
    );

    // ── Test Mode ──────────────────────────────────────────────────────────

    section("Test Mode");
    println!("  When enabled, these overrides are applied every launch (same as --test).");

    let default_test = prompt_bool(
        "Always run in test mode?",
        d.map(|c| c.default_test).unwrap_or(false),
    );
    let test = {
        let prev = d.and_then(|c| c.test.as_ref());
        let test_db = prompt_opt_str(
            "  Test DB",
            prev.and_then(|t| t.db.as_deref())
                .map(db_strip)
                .as_deref(),
            "overrides DB when running in test mode",
        )
        .map(|s| db_full(&s));
        let test_host = prompt_opt_str(
            "  Test aggregator host",
            prev.and_then(|t| t.aggregator_host.as_deref()),
            "",
        );
        let test_port =
            prompt_opt_u16("  Test aggregator port", prev.and_then(|t| t.aggregator_port));
        let test_player =
            prompt_opt_u8("  Test player number", prev.and_then(|t| t.preferred_player));
        if test_db.is_none()
            && test_host.is_none()
            && test_port.is_none()
            && test_player.is_none()
        {
            None
        } else {
            Some(TrackerTestOverrides {
                db: test_db,
                aggregator_host: test_host,
                aggregator_port: test_port,
                preferred_player: test_player,
            })
        }
    };

    // ── OBS Clips ──────────────────────────────────────────────────────────

    section("OBS Clips");
    println!("  Save replay-buffer segments automatically on key events.");

    let obs_prev = d.map(|c| &c.obs);
    let obs_clip_death =
        prompt_bool("Clip on death?", obs_prev.map(|o| o.clip_on_death).unwrap_or(false));
    let obs_clip_shiny =
        prompt_bool("Clip on shiny?", obs_prev.map(|o| o.clip_on_shiny).unwrap_or(false));
    let obs_clip_wipe =
        prompt_bool("Clip on wipe?", obs_prev.map(|o| o.clip_on_wipe).unwrap_or(false));
    let obs_clip_badge =
        prompt_bool("Clip on badge?", obs_prev.map(|o| o.clip_on_badge).unwrap_or(false));

    let obs_used =
        obs_clip_death || obs_clip_shiny || obs_clip_wipe || obs_clip_badge;
    let (obs_host, obs_port, obs_password) = if obs_used {
        let host = prompt_str(
            "  OBS host",
            obs_prev.map(|o| o.host.as_str()).unwrap_or("localhost"),
            "",
        );
        let port = prompt_u16("  OBS port", obs_prev.map(|o| o.port).unwrap_or(4455));
        let pass = prompt_opt_str(
            "  OBS password",
            obs_prev.and_then(|o| o.password.as_deref()),
            "blank = auth disabled",
        );
        (host, port, pass)
    } else {
        (
            obs_prev
                .map(|o| o.host.clone())
                .unwrap_or_else(|| "localhost".to_string()),
            obs_prev.map(|o| o.port).unwrap_or(4455),
            obs_prev.and_then(|o| o.password.clone()),
        )
    };

    // ── Webhooks ────────────────────────────────────────────────────────────

    section("Webhooks");
    println!("  POST JSON to a URL on game events (Discord, stream alerts, etc.).");
    println!("  Templates may use {{player}}, {{pokemon.nickname}}, {{pokemon.species}}, etc.");

    let wh_prev = d.map(|c| &c.webhooks);

    let death_url = prompt_opt_str(
        "Death URL",
        wh_prev.and_then(|w| w.death_url.as_deref()),
        "https://…",
    );
    let death_template = if death_url.is_some() {
        prompt_opt_str(
            "  Death template",
            wh_prev.and_then(|w| w.death_template.as_deref()),
            r#"blank = default JSON"#,
        )
    } else {
        None
    };

    let catch_url = prompt_opt_str(
        "Catch URL",
        wh_prev.and_then(|w| w.catch_url.as_deref()),
        "https://…",
    );
    let catch_template = if catch_url.is_some() {
        prompt_opt_str(
            "  Catch template",
            wh_prev.and_then(|w| w.catch_template.as_deref()),
            "",
        )
    } else {
        None
    };

    let shiny_url = prompt_opt_str(
        "Shiny URL",
        wh_prev.and_then(|w| w.shiny_url.as_deref()),
        "https://…",
    );
    let shiny_template = if shiny_url.is_some() {
        prompt_opt_str(
            "  Shiny template",
            wh_prev.and_then(|w| w.shiny_template.as_deref()),
            "",
        )
    } else {
        None
    };

    let wipe_url = prompt_opt_str(
        "Wipe URL",
        wh_prev.and_then(|w| w.wipe_url.as_deref()),
        "https://…",
    );
    let wipe_template = if wipe_url.is_some() {
        prompt_opt_str(
            "  Wipe template",
            wh_prev.and_then(|w| w.wipe_template.as_deref()),
            "",
        )
    } else {
        None
    };

    let notify_on_death = prompt_bool(
        "Desktop notification on death?",
        wh_prev.map(|w| w.notify_on_death).unwrap_or(false),
    );
    let notify_on_shiny = prompt_bool(
        "Desktop notification on shiny?",
        wh_prev.map(|w| w.notify_on_shiny).unwrap_or(false),
    );
    let notify_on_wipe = prompt_bool(
        "Desktop notification on wipe?",
        wh_prev.map(|w| w.notify_on_wipe).unwrap_or(false),
    );

    // ── LiveSplit ───────────────────────────────────────────────────────────

    section("LiveSplit");

    let livesplit_host = prompt_opt_str(
        "LiveSplit Server host",
        d.and_then(|c| c.livesplit_host.as_deref()),
        "blank = disabled",
    );
    let livesplit_port = if livesplit_host.is_some() {
        prompt_opt_u16(
            "LiveSplit Server port",
            d.and_then(|c| c.livesplit_port).or(Some(16834)),
        )
    } else {
        d.and_then(|c| c.livesplit_port)
    };
    let livesplit_split_on_badges = prompt_bool(
        "Split on badge earned?",
        d.map(|c| c.livesplit_split_on_badges).unwrap_or(false),
    );
    let livesplit_split_on_clear = prompt_bool(
        "Split on Champion defeated?",
        d.map(|c| c.livesplit_split_on_clear).unwrap_or(true),
    );

    // ── Discord Rich Presence ───────────────────────────────────────────────

    section("Discord Rich Presence");

    let discord_client_id = prompt_opt_u64(
        "Discord application client ID",
        d.and_then(|c| c.discord_client_id),
    );

    // ── Assemble config ─────────────────────────────────────────────────────

    let cfg = TrackerConfig {
        rom,
        db: db_full(&db_raw),
        clean,
        mode,
        aggregator_host,
        aggregator_port,
        preferred_player,
        default_test,
        test,
        poll_ms,
        webhooks: WebhookConfig {
            death_url,
            death_template,
            catch_url,
            catch_template,
            shiny_url,
            shiny_template,
            wipe_url,
            wipe_template,
            notify_on_death,
            notify_on_shiny,
            notify_on_wipe,
            // Preserve fields not exposed in this editor.
            badge_url: wh_prev.and_then(|w| w.badge_url.clone()),
            badge_template: wh_prev.and_then(|w| w.badge_template.clone()),
            nickname_url: wh_prev.and_then(|w| w.nickname_url.clone()),
            nickname_template: wh_prev.and_then(|w| w.nickname_template.clone()),
            nuzlocke_url: wh_prev.and_then(|w| w.nuzlocke_url.clone()),
            nuzlocke_template: wh_prev.and_then(|w| w.nuzlocke_template.clone()),
            discord_webhook_url: wh_prev.and_then(|w| w.discord_webhook_url.clone()),
            hmac_secret: wh_prev.and_then(|w| w.hmac_secret.clone()),
        },
        obs: ObsConfig {
            host: obs_host,
            port: obs_port,
            password: obs_password,
            clip_on_death: obs_clip_death,
            clip_on_shiny: obs_clip_shiny,
            clip_on_wipe: obs_clip_wipe,
            clip_on_badge: obs_clip_badge,
            // Preserve fields not exposed in this editor.
            scene_on_death: obs_prev.and_then(|o| o.scene_on_death.clone()),
            scene_on_wipe: obs_prev.and_then(|o| o.scene_on_wipe.clone()),
            scene_on_shiny: obs_prev.and_then(|o| o.scene_on_shiny.clone()),
            scene_on_badge: obs_prev.and_then(|o| o.scene_on_badge.clone()),
            scene_on_catch: obs_prev.and_then(|o| o.scene_on_catch.clone()),
        },
        dupes_clause,
        allow_species_repeats,
        preset: None,
        run_start_balls,
        livesplit_host,
        livesplit_port,
        livesplit_split_on_badges,
        livesplit_split_on_clear,
        discord_client_id,
        twitch_helix: d.and_then(|c| c.twitch_helix.clone()),
        username,
    };

    // ── Validate ─────────────────────────────────────────────────────────────

    let errors = crate::config::validate_config(&cfg);
    if !errors.is_empty() {
        println!();
        println!("  ⚠  Validation warnings:");
        for e in &errors {
            println!("      - {e}");
        }
    }

    // ── Confirm & save ────────────────────────────────────────────────────────

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
