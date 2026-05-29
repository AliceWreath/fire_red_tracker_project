//! # FireRed Aggregator
//!
//! Entry point for the multi-player aggregator binary.
//!
//! Accepts one or more `host:port` addresses as arguments, each pointing to a
//! running tracker server instance. For each address it creates a [`MonitorSlot`]
//! and spawns a background client thread via [`spawn_client`], then opens an
//! [`AggregatorApp`] window that renders all players side-by-side.
//!
//! # Usage
//!
//! ```text
//! aggregator <HOST:PORT> [HOST:PORT ...]
//!
//! # Two local servers on different ports:
//! aggregator localhost:7878 localhost:7979
//! ```
//!
//! # Window sizing
//!
//! The initial window width is `320 × max(slot_count, 2)` logical pixels,
//! giving each player column roughly 320 pixels. The window is resizable after
//! launch and the column layout reflows automatically.

mod app;
mod client;
mod config;
mod web;

use app::AggregatorApp;
use clap::Parser;
use client::{MonitorSlot, spawn_client};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// Multi-player FireRed tracker aggregator.
///
/// Server addresses, database, and WebSocket overlay settings are read from
/// the config file at first launch and saved for future runs.  Any value can
/// be overridden for a single run with the corresponding argument below.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Override: one or more tracker server addresses in `host:port` format.
    /// Replaces the address list stored in the config file for this run.
    #[arg(value_name = "HOST:PORT")]
    addrs: Vec<String>,

    /// Path to the config file (default: ~/.config/fire_red_aggregator/config.toml).
    #[arg(long, value_name = "FILE")]
    config: Option<String>,

    /// Override the database connection string stored in the config file.
    #[arg(long = "db", value_name = "CONN")]
    db: Option<String>,

    /// Override: run headless with a WebSocket overlay server on this port.
    /// OBS connects to the served URL as a Browser Source.
    #[arg(long = "ws-port", value_name = "PORT")]
    ws_port: Option<u16>,
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

/// Parses CLI overrides, loads (or prompts for) the config, then creates one
/// [`MonitorSlot`] and one background client thread per address before
/// launching the egui window.
fn main() {
    let cli = Cli::parse();

    // Load config (prompts on first run), then overlay any CLI overrides.
    let config_path = cli.config.as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config::default_config_path);
    let cfg = config::load_or_prompt(&config_path);

    // Addresses: CLI positional args replace config list when provided.
    let addrs = if cli.addrs.is_empty() { cfg.addrs } else { cli.addrs };
    if addrs.is_empty() {
        eprintln!("Error: no server addresses provided. Add them to the config file or pass them as arguments.");
        std::process::exit(1);
    }

    // db: CLI arg overrides config.
    let db = cli.db.or(cfg.db).map(|s| {
        if s.starts_with("postgresql://") || s.starts_with("postgres://") {
            s
        } else {
            format!("postgresql://{}", s)
        }
    });

    // ws-port: CLI arg overrides config.
    let ws_port = cli.ws_port.or(cfg.ws_port);

    let slots: Vec<MonitorSlot> = addrs
        .into_iter()
        .enumerate()
        .map(|(i, addr)| {
            let slot = MonitorSlot::new(i, addr.clone(), db.clone());
            spawn_client(
                addr,
                slot.state.clone(),
                slot.pending_textures.clone(),
                slot.known_species.clone(),
                slot.texture_request_queue.clone(),
                slot.label.clone(),
                slot.sprite_cache.clone(),
                slot.command_queue.clone(),
                slot.run_changed.clone(),
            );
            slot
        })
        .collect();

    if let Some(port) = ws_port {
        // Headless WebSocket overlay mode — no window opened.
        web::run(slots, port);
    } else {
        // Normal egui window mode.
        let slot_count = slots.len();
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Fire Red Aggregator")
                .with_inner_size([(320 * slot_count.max(2)) as f32, 1000.0]),
            ..Default::default()
        };
        let _ = eframe::run_native(
            "Fire Red Aggregator",
            options,
            Box::new(move |cc| Ok(Box::new(AggregatorApp::new(cc, slots)))),
        );
    }
}
