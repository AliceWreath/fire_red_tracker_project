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
mod web;

use app::AggregatorApp;
use clap::Parser;
use client::{MonitorSlot, spawn_client};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// Multi-player FireRed tracker aggregator.
///
/// Connect to one or more tracker server instances and display all players
/// side-by-side in a single window.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// One or more tracker server addresses in `host:port` format.
    ///
    /// Example: `localhost:7878 localhost:7979`
    #[arg(required = true, value_name = "HOST:PORT")]
    addrs: Vec<String>,

    /// PostgreSQL connection string shared by all player slots.
    ///
    /// Example: `--db postgresql://localhost/nuzlocke`
    #[arg(long = "db", value_name = "CONN")]
    db: Option<String>,

    /// Run as a headless WebSocket overlay server instead of opening a window.
    /// OBS connects to the served URL as a Browser Source.
    ///
    /// Example: `--ws-port 9090`
    #[arg(long = "ws-port", value_name = "PORT")]
    ws_port: Option<u16>,

}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

/// Parses server addresses, creates one [`MonitorSlot`] and one background
/// client thread per address, then launches the egui window.
fn main() {
    let cli = Cli::parse();

    // For each address: create a MonitorSlot, then hand clones of its shared
    // Arcs to spawn_client so the network thread and the GUI share the same
    // state without the slot giving up ownership of the Arcs.
    let db = cli.db;
    let slots: Vec<MonitorSlot> = cli
        .addrs
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
            );
            slot
        })
        .collect();

    if let Some(port) = cli.ws_port {
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
            Box::new(move |cc| Box::new(AggregatorApp::new(cc, slots))),
        );
    }
}
