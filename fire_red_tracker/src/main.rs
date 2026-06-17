//! # FireRed Tracker
//!
//! A real-time Pokémon FireRed party and encounter monitor.
//!
//! # Modes
//!
//! - **Standalone** — reads the ROM and game memory locally, renders the GUI.
//! - **Connected** — like standalone but connects to an aggregator and streams
//!   game state to it. Runs headless (no local GUI window).
//!
//! # Configuration
//!
//! Settings are stored in `~/.config/fire_red_tracker/config.toml`. On first
//! launch the tracker prompts for each value and writes the file. Any value can
//! be overridden for a single run via CLI flags:
//!
//! ```text
//! tracker                                        # use config defaults
//! tracker [ROM]                                  # override ROM path
//! tracker --clean                                # enable ability names
//! tracker --db <CONN>                            # override database
//! tracker connect [--host <HOST>] [--port <N>]   # force connected mode
//! tracker --new-run                              # start a new nuzlocke run
//! tracker --list-runs                            # print stored runs and exit
//! tracker --config <FILE>                        # use an alternate config file
//! ```
//!
//! # Module layout
//!
//! | Module        | Responsibility                                         |
//! |---------------|--------------------------------------------------------|
//! | [`cli`]       | clap CLI struct and subcommand definitions             |
//! | [`config`]    | Config file loading, saving, and first-run prompts     |
//! | [`encounter`] | `EncounterTracker` — wild battle and catch detection   |
//! | [`game`]      | EWRAM/IWRAM helpers, `is_shiny`, `game_is_loaded`      |
//! | [`textures`]  | Sprite loading, compression, `PendingTexture`          |
//! | [`gui`]       | `WindowInfo`, egui rendering, party/encounter panels   |
//! | [`server`]    | Aggregator connection handler                          |

mod cli;
mod config;
mod discord;
mod encounter;
mod game;
mod gui;
mod helix;
mod livesplit;
mod server;
mod textures;
mod type_coverage;
mod webhook;

use clap::Parser;
use cli::{Cli, Command};
use colored::Colorize;

use fire_red_loop::*;
use fire_red_states::*;
use game::{
    check_for_dead_pokemon, check_for_new_pokemon, check_for_new_trainer_battles,
    check_for_run_over, fill_party_list, game_is_loaded, is_shiny, map_state_from_ewram,
};
#[cfg(feature = "dev-tools")]
use game::{scan_for_balls_pocket, scan_for_security_key};
use gui::{PARTY_WINDOW, WindowInfo};
use server::{RomSpriteCache, handle_client};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How often (in seconds) the DB checks for deaths/new-pokemon are run when
/// the party size has not changed, to catch in-place changes such as HP loss.
const FORCE_PARTY_CHECK_INTERVAL: u64 = 1;

// ---------------------------------------------------------------------------
// Party-event helper
// ---------------------------------------------------------------------------

