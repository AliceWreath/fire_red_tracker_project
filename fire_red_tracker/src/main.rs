//! # FireRed Tracker
//!
//! A real-time Pokémon FireRed party and encounter monitor with an egui GUI.
//!
//! # Modes
//!
//! - **Standalone** — reads the ROM and game memory locally, renders the GUI.
//! - **Server** — like standalone but also accepts TCP client connections and
//!   streams [`GameState`] updates and sprite data to them. Runs headless.
//! - **Client** — connects to a server over TCP, receives state and sprites,
//!   and renders the GUI without needing the ROM locally.
//!
//! # Usage
//!
//! ```text
//! tracker <ROM>                                    # standalone
//! tracker <ROM> --clean                            # standalone, ability names enabled
//! tracker <ROM> server [--port <PORT>]             # server (default port 7878)
//! tracker client [--host <HOST>] [--port <PORT>]   # client (default 127.0.0.1:7878)
//! ```
//!
//! ROM paths containing spaces can be quoted: `tracker "My ROMs/firered.gba"`
//!
//! # Module layout
//!
//! | Module        | Responsibility                                         |
//! |---------------|--------------------------------------------------------|
//! | [`cli`]       | clap CLI struct and subcommand definitions             |
//! | [`game`]      | EWRAM/IWRAM helpers, `is_shiny`, `game_is_loaded`      |
//! | [`textures`]  | Sprite loading, compression, `PendingTexture`          |
//! | [`gui`]       | `WindowInfo`, egui rendering, party/encounter panels   |
//! | [`server`]    | Per-client TCP handler for server mode                 |

