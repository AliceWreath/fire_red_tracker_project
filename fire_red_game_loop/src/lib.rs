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
    /// If set, this run ID is installed as the thread-local active run at the
    /// start of the game-loop thread.  Used by direct-mode slots so each slot
    /// writes to its own run without touching the global `DbState.run_id`.
    pub thread_run_id: Option<u32>,
    /// If set, the game loop exits when this flag becomes `true`.
    /// Used by direct-mode `DirectConnector::disconnect` to stop a slot.
    pub shutdown: Option<Arc<AtomicBool>>,
    /// Per-user OBS/webhook config for this direct-mode slot. When set,
    /// `spawn_game_loop` calls `webhook::init_for_thread` so that OBS events
    /// use this user's config instead of the global config-file settings.
    pub obs_config: Option<(crate::config::WebhookConfig, crate::config::ObsConfig)>,
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
    /// Cached badge state written by the game-loop thread (which has the
    /// per-connection MemoryContext).  Read by `assemble_game_state` from the
    /// BroadcastLoop thread, which has no MemoryContext.
    pub badge_state: Arc<Mutex<Option<fire_red_badge::BadgeState>>>,
    /// Cached money value, same reasoning as `badge_state`.
    pub money: Arc<Mutex<u32>>,
    /// Cached player name, same reasoning as `badge_state`.
    pub player_name: Arc<Mutex<String>>,
    /// Cached play-time (hours, minutes, seconds), same reasoning as `badge_state`.
    pub play_time: Arc<Mutex<(u16, u8, u8)>>,
    /// Cached current map (group_id, name_id), same reasoning as `badge_state`.
    pub current_map: Arc<Mutex<(u8, u8)>>,
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
            badge_state: Arc::new(Mutex::new(None)),
            money: Arc::new(Mutex::new(0)),
            player_name: Arc::new(Mutex::new(String::new())),
            play_time: Arc::new(Mutex::new((0, 0, 0))),
            current_map: Arc::new(Mutex::new((0, 0))),
        }
    }
}