/// Checks for new catches, deaths, and run-over in one place.
///
/// Called after every party refresh — both on party-size changes and on the
/// periodic 1-second force-check. Returns `true` if a wipe was detected so
/// the caller can stop the encounter tracker.
fn handle_party_events(
    thread_party: &Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    enc_tracker: &mut encounter::EncounterTracker,
    thread_wipe_signal: &Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    check_for_new_pokemon(thread_party);
    check_for_dead_pokemon(thread_party, enc_tracker.run_tracking_active());
    if check_for_run_over(thread_party, enc_tracker.run_tracking_active()) {
        enc_tracker.mark_wipe();
        thread_wipe_signal.store(true, std::sync::atomic::Ordering::Release);
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Box data helpers
// ---------------------------------------------------------------------------

/// Reads all occupied PC box slots from the EWRAM snapshot and converts them
/// to [`BoxEntry`] values suitable for network transmission.
fn build_box_entries() -> Vec<BoxEntry> {
    fire_red_box_monitor::get_box_entries_positioned()
        .into_iter()
        .map(|(box_idx, slot_idx, mon)| {
            let personality = mon.personality;
            let ot_id = mon.ot_id;
            let iv = &mon.secure.misc.iv_egg_ability;
            BoxEntry {
                box_index: box_idx,
                slot_index: slot_idx,
                species: mon.secure.growth.species,
                species_name: mon.secure.growth.species_string.clone(),
                nickname: mon.nickname_string.clone(),
                personality,
                ot_id,
                is_shiny: is_shiny(personality, ot_id),
                nature: fire_red_database::nature_name(personality).to_string(),
                iv_hp: iv.hp_iv,
                iv_atk: iv.attack_iv,
                iv_def: iv.defense_iv,
                iv_spe: iv.speed_iv,
                iv_spa: iv.sp_attack_iv,
                iv_spd: iv.sp_def_iv,
                is_egg: iv.egg != 0,
                gender: mon.gender,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Statics
// ---------------------------------------------------------------------------

static MAIN_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static NET_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Set to `false` by the Ctrl-C handler to trigger a clean shutdown.
static RUNNING: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn do_update() {
    println!(
        "Checking for updates (current version: v{})...",
        env!("CARGO_PKG_VERSION")
    );
    let result = self_update::backends::github::Update::configure()
        .repo_owner("AliceWreath")
        .repo_name("fire_red_tracker_project")
        .bin_name("fire_red_tracker")
        .identifier("fire_red_tracker")
        .show_download_progress(true)
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()
        .and_then(|u| u.update());

    match result {
        Ok(self_update::Status::UpToDate(v)) => {
            println!("Already up to date (v{}).", v);
        }
        Ok(self_update::Status::Updated(v)) => {
            println!(
                "Updated to v{}. Restart the tracker to use the new version.",
                v
            );
        }
        Err(e) => {
            tracing::error!("Update failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.update {
        do_update();
        return;
    }

    tracing::info!("FireRed Tracker v{}", env!("CARGO_PKG_VERSION"));

    // Load config (prompts on first run), then overlay any CLI overrides.
    let config_path = cli
        .config
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_config_path);

    if cli.config_editor {
        config::run_config_editor(&config_path);
        return;
    }

    let cfg = config::load_or_prompt(&config_path);
    let cfg_gui = cfg.clone();
    let config_path_gui = config_path.clone();

    // Validate config early so misconfigurations surface before any threads start.
    let validation_errors = config::validate_config(&cfg);
    if !validation_errors.is_empty() {
        tracing::error!("Config validation failed:");
        for e in &validation_errors {
            tracing::error!("  - {e}");
        }
        std::process::exit(1);
    }

    // test section: applied on top of base config, below explicit CLI flags.
    let use_test = cli.test || cfg.default_test;
    let test_ov = if use_test { cfg.test.clone() } else { None };
    let test = test_ov.as_ref();
    if use_test {
        if cli.run_id.is_some() {
            tracing::error!(
                "--run-id and test mode are mutually exclusive \
                       (test mode always starts a new run; drop --run-id or disable test mode)."
            );
            std::process::exit(1);
        }
        println!("Test mode active — using [test] config overrides and starting a new run.");
    }

    // Mode: CLI subcommand wins; otherwise use what the config says (with test overrides).
    let mode = match cli.command {
        Some(Command::Connect { host, port }) => Mode::Connected { host, port },
        None => match cfg.mode {
            config::ConfigMode::Standalone => Mode::Standalone,
            config::ConfigMode::Connected => Mode::Connected {
                host: test
                    .and_then(|t| t.aggregator_host.clone())
                    .unwrap_or_else(|| cfg.aggregator_host.clone()),
                port: test
                    .and_then(|t| t.aggregator_port)
                    .unwrap_or(cfg.aggregator_port),
            },
        },
    };

    // Priority: base config → [test] overrides → explicit CLI flags.
    let db_conn = cli
        .db
        .or_else(|| test.and_then(|t| t.db.clone()))
        .unwrap_or(cfg.db);
    if let Err(e) = fire_red_database::initialize(&db_conn) {
        tracing::error!("{e}");
        std::process::exit(1);
    }

    // --list-runs: print stored runs and exit without starting the tracker.
    if cli.list_runs {
        let runs = fire_red_database::list_runs().unwrap_or_else(|e| {
            tracing::error!("{e}");
            std::process::exit(1);
        });
        if runs.is_empty() {
            println!("No runs found.");
        } else {
            let active = fire_red_database::active_run_id();
            println!("{:<5} {:<12} {:<26} Deaths", "ID", "Player", "Started");
            println!("{}", "-".repeat(60));
            for (id, name, started_at, dead_count) in &runs {
                let marker = if active == Some(*id) { " <active>" } else { "" };
                println!(
                    "{:<5} {:<12} {:<26} {}{}",
                    id,
                    name,
                    fire_red_database::format_timestamp(*started_at),
                    dead_count,
                    marker,
                );
            }
        }
        return;
    }

    // Initialize the nuzlocke run before the game thread starts so that
    // is_dead() / mark_dead() always have a valid active run ID.
    // test mode implies --new-run so test sessions never pollute production history.
    let new_run = cli.new_run || use_test;
    match (cli.run_id, new_run) {
        (Some(id), _) => match fire_red_database::resume_run(id) {
            Ok(true) => println!("Resuming run #{}.", id),
            Ok(false) => {
                tracing::error!(
                    "run #{} not found. Use --list-runs to see available runs.",
                    id
                );
                std::process::exit(1);
            }
            Err(e) => {
                tracing::error!("{e}");
                std::process::exit(1);
            }
        },
        (None, true) => {
            let id = fire_red_database::new_run("Unknown").unwrap_or_else(|e| {
                tracing::error!("{e}");
                std::process::exit(1);
            });
            println!("Started new run #{}.", id);
        }
        (None, false) => {
            let id = fire_red_database::get_or_create_run("Unknown").unwrap_or_else(|e| {
                tracing::error!("{e}");
                std::process::exit(1);
            });
            println!("Using run #{}.", id);
        }
    }

    webhook::init(cfg.webhooks.clone(), cfg.obs.clone());
    livesplit::init(
        cfg.livesplit_host.clone(),
        cfg.livesplit_port.unwrap_or(16834),
    );
    discord::init(cfg.discord_client_id);
    if let Some(helix_cfg) = cfg.twitch_helix.clone() {
        helix::init(helix_cfg);
    }

    let is_clean = cfg.clean || cli.clean;
    let poll_ms = Arc::new(AtomicU64::new(cfg.poll_ms.clamp(20, 2000)));
    let rom_path = cli.rom.unwrap_or(cfg.rom);
    let dupes_clause = cfg.dupes_clause;
    let allow_species_repeats = cfg.allow_species_repeats;
    let run_start_balls = cfg.run_start_balls.unwrap_or(5) as u32;
    let livesplit_split_on_badges = cfg.livesplit_split_on_badges;
    let livesplit_split_on_clear = cfg.livesplit_split_on_clear;
    #[cfg(feature = "dev-tools")]
    let do_scan_balls = cli.scan_balls_pocket;
    #[cfg(feature = "dev-tools")]
    let do_scan_sec_key = cli.scan_security_key;
    let preferred_player = cli
        .preferred_player
        .or_else(|| test.and_then(|t| t.preferred_player))
        .or(cfg.preferred_player);

    let game_loaded: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let run_changed: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let wipe_signal: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    let shared_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>> =
        Arc::new(Mutex::new(Vec::new()));
    let shared_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>> = Arc::new(
        Mutex::new(fire_red_pokemon_data::WildPokemonHeader::default()),
    );
    let shared_box: Arc<Mutex<Vec<BoxEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let shared_bag: Arc<Mutex<Option<fire_red_states::BagPockets>>> = Arc::new(Mutex::new(None));
    let shared_warnings: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sprite_cache: RomSpriteCache = Arc::new(Mutex::new(HashMap::new()));

    // ── Game-polling thread (both modes) ──────────────────────────────────────
    {
        let thread_party = shared_party.clone();
        let thread_encounters = shared_encounters.clone();
        let thread_box = shared_box.clone();
        let thread_bag = shared_bag.clone();
        let thread_game_loaded = game_loaded.clone();
        let thread_run_changed = run_changed.clone();
        let thread_wipe_signal = wipe_signal.clone();
        let thread_warnings = shared_warnings.clone();
        let thread_poll_ms = poll_ms.clone();

        let main_thread = std::thread::spawn(move || {
            match start_loop(rom_path.as_str(), is_clean) {
                0 => tracing::info!("Monitor loop started."),
                code => {
                    tracing::error!("Failed to start monitor loop (code {}).", code);
                    std::process::exit(1);
                }
            }

            tracing::info!("Waiting for initial map state...");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if map_state_from_ewram().is_some() {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    tracing::warn!("Map state did not populate within 5 seconds.");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            #[cfg(feature = "dev-tools")]
            if do_scan_balls {
                scan_for_balls_pocket();
                std::process::exit(0);
            }

            #[cfg(feature = "dev-tools")]
            if let Some(qty) = do_scan_sec_key {
                scan_for_security_key(qty);
                std::process::exit(0);
            }

            let initial_state = map_state_from_ewram().unwrap_or(FireRedState {
                map_group_id: 0,
                map_name_id: 0,
            });

            *thread_encounters.lock_or_recover() = get_area_pokemon_id_for_state(&initial_state);

            let mut current_state = initial_state;
            let mut old_party_size = get_party_size();
            let mut last_party_refresh = std::time::Instant::now();
            let mut last_bag_refresh = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now);
            let mut state_initialized = false;
            let mut enc_tracker = encounter::EncounterTracker::new();
            // None = "uninitialized"; check_for_new_badges adopts existing
            // badges silently on the first call with this sentinel value.
            let mut last_badge_mask: Option<u8> = None;
            // None = "uninitialized"; check_for_new_trainer_battles adopts
            // existing flags silently on the first call.
            let mut last_trainer_flags: Option<Vec<u8>> = None;
            // Track the last player name to detect save-file switches mid-session.
            let mut last_player_name = String::new();
            // HP history tracking: remember each party mon's last known HP so we
            // only write to the DB when it actually changes.
            let mut last_party_hp: HashMap<u32, u16> = HashMap::new();
            // Enemy HP tracking: record initial and final HP for each encounter.
            // gEnemyParty[0] is never cleared between battles; a personality
            // change signals a new battle rather than presence/absence.
            // `enemy_warmed_up` starts false so we skip the stale value that
            // may be in EWRAM at startup (from a previous battle before launch).
            let mut last_enemy_personality: u32 = 0;
            let mut last_enemy_hp: u16 = 0;
            let mut last_enemy_max_hp: u16 = 0;
            let mut enemy_warmed_up = false;
            // Area visit tracking: record the DB row id of the currently-open
            // visit so it can be closed when the player changes maps.
            let mut last_area_visit_id: Option<i64> = None;

            // Set player name before the startup party scan so records written
            // to caught_pokemon have the correct player attribution. The trainer
            // data has been initialised from EWRAM at this point, so this
            // succeeds for any loaded save. For a brand-new game the name may
            // still be blank; the main loop will set it on the first iteration
            // where the trainer name becomes available.
            let mut player_name_set = {
                let name = get_trainer_name();
                if !name.trim().is_empty() {
                    fire_red_database::set_player_name(&name);
                    true
                } else {
                    false
                }
            };

            // Seed the enc_tracker latch from the database. If encounters
            // already exist for this run the player had balls at some point,
            // so deaths should be recorded immediately on resume without
            // waiting for a new wild encounter. DB-based seeding avoids the
            // EWRAM false-positives that has_pokeballs() can produce at
            // startup from stale memory.
            enc_tracker.seed_from_db();

            fill_party_list(&thread_party);
            if handle_party_events(&thread_party, &mut enc_tracker, &thread_wipe_signal) {
                last_badge_mask = None;
            }

            loop {
                if !game_is_loaded() {
                    thread_game_loaded.store(false, Ordering::Release);
                    *thread_encounters.lock_or_recover() =
                        fire_red_pokemon_data::WildPokemonHeader::default();
                    *thread_party.lock_or_recover() = Vec::new();
                    state_initialized = false;
                    player_name_set = false;
                    last_badge_mask = None;
                    last_trainer_flags = None;
                    current_state = FireRedState {
                        map_group_id: 0xFF,
                        map_name_id: 0xFF,
                    };
                    enc_tracker.reset();
                    last_party_hp.clear();
                    last_enemy_personality = 0;
                    enemy_warmed_up = false;
                    if let Some(vid) = last_area_visit_id.take() {
                        fire_red_database::close_area_visit(vid, fire_red_database::unix_now());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
                thread_game_loaded.store(true, Ordering::Release);

                if !player_name_set {
                    let name = get_trainer_name();
                    if !name.trim().is_empty() {
                        // Warn if the player name changed since the last time the
                        // game was loaded — this typically means the user soft-reset
                        // into a different save file during the same tracker session.
                        if !last_player_name.is_empty() && name != last_player_name {
                            tracing::warn!(
                                "player name changed from '{}' to '{}' after reload — \
                                 possible save-file switch. Death/encounter records may now \
                                 belong to a different run.",
                                last_player_name,
                                name
                            );
                        }
                        last_player_name = name.clone();
                        fire_red_database::set_player_name(&name);
                        player_name_set = true;
                        // Re-seed the encounter latch so that if the tracker was
                        // started (or restarted) while the player already had balls
                        // and encounters in the DB, deaths are recorded immediately
                        // without waiting for the next wild encounter.
                        enc_tracker.seed_from_db();
                    }
                }

                let state = map_state_from_ewram().unwrap_or(current_state);
                let party_size = get_party_size();

                if !state_initialized && (state.map_group_id != 0 || state.map_name_id != 0) {
                    state_initialized = true;
                    current_state = state;
                    *thread_encounters.lock_or_recover() =
                        get_area_pokemon_id_for_state(&current_state);
                    // Open the initial area visit.
                    let zone = fire_red_loop::get_area_name_for(
                        current_state.map_group_id,
                        current_state.map_name_id,
                    );
                    last_area_visit_id = fire_red_database::open_area_visit(
                        current_state.map_group_id,
                        current_state.map_name_id,
                        zone,
                        fire_red_database::unix_now(),
                    );
                }

                if state_initialized && current_state != state {
                    current_state = state;
                    *thread_encounters.lock_or_recover() =
                        get_area_pokemon_id_for_state(&current_state);
                    let zone = fire_red_loop::get_area_name_for(
                        current_state.map_group_id,
                        current_state.map_name_id,
                    );
                    let zone_str = if zone.is_empty() {
                        format!(
                            "{}\u{B7}{}",
                            current_state.map_group_id, current_state.map_name_id
                        )
                    } else {
                        zone.to_string()
                    };
                    // Close the previous area visit and open a new one.
                    let now = fire_red_database::unix_now();
                    if let Some(vid) = last_area_visit_id.take() {
                        fire_red_database::close_area_visit(vid, now);
                    }
                    last_area_visit_id = fire_red_database::open_area_visit(
                        current_state.map_group_id,
                        current_state.map_name_id,
                        zone,
                        now,
                    );
                    discord::update(discord::Presence {
                        details: zone_str,
                        state: format!("Party: {}", party_size),
                        large_image: "pokeball",
                        large_text: fire_red_loop::get_trainer_name(),
                    });
                }

                // Refresh HP/status from the EWRAM buffer every tick so the
                // aggregator always sees current values without waiting for a
                // size change or the periodic DB-check interval.
                fill_party_list(&thread_party);

                // Track the lowest HP ratio seen per Pokémon for closest-call analytics.
                // Also log every HP change for the full per-Pokémon HP history.
                for mon in thread_party.lock_or_recover().iter() {
                    let personality = mon.box_mon.personality;
                    let hp = mon.hp;
                    let max_hp = mon.max_hp;
                    if personality != 0 && hp > 0 && max_hp > 0 {
                        fire_red_database::update_min_hp_seen(personality, hp, max_hp);
                        let changed = last_party_hp.get(&personality).is_none_or(|&last| last != hp);
                        if changed {
                            fire_red_database::record_hp_observation(personality, hp, max_hp);
                            last_party_hp.insert(personality, hp);
                        }
                    }
                }

                // Enemy HP tracking: log initial HP at battle start and final HP
                // when a new personality is detected (previous battle ended).
                if let Some((enemy_p, enemy_hp, enemy_max_hp)) =
                    crate::game::read_enemy_slot0_raw()
                {
                    if !enemy_warmed_up {
                        // Discard the first personality seen after (re)load; it may
                        // be stale from a battle before the tracker started.
                        last_enemy_personality = enemy_p;
                        enemy_warmed_up = true;
                    } else if enemy_p != last_enemy_personality {
                        // Personality changed → previous battle ended, new one began.
                        if last_enemy_personality != 0 {
                            fire_red_database::record_enemy_hp(
                                last_enemy_personality,
                                last_enemy_hp,
                                last_enemy_max_hp,
                                "final",
                            );
                        }
                        fire_red_database::record_enemy_hp(enemy_p, enemy_hp, enemy_max_hp, "initial");
                        last_enemy_personality = enemy_p;
                    }
                    last_enemy_hp = enemy_hp;
                    last_enemy_max_hp = enemy_max_hp;
                }

                if old_party_size != party_size {
                    old_party_size = party_size;
                    update_box_list();
                    *thread_box.lock_or_recover() = build_box_entries();
                    if handle_party_events(&thread_party, &mut enc_tracker, &thread_wipe_signal) {
                        last_badge_mask = None;
                    }
                    // Presence update: party size changed (catch, death, trade, etc).
                    if state_initialized {
                        let zone = fire_red_loop::get_area_name_for(
                            current_state.map_group_id,
                            current_state.map_name_id,
                        );
                        let zone_str = if zone.is_empty() {
                            format!(
                                "{}\u{B7}{}",
                                current_state.map_group_id, current_state.map_name_id
                            )
                        } else {
                            zone.to_string()
                        };
                        discord::update(discord::Presence {
                            details: zone_str,
                            state: format!("Party: {}", party_size),
                            large_image: "pokeball",
                            large_text: fire_red_loop::get_trainer_name(),
                        });
                    }
                }

                if last_party_refresh.elapsed().as_secs() >= FORCE_PARTY_CHECK_INTERVAL {
                    last_party_refresh = std::time::Instant::now();
                    if handle_party_events(&thread_party, &mut enc_tracker, &thread_wipe_signal) {
                        last_badge_mask = None;
                    }
                }

                if thread_run_changed.swap(false, Ordering::AcqRel) {
                    enc_tracker.reset();
                    last_badge_mask = None;
                    last_trainer_flags = None;
                    player_name_set = false;
                    if let Some(vid) = last_area_visit_id.take() {
                        fire_red_database::close_area_visit(vid, fire_red_database::unix_now());
                    }
                }

                if state_initialized {
                    enc_tracker.tick(
                        current_state,
                        &thread_party,
                        dupes_clause,
                        allow_species_repeats,
                        run_start_balls,
                    );
                    let drained = enc_tracker.drain_warnings();
                    if !drained.is_empty() {
                        thread_warnings.lock_or_recover().extend(drained);
                    }
                    last_badge_mask = game::check_for_new_badges(
                        last_badge_mask,
                        livesplit_split_on_badges,
                        livesplit_split_on_clear,
                        &thread_party,
                    );
                    last_trainer_flags = check_for_new_trainer_battles(last_trainer_flags);
                }

                if last_bag_refresh.elapsed() >= std::time::Duration::from_secs(2) {
                    *thread_bag.lock_or_recover() = game::read_bag_pockets();
                    last_bag_refresh = std::time::Instant::now();
                }

                std::thread::sleep(std::time::Duration::from_millis(
                    thread_poll_ms.load(Ordering::Relaxed),
                ));
            }
        });

        *MAIN_THREAD_HANDLE.lock_or_recover() = Some(main_thread);
    }

    // ── Config hot-reload thread ──────────────────────────────────────────────
    {
        let reload_path = config_path.clone();
        let reload_ms = poll_ms.clone();
        std::thread::spawn(move || {
            let mut last_mtime = std::fs::metadata(&reload_path)
                .and_then(|m| m.modified())
                .ok();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let current_mtime = std::fs::metadata(&reload_path)
                    .and_then(|m| m.modified())
                    .ok();
                if current_mtime == last_mtime || current_mtime.is_none() {
                    continue;
                }
                last_mtime = current_mtime;
                match config::try_load_config(&reload_path) {
                    Some(new_cfg) => {
                        let errors = config::validate_config(&new_cfg);
                        if !errors.is_empty() {
                            for e in &errors {
                                tracing::warn!("Hot-reload config error: {e}");
                            }
                            continue;
                        }
                        webhook::reinit(new_cfg.webhooks.clone(), new_cfg.obs.clone());
                        reload_ms.store(new_cfg.poll_ms.clamp(20, 2000), Ordering::Relaxed);
                        tracing::info!("Config hot-reloaded from {}", reload_path.display());
                    }
                    None => tracing::warn!("Hot-reload: failed to parse {}", reload_path.display()),
                }
            }
        });
    }

    // ── Connected mode: dial out to the aggregator ────────────────────────────
    if let Mode::Connected { host, port } = &mode {
        let addr = format!("{}:{}", host, port);
        let net_party = shared_party.clone();
        let net_encounters = shared_encounters.clone();
        let net_box = shared_box.clone();
        let net_bag = shared_bag.clone();
        let net_cache = sprite_cache.clone();
        let net_loaded = game_loaded.clone();
        let net_run_changed = run_changed.clone();
        let net_wipe_signal = wipe_signal.clone();
        let net_warnings = shared_warnings.clone();

        let net_thread = std::thread::spawn(move || {
            let mut delay_secs: u64 = 5;
            loop {
                tracing::info!("Connecting to aggregator at {}...", addr);
                match TcpStream::connect(&addr) {
                    Ok(stream) => {
                        tracing::info!("Connected to aggregator.");
                        delay_secs = 5;
                        handle_client(
                            stream,
                            net_party.clone(),
                            net_encounters.clone(),
                            net_box.clone(),
                            net_bag.clone(),
                            net_cache.clone(),
                            net_loaded.clone(),
                            net_run_changed.clone(),
                            net_wipe_signal.clone(),
                            preferred_player,
                            net_warnings.clone(),
                        );
                        tracing::info!("Disconnected from aggregator.");
                    }
                    Err(e) => tracing::warn!("Failed to connect to aggregator: {e}"),
                }
                std::thread::sleep(std::time::Duration::from_secs(delay_secs));
                delay_secs = (delay_secs * 2).min(60);
            }
        });

        *NET_THREAD_HANDLE.lock_or_recover() = Some(net_thread);

        println!(
            "{}",
            "***** Connected mode — no GUI. Press Ctrl-C to exit. *****"
                .green()
                .bold()
        );
        ctrlc::set_handler(|| {
            RUNNING.store(false, Ordering::Release);
            println!("\nShutting down...");
            std::process::exit(0);
        })
        .expect("Error setting Ctrl-C handler.");
        while RUNNING.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        return;
    }

    // ── Standalone: show local GUI ────────────────────────────────────────────
    let update_available: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    {
        let flag = update_available.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let result = self_update::backends::github::Update::configure()
                .repo_owner("AliceWreath")
                .repo_name("fire_red_tracker_project")
                .bin_name("fire_red_tracker")
                .identifier("fire_red_tracker")
                .current_version(env!("CARGO_PKG_VERSION"))
                .build()
                .and_then(|u| u.get_latest_release());
            if let Ok(release) = result {
                let latest = release.version.trim_start_matches('v');
                if latest != env!("CARGO_PKG_VERSION") {
                    *flag.lock_or_recover() = Some(release.version.clone());
                }
            }
        });
    }

    let _ = eframe::run_native(
        "Tracker",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([PARTY_WINDOW.0, PARTY_WINDOW.1]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(WindowInfo::new(
                cc,
                shared_party,
                shared_encounters,
                Arc::new(Mutex::new(Vec::new())),
                Arc::new(Mutex::new(std::collections::HashSet::new())),
                None,
                config_path_gui,
                &cfg_gui,
                update_available,
            )))
        }),
    );
}