mod cli;
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
use textures::{PendingTexture, decompress_pixels};
use std::collections::{HashMap, VecDeque};
use std::net::{TcpListener, TcpStream};
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
static CLIENT_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static SERVER_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Set to `false` by the Ctrl-C handler to trigger a clean shutdown in server mode.
static RUNNING: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let mode = match cli.command {
        Some(Command::Server { port })       => Mode::Server { port },
        Some(Command::Client { host, port }) => Mode::Client { host, port },
        None                                 => Mode::Standalone,
    };

    let db_conn = if cli.db.starts_with("postgresql://") || cli.db.starts_with("postgres://") {
        cli.db.clone()
    } else {
        format!("postgresql://{}", cli.db)
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

    let is_clean = cli.clean;

    // ROM is required in standalone and server modes.
    let rom_path = match &mode {
        Mode::Client { .. } => String::new(),
        _ => match cli.rom {
            Some(path) => path,
            None => {
                eprintln!("Error: a ROM path is required in standalone and server modes.");
                eprintln!("Usage: tracker <ROM> [--clean]");
                eprintln!("       tracker <ROM> server [--port <PORT>]");
                eprintln!("       tracker client [--host <HOST>] [--port <PORT>]");
                std::process::exit(1);
            }
        },
    };

    // Tracks whether the game is fully loaded (past title screen).
    // Shared between the game-polling thread (writer) and handle_client (reader).
    let game_loaded: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    // Shared state between the game-polling / network threads and the GUI.
    let shared_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>> =
        Arc::new(Mutex::new(Vec::new()));
    let shared_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>> =
        Arc::new(Mutex::new(fire_red_pokemon_data::WildPokemonHeader::default()));

    // Sprite cache shared across all server clients to avoid re-decoding the ROM.
    let sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Client-mode texture pipeline.
    let pending_textures: Arc<Mutex<Vec<PendingTexture>>> = Arc::new(Mutex::new(Vec::new()));
    let known_species: Arc<Mutex<std::collections::HashSet<u16>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    let texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>> =
        Arc::new(Mutex::new(VecDeque::new()));

    match &mode {
        Mode::Standalone | Mode::Server { .. } => {
            let thread_party       = shared_party.clone();
            let thread_encounters  = shared_encounters.clone();
            let thread_game_loaded = game_loaded.clone();

            let main_thread = std::thread::spawn(move || {
                match start_loop(rom_path.as_str(), is_clean) {
                    0    => println!("Monitor loop started."),
                    code => {
                        eprintln!("Failed to start monitor loop (code {}).", code);
                        std::process::exit(1);
                    }
                }

                // Wait for the EWRAM snapshot to contain a real non-zero map state.
                // The memory loop needs one full poll cycle (~500ms) to populate.
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

                // Load initial encounters directly from EWRAM, bypassing STATE which
                // may not have been written by the map-polling thread yet.
                let initial_state = map_state_from_ewram()
                    .unwrap_or(FireRedState { map_group_id: 0, map_name_id: 0 });

                println!(
                    "Map state ready: group={} name={}",
                    initial_state.map_group_id, initial_state.map_name_id,
                );

                *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                    get_area_pokemon_id_for_state(&initial_state);

                let mut current_state      = initial_state;
                let mut old_party_size     = get_party_size();
                let mut last_party_refresh = std::time::Instant::now();
                let mut state_initialized  = false;
                let mut player_name_set    = false;

                fill_party_list(&thread_party);
                check_for_new_pokemon(&thread_party);
                check_for_dead_pokemon(&thread_party);

                loop {
                    // Check whether the game is fully loaded before doing anything else.
                    // On reset or title screen, clear stale state and wait.
                    if !game_is_loaded() {
                        thread_game_loaded.store(false, Ordering::SeqCst);
                        *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                            fire_red_pokemon_data::WildPokemonHeader::default();
                        *thread_party.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
                        state_initialized = false;
                        player_name_set   = false;
                        current_state = FireRedState { map_group_id: 0xFF, map_name_id: 0xFF };
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        continue;
                    }
                    thread_game_loaded.store(true, Ordering::SeqCst);

                    // Capture the player name once the game is confirmed loaded.
                    if !player_name_set {
                        let name = get_trainer_name();
                        if !name.trim().is_empty() {
                            fire_red_database::set_player_name(&name);
                            player_name_set = true;
                        }
                    }

                    // Read map state directly from EWRAM — faster and more reliable than
                    // get_value() which lags by up to ~833ms through two polling intervals.
                    let state      = map_state_from_ewram().unwrap_or(current_state);
                    let party_size = get_party_size();

                    // Mark as initialized once we see a real non-zero state, and
                    // immediately populate encounters for the current map so the
                    // window is not empty on first load or after a reconnect.
                    if !state_initialized
                        && (state.map_group_id != 0 || state.map_name_id != 0)
                    {
                        state_initialized = true;
                        current_state = state;
                        *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                            get_area_pokemon_id_for_state(&current_state);
                    }

                    // Update encounters when the map changes.
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

                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });

            *MAIN_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(main_thread);

            if let Mode::Server { port } = &mode {
                let port              = *port;
                let server_party      = shared_party.clone();
                let server_encounters = shared_encounters.clone();
                let server_cache      = sprite_cache.clone();

                let server_thread = std::thread::spawn(move || {
                    let listener = match TcpListener::bind(format!("0.0.0.0:{}", port)) {
                        Ok(l)  => l,
                        Err(e) => { eprintln!("Failed to bind port {}: {}", port, e); return; }
                    };
                    println!("Server listening on port {}.", port);

                    for stream in listener.incoming() {
                        if !RUNNING.load(Ordering::SeqCst) { break; }
                        match stream {
                            Ok(s) => {
                                let party      = server_party.clone();
                                let encounters = server_encounters.clone();
                                let cache      = server_cache.clone();
                                let loaded     = game_loaded.clone();
                                std::thread::spawn(move || handle_client(s, party, encounters, cache, loaded));
                            }
                            Err(e) => eprintln!("Connection error: {}", e),
                        }
                    }
                });

                *SERVER_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(server_thread);
            }
        }

        Mode::Client { host, port } => {
            let addr              = format!("{}:{}", host, port);
            let client_party      = shared_party.clone();
            let client_encounters = shared_encounters.clone();
            let client_pending    = pending_textures.clone();
            let client_known      = known_species.clone();
            let client_queue      = texture_request_queue.clone();

            let client_thread = std::thread::spawn(move || {
                loop {
                    println!("Connecting to {}...", addr);
                    match TcpStream::connect(&addr) {
                        Ok(stream) => {
                            println!("Connected to server.");

                            let mut write_stream = match stream.try_clone() {
                                Ok(s)  => s,
                                Err(e) => { eprintln!("Failed to clone stream: {}", e); break; }
                            };
                            let mut read_stream  = stream;
                            let connected        = Arc::new(AtomicBool::new(true));
                            let connected_writer = connected.clone();
                            let writer_queue     = client_queue.clone();

                            // Writer thread: batches and sends texture requests every 50ms.
                            let writer = std::thread::spawn(move || {
                                while connected_writer.load(Ordering::SeqCst) {
                                    let batch = {
                                        let mut q = writer_queue.lock().unwrap_or_else(|e| e.into_inner());
                                        let mut all: Vec<u16> = q.drain(..).flatten().collect();
                                        all.sort();
                                        all.dedup();
                                        all
                                    };
                                    if !batch.is_empty() {
                                        if send_message(
                                            &mut write_stream,
                                            &ClientMessage::RequestTextures(batch),
                                        ).is_err() { break; }
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                            });

                            // Reader loop: receives State and Textures from the server.
                            loop {
                                match recv_message::<ServerMessage>(&mut read_stream) {
                                    Ok(ServerMessage::State(state)) => {
                                        *client_party.lock().unwrap_or_else(|e| e.into_inner()) =
                                            state.party;
                                        *client_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                                            state.encounters;
                                    }
                                    Ok(ServerMessage::Textures(sprites)) => {
                                        let mut pending = client_pending.lock().unwrap_or_else(|e| e.into_inner());
                                        let mut known   = client_known.lock().unwrap_or_else(|e| e.into_inner());
                                        for sprite in sprites {
                                            known.insert(sprite.species);
                                            pending.push(PendingTexture {
                                                species: sprite.species,
                                                shiny:   sprite.shiny,
                                                pixels:  decompress_pixels(&sprite.pixels),
                                                width:   sprite.width,
                                                height:  sprite.height,
                                            });
                                        }
                                    }
                                    Err(e) => { eprintln!("Lost connection: {}", e); break; }
                                }
                            }

                            connected.store(false, Ordering::SeqCst);
                            let _ = writer.join();
                        }
                        Err(e) => eprintln!("Failed to connect: {}", e),
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            });

            *CLIENT_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(client_thread);
        }
    }

    // ── Server mode: headless, park until Ctrl-C ─────────────────────────────
    if let Mode::Server { .. } = &mode {
        println!("{}", "***** Server mode — no GUI. Press Ctrl-C to exit. *****".green().bold());
        ctrlc::set_handler(|| {
            RUNNING.store(false, Ordering::SeqCst);
            println!("\nShutting down...");
            std::process::exit(0);
        }).expect("Error setting Ctrl-C handler.");
        while RUNNING.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        return;
    }

    // ── GUI (Standalone + Client) ─────────────────────────────────────────────
    let app_title = match &mode {
        Mode::Standalone            => "Tracker".to_string(),
        Mode::Server { port }       => format!("Tracker (Server :{})", port),
        Mode::Client { host, port } => format!("Tracker (Client {}:{})", host, port),
    };

    let queue_for_gui = match &mode {
        Mode::Client { .. } => Some(texture_request_queue),
        _                   => None,
    };

    let _ = eframe::run_native(
        &app_title,
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
                pending_textures,
                known_species,
                queue_for_gui,
            )))
        }),
    );
}
