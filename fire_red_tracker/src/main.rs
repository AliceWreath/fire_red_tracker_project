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

use clap::Parser;
use cli::{Cli, Command};
use colored::Colorize;
use fire_red_loop::*;
use fire_red_states::*;
use game::{check_for_dead_pokemon, check_for_new_pokemon, fill_party_list, game_is_loaded, map_state_from_ewram};
use gui::{WindowInfo, PARTY_WINDOW};
use server::handle_client;
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How often (in seconds) the party list is force-refreshed even when the
/// party size has not changed, to catch in-place changes such as HP loss.
const FORCE_PARTY_CHECK_INTERVAL: u64 = 5;

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

fn main() {
    let cli = Cli::parse();

    // Load config (prompts on first run), then overlay any CLI overrides.
    let config_path = cli.config.as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_config_path);
    let cfg = config::load_or_prompt(&config_path);

    // Mode: CLI subcommand wins; otherwise use what the config says.
    let mode = match cli.command {
        Some(Command::Connect { host, port }) => Mode::Connected { host, port },
        None => match cfg.mode {
            config::ConfigMode::Standalone => Mode::Standalone,
            config::ConfigMode::Connected  => Mode::Connected {
                host: cfg.aggregator_host.clone(),
                port: cfg.aggregator_port,
            },
        },
    };

    // db: CLI arg overrides config.
    let db_raw = cli.db.unwrap_or(cfg.db);
    let db_conn = if db_raw.starts_with("postgresql://") || db_raw.starts_with("postgres://") {
        db_raw
    } else {
        format!("postgresql://{}", db_raw)
    };
    fire_red_database::initialize(&db_conn);

    // --list-runs: print stored runs and exit without starting the tracker.
    if cli.list_runs {
        let runs = fire_red_database::list_runs();
        if runs.is_empty() {
            println!("No runs found.");
        } else {
            let active = fire_red_database::active_run_id();
            println!("{:<5} {:<12} {:<26} {}", "ID", "Player", "Started", "Deaths");
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
    match (cli.run_id, cli.new_run) {
        (Some(id), _) => {
            if !fire_red_database::resume_run(id) {
                eprintln!("Error: run #{} not found. Use --list-runs to see available runs.", id);
                std::process::exit(1);
            }
            println!("Resuming run #{}.", id);
        }
        (None, true) => {
            let id = fire_red_database::new_run("Unknown");
            println!("Started new run #{}.", id);
        }
        (None, false) => {
            let id = fire_red_database::get_or_create_run("Unknown");
            println!("Using run #{}.", id);
        }
    }

    let is_clean  = cfg.clean || cli.clean;
    let rom_path  = cli.rom.unwrap_or(cfg.rom);

    let game_loaded: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let run_changed: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    let shared_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>> =
        Arc::new(Mutex::new(Vec::new()));
    let shared_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>> =
        Arc::new(Mutex::new(fire_red_pokemon_data::WildPokemonHeader::default()));
    let sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // ── Game-polling thread (both modes) ──────────────────────────────────────
    {
        let thread_party       = shared_party.clone();
        let thread_encounters  = shared_encounters.clone();
        let thread_game_loaded = game_loaded.clone();
        let thread_run_changed = run_changed.clone();

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

            let initial_state = map_state_from_ewram()
                .unwrap_or(FireRedState { map_group_id: 0, map_name_id: 0 });

            *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                get_area_pokemon_id_for_state(&initial_state);

            let mut current_state      = initial_state;
            let mut old_party_size     = get_party_size();
            let mut last_party_refresh = std::time::Instant::now();
            let mut state_initialized  = false;
            let mut player_name_set    = false;
            let mut enc_tracker        = encounter::EncounterTracker::new();

            fill_party_list(&thread_party);
            check_for_new_pokemon(&thread_party);
            check_for_dead_pokemon(&thread_party);

            loop {
                if !game_is_loaded() {
                    thread_game_loaded.store(false, Ordering::Release);
                    *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                        fire_red_pokemon_data::WildPokemonHeader::default();
                    *thread_party.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
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
                    *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                        get_area_pokemon_id_for_state(&current_state);
                }

                if state_initialized && current_state != state {
                    current_state = state;
                    *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                        get_area_pokemon_id_for_state(&current_state);
                }

                if old_party_size != party_size {
                    old_party_size = party_size;
                    update_box_list();
                    fill_party_list(&thread_party);
                    check_for_dead_pokemon(&thread_party);
                }

                if last_party_refresh.elapsed().as_secs() >= FORCE_PARTY_CHECK_INTERVAL {
                    last_party_refresh = std::time::Instant::now();
                    fill_party_list(&thread_party);
                    check_for_dead_pokemon(&thread_party);
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

        *MAIN_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(main_thread);
    }

    // ── Connected mode: dial out to the aggregator ────────────────────────────
    if let Mode::Connected { host, port } = &mode {
        let addr              = format!("{}:{}", host, port);
        let net_party         = shared_party.clone();
        let net_encounters    = shared_encounters.clone();
        let net_cache         = sprite_cache.clone();
        let net_loaded        = game_loaded.clone();
        let net_run_changed   = run_changed.clone();

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
                            net_cache.clone(),
                            net_loaded.clone(),
                            net_run_changed.clone(),
                        );
                        println!("Disconnected from aggregator.");
                    }
                    Err(e) => eprintln!("Failed to connect to aggregator: {}", e),
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        });

        *NET_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(net_thread);

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
        Box::new(|cc| {
            Ok(Box::new(WindowInfo::new(
                cc,
                shared_party,
                shared_encounters,
                Arc::new(Mutex::new(Vec::new())),
                Arc::new(Mutex::new(std::collections::HashSet::new())),
                None,
            )))
        }),
    );
}
