//! # Server: per-client handler
//!
//! Manages the full lifecycle of a single TCP client connection in server mode.
//!
//! [`handle_client`] spawns a **reader thread** that handles
//! [`ClientMessage::RequestTextures`] and responds with compressed sprite data
//! for both normal and shiny variants, cached in `sprite_cache` to avoid
//! re-decoding the ROM on repeated requests.
//!
//! The **writer loop** (on the calling thread) broadcasts a
//! [`ServerMessage::State`] snapshot every 100 ms and exits on any write error
//! (client disconnect).

use crate::textures::build_sprite_data;
use fire_red_states::*;
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Manages the full lifecycle of a single TCP client connection in server mode.
///
/// # Arguments
///
/// * `stream`            — Connected TCP stream.
/// * `server_party`      — Shared party data to broadcast.
/// * `server_encounters` — Shared encounter data to broadcast.
/// * `sprite_cache`      — Per-process sprite cache to amortise ROM decode cost.
/// * `game_loaded`       — Set to `false` during reset/title screen to suppress
///                         stale badge data from being sent to clients.
/// * `run_changed`       — Set to `true` when a `EndRun` or `NewRun` command is
///                         processed so the game loop can reset encounter state.
pub fn handle_client(
    stream: TcpStream,
    server_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    server_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>>,
    game_loaded: Arc<AtomicBool>,
    run_changed: Arc<AtomicBool>,
) {
    println!(
        "Client connected: {}",
        stream.peer_addr().map_or_else(|_| "unknown".to_string(), |a| a.to_string()),
    );

    let read_stream = match stream.try_clone() {
        Ok(s)  => s,
        Err(e) => { eprintln!("Failed to clone stream: {}", e); return; }
    };
    let write_stream = Arc::new(Mutex::new(stream));

    // Reader thread: responds to texture requests and run commands.
    let write_stream_clone = write_stream.clone();
    let cache_clone        = sprite_cache.clone();
    std::thread::spawn(move || {
        let mut read_stream = read_stream;
        loop {
            match recv_message::<ClientMessage>(&mut read_stream) {
                Ok(ClientMessage::RequestTextures(species_list)) => {
                    let rom = fire_red_rom_buffer::get_rom();
                    let mut sprites: Vec<SpriteData> = Vec::new();

                    for species in species_list {
                        if species == 0 || species > 386 { continue; }
                        // Always send both variants so the client never needs the ROM.
                        for shiny in [false, true] {
                            let key = (species, shiny);
                            let mut cache = cache_clone.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(data) = cache.get(&key) {
                                sprites.push(data.clone());
                            } else if let Some(data) = build_sprite_data(rom, species, shiny) {
                                cache.insert(key, data.clone());
                                sprites.push(data);
                            }
                        }
                    }

                    if !sprites.is_empty() {
                        let mut ws = write_stream_clone.lock().unwrap_or_else(|e| e.into_inner());
                        if send_message(&mut *ws, &ServerMessage::Textures(sprites)).is_err() {
                            break;
                        }
                    }
                }
                Ok(ClientMessage::EndRun) => {
                    fire_red_database::end_run();
                    run_changed.store(true, Ordering::Release);
                    let mut ws = write_stream_clone.lock().unwrap_or_else(|e| e.into_inner());
                    if send_message(&mut *ws, &ServerMessage::RunChanged(None)).is_err() {
                        break;
                    }
                }
                Ok(ClientMessage::NewRun) => {
                    let id = fire_red_database::new_run("Unknown");
                    run_changed.store(true, Ordering::Release);
                    let mut ws = write_stream_clone.lock().unwrap_or_else(|e| e.into_inner());
                    if send_message(&mut *ws, &ServerMessage::RunChanged(Some(id))).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Writer loop: pushes GameState every 100 ms.
    loop {
        let state = {
            let party      = server_party.lock().unwrap_or_else(|e| e.into_inner());
            let encounters = server_encounters.lock().unwrap_or_else(|e| e.into_inner());
            GameState {
                party:       party.clone(),
                encounters:  encounters.clone(),
                player_name: fire_red_loop::get_trainer_name(),
                // Only read badge state when the game is fully loaded.
                // During a reset or title screen this returns None so clients
                // clear their badge display rather than showing stale data.
                badge_state: if game_loaded.load(Ordering::Acquire) {
                    fire_red_badge::read_badge_state()
                } else {
                    None
                },
            }
        };

        let mut ws = write_stream.lock().unwrap_or_else(|e| e.into_inner());
        if send_message(&mut *ws, &ServerMessage::State(state)).is_err() {
            println!("Client disconnected.");
            break;
        }
        drop(ws);

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
