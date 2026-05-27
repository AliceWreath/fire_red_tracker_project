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

    /// PostgreSQL connection string for each player's nuzlocke database, in
    /// the same order as the server addresses.  May be specified fewer times
    /// than there are addresses; unmatched slots will show no catch/death
    /// history.
    ///
    /// Example: `--db postgresql://localhost/nuzlocke --db postgresql://192.168.1.2/nuzlocke`
    #[arg(long = "db", value_name = "CONN")]
    dbs: Vec<String>,
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
    let dbs = cli.dbs;
    let slots: Vec<MonitorSlot> = cli
        .addrs
        .into_iter()
        .enumerate()
        .map(|(i, addr)| {
            let db_path = dbs.get(i).cloned();
            let slot = MonitorSlot::new(i, addr.clone(), db_path);
            spawn_client(
                addr,
                slot.state.clone(),
                slot.pending_textures.clone(),
                slot.known_species.clone(),
                slot.texture_request_queue.clone(),
                slot.label.clone(),
            );
            slot
        })
        .collect();

    // Scale the initial window width with the number of slots (minimum 2
    // columns) so the layout isn't cramped when launched with many players.
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
