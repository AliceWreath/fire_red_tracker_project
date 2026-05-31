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
use std::net::TcpListener;
use std::sync::Arc;

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

    /// Check GitHub for a newer release and replace this binary if one is found.
    #[arg(long)]
    update: bool,
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

    let config_path = cli.config.as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_config_path);
    let cfg = config::load_or_prompt(&config_path);

    // db: CLI arg overrides config.
    let db = cli.db.or(cfg.db).map(|s| {
        if s.starts_with("postgresql://") || s.starts_with("postgres://") {
            s
        } else {
            format!("postgresql://{}", s)
        }
    });

    let listen_port = cli.listen_port.unwrap_or(cfg.listen_port);
    let ws_port     = cli.ws_port.or(cfg.ws_port);

    // Shared slot list — grown as trackers connect.
    let shared_slots: SharedSlots = Arc::new(std::sync::Mutex::new(Vec::new()));

    // TCP listener — accepts incoming tracker connections.
    let listener = TcpListener::bind(format!("0.0.0.0:{}", listen_port))
        .unwrap_or_else(|e| {
            eprintln!("Failed to bind port {}: {}", listen_port, e);
            std::process::exit(1);
        });
    println!("Aggregator listening on port {} for tracker connections.", listen_port);

    let listener_slots = shared_slots.clone();
    let listener_db    = db.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s)  => s,
                Err(e) => {
                    eprintln!("Accept error: {}", e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            };
            let peer = stream.peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            println!("Tracker connected from {}", peer);

            // Reuse the first disconnected slot, or create a new one.
            let slot_arc = {
                let mut slots = listener_slots.lock().unwrap_or_else(|e| e.into_inner());
                let reuse = slots.iter().find(|s| {
                    s.state.lock().unwrap_or_else(|e| e.into_inner()).is_none()
                }).cloned();
                if let Some(s) = reuse {
                    // Reset stale per-connection state before handing to a new tracker.
                    s.known_species.lock().unwrap_or_else(|e| e.into_inner()).clear();
                    s.command_queue.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
            let cmd_queue    = slot_arc.command_queue.clone();
            let run_chg      = slot_arc.run_changed.clone();
            let box_data     = slot_arc.box_data.clone();

            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle_tracker_connection(
                        stream, state.clone(), pending, known, tex_queue,
                        label, sprite_cache, cmd_queue, run_chg, box_data,
                    );
                }));
                if result.is_err() {
                    eprintln!("Tracker thread for {} panicked — clearing slot state.", peer);
                    *state.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                println!("Tracker from {} disconnected.", peer);
            });
        }
    });

    if let Some(port) = ws_port {
        // Headless WebSocket overlay mode.
        web::run(shared_slots, port, db);
    } else {
        // Normal egui window mode.
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Fire Red Aggregator")
                .with_inner_size([640.0, 1000.0]),
            ..Default::default()
        };
        let _ = eframe::run_native(
            "Fire Red Aggregator",
            options,
            Box::new(move |cc| Ok(Box::new(AggregatorApp::new(cc, shared_slots)))),
        );
    }
}
