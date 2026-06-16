//! # fire_red_game_loop
//!
//! Shared game-polling logic for both the tracker (local GUI / connected mode)
//! and the aggregator (direct mode, where the aggregator polls RetroArch on
//! behalf of a remote gaming machine).
//!
//! # Usage
//!
//! 1. Optionally call [`fire_red_retroarch_interfacing::init`] with the host
//!    and port of the RetroArch instance (defaults to `127.0.0.1:55355`).
//! 2. Build a [`GameLoopConfig`] and a [`GameLoopState`].
//! 3. Call [`spawn_game_loop`]; it spawns the polling thread and returns.
//! 4. Read game state from the arcs in [`GameLoopState`].  Write injection
//!    commands into `command_queue` to have them executed on the next tick.

pub mod config;
pub mod discord;
pub mod encounter;
pub mod game;
pub mod helix;
pub mod livesplit;
pub mod webhook;

use fire_red_states::{BagPockets, BoxEntry, ClientMessage, GameState, LockOrRecover};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for the game-polling loop.
pub struct GameLoopConfig {
    /// Hostname or IP of the RetroArch instance to poll.
    /// Defaults to `"127.0.0.1"` (same machine as the tracker).
    pub retroarch_host: String,
    /// RetroArch network-commands UDP port (default 55355).
    pub retroarch_port: u16,
    /// Path to the FireRed ROM file on the local machine.
    pub rom_path: String,
    /// Whether to wipe the database on startup (`--clean`).
    pub is_clean: bool,
    /// How the dupes clause is applied to wild encounters.
    pub dupes_clause: config::DupesClauseMode,
    /// Allow the same species on multiple routes (randomiser mode).
    pub allow_species_repeats: bool,
    /// Minimum Pokéball count before run tracking starts.
    pub run_start_balls: u32,
    /// Preferred display slot sent to the aggregator on every tick.
    pub preferred_player: Option<u8>,
    /// Game-polling interval in ms, shared with config hot-reload.
    pub poll_ms: Arc<AtomicU64>,
    /// Send a LiveSplit split when a badge is earned.
    pub livesplit_split_on_badges: bool,
    /// Send a LiveSplit split when the Champion is defeated.
    pub livesplit_split_on_clear: bool,
}

/// Shared state written by the game-polling loop and read by the caller.
///
/// In tracker mode the caller is `server.rs` (sends over TCP).
/// In aggregator direct mode the caller is `direct.rs` (writes to a
/// [`MonitorSlot`] directly).
pub struct GameLoopState {
    /// Current party Pokémon.
    pub party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    /// Wild-encounter table for the current area.
    pub encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    /// PC box snapshot (updated on party-size change).
    pub box_entries: Arc<Mutex<Vec<BoxEntry>>>,
    /// Bag pockets snapshot (updated every ~2 s).
    pub bag: Arc<Mutex<Option<BagPockets>>>,
    /// True while a FireRed save is actively loaded in RetroArch.
    pub game_loaded: Arc<AtomicBool>,
    /// Toggled when `EndRun`/`NewRun` is processed; caller resets to `false`.
    pub run_changed: Arc<AtomicBool>,
    /// Set when a party wipe ends the run.
    pub wipe_signal: Arc<AtomicBool>,
    /// One-shot Nuzlocke-clause warnings drained after each broadcast.
    pub warnings: Arc<Mutex<Vec<String>>>,
    /// Injection commands to execute on the next tick
    /// (e.g. `ClientMessage::GiveItem { … }`).
    pub command_queue: Arc<Mutex<VecDeque<ClientMessage>>>,
}