impl Default for GameLoopState {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Box data helper
// ---------------------------------------------------------------------------

/// Returns the most significant status name from a Gen III status bitmask.
///
/// Bit 3=PSN, 4=BRN, 5=FRZ, 6=PAR, 7=TOX; bits 0-2=sleep turn counter.
fn status_name_for(status: u32) -> &'static str {
    if status & (1 << 6) != 0 { return "PAR"; }
    if status & (1 << 5) != 0 { return "FRZ"; }
    if status & (1 << 4) != 0 { return "BRN"; }
    if status & (1 << 7) != 0 { return "TOX"; }
    if status & (1 << 3) != 0 { return "PSN"; }
    if status & 0b111 != 0    { return "SLP"; }
    "OK"
}

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
fn process_commands(command_queue: &Arc<Mutex<VecDeque<ClientMessage>>>, run_changed: &Arc<AtomicBool>) {
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
                    run_changed.store(true, Ordering::Release);
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
    let thread_party        = state.party.clone();
    let thread_encounters   = state.encounters.clone();
    let thread_box          = state.box_entries.clone();
    let thread_bag          = state.bag.clone();
    let thread_loaded       = state.game_loaded.clone();
    let thread_run_chg      = state.run_changed.clone();
    let thread_wipe         = state.wipe_signal.clone();
    let thread_warnings     = state.warnings.clone();
    let thread_cmds         = state.command_queue.clone();
    let thread_badge_state  = state.badge_state.clone();
    let thread_money        = state.money.clone();
    let thread_player_name  = state.player_name.clone();
    let thread_play_time    = state.play_time.clone();
    let thread_current_map  = state.current_map.clone();

    std::thread::spawn(move || {
        fire_red_retroarch_interfacing::set_thread_addr(&cfg.retroarch_host, cfg.retroarch_port);
        if let Some(run_id) = cfg.thread_run_id {
            fire_red_database::set_thread_run_id(run_id);
        }
        if let Some((wh_cfg, obs_cfg)) = cfg.obs_config {
            crate::webhook::init_for_thread(wh_cfg, obs_cfg);
        }
        let thread_shutdown = cfg.shutdown.clone();
        use fire_red_loop::*;

        // Create per-connection contexts and register them as thread-locals so
        // get_ewram(), get_party(), get_static_trainer_data() etc. return data
        // from this connection's RetroArch instance, not the global singletons.
        let mem_ctx     = fire_red_memory::MemoryContext::new();
        let party_ctx   = fire_red_party_monitor::PartyContext::new();
        let trainer_ctx = fire_red_trainer_data::TrainerContext::new();
        fire_red_memory::set_thread_memory_context(mem_ctx.clone());
        fire_red_party_monitor::set_thread_party_context(party_ctx.clone());
        fire_red_trainer_data::set_thread_trainer_context(trainer_ctx.clone());

        let box_running = match start_loop_ctx(
            cfg.rom_path.as_str(),
            cfg.is_clean,
            mem_ctx.clone(),
            party_ctx.clone(),
            trainer_ctx.clone(),
        ) {
            Ok(br) => { tracing::info!("Per-connection loop started."); br }
            Err(code) => {
                tracing::error!(
                    "Failed to start per-connection loop (code {}). Slot will not poll.",
                    code
                );
                return;
            }
        };

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
        *thread_current_map.lock_or_recover() = (initial_state.map_group_id, initial_state.map_name_id);

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
        let mut last_pp: HashMap<u32, [u8; 4]> = HashMap::new();
        let mut last_friendship: HashMap<u32, u8> = HashMap::new();
        let mut last_status: HashMap<u32, u32> = HashMap::new();
        let mut last_enemy_personality: u32 = 0;
        let mut last_enemy_hp: u16  = 0;
        let mut last_enemy_max_hp: u16 = 0;
        let mut enemy_warmed_up    = false;
        let mut last_area_visit_id: Option<i64> = None;

        let mut player_name_set = {
            let name = get_trainer_name();
            if !name.trim().is_empty() {
                fire_red_database::set_thread_player_name(&name);
                *thread_player_name.lock_or_recover() = name;
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
            if let Some(ref sd) = thread_shutdown
                && sd.load(Ordering::Acquire) { break; }

            // Process injection commands first so they're applied on next read.
            process_commands(&thread_cmds, &thread_run_chg);

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
                *thread_current_map.lock_or_recover() = (0, 0);
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
                    fire_red_database::set_thread_player_name(&name);
                    *thread_player_name.lock_or_recover() = name;
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
                *thread_current_map.lock_or_recover() = (current_state.map_group_id, current_state.map_name_id);
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
                *thread_current_map.lock_or_recover() = (current_state.map_group_id, current_state.map_name_id);
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

            // PP-delta move tracking and friendship change logging.
            for mon in thread_party.lock_or_recover().iter() {
                let personality = mon.box_mon.personality;
                if personality == 0 || mon.box_mon.secure.growth.species == 0 {
                    continue;
                }
                let cur_pp = mon.box_mon.secure.attack.pp;
                let moves  = mon.box_mon.secure.attack.moves;
                if let Some(&prev_pp) = last_pp.get(&personality) {
                    for slot in 0..4usize {
                        let move_id = moves[slot];
                        if move_id == 0 { continue; }
                        if cur_pp[slot] < prev_pp[slot] {
                            let uses = (prev_pp[slot] - cur_pp[slot]) as i32;
                            fire_red_database::log_move_use(
                                &fire_red_loop::get_trainer_name(),
                                personality,
                                slot as u8,
                                move_id,
                                fire_red_database::move_name(move_id),
                                uses,
                            );
                        }
                    }
                }
                last_pp.insert(personality, cur_pp);

                let cur_friendship = mon.box_mon.secure.growth.friendship;
                let prev_friendship = last_friendship.get(&personality).copied();
                if prev_friendship != Some(cur_friendship) {
                    let is_first = prev_friendship.is_none();
                    let crossed_220 = !is_first
                        && prev_friendship.is_some_and(|p| p < 220)
                        && cur_friendship >= 220;
                    fire_red_database::log_friendship(
                        &fire_red_loop::get_trainer_name(),
                        personality,
                        &mon.box_mon.nickname_string,
                        &mon.box_mon.secure.growth.species_string,
                        cur_friendship,
                    );
                    if crossed_220 {
                        crate::webhook::fire_event(crate::webhook::WebhookEvent::FriendshipThreshold {
                            player: fire_red_loop::get_trainer_name(),
                            timestamp: fire_red_database::unix_now(),
                            nickname: mon.box_mon.nickname_string.clone(),
                            species: mon.box_mon.secure.growth.species_string.clone(),
                            friendship: cur_friendship,
                        });
                    }
                    last_friendship.insert(personality, cur_friendship);
                }

                // Status condition onset / clear detection.
                let cur_status = mon.status;
                let prev_status = last_status.get(&personality).copied().unwrap_or(0);
                if prev_status != cur_status {
                    let status_name = status_name_for(cur_status.max(prev_status));
                    let event_type  = if cur_status == 0 { "clear" } else { "onset" };
                    fire_red_database::log_status_event(
                        &fire_red_loop::get_trainer_name(),
                        personality,
                        &mon.box_mon.nickname_string,
                        &mon.box_mon.secure.growth.species_string,
                        status_name,
                        cur_status,
                        event_type,
                    );
                    last_status.insert(personality, cur_status);
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
                last_pp.clear();
                last_friendship.clear();
                last_status.clear();
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
                let (new_mask, cached_bs) = game::check_for_new_badges(
                    last_badge_mask,
                    cfg.livesplit_split_on_badges,
                    cfg.livesplit_split_on_clear,
                    &thread_party,
                );
                last_badge_mask = new_mask;
                if let Some(bs) = cached_bs {
                    *thread_badge_state.lock_or_recover() = Some(bs);
                }
                *thread_money.lock_or_recover() = game::read_money();
                *thread_play_time.lock_or_recover() = fire_red_loop::get_play_time_components();
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

        // Stop this connection's subsystem threads before the game-loop thread
        // exits so no background thread outlives its MemoryContext.
        stop_loop_ctx(&mem_ctx, &party_ctx, &trainer_ctx, &box_running);
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
    let player_name = state.player_name.lock_or_recover().clone();
    let (map_group_id, map_name_id) = *state.current_map.lock_or_recover();

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

    let badge_state = state.badge_state.lock_or_recover().clone();
    let money = *state.money.lock_or_recover();
    let (play_time_hours, play_time_minutes, play_time_seconds) =
        *state.play_time.lock_or_recover();

    GameState {
        party,
        encounters,
        player_name,
        badge_state,
        zone_name,
        current_map_group: map_group_id,
        current_map_name:  map_name_id,
        preferred_player,
        warnings,
        money,
        play_time_hours,
        play_time_minutes,
        play_time_seconds,
    }
}
