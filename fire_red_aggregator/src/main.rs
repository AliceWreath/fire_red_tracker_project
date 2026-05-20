//! # FireRed Aggregator
//! 
//! Entry point for the multi-player aggregator binary.
//! 
//! Accepts one or more `host:port` addresses as command-line arguments, each
//! pointing to a running tracker server instance. For each address it creates
//! a [`MonitorSlot`] and spawns a background client thread via [`spawn_client`],
//! then opens an [`AggregatorApp`] window that renders all players side-by-side.
//! 
//! ## Usage
//! 
//! ```text
//! aggregator <host:post> [host:port ...]
//! 
//! # Example -  two local servers on different ports:
//! aggregator localhost:7878 localhost:7979
//! ```
//! 
//! ## Window sizing
//! 
//! The inital window width is `320 x max(slot_count, 2)` pixels, giving each
//! player column roughly 320 logical pixels. The window is resizable after
//! launch and the column layout reflows automatically.

mod app;
mod client;

use app::AggregatorApp;
use client::{MonitorSlot, spawn_client};

/// Prints usage instructions to stderr.
/// 
/// # Arguments
/// * `name` - The executable name taken from `args[0]`, shown verbtim in the
///            usage string so it reflects whatever the binary was invoked as.
fn print_usage(name: &str) {
    eprintln!("Usage: {} <host:port> [host:port ...]", name);
    eprintln!("  Example: {} localhost:7878 localhost:7879", name);
}

/// Parses server addresses from command-line arguments, creates one
/// [`MonitorSlot`] and one background client thread per address, then launches
/// the egui window.
/// 
/// Exits with code `1` if no addressess are provided.
fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    let addrs: Vec<String> = args[1..].to_vec();

    // For each address: create a MonitorSlot, then hand clones of its shared
    // arcs to spawn_client so the network thread and the GUI share the same
    // state without the slot giving up ownership of the arcs.
    let slots: Vec<MonitorSlot> = addrs
        .into_iter()
        .enumerate()
        .map(|(i, addr)| {
            let slot = MonitorSlot::new(i, addr.clone());
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

    // Initial window width scales with the number of slots (min 2 columns worth
    // of space) so the layout isn't cramped when launched with many players.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fire Red Aggregator")
            .with_inner_size([
                (320 * slots.len().max(2)) as f32,
                1000.0,
            ]),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Fire Red Aggregator",
        options,
        Box::new(move |cc| Box::new(AggregatorApp::new(cc, slots))),
    );
}