impl GameLoopState {
    pub fn new() -> Self {
        Self {
            party: Arc::new(Mutex::new(Vec::new())),
            encounters: Arc::new(Mutex::new(
                fire_red_pokemon_data::WildPokemonHeader::default(),
            )),
            box_entries: Arc::new(Mutex::new(Vec::new())),
            bag: Arc::new(Mutex::new(None)),
            game_loaded: Arc::new(AtomicBool::new(false)),
            run_changed: Arc::new(AtomicBool::new(false)),
            wipe_signal: Arc::new(AtomicBool::new(false)),
            warnings: Arc::new(Mutex::new(Vec::new())),
            command_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

impl Default for GameLoopState {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Box data helper
// ---------------------------------------------------------------------------

fn build_box_entries() -> Vec<BoxEntry> {
    fire_red_box_monitor::get_box_entries_positioned()
        .into_iter()
        .map(|(box_idx, slot_idx, mon)| {
            let personality = mon.personality;
            let ot_id = mon.ot_id;
            let iv = &mon.secure.misc.iv_egg_ability;
            BoxEntry {
                box_index: box_idx,
                slot_index: slot_idx,
                species: mon.secure.growth.species,
                species_name: mon.secure.growth.species_string.clone(),
                nickname: mon.nickname_string.clone(),
                personality,
                ot_id,
                is_shiny: game::is_shiny(personality, ot_id),
                nature: fire_red_database::nature_name(personality).to_string(),
                iv_hp: iv.hp_iv,
                iv_atk: iv.attack_iv,
                iv_def: iv.defense_iv,
                iv_spe: iv.speed_iv,
                iv_spa: iv.sp_attack_iv,
                iv_spd: iv.sp_def_iv,
                is_egg: iv.egg != 0,
                gender: mon.gender,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Party-event helper
// ---------------------------------------------------------------------------

fn handle_party_events(
    thread_party: &Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    enc_tracker: &mut encounter::EncounterTracker,
    thread_wipe_signal: &Arc<AtomicBool>,
) -> bool {
    game::check_for_new_pokemon(thread_party);
    game::check_for_dead_pokemon(thread_party, enc_tracker.run_tracking_active());
    if game::check_for_run_over(thread_party, enc_tracker.run_tracking_active()) {
        enc_tracker.mark_wipe();
        thread_wipe_signal.store(true, Ordering::Release);
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Command processor
// ---------------------------------------------------------------------------

/// Drains and executes any pending injection commands from the queue.
fn process_commands(command_queue: &Arc<Mutex<VecDeque<ClientMessage>>>) {
    let cmds: Vec<ClientMessage> = {
        let mut q = command_queue.lock_or_recover();
        q.drain(..).collect()
    };
    for cmd in cmds {
        match cmd {
            ClientMessage::GiveItem { item_id, quantity } => { game::give_item(item_id, quantity); }
            ClientMessage::TakeItem { item_id, quantity } => { game::take_item(item_id, quantity); }
            ClientMessage::MakeShiny { party_position } => { game::make_shiny(party_position as usize); }
            ClientMessage::ChangeSpecies { party_position, new_species } => { game::change_species(party_position as usize, new_species); }
            ClientMessage::ChangeAbility { party_position, ability_slot } => { game::change_ability(party_position as usize, ability_slot); }
            ClientMessage::ChangeGender { party_position, target_gender } => { game::change_gender(party_position as usize, target_gender); }
            ClientMessage::ChangeNickname { party_position, nickname } => { game::change_nickname(party_position as usize, &nickname); }
            ClientMessage::ChangeHeldItem { party_position, item_id } => { game::change_held_item(party_position as usize, item_id); }
            ClientMessage::CureStatus { party_position } => { game::cure_status(party_position as usize); }
            ClientMessage::ChangeNature { party_position, target_nature } => { game::change_nature(party_position as usize, target_nature); }
            ClientMessage::RestorePp { party_position } => { game::restore_pp(party_position as usize); }
            ClientMessage::SetFriendship { party_position, friendship } => { game::set_friendship(party_position as usize, friendship); }
            ClientMessage::ChangeMove { party_position, slot, move_id } => { game::change_move(party_position as usize, slot, move_id); }
            ClientMessage::SetIvs { party_position, hp, atk, def, spd, spa, spdef } => { game::set_ivs(party_position as usize, hp, atk, def, spd, spa, spdef); }
            ClientMessage::IncreaseIvs { party_position, hp, atk, def, spd, spa, spdef } => { game::increase_ivs(party_position as usize, hp, atk, def, spd, spa, spdef); }
            ClientMessage::SetEvs { party_position, hp, atk, def, spd, spa, spdef } => { game::set_evs(party_position as usize, hp, atk, def, spd, spa, spdef); }
            ClientMessage::IncreaseEvs { party_position, hp, atk, def, spd, spa, spdef } => { game::increase_evs(party_position as usize, hp, atk, def, spd, spa, spdef); }
            ClientMessage::RestoreHp { party_position } => { game::restore_hp(party_position as usize); }
            ClientMessage::HealParty => { game::heal_party(); }
            ClientMessage::SetExp { party_position, exp } => { game::set_exp(party_position as usize, exp); }
            ClientMessage::SetLevel { party_position, level } => { game::set_level(party_position as usize, level); }
            ClientMessage::LearnMove { party_position, move_id } => { game::learn_move(party_position as usize, move_id); }
            ClientMessage::ForgetMove { party_position, slot } => { game::forget_move(party_position as usize, slot); }
            ClientMessage::SetPokerus { party_position } => { game::set_pokerus(party_position as usize); }
            ClientMessage::SetPpUps { party_position, pp0, pp1, pp2, pp3 } => { game::set_pp_ups(party_position as usize, pp0, pp1, pp2, pp3); }
            ClientMessage::RevivePokemon { party_position, personality } => { game::revive_pokemon(party_position as usize, personality); }
            ClientMessage::UndoLastCommand => { game::undo_last_command(); }
            ClientMessage::EndRun => {
                fire_red_database::end_run();
            }
            ClientMessage::NewRun => {
                if let Ok(id) = fire_red_database::new_run("Unknown") {
                    tracing::info!("New run #{} started via command queue.", id);
                }
            }
            ClientMessage::RequestTextures(_) | ClientMessage::Hello(_) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawns the game-polling thread and returns immediately.
///
/// The thread runs until the process exits.  All state is written into the
/// arcs in `state`; callers read from those arcs on their own schedule.
///
/// The RetroArch address is taken from [`GameLoopConfig::retroarch_host`] and
/// [`GameLoopConfig::retroarch_port`] and set as a thread-local inside the
/// spawned thread, so multiple concurrent threads can each poll a different host.
pub fn spawn_game_loop(
    cfg: GameLoopConfig,
    state: Arc<GameLoopState>,
) -> std::thread::JoinHandle<()> {
    let thread_party     = state.party.clone();
    let thread_encounters= state.encounters.clone();
    let thread_box       = state.box_entries.clone();
    let thread_bag       = state.bag.clone();
    let thread_loaded    = state.game_loaded.clone();
    let thread_run_chg   = state.run_changed.clone();
    let thread_wipe      = state.wipe_signal.clone();
    let thread_warnings  = state.warnings.clone();
    let thread_cmds      = state.command_queue.clone();

    std::thread::spawn(move || {
        fire_red_retroarch_interfacing::set_thread_addr(&cfg.retroarch_host, cfg.retroarch_port);
        use fire_red_loop::*;

        match start_loop(cfg.rom_path.as_str(), cfg.is_clean) {
            0 => tracing::info!("Monitor loop started."),
            code => {
                tracing::error!("Failed to start monitor loop (code {}).", code);
                std::process::exit(1);
            }
        }

        tracing::info!("Waiting for initial map state...");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if game::map_state_from_ewram().is_some() { break; }
            if std::time::Instant::now() > deadline {
                tracing::warn!("Map state did not populate within 5 seconds.");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let initial_state = game::map_state_from_ewram().unwrap_or(FireRedState {
            map_group_id: 0,
            map_name_id: 0,
        });
        *thread_encounters.lock_or_recover() = get_area_pokemon_id_for_state(&initial_state);

        let mut current_state = initial_state;
        let mut old_party_size = get_party_size();
        let mut last_party_refresh = std::time::Instant::now();
        let mut last_bag_refresh   = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(10))
            .unwrap_or_else(std::time::Instant::now);
        let mut state_initialized  = false;
        let mut enc_tracker        = encounter::EncounterTracker::new();
        let mut last_badge_mask: Option<u8>    = None;
        let mut last_trainer_flags: Option<Vec<u8>> = None;
        let mut last_player_name   = String::new();
        let mut last_party_hp: HashMap<u32, u16> = HashMap::new();
        let mut last_enemy_personality: u32 = 0;
        let mut last_enemy_hp: u16  = 0;
        let mut last_enemy_max_hp: u16 = 0;
        let mut enemy_warmed_up    = false;
        let mut last_area_visit_id: Option<i64> = None;

        let mut player_name_set = {
            let name = get_trainer_name();
            if !name.trim().is_empty() {
                fire_red_database::set_player_name(&name);
                true
            } else {
                false
            }
        };

        enc_tracker.seed_from_db();
        game::fill_party_list(&thread_party);
        if handle_party_events(&thread_party, &mut enc_tracker, &thread_wipe) {
            last_badge_mask = None;
        }

        loop {
            // Process injection commands first so they're applied on next read.
            process_commands(&thread_cmds);

            if !game::game_is_loaded() {
                thread_loaded.store(false, Ordering::Release);
                *thread_encounters.lock_or_recover() =
                    fire_red_pokemon_data::WildPokemonHeader::default();
                *thread_party.lock_or_recover() = Vec::new();
                state_initialized  = false;
                player_name_set    = false;
                last_badge_mask    = None;
                last_trainer_flags = None;
                current_state      = FireRedState { map_group_id: 0xFF, map_name_id: 0xFF };
                enc_tracker.reset();
                last_party_hp.clear();
                last_enemy_personality = 0;
                enemy_warmed_up = false;
                if let Some(vid) = last_area_visit_id.take() {
                    fire_red_database::close_area_visit(vid, fire_red_database::unix_now());
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
            thread_loaded.store(true, Ordering::Release);

            if !player_name_set {
                let name = get_trainer_name();
                if !name.trim().is_empty() {
                    if !last_player_name.is_empty() && name != last_player_name {
                        tracing::warn!(
                            "player name changed from '{}' to '{}' after reload — \
                             possible save-file switch.",
                            last_player_name, name
                        );
                    }
                    last_player_name = name.clone();
                    fire_red_database::set_player_name(&name);
                    player_name_set = true;
                    enc_tracker.seed_from_db();
                }
            }

            let state = game::map_state_from_ewram().unwrap_or(current_state);
            let party_size = get_party_size();

            if !state_initialized && (state.map_group_id != 0 || state.map_name_id != 0) {
                state_initialized = true;
                current_state = state;
                *thread_encounters.lock_or_recover() = get_area_pokemon_id_for_state(&current_state);
                let zone = get_area_name_for(current_state.map_group_id, current_state.map_name_id);
                last_area_visit_id = fire_red_database::open_area_visit(
                    current_state.map_group_id,
                    current_state.map_name_id,
                    zone,
                    fire_red_database::unix_now(),
                );
            }

            if state_initialized && current_state != state {
                current_state = state;
                *thread_encounters.lock_or_recover() = get_area_pokemon_id_for_state(&current_state);
                let zone = get_area_name_for(current_state.map_group_id, current_state.map_name_id);
                let now = fire_red_database::unix_now();
                if let Some(vid) = last_area_visit_id.take() {
                    fire_red_database::close_area_visit(vid, now);
                }
                last_area_visit_id = fire_red_database::open_area_visit(
                    current_state.map_group_id,
                    current_state.map_name_id,
                    zone,
                    now,
                );
                discord::update(discord::Presence {
                    details: if zone.is_empty() {
                        format!("{}\u{B7}{}", current_state.map_group_id, current_state.map_name_id)
                    } else {
                        zone.to_string()
                    },
                    state: format!("Party: {}", party_size),
                    large_image: "pokeball",
                    large_text: get_trainer_name(),
                });
            }

            game::fill_party_list(&thread_party);

            for mon in thread_party.lock_or_recover().iter() {
                let personality = mon.box_mon.personality;
                let hp = mon.hp;
                let max_hp = mon.max_hp;
                if personality != 0 && hp > 0 && max_hp > 0 {
                    fire_red_database::update_min_hp_seen(personality, hp, max_hp);
                    let changed = last_party_hp.get(&personality).is_none_or(|&last| last != hp);
                    if changed {
                        fire_red_database::record_hp_observation(personality, hp, max_hp);
                        last_party_hp.insert(personality, hp);
                    }
                }
            }

            if let Some((enemy_p, enemy_hp, enemy_max_hp)) = game::read_enemy_slot0_raw() {
                if !enemy_warmed_up {
                    last_enemy_personality = enemy_p;
                    enemy_warmed_up = true;
                } else if enemy_p != last_enemy_personality {
                    if last_enemy_personality != 0 {
                        fire_red_database::record_enemy_hp(
                            last_enemy_personality, last_enemy_hp, last_enemy_max_hp, "final",
                        );
                    }
                    fire_red_database::record_enemy_hp(enemy_p, enemy_hp, enemy_max_hp, "initial");
                    last_enemy_personality = enemy_p;
                }
                last_enemy_hp     = enemy_hp;
                last_enemy_max_hp = enemy_max_hp;
            }

            if old_party_size != party_size {
                old_party_size = party_size;
                fire_red_box_monitor::update_box_list();
                *thread_box.lock_or_recover() = build_box_entries();
                if handle_party_events(&thread_party, &mut enc_tracker, &thread_wipe) {
                    last_badge_mask = None;
                }
                if state_initialized {
                    let zone = get_area_name_for(
                        current_state.map_group_id, current_state.map_name_id,
                    );
                    discord::update(discord::Presence {
                        details: if zone.is_empty() {
                            format!("{}\u{B7}{}", current_state.map_group_id, current_state.map_name_id)
                        } else {
                            zone.to_string()
                        },
                        state: format!("Party: {}", party_size),
                        large_image: "pokeball",
                        large_text: get_trainer_name(),
                    });
                }
            }

            if last_party_refresh.elapsed().as_secs() >= 1 {
                last_party_refresh = std::time::Instant::now();
                if handle_party_events(&thread_party, &mut enc_tracker, &thread_wipe) {
                    last_badge_mask = None;
                }
            }

            if thread_run_chg.swap(false, Ordering::AcqRel) {
                enc_tracker.reset();
                last_badge_mask    = None;
                last_trainer_flags = None;
                player_name_set    = false;
                if let Some(vid) = last_area_visit_id.take() {
                    fire_red_database::close_area_visit(vid, fire_red_database::unix_now());
                }
            }

            if state_initialized {
                enc_tracker.tick(
                    current_state,
                    &thread_party,
                    cfg.dupes_clause,
                    cfg.allow_species_repeats,
                    cfg.run_start_balls,
                );
                let drained = enc_tracker.drain_warnings();
                if !drained.is_empty() {
                    thread_warnings.lock_or_recover().extend(drained);
                }
                last_badge_mask = game::check_for_new_badges(
                    last_badge_mask,
                    cfg.livesplit_split_on_badges,
                    cfg.livesplit_split_on_clear,
                );
                last_trainer_flags = game::check_for_new_trainer_battles(last_trainer_flags);
            }

            if last_bag_refresh.elapsed() >= std::time::Duration::from_secs(2) {
                *thread_bag.lock_or_recover() = game::read_bag_pockets();
                last_bag_refresh = std::time::Instant::now();
            }

            std::thread::sleep(std::time::Duration::from_millis(
                cfg.poll_ms.load(Ordering::Relaxed),
            ));
        }
    })
}

// ---------------------------------------------------------------------------
// Direct-mode GameState assembler
// ---------------------------------------------------------------------------

/// Assembles a [`GameState`] snapshot from the current arc values.
///
/// Called by aggregator direct mode every ~100 ms to populate a `MonitorSlot`.
pub fn assemble_game_state(
    state: &GameLoopState,
    preferred_player: Option<u8>,
) -> GameState {
    use fire_red_loop::get_value;

    let pos = get_value();
    let player_name = fire_red_loop::get_trainer_name();
    let badge_state = if state.game_loaded.load(Ordering::Acquire) {
        fire_red_badge::read_badge_state()
    } else {
        None
    };

    let party     = state.party.lock_or_recover().clone();
    let encounters = state.encounters.lock_or_recover().clone();

    let has_encounters = !encounters.land_mon_encounters.wild_pokemon_list.is_empty()
        || !encounters.water_mon_encounters.wild_pokemon_list.is_empty()
        || !encounters.rock_smash_encounters.wild_pokemon_list.is_empty()
        || !encounters.fishing_encounters.wild_pokemon_list.is_empty();

    let zone_name = if has_encounters {
        let name = fire_red_loop::get_area_name_for(encounters.map_group, encounters.map_num);
        if !name.is_empty() {
            name.to_string()
        } else {
            format!("{}\u{00B7}{}", encounters.map_group, encounters.map_num)
        }
    } else {
        String::new()
    };

    let warnings: Vec<String> = state.warnings.lock_or_recover().drain(..).collect();

    let money = crate::game::read_money();
    let (play_time_hours, play_time_minutes, play_time_seconds) =
        fire_red_loop::get_play_time_components();

    GameState {
        party,
        encounters,
        player_name,
        badge_state,
        zone_name,
        current_map_group: pos.map_group_id,
        current_map_name:  pos.map_name_id,
        preferred_player,
        warnings,
        money,
        play_time_hours,
        play_time_minutes,
        play_time_seconds,
    }
}
