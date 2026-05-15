mod app;
mod client;

use app::AggregatorApp;
use client::{MonitorSlot, spawn_client};

fn print_usage(name: &str) {
    eprintln!("Usage: {} <rom_path> <host:port> [host:port ...]", name);
    eprintln!("  Example: {} firered.gba localhost:7878 localhost:7879", name);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    let rom_path = &args[1];
    let addrs: Vec<String> = args[2..].to_vec();

    // Load ROM for texture rendering
    fire_red_rom_buffer::init_rom(rom_path).unwrap_or_else(|e| {
        eprintln!("Failed to load ROM at '{}': {}", rom_path, e);
        std::process::exit(1);
    });

    // Build slots and spawn one client thread per monitor address
    let slots: Vec<MonitorSlot> = addrs
        .into_iter()
        .enumerate()
        .map(|(i, addr)| {
            let slot = MonitorSlot::new(i, addr.clone());
            spawn_client(addr, slot.state.clone());
            slot
        })
        .collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fire Red Aggregator")
            .with_inner_size([
                // Reasonable starting width: 320px per player
                (320 * slots.len().max(2)) as f32,
                800.0,
            ]),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Fire Red Aggregator",
        options,
        Box::new(move |cc| Box::new(AggregatorApp::new(cc, slots))),
    );
}