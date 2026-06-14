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
use fire_red_states::{
    BagPockets, BoxEntry, ClientMessage, GameState, LockOrRecover, MAX_NATIONAL_DEX_FIRERED,
    ServerMessage, SpriteData, SpriteVariant, recv_message, send_message,
};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) type RomSpriteCache = Arc<Mutex<HashMap<(u16, bool, SpriteVariant), SpriteData>>>;

/// Manages the full lifecycle of a single TCP client connection in server mode.
///
/// # Arguments
///
/// * `stream`            — Connected TCP stream.
/// * `server_party`      — Shared party data to broadcast.
/// * `server_encounters` — Shared encounter data to broadcast.
/// * `server_box`        — Shared PC box snapshot; sent once on connect then every 5 s.
/// * `server_bag`        — Shared bag pockets snapshot; sent every 2 s.
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
    server_bag: Arc<Mutex<Option<BagPockets>>>,
    sprite_cache: RomSpriteCache,
    game_loaded: Arc<AtomicBool>,
    run_changed: Arc<AtomicBool>,
    wipe_signal: Arc<AtomicBool>,
    preferred_player: Option<u8>,
    server_warnings: Arc<Mutex<Vec<String>>>,
) {
    tracing::info!(
        "Client connected: {}",
        stream
            .peer_addr()
            .map_or_else(|_| "unknown".to_string(), |a| a.to_string()),
    );

    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to clone stream: {}", e);
            return;
        }
    };
    let write_stream = Arc::new(Mutex::new(stream));

    // Reader thread: responds to texture requests and run commands.
    let write_stream_clone = write_stream.clone();
    let cache_clone = sprite_cache.clone();
    std::thread::spawn(move || {
        let mut read_stream = read_stream;
        loop {
            match recv_message::<ClientMessage>(&mut read_stream) {
                Ok(ClientMessage::RequestTextures(species_list)) => {
                    let rom = fire_red_rom_buffer::get_rom();
                    let mut sprites: Vec<SpriteData> = Vec::new();

                    for species in species_list {
                        if species == 0 || species > MAX_NATIONAL_DEX_FIRERED {
                            continue;
                        }
                        // Send front and back sprites for both palette variants.
                        for shiny in [false, true] {
                            let front_key = (species, shiny, SpriteVariant::Front);
                            // Check the cache under a short-lived lock; release before
                            // calling build_sprite_data so ROM decode doesn't hold the
                            // mutex for the full decode duration.
                            let cached_front =
                                cache_clone.lock_or_recover().get(&front_key).cloned();
                            if let Some(data) = cached_front {
                                sprites.push(data);
                            } else if let Some(data) = build_sprite_data(rom, species, shiny) {
                                cache_clone
                                    .lock_or_recover()
                                    .insert(front_key, data.clone());
                                sprites.push(data);
                            }

                            let back_key = (species, shiny, SpriteVariant::Back);
                            let cached_back = cache_clone.lock_or_recover().get(&back_key).cloned();
                            if let Some(data) = cached_back {
                                sprites.push(data);
                            } else if let Some(data) = build_sprite_data_back(rom, species, shiny) {
                                cache_clone.lock_or_recover().insert(back_key, data.clone());
                                sprites.push(data);
                            }
                        }
                    }

                    // Always send a reply so the aggregator knows this batch is
                    // resolved — even an empty Textures([]) prevents it from
                    // re-queuing the same species on every tick when ROM decode
                    // fails for all requested species.
                    let mut ws = write_stream_clone.lock_or_recover();
                    if send_message(&mut ws, &ServerMessage::Textures(sprites)).is_err() {
                        break;
                    }
                }
                Ok(ClientMessage::Hello(version)) => {
                    tracing::info!("Aggregator v{}", version);
                }
                Ok(ClientMessage::EndRun) => {
                    fire_red_database::end_run();
                    run_changed.store(true, Ordering::Release);
                    let mut ws = write_stream_clone.lock_or_recover();
                    if send_message(&mut ws, &ServerMessage::RunChanged(None)).is_err() {
                        break;
                    }
                }
                Ok(ClientMessage::NewRun) => match fire_red_database::new_run("Unknown") {
                    Ok(id) => {
                        run_changed.store(true, Ordering::Release);
                        let mut ws = write_stream_clone.lock_or_recover();
                        if send_message(&mut ws, &ServerMessage::RunChanged(Some(id))).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::error!("Failed to create new run: {e}"),
                },
                Ok(ClientMessage::GiveItem { item_id, quantity }) => {
                    crate::game::give_item(item_id, quantity);
                }
                Ok(ClientMessage::MakeShiny { party_position }) => {
                    crate::game::make_shiny(party_position as usize);
                }
                Ok(ClientMessage::TakeItem { item_id, quantity }) => {
                    crate::game::take_item(item_id, quantity);
                }
                Ok(ClientMessage::ChangeSpecies {
                    party_position,
                    new_species,
                }) => {
                    crate::game::change_species(party_position as usize, new_species);
                }
                Ok(ClientMessage::ChangeAbility {
                    party_position,
                    ability_slot,
                }) => {
                    crate::game::change_ability(party_position as usize, ability_slot);
                }
                Ok(ClientMessage::ChangeGender {
                    party_position,
                    target_gender,
                }) => {
                    crate::game::change_gender(party_position as usize, target_gender);
                }
                Ok(ClientMessage::ChangeNickname {
                    party_position,
                    nickname,
                }) => {
                    crate::game::change_nickname(party_position as usize, &nickname);
                }
                Ok(ClientMessage::ChangeHeldItem {
                    party_position,
                    item_id,
                }) => {
                    crate::game::change_held_item(party_position as usize, item_id);
                }
                Ok(ClientMessage::CureStatus { party_position }) => {
                    crate::game::cure_status(party_position as usize);
                }
                Ok(ClientMessage::ChangeNature {
                    party_position,
                    target_nature,
                }) => {
                    crate::game::change_nature(party_position as usize, target_nature);
                }
                Ok(ClientMessage::RestorePp { party_position }) => {
                    crate::game::restore_pp(party_position as usize);
                }
                Ok(ClientMessage::SetFriendship {
                    party_position,
                    friendship,
                }) => {
                    crate::game::set_friendship(party_position as usize, friendship);
                }
                Ok(ClientMessage::ChangeMove {
                    party_position,
                    slot,
                    move_id,
                }) => {
                    crate::game::change_move(party_position as usize, slot, move_id);
                }
                Ok(ClientMessage::SetIvs {
                    party_position,
                    hp,
                    atk,
                    def,
                    spd,
                    spa,
                    spdef,
                }) => {
                    crate::game::set_ivs(party_position as usize, hp, atk, def, spd, spa, spdef);
                }
                Ok(ClientMessage::IncreaseIvs {
                    party_position,
                    hp,
                    atk,
                    def,
                    spd,
                    spa,
                    spdef,
                }) => {
                    crate::game::increase_ivs(
                        party_position as usize,
                        hp,
                        atk,
                        def,
                        spd,
                        spa,
                        spdef,
                    );
                }
                Ok(ClientMessage::SetEvs {
                    party_position,
                    hp,
                    atk,
                    def,
                    spd,
                    spa,
                    spdef,
                }) => {
                    crate::game::set_evs(party_position as usize, hp, atk, def, spd, spa, spdef);
                }
                Ok(ClientMessage::IncreaseEvs {
                    party_position,
                    hp,
                    atk,
                    def,
                    spd,
                    spa,
                    spdef,
                }) => {
                    crate::game::increase_evs(
                        party_position as usize,
                        hp,
                        atk,
                        def,
                        spd,
                        spa,
                        spdef,
                    );
                }
                Ok(ClientMessage::RestoreHp { party_position }) => {
                    crate::game::restore_hp(party_position as usize);
                }
                Ok(ClientMessage::HealParty) => {
                    crate::game::heal_party();
                }
                Ok(ClientMessage::SetExp {
                    party_position,
                    exp,
                }) => {
                    crate::game::set_exp(party_position as usize, exp);
                }
                Ok(ClientMessage::SetLevel {
                    party_position,
                    level,
                }) => {
                    crate::game::set_level(party_position as usize, level);
                }
                Ok(ClientMessage::LearnMove {
                    party_position,
                    move_id,
                }) => {
                    crate::game::learn_move(party_position as usize, move_id);
                }
                Ok(ClientMessage::ForgetMove {
                    party_position,
                    slot,
                }) => {
                    crate::game::forget_move(party_position as usize, slot);
                }
                Ok(ClientMessage::SetPokerus { party_position }) => {
                    crate::game::set_pokerus(party_position as usize);
                }
                Err(_) => break,
            }
        }
    });

    // Writer loop: pushes GameState every 100 ms, BoxData every 5 s, Bag every 2 s.
    let mut last_box_send = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(10))
        .unwrap_or_else(std::time::Instant::now);
    let mut last_bag_send = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(10))
        .unwrap_or_else(std::time::Instant::now);

    loop {
        // Read the true player position and trainer name before acquiring any
        // shared locks. get_value() acquires the STATE mutex; if it were called
        // while holding server_encounters it would block the game thread for up
        // to SLEEP_DURATION (333 ms) every tick.
        let pos = fire_red_loop::get_value();
        let player_name = fire_red_loop::get_trainer_name();
        let badge_state = if game_loaded.load(Ordering::Acquire) {
            fire_red_badge::read_badge_state()
        } else {
            None
        };

        let state = {
            let party = server_party.lock_or_recover();
            let encounters = server_encounters.lock_or_recover();

            // Only resolve a zone name when the encounter header actually contains
            // pokemon — the default WildPokemonHeader (map_group=0, map_num=0)
            // is returned for all non-encounter maps and must not be looked up.
            let has_encounters = !encounters.land_mon_encounters.wild_pokemon_list.is_empty()
                || !encounters.water_mon_encounters.wild_pokemon_list.is_empty()
                || !encounters
                    .rock_smash_encounters
                    .wild_pokemon_list
                    .is_empty()
                || !encounters.fishing_encounters.wild_pokemon_list.is_empty();

            let zone_name = if has_encounters {
                let name =
                    fire_red_loop::get_area_name_for(encounters.map_group, encounters.map_num);
                if !name.is_empty() {
                    name.to_string()
                } else {
                    format!("{}\u{00B7}{}", encounters.map_group, encounters.map_num)
                }
            } else {
                String::new()
            };

            GameState {
                party: party.clone(),
                encounters: encounters.clone(),
                player_name,
                badge_state,
                zone_name,
                current_map_group: pos.map_group_id,
                current_map_name: pos.map_name_id,
                preferred_player,
                warnings: server_warnings.lock_or_recover().drain(..).collect(),
            }
        };

        let mut ws = write_stream.lock_or_recover();
        if send_message(&mut ws, &ServerMessage::State(Box::new(state))).is_err() {
            tracing::info!("Client disconnected.");
            break;
        }

        // Load without consuming; only clear after confirmed delivery so a
        // disconnect mid-send cannot permanently drop the wipe notification.
        if wipe_signal.load(Ordering::Acquire) {
            if send_message(&mut ws, &ServerMessage::RunChanged(None)).is_err() {
                tracing::info!("Client disconnected.");
                break;
            }
            wipe_signal.store(false, Ordering::Release);
        }

        // Send box snapshot on first tick and every 5 seconds thereafter.
        if last_box_send.elapsed() >= std::time::Duration::from_secs(5) {
            let entries = server_box.lock_or_recover().clone();
            if send_message(&mut ws, &ServerMessage::BoxData(entries)).is_err() {
                tracing::info!("Client disconnected.");
                break;
            }
            last_box_send = std::time::Instant::now();
        }

        // Send bag pockets every 2 seconds if data is available.
        if last_bag_send.elapsed() >= std::time::Duration::from_secs(2) {
            if let Some(pockets) = server_bag.lock_or_recover().clone()
                && send_message(&mut ws, &ServerMessage::Bag(pockets)).is_err()
            {
                tracing::info!("Client disconnected.");
                break;
            }
            last_bag_send = std::time::Instant::now();
        }

        drop(ws);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
