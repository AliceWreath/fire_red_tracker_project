//! # FireRed Aggregator
//!
//! Listens for incoming tracker connections and displays all connected players
//! side-by-side.  Each tracker dials out to the aggregator — no addresses need
//! to be pre-configured here.
//!
//! # Usage
//!
//! ```text
//! aggregator                          # use config defaults
//! aggregator --listen-port <PORT>     # override listen port
//! aggregator --ws-port <PORT>         # headless WebSocket overlay mode
//! ```

mod app;
mod client;
mod config;
mod web;

use app::AggregatorApp;
use clap::Parser;
use client::{MonitorSlot, SharedSlots, handle_tracker_connection};
use fire_red_states::LockOrRecover;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};


// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to the config file (default: ~/.config/fire_red_aggregator/config.toml).
    #[arg(long, value_name = "FILE")]
    config: Option<String>,

    /// Override the database connection string stored in the config file.
    #[arg(long = "db", value_name = "CONN")]
    db: Option<String>,

    /// Override: listen for tracker connections on this port.
    #[arg(long = "listen-port", value_name = "PORT")]
    listen_port: Option<u16>,

    /// Override: run headless with a WebSocket overlay server on this port.
    #[arg(long = "ws-port", value_name = "PORT")]
    ws_port: Option<u16>,

    /// Open the configuration editor and exit.
    #[arg(long)]
    config_editor: bool,

    /// Check GitHub for a newer release and replace this binary if one is found.
    #[arg(long)]
    update: bool,

    /// Apply the [test] section from the config file on top of normal settings.
    /// Explicit flags (--db, --listen-port, --ws-port) still override the test section.
    #[arg(long)]
    test: bool,

    /// Disable all injection API endpoints (give_item, make_shiny, change_species, etc.).
    /// Overrides allow_injections = true in the config file.
    #[arg(long)]
    no_injections: bool,
}

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
        .bin_name("fire_red_aggregator")
        .identifier("fire_red_aggregator")
        .show_download_progress(true)
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()
        .and_then(|u| u.update());

    match result {
        Ok(self_update::Status::UpToDate(v)) => {
            println!("Already up to date (v{}).", v);
        }
        Ok(self_update::Status::Updated(v)) => {
            println!("Updated to v{}. Restart the aggregator to use the new version.", v);
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

    let config_path = cli.config.as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_config_path);

    if cli.config_editor {
        config::run_config_editor(&config_path);
        return;
    }

    let cfg     = config::load_or_prompt(&config_path);
    let cfg_ref = cfg.clone();

    // test section: applied on top of base config, below explicit CLI flags.
    let use_test = cli.test || cfg.default_test;
    let test_ov  = if use_test { cfg.test.clone() } else { None };
    let test     = test_ov.as_ref();
    if use_test {
        println!("Test mode active — using [test] config overrides.");
    }

    // Priority: base config → [test] overrides → explicit CLI flags.
    // DB URL normalization (postgresql:// prefix) is handled inside initialize().
    let db = cli.db
        .or_else(|| test.and_then(|t| t.db.clone()))
        .or(cfg.db);

    let listen_port = cli.listen_port
        .or_else(|| test.and_then(|t| t.listen_port))
        .unwrap_or(cfg.listen_port);
    let ws_port = cli.ws_port
        .or_else(|| test.and_then(|t| t.ws_port))
        .or(cfg.ws_port);
    // --no-injections overrides allow_injections = true in config; config false always wins.
    let allow_injections = cfg.allow_injections && !cli.no_injections;

    // Shared slot list — grown as trackers connect.
    let shared_slots: SharedSlots = Arc::new(std::sync::Mutex::new(Vec::new()));

    // TCP listener — accepts incoming tracker connections.
    let listener = TcpListener::bind(format!("0.0.0.0:{}", listen_port))
        .unwrap_or_else(|e| {
            tracing::error!("Failed to bind port {}: {}", listen_port, e);
            std::process::exit(1);
        });
    tracing::info!("Aggregator listening on port {} for tracker connections.", listen_port);

    let listener_slots = shared_slots.clone();
    let listener_db    = db.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s)  => s,
                Err(e) => {
                    tracing::warn!("Accept error: {}", e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            };
            let peer = stream.peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            tracing::info!("Tracker connected from {}", peer);

            // Reuse the first disconnected slot, or create a new one.
            let slot_arc = {
                let mut slots = listener_slots.lock_or_recover();
                let reuse = slots.iter().find(|s| {
                    s.state.lock_or_recover().is_none()
                }).cloned();
                if let Some(s) = reuse {
                    // Reset stale per-connection state before handing to a new tracker.
                    s.known_species.lock_or_recover().clear();
                    s.command_queue.lock_or_recover().clear();
                    s.run_changed.store(false, std::sync::atomic::Ordering::Relaxed);
                    s
                } else {
                    let idx = slots.len();
                    let new = Arc::new(MonitorSlot::new(idx, peer.clone(), listener_db.clone()));
                    slots.push(new.clone());
                    new
                }
            };

            let state        = slot_arc.state.clone();
            let pending      = slot_arc.pending_textures.clone();
            let known        = slot_arc.known_species.clone();
            let tex_queue    = slot_arc.texture_request_queue.clone();
            let label        = slot_arc.label.clone();
            let sprite_cache = slot_arc.sprite_cache.clone();
            let bag_data     = slot_arc.bag_data.clone();
            let cmd_queue    = slot_arc.command_queue.clone();
            let run_chg      = slot_arc.run_changed.clone();
            let box_data     = slot_arc.box_data.clone();

            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle_tracker_connection(
                        stream, state.clone(), pending, known, tex_queue,
                        label, sprite_cache, cmd_queue, run_chg, box_data, bag_data,
                    );
                }));
                if result.is_err() {
                    tracing::error!("Tracker thread for {} panicked — clearing slot state.", peer);
                    *state.lock_or_recover() = None;
                }
                tracing::info!("Tracker from {} disconnected.", peer);
            });
        }
    });

    if let Some(port) = ws_port {
        // Headless WebSocket overlay mode.
        web::run(shared_slots, port, db, use_test, allow_injections);
    } else {
        // Normal egui window mode.
        let update_available: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        {
            let flag = update_available.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(10));
                let result = self_update::backends::github::Update::configure()
                    .repo_owner("AliceWreath")
                    .repo_name("fire_red_tracker_project")
                    .bin_name("fire_red_aggregator")
                    .identifier("fire_red_aggregator")
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

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Fire Red Aggregator")
                .with_inner_size([640.0, 1000.0]),
            ..Default::default()
        };
        if let Err(e) = eframe::run_native(
            "Fire Red Aggregator",
            options,
            Box::new(move |cc| Ok(Box::new(AggregatorApp::new(cc, shared_slots, config_path, &cfg_ref, update_available)))),
        ) {
            tracing::error!("GUI exited with error: {e}");
        }
    }
}
