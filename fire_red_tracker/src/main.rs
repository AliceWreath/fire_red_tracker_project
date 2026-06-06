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
mod encounter;
mod game;
mod gui;
mod server;
mod textures;
mod webhook;

use clap::Parser;
use cli::{Cli, Command};
use colored::Colorize;
use fire_red_loop::*;
use fire_red_states::*;
use game::{check_for_dead_pokemon, check_for_new_pokemon, check_for_run_over, fill_party_list, game_is_loaded, is_shiny, map_state_from_ewram, scan_for_balls_pocket, scan_for_security_key};
use gui::{WindowInfo, PARTY_WINDOW};
use server::handle_client;
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for Mutex<T> {
    #[track_caller]
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            let loc = std::panic::Location::caller();
            eprintln!("Warning: mutex poisoned at {}:{}: {e}", loc.file(), loc.line());
            e.into_inner()
        })
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How often (in seconds) the DB checks for deaths/new-pokemon are run when
/// the party size has not changed, to catch in-place changes such as HP loss.
const FORCE_PARTY_CHECK_INTERVAL: u64 = 1;

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
            let ot_id       = mon.ot_id;
            let iv          = &mon.secure.misc.iv_egg_ability;
            BoxEntry {
                box_index:    box_idx,
                slot_index:   slot_idx,
                species:      mon.secure.growth.species,
                species_name: mon.secure.growth.species_string.clone(),
                nickname:     mon.nickname_string.clone(),
                personality,
                ot_id,
                is_shiny:     is_shiny(personality, ot_id),
                nature:       fire_red_database::nature_name(personality).to_string(),
                iv_hp:        iv.hp_iv,
                iv_atk:       iv.attack_iv,
                iv_def:       iv.defense_iv,
                iv_spe:       iv.speed_iv,
                iv_spa:       iv.sp_attack_iv,
                iv_spd:       iv.sp_def_iv,
                is_egg:       iv.egg != 0,
                gender:       mon.gender,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Statics
// ---------------------------------------------------------------------------

static MAIN_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static NET_THREAD_HANDLE:  Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

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
            println!("Updated to v{}. Restart the tracker to use the new version.", v);
        }
        Err(e) => {
            eprintln!("Update failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn main() {
    let cli = Cli::parse();

    if cli.update {
        do_update();
        return;
    }

    println!("FireRed Tracker v{}", env!("CARGO_PKG_VERSION"));

    // Load config (prompts on first run), then overlay any CLI overrides.
    let config_path = cli.config.as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_config_path);

    if cli.config_editor {
        config::run_config_editor(&config_path);
        return;
    }

    let cfg             = config::load_or_prompt(&config_path);
    let cfg_gui         = cfg.clone();
    let config_path_gui = config_path.clone();

    // test section: applied on top of base config, below explicit CLI flags.
    let use_test = cli.test || cfg.default_test;
    let test_ov  = if use_test { cfg.test.clone() } else { None };
    let test     = test_ov.as_ref();
    if use_test {
        println!("Test mode active — using [test] config overrides and starting a new run.");
    }

    // Mode: CLI subcommand wins; otherwise use what the config says (with test overrides).
    let mode = match cli.command {
        Some(Command::Connect { host, port }) => Mode::Connected { host, port },
        None => match cfg.mode {
            config::ConfigMode::Standalone => Mode::Standalone,
            config::ConfigMode::Connected  => Mode::Connected {
                host: test.and_then(|t| t.aggregator_host.clone())
                    .unwrap_or_else(|| cfg.aggregator_host.clone()),
                port: test.and_then(|t| t.aggregator_port)
                    .unwrap_or(cfg.aggregator_port),
            },
        },
    };

    // Priority: base config → [test] overrides → explicit CLI flags.
    let db_raw = cli.db
        .or_else(|| test.and_then(|t| t.db.clone()))
        .unwrap_or(cfg.db);
    let db_conn = if db_raw.starts_with("postgresql://") || db_raw.starts_with("postgres://") {
        db_raw
    } else {
        format!("postgresql://{}", db_raw)
    };
    if let Err(e) = fire_red_database::initialize(&db_conn) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }

    // --list-runs: print stored runs and exit without starting the tracker.
    if cli.list_runs {
        let runs = fire_red_database::list_runs().unwrap_or_else(|e| {
            eprintln!("error: {e}");
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
                    id, name,
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
        (Some(id), _) => {
            match fire_red_database::resume_run(id) {
                Ok(true)  => println!("Resuming run #{}.", id),
                Ok(false) => {
                    eprintln!("Error: run #{} not found. Use --list-runs to see available runs.", id);
                    std::process::exit(1);
                }
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        (None, true) => {
            let id = fire_red_database::new_run("Unknown").unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1);
            });
            println!("Started new run #{}.", id);
        }
        (None, false) => {
            let id = fire_red_database::get_or_create_run("Unknown").unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1);
            });
            println!("Using run #{}.", id);
        }
    }

    webhook::init(cfg.webhooks.clone());

    let is_clean            = cfg.clean || cli.clean;
    let rom_path            = cli.rom.unwrap_or(cfg.rom);
    let do_scan_balls       = cli.scan_balls_pocket;
    let do_scan_sec_key     = cli.scan_security_key;
    let preferred_player = cli.preferred_player
        .or_else(|| test.and_then(|t| t.preferred_player))
        .or(cfg.preferred_player);

    let game_loaded:  Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let run_changed:  Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let wipe_signal:  Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    let shared_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>> =
        Arc::new(Mutex::new(Vec::new()));
    let shared_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>> =
        Arc::new(Mutex::new(fire_red_pokemon_data::WildPokemonHeader::default()));
    let shared_box: Arc<Mutex<Vec<BoxEntry>>> =
        Arc::new(Mutex::new(Vec::new()));
    let sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // ── Game-polling thread (both modes) ──────────────────────────────────────
    {
        let thread_party       = shared_party.clone();
        let thread_encounters  = shared_encounters.clone();
        let thread_box         = shared_box.clone();
        let thread_game_loaded = game_loaded.clone();
        let thread_run_changed = run_changed.clone();
        let thread_wipe_signal = wipe_signal.clone();

        let main_thread = std::thread::spawn(move || {
            match start_loop(rom_path.as_str(), is_clean) {
                0    => println!("Monitor loop started."),
                code => {
                    eprintln!("Failed to start monitor loop (code {}).", code);
                    std::process::exit(1);
                }
            }

            println!("Waiting for initial map state...");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if map_state_from_ewram().is_some() { break; }
                if std::time::Instant::now() > deadline {
                    eprintln!("Warning: map state did not populate within 5 seconds.");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            if do_scan_balls {
                scan_for_balls_pocket();
                std::process::exit(0);
            }

            if let Some(qty) = do_scan_sec_key {
                scan_for_security_key(qty);
                std::process::exit(0);
            }

            let initial_state = map_state_from_ewram()
                .unwrap_or(FireRedState { map_group_id: 0, map_name_id: 0 });

            *thread_encounters.lock_or_recover() =
                get_area_pokemon_id_for_state(&initial_state);

            let mut current_state      = initial_state;
            let mut old_party_size     = get_party_size();
            let mut last_party_refresh = std::time::Instant::now();
            let mut state_initialized  = false;
            let mut enc_tracker        = encounter::EncounterTracker::new();

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
            check_for_new_pokemon(&thread_party);
            check_for_dead_pokemon(&thread_party, enc_tracker.run_tracking_active());
            if check_for_run_over(&thread_party, enc_tracker.run_tracking_active()) {
                enc_tracker.mark_wipe();
                thread_wipe_signal.store(true, Ordering::Release);
            }

            loop {
                if !game_is_loaded() {
                    thread_game_loaded.store(false, Ordering::Release);
                    *thread_encounters.lock_or_recover() =
                        fire_red_pokemon_data::WildPokemonHeader::default();
                    *thread_party.lock_or_recover() = Vec::new();
                    state_initialized = false;
                    player_name_set   = false;
                    current_state = FireRedState { map_group_id: 0xFF, map_name_id: 0xFF };
                    enc_tracker.reset();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }
                thread_game_loaded.store(true, Ordering::Release);

                if !player_name_set {
                    let name = get_trainer_name();
                    if !name.trim().is_empty() {
                        fire_red_database::set_player_name(&name);
                        player_name_set = true;
                    }
                }

                let state      = map_state_from_ewram().unwrap_or(current_state);
                let party_size = get_party_size();

                if !state_initialized
                    && (state.map_group_id != 0 || state.map_name_id != 0)
                {
                    state_initialized = true;
                    current_state = state;
                    *thread_encounters.lock_or_recover() =
                        get_area_pokemon_id_for_state(&current_state);
                }

                if state_initialized && current_state != state {
                    current_state = state;
                    *thread_encounters.lock_or_recover() =
                        get_area_pokemon_id_for_state(&current_state);
                }

                // Refresh HP/status from the EWRAM buffer every tick so the
                // aggregator always sees current values without waiting for a
                // size change or the periodic DB-check interval.
                fill_party_list(&thread_party);

                if old_party_size != party_size {
                    old_party_size = party_size;
                    update_box_list();
                    *thread_box.lock_or_recover() = build_box_entries();
                    check_for_new_pokemon(&thread_party);
                    check_for_dead_pokemon(&thread_party, enc_tracker.run_tracking_active());
                    if check_for_run_over(&thread_party, enc_tracker.run_tracking_active()) {
                        enc_tracker.mark_wipe();
                        thread_wipe_signal.store(true, Ordering::Release);
                    }
                }

                if last_party_refresh.elapsed().as_secs() >= FORCE_PARTY_CHECK_INTERVAL {
                    last_party_refresh = std::time::Instant::now();
                    check_for_new_pokemon(&thread_party);
                    check_for_dead_pokemon(&thread_party, enc_tracker.run_tracking_active());
                    if check_for_run_over(&thread_party, enc_tracker.run_tracking_active()) {
                        enc_tracker.mark_wipe();
                        thread_wipe_signal.store(true, Ordering::Release);
                    }
                }

                if thread_run_changed.swap(false, Ordering::AcqRel) {
                    enc_tracker.reset();
                    player_name_set = false;
                }

                if state_initialized {
                    enc_tracker.tick(current_state, &thread_party);
                }

                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        *MAIN_THREAD_HANDLE.lock_or_recover() = Some(main_thread);
    }

    // ── Connected mode: dial out to the aggregator ────────────────────────────
    if let Mode::Connected { host, port } = &mode {
        let addr              = format!("{}:{}", host, port);
        let net_party         = shared_party.clone();
        let net_encounters    = shared_encounters.clone();
        let net_box           = shared_box.clone();
        let net_cache         = sprite_cache.clone();
        let net_loaded        = game_loaded.clone();
        let net_run_changed   = run_changed.clone();
        let net_wipe_signal   = wipe_signal.clone();

        let net_thread = std::thread::spawn(move || {
            loop {
                println!("Connecting to aggregator at {}...", addr);
                match TcpStream::connect(&addr) {
                    Ok(stream) => {
                        println!("Connected to aggregator.");
                        handle_client(
                            stream,
                            net_party.clone(),
                            net_encounters.clone(),
                            net_box.clone(),
                            net_cache.clone(),
                            net_loaded.clone(),
                            net_run_changed.clone(),
                            net_wipe_signal.clone(),
                            preferred_player,
                        );
                        println!("Disconnected from aggregator.");
                    }
                    Err(e) => eprintln!("Failed to connect to aggregator: {}", e),
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        });

        *NET_THREAD_HANDLE.lock_or_recover() = Some(net_thread);

        println!("{}", "***** Connected mode — no GUI. Press Ctrl-C to exit. *****".green().bold());
        ctrlc::set_handler(|| {
            RUNNING.store(false, Ordering::Release);
            println!("\nShutting down...");
            std::process::exit(0);
        }).expect("Error setting Ctrl-C handler.");
        while RUNNING.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        return;
    }

    // ── Standalone: show local GUI ────────────────────────────────────────────
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
            )))
        }),
    );
}
