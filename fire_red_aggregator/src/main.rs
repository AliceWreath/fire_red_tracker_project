mod app;
mod client;

use app::AggregatorApp;
use client::{MonitorSlot, spawn_client};

fn print_usage(name: &str) {
    eprintln!("Usage: {} <host:port> [host:port ...]", name);
    eprintln!("  Example: {} localhost:7878 localhost:7879", name);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    let addrs: Vec<String> = args[1..].to_vec();

    // Build slots and spawn one client thread per monitor address
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
            );
            slot
        })
        .collect();

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
