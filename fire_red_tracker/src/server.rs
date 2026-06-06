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

use crate::textures::{build_sprite_data, build_sprite_data_back};
use fire_red_states::*;
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for Mutex<T> {
    #[track_caller]
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            let loc = std::panic::Location::caller();
            eprintln!("Warning: mutex poisoned at {}:{}: {e}", loc.file(), loc.line());
            e.into_inner()
        })
    }
}

/// Manages the full lifecycle of a single TCP client connection in server mode.
///
/// # Arguments
///
/// * `stream`            — Connected TCP stream.
/// * `server_party`      — Shared party data to broadcast.
/// * `server_encounters` — Shared encounter data to broadcast.
/// * `server_box`        — Shared PC box snapshot; sent once on connect then every 5 s.
/// * `sprite_cache`      — Per-process sprite cache to amortise ROM decode cost.
/// * `game_loaded`       — Set to `false` during reset/title screen to suppress
///   stale badge data from being sent to clients.
/// * `run_changed`       — Set to `true` when a `EndRun` or `NewRun` command is
///   processed so the game loop can reset encounter state.
/// * `preferred_player`  — Preferred display slot sent to the aggregator on
///   every tick so it can sort columns correctly.
#[allow(clippy::too_many_arguments)]
pub fn handle_client(
    stream: TcpStream,
    server_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    server_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    server_box: Arc<Mutex<Vec<BoxEntry>>>,
    sprite_cache: Arc<Mutex<HashMap<(u16, bool, SpriteVariant), SpriteData>>>,
    game_loaded: Arc<AtomicBool>,
    run_changed: Arc<AtomicBool>,
    wipe_signal: Arc<AtomicBool>,
    preferred_player: Option<u8>,
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
                        if species == 0 || species > MAX_NATIONAL_DEX_FIRERED { continue; }
                        // Send front and back sprites for both palette variants.
                        for shiny in [false, true] {
                            let front_key = (species, shiny, SpriteVariant::Front);
                            let mut cache = cache_clone.lock_or_recover();
                            if let Some(data) = cache.get(&front_key) {
                                sprites.push(data.clone());
                            } else if let Some(data) = build_sprite_data(rom, species, shiny) {
                                cache.insert(front_key, data.clone());
                                sprites.push(data);
                            }

                            let back_key = (species, shiny, SpriteVariant::Back);
                            if let Some(data) = cache.get(&back_key) {
                                sprites.push(data.clone());
                            } else if let Some(data) = build_sprite_data_back(rom, species, shiny) {
                                cache.insert(back_key, data.clone());
                                sprites.push(data);
                            }
                        }
                    }

                    if !sprites.is_empty() {
                        let mut ws = write_stream_clone.lock_or_recover();
                        if send_message(&mut ws, &ServerMessage::Textures(sprites)).is_err() {
                            break;
                        }
                    }
                }
                Ok(ClientMessage::Hello(version)) => {
                    println!("Aggregator v{}", version);
                }
                Ok(ClientMessage::EndRun) => {
                    fire_red_database::end_run();
                    run_changed.store(true, Ordering::Release);
                    let mut ws = write_stream_clone.lock_or_recover();
                    if send_message(&mut ws, &ServerMessage::RunChanged(None)).is_err() {
                        break;
                    }
                }
                Ok(ClientMessage::NewRun) => {
                    match fire_red_database::new_run("Unknown") {
                        Ok(id) => {
                            run_changed.store(true, Ordering::Release);
                            let mut ws = write_stream_clone.lock_or_recover();
                            if send_message(&mut ws, &ServerMessage::RunChanged(Some(id))).is_err() {
                                break;
                            }
                        }
                        Err(e) => eprintln!("Failed to create new run: {e}"),
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Writer loop: pushes GameState every 100 ms and BoxData every 5 s.
    let mut last_box_send = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(10))
        .unwrap_or_else(std::time::Instant::now);

    loop {
        // Read the true player position and trainer name before acquiring any
        // shared locks. get_value() acquires the STATE mutex; if it were called
        // while holding server_encounters it would block the game thread for up
        // to SLEEP_DURATION (333 ms) every tick.
        let pos         = fire_red_loop::get_value();
        let player_name = fire_red_loop::get_trainer_name();
        let badge_state = if game_loaded.load(Ordering::Acquire) {
            fire_red_badge::read_badge_state()
        } else {
            None
        };

        let state = {
            let party      = server_party.lock_or_recover();
            let encounters = server_encounters.lock_or_recover();

            // Only resolve a zone name when the encounter header actually contains
            // pokemon — the default WildPokemonHeader (map_group=0, map_num=0)
            // is returned for all non-encounter maps and must not be looked up.
            let has_encounters =
                !encounters.land_mon_encounters.wild_pokemon_list.is_empty()
                || !encounters.water_mon_encounters.wild_pokemon_list.is_empty()
                || !encounters.rock_smash_encounters.wild_pokemon_list.is_empty()
                || !encounters.fishing_encounters.wild_pokemon_list.is_empty();

            let zone_name = if has_encounters {
                let name = fire_red_loop::get_area_name_for(
                    encounters.map_group,
                    encounters.map_num,
                );
                if !name.is_empty() {
                    name.to_string()
                } else {
                    format!("{}\u{00B7}{}", encounters.map_group, encounters.map_num)
                }
            } else {
                String::new()
            };

            GameState {
                party:             party.clone(),
                encounters:        encounters.clone(),
                player_name,
                badge_state,
                zone_name,
                current_map_group: pos.map_group_id,
                current_map_name:  pos.map_name_id,
                preferred_player,
            }
        };

        let mut ws = write_stream.lock_or_recover();
        if send_message(&mut ws, &ServerMessage::State(Box::new(state))).is_err() {
            println!("Client disconnected.");
            break;
        }

        if wipe_signal.swap(false, Ordering::AcqRel)
            && send_message(&mut ws, &ServerMessage::RunChanged(None)).is_err()
        {
            println!("Client disconnected.");
            break;
        }

        // Send box snapshot on first tick and every 5 seconds thereafter.
        if last_box_send.elapsed() >= std::time::Duration::from_secs(5) {
            let entries = server_box.lock_or_recover().clone();
            if send_message(&mut ws, &ServerMessage::BoxData(entries)).is_err() {
                println!("Client disconnected.");
                break;
            }
            last_box_send = std::time::Instant::now();
        }

        drop(ws);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
