//! WebSocket overlay server mode.
//!
//! Activated via `--ws-port PORT`. Runs a headless HTTP + WebSocket server
//! instead of the egui window. OBS connects to it as a Browser Source.
//!
//! - `GET /`   — serves the HTML overlay page
//! - `GET /ws` — WebSocket endpoint; pushes JSON state ~10/s
//!
//! Sprites are embedded directly in the JSON as base64 PNG data URIs, so no
//! separate HTTP sprite endpoint or browser caching issues exist.

use crate::app::sort_gifts_by_caught_at;
use crate::client::{MonitorSlot, PngSpriteCache, SharedSlots, encode_png};
use axum::{
    Extension, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse},
    routing::{delete, get, patch, post},
};
use fire_red_database::{CaughtPokemon, DeadPokemon, User};
use fire_red_states::{
    ClientMessage, GameState, LockOrRecover, MAX_NATIONAL_DEX_FIRERED, is_shiny,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

// Base64 encoding delegated to the shared implementation in fire_red_states.
use fire_red_states::base64_encode;

// ---------------------------------------------------------------------------
// JSON DTOs
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, Clone)]
struct RunSummaryDto {
    run_id: u32,
    player_name: String,
    started_at: String,
    ended_at: Option<String>,
    deaths: usize,
    caught: usize,
}

#[derive(serde::Serialize, Clone)]
struct DbEncounterDto {
    species_name: String,
    level: u8,
    caught: bool,
    is_shiny: bool,
    encountered_at: String,
    area: String,
    sprite: Option<String>,
    map_group: u8,
    map_name: u8,
}

#[derive(serde::Serialize, Clone)]
struct SlotDto {
    label: String,
    connected: bool,
    db_connected: bool,
    active_run_id: Option<u32>,
    run_summary: Option<RunSummaryDto>,
    db_encounters: Vec<DbEncounterDto>,
    badges: Vec<bool>,
    next_gym: Option<GymDto>,
    party: Vec<MemberDto>,
    encounters: Vec<EncounterGroupDto>,
    dead: Vec<DeadMonDto>,
    caught: Vec<CaughtMonDto>,
    box_pokemon: Vec<BoxMonDto>,
    /// map_group of the current wild-encounter zone (0 if no encounter area).
    current_map_group: u8,
    /// map_name of the current wild-encounter zone (0 if no encounter area).
    current_map_name: u8,
    /// Human-readable name for the current zone, empty when not in a wild area.
    current_zone_name: String,
    /// Encounters from the most recently completed run, for cross-run hints.
    prev_run_encounters: Vec<DbEncounterDto>,
    /// Elite 4 + Champion defeat flags: indices 0–4 = Lorelei, Bruno, Agatha, Lance, Blue.
    e4_progress: Vec<bool>,
    /// True when all 8 badges and all 5 Elite 4 members (incl. Champion) are defeated.
    game_cleared: bool,
    /// Injection events (give/take item, make shiny, etc.) queued since the last
    /// tick. Drained on every broadcast; alerts.html shows toasts for each entry.
    injection_events: Vec<serde_json::Value>,
    /// Current Pokédollar balance (decrypted from SaveBlock1).
    money: u32,
    /// In-game save-file play time: hours component.
    play_time_hours: u16,
    /// In-game save-file play time: minutes component (0–59).
    play_time_minutes: u8,
    /// In-game save-file play time: seconds component (0–59).
    play_time_seconds: u8,
    /// User-defined run goals from the `run_goals` DB table.
    goals: Vec<GoalDto>,
    /// Upcoming gym leader's full party read from ROM (randomizer-aware).
    leader_party: Vec<LeaderPartyMonDto>,
}

#[derive(serde::Serialize, Clone)]
struct DeadMonDto {
    nickname: String,
    species_name: String,
    level: u8,
    nature: String,
    shiny: bool,
    soul_link: bool,
    died_at: String,
    gender: u8,
    max_hp: u16,
    attack: u16,
    defense: u16,
    speed: u16,
    sp_attack: u16,
    sp_defense: u16,
    iv_hp: u8,
    iv_atk: u8,
    iv_def: u8,
    iv_spe: u8,
    iv_spa: u8,
    iv_spd: u8,
    ev_hp: u8,
    ev_atk: u8,
    ev_def: u8,
    ev_spe: u8,
    ev_spa: u8,
    ev_spd: u8,
    sprite: Option<String>,
    killed_by: Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct CaughtMonDto {
    nickname: String,
    species_name: String,
    level: u8,
    nature: String,
    shiny: bool,
    caught_at: String,
    met_location_name: String,
    gender: u8,
    iv_hp: u8,
    iv_atk: u8,
    iv_def: u8,
    iv_spe: u8,
    iv_spa: u8,
    iv_spd: u8,
    sprite: Option<String>,
    /// GBA personality value — exposed so the override manager can identify mons.
    personality: u32,
    /// True when this Pokémon has a death record or is a soul-link casualty.
    dead: bool,
}

#[derive(serde::Serialize, Clone)]
struct BoxMonDto {
    box_index: u8,
    slot_index: u8,
    species_name: String,
    nickname: String,
    is_shiny: bool,
    nature: String,
    is_egg: bool,
    iv_hp: u8,
    iv_atk: u8,
    iv_def: u8,
    iv_spe: u8,
    iv_spa: u8,
    iv_spd: u8,
    /// `0` = male, `1` = female, `2` = genderless.
    gender: u8,
    sprite: Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct EncounterGroupDto {
    label: String,
    /// Party-wide encounter rate (0–255) for this encounter type.
    encounter_rate: u8,
    mons: Vec<EncounterMonDto>,
}

#[derive(serde::Serialize, Clone)]
struct EncounterMonDto {
    species_name: String,
    min_level: u8,
    max_level: u8,
    sprite: Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct GymDto {
    leader: String,
    city: String,
    max_level: u8,
    /// Primary type ID of the gym leader / Elite 4 member (Gen III ID, 0–16).
    /// Used by overlay pages to pre-highlight relevant matchups.
    type_id: u8,
}

#[derive(serde::Serialize, Clone)]
struct GoalDto {
    id: i32,
    text: String,
    completed: bool,
}

/// One Pokémon on the upcoming gym leader's team, read directly from ROM
/// so randomizer runs show the actual (post-randomization) team.
#[derive(serde::Serialize, Clone)]
struct LeaderPartyMonDto {
    species_name: String,
    level: u8,
    moves: [String; 4],
    type1: u8,
    type2: u8,
    sprite: Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct SoulLinkPartnerDto {
    nickname: String,
    player: String,
}

#[derive(serde::Serialize, Clone)]
struct MemberDto {
    nickname: String,
    species_name: String,
    level: u8,
    hp: u16,
    max_hp: u16,
    exp: u32,
    nature: String,
    shiny: bool,
    dead: bool,
    soul_link_kill: bool,
    soul_link_partner: Option<SoulLinkPartnerDto>,
    died_at: Option<String>,
    attack: u16,
    defense: u16,
    speed: u16,
    sp_attack: u16,
    sp_defense: u16,
    /// `0` = male, `1` = female, `2` = genderless.
    gender: u8,
    ability: String,
    held_item: String,
    held_item_id: u16,
    growth_rate: String,
    ev_hp: u8,
    ev_atk: u8,
    ev_def: u8,
    ev_spe: u8,
    ev_spa: u8,
    ev_spd: u8,
    iv_hp: u8,
    iv_atk: u8,
    iv_def: u8,
    iv_spe: u8,
    iv_spa: u8,
    iv_spd: u8,
    /// Base64 PNG data URI for the sprite, e.g. `data:image/png;base64,...`.
    /// `None` while the sprite is still in transit from the tracker server.
    sprite: Option<String>,
    /// Unique personality value — used by the overlay to detect death transitions.
    personality: u32,
    /// Status condition bitmask (Gen III encoding):
    /// bits 0-2 = sleep turns, bit 3 = PSN, bit 4 = BRN, bit 5 = FRZ, bit 6 = PAR, bit 7 = TOX.
    status: u32,
    /// Current move names (empty string for empty slots).
    moves: [String; 4],
    /// Current PP for each move slot.
    pp: [u8; 4],
    /// Gen III type ID for the species' first type (0=Normal … 16=Dark).
    type1: u8,
    /// Gen III type ID for the species' second type; equals `type1` for mono-type species.
    type2: u8,
}

// ---------------------------------------------------------------------------
// DB + soul-link state (mirrors AggregatorApp in app.rs)
// ---------------------------------------------------------------------------

struct SlotCache {
    caught: Vec<CaughtPokemon>,
    encounters: Vec<fire_red_database::Encounter>,
    prev_encounters: Vec<fire_red_database::Encounter>,
    last_refresh: Instant,
}

impl SlotCache {
    fn new() -> Self {
        Self {
            caught: Vec::new(),
            encounters: Vec::new(),
            prev_encounters: Vec::new(),
            last_refresh: Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
        }
    }
}

/// Maps a gym leader / Elite 4 member name to their primary type ID (Gen III).
/// Returns 0 (Normal) for unrecognised names such as the post-game Champion rematch.
fn leader_type_id(leader: &str) -> u8 {
    match leader {
        "Brock" => 5,      // Rock
        "Misty" => 10,     // Water
        "Lt. Surge" => 12, // Electric
        "Erika" => 11,     // Grass
        "Koga" => 3,       // Poison
        "Sabrina" => 13,   // Psychic
        "Blaine" => 9,     // Fire
        "Giovanni" => 4,   // Ground
        "Lorelei" => 14,   // Ice
        "Bruno" => 1,      // Fighting
        "Agatha" => 7,     // Ghost
        "Lance" => 15,     // Dragon
        "Blue" => 0,       // Normal (mixed team)
        _ => 0,
    }
}

/// Maps a gym leader name to their vanilla trainer index in `gTrainers`.
///
/// These are the indices for the main (first-encounter) battle in FireRed USA
/// Rev 1.  Randomizers keep the same indices but replace the party ROM data at
/// the pointer stored in the trainer entry, so reading from ROM at runtime picks
/// up the randomized team automatically.
fn leader_trainer_index(leader: &str) -> Option<usize> {
    match leader {
        "Brock" => Some(54),
        "Misty" => Some(55),
        "Lt. Surge" => Some(56),
        "Erika" => Some(57),
        "Koga" => Some(58),
        "Sabrina" => Some(59),
        "Blaine" => Some(60),
        "Giovanni" => Some(61),
        "Lorelei" => Some(118),
        "Bruno" => Some(119),
        "Agatha" => Some(120),
        "Lance" => Some(121),
        "Blue" => Some(148),
        _ => None,
    }
}

/// Trainer entry size in `gTrainers` (40 bytes = 0x28).
const TRAINER_ENTRY_SIZE: usize = 40;

/// GBA ROM bus base address; subtract to get ROM file offset.
const ROM_BUS_BASE: u32 = 0x0800_0000;

/// Reads the gym leader's party from the loaded ROM and builds the DTO list.
///
/// Handles all four party struct layouts (no-item/custom-moves combinations).
/// Returns an empty vec if the ROM is not loaded, the trainer index is unknown,
/// or the ROM file is too small to contain the expected data.
fn build_leader_party(leader_name: &str) -> Vec<LeaderPartyMonDto> {
    let trainer_idx = match leader_trainer_index(leader_name) {
        Some(i) => i,
        None => return vec![],
    };
    let rom = match fire_red_rom_buffer::try_get_rom() {
        Some(r) => r,
        None => return vec![],
    };
    let trainer_table = fire_red_rom_buffer::get_rom_addresses().trainer_data_addr;
    if trainer_table == 0 {
        return vec![];
    }
    let entry_off = trainer_table + trainer_idx * TRAINER_ENTRY_SIZE;
    if rom.len() < entry_off + TRAINER_ENTRY_SIZE {
        return vec![];
    }
    let entry = &rom[entry_off..entry_off + TRAINER_ENTRY_SIZE];

    let party_flags  = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
    let party_size   = entry[0x20] as usize;
    let party_ptr    = u32::from_le_bytes([entry[0x24], entry[0x25], entry[0x26], entry[0x27]]);

    if party_ptr < ROM_BUS_BASE || party_size == 0 || party_size > 6 {
        return vec![];
    }
    let party_off = (party_ptr - ROM_BUS_BASE) as usize;

    // Struct sizes based on party_flags bits.
    let has_moves = (party_flags & 0x01) != 0;
    let has_item  = (party_flags & 0x02) != 0;
    let entry_bytes: usize = match (has_item, has_moves) {
        (false, false) => 4,
        (true,  false) => 8,
        (false, true)  => 12,
        (true,  true)  => 16,
    };

    if rom.len() < party_off + party_size * entry_bytes {
        return vec![];
    }

    (0..party_size)
        .filter_map(|i| {
            let base = party_off + i * entry_bytes;
            let b = &rom[base..base + entry_bytes];
            let level   = b[1];
            let species = u16::from_le_bytes([b[2], b[3]]);
            if species == 0 || species > fire_red_states::MAX_NATIONAL_DEX_FIRERED {
                return None;
            }
            let mut moves = [String::new(), String::new(), String::new(), String::new()];
            if has_moves {
                let moves_start = if has_item { 6 } else { 4 };
                for (m, mv) in moves.iter_mut().enumerate() {
                    let off = moves_start + m * 2;
                    if off + 1 < entry_bytes {
                        let id = u16::from_le_bytes([b[off], b[off + 1]]);
                        *mv = fire_red_database::move_name(id).to_string();
                    }
                }
            }
            let species_name = fire_red_text::get_pokemon_name_by_number(species as usize)
                .unwrap_or_else(|e| e);
            let (type1, type2) = fire_red_party_monitor::species_type_static(species);
            Some(LeaderPartyMonDto {
                species_name,
                level,
                moves,
                type1,
                type2,
                sprite: None, // sprites are not pre-loaded for leader panel; overlay fetches via /api/sprite
            })
        })
        .collect()
}

struct BroadcastLoop {
    live_slots: SharedSlots,
    caches: Vec<SlotCache>,
    soul_link_propagated: HashSet<(usize, u32)>,
    /// Manual soul-link overrides for the current run (personality → partner_personality).
    /// Refreshed alongside the caught cache; consulted before automatic met_location pairing.
    soul_link_overrides: HashMap<u32, u32>,
    last_json: String,
    sprites: PngSpriteCache,
    /// Per-slot: set of (run_id) for which we have already triggered a backup so we
    /// don't fire again on subsequent ticks.
    backup_done: HashSet<u32>,
    /// Per-slot: badge count observed on the previous tick, for LiveSplit split detection.
    prev_badge_counts: Vec<usize>,
    /// DB connection string (for auto-backup).
    db_conn: Option<String>,
    /// Directory to write auto-backup files into.
    backup_dir: Option<String>,
    /// Whether to fire a LiveSplit split on each new badge.
    livesplit_split_on_badges: bool,
}

impl BroadcastLoop {
    fn new(
        live_slots: SharedSlots,
        sprites: PngSpriteCache,
        db_conn: Option<String>,
        backup_dir: Option<String>,
        livesplit_split_on_badges: bool,
    ) -> Self {
        Self {
            live_slots,
            caches: Vec::new(),
            soul_link_propagated: HashSet::new(),
            soul_link_overrides: HashMap::new(),
            last_json: String::new(),
            sprites,
            backup_done: HashSet::new(),
            prev_badge_counts: Vec::new(),
            db_conn,
            backup_dir,
            livesplit_split_on_badges,
        }
    }

    /// Requests sprites for party members and encounter pokemon not yet cached.
    fn request_sprites(&self, slots: &[Arc<MonitorSlot>], states: &[(String, Option<GameState>)]) {
        let cache = self.sprites.lock_or_recover();
        for (i, slot) in slots.iter().enumerate() {
            let Some(gs) = &states[i].1 else { continue };
            let mut known = slot.known_species.lock_or_recover();
            let mut needed: Vec<u16> = Vec::new();

            // Party sprites (normal + shiny variant if shiny)
            for p in &gs.party {
                let s = p.box_mon.secure.growth.species;
                let shiny = is_shiny(p.box_mon.personality, p.box_mon.ot_id);
                if s == 0 || s > MAX_NATIONAL_DEX_FIRERED {
                    continue;
                }
                if !known.contains(&s) && !cache.contains_key(&(s, false)) {
                    needed.push(s);
                    known.insert(s);
                }
                if shiny && !cache.contains_key(&(s, true)) {
                    needed.push(s);
                }
            }

            // Encounter sprites (normal variant only)
            let enc = &gs.encounters;
            let all_enc = enc
                .land_mon_encounters
                .wild_pokemon_list
                .iter()
                .chain(enc.water_mon_encounters.wild_pokemon_list.iter())
                .chain(enc.rock_smash_encounters.wild_pokemon_list.iter())
                .chain(enc.fishing_encounters.wild_pokemon_list.iter());
            for w in all_enc {
                let s = w.species;
                if s == 0 || s > MAX_NATIONAL_DEX_FIRERED {
                    continue;
                }
                if !known.contains(&s) && !cache.contains_key(&(s, false)) {
                    needed.push(s);
                    known.insert(s);
                }
            }

            drop(known);
            if !needed.is_empty() {
                needed.sort();
                needed.dedup();
                slot.texture_request_queue
                    .lock_or_recover()
                    .push_back(needed);
            }
        }
    }

    /// Drains any pending textures from the TCP pipeline into the sprite cache.
    /// Also wires the shared sprite cache into any slot that connected after
    /// `run()` started (identified by having `sprite_cache = None`).
    fn drain_sprites(&mut self, slots: &[Arc<MonitorSlot>]) {
        for slot in slots {
            let mut sc = slot.sprite_cache.lock_or_recover();
            if sc.is_none() {
                *sc = Some(self.sprites.clone());
            }
            drop(sc);

            let mut pending = slot.pending_textures.lock_or_recover();
            if pending.is_empty() {
                continue;
            }
            let drained: Vec<_> = pending.drain(..).collect();
            drop(pending);
            let mut cache = self.sprites.lock_or_recover();
            for pt in drained {
                let key = (pt.species, pt.shiny);
                if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(key)
                    && let Some(png) = encode_png(&pt.pixels, pt.width, pt.height)
                {
                    e.insert(png);
                }
            }
        }
    }

    /// Returns a `data:image/png;base64,...` URI for the given species/shiny
    /// if the sprite has been received and encoded, or `None` otherwise.
    fn sprite_uri(&self, species: u16, shiny: bool) -> Option<String> {
        let cache = self.sprites.lock_or_recover();
        cache
            .get(&(species, shiny))
            .map(|png| format!("data:image/png;base64,{}", base64_encode(png)))
    }

    /// Propagates soul-link deaths across slots (DB-persisted and live) and
    /// returns the set of personality values that are soul-link-dead per slot.
    fn propagate_soul_links(
        &mut self,
        slots: &[Arc<MonitorSlot>],
        states: &[(String, Option<GameState>)],
        all_dead: &[HashMap<u32, DeadPokemon>],
    ) -> Vec<HashSet<u32>> {
        let n = slots.len();

        // Pre-sort gifts per slot once; reused by both the DB propagation loop
        // and the live detection loop. Gift Pokémon (met_location = 0) are
        // soul-linked by receipt order (caught_at) instead of by location —
        // matching the pairing used in soul_link_kill_candidates and update().
        let sorted_gifts: Vec<Vec<&CaughtPokemon>> = self
            .caches
            .iter()
            .map(|c| sort_gifts_by_caught_at(&c.caught))
            .collect();

        // DB soul-link death propagation
        for i in 0..n {
            let dead_personalities: Vec<u32> = all_dead[i].keys().copied().collect();
            for dead_p in dead_personalities {
                let met_loc = self.caches[i]
                    .caught
                    .iter()
                    .find(|c| c.personality == dead_p)
                    .map(|c| c.met_location)
                    .unwrap_or(0);

                // For gift Pokémon (met_loc == 0) find the receipt-order index;
                // for non-gifts the met_location itself is the pairing key.
                let gift_idx: Option<usize> = if met_loc == 0 {
                    sorted_gifts[i].iter().position(|c| c.personality == dead_p)
                } else {
                    None
                };
                if met_loc == 0 && gift_idx.is_none() {
                    continue;
                }

                for j in 0..n {
                    if j == i {
                        continue;
                    }
                    // Check for a manual override first; fall back to automatic
                    // met_location / receipt-order pairing if none is set.
                    let partner = if let Some(&override_p) = self.soul_link_overrides.get(&dead_p) {
                        self.caches[j]
                            .caught
                            .iter()
                            .find(|c| c.personality == override_p)
                            .cloned()
                    } else if met_loc == 0 {
                        gift_idx
                            .and_then(|idx| sorted_gifts[j].get(idx))
                            .map(|c| (*c).clone())
                    } else {
                        self.caches[j]
                            .caught
                            .iter()
                            .find(|c| c.met_location == met_loc && c.personality != dead_p)
                            .cloned()
                    };
                    if let Some(p) = partner {
                        let key = (j, p.personality);
                        let already_dead = all_dead[j].contains_key(&p.personality);
                        let already_propagated = self.soul_link_propagated.contains(&key);
                        if !already_dead && !already_propagated {
                            // Some(_) = run_id known and DB responded (new or pre-existing row).
                            // None    = run_id unknown or error; retry next frame.
                            let result = slots[j]
                                .db
                                .as_ref()
                                .and_then(|db| db.mark_soul_link_dead(&p));
                            if result.is_some() {
                                self.soul_link_propagated.insert(key);
                            }
                        }
                    }
                }
            }
        }

        // Live soul-link dead detection — uses sorted_gifts built above so
        // gift pairing is consistent with the DB propagation path.
        let mut live_soul_link_dead: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        for i in 0..n {
            let Some(gs_i) = &states[i].1 else { continue };
            for p_i in &gs_i.party {
                if p_i.hp != 0 {
                    continue;
                }
                let met_i = p_i.box_mon.secure.misc.met_location;
                for j in 0..n {
                    if j == i {
                        continue;
                    }
                    let personality_i = p_i.box_mon.personality;
                    if let Some(&override_p) = self.soul_link_overrides.get(&personality_i) {
                        // Manual override supersedes met_location pairing — mirrors DB path.
                        let Some(gs_j) = &states[j].1 else { continue };
                        if let Some(partner) = gs_j
                            .party
                            .iter()
                            .find(|p| p.box_mon.personality == override_p)
                        {
                            live_soul_link_dead[j].insert(partner.box_mon.personality);
                        }
                    } else if met_i == 0 {
                        // Gift Pokémon: pair by receipt order — matches DB path.
                        let Some(idx) = sorted_gifts[i]
                            .iter()
                            .position(|c| c.personality == personality_i)
                        else {
                            continue;
                        };
                        if let Some(partner) = sorted_gifts[j].get(idx) {
                            live_soul_link_dead[j].insert(partner.personality);
                        }
                    } else {
                        let Some(gs_j) = &states[j].1 else { continue };
                        for p_j in &gs_j.party {
                            if p_j.box_mon.secure.misc.met_location == met_i {
                                live_soul_link_dead[j].insert(p_j.box_mon.personality);
                            }
                        }
                    }
                }
            }
        }
        live_soul_link_dead
    }

    /// Builds the party DTO list for one slot.
    fn build_party_dto(
        &self,
        slot_idx: usize,
        gs: &GameState,
        dead_records: &HashMap<u32, DeadPokemon>,
        soul_link_dead: &HashSet<u32>,
        states: &[(String, Option<GameState>)],
    ) -> Vec<MemberDto> {
        let n = states.len();
        gs.party
            .iter()
            .map(|p| {
                let personality = p.box_mon.personality;
                let ot_id = p.box_mon.ot_id;
                let shiny = is_shiny(personality, ot_id);
                let met = p.box_mon.secure.misc.met_location;
                let species = p.box_mon.secure.growth.species;
                let is_soul_link = soul_link_dead.contains(&personality);
                let dead_record = dead_records.get(&personality);
                let dead = dead_record.is_some() || p.hp == 0 || is_soul_link;

                // A manual override supersedes met_location pairing entirely.
                // Without one, fall back to the original location-match logic.
                let soul_link_partner =
                    if let Some(&override_p) = self.soul_link_overrides.get(&personality) {
                        let mut found = None;
                        'outer: for (j, (player_j, state_j)) in states.iter().enumerate().take(n) {
                            if j == slot_idx {
                                continue;
                            }
                            if let Some(gs_j) = state_j
                                && let Some(p_j) = gs_j
                                    .party
                                    .iter()
                                    .find(|p| p.box_mon.personality == override_p)
                            {
                                found = Some(SoulLinkPartnerDto {
                                    nickname: p_j.get_nickname_string(),
                                    player: player_j.clone(),
                                });
                                break 'outer;
                            }
                            // Partner may be dead or in-box; fall back to the caught cache.
                            if let Some(c) = self.caches[j]
                                .caught
                                .iter()
                                .find(|c| c.personality == override_p)
                            {
                                found = Some(SoulLinkPartnerDto {
                                    nickname: c.nickname.clone(),
                                    player: player_j.clone(),
                                });
                                break 'outer;
                            }
                        }
                        found
                    } else if met == 0 {
                        None
                    } else {
                        let mut found = None;
                        'outer: for (j, (player_j, state_j)) in states.iter().enumerate().take(n) {
                            if j == slot_idx {
                                continue;
                            }
                            if let Some(gs_j) = state_j {
                                for p_j in &gs_j.party {
                                    if p_j.box_mon.secure.misc.met_location == met {
                                        found = Some(SoulLinkPartnerDto {
                                            nickname: p_j.get_nickname_string(),
                                            player: player_j.clone(),
                                        });
                                        break 'outer;
                                    }
                                }
                            }
                        }
                        found
                    };

                let (died_at, soul_link_kill, attack, defense, speed, sp_attack, sp_defense) =
                    if let Some(r) = dead_record {
                        (
                            Some(fire_red_database::format_timestamp(r.died_at)),
                            r.is_soul_link_death,
                            r.attack,
                            r.defense,
                            r.speed,
                            r.sp_attack,
                            r.sp_defense,
                        )
                    } else {
                        (
                            None,
                            false,
                            p.attack,
                            p.defense,
                            p.speed,
                            p.sp_attack,
                            p.sp_defense,
                        )
                    };

                let sprite = self.sprite_uri(species, shiny);
                let (type1, type2) = fire_red_party_monitor::species_type_static(species);

                MemberDto {
                    nickname: p.get_nickname_string(),
                    species_name: p.box_mon.secure.growth.species_string.clone(),
                    level: p.level,
                    hp: p.hp,
                    max_hp: p.max_hp,
                    exp: p.box_mon.secure.growth.experience,
                    nature: fire_red_database::nature_name(personality).to_string(),
                    shiny,
                    dead,
                    soul_link_kill,
                    soul_link_partner,
                    died_at,
                    attack,
                    defense,
                    speed,
                    sp_attack,
                    sp_defense,
                    gender: p.box_mon.gender,
                    ability: p.box_mon.ability_string.clone(),
                    held_item: p.box_mon.secure.growth.held_item_string.clone(),
                    held_item_id: p.box_mon.secure.growth.held_item,
                    growth_rate: p.box_mon.secure.growth.growth_rate_string.clone(),
                    iv_hp: p.box_mon.secure.misc.iv_egg_ability.hp_iv,
                    iv_atk: p.box_mon.secure.misc.iv_egg_ability.attack_iv,
                    iv_def: p.box_mon.secure.misc.iv_egg_ability.defense_iv,
                    iv_spe: p.box_mon.secure.misc.iv_egg_ability.speed_iv,
                    iv_spa: p.box_mon.secure.misc.iv_egg_ability.sp_attack_iv,
                    iv_spd: p.box_mon.secure.misc.iv_egg_ability.sp_def_iv,
                    ev_hp: p.box_mon.secure.ev_condition.hp_ev,
                    ev_atk: p.box_mon.secure.ev_condition.attack_ev,
                    ev_def: p.box_mon.secure.ev_condition.defense_ev,
                    ev_spe: p.box_mon.secure.ev_condition.speed_ev,
                    ev_spa: p.box_mon.secure.ev_condition.sp_attack_ev,
                    ev_spd: p.box_mon.secure.ev_condition.sp_defense_ev,
                    sprite,
                    personality,
                    status: p.status,
                    moves: {
                        let m = &p.box_mon.secure.attack.moves;
                        [
                            fire_red_database::move_name(m[0]).to_string(),
                            fire_red_database::move_name(m[1]).to_string(),
                            fire_red_database::move_name(m[2]).to_string(),
                            fire_red_database::move_name(m[3]).to_string(),
                        ]
                    },
                    pp: p.box_mon.secure.attack.pp,
                    type1,
                    type2,
                }
            })
            .collect()
    }

    /// Builds the dead-mon DTO list for one slot, sorted newest-first.
    fn build_dead_dto(&self, dead_records: &HashMap<u32, DeadPokemon>) -> Vec<DeadMonDto> {
        let mut dead_sorted: Vec<&DeadPokemon> = dead_records.values().collect();
        dead_sorted.sort_by_key(|b| std::cmp::Reverse(b.died_at));
        dead_sorted
            .iter()
            .map(|dp| DeadMonDto {
                nickname: dp.nickname.clone(),
                species_name: dp.species_name.clone(),
                level: dp.level,
                nature: dp.nature.clone(),
                shiny: dp.is_shiny,
                soul_link: dp.is_soul_link_death,
                gender: dp.gender,
                died_at: fire_red_database::format_timestamp(dp.died_at),
                max_hp: dp.max_hp,
                attack: dp.attack,
                defense: dp.defense,
                speed: dp.speed,
                sp_attack: dp.sp_attack,
                sp_defense: dp.sp_defense,
                iv_hp: dp.ivs.hp,
                iv_atk: dp.ivs.attack,
                iv_def: dp.ivs.defense,
                iv_spe: dp.ivs.speed,
                iv_spa: dp.ivs.sp_attack,
                iv_spd: dp.ivs.sp_defense,
                ev_hp: dp.evs.hp,
                ev_atk: dp.evs.attack,
                ev_def: dp.evs.defense,
                ev_spe: dp.evs.speed,
                ev_spa: dp.evs.sp_attack,
                ev_spd: dp.evs.sp_defense,
                sprite: self.sprite_uri(dp.species, dp.is_shiny),
                killed_by: dp.killed_by_species.clone(),
            })
            .collect()
    }

    /// Runs one tick: refreshes DB caches, propagates soul-link deaths, and
    /// returns a JSON string if the state has changed since the last tick.
    fn tick(&mut self) -> Option<String> {
        let slots: Vec<Arc<MonitorSlot>> = self.live_slots.lock_or_recover().clone();
        let n = slots.len();
        while self.caches.len() < n {
            self.caches.push(SlotCache::new());
        }

        // Collect live states
        let states: Vec<(String, Option<GameState>)> = slots
            .iter()
            .map(|s| {
                let state = s.state.lock_or_recover().clone();
                let label = s.label.lock_or_recover().clone();
                (label, state)
            })
            .collect();

        // Sprite pipeline
        self.request_sprites(&slots, &states);
        self.drain_sprites(&slots);

        // If the tracker confirmed a run change, mark the DB reader dirty so
        // sync_player re-queries even though the player name hasn't changed.
        for slot in &slots {
            if slot
                .run_changed
                .swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                if let Some(db) = &slot.db {
                    db.mark_dirty();
                }
                self.soul_link_propagated.clear();
                self.soul_link_overrides.clear();
            }
        }

        // Sync DB run IDs
        let mut run_id_changed = vec![false; n];
        for (i, slot) in slots.iter().enumerate() {
            if let Some(db) = &slot.db {
                run_id_changed[i] = db.sync_player(&states[i].0);
            }
        }

        // Refresh caught cache and soul-link override map.
        let now = Instant::now();
        for i in 0..n {
            let stale = now.duration_since(self.caches[i].last_refresh) >= Duration::from_secs(1);
            if (run_id_changed[i] || stale)
                && let Some(db) = &slots[i].db
            {
                let label = &states[i].0;
                self.caches[i].caught = db.list_caught(label);
                self.caches[i].encounters = db.list_encounters(label);
                self.caches[i].prev_encounters = db.list_prev_run_encounters(label);
                self.caches[i].last_refresh = now;
                // Overrides are run-wide; load once from the first slot that has a DB.
                // The map is cleared on run_id change below, so this stays consistent.
                if i == 0 || self.soul_link_overrides.is_empty() {
                    self.soul_link_overrides = db.load_soul_link_overrides();
                }
            }
        }

        // Dead records (fresh every tick), filtered per player by name.
        let all_dead: Vec<HashMap<u32, DeadPokemon>> = (0..n)
            .map(|i| {
                slots[i]
                    .db
                    .as_ref()
                    .map(|db| db.list_dead_with_records(&states[i].0))
                    .unwrap_or_default()
            })
            .collect();

        // Snapshot box data per slot for use in sprite requests and DTO building.
        let all_box: Vec<Vec<fire_red_states::BoxEntry>> = slots
            .iter()
            .map(|s| s.box_data.lock_or_recover().clone())
            .collect();

        // Request sprites for dead, caught, and box pokemon not yet in the cache
        {
            let cache = self.sprites.lock_or_recover();
            for (i, slot) in slots.iter().enumerate() {
                let mut known = slot.known_species.lock_or_recover();
                let mut needed: Vec<u16> = Vec::new();
                for dp in all_dead[i].values() {
                    let s = dp.species;
                    if s > 0
                        && s <= MAX_NATIONAL_DEX_FIRERED
                        && !known.contains(&s)
                        && !cache.contains_key(&(s, dp.is_shiny))
                    {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                for cp in &self.caches[i].caught {
                    let s = cp.species;
                    if s > 0
                        && s <= MAX_NATIONAL_DEX_FIRERED
                        && !known.contains(&s)
                        && !cache.contains_key(&(s, cp.is_shiny))
                    {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                for enc in &self.caches[i].encounters {
                    let s = enc.species;
                    if s > 0
                        && s <= MAX_NATIONAL_DEX_FIRERED
                        && !known.contains(&s)
                        && !cache.contains_key(&(s, false))
                    {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                for be in &all_box[i] {
                    let s = be.species;
                    if s > 0
                        && s <= MAX_NATIONAL_DEX_FIRERED
                        && !known.contains(&s)
                        && !cache.contains_key(&(s, be.is_shiny))
                    {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                drop(known);
                if !needed.is_empty() {
                    needed.sort();
                    needed.dedup();
                    slot.texture_request_queue
                        .lock_or_recover()
                        .push_back(needed);
                }
            }
        }

        // Soul-link death propagation (DB-persisted + live)
        let live_soul_link_dead = self.propagate_soul_links(&slots, &states, &all_dead);

        // Determine display order: sort by (preferred_player, player_name).
        // Slots with no preference sort last; ties break alphabetically by name.
        let mut display_order: Vec<usize> = (0..n).collect();
        display_order.sort_by(|&i, &j| {
            let pi = states[i]
                .1
                .as_ref()
                .and_then(|gs| gs.preferred_player)
                .map(u32::from)
                .unwrap_or(u32::MAX);
            let pj = states[j]
                .1
                .as_ref()
                .and_then(|gs| gs.preferred_player)
                .map(u32::from)
                .unwrap_or(u32::MAX);
            pi.cmp(&pj)
                .then_with(|| states[i].0.to_lowercase().cmp(&states[j].0.to_lowercase()))
        });

        // Grow per-slot tracking vecs if new slots appeared.
        while self.prev_badge_counts.len() < n {
            self.prev_badge_counts.push(0);
        }

        // LiveSplit badge splits + game-cleared auto-backup.
        for i in 0..n {
            let badge_state = states[i].1.as_ref().and_then(|gs| gs.badge_state.as_ref());
            let badge_count = badge_state
                .map(|b| b.badges.iter().filter(|&&v| v).count())
                .unwrap_or(0);
            let game_cleared = badge_state.map(|b| b.game_complete()).unwrap_or(false);
            let run_id = slots[i].db.as_ref().and_then(|db| db.active_run_id());

            if self.livesplit_split_on_badges && badge_count > self.prev_badge_counts[i] {
                fire_red_game_loop::livesplit::split();
            }
            self.prev_badge_counts[i] = badge_count;

            if game_cleared
                && let (Some(rid), Some(conn), Some(dir)) =
                    (run_id, self.db_conn.as_ref(), self.backup_dir.as_ref())
                    && self.backup_done.insert(rid) {
                        let conn2 = conn.clone();
                        let dir2 = dir.clone();
                        std::thread::spawn(move || {
                            let json = fire_red_database::export_run(&conn2, rid);
                            let ts = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let path = std::path::Path::new(&dir2)
                                .join(format!("run_{rid}_{ts}.json"));
                            if let Err(e) = std::fs::create_dir_all(&dir2) {
                                tracing::warn!("auto-backup: could not create backup_dir: {e}");
                            } else if let Err(e) =
                                std::fs::write(&path, json.to_string())
                            {
                                tracing::warn!("auto-backup: write failed: {e}");
                            } else {
                                tracing::info!("auto-backup: wrote {}", path.display());
                            }
                        });
                    }
        }

        // Build JSON payload
        let slots_dto: Vec<SlotDto> = display_order
            .iter()
            .copied()
            .map(|i| {
                let (label, state) = &states[i];
                let dead_records = &all_dead[i];
                let soul_link_dead = &live_soul_link_dead[i];
                let db_connected = slots[i].db.is_some();
                let active_run_id = slots[i].db.as_ref().and_then(|db| db.active_run_id());

                let run_summary = slots[i].db.as_ref().and_then(|db| db.run_summary()).map(
                    |(run_id, player_name, started_at, ended_at, deaths, caught)| RunSummaryDto {
                        run_id,
                        player_name,
                        started_at: fire_red_database::format_timestamp(started_at),
                        ended_at: ended_at.map(fire_red_database::format_timestamp),
                        deaths,
                        caught,
                    },
                );

                let db_encounters: Vec<DbEncounterDto> = self.caches[i]
                    .encounters
                    .iter()
                    .map(|enc| DbEncounterDto {
                        species_name: enc.species_name.clone(),
                        level: enc.level,
                        caught: enc.caught,
                        is_shiny: enc.is_shiny,
                        encountered_at: fire_red_database::format_timestamp(enc.encountered_at),
                        area: {
                            let n =
                                fire_red_location_names::map_area_name(enc.map_group, enc.map_name);
                            if n.is_empty() {
                                format!("{}\u{B7}{}", enc.map_group, enc.map_name)
                            } else {
                                n.to_string()
                            }
                        },
                        sprite: self.sprite_uri(enc.species, enc.is_shiny),
                        map_group: enc.map_group,
                        map_name: enc.map_name,
                    })
                    .collect();

                let (connected, badges, next_gym, e4_progress, game_cleared, party, encounters) =
                    match state {
                        None => (
                            false,
                            vec![false; 8],
                            None,
                            vec![false; 5],
                            false,
                            vec![],
                            vec![],
                        ),
                        Some(gs) => {
                            let badges: Vec<bool> = gs
                                .badge_state
                                .as_ref()
                                .map(|b| b.badges.to_vec())
                                .unwrap_or_else(|| vec![false; 8]);

                            let next_gym = gs
                                .badge_state
                                .as_ref()
                                .and_then(|b| b.next_gym.as_ref())
                                .map(|g| GymDto {
                                    leader: g.leader.clone(),
                                    city: g.city.clone(),
                                    max_level: g.max_level,
                                    type_id: leader_type_id(&g.leader),
                                });

                            let e4_progress: Vec<bool> = gs
                                .badge_state
                                .as_ref()
                                .map(|b| b.e4.to_vec())
                                .unwrap_or_else(|| vec![false; 5]);

                            let game_cleared = gs
                                .badge_state
                                .as_ref()
                                .map(|b| b.game_complete())
                                .unwrap_or(false);

                            let party =
                                self.build_party_dto(i, gs, dead_records, soul_link_dead, &states);

                            // Build encounter groups (skip empty ones)
                            let enc = &gs.encounters;
                            let mut encounters: Vec<EncounterGroupDto> = Vec::new();

                            let land: Vec<EncounterMonDto> = enc
                                .land_mon_encounters
                                .wild_pokemon_list
                                .iter()
                                .filter(|w| w.species > 0 && w.species <= MAX_NATIONAL_DEX_FIRERED)
                                .map(|w| EncounterMonDto {
                                    species_name: fire_red_text::get_pokemon_name_by_number(w.species as usize).unwrap_or_else(|e| e),
                                    min_level: w.min_level,
                                    max_level: w.max_level,
                                    sprite: self.sprite_uri(w.species, false),
                                })
                                .collect();
                            if !land.is_empty() {
                                encounters.push(EncounterGroupDto {
                                    label: "Land".into(),
                                    encounter_rate: enc.land_mon_encounters.encounter_rate,
                                    mons: land,
                                });
                            }

                            let water_rate = enc.water_mon_encounters.encounter_rate.max(enc.fishing_encounters.encounter_rate);
                            let water_fish: Vec<EncounterMonDto> = enc
                                .water_mon_encounters
                                .wild_pokemon_list
                                .iter()
                                .chain(enc.fishing_encounters.wild_pokemon_list.iter())
                                .filter(|w| w.species > 0 && w.species <= MAX_NATIONAL_DEX_FIRERED)
                                .map(|w| EncounterMonDto {
                                    species_name: fire_red_text::get_pokemon_name_by_number(w.species as usize).unwrap_or_else(|e| e),
                                    min_level: w.min_level,
                                    max_level: w.max_level,
                                    sprite: self.sprite_uri(w.species, false),
                                })
                                .collect();
                            if !water_fish.is_empty() {
                                encounters.push(EncounterGroupDto {
                                    label: "Water / Fishing".into(),
                                    encounter_rate: water_rate,
                                    mons: water_fish,
                                });
                            }

                            let rock: Vec<EncounterMonDto> = enc
                                .rock_smash_encounters
                                .wild_pokemon_list
                                .iter()
                                .filter(|w| w.species > 0 && w.species <= MAX_NATIONAL_DEX_FIRERED)
                                .map(|w| EncounterMonDto {
                                    species_name: fire_red_text::get_pokemon_name_by_number(w.species as usize).unwrap_or_else(|e| e),
                                    min_level: w.min_level,
                                    max_level: w.max_level,
                                    sprite: self.sprite_uri(w.species, false),
                                })
                                .collect();
                            if !rock.is_empty() {
                                encounters.push(EncounterGroupDto {
                                    label: "Rock Smash".into(),
                                    encounter_rate: enc.rock_smash_encounters.encounter_rate,
                                    mons: rock,
                                });
                            }

                            (
                                true,
                                badges,
                                next_gym,
                                e4_progress,
                                game_cleared,
                                party,
                                encounters,
                            )
                        }
                    };

                // dead_records and caches are already filtered by player_name in
                // list_dead_with_records / list_caught, so no further filtering needed.
                let dead = self.build_dead_dto(dead_records);

                let caught: Vec<CaughtMonDto> = self.caches[i]
                    .caught
                    .iter()
                    .rev()
                    .map(|cp| CaughtMonDto {
                        nickname: cp.nickname.clone(),
                        species_name: cp.species_name.clone(),
                        level: cp.level,
                        nature: cp.nature.clone(),
                        shiny: cp.is_shiny,
                        caught_at: fire_red_database::format_timestamp(cp.caught_at),
                        met_location_name: if cp.location_name.is_empty() {
                            fire_red_location_names::location_name(cp.met_location).to_string()
                        } else {
                            cp.location_name.clone()
                        },
                        gender: cp.gender,
                        iv_hp: cp.ivs.hp,
                        iv_atk: cp.ivs.attack,
                        iv_def: cp.ivs.defense,
                        iv_spe: cp.ivs.speed,
                        iv_spa: cp.ivs.sp_attack,
                        iv_spd: cp.ivs.sp_defense,
                        sprite: self.sprite_uri(cp.species, cp.is_shiny),
                        personality: cp.personality,
                        dead: dead_records.contains_key(&cp.personality)
                            || soul_link_dead.contains(&cp.personality),
                    })
                    .collect();

                let box_pokemon: Vec<BoxMonDto> = all_box[i]
                    .iter()
                    .map(|be| BoxMonDto {
                        box_index: be.box_index,
                        slot_index: be.slot_index,
                        species_name: be.species_name.clone(),
                        nickname: be.nickname.clone(),
                        is_shiny: be.is_shiny,
                        nature: be.nature.clone(),
                        is_egg: be.is_egg,
                        gender: be.gender,
                        iv_hp: be.iv_hp,
                        iv_atk: be.iv_atk,
                        iv_def: be.iv_def,
                        iv_spe: be.iv_spe,
                        iv_spa: be.iv_spa,
                        iv_spd: be.iv_spd,
                        sprite: self.sprite_uri(be.species, be.is_shiny),
                    })
                    .collect();

                // True player position from EWRAM, not from the encounter header.
                // On randomized ROMs the encounter slot key may differ from the
                // physical map position; always use the EWRAM-derived value.
                let (current_map_group, current_map_name) = match state {
                    Some(gs) => (gs.current_map_group, gs.current_map_name),
                    None => (0u8, 0u8),
                };
                let current_zone_name = match state {
                    Some(gs) if !gs.zone_name.is_empty() => gs.zone_name.clone(),
                    _ => {
                        fire_red_location_names::map_area_name(current_map_group, current_map_name)
                            .to_string()
                    }
                };

                // Encounters from the previous completed run for cross-run hints
                let prev_run_encounters: Vec<DbEncounterDto> = self.caches[i]
                    .prev_encounters
                    .iter()
                    .map(|enc| DbEncounterDto {
                        species_name: enc.species_name.clone(),
                        level: enc.level,
                        caught: enc.caught,
                        is_shiny: enc.is_shiny,
                        encountered_at: fire_red_database::format_timestamp(enc.encountered_at),
                        area: {
                            let n =
                                fire_red_location_names::map_area_name(enc.map_group, enc.map_name);
                            if n.is_empty() {
                                format!("{}\u{B7}{}", enc.map_group, enc.map_name)
                            } else {
                                n.to_string()
                            }
                        },
                        sprite: self.sprite_uri(enc.species, enc.is_shiny),
                        map_group: enc.map_group,
                        map_name: enc.map_name,
                    })
                    .collect();

                // Push clause-enforcement warnings from the tracker as inject-style toasts.
                if let Some(gs) = state
                    && !gs.warnings.is_empty()
                {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0);
                    let mut queue = slots[i].injection_events.lock_or_recover();
                    for (j, warn) in gs.warnings.iter().enumerate() {
                        queue.push_back(serde_json::json!({
                            "type":  "clause_warning",
                            "label": warn,
                            "at":    now_ms + j as u128,
                        }));
                    }
                }

                let injection_events: Vec<serde_json::Value> = slots[i]
                    .injection_events
                    .lock_or_recover()
                    .drain(..)
                    .collect();

                let money = state.as_ref().map_or(0, |gs| gs.money);
                let play_time_hours = state.as_ref().map_or(0, |gs| gs.play_time_hours);
                let play_time_minutes = state.as_ref().map_or(0, |gs| gs.play_time_minutes);
                let play_time_seconds = state.as_ref().map_or(0, |gs| gs.play_time_seconds);

                let goals: Vec<GoalDto> = slots[i]
                    .db
                    .as_ref()
                    .map(|db| db.list_goals().into_iter().map(|g| GoalDto { id: g.id, text: g.text, completed: g.completed }).collect())
                    .unwrap_or_default();

                let leader_party: Vec<LeaderPartyMonDto> = if let Some(gym) = &next_gym {
                    build_leader_party(&gym.leader)
                } else {
                    vec![]
                };

                SlotDto {
                    label: label.clone(),
                    connected,
                    db_connected,
                    active_run_id,
                    run_summary,
                    db_encounters,
                    badges,
                    next_gym,
                    party,
                    encounters,
                    dead,
                    caught,
                    box_pokemon,
                    current_map_group,
                    current_map_name,
                    current_zone_name,
                    prev_run_encounters,
                    e4_progress,
                    game_cleared,
                    injection_events,
                    money,
                    play_time_hours,
                    play_time_minutes,
                    play_time_seconds,
                    goals,
                    leader_party,
                }
            })
            .collect();

        let json = serde_json::to_string(&slots_dto).unwrap_or_else(|_| "[]".to_string());
        if json == self.last_json {
            return None;
        }
        self.last_json = json.clone();
        Some(json)
    }
}

// ---------------------------------------------------------------------------
// Axum shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct WebState {
    tx: watch::Sender<String>,
    live_slots: SharedSlots,
    db_conn: Option<String>,
    testing: bool,
    allow_injections: bool,
    connector: Option<Arc<crate::direct::DirectConnector>>,
    discord_slash: Option<crate::config::DiscordSlashConfig>,
    /// Path to the TOML config file, used by the hot-reload endpoint.
    config_path: Option<Arc<String>>,
    /// In-memory map from user_id to their most recently connected run_id.
    user_active_run: Arc<Mutex<HashMap<u32, u32>>>,
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

const OVERLAY_HTML: &str = include_str!("overlay.html");
const FOCUSED_HTML: &str = include_str!("focused.html");
const DBVIEWER_HTML: &str = include_str!("db.html");
const HISTORY_HTML: &str = include_str!("history.html");
const ALERTS_HTML: &str = include_str!("alerts.html");
const ROUTES_HTML: &str = include_str!("routes.html");
const PARTY_PLAIN_HTML: &str = include_str!("party_plain.html");
const CMD_HTML: &str = include_str!("cmd.html");
const DBQUERY_HTML: &str = include_str!("dbquery.html");
const RUNSTATS_HTML: &str = include_str!("run_stats.html");
const SHINY_HTML: &str = include_str!("shiny.html");
const MEMORIAL_HTML: &str = include_str!("memorial.html");
const SOULLINK_HTML: &str = include_str!("soullink.html");
const SOULLINK_MANAGE_HTML: &str = include_str!("soullink_manage.html");
const TYPES_HTML: &str = include_str!("types.html");
const ABOUT_HTML: &str = include_str!("about.html");
const COMPARE_HTML: &str = include_str!("compare.html");
const ITEMS_HTML: &str = include_str!("items.html");
const MOVES_HTML: &str = include_str!("moves.html");
const MOBILE_HTML: &str = include_str!("mobile.html");
const TRAINERS_HTML: &str = include_str!("trainers.html");
const TIMELINE_HTML: &str = include_str!("timeline.html");
const SPECIES_HTML: &str = include_str!("species.html");
const DEATHS_HTML: &str = include_str!("deaths.html");
const ENCOUNTER_COUNT_HTML: &str = include_str!("encounter_count.html");
const HP_HTML: &str = include_str!("hp.html");
const BADGES_HTML: &str = include_str!("badges.html");
const NEXT_GYM_HTML: &str = include_str!("next_gym.html");
const ENCOUNTER_TABLE_HTML: &str = include_str!("encounter_table.html");
const MONEY_HTML: &str = include_str!("money.html");
const PLAYTIME_HTML: &str = include_str!("playtime.html");
const GOALS_HTML: &str = include_str!("goals.html");
const VS_LEADER_HTML: &str = include_str!("vs_leader.html");

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Extracts the DB connection string from `WebState::db_conn`, returning an
/// error JSON response if none is configured. Used in every DB-backed handler.
macro_rules! require_db {
    ($state:expr) => {
        match $state.db_conn {
            Some(s) => s,
            None => return axum::Json(serde_json::json!({ "error": "No database configured" })),
        }
    };
}

const TESTING_BANNER: &str = r#"<div id="testing-banner" style="position:fixed;top:0;left:0;right:0;z-index:9999;background:#b00;color:#fff;font-weight:bold;text-align:center;padding:4px 0;font-family:sans-serif;font-size:14px;">[TESTING]</div>"#;

fn apply_page(html: &str, testing: bool) -> String {
    apply_page_with_theme(html, testing, None)
}

/// Renders an HTML page, injecting the version, optional testing banner,
/// and an optional theme by setting `data-theme` on `<html>` and replacing
/// the `<!-- THEME_SLOT -->` placeholder with a `<script>` that applies it.
///
/// Supported theme values: `dark` (default, no-op), `light`, and any custom
/// string that maps to a CSS `data-theme` attribute value.
fn apply_page_with_theme(html: &str, testing: bool, theme: Option<&str>) -> String {
    let html = html.replace("__VERSION__", VERSION);

    // Inject theme attribute and a tiny script that sets it before first paint,
    // preventing a flash of the default (dark) theme.
    //
    // Only themes whose names consist entirely of `[a-zA-Z0-9_-]` are accepted.
    // Any theme containing other characters is rejected and treated as the default
    // rather than silently concatenating the sanitized fragments (which would
    // produce confusing output and mask typos).
    let html = match theme {
        None | Some("dark") | Some("") => html.replace("<!-- THEME_SLOT -->", ""),
        Some(t) => {
            let all_safe = t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            let within_len = t.len() <= 32;
            if all_safe && within_len {
                let injection =
                    format!(r#"<script>document.documentElement.dataset.theme="{t}"</script>"#);
                html.replace("<!-- THEME_SLOT -->", &injection)
            } else {
                // Invalid theme — fall back to default (dark) silently.
                html.replace("<!-- THEME_SLOT -->", "")
            }
        }
    };

    if testing {
        html.replacen("<body>", &format!("<body>{}", TESTING_BANNER), 1)
    } else {
        html
    }
}

async fn serve_html(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(OVERLAY_HTML, state.testing, theme))
}

async fn serve_focused(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(FOCUSED_HTML, state.testing, theme))
}

async fn serve_party(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    if params.contains_key("plain-view") {
        Html(apply_page_with_theme(
            PARTY_PLAIN_HTML,
            state.testing,
            theme,
        ))
    } else {
        Html(apply_page_with_theme(FOCUSED_HTML, state.testing, theme))
    }
}

async fn serve_db_viewer(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(DBVIEWER_HTML, state.testing))
}

async fn serve_history(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(HISTORY_HTML, state.testing))
}

async fn serve_alerts(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(ALERTS_HTML, state.testing))
}

async fn serve_routes(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(ROUTES_HTML, state.testing))
}

async fn serve_db_json(State(state): State<WebState>) -> axum::Json<serde_json::Value> {
    let conn = match state.db_conn {
        Some(s) => s,
        None => return axum::Json(serde_json::json!({ "error": "No database configured" })),
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::dump_all(&conn)).await;
    axum::Json(result.unwrap_or_else(|e| {
        tracing::error!("db dump task failed: {e}");
        serde_json::json!({ "error": "Query failed" })
    }))
}

async fn clear_db(
    State(state): State<WebState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if params.get("confirm").map(String::as_str) != Some("true") {
        return (
            StatusCode::BAD_REQUEST,
            "Add ?confirm=true to confirm database wipe".to_string(),
        );
    }
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "No database configured".to_string(),
            );
        }
    };
    match tokio::task::spawn_blocking(move || fire_red_database::clear_all_records(&conn)).await {
        Ok(Ok(())) => (StatusCode::OK, "ok".to_string()),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Task panicked".to_string(),
        ),
    }
}

/// Returns the full current state as a JSON array of slot objects — same
/// payload the WebSocket would push on the next tick.
async fn api_state(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let json = state.tx.borrow().clone();
    let raw = if json.is_empty() { "[]".to_string() } else { json };
    let body = filter_slots_for_user(&raw, user.id).await;
    ([(header::CONTENT_TYPE, "application/json")], body)
}

/// Returns a single slot object by zero-based index, or 404 if out of range.
async fn api_slot(State(state): State<WebState>, Path(index): Path<usize>) -> impl IntoResponse {
    let json = state.tx.borrow().clone();
    let slots: serde_json::Value =
        serde_json::from_str(&json).unwrap_or(serde_json::Value::Array(vec![]));
    match slots.get(index) {
        Some(slot) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            slot.to_string(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "slot index out of range").into_response(),
    }
}

/// `GET /api/slot/:index/odds` — wildmon encounter table for the given slot's current map.
///
/// Returns the full [`WildPokemonHeader`] for whichever map the tracker in that
/// slot is currently on, broken down by encounter type (land, water, rock-smash,
/// fishing). Each encounter entry includes species id, min/max level, and the
/// party-wide encounter rate for the type.
///
/// Returns `{ "error": "..." }` if the slot is out of range, disconnected, or
/// the current map has no wild encounters.
async fn api_slot_odds(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return axum::Json(serde_json::json!({ "error": "slot index out of range" })),
    };
    let gs = match slot.state.lock_or_recover().clone() {
        Some(gs) => gs,
        None => return axum::Json(serde_json::json!({ "error": "slot not connected" })),
    };
    let h = &gs.encounters;
    let make_list = |info: &fire_red_pokemon_data::WildPokemonInfo| -> serde_json::Value {
        if info.encounter_rate == 0 {
            return serde_json::Value::Null;
        }
        serde_json::json!({
            "encounter_rate": info.encounter_rate,
            "slots": info.wild_pokemon_list.iter().map(|p| serde_json::json!({
                "species":    p.species,
                "min_level":  p.min_level,
                "max_level":  p.max_level,
            })).collect::<Vec<_>>()
        })
    };
    axum::Json(serde_json::json!({
        "map_group": h.map_group,
        "map_name":  h.map_num,
        "land":        make_list(&h.land_mon_encounters),
        "water":       make_list(&h.water_mon_encounters),
        "rock_smash":  make_list(&h.rock_smash_encounters),
        "fishing":     make_list(&h.fishing_encounters),
    }))
}

/// Returns a plain-text one-line summary of a tracker slot, suitable for chat
/// bots or stream commands. Format: `"<Player> — <HP>/<MaxHP> — <MapName>"`.
/// Returns `"Slot <n> not found"` or `"Slot <n> not connected"` on error.
async fn api_bot_summary(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(index): Path<usize>,
) -> String {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return format!("Slot {index} not found"),
    };
    if let Some(rid) = slot.db.as_ref().and_then(|db| db.get_run_id()) {
        let uid = user.id;
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);
        if !can {
            return format!("Slot {index} not found");
        }
    }
    let gs = match slot.state.lock_or_recover().clone() {
        Some(gs) => gs,
        None => return format!("Slot {index} not connected"),
    };
    let player = &gs.player_name;
    let map = if gs.zone_name.is_empty() {
        "Unknown location"
    } else {
        &gs.zone_name
    };
    let (hp, max_hp) = gs.party.first().map(|p| (p.hp, p.max_hp)).unwrap_or((0, 0));
    format!("{player} — {hp}/{max_hp} HP — {map}")
}

async fn serve_compare(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(COMPARE_HTML, state.testing))
}

async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let show = params.get("show").cloned();
    let user_id = user.id;
    ws.on_upgrade(move |socket| {
        handle_socket(socket, state.tx.subscribe(), state.live_slots, show, user_id)
    })
}

/// Strips fields from a slot-array JSON string that the given `show` view does
/// not render, reducing per-tick payload size for narrow views.
fn filter_slots_json(json: &str, show: &str) -> String {
    let strip: &[&str] = match show {
        "box" => &[
            "party",
            "encounters",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        "dead" => &["encounters", "box_pokemon", "caught", "prev_run_encounters"],
        "caught" => &["encounters", "box_pokemon", "dead", "prev_run_encounters"],
        "memorial" => &[
            "encounters",
            "box_pokemon",
            "caught",
            "prev_run_encounters",
            "db_encounters",
        ],
        "soullink" => &[
            "encounters",
            "box_pokemon",
            "db_encounters",
            "prev_run_encounters",
        ],
        // types page only needs party (with type fields), badge state, and next_gym.
        "types" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // deaths overlay only needs the dead list and run_summary.
        "deaths" => &[
            "party",
            "encounters",
            "box_pokemon",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // counter overlay only needs db_encounters for counts.
        "counter" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "prev_run_encounters",
        ],
        // hp overlay only needs party (hp/status) and badges.
        "hp" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // badges overlay only needs badges.
        "badges" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // nextgym overlay needs party (types) + next_gym + badges.
        "nextgym" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // encounter_table overlay only needs encounters (with species_name/rate).
        "encounter_table" => &[
            "party",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // money overlay only needs the money field.
        "money" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // playtime overlay needs play_time_* and run_summary (for wall-clock).
        "playtime" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // goals overlay only needs goals list.
        "goals" => &[
            "party",
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        // vs_leader overlay needs next_gym + leader_party + party types.
        "vs_leader" => &[
            "encounters",
            "box_pokemon",
            "dead",
            "caught",
            "db_encounters",
            "prev_run_encounters",
        ],
        _ => return json.to_owned(),
    };
    let Ok(mut slots) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return json.to_owned();
    };
    for slot in &mut slots {
        if let Some(obj) = slot.as_object_mut() {
            for key in strip {
                obj.remove(*key);
            }
        }
    }
    serde_json::to_string(&slots).unwrap_or_else(|_| json.to_owned())
}

async fn handle_socket(
    socket: axum::extract::ws::WebSocket,
    mut rx: watch::Receiver<String>,
    live_slots: SharedSlots,
    show: Option<String>,
    user_id: u32,
) {
    // Pre-fetch the set of run IDs this user can access once at connect time
    // so we can filter every broadcast tick without hitting the DB.
    let accessible: HashSet<u32> =
        tokio::task::spawn_blocking(move || fire_red_database::get_accessible_run_ids(user_id))
            .await
            .unwrap_or(Ok(HashSet::new()))
            .unwrap_or_default();

    // Filter helper: replaces inaccessible slots with null (preserving array
    // positions so /:index/ overlay URLs remain stable), then applies the
    // show-filter on top.
    let filter_json = |raw: &str| -> String {
        let arr: serde_json::Value =
            serde_json::from_str(raw).unwrap_or(serde_json::Value::Array(vec![]));
        let user_slots: Vec<serde_json::Value> = arr
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|s| match s.get("active_run_id").and_then(|v| v.as_u64()) {
                None => s,
                Some(rid) if accessible.contains(&(rid as u32)) => s,
                _ => serde_json::Value::Null,
            })
            .collect();
        let filtered =
            serde_json::to_string(&serde_json::Value::Array(user_slots)).unwrap_or_default();
        match &show {
            Some(s) => filter_slots_json(&filtered, s),
            None => filtered,
        }
    };

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Send current state immediately so the browser isn't blank on connect.
    {
        let current = rx.borrow_and_update().clone();
        if !current.is_empty() {
            let msg = filter_json(&current);
            if ws_tx
                .send(axum::extract::ws::Message::Text(msg))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // Forward incoming browser commands only to slots accessible by this user.
    let live_slots_cmd = live_slots.clone();
    let accessible_cmd = {
        // Re-fetch accessible run IDs for the command-forwarding closure.
        tokio::task::spawn_blocking(move || fire_red_database::get_accessible_run_ids(user_id))
            .await
            .unwrap_or(Ok(HashSet::new()))
            .unwrap_or_default()
    };
    tokio::spawn(async move {
        while let Some(Ok(axum::extract::ws::Message::Text(text))) = ws_rx.next().await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                let msg = match val["cmd"].as_str().unwrap_or("") {
                    "end_run" => Some(ClientMessage::EndRun),
                    "new_run" => Some(ClientMessage::NewRun),
                    _ => None,
                };
                if let Some(msg) = msg {
                    let slots = live_slots_cmd.lock_or_recover().clone();
                    for slot in &slots {
                        let run_id = slot.db.as_ref().and_then(|db| db.get_run_id());
                        let allowed = match run_id {
                            None => true,
                            Some(rid) => accessible_cmd.contains(&rid),
                        };
                        if allowed {
                            slot.command_queue.lock_or_recover().push_back(msg.clone());
                        }
                    }
                }
            }
        }
    });

    // Push state updates whenever the broadcast channel changes.
    loop {
        if rx.changed().await.is_err() {
            break;
        }
        let raw = rx.borrow_and_update().clone();
        let msg = filter_json(&raw);
        if ws_tx
            .send(axum::extract::ws::Message::Text(msg))
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn serve_cmd(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(CMD_HTML, state.testing))
}

async fn serve_db_query(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(DBQUERY_HTML, state.testing))
}

async fn serve_run_stats(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(RUNSTATS_HTML, state.testing))
}

async fn serve_shiny(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SHINY_HTML, state.testing))
}

async fn serve_memorial(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(MEMORIAL_HTML, state.testing))
}

async fn serve_soullink(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SOULLINK_HTML, state.testing))
}

async fn serve_types_page(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(TYPES_HTML, state.testing, theme))
}

async fn serve_about(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(ABOUT_HTML, state.testing))
}

/// `GET /api/run/:id/stats` — per-run statistics JSON.
async fn api_run_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::run_stats(&conn, run_id)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/route_stats` — per-route catch-rate statistics JSON.
async fn api_run_route_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::route_stats(&conn, run_id)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/export` — full run export.
///
/// - Without query params (or `?format=json`): returns the full run as JSON
///   (metadata, caught, dead, encounters).
/// - `?format=csv`: returns three CSV sections (caught, dead, encounters) joined
///   by blank lines. Content-Type is `text/csv`.
async fn api_run_export(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response();
        }
    };
    if params.get("format").map(|s| s.as_str()) == Some("csv") {
        let result =
            tokio::task::spawn_blocking(move || fire_red_database::export_run_csv(&conn, run_id))
                .await;
        match result {
            Ok(Ok(csv)) => (
                [
                    ("content-type", "text/csv"),
                    (
                        "content-disposition",
                        &format!("attachment; filename=\"run_{run_id}.csv\""),
                    ),
                ],
                csv,
            )
                .into_response(),
            Ok(Err(e)) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
            Err(_) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Task panicked",
            )
                .into_response(),
        }
    } else {
        let result =
            tokio::task::spawn_blocking(move || fire_red_database::export_run(&conn, run_id)).await;
        axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
            .into_response()
    }
}

/// `GET /api/run/:id/route_odds` — encountered and unencountered wild areas for a run.
///
/// Returns `encountered` (routes already visited with species and catch info)
/// and `unencountered` (all known FireRed wild areas not yet recorded).
async fn api_run_route_odds(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::route_odds_json(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/webhook_log` — webhook delivery receipt log for a run.
async fn api_run_webhook_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::get_webhook_log_json(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/soul_link/overrides` — list all manual soul-link overrides for a run.
async fn api_run_soul_link_overrides(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::soul_link_overrides_json(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `POST /api/run/:id/soul_link/override` — set a manual soul-link pairing.
///
/// Body: `{ "personality": <u32>, "partner_personality": <u32> }`.
/// Replaces any existing override for the same `personality`.
async fn api_set_soul_link_override(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let personality = match body["personality"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
    {
        Some(v) => v,
        None => {
            return axum::Json(serde_json::json!({ "error": "Missing or invalid 'personality'" }));
        }
    };
    let partner_personality = match body["partner_personality"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
    {
        Some(v) => v,
        None => {
            return axum::Json(
                serde_json::json!({ "error": "Missing or invalid 'partner_personality'" }),
            );
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::set_soul_link_override_by_run(
            &conn,
            run_id,
            personality,
            partner_personality,
        )
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `DELETE /api/run/:id/soul_link/override/:personality` — remove a manual override.
async fn api_clear_soul_link_override(
    State(state): State<WebState>,
    Path((run_id, personality)): Path<(u32, u64)>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let p = match u32::try_from(personality) {
        Ok(v) => v,
        Err(_) => return axum::Json(serde_json::json!({ "error": "personality out of range" })),
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::clear_soul_link_override_by_run(&conn, run_id, p)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /soullink/manage` — Soul Link partner override management page.
async fn serve_soullink_manage(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SOULLINK_MANAGE_HTML, state.testing))
}

/// `GET /:index/items` — bag item viewer page for a specific tracker slot.
async fn serve_items(
    State(state): State<WebState>,
    Path(_index): Path<usize>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(ITEMS_HTML, state.testing, theme))
}

/// `GET /:index/moves` — move / PP overlay for a specific tracker slot.
async fn serve_moves_page(
    State(state): State<WebState>,
    Path(_index): Path<usize>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(MOVES_HTML, state.testing, theme))
}

/// `GET /party/mobile` — mobile-friendly party viewer.
async fn serve_mobile_party(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(MOBILE_HTML, state.testing))
}

/// `GET /timeline` and `GET /run/:id/timeline` — visual run timeline page.
async fn serve_timeline(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(TIMELINE_HTML, state.testing))
}

/// `GET /species` — cross-run per-species survival statistics page.
async fn serve_species(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SPECIES_HTML, state.testing))
}

/// `GET /api/species/stats` — cross-run per-species survival statistics JSON.
async fn api_species_stats(State(state): State<WebState>) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || fire_red_database::species_stats(&conn)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /trainers` and `GET /run/:id/trainers` — trainer battle log page.
async fn serve_trainers(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(TRAINERS_HTML, state.testing))
}

/// `GET /:index/deaths` — compact death-counter overlay for a small OBS Browser Source.
///
/// Shows a large red death count. Subscribes to `?show=deaths` WS filter so
/// the browser only receives the `dead` list and `run_summary`; no party,
/// encounter, or box data is transferred.
async fn serve_deaths_overlay(
    State(state): State<WebState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(|s| s.as_str());
    Html(apply_page_with_theme(DEATHS_HTML, state.testing, theme))
}

/// `GET /:index/encounter_count` — encounter counter overlay for OBS.
///
/// Shows the total encounter count for the run with a caught/missed breakdown.
/// Subscribes to `?show=counter` WS filter (only `db_encounters` transferred).
async fn serve_encounter_count(
    State(state): State<WebState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(|s| s.as_str());
    Html(apply_page_with_theme(ENCOUNTER_COUNT_HTML, state.testing, theme))
}

/// `GET /api/run/:id/trainers` — trainer battle log JSON for a run.
async fn api_run_trainers(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_trainer_defeats_json(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/slot/:index/bag` — bag pockets JSON for a specific tracker slot.
async fn api_bag(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return axum::Json(serde_json::json!({ "error": "slot index out of range" })),
    };
    match slot.bag_data.lock_or_recover().clone() {
        Some(pockets) => axum::Json(serde_json::json!({
            "items":     pockets.items.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
            "key_items": pockets.key_items.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
            "balls":     pockets.balls.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
            "tms":       pockets.tms.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
        })),
        None => axum::Json(serde_json::json!({ "error": "bag data not yet available" })),
    }
}

/// `GET /api/run/:id/shiny` — shiny odds statistics JSON for a run.
async fn api_shiny_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::shiny_stats(&conn, run_id)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/timeline` — chronological event log for the **active** run.
///
/// Includes both a Unix integer timestamp (`occurred_at`) and a human-readable
/// `occurred_at_human` string.
///
/// Status codes:
/// - `200 OK`                  — timeline returned successfully.
/// - `404 Not Found`           — no run is currently active.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
async fn api_active_timeline(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let uid = user.id;
    let result =
        tokio::task::spawn_blocking(move || {
            fire_red_database::active_run_timeline_for_user_json(&conn, uid)
        })
            .await
            .unwrap_or_else(|_| {
                Err(fire_red_database::EventsError::QueryFailed(
                    "Task panicked".into(),
                ))
            });

    match result {
        Ok(body) => (StatusCode::OK, axum::Json(body)).into_response(),
        Err(fire_red_database::EventsError::NoActiveRun) => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "no active run" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/run/:id/events` — chronological event log for a run.
///
/// Status codes:
/// - `200 OK`                  — events returned.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
async fn api_run_events(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::list_events_json(&conn, run_id))
            .await
            .unwrap_or_else(|_| {
                Err(fire_red_database::EventsError::QueryFailed(
                    "Task panicked".into(),
                ))
            });

    match result {
        Ok(body) => (StatusCode::OK, axum::Json(body)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/runs` — summary list of runs accessible to the authenticated user.
async fn api_runs(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::list_runs_for_user_json(&conn, uid)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `POST /api/run/import` — import a run from the JSON format produced by `/api/run/:id/export`.
///
/// Creates a new run with a fresh id and re-inserts caught, dead, and encounter records.
/// The imported run is linked to the authenticated user. Returns `{ "run_id": <new_id> }`.
async fn api_run_import(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let result = tokio::task::spawn_blocking(move || {
        let val = fire_red_database::import_run(&conn, &body);
        if let Some(run_id) = val.get("run_id").and_then(|v| v.as_u64()).map(|v| v as u32) {
            let _ = fire_red_database::link_run_to_user(run_id, uid);
        }
        val
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// Returns the current time as a Unix timestamp (seconds).
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `POST /api/slot/:index/give_item` — inject an item into the player's bag.
///
/// Body: `{ "item_id": <u16>, "quantity": <u16 1–99> }`.
///
/// Queues a [`ClientMessage::GiveItem`] for the tracker in the given slot, which
/// writes the item directly into the in-memory items pocket via RetroArch's
/// `WRITE_CORE_MEMORY` command. The write happens asynchronously on the tracker
/// side; this endpoint returns 200 as soon as the command is enqueued.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but tracker is not connected.
async fn api_give_item(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let item_id = match body["item_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) if v > 0 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "item_id must be a positive u16".to_string(),
            );
        }
    };
    let quantity = match body["quantity"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
    {
        Some(v) if v > 0 && v <= 99 => v,
        _ => return (StatusCode::BAD_REQUEST, "quantity must be 1–99".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::GiveItem { item_id, quantity });
    let rom = fire_red_rom_buffer::get_rom();
    let item_name = fire_red_party_monitor::get_item_string_from_id(rom, item_id);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "give_item",
            "label": format!("Gave {quantity}× {item_name}"),
        }));
    (
        StatusCode::OK,
        format!("queued give_item item_id={item_id} quantity={quantity} for slot {index}"),
    )
}

/// `POST /api/slot/:index/make_shiny` — make a party Pokémon shiny in-memory.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Queues a [`ClientMessage::MakeShiny`] for the tracker, which patches the
/// Pokémon's stored OT Secret ID so the Gen III shiny formula holds.
/// Nature, ability, gender, and all other personality-derived properties are
/// preserved. Returns 200 as soon as the command is enqueued.
async fn api_make_shiny(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::MakeShiny { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "make_shiny",
            "label": format!("Made party[{party_position}] shiny"),
        }));
    (
        StatusCode::OK,
        format!("queued make_shiny party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/take_item` — remove an item from the player's bag.
///
/// Body: `{ "item_id": <u16>, "quantity": <u16 1–99> }`.
///
/// Queues a [`ClientMessage::TakeItem`] for the tracker. If the current stack
/// quantity is ≤ `quantity` the item is fully removed and the pocket is
/// compacted; otherwise only the quantity is decremented. Returns 200 as soon
/// as the command is enqueued.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but tracker is not connected.
async fn api_take_item(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let item_id = match body["item_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) if v > 0 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "item_id must be a positive u16".to_string(),
            );
        }
    };
    let quantity = match body["quantity"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
    {
        Some(v) if v > 0 && v <= 99 => v,
        _ => return (StatusCode::BAD_REQUEST, "quantity must be 1–99".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::TakeItem { item_id, quantity });
    let rom = fire_red_rom_buffer::get_rom();
    let item_name = fire_red_party_monitor::get_item_string_from_id(rom, item_id);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "take_item",
            "label": format!("Took {quantity}× {item_name}"),
        }));
    (
        StatusCode::OK,
        format!("queued take_item item_id={item_id} quantity={quantity} for slot {index}"),
    )
}

/// `POST /api/slot/:index/change_species` — change a party Pokémon's species.
///
/// Body: `{ "party_position": <u8 0–5>, "new_species": <u16 1–386> }`.
///
/// Queues a [`ClientMessage::ChangeSpecies`] for the tracker, which decrypts
/// the party Pokémon's data block, updates the species field in the Growth
/// substructure, recalculates the checksum, and re-encrypts. Personality,
/// nickname, moves, EVs, IVs, nature, ability, and gender are all preserved.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but tracker is not connected.
async fn api_change_species(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let new_species = match body["new_species"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
    {
        Some(v) if v > 0 && v <= 386 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "new_species must be 1–386".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeSpecies {
            party_position,
            new_species,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_species",
            "label": format!("Changed party[{party_position}] to species #{new_species}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_species party_position={party_position} new_species={new_species} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_ability` — switch a party Pokémon's ability slot.
///
/// Body: `{ "party_position": <u8 0–5>, "ability_slot": <u8 0 or 1> }`.
///
/// Queues a [`ClientMessage::ChangeAbility`] for the tracker. Sets or clears
/// bit 31 of the IV/egg/ability word in the Misc substructure; all other fields
/// (species, EVs, IVs, moves, nature, personality) are preserved. The checksum
/// is recalculated and the data block re-encrypted.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but tracker is not connected.
async fn api_change_ability(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let ability_slot = match body["ability_slot"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v <= 1 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "ability_slot must be 0 or 1".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeAbility {
            party_position,
            ability_slot,
        });
    let ability_label = if ability_slot == 0 {
        "primary"
    } else {
        "secondary"
    };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_ability",
            "label": format!("Party[{party_position}] → {ability_label} ability"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_ability party_position={party_position} ability_slot={ability_slot} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_gender` — change the gender of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "target_gender": <u8 0 or 1> }` where
/// 0 = male and 1 = female.
///
/// Adjusts only the low byte of the personality, preserving nature
/// (personality % 25). If the Pokémon is currently shiny only bytes that keep
/// the shiny formula satisfied are considered; the command is rejected (logged as
/// a warning) if none exist for the requested gender.  Genderless and
/// fixed-gender species are also rejected.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but tracker is not connected.
async fn api_change_gender(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let target_gender = match body["target_gender"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v <= 1 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "target_gender must be 0 (male) or 1 (female)".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeGender {
            party_position,
            target_gender,
        });
    let gender_label = if target_gender == 0 { "male" } else { "female" };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_gender",
            "label": format!("Party[{party_position}] → {gender_label}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_gender party_position={party_position} target_gender={target_gender} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_nickname` — rename a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "nickname": <string, max 10 chars> }`.
///
/// The nickname is sent as UTF-8; the tracker converts it to GBA encoding and
/// silently drops unmapped characters. Only the 10-byte nickname field is written;
/// the encrypted data block (nature, IVs, etc.) is untouched.
async fn api_change_nickname(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let nickname = match body["nickname"].as_str() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "nickname must be a non-empty string".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeNickname {
            party_position,
            nickname: nickname.clone(),
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_nickname",
            "label": format!("Renamed party[{party_position}] to \"{nickname}\""),
        }));
    (
        StatusCode::OK,
        format!("queued change_nickname party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/change_held_item` — set the held item of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "item_id": <u16> }`.
/// Use `item_id = 0` to remove the held item.
///
/// Decrypts the Growth substructure, writes the held-item field, recalculates
/// the checksum, and re-encrypts. All other data is preserved.
async fn api_change_held_item(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let item_id = match body["item_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "item_id must be a u16 (0 = remove)".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeHeldItem {
            party_position,
            item_id,
        });
    let label = if item_id == 0 {
        format!("Removed party[{party_position}] held item")
    } else {
        let rom = fire_red_rom_buffer::get_rom();
        let item_name = fire_red_party_monitor::get_item_string_from_id(rom, item_id);
        format!("party[{party_position}] now holds {item_name}")
    };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_held_item", "label": label,
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_held_item party_position={party_position} item_id={item_id} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/cure_status` — clear the status condition of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Writes 4 zero bytes to the status word (bytes 80–83 of the PartyPokemon
/// struct), clearing burn, sleep turn counter, paralysis, poison, freeze,
/// and Toxic stage in one write.
async fn api_cure_status(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::CureStatus { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "cure_status",
            "label": format!("Cured party[{party_position}] status"),
        }));
    (
        StatusCode::OK,
        format!("queued cure_status party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/change_nature` — change the nature of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "nature": <u8 0–24> }`.
///
/// Adjusts the low byte of the personality to satisfy `personality % 25 ==
/// nature` while preserving the current gender (for species with
/// personality-derived gender) and shiny status. Substructures are rearranged
/// when `personality % 24` changes. Returns `200` with an explanatory message
/// if no single low byte satisfies all constraints simultaneously.
///
/// Nature indices: 0=Hardy 1=Lonely 2=Brave 3=Adamant 4=Naughty 5=Bold
/// 6=Docile 7=Relaxed 8=Impish 9=Lax 10=Timid 11=Hasty 12=Serious 13=Jolly
/// 14=Naive 15=Modest 16=Mild 17=Quiet 18=Bashful 19=Rash 20=Calm 21=Gentle
/// 22=Sassy 23=Careful 24=Quirky
async fn api_change_nature(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let nature = match body["nature"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if v <= 24 => v,
        _ => return (StatusCode::BAD_REQUEST, "nature must be 0–24".to_string()),
    };
    const NATURE_NAMES: [&str; 25] = [
        "Hardy", "Lonely", "Brave", "Adamant", "Naughty", "Bold", "Docile", "Relaxed", "Impish",
        "Lax", "Timid", "Hasty", "Serious", "Jolly", "Naive", "Modest", "Mild", "Quiet", "Bashful",
        "Rash", "Calm", "Gentle", "Sassy", "Careful", "Quirky",
    ];
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeNature {
            party_position,
            target_nature: nature,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_nature",
            "label": format!("Party[{party_position}] → {} nature", NATURE_NAMES[nature as usize]),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_nature party_position={party_position} nature={nature} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/restore_pp` — restore all move PP to current maximums.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Decrypts the Attacks and Growth substructures, computes maximum PP for each
/// equipped move slot (base PP + PP-Up bonus), and writes the result back.
/// Personality, nature, shiny status, and all other data are untouched.
async fn api_restore_pp(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::RestorePp { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "restore_pp",
            "label": format!("Restored party[{party_position}] PP"),
        }));
    (
        StatusCode::OK,
        format!("queued restore_pp party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_friendship` — set the friendship (happiness) byte.
///
/// Body: `{ "party_position": <u8 0–5>, "friendship": <u8 0–255> }`.
///
/// Common values: 0 = min (max Frustration damage), 255 = max (Happiness
/// evolutions trigger, max Return damage).
async fn api_set_friendship(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let friendship = match body["friendship"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "friendship must be 0–255".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetFriendship {
            party_position,
            friendship,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_friendship",
            "label": format!("party[{party_position}] friendship → {friendship}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued set_friendship party_position={party_position} friendship={friendship} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_move` — replace a move slot.
///
/// Body: `{ "party_position": <u8 0–5>, "slot": <u8 0–3>, "move_id": <u16> }`.
///
/// PP is set to the new move's maximum (base PP + existing PP-Up bonus).
/// Use `move_id = 0` to clear the slot.
async fn api_change_move(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let move_slot = match body["slot"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if v <= 3 => v,
        _ => return (StatusCode::BAD_REQUEST, "slot must be 0–3".to_string()),
    };
    let move_id = match body["move_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "move_id must be a u16".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeMove {
            party_position,
            slot: move_slot,
            move_id,
        });
    let label = if move_id == 0 {
        format!("Cleared party[{party_position}] move slot {move_slot}")
    } else {
        format!("party[{party_position}] move {move_slot} → move_id {move_id}")
    };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_move", "label": label,
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_move party_position={party_position} slot={move_slot} move_id={move_id} for slot {index}"
        ),
    )
}

/// Shared stat-field parser for IV/EV handlers — extracts `hp/atk/def/spd/spa/spdef`
/// from a JSON body, returning an error string on the first missing or invalid field.
fn parse_six_stats(body: &serde_json::Value) -> Result<(u8, u8, u8, u8, u8, u8), String> {
    let mut vals = [0u8; 6];
    for (i, key) in ["hp", "atk", "def", "spd", "spa", "spdef"]
        .iter()
        .enumerate()
    {
        vals[i] = body[key]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| format!("{key} must be 0–255"))?;
    }
    Ok((vals[0], vals[1], vals[2], vals[3], vals[4], vals[5]))
}

/// `POST /api/slot/:index/set_ivs` — set all six IVs of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
/// Values are clamped to 31 by the tracker. Egg and ability bits are preserved.
async fn api_set_ivs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetIvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_ivs",
            "label": format!("party[{party_position}] IVs → {hp}/{atk}/{def}/{spd}/{spa}/{spdef}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_ivs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/increase_ivs` — add to each IV, clamping at 31.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
async fn api_increase_ivs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::IncreaseIvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events.lock_or_recover().push_back(serde_json::json!({
        "at": now_secs(), "kind": "increase_ivs",
        "label": format!("party[{party_position}] IVs +{hp}/+{atk}/+{def}/+{spd}/+{spa}/+{spdef}"),
    }));
    (
        StatusCode::OK,
        format!("queued increase_ivs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_evs` — set all six EVs of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8 0–255>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
/// The 510-total game cap is not enforced. Contest-condition bytes are preserved.
async fn api_set_evs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetEvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_evs",
            "label": format!("party[{party_position}] EVs → {hp}/{atk}/{def}/{spd}/{spa}/{spdef}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_evs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/increase_evs` — add to each EV, clamping at 255.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
async fn api_increase_evs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::IncreaseEvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events.lock_or_recover().push_back(serde_json::json!({
        "at": now_secs(), "kind": "increase_evs",
        "label": format!("party[{party_position}] EVs +{hp}/+{atk}/+{def}/+{spd}/+{spa}/+{spdef}"),
    }));
    (
        StatusCode::OK,
        format!("queued increase_evs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/restore_hp` — restore a party Pokémon's current HP to maximum.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Reads the calculated max-HP word from PartyPokemon offset 88–89 and writes
/// it to offset 86–87. No encrypted data block is touched.
async fn api_restore_hp(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::RestoreHp { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "restore_hp",
            "label": format!("Restored party[{party_position}] HP to full"),
        }));
    (
        StatusCode::OK,
        format!("queued restore_hp party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/heal_party` — restore HP and cure status for the whole party.
///
/// No request body required.
///
/// The tracker reuses a single UDP socket and processes all six party slots in
/// one pass: zeroes the status word and writes max HP to current HP for every
/// occupied slot.
async fn api_heal_party(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::HealParty);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "heal_party",
            "label": "Full party heal (HP + status)",
        }));
    (
        StatusCode::OK,
        format!("queued heal_party for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_exp` — set the experience points of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "exp": <u32> }`.
/// Updates the Growth substructure; the level byte is not changed.
async fn api_set_exp(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let exp = match body["exp"].as_u64().and_then(|v| u32::try_from(v).ok()) {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "exp must be a u32".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetExp {
            party_position,
            exp,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_exp",
            "label": format!("party[{party_position}] exp → {exp}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_exp party_position={party_position} exp={exp} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_level` — set the level of a party Pokémon (1–100).
///
/// Body: `{ "party_position": <u8 0–5>, "level": <u8 1–100> }`.
/// Writes both the level byte and updates the experience in the Growth
/// substructure to the Gen III minimum for the target level.
async fn api_set_level(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let level = match body["level"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if (1..=100).contains(&v) => v,
        _ => return (StatusCode::BAD_REQUEST, "level must be 1–100".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetLevel {
            party_position,
            level,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_level",
            "label": format!("party[{party_position}] → level {level}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_level party_position={party_position} level={level} for slot {index}"),
    )
}

/// `POST /api/slot/:index/learn_move` — add a move to the first empty move slot.
///
/// Body: `{ "party_position": <u8 0–5>, "move_id": <u16> }`.
/// No-op if the Pokémon already knows the move or all four slots are occupied.
async fn api_learn_move(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let move_id = match body["move_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) if v > 0 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "move_id must be a non-zero u16".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::LearnMove {
            party_position,
            move_id,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "learn_move",
            "label": format!("party[{party_position}] learn move_id={move_id}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued learn_move party_position={party_position} move_id={move_id} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/forget_move` — clear a move slot and compact.
///
/// Body: `{ "party_position": <u8 0–5>, "slot": <u8 0–3> }`.
/// Clears the move at `slot` and shifts subsequent moves left to fill the gap.
async fn api_forget_move(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let move_slot = match body["slot"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if v < 4 => v,
        _ => return (StatusCode::BAD_REQUEST, "slot must be 0–3".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ForgetMove {
            party_position,
            slot: move_slot,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "forget_move",
            "label": format!("party[{party_position}] forget slot {move_slot}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued forget_move party_position={party_position} slot={move_slot} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/set_pokerus` — infect a party Pokémon with Pokérus.
///
/// Body: `{ "party_position": <u8 0–5> }`.
/// Sets Pokérus to strain 1, 4 days remaining. No-op if already actively infected.
async fn api_set_pokerus(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetPokerus { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_pokerus",
            "label": format!("party[{party_position}] infected with Pokérus"),
        }));
    (
        StatusCode::OK,
        format!("queued set_pokerus party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_pp_ups` — set PP-Up counts for all four move slots.
///
/// Body: `{ "party_position": <u8 0–5>, "pp0": <u8 0–3>, "pp1": <u8 0–3>,
///          "pp2": <u8 0–3>, "pp3": <u8 0–3> }`.
/// Sets the PP-Up bonus for each slot and refills current PP to the new maximum.
async fn api_set_pp_ups(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let parse_pp = |key: &str| -> Option<u8> {
        body[key].as_u64().and_then(|v| u8::try_from(v).ok()).filter(|&v| v <= 3)
    };
    let pp0 = match parse_pp("pp0") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp0 must be 0–3".to_string()),
    };
    let pp1 = match parse_pp("pp1") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp1 must be 0–3".to_string()),
    };
    let pp2 = match parse_pp("pp2") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp2 must be 0–3".to_string()),
    };
    let pp3 = match parse_pp("pp3") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp3 must be 0–3".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetPpUps {
            party_position,
            pp0,
            pp1,
            pp2,
            pp3,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_pp_ups",
            "label": format!(
                "party[{party_position}] PP-Ups → ({pp0},{pp1},{pp2},{pp3})"
            ),
        }));
    (
        StatusCode::OK,
        format!(
            "queued set_pp_ups party_position={party_position} \
             pp0={pp0},pp1={pp1},pp2={pp2},pp3={pp3} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/revive_pokemon` — revive a dead Pokémon into a party slot.
///
/// Body: `{ "party_position": <u8 0–5>, "personality": <u32> }`.
/// Looks up the Pokémon by `personality` in the current run's `dead_pokemon`
/// table and writes it at `party_position` with 1 HP.
async fn api_revive_pokemon(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let personality = match body["personality"].as_u64().and_then(|v| u32::try_from(v).ok()) {
        Some(v) if v != 0 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "personality must be a non-zero u32".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::RevivePokemon {
            party_position,
            personality,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "revive_pokemon",
            "label": format!(
                "party[{party_position}] ← revive personality={personality:#010x}"
            ),
        }));
    (
        StatusCode::OK,
        format!(
            "queued revive_pokemon party_position={party_position} \
             personality={personality:#010x} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/undo` — revert the last injection command for the given slot.
///
/// Sends [`ClientMessage::UndoLastCommand`] to the tracker, which writes the
/// bytes that were captured before the last `write_to_retroarch` call back to
/// RetroArch memory.  No-op if no injection command has been executed since the
/// tracker connected.
///
/// - `200 OK` — command enqueued.
/// - `403 Forbidden` — injection commands are disabled.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot not connected.
async fn api_undo(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::UndoLastCommand);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "undo",
            "label": "undo last command",
        }));
    (
        StatusCode::OK,
        format!("queued undo for slot {index}"),
    )
}

/// `GET /api/runs/compare?ids=1,2,3` — side-by-side stats for multiple runs.
///
/// Query param `ids` is a comma-separated list of run IDs (max 20).
async fn api_runs_compare(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let ids_str = match params.get("ids") {
        Some(s) => s.clone(),
        None => return axum::Json(serde_json::json!({ "error": "Missing 'ids' query parameter" })),
    };
    let requested: Vec<u32> = ids_str
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .take(20)
        .collect();
    if requested.is_empty() {
        return axum::Json(serde_json::json!({ "error": "No valid run IDs provided" }));
    }
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    let run_ids: Vec<u32> = requested.into_iter().filter(|id| accessible.contains(id)).collect();
    if run_ids.is_empty() {
        return axum::Json(serde_json::json!({ "error": "No accessible run IDs provided" }));
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::run_comparison(&conn, &run_ids)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/luck` — luck/RNG analysis for a single run.
///
/// Returns shiny rate vs expected (1/8192), per-area encounter list.
async fn api_run_luck(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::run_luck_stats(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/catch_rate?species=X&hp=Y&max_hp=Z&status=W&ball=B`
///
/// Computes the Gen III catch probability using the ROM's species catch rate.
///
/// - `species` — species ID (1–386)
/// - `hp` — current HP
/// - `max_hp` — max HP
/// - `status` — `none` | `sleep` | `freeze` | `paralyze` | `poison` | `burn`
///   (default: `none`)
/// - `ball` — `pokeball` | `greatball` | `ultraball` | `masterball` |
///   `safariball` | `netball` | `nestball` | `repeatball` |
///   `timerball` | `diveball` | `premierball` (default: `pokeball`)
async fn api_catch_rate(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let parse_u16 = |key: &str| -> Option<u16> {
        params.get(key)?.parse::<u16>().ok()
    };
    let species = match parse_u16("species") {
        Some(s) if s > 0 && s <= MAX_NATIONAL_DEX_FIRERED => s,
        _ => {
            return axum::Json(
                serde_json::json!({ "error": "species must be 1–386" }),
            );
        }
    };
    let hp = match parse_u16("hp") {
        Some(v) => v,
        None => return axum::Json(serde_json::json!({ "error": "hp required" })),
    };
    let max_hp = match parse_u16("max_hp") {
        Some(v) if v > 0 => v,
        _ => return axum::Json(serde_json::json!({ "error": "max_hp must be > 0" })),
    };

    let status = params.get("status").map(|s| s.as_str()).unwrap_or("none");
    let (status_num, status_label): (u32, &str) = match status {
        "sleep" | "freeze" => (15, status),
        "paralyze" | "poison" | "burn" => (12, status),
        _ => (10, "none"),
    };

    let ball = params.get("ball").map(|s| s.as_str()).unwrap_or("pokeball");
    let (ball_num, ball_label): (u32, &str) = match ball {
        "masterball"  => (255, "masterball"),
        "ultraball"   => (20, "ultraball"),   // 2.0 × 10
        "greatball"   => (15, "greatball"),   // 1.5 × 10
        "safariball"  => (15, "safariball"),
        "netball"     => (30, "netball"),     // 3.0 × 10
        "nestball"    => (10, "nestball"),    // simplified to 1.0
        "repeatball"  => (30, "repeatball"),  // 3.0 × 10
        "timerball"   => (40, "timerball"),   // max 4.0 × 10
        "diveball"    => (35, "diveball"),    // 3.5 × 10
        "premierball" => (10, "premierball"),
        _             => (10, "pokeball"),
    };

    // Look up catch rate from ROM base stats (28 bytes/entry, catch_rate at byte 8).
    const BASE_STATS_SIZE: usize = 28;
    const CATCH_RATE_OFFSET: usize = 8;
    let catch_rate = if let Some(rom) = fire_red_rom_buffer::try_get_rom() {
        let addrs = fire_red_rom_buffer::get_rom_addresses();
        let off = addrs.base_stats_addr + species as usize * BASE_STATS_SIZE + CATCH_RATE_OFFSET;
        rom.get(off).copied().unwrap_or(45)
    } else {
        45 // fallback: average catch rate if ROM not loaded
    };

    // Gen III modified catch rate:
    //   a = floor((3*M - 2*H) * rate * ball_num/10) / (3*M) * status_num/10
    // where M=max_hp, H=hp. We use u64 to avoid overflow.
    let m = max_hp as u64;
    let h = hp.min(max_hp) as u64;
    let numer = (3 * m - 2 * h) * (catch_rate as u64) * (ball_num as u64);
    let denom = 3 * m * 10;
    let a_raw = numer / denom;
    let a = (a_raw * status_num as u64 / 10).min(255);

    let guaranteed = a >= 255 || ball_num >= 255 * 10;
    let catch_probability_pct = if guaranteed {
        100.0f64
    } else {
        // b = floor(65536 / (255/a)^0.25)
        let b = (65536.0 / (255.0 / a as f64).powf(0.25)) as u64;
        let b = b.min(65535) as f64;
        // P = (b/65536)^4
        let p = (b / 65536.0).powi(4);
        (p * 10000.0).round() / 100.0
    };

    axum::Json(serde_json::json!({
        "species": species,
        "catch_rate": catch_rate,
        "hp": hp,
        "max_hp": max_hp,
        "status": status_label,
        "status_bonus": status_num as f64 / 10.0,
        "ball": ball_label,
        "ball_bonus": ball_num as f64 / 10.0,
        "modified_catch_rate": a,
        "guaranteed": guaranteed,
        "catch_probability_pct": catch_probability_pct,
    }))
}

/// `GET /api/run/:id/closest_calls` — Pokémon that came closest to fainting.
///
/// Returns up to 50 entries ordered by lowest HP/max_HP ratio ever observed.
async fn api_run_closest_calls(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::closest_calls(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/pokemon/:personality/hp_history` — full HP timeline for one Pokémon.
///
/// Returns every HP change observed while the Pokémon was in the active party,
/// ordered oldest-first.
async fn api_run_pokemon_hp_history(
    State(state): State<WebState>,
    Path((run_id, personality)): Path<(u32, u32)>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_hp_history(&conn, run_id, personality)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/enemy_hp_log` — enemy Pokémon HP at start and end of each encounter.
///
/// Groups by enemy personality. Each entry shows initial HP, final HP, and
/// total damage dealt. Species name is inferred from the nearest first-encounter record.
async fn api_run_enemy_hp_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::get_enemy_hp_log(&conn, run_id))
            .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/battle_damage` — per-battle damage summary.
///
/// Groups damage events (HP decreases) across all party Pokémon into battles
/// using a 120-second gap threshold. Returns each battle's time window, which
/// Pokémon were involved, and how much damage each took.
async fn api_run_battle_damage(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_battle_damage_log(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/summary` — Markdown text recap for a completed (or in-progress) run.
///
/// Append `?format=text` to receive `text/plain` (Markdown source directly); omit it to
/// receive `{ "markdown": "..." }` JSON. Returns `404` when the run is not found.
async fn api_run_summary(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::response::AppendHeaders([("content-type", "text/plain")]),
                "No database configured".to_string(),
            )
                .into_response()
        }
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::run_summary_markdown(&conn, run_id))
            .await
            .unwrap_or_else(|_| Err("Task panicked".to_string()));

    match result {
        Err(e) if e.contains("not found") => (
            StatusCode::NOT_FOUND,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            e,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            e,
        )
            .into_response(),
        Ok(md) => {
            if params.get("format").map(|s| s.as_str()) == Some("text") {
                (
                    StatusCode::OK,
                    axum::response::AppendHeaders([("content-type", "text/plain; charset=utf-8")]),
                    md,
                )
                    .into_response()
            } else {
                axum::Json(serde_json::json!({ "markdown": md })).into_response()
            }
        }
    }
}

/// `PATCH /api/run/:id/event/:event_id/note` — set or replace a free-text
/// annotation on an event log entry.
///
/// Request body: `{ "note": "some text" }`.
/// Passing an empty string clears the annotation without deleting the event.
///
/// Status codes:
/// - `200 OK`                  — note saved.
/// - `400 Bad Request`         — body missing or `note` field not a string.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
async fn api_set_event_note(
    State(state): State<WebState>,
    Path((_run_id, event_id)): Path<(u32, i32)>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let Some(note) = body.get("note").and_then(|v| v.as_str()).map(|s| s.to_string()) else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "Missing or invalid 'note' field" })),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::set_event_note(&conn, event_id, &note))
            .await
            .unwrap_or_else(|_| Err("Task panicked".into()));
    match result {
        Ok(()) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// `DELETE /api/run/:id/event/:event_id/note` — clear the annotation on an
/// event log entry (equivalent to PATCH with `"note": ""`).
async fn api_clear_event_note(
    State(state): State<WebState>,
    Path((_run_id, event_id)): Path<(u32, i32)>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::set_event_note(&conn, event_id, ""))
            .await
            .unwrap_or_else(|_| Err("Task panicked".into()));
    match result {
        Ok(()) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// `GET /api/run/:id/pokepaste` — export the run's Pokémon in Pokepaste format.
///
/// Returns `text/plain` with living party members first (`# Living Party`) and
/// fallen members second (`# Fallen`). Move data is only available for fallen
/// members (the surviving-party snapshot is captured at catch time, before moves
/// are trained). Ideal for sharing party state on [Pokémon Showdown](https://pokepast.es/).
///
/// Status codes:
/// - `200 OK`                  — Pokepaste text returned.
/// - `503 Service Unavailable` — no database configured.
/// - `500 Internal Server Error` — DB connection or query failure.
async fn api_run_pokepaste(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            "No database configured".to_string(),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::pokepaste_export(&conn, run_id))
            .await
            .unwrap_or_else(|_| Err("Task panicked".into()));
    match result {
        Ok(text) => (
            StatusCode::OK,
            axum::response::AppendHeaders([("content-type", "text/plain; charset=utf-8")]),
            text,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::response::AppendHeaders([("content-type", "text/plain")]),
            e,
        )
            .into_response(),
    }
}

/// `GET /api/run/:id/splits` — badge split times for a run.
///
/// Returns the wall-clock timestamp, elapsed seconds from run start, and
/// seconds since the previous badge for each of the up to 8 gym badges (plus
/// the game-clear event if recorded).
async fn api_run_splits(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::badge_splits(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// `GET /api/run/:id/catch_log` — catch attempt log for a run.
///
/// Each Nuzlocke first-encounter attempt (per area) is recorded with the
/// species name, area, total Pokéballs thrown, and whether the catch succeeded.
/// Summary totals (`total_balls_thrown`, `most_balls_in_one_encounter`) are
/// included at the top level.
async fn api_run_catch_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::catch_attempt_log(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// `GET /api/run/:id/difficulty` — composite difficulty score for a run.
///
/// Returns a 0–100 score derived from death ratio (40 %), HP danger (30 %),
/// catch miss rate (20 %), and trainer battle load (10 %), plus the raw
/// component values and input counts used to compute them.
async fn api_run_difficulty(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::difficulty_score(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// `GET /api/run/:id/area_times` — per-area time breakdown for a run.
///
/// Groups `area_visits` rows by area name and sums the total seconds spent in
/// each area, sorted by time descending. Open visits (player currently in that
/// area) use the current time as the exit. Each entry also includes a
/// human-readable `formatted` string and the visit count.
async fn api_run_area_times(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::area_time_breakdown(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_run_death_map(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::death_map(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_run_level_curve(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::level_curve(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_run_move_usage(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::move_usage(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_run_friendship(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::friendship_history(&conn, run_id))
            .await
            .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_slot_ev_progress(
    State(state): State<WebState>,
    Path(slot_index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let slots = state.live_slots.lock_or_recover();
    let Some(slot) = slots.get(slot_index) else {
        return axum::Json(serde_json::json!({ "error": "Slot index out of range" }));
    };
    let game_state = slot.state.lock_or_recover();
    let Some(ref gs) = *game_state else {
        return axum::Json(serde_json::json!({ "error": "Slot not connected" }));
    };
    let ev_list: Vec<serde_json::Value> = gs.party.iter()
        .filter(|p| p.box_mon.secure.growth.species != 0)
        .map(|p| {
            let ev = &p.box_mon.secure.ev_condition;
            let total = ev.hp_ev as u32
                + ev.attack_ev as u32
                + ev.defense_ev as u32
                + ev.speed_ev as u32
                + ev.sp_attack_ev as u32
                + ev.sp_defense_ev as u32;
            let remaining_total = 510u32.saturating_sub(total);
            serde_json::json!({
                "personality": p.box_mon.personality,
                "nickname":    p.box_mon.nickname_string,
                "species":     p.box_mon.secure.growth.species_string,
                "hp":         ev.hp_ev,
                "attack":     ev.attack_ev,
                "defense":    ev.defense_ev,
                "speed":      ev.speed_ev,
                "sp_attack":  ev.sp_attack_ev,
                "sp_defense": ev.sp_defense_ev,
                "total":      total,
                "remaining":  remaining_total,
                "hp_capped":         ev.hp_ev >= 252,
                "attack_capped":     ev.attack_ev >= 252,
                "defense_capped":    ev.defense_ev >= 252,
                "speed_capped":      ev.speed_ev >= 252,
                "sp_attack_capped":  ev.sp_attack_ev >= 252,
                "sp_defense_capped": ev.sp_defense_ev >= 252,
                "fully_trained": total >= 510,
            })
        })
        .collect();
    axum::Json(serde_json::json!(ev_list))
}

/// Broadcasts a command to all connected tracker slots.
///
/// Supported commands (no request body needed — suitable for Stream Deck buttons):
///
/// | `cmd`       | Effect                                                   |
/// |-------------|----------------------------------------------------------|
/// | `end_run`   | End the active run for every connected player.           |
/// | `new_run`   | Start a new run for every connected player.              |
/// | `heal_all`  | Heal HP/PP/status of every party Pokémon for all slots.  |
async fn api_command(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(cmd): Path<String>,
) -> impl IntoResponse {
    let msg = match cmd.as_str() {
        "end_run"  => ClientMessage::EndRun,
        "new_run"  => ClientMessage::NewRun,
        "heal_all" => ClientMessage::HealParty,
        other => return (StatusCode::BAD_REQUEST, format!("Unknown command: {other}")),
    };
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    let slots = state.live_slots.lock_or_recover().clone();
    let mut count = 0usize;
    for slot in &slots {
        let run_id = slot.db.as_ref().and_then(|db| db.get_run_id());
        if run_id.is_none_or(|rid| accessible.contains(&rid)) {
            slot.command_queue.lock_or_recover().push_back(msg.clone());
            count += 1;
        }
    }
    (
        StatusCode::OK,
        format!("Command '{cmd}' sent to {count} slot(s)"),
    )
}

/// Sends a no-body command to a single tracker slot. Designed for Stream Deck
/// buttons where a separate body editor is inconvenient.
///
/// Supported per-slot commands:
///
/// | `cmd`         | Effect                                                  |
/// |---------------|---------------------------------------------------------|
/// | `heal_party`  | Heal HP/PP/status of all party Pokémon for this slot.   |
async fn api_slot_command(
    State(state): State<WebState>,
    Path((index, cmd)): Path<(usize, String)>,
) -> impl IntoResponse {
    let msg = match cmd.as_str() {
        "heal_party" => ClientMessage::HealParty,
        other => return (StatusCode::BAD_REQUEST, format!("Unknown slot command: {other}")),
    };
    let slots = state.live_slots.lock_or_recover();
    let Some(slot) = slots.get(index) else {
        return (StatusCode::NOT_FOUND, format!("No slot at index {index}"));
    };
    slot.command_queue.lock_or_recover().push_back(msg);
    (StatusCode::OK, format!("Command '{cmd}' sent to slot {index}"))
}

/// Runs arbitrary SQL against the database and returns results as JSON.
///
/// Restricted to loopback connections — returns 403 for any remote caller.
/// `POST /api/slot/:index/refresh_rom` — force-re-download the cached ROM for a
/// direct-mode slot from its RetroArch instance.
///
/// Deletes the cached `.gba` file, re-fetches the full 16 MiB ROM from RetroArch
/// over UDP (takes 5–15 s on a typical LAN), and replaces the in-memory ROM
/// buffer used by the sprite loader so new sprites are decoded from the fresh ROM.
///
/// Returns 400 if the slot is not in direct mode, 404 if the slot index is out of
/// range, or 503 if RetroArch is unreachable.
async fn api_refresh_rom(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => {
            return (StatusCode::NOT_FOUND, "slot index out of range".to_string())
                .into_response()
        }
    };
    let host_port = match &slot.direct_host {
        Some(hp) => hp.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "slot is not in direct mode — ROM refresh is only available for \
                 direct-mode connections"
                    .to_string(),
            )
                .into_response()
        }
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(55355)),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("malformed direct_host: {}", host_port),
            )
                .into_response()
        }
    };

    let rom_bytes_arc    = slot.rom_bytes.clone();
    let rom_identity_arc = slot.rom_identity.clone();
    let known_species    = slot.known_species.clone();
    let pending_textures = slot.pending_textures.clone();
    let sprite_cache     = slot.sprite_cache.clone();
    let game_encounters  = slot.game_encounters.clone();

    let result = tokio::task::spawn_blocking(move || {
        crate::rom_fetch::force_fetch_rom(&host, port)
            .and_then(|path| std::fs::read(&path).map_err(|e| e.to_string()))
    })
    .await;

    match result {
        Ok(Ok(bytes)) => {
            let new_id  = crate::direct::rom_identity_from_bytes(&bytes);
            let old_id  = rom_identity_arc.lock_or_recover().clone();
            let changed = old_id != new_id && !old_id.is_empty();

            if changed {
                tracing::info!(
                    "ROM force-refresh: slot {} — ROM changed from \"{}\" to \"{}\"",
                    index, old_id, new_id
                );
            } else {
                tracing::info!(
                    "ROM force-refresh: slot {} — same ROM identity \"{}\" (re-fetched bytes)",
                    index, new_id
                );
            }

            // Update ROM bytes and identity.
            *rom_identity_arc.lock_or_recover() = new_id.clone();
            *rom_bytes_arc.lock_or_recover()    = bytes;

            // Clear sprite pipeline so sprites are re-decoded from the new ROM.
            known_species.lock_or_recover().clear();
            pending_textures.lock_or_recover().clear();
            if let Some(cache_arc) = sprite_cache.lock_or_recover().as_ref() {
                cache_arc.lock_or_recover().clear();
            }

            // Reset the game loop's encounter-table cache so stale area data
            // from the old ROM is evicted immediately.
            if let Some(enc_arc) = game_encounters.lock_or_recover().as_ref() {
                *enc_arc.lock_or_recover() =
                    fire_red_pokemon_data::WildPokemonHeader::default();
            }

            let body = if changed {
                format!("ROM changed: {} → {}", old_id, new_id)
            } else {
                format!("ROM refreshed ({})", new_id)
            };
            (StatusCode::OK, body).into_response()
        }
        Ok(Err(e)) => {
            tracing::error!("ROM force-refresh failed for slot {}: {}", index, e);
            (StatusCode::SERVICE_UNAVAILABLE, e).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_db_query(
    State(state): State<WebState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    if !addr.ip().is_loopback() {
        return axum::Json(
            serde_json::json!({ "error": "Forbidden: endpoint only available on localhost" }),
        );
    }
    let conn = require_db!(state);
    let sql = match body["sql"].as_str() {
        Some(s) => s.to_string(),
        None => return axum::Json(serde_json::json!({ "error": "Missing 'sql' field" })),
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::run_sql(&conn, &sql)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

async fn serve_hp_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(HP_HTML, state.testing, theme))
}

async fn serve_badges_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(BADGES_HTML, state.testing, theme))
}

async fn serve_next_gym_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(NEXT_GYM_HTML, state.testing, theme))
}

async fn serve_encounter_table_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(ENCOUNTER_TABLE_HTML, state.testing, theme))
}

async fn serve_money_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(MONEY_HTML, state.testing, theme))
}

async fn serve_playtime_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(PLAYTIME_HTML, state.testing, theme))
}

async fn serve_goals_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(GOALS_HTML, state.testing, theme))
}

async fn serve_vs_leader_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(VS_LEADER_HTML, state.testing, theme))
}

/// `POST /api/goal` — create a new run goal.
///
/// Body: `{"run_id": <u32>, "text": "<string>"}`
async fn api_post_goal(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let run_id = match body["run_id"].as_u64() {
        Some(id) => id as u32,
        None => return axum::Json(serde_json::json!({ "error": "missing run_id" })),
    };
    let text = match body["text"].as_str() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return axum::Json(serde_json::json!({ "error": "missing or empty text" })),
    };
    let uid = user.id;
    let can = tokio::task::spawn_blocking(move || {
        fire_red_database::user_can_access_run(run_id, uid)
    })
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);
    if !can {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::create_goal(&conn, run_id, &text)
    })
    .await;
    match result {
        Ok(Some(id)) => axum::Json(serde_json::json!({ "id": id })),
        _ => axum::Json(serde_json::json!({ "error": "failed to create goal" })),
    }
}

/// `PATCH /api/goal/:id/complete` — mark a goal as completed.
async fn api_complete_goal(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(goal_id): Path<i32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let conn_clone = conn.clone();
    let gid = goal_id;
    let run_id = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_id_for_goal(&conn_clone, gid)
    })
    .await
    .unwrap_or(None);
    if let Some(rid) = run_id {
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);
        if !can {
            return axum::Json(serde_json::json!({ "error": "access denied" }));
        }
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::complete_goal(&conn, goal_id)
    })
    .await;
    match result {
        Ok(true) => axum::Json(serde_json::json!({ "ok": true })),
        _ => axum::Json(serde_json::json!({ "error": "goal not found or update failed" })),
    }
}

/// `DELETE /api/goal/:id` — delete a goal.
async fn api_delete_goal(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(goal_id): Path<i32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let uid = user.id;
    let conn_clone = conn.clone();
    let gid = goal_id;
    let run_id = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_id_for_goal(&conn_clone, gid)
    })
    .await
    .unwrap_or(None);
    if let Some(rid) = run_id {
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);
        if !can {
            return axum::Json(serde_json::json!({ "error": "access denied" }));
        }
    }
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::delete_goal(&conn, goal_id)
    })
    .await;
    match result {
        Ok(true) => axum::Json(serde_json::json!({ "ok": true })),
        _ => axum::Json(serde_json::json!({ "error": "goal not found" })),
    }
}

// ---------------------------------------------------------------------------
// Batch injection (POST /api/batch)
// ---------------------------------------------------------------------------

/// `POST /api/batch` — apply an ordered list of injection commands in one request.
///
/// Body: a JSON array of `{ "slot": <usize>, "message": <ClientMessage> }` objects.
/// All commands are validated first, then enqueued atomically (one lock per slot).
/// Returns `{ "queued": <count> }` on success or `{ "error": "..." }` on the first
/// validation failure.
async fn api_batch_inject(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    if !state.allow_injections {
        return axum::Json(serde_json::json!({ "error": "injection commands are disabled" }));
    }
    let items = match body.as_array() {
        Some(a) => a,
        None => return axum::Json(serde_json::json!({ "error": "body must be a JSON array" })),
    };

    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();

    let slots = state.live_slots.lock_or_recover().clone();

    // Validate and decode every item before touching any queue.
    struct Decoded {
        slot_idx: usize,
        msg: ClientMessage,
    }
    let mut decoded: Vec<Decoded> = Vec::with_capacity(items.len());
    for (pos, item) in items.iter().enumerate() {
        let slot_idx = match item["slot"].as_u64().and_then(|v| usize::try_from(v).ok()) {
            Some(v) => v,
            None => return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: 'slot' must be a non-negative integer")
            })),
        };
        if slot_idx >= slots.len() {
            return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: slot {slot_idx} out of range")
            }));
        }
        if let Some(rid) = slots[slot_idx].db.as_ref().and_then(|db| db.get_run_id())
            && !accessible.contains(&rid)
        {
            return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: access denied for slot {slot_idx}")
            }));
        }
        let msg: ClientMessage = match serde_json::from_value(item["message"].clone()) {
            Ok(m) => m,
            Err(e) => return axum::Json(serde_json::json!({
                "error": format!("item[{pos}]: invalid message: {e}")
            })),
        };
        decoded.push(Decoded { slot_idx, msg });
    }

    // Enqueue all commands. Group by slot to minimise lock acquisitions.
    let count = decoded.len();
    for d in decoded {
        slots[d.slot_idx]
            .command_queue
            .lock_or_recover()
            .push_back(d.msg);
    }
    axum::Json(serde_json::json!({ "queued": count }))
}

// ---------------------------------------------------------------------------
// Preset party builds
// ---------------------------------------------------------------------------

/// `POST /api/preset` — save a named party preset.
///
/// Body: `{ "name": "<str>", "commands": [<ClientMessage>, ...] }`.
async fn api_save_preset(
    State(state): State<WebState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let name = match body["name"].as_str() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return axum::Json(serde_json::json!({ "error": "'name' must be a non-empty string" })),
    };
    let commands = match body.get("commands") {
        Some(v) => v.clone(),
        None => return axum::Json(serde_json::json!({ "error": "missing 'commands' array" })),
    };
    let config_json = commands.to_string();
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::save_preset(&conn, &name, &config_json)
    })
    .await;
    match result {
        Ok(Ok(())) => axum::Json(serde_json::json!({ "ok": true })),
        Ok(Err(e)) => axum::Json(serde_json::json!({ "error": e })),
        Err(_) => axum::Json(serde_json::json!({ "error": "Task panicked" })),
    }
}

/// `GET /api/presets` — list all saved presets.
async fn api_list_presets(
    State(state): State<WebState>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || fire_red_database::list_presets(&conn)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `DELETE /api/preset/:name` — delete a preset.
async fn api_delete_preset(
    State(state): State<WebState>,
    Path(name): Path<String>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::delete_preset(&conn, &name)).await;
    match result {
        Ok(true) => axum::Json(serde_json::json!({ "ok": true })),
        _ => axum::Json(serde_json::json!({ "error": "preset not found" })),
    }
}

/// `POST /api/preset/:name/apply` — enqueue all commands from a preset for a slot.
///
/// Body: `{ "slot": <usize> }`.
async fn api_apply_preset(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    if !state.allow_injections {
        return axum::Json(serde_json::json!({ "error": "injection commands are disabled" }));
    }
    let conn = require_db!(state.clone());
    let slot_idx = match body["slot"].as_u64().and_then(|v| usize::try_from(v).ok()) {
        Some(v) => v,
        None => return axum::Json(serde_json::json!({ "error": "'slot' must be a non-negative integer" })),
    };
    let slots = state.live_slots.lock_or_recover().clone();
    if slot_idx >= slots.len() {
        return axum::Json(serde_json::json!({ "error": "slot index out of range" }));
    }
    if let Some(rid) = slots[slot_idx].db.as_ref().and_then(|db| db.get_run_id()) {
        let uid = user.id;
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);
        if !can {
            return axum::Json(serde_json::json!({ "error": "access denied" }));
        }
    }
    let commands_val = match tokio::task::spawn_blocking(move || {
        fire_red_database::get_preset(&conn, &name)
    })
    .await
    {
        Ok(Some(v)) => v,
        Ok(None) => return axum::Json(serde_json::json!({ "error": "preset not found" })),
        Err(_) => return axum::Json(serde_json::json!({ "error": "Task panicked" })),
    };
    let arr = match commands_val.as_array() {
        Some(a) => a.clone(),
        None => return axum::Json(serde_json::json!({ "error": "preset 'commands' is not an array" })),
    };
    let mut count = 0usize;
    {
        let mut queue = slots[slot_idx].command_queue.lock_or_recover();
        for val in &arr {
            if let Ok(msg) = serde_json::from_value::<ClientMessage>(val.clone()) {
                queue.push_back(msg);
                count += 1;
            }
        }
    }
    axum::Json(serde_json::json!({ "queued": count }))
}

// ---------------------------------------------------------------------------
// Challenge rules (GET/PATCH /api/run/:id/rules)
// ---------------------------------------------------------------------------

/// `GET /api/run/:id/rules` — fetch nuzlocke variant flags for a run.
async fn api_get_run_rules(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_run_rules(&conn, run_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `PATCH /api/run/:id/rules` — update one or more nuzlocke variant flags.
///
/// Body: any subset of `{ "duplicate_clause": bool, "species_clause": bool,
/// "gift_clause": bool, "shiny_clause": bool }`. Unspecified fields are unchanged.
async fn api_patch_run_rules(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::set_run_rules(&conn, run_id, &body)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

// ---------------------------------------------------------------------------
// Per-section CSV exports
// ---------------------------------------------------------------------------

fn csv_response(
    result: Result<Result<String, String>, tokio::task::JoinError>,
    run_id: u32,
    suffix: &str,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match result {
        Ok(Ok(csv)) => (
            [
                ("content-type", "text/csv".to_string()),
                (
                    "content-disposition",
                    format!("attachment; filename=\"run_{run_id}_{suffix}.csv\""),
                ),
            ],
            csv,
        )
            .into_response(),
        Ok(Err(e)) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(_) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Task panicked").into_response(),
    }
}

/// `GET /api/run/:id/encounters.csv` — first encounter per area as CSV.
async fn api_run_encounters_csv(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response()
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::export_encounters_csv(&conn, run_id)
    })
    .await;
    csv_response(result, run_id, "encounters")
}

/// `GET /api/run/:id/deaths.csv` — death log as CSV.
async fn api_run_deaths_csv(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response()
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::export_deaths_csv(&conn, run_id)
    })
    .await;
    csv_response(result, run_id, "deaths")
}

/// `GET /api/run/:id/events.csv` — event log as CSV.
async fn api_run_events_csv(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let conn = match state.db_conn {
        Some(s) => s,
        None => {
            return axum::Json(serde_json::json!({ "error": "No database configured" }))
                .into_response()
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::export_events_csv(&conn, run_id)
    })
    .await;
    csv_response(result, run_id, "events")
}

// ---------------------------------------------------------------------------
// Discord slash-command interactions endpoint
// ---------------------------------------------------------------------------

/// Body of a Discord Interactions POST (we only need a handful of fields).
#[derive(serde::Deserialize)]
struct DiscordInteraction {
    #[serde(rename = "type")]
    kind: u8,
    data: Option<DiscordInteractionData>,
}

#[derive(serde::Deserialize)]
struct DiscordInteractionData {
    name: Option<String>,
}

/// `POST /interactions` — Discord Interactions endpoint.
///
/// Verifies the Ed25519 signature, responds to ping (type 1), and handles
/// `/party`, `/status`, `/deaths` application commands (type 2) ephemerally.
async fn discord_interactions(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // ── Signature verification ──────────────────────────────────────────────
    let public_key_hex = state
        .discord_slash
        .as_ref()
        .map(|c| c.public_key.as_str())
        .unwrap_or("");

    let sig_header = headers
        .get("x-signature-ed25519")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ts_header = headers
        .get("x-signature-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_discord_signature(public_key_hex, sig_header, ts_header, &body) {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "invalid signature" }))).into_response();
    }

    // ── Parse body ─────────────────────────────────────────────────────────
    let interaction: DiscordInteraction = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": "bad body" }))).into_response(),
    };

    // ── Handle ping (type 1) ───────────────────────────────────────────────
    if interaction.kind == 1 {
        return axum::Json(serde_json::json!({ "type": 1 })).into_response();
    }

    // ── Handle application command (type 2) ────────────────────────────────
    if interaction.kind == 2 {
        let cmd_name = interaction
            .data
            .as_ref()
            .and_then(|d| d.name.as_deref())
            .unwrap_or("");

        let content = {
            let slots = state.live_slots.lock_or_recover();
            build_slash_response(cmd_name, &slots)
        };

        // Ephemeral message response (type 4, flags 64)
        return axum::Json(serde_json::json!({
            "type": 4,
            "data": {
                "content": content,
                "flags": 64
            }
        })).into_response();
    }

    (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": "unknown interaction type" }))).into_response()
}

fn verify_discord_signature(public_key_hex: &str, signature_hex: &str, timestamp: &str, body: &[u8]) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};

    let pub_bytes = match hex_decode(public_key_hex) {
        Some(b) if b.len() == 32 => b,
        _ => return false,
    };
    let sig_bytes = match hex_decode(signature_hex) {
        Some(b) if b.len() == 64 => b,
        _ => return false,
    };

    let key = match VerifyingKey::from_bytes(pub_bytes[..32].try_into().unwrap_or(&[0u8; 32])) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig = Signature::from_bytes(sig_bytes[..64].try_into().unwrap_or(&[0u8; 64]));

    let mut message = timestamp.as_bytes().to_vec();
    message.extend_from_slice(body);

    use ed25519_dalek::Verifier;
    key.verify(&message, &sig).is_ok()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) { return None; }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn build_slash_response(cmd: &str, slots: &[Arc<crate::client::MonitorSlot>]) -> String {
    match cmd {
        "party" => {
            if slots.is_empty() {
                return "No trackers connected.".to_string();
            }
            let mut lines = Vec::new();
            for (i, slot) in slots.iter().enumerate() {
                let gs = slot.state.lock_or_recover();
                if let Some(gs) = gs.as_ref() {
                    let members: Vec<String> = gs.party.iter()
                        .filter(|m| m.box_mon.secure.growth.species > 0)
                        .map(|m| format!("{} Lv.{}", m.box_mon.secure.growth.species_string, m.level))
                        .collect();
                    if !members.is_empty() {
                        lines.push(format!("**Slot {}** ({}): {}", i + 1, gs.player_name, members.join(", ")));
                    }
                }
            }
            if lines.is_empty() { "No party data available.".to_string() } else { lines.join("\n") }
        }
        "status" => {
            if slots.is_empty() {
                return "No trackers connected.".to_string();
            }
            let slot = &slots[0];
            let gs = slot.state.lock_or_recover();
            if let Some(gs) = gs.as_ref() {
                let badges = gs.badge_state.as_ref()
                    .map(|b| b.badges.iter().filter(|&&v| v).count())
                    .unwrap_or(0);
                let zone = if gs.zone_name.is_empty() { "unknown".to_string() } else { gs.zone_name.clone() };
                format!("**{}** — {} badge(s) — currently at {}", gs.player_name, badges, zone)
            } else {
                "Tracker connected but no game data yet.".to_string()
            }
        }
        "deaths" => {
            let total: usize = slots.iter().map(|slot| {
                slot.db.as_ref()
                    .and_then(|db| db.active_run_id())
                    .map(|_| {
                        let player = slot.state.lock_or_recover()
                            .as_ref()
                            .map(|gs| gs.player_name.clone())
                            .unwrap_or_default();
                        if let Some(db) = slot.db.as_ref() {
                            db.list_dead_with_records(&player).len()
                        } else { 0 }
                    })
                    .unwrap_or(0)
            }).sum();
            format!("Total deaths across all slots: **{}**", total)
        }
        _ => format!("Unknown command: {cmd}"),
    }
}

/// Register `/party`, `/status`, `/deaths` slash commands with Discord.
/// Called once at startup when `[discord_slash]` is configured.
pub fn register_slash_commands(cfg: &crate::config::DiscordSlashConfig) {
    let commands = serde_json::json!([
        { "name": "party", "description": "Show the current party for all connected slots", "type": 1 },
        { "name": "status", "description": "Show run status (badges, location) for slot 0", "type": 1 },
        { "name": "deaths", "description": "Show total death count across all slots", "type": 1 }
    ]);

    let url = if let Some(guild_id) = cfg.guild_id {
        format!(
            "https://discord.com/api/v10/applications/{}/guilds/{}/commands",
            cfg.app_id, guild_id
        )
    } else {
        format!(
            "https://discord.com/api/v10/applications/{}/commands",
            cfg.app_id
        )
    };

    let token = cfg.token.clone();
    let commands_str = commands.to_string();
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::new();
        match client
            .put(&url)
            .header("Authorization", format!("Bot {}", token))
            .header("content-type", "application/json")
            .body(commands_str)
            .send()
        {
            Ok(r) if r.status().is_success() => {
                tracing::info!("Discord slash commands registered successfully.");
            }
            Ok(r) => {
                tracing::warn!("Discord slash command registration failed: HTTP {}", r.status());
            }
            Err(e) => {
                tracing::warn!("Discord slash command registration error: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Router construction (extracted for testability)
// ---------------------------------------------------------------------------

fn build_router(web_state: WebState) -> Router {
    Router::new()
        .route("/", get(serve_login_page))
        .route("/overlay", get(serve_html))
        .route("/ws", get(ws_handler))
        .route("/db", get(serve_db_viewer))
        .route("/db.json", get(serve_db_json))
        .route("/db/clear", post(clear_db))
        .route("/db/query", get(serve_db_query))
        .route("/cmd", get(serve_cmd))
        .route("/api/state", get(api_state))
        .route("/api/slot/:index", get(api_slot))
        .route("/api/slot/:index/odds", get(api_slot_odds))
        .route("/api/slot/:index/give_item", post(api_give_item))
        .route("/api/slot/:index/take_item", post(api_take_item))
        .route("/api/slot/:index/change_species", post(api_change_species))
        .route("/api/slot/:index/change_ability", post(api_change_ability))
        .route("/api/slot/:index/change_gender", post(api_change_gender))
        .route("/api/slot/:index/make_shiny", post(api_make_shiny))
        .route(
            "/api/slot/:index/change_nickname",
            post(api_change_nickname),
        )
        .route(
            "/api/slot/:index/change_held_item",
            post(api_change_held_item),
        )
        .route("/api/slot/:index/cure_status", post(api_cure_status))
        .route("/api/slot/:index/change_nature", post(api_change_nature))
        .route("/api/slot/:index/restore_pp", post(api_restore_pp))
        .route("/api/slot/:index/set_friendship", post(api_set_friendship))
        .route("/api/slot/:index/change_move", post(api_change_move))
        .route("/api/slot/:index/set_ivs", post(api_set_ivs))
        .route("/api/slot/:index/increase_ivs", post(api_increase_ivs))
        .route("/api/slot/:index/set_evs", post(api_set_evs))
        .route("/api/slot/:index/increase_evs", post(api_increase_evs))
        .route("/api/slot/:index/restore_hp", post(api_restore_hp))
        .route("/api/slot/:index/heal_party", post(api_heal_party))
        .route("/api/slot/:index/set_exp", post(api_set_exp))
        .route("/api/slot/:index/set_level", post(api_set_level))
        .route("/api/slot/:index/learn_move", post(api_learn_move))
        .route("/api/slot/:index/forget_move", post(api_forget_move))
        .route("/api/slot/:index/set_pokerus", post(api_set_pokerus))
        .route("/api/slot/:index/set_pp_ups", post(api_set_pp_ups))
        .route("/api/slot/:index/revive_pokemon", post(api_revive_pokemon))
        .route("/api/slot/:index/undo", post(api_undo))
        .route("/api/slot/:index/refresh_rom", post(api_refresh_rom))
        .route("/api/bot/:index", get(api_bot_summary))
        .route("/api/command/:cmd", post(api_command))
        .route("/api/db/query", post(api_db_query))
        .route("/api/runs", get(api_runs))
        .route("/api/run/import", post(api_run_import))
        .route("/api/run/:id/stats", get(api_run_stats))
        .route("/api/run/:id/route_stats", get(api_run_route_stats))
        .route("/api/run/:id/route_odds", get(api_run_route_odds))
        .route("/api/run/:id/webhook_log", get(api_run_webhook_log))
        .route(
            "/api/run/:id/soul_link/overrides",
            get(api_run_soul_link_overrides),
        )
        .route(
            "/api/run/:id/soul_link/override",
            post(api_set_soul_link_override),
        )
        .route(
            "/api/run/:id/soul_link/override/:personality",
            delete(api_clear_soul_link_override),
        )
        .route("/api/run/:id/shiny", get(api_shiny_stats))
        .route("/api/run/:id/export", get(api_run_export))
        .route("/api/run/:id/events", get(api_run_events))
        .route("/api/timeline", get(api_active_timeline))
        .route("/history", get(serve_history))
        .route("/shiny", get(serve_shiny))
        .route("/memorial", get(serve_memorial))
        .route("/soullink", get(serve_soullink))
        .route("/soullink/manage", get(serve_soullink_manage))
        .route("/alerts", get(serve_alerts))
        .route("/:index/alerts", get(serve_alerts))
        .route("/:index/routes", get(serve_routes))
        .route("/:index/party", get(serve_party))
        .route("/:index/encounters", get(serve_focused))
        .route("/:index/dead", get(serve_focused))
        .route("/:index/caught", get(serve_focused))
        .route("/:index/box", get(serve_focused))
        .route("/:index/types", get(serve_types_page))
        .route("/:index/items", get(serve_items))
        .route("/:index/moves", get(serve_moves_page))
        .route("/api/slot/:index/bag", get(api_bag))
        .route("/run/:id/stats", get(serve_run_stats))
        .route("/run/:id/memorial", get(serve_memorial))
        .route("/run/:id/timeline", get(serve_timeline))
        .route("/party/mobile", get(serve_mobile_party))
        .route("/timeline", get(serve_timeline))
        .route("/species", get(serve_species))
        .route("/api/species/stats", get(api_species_stats))
        .route("/trainers", get(serve_trainers))
        .route("/run/:id/trainers", get(serve_trainers))
        .route("/api/run/:id/trainers", get(api_run_trainers))
        .route("/api/runs/compare", get(api_runs_compare))
        .route("/api/run/:id/luck", get(api_run_luck))
        .route("/api/run/:id/closest_calls", get(api_run_closest_calls))
        .route("/api/catch_rate", get(api_catch_rate))
        .route(
            "/api/run/:id/pokemon/:personality/hp_history",
            get(api_run_pokemon_hp_history),
        )
        .route("/api/run/:id/enemy_hp_log", get(api_run_enemy_hp_log))
        .route("/api/run/:id/battle_damage", get(api_run_battle_damage))
        .route("/api/run/:id/summary", get(api_run_summary))
        .route(
            "/api/run/:id/event/:event_id/note",
            patch(api_set_event_note).delete(api_clear_event_note),
        )
        .route("/api/run/:id/pokepaste", get(api_run_pokepaste))
        .route("/api/run/:id/splits", get(api_run_splits))
        .route("/api/run/:id/catch_log", get(api_run_catch_log))
        .route("/api/run/:id/difficulty", get(api_run_difficulty))
        .route("/api/run/:id/area_times", get(api_run_area_times))
        .route("/api/run/:id/death_map", get(api_run_death_map))
        .route("/api/run/:id/level_curve", get(api_run_level_curve))
        .route("/api/run/:id/move_usage", get(api_run_move_usage))
        .route("/api/run/:id/friendship", get(api_run_friendship))
        .route("/api/slot/:index/ev_progress", get(api_slot_ev_progress))
        .route("/:index/deaths", get(serve_deaths_overlay))
        .route("/:index/encounter_count", get(serve_encounter_count))
        .route("/:index/hp", get(serve_hp_overlay))
        .route("/:index/badges", get(serve_badges_overlay))
        .route("/:index/nextgym", get(serve_next_gym_overlay))
        .route("/:index/encounter_table", get(serve_encounter_table_overlay))
        .route("/:index/money", get(serve_money_overlay))
        .route("/:index/playtime", get(serve_playtime_overlay))
        .route("/:index/goals", get(serve_goals_overlay))
        .route("/:index/vs_leader", get(serve_vs_leader_overlay))
        .route("/api/goal", post(api_post_goal))
        .route("/api/goal/:id/complete", patch(api_complete_goal))
        .route("/api/goal/:id", delete(api_delete_goal))
        .route("/api/slot/:index/command/:cmd", post(api_slot_command))
        .route("/about", get(serve_about))
        .route("/compare", get(serve_compare))
        .route("/join", get(serve_join))
        .route("/register", get(serve_register))
        .route("/api/direct/connect", post(api_direct_connect).delete(api_direct_disconnect))
        .route("/api/direct/hosts", get(api_direct_hosts))
        .route("/api/run", post(api_create_run))
        .route("/api/run/:id/resume", post(api_resume_run))
        // Batch injection
        .route("/api/batch", post(api_batch_inject))
        // Presets
        .route("/api/preset", post(api_save_preset))
        .route("/api/presets", get(api_list_presets))
        .route("/api/preset/:name", delete(api_delete_preset))
        .route("/api/preset/:name/apply", post(api_apply_preset))
        // Challenge rules
        .route("/api/run/:id/rules", get(api_get_run_rules).patch(api_patch_run_rules))
        // Per-section CSV exports
        .route("/api/run/:id/encounters.csv", get(api_run_encounters_csv))
        .route("/api/run/:id/deaths.csv", get(api_run_deaths_csv))
        .route("/api/run/:id/events.csv", get(api_run_events_csv))
        // Discord slash-command interactions endpoint
        .route("/interactions", post(discord_interactions))
        // Analytics: type usage heatmap, ghost run comparison, shiny pressure, status log
        .route("/api/run/:id/type_matchups", get(api_run_type_matchups))
        .route("/api/run/:id/vs/:ghost_id", get(api_run_ghost_compare))
        .route("/api/slot/:index/shiny_pressure", get(api_slot_shiny_pressure))
        .route("/api/run/:id/status_log", get(api_run_status_log))
        .route("/api/run/:id/dex", get(api_run_dex))
        // Share URL
        .route("/api/run/:id/share", post(api_create_share))
        .route("/share/:token/state", get(api_share_state))
        // Config hot-reload
        .route("/api/config/reload", post(api_config_reload))
        // Donation/alert trigger bridge
        .route("/api/webhook/donation", post(api_donation_webhook))
        // Savefile snapshot import
        .route("/api/savefile", post(api_import_savefile))
        // User accounts
        .route("/api/users", post(api_register_user).get(api_list_users))
        .route("/api/login", post(api_login))
        .route("/api/logout", post(api_logout))
        .route("/api/me", get(api_me))
        .route("/api/me/dashboard", get(api_me_dashboard))
        .route("/api/me/active_run", get(api_me_active_run).put(api_me_set_active_run))
        .route("/api/user/:id/runs", get(api_user_runs))
        // Run invites and access requests
        .route("/api/run/:id/invite", post(api_run_invite))
        .route("/api/run/:id/invites", get(api_run_invites))
        .route("/api/run/:id/invite/accept", post(api_run_invite_accept))
        .route("/api/run/:id/invite/decline", post(api_run_invite_decline))
        .route("/api/run/:id/invite/request", post(api_run_invite_request))
        .route("/api/run/:id/invite/requests", get(api_run_invite_requests))
        .route("/api/run/:id/invite/request/:uid/approve", post(api_run_invite_request_approve))
        .route("/api/run/:id/invite/request/:uid/deny", post(api_run_invite_request_deny))
        .route("/api/me/run_statuses", get(api_me_run_statuses))
        .route("/api/me/run_requests", get(api_me_run_requests))
        // Dashboard page
        .route("/dashboard", get(serve_dashboard))
        // Overlays
        .route("/:index/dex", get(serve_dex_overlay))
        .route("/:index/typechart", get(serve_typechart_overlay))
        // ── Middleware stack (last added = outermost = runs first) ──────────
        // 3. Slot access: check ownership before any request to /api/slot/:idx/…
        .layer(axum::middleware::from_fn_with_state(
            web_state.clone(),
            slot_access_middleware,
        ))
        // 2. Run access: check user_can_access_run for /api/run/:id/… routes
        .layer(axum::middleware::from_fn(run_access_middleware))
        // 1. Auth wall: require a valid session for all non-public routes
        .layer(axum::middleware::from_fn(auth_middleware))
        .with_state(web_state)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Optional configuration passed to [`run`] — bundles flags that are not
/// needed by every call site.
pub struct WebRunConfig {
    pub db_conn: Option<String>,
    pub testing: bool,
    pub allow_injections: bool,
    pub connector: Option<Arc<crate::direct::DirectConnector>>,
    pub backup_dir: Option<String>,
    pub livesplit_split_on_badges: bool,
    pub discord_slash: Option<crate::config::DiscordSlashConfig>,
    /// Optional path to the TOML config file for config hot-reload support.
    pub config_path: Option<String>,
}

pub fn run(live_slots: SharedSlots, port: u16, cfg: WebRunConfig) {
    let WebRunConfig {
        db_conn,
        testing,
        allow_injections,
        connector,
        backup_dir,
        livesplit_split_on_badges,
        discord_slash,
        config_path,
    } = cfg;
    let sprites: PngSpriteCache = Arc::new(Mutex::new(HashMap::new()));

    // Wire the shared sprite cache into any already-connected slots and keep
    // it available for slots that connect later (BroadcastLoop sets it on drain).
    {
        let slots = live_slots.lock_or_recover();
        for slot in slots.iter() {
            *slot.sprite_cache.lock_or_recover() = Some(sprites.clone());
        }
    }

    let (tx, _rx) = watch::channel::<String>(String::new());
    let tx_bg = tx.clone();
    let sprites_loop = sprites.clone();
    let loop_slots = live_slots.clone();
    let loop_db = db_conn.clone();
    let loop_backup_dir = backup_dir.clone();

    std::thread::spawn(move || {
        let mut bloop = BroadcastLoop::new(
            loop_slots,
            sprites_loop,
            loop_db,
            loop_backup_dir,
            livesplit_split_on_badges,
        );
        loop {
            if let Some(json) = bloop.tick() {
                let _ = tx_bg.send(json);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    let web_state = WebState {
        tx,
        live_slots,
        db_conn,
        testing,
        allow_injections,
        connector,
        discord_slash,
        config_path: config_path.map(Arc::new),
        user_active_run: Arc::new(Mutex::new(HashMap::new())),
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let app = build_router(web_state);

        let addr = format!("0.0.0.0:{}", port);
        tracing::info!("WebSocket overlay listening on http://{}", addr);
        tracing::info!("Add in OBS as Browser Source: http://localhost:{}", port);

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("failed to bind WebSocket port");
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            tracing::error!("WebSocket server error: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Run management endpoints
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
struct CreateRunBody {
    player_name: Option<String>,
}

/// `POST /api/run` — create a new run and return its ID.
///
/// Requires authentication. The run is linked to the caller's account and
/// their username is used as the player name (overriding any `player_name`
/// in the body).
async fn api_create_run(
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<CreateRunBody>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "authentication required" })));
    };
    let fallback_name = body.player_name.unwrap_or_else(|| "Unknown".into());

    let result = tokio::task::spawn_blocking(move || -> Result<u32, (StatusCode, String)> {
        let user = fire_red_database::validate_session(&token)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;

        let player_name = if user.username.is_empty() { fallback_name } else { user.username };
        let run_id = fire_red_database::create_run_for_slot(&player_name)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let _ = fire_red_database::link_run_to_user(run_id, user.id);
        Ok(run_id)
    }).await;

    match result {
        Ok(Ok(run_id)) => (StatusCode::CREATED, axum::Json(serde_json::json!({ "run_id": run_id }))),
        Ok(Err((status, e))) => (status, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/resume` — set an existing run as the global active run.
///
/// Requires authentication.  The caller must own the run or have an accepted
/// invite.  In direct mode each slot manages its own run context via
/// `run_id` in `POST /api/direct/connect` instead.
async fn api_resume_run(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "authentication required" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<u32, (StatusCode, String)> {
        let user = fire_red_database::validate_session(&token)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;

        match fire_red_database::user_can_access_run(run_id, user.id) {
            Ok(true) => {}
            Ok(false) => return Err((StatusCode::FORBIDDEN, "you do not have access to this run".into())),
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
        }
        match fire_red_database::resume_run(run_id) {
            Ok(true) => Ok(user.id),
            Ok(false) => Err((StatusCode::NOT_FOUND, format!("run #{run_id} not found"))),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
        }
    }).await;

    match result {
        Ok(Ok(user_id)) => {
            state.user_active_run.lock().unwrap().insert(user_id, run_id);
            (StatusCode::OK, axum::Json(serde_json::json!({ "run_id": run_id })))
        }
        Ok(Err((status, e))) => (status, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

// ---------------------------------------------------------------------------
// Create-account page
// ---------------------------------------------------------------------------

const REGISTER_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Create Account – Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:1rem}
.card{background:#16213e;border:1px solid #0f3460;border-radius:10px;padding:2rem;width:100%;max-width:400px}
h1{font-size:1.3rem;color:#e94560;margin-bottom:.3rem}
.sub{color:#888;font-size:.85rem;margin-bottom:1.5rem}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input{width:100%;padding:.55rem .75rem;background:#0f3460;border:1px solid #444;border-radius:4px;color:#eee;font-size:.95rem;margin-bottom:1rem}
input:focus{outline:none;border-color:#e94560}
.btn{display:block;width:100%;padding:.6rem;border:none;border-radius:4px;font-size:1rem;cursor:pointer}
.btn-primary{background:#e94560;color:#fff}
.btn-primary:hover{background:#c73652}
.btn-primary:disabled{background:#555;cursor:default}
.msg{margin-top:.9rem;padding:.55rem;border-radius:4px;text-align:center;font-size:.875rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.footer{margin-top:1.2rem;text-align:center;font-size:.82rem;color:#666}
.footer a{color:#5090e0;text-decoration:none}
.footer a:hover{text-decoration:underline}
.req{font-size:.75rem;color:#666;margin-top:-.6rem;margin-bottom:.9rem}
</style>
</head>
<body>
<div class="card">
  <h1>Create Account</h1>
  <p class="sub">Fire Red Tracker</p>
  <form id="reg-form" onsubmit="doRegister(event)">
    <label for="uname">Username</label>
    <input id="uname" type="text" placeholder="pick a username" autocomplete="username" required maxlength="64">
    <label for="upass">Password</label>
    <input id="upass" type="password" placeholder="at least 8 characters" autocomplete="new-password" required minlength="8">
    <p class="req">Minimum 8 characters.</p>
    <label for="upass2">Confirm Password</label>
    <input id="upass2" type="password" placeholder="repeat password" autocomplete="new-password" required>
    <button class="btn btn-primary" id="reg-btn" type="submit">Create Account</button>
  </form>
  <div id="msg" class="msg"></div>
  <div class="footer">Already have an account? <a href="/join">Log in on the join page</a></div>
</div>
<script>
async function doRegister(e){
  e.preventDefault();
  const msg=document.getElementById('msg');
  const btn=document.getElementById('reg-btn');
  msg.className='msg';
  const u=document.getElementById('uname').value.trim();
  const p=document.getElementById('upass').value;
  const p2=document.getElementById('upass2').value;
  if(p!==p2){msg.className='msg err';msg.textContent='Passwords do not match.';return;}
  if(p.length<8){msg.className='msg err';msg.textContent='Password must be at least 8 characters.';return;}
  btn.disabled=true;
  try{
    const r=await fetch('/api/users',{
      method:'POST',
      headers:{'Content-Type':'application/json'},
      body:JSON.stringify({username:u,password:p})
    });
    const d=await r.json();
    if(r.ok){
      msg.className='msg ok';
      msg.textContent='Account created! Redirecting to the join page…';
      setTimeout(()=>window.location.href='/join',1200);
    }else{
      msg.className='msg err';
      msg.textContent=d.error||('Error '+r.status);
      btn.disabled=false;
    }
  }catch(err){
    msg.className='msg err';
    msg.textContent='Network error: '+err.message;
    btn.disabled=false;
  }
}
</script>
</body>
</html>"#;

async fn serve_register() -> Html<&'static str> {
    Html(REGISTER_HTML)
}

// ---------------------------------------------------------------------------
// Run select / join page
// ---------------------------------------------------------------------------

const JOIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Run Select – Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;padding:2rem 1rem}
.container{max-width:860px;margin:0 auto}
h1{font-size:1.5rem;color:#e94560;margin-bottom:1.5rem;display:flex;align-items:center;justify-content:space-between}
h1 .user-pill{display:inline-flex;align-items:center;gap:.6rem;background:#1a3a1a;border:1px solid #2d5a2d;border-radius:20px;padding:.2rem .85rem;font-size:.82rem;color:#7dce7d}
.section{background:#16213e;border:1px solid #0f3460;border-radius:8px;padding:1.5rem;margin-bottom:1.5rem}
.section-title{font-size:.95rem;font-weight:700;color:#ccc;margin-bottom:1rem;padding-bottom:.5rem;border-bottom:1px solid #1e3a6e;display:flex;align-items:center;justify-content:space-between}
.btn{display:inline-block;padding:.45rem 1.1rem;border:none;border-radius:4px;font-size:.875rem;cursor:pointer;text-decoration:none;line-height:1.4}
.btn-primary{background:#e94560;color:#fff}
.btn-primary:hover{background:#c73652}
.btn-primary:disabled{background:#555;cursor:default}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499}
.btn-secondary:hover{background:#253d6a}
.btn-success{background:#1a5c2e;color:#7dce7d;border:1px solid #2d8a2d}
.btn-success:hover{background:#1e6a34}
.btn-danger{background:#5c1a1a;color:#ce7d7d;border:1px solid #8a2d2d}
.btn-danger:hover{background:#6a1e1e}
.btn-warn{background:#4a3a00;color:#e0c040;border:1px solid #7a6000}
.btn-connect{background:#0f3a4a;color:#7dd;border:1px solid #1a6a7a}
.btn-connect:hover{background:#145060}
.btn-sm{padding:.28rem .65rem;font-size:.78rem}
.btn-xs{padding:.18rem .5rem;font-size:.72rem}
.page-select{background:#1e3a6e;color:#aad;border:1px solid #2d5499;border-radius:4px;padding:.18rem .5rem;font-size:.72rem;cursor:pointer}
.page-select:focus{outline:none;border-color:#e94560}
table{width:100%;border-collapse:collapse;font-size:.85rem}
th{text-align:left;color:#888;font-weight:600;font-size:.72rem;text-transform:uppercase;letter-spacing:.4px;padding:.4rem .6rem;border-bottom:1px solid #1e3a6e}
td{padding:.42rem .6rem;border-bottom:1px solid rgba(255,255,255,0.04);vertical-align:middle}
tr:hover td{background:rgba(255,255,255,0.03)}
.run-id{color:#5090e0;font-weight:600}
.run-active{color:#60e060;font-size:.75rem;font-weight:700}
.deaths{color:#e06060}
.catches{color:#60d060}
.badge-owner{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a5c;color:#5090e0;border:1px solid #2d5499;vertical-align:middle;margin-left:.3rem}
.badge-invited{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a1a;color:#7dce7d;border:1px solid #2d8a2d;vertical-align:middle;margin-left:.3rem}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input[type=text],input[type=password],input[type=number],select{width:100%;padding:.5rem .7rem;background:#0f3460;border:1px solid #444;border-radius:4px;color:#eee;font-size:.9rem;margin-bottom:.8rem}
input:focus,select:focus{outline:none;border-color:#e94560}
select option{background:#0f3460}
.msg{margin-top:.6rem;padding:.5rem;border-radius:4px;text-align:center;font-size:.85rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.loading{color:#888;font-size:.85rem}
.td-actions{text-align:right;white-space:nowrap;gap:3px;display:flex;justify-content:flex-end;flex-wrap:wrap}
.form-row{display:flex;gap:.6rem;align-items:flex-end}
.form-row>*{flex:1;margin-bottom:0}
.form-row .btn{flex:0 0 auto;white-space:nowrap}
.radio-group{display:flex;flex-direction:column;gap:.5rem;margin-bottom:.8rem}
.radio-group label{display:flex;align-items:center;gap:.5rem;font-size:.875rem;color:#ccc;cursor:pointer;margin:0}
.radio-group input[type=radio]{width:auto;margin:0}
.req-row{display:flex;align-items:center;gap:.6rem;padding:.5rem 0;border-bottom:1px solid rgba(255,255,255,0.05);flex-wrap:wrap}
.req-row:last-child{border-bottom:none}
.req-info{flex:1;font-size:.85rem}
.req-user{color:#eee;font-weight:600}
.req-run{color:#5090e0;font-size:.8rem}
</style>
</head>
<body>
<div class="container">
<h1>
  <span>Run Select</span>
  <span id="user-pill" class="user-pill" style="display:none"></span>
</h1>

<!-- ── Your Runs ───────────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">
    <span>Your Runs</span>
    <button class="btn btn-success btn-sm" onclick="createRun()">+ New Run</button>
  </div>
  <div id="my-runs-status" class="loading">Loading…</div>
  <table id="my-runs-table" style="display:none">
    <thead><tr><th>#</th><th>Started</th><th>Status</th><th>Caught</th><th>Deaths</th><th></th></tr></thead>
    <tbody id="my-runs-body"></tbody>
  </table>
  <div id="msg-my-run" class="msg"></div>
</div>

<!-- ── Pending Invites ─────────────────────────────────────────────── -->
<div class="section" id="pending-invites-section" style="display:none">
  <div class="section-title">Pending Invites</div>
  <div id="pending-invites-list"></div>
</div>

<!-- ── All Runs ────────────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">All Runs</div>
  <div id="runs-status" class="loading">Loading runs…</div>
  <table id="runs-table" style="display:none">
    <thead><tr><th>#</th><th>Player</th><th>Started</th><th>Status</th><th>Caught</th><th>Deaths</th><th></th></tr></thead>
    <tbody id="runs-body"></tbody>
  </table>
  <div id="msg-run" class="msg"></div>
</div>

<!-- ── Pending Requests on Your Runs ──────────────────────────────── -->
<div class="section" id="requests-section" style="display:none">
  <div class="section-title">Access Requests on Your Runs</div>
  <div id="requests-list"></div>
</div>

<!-- ── Connect to RetroArch (direct mode only) ────────────────────── -->
<div class="section" id="direct-section" style="display:DIRECT_SECTION_DISPLAY">
  <div class="section-title">Connect to RetroArch</div>
  <p style="color:#aaa;font-size:.875rem;line-height:1.5;margin-bottom:1rem">Enter the IP of the machine running RetroArch. Network Commands must be enabled in RetroArch settings.</p>
  <form id="connect-form" onsubmit="doConnect(event)">
    <div class="form-row">
      <div><label for="c-host">RetroArch IP</label><input id="c-host" type="text" placeholder="192.168.1.x" required></div>
      <div style="flex:0 0 110px"><label for="c-port">Port</label><input id="c-port" type="number" value="DEFAULT_PORT" min="1" max="65535" required></div>
    </div>
    <label>Run</label>
    <div class="radio-group">
      <label><input type="radio" name="run-choice" value="new" checked onchange="updateRunPicker()"> Start a new run</label>
      <label><input type="radio" name="run-choice" value="existing" onchange="updateRunPicker()"> Resume an existing run</label>
    </div>
    <div id="run-picker-wrap" style="display:none">
      <label for="run-picker">Select run to resume</label>
      <select id="run-picker"><option value="">— loading runs —</option></select>
    </div>
    <button class="btn btn-primary" id="connect-btn" type="submit" style="width:100%">Connect</button>
  </form>
  <div id="msg-connect" class="msg"></div>
  <div id="active-hosts" style="display:none;margin-top:1rem;font-size:.8rem;color:#888">
    <strong>Currently connected hosts:</strong>
    <ul id="host-list" style="margin-top:.4rem;padding-left:1.2rem;color:#aaa"></ul>
  </div>
</div>

</div><!-- /container -->
<script>
const TOKEN_KEY='frt_session';
const CLIENT_IP='__CLIENT_IP__';
const DIRECT_PORT=DEFAULT_PORT;
const DIRECT_ACTIVE=DIRECT_MODE_ACTIVE;
let SESSION=localStorage.getItem(TOKEN_KEY)||null;
let ME=null;
let ALL_RUNS=[];
let MY_STATUSES={};// run_id (string) → 'owner'|'accepted'|'pending_invite'|'pending_request'

function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function fmtDate(iso){if(!iso)return'—';try{return new Date(iso).toLocaleDateString();}catch{return iso;}}
function authHdr(){return SESSION?{'Authorization':'Bearer '+SESSION}:{};}
function openRunPage(runId,sel){
  const p=sel.value;sel.value='';if(!p)return;
  const url=p==='stats'?'/run/'+runId+'/stats':'/'+p+'?run='+runId;
  const tok=localStorage.getItem(TOKEN_KEY);
  if(tok)fetch('/api/me/active_run',{method:'PUT',headers:{'Content-Type':'application/json','Authorization':'Bearer '+tok},body:JSON.stringify({run_id:runId})}).catch(()=>{});
  window.open(url,'_blank');
}

async function init(){
  if(!SESSION){window.location.href='/';return;}
  const r=await fetch('/api/me',{headers:authHdr()}).catch(()=>null);
  if(!r||!r.ok){SESSION=null;localStorage.removeItem(TOKEN_KEY);window.location.href='/';return;}
  ME=await r.json();
  document.getElementById('user-pill').textContent='● '+ME.username;
  document.getElementById('user-pill').style.display='';
  await Promise.all([loadStatuses(),loadAllRuns()]);
  loadMyRuns();
  loadPendingInvites();
  loadAccessRequests();
  loadHosts();
}

async function loadStatuses(){
  const r=await fetch('/api/me/run_statuses',{headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){const d=await r.json();MY_STATUSES=d.statuses||{};}
}

async function loadAllRuns(){
  const st=document.getElementById('runs-status');
  try{
    const r=await fetch('/api/runs');
    if(r.ok){const d=await r.json();ALL_RUNS=d.runs||[];renderAllRuns();}
    else{st.textContent='No database connected.';}
  }catch(e){st.textContent='Could not load runs.';}
  populateRunPicker();
}

function renderAllRuns(){
  const st=document.getElementById('runs-status');
  const tbl=document.getElementById('runs-table');
  const tbody=document.getElementById('runs-body');
  if(!ALL_RUNS.length){st.textContent='No runs yet.';st.style.display='';tbl.style.display='none';return;}
  st.style.display='none';tbl.style.display='';
  tbody.innerHTML='';
  for(const run of ALL_RUNS){
    const status=MY_STATUSES[String(run.id)];
    const hasAccess=(status==='owner'||status==='accepted');
    const active=run.ended_at==null;
    let actions='';
    if(hasAccess){
      actions+='<select class="page-select" onchange="openRunPage('+run.id+',this)"><option value="">Open page…</option><option value="overlay">Overlay</option><option value="history">History</option><option value="stats">Stats</option><option value="shiny">Shiny</option><option value="memorial">Memorial</option><option value="trainers">Trainers</option><option value="timeline">Timeline</option></select> ';
      actions+='<button class="btn btn-success btn-xs" onclick="resumeRun('+run.id+')">Resume</button>';
      if(DIRECT_ACTIVE&&active)actions+=' <button class="btn btn-connect btn-xs" onclick="quickConnect('+run.id+')" title="Connect your RetroArch and open overlay">Quick Connect</button>';
    }else if(status==='pending_request'){
      actions='<span style="color:#888;font-size:.78rem">Request pending…</span>';
    }else if(status==='pending_invite'){
      actions='<span style="color:#e0c040;font-size:.78rem">Invite pending</span>';
    }else{
      actions='<button class="btn btn-warn btn-xs" onclick="requestAccess('+run.id+',this)">Request Access</button>';
    }
    const ownerBadge=status==='owner'?'<span class="badge-owner">owner</span>'
                    :status==='accepted'?'<span class="badge-invited">invited</span>':'';
    const tr=document.createElement('tr');
    tr.innerHTML=
      '<td><span class="run-id">#'+run.id+'</span>'+ownerBadge+'</td>'
      +'<td>'+esc(run.player_name||'—')+'</td>'
      +'<td style="color:#888;font-size:.8rem">'+fmtDate(run.started_at)+'</td>'
      +'<td>'+(active?'<span class="run-active">● Active</span>':'<span style="color:#555;font-size:.8rem">ended</span>')+'</td>'
      +'<td><span class="catches">'+(run.catches??0)+'</span></td>'
      +'<td><span class="deaths">'+(run.deaths??0)+'</span></td>'
      +'<td class="td-actions">'+actions+'</td>';
    tbody.appendChild(tr);
  }
}

async function loadMyRuns(){
  if(!ME)return;
  const st=document.getElementById('my-runs-status');
  st.textContent='Loading…';st.style.display='';
  document.getElementById('my-runs-table').style.display='none';
  try{
    const r=await fetch('/api/user/'+ME.id+'/runs',{headers:authHdr()});
    if(!r.ok){st.textContent='Could not load your runs.';return;}
    const d=await r.json();
    const runs=d.runs||[];
    const tbody=document.getElementById('my-runs-body');
    const tbl=document.getElementById('my-runs-table');
    if(!runs.length){st.textContent='No runs yet.';st.style.display='';tbl.style.display='none';return;}
    st.style.display='none';tbl.style.display='';
    tbody.innerHTML='';
    for(const run of runs){
      const active=run.ended_at==null;
      const badge=run.is_owner?'<span class="badge-owner">owner</span>':'<span class="badge-invited">invited</span>';
      let actions=
        '<select class="page-select" onchange="openRunPage('+run.id+',this)"><option value="">Open page…</option><option value="overlay">Overlay</option><option value="history">History</option><option value="stats">Stats</option><option value="shiny">Shiny</option><option value="memorial">Memorial</option><option value="trainers">Trainers</option><option value="timeline">Timeline</option></select> '
        +'<button class="btn btn-success btn-xs" onclick="resumeRun('+run.id+')">Resume</button>'
        +(DIRECT_ACTIVE&&active?' <button class="btn btn-connect btn-xs" onclick="quickConnect('+run.id+')" title="Connect your RetroArch and open overlay">Quick Connect</button>':'');
      const tr=document.createElement('tr');
      tr.innerHTML=
        '<td><span class="run-id">#'+run.id+'</span>'+badge+'</td>'
        +'<td style="color:#888;font-size:.8rem">'+fmtDate(run.started_at)+'</td>'
        +'<td>'+(active?'<span class="run-active">● Active</span>':'<span style="color:#555;font-size:.8rem">ended</span>')+'</td>'
        +'<td><span class="catches">'+(run.catches??0)+'</span></td>'
        +'<td><span class="deaths">'+(run.deaths??0)+'</span></td>'
        +'<td class="td-actions">'+actions+'</td>';
      tbody.appendChild(tr);
    }
  }catch(e){document.getElementById('my-runs-status').textContent='Could not load your runs.';}
}

async function loadPendingInvites(){
  const sec=document.getElementById('pending-invites-section');
  const list=document.getElementById('pending-invites-list');
  const pending=Object.entries(MY_STATUSES)
    .filter(([,v])=>v==='pending_invite')
    .map(([id])=>parseInt(id,10));
  if(!pending.length){sec.style.display='none';return;}
  // Look up run details from ALL_RUNS
  list.innerHTML='';
  for(const runId of pending){
    const run=ALL_RUNS.find(r=>r.id===runId);
    if(!run)continue;
    const row=document.createElement('div');
    row.className='req-row';
    row.id='inv-row-'+runId;
    row.innerHTML=
      '<div class="req-info"><span class="req-user">Run #'+runId+'</span>'
      +' <span class="req-run">'+esc(run.player_name||'—')+'</span></div>'
      +'<button class="btn btn-success btn-sm" onclick="respondInvite('+runId+',true)">Accept</button>'
      +'<button class="btn btn-danger btn-sm" onclick="respondInvite('+runId+',false)">Decline</button>';
    list.appendChild(row);
  }
  if(list.children.length)sec.style.display='';
}

async function respondInvite(runId,accept){
  const ep=accept?'accept':'decline';
  const r=await fetch('/api/run/'+runId+'/invite/'+ep,{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    const row=document.getElementById('inv-row-'+runId);
    if(row)row.remove();
    const list=document.getElementById('pending-invites-list');
    if(!list.children.length)document.getElementById('pending-invites-section').style.display='none';
    MY_STATUSES[String(runId)]=accept?'accepted':undefined;
    if(accept){await loadStatuses();loadMyRuns();renderAllRuns();}
    else{delete MY_STATUSES[String(runId)];renderAllRuns();}
  }
}

async function createRun(){
  const msg=document.getElementById('msg-my-run');
  const r=await fetch('/api/run',{method:'POST',headers:{'Content-Type':'application/json',...authHdr()},body:JSON.stringify({})}).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    msg.className='msg ok';msg.textContent='Created run #'+d.run_id+'.';
    setTimeout(async()=>{await loadStatuses();await loadAllRuns();loadMyRuns();},600);
  }else{
    msg.className='msg err';msg.textContent=d.error||'Failed.';
  }
}

async function resumeRun(runId){
  const r=await fetch('/api/run/'+runId+'/resume',{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    const msg=document.getElementById('msg-my-run');
    msg.className='msg ok';msg.textContent='Run #'+runId+' set as active.';
  }else if(r){
    const d=await r.json().catch(()=>({}));
    const msg=document.getElementById('msg-my-run');
    msg.className='msg err';msg.textContent=d.error||'Could not resume run.';
  }
}

async function requestAccess(runId,btn){
  btn.disabled=true;
  const r=await fetch('/api/run/'+runId+'/invite/request',{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    MY_STATUSES[String(runId)]='pending_request';
    renderAllRuns();
  }else{
    btn.disabled=false;
    if(r){const d=await r.json().catch(()=>({}));alert(d.error||'Request failed.');}
  }
}

async function loadAccessRequests(){
  const r=await fetch('/api/me/run_requests',{headers:authHdr()}).catch(()=>null);
  if(!r||!r.ok)return;
  const d=await r.json();
  const reqs=d.requests||[];
  if(!reqs.length)return;
  const sec=document.getElementById('requests-section');
  const list=document.getElementById('requests-list');
  list.innerHTML='';
  for(const req of reqs){
    const row=document.createElement('div');
    row.className='req-row';
    row.id='req-row-'+req.invite_id;
    row.innerHTML=
      '<div class="req-info">'
        +'<span class="req-user">'+esc(req.username)+'</span>'
        +' <span class="req-run">wants access to Run #'+req.run_id+' ('+esc(req.player_name)+')</span>'
        +'<div style="color:#666;font-size:.75rem">'+fmtDate(req.created_at)+'</div>'
      +'</div>'
      +'<button class="btn btn-success btn-sm" onclick="respondRequest('+req.run_id+','+req.user_id+','+req.invite_id+',true)">Approve</button>'
      +'<button class="btn btn-danger btn-sm" onclick="respondRequest('+req.run_id+','+req.user_id+','+req.invite_id+',false)">Deny</button>';
    list.appendChild(row);
  }
  sec.style.display='';
}

async function respondRequest(runId,userId,inviteId,approve){
  const ep=approve?'approve':'deny';
  const r=await fetch('/api/run/'+runId+'/invite/request/'+userId+'/'+ep,{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    const row=document.getElementById('req-row-'+inviteId);
    if(row)row.remove();
    const list=document.getElementById('requests-list');
    if(!list.children.length)document.getElementById('requests-section').style.display='none';
  }
}

// ── Quick connect ────────────────────────────────────────────────────
async function quickConnect(runId){
  const msg=document.getElementById('msg-my-run');
  msg.className='msg';
  const r=await fetch('/api/direct/connect',{
    method:'POST',
    headers:{'Content-Type':'application/json',...authHdr()},
    body:JSON.stringify({host:CLIENT_IP,port:DIRECT_PORT,run_id:runId}),
  }).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    window.open('/overlay?run='+runId,'_blank');
  }else{
    msg.className='msg err';msg.textContent=d.error||'Connection failed.';
  }
}

// ── Direct mode ──────────────────────────────────────────────────────
function populateRunPicker(){
  const sel=document.getElementById('run-picker');
  sel.innerHTML='<option value="">— new run —</option>';
  // Only show runs the user can access
  for(const run of ALL_RUNS){
    const status=MY_STATUSES[String(run.id)];
    if(status!=='owner'&&status!=='accepted')continue;
    const opt=document.createElement('option');
    opt.value=run.id;
    opt.textContent='#'+run.id+' '+(run.player_name||'Unknown')+' ('+fmtDate(run.started_at)+')';
    sel.appendChild(opt);
  }
}

function updateRunPicker(){
  const choice=document.querySelector('input[name="run-choice"]:checked').value;
  document.getElementById('run-picker-wrap').style.display=(choice==='existing'?'':'none');
}

async function doConnect(e){
  e.preventDefault();
  const host=document.getElementById('c-host').value.trim();
  const port=parseInt(document.getElementById('c-port').value,10);
  const choice=document.querySelector('input[name="run-choice"]:checked').value;
  const runIdVal=document.getElementById('run-picker').value;
  const run_id=choice==='existing'&&runIdVal?parseInt(runIdVal,10):null;
  const msg=document.getElementById('msg-connect');
  const btn=document.getElementById('connect-btn');
  msg.className='msg';btn.disabled=true;
  try{
    const body={host,port};
    if(run_id!=null)body.run_id=run_id;
    const r=await fetch('/api/direct/connect',{method:'POST',headers:{'Content-Type':'application/json',...authHdr()},body:JSON.stringify(body)});
    const d=await r.json();
    msg.className='msg '+(r.ok?'ok':'err');
    msg.textContent=r.ok?(d.message||'Connection request sent.'):(d.error||'Connection failed.');
  }catch(err){
    msg.className='msg err';msg.textContent='Request failed: '+err.message;
  }
  btn.disabled=false;
}

async function loadHosts(){
  try{
    const r=await fetch('/api/direct/hosts');
    if(r.ok){
      const d=await r.json();
      const el=document.getElementById('active-hosts');
      const ul=document.getElementById('host-list');
      ul.innerHTML='';
      if(d.hosts&&d.hosts.length>0){
        d.hosts.forEach(h=>{
          const li=document.createElement('li');
          li.style.cssText='display:flex;align-items:center;gap:.5rem;margin-bottom:.25rem';
          const span=document.createElement('span');span.textContent=h;
          const btn=document.createElement('button');
          btn.textContent='Disconnect';
          btn.style.cssText='font-size:.7rem;padding:1px 6px;cursor:pointer;background:#c0392b;color:#fff;border:none;border-radius:3px';
          btn.onclick=async()=>{
            btn.disabled=true;
            const [host,port]=h.split(':');
            const res=await fetch('/api/direct/connect',{method:'DELETE',headers:{'Content-Type':'application/json',...authHdr()},body:JSON.stringify({host,port:port?parseInt(port,10):undefined})}).catch(()=>null);
            if(res&&res.ok){li.remove();if(!ul.children.length)el.style.display='none';}
            else{btn.disabled=false;}
          };
          li.appendChild(span);li.appendChild(btn);ul.appendChild(li);
        });
        el.style.display='';
      }else{el.style.display='none';}
    }
  }catch(e){}
}

init();
</script>
</body>
</html>"#;

async fn serve_join(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<WebState>,
) -> impl IntoResponse {
    let direct_visible = if state.connector.is_some() { "block" } else { "none" };
    let default_port = state.connector.as_ref().map(|c| c.default_port).unwrap_or(55355);
    let client_ip = addr.ip().to_string();
    let direct_active = if state.connector.is_some() { "true" } else { "false" };
    let html = JOIN_HTML
        .replace("DIRECT_SECTION_DISPLAY", direct_visible)
        .replace("DIRECT_MODE_ACTIVE", direct_active)
        .replace("DEFAULT_PORT", &default_port.to_string())
        .replace("192.168.1.x", &client_ip)
        .replace("__CLIENT_IP__", &client_ip);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

#[derive(serde::Deserialize)]
struct DirectConnectBody {
    host: String,
    port: Option<u16>,
    /// Existing run ID to resume. Omit (or pass `null`) to start a new run.
    run_id: Option<u32>,
}

async fn api_direct_connect(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<DirectConnectBody>,
) -> impl IntoResponse {
    let Some(connector) = &state.connector else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Direct mode is not active."})),
        );
    };

    let host = body.host.trim().to_string();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "host must not be empty"})),
        );
    }

    // When resuming an existing run, require auth and access.
    // Returns the authenticated user_id so we can record the active run.
    let mut authed_user_id: Option<u32> = None;
    if let Some(run_id) = body.run_id {
        let Some(token) = extract_bearer(&headers) else {
            return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "authentication required to resume a run"})));
        };
        let check = tokio::task::spawn_blocking(move || -> Result<u32, (StatusCode, String)> {
            let user = fire_red_database::validate_session(&token)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
                .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;
            match fire_red_database::user_can_access_run(run_id, user.id) {
                Ok(true) => Ok(user.id),
                Ok(false) => Err((StatusCode::FORBIDDEN, "you do not have access to this run".into())),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
            }
        }).await;
        match check {
            Ok(Ok(uid)) => { authed_user_id = Some(uid); }
            Ok(Err((status, e))) => return (status, axum::Json(serde_json::json!({"error": e}))),
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "Task panicked"}))),
        }
    }

    let port = body.port.unwrap_or(connector.default_port);

    // When targeting a specific run, always disconnect the host first so it
    // can switch away from whatever run (if any) it was previously polling.
    if body.run_id.is_some() {
        connector.disconnect(&host, port);
    }

    let accepted = connector.connect(host.clone(), port, body.run_id);

    // Record user → run association so the overlay can auto-detect it.
    if let (Some(uid), Some(run_id)) = (authed_user_id, body.run_id) {
        state.user_active_run.lock().unwrap().insert(uid, run_id);
    }

    if accepted {
        tracing::info!("Direct mode: /join accepted {}:{} (run={:?})", host, port, body.run_id);
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "message": "Connection request received. Your slot will appear in a few seconds \
                            once the ROM is identified."
            })),
        )
    } else {
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "message": "Already connected.",
                "already": true
            })),
        )
    }
}

async fn api_direct_hosts(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    // Collect the direct_host values for slots this user can access.
    let my_hosts: HashSet<String> = {
        let slots = state.live_slots.lock_or_recover();
        slots
            .iter()
            .filter(|s| {
                let run_id = s.db.as_ref().and_then(|db| db.get_run_id());
                run_id.is_none_or(|rid| accessible.contains(&rid))
            })
            .filter_map(|s| s.direct_host.clone())
            .collect()
    };
    let all_hosts = state.connector.as_ref().map(|c| c.active_hosts()).unwrap_or_default();
    let hosts: Vec<String> = all_hosts.into_iter().filter(|h| my_hosts.contains(h)).collect();
    axum::Json(serde_json::json!({"hosts": hosts}))
}

#[derive(serde::Deserialize)]
struct DirectDisconnectBody {
    host: String,
    port: Option<u16>,
}

async fn api_direct_disconnect(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<DirectDisconnectBody>,
) -> impl IntoResponse {
    let Some(connector) = &state.connector else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Direct mode is not active."})),
        );
    };
    let host = body.host.trim().to_string();
    let port = body.port.unwrap_or(connector.default_port);
    let host_key = format!("{}:{}", host, port);
    // Only allow disconnect if the host belongs to a slot the user can access.
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    let owns_host = {
        let slots = state.live_slots.lock_or_recover();
        slots.iter().any(|s| {
            if s.direct_host.as_deref() != Some(&host_key) {
                return false;
            }
            let run_id = s.db.as_ref().and_then(|db| db.get_run_id());
            run_id.is_none_or(|rid| accessible.contains(&rid))
        })
    };
    if !owns_host {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"error": "access denied"})),
        );
    }
    if connector.disconnect(&host, port) {
        tracing::info!("Direct mode: disconnected {}:{}", host, port);
        (StatusCode::OK, axum::Json(serde_json::json!({"ok": true})))
    } else {
        (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Host not connected."})),
        )
    }
}

// ---------------------------------------------------------------------------
// New analytics handlers (v0.9.54)
// ---------------------------------------------------------------------------

async fn api_run_type_matchups(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::type_matchup_heatmap(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_run_ghost_compare(
    State(state): State<WebState>,
    Path((run_id, ghost_id)): Path<(u32, u32)>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::ghost_run_comparison(&conn, run_id, ghost_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_slot_shiny_pressure(
    State(state): State<WebState>,
    Path(slot_index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let run_id = {
        let slots = state.live_slots.lock_or_recover();
        let Some(slot) = slots.get(slot_index) else {
            return axum::Json(serde_json::json!({ "error": "Slot index out of range" }));
        };
        slot.db.as_ref().and_then(|db| db.active_run_id())
    };
    let Some(run_id) = run_id else {
        return axum::Json(serde_json::json!({ "error": "No active run for this slot" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::shiny_pressure(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_run_status_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_status_log(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

async fn api_run_dex(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::dex_count(&conn, run_id)
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// POST /api/run/:id/share — mint a 24-hour read-only share token for this run.
async fn api_create_share(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    if state.db_conn.is_none() {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    }
    let token = tokio::task::spawn_blocking(move || {
        fire_red_database::create_share_token(run_id, 86400)
    })
    .await
    .unwrap_or(None);
    match token {
        Some(t) => axum::Json(serde_json::json!({ "token": t, "ttl_secs": 86400 })),
        None => axum::Json(serde_json::json!({ "error": "Failed to create share token" })),
    }
}

/// GET /share/:token/state — return read-only run stats for the token's run.
async fn api_share_state(
    State(state): State<WebState>,
    Path(token): Path<String>,
) -> axum::Json<serde_json::Value> {
    let Some(conn) = state.db_conn else {
        return axum::Json(serde_json::json!({ "error": "No database configured" }));
    };
    let run_id = {
        let conn2 = conn.clone();
        tokio::task::spawn_blocking(move || fire_red_database::resolve_share_token(&conn2, &token))
            .await
            .unwrap_or(None)
    };
    let Some(run_id) = run_id else {
        return axum::Json(serde_json::json!({ "error": "Invalid or expired share token" }));
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::run_stats(&conn, run_id))
        .await
        .unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }));
    axum::Json(result)
}

/// POST /api/config/reload — re-parse the aggregator config file and validate it.
///
/// Returns `{ "ok": true, "path": "..." }` on success or `{ "error": "..." }` on
/// parse failure. Requires `config_path` to be populated (set from `--config` CLI arg).
/// Useful for verifying edits before a full restart.
async fn api_config_reload(
    State(state): State<WebState>,
) -> axum::Json<serde_json::Value> {
    let Some(path) = state.config_path else {
        return axum::Json(serde_json::json!({ "error": "No config path available (run with --config)" }));
    };
    let result = tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(&*path)
            .map_err(|e| format!("Cannot read config file: {e}"))?;
        let cfg: crate::config::AggregatorConfig = toml::from_str(&text)
            .map_err(|e| format!("TOML parse error: {e}"))?;
        Ok::<_, String>(serde_json::json!({
            "ok": true,
            "path": *path,
            "db": cfg.db.is_some(),
            "ws_port": cfg.ws_port,
            "twitch": cfg.twitch.is_some(),
            "discord_slash": cfg.discord_slash.is_some(),
        }))
    })
    .await
    .unwrap_or_else(|_| Err("Task panicked".into()));
    axum::Json(result.unwrap_or_else(|e| serde_json::json!({ "error": e })))
}

// ---------------------------------------------------------------------------
// New overlay handlers
// ---------------------------------------------------------------------------

/// POST /api/webhook/donation — ingest a StreamElements/Streamlabs donation alert.
///
/// Accepts generic JSON with a `type` field (`"donation"`, `"subscription"`, etc.)
/// and an optional `amount` (number). Fires a WebSocket overlay event to all
/// connected clients. If `heal_on_donation` is true in the query params, also
/// queues a `HealParty` command to all slots.
async fn api_donation_webhook(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let event_type = body.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("donation")
        .to_string();
    let amount = body.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let donor  = body.get("name").or_else(|| body.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or("Anonymous")
        .to_string();

    let ws_event = serde_json::json!({
        "event":  "donation",
        "type":   event_type,
        "amount": amount,
        "donor":  donor,
    });
    let _ = state.tx.send(serde_json::to_string(&ws_event).unwrap_or_default());

    if params.get("heal_on_donation").is_some_and(|v| v == "true" || v == "1") {
        let slots = state.live_slots.lock_or_recover();
        for slot in slots.iter() {
            slot.command_queue.lock_or_recover().push_back(ClientMessage::HealParty);
        }
    }

    axum::Json(serde_json::json!({ "ok": true }))
}

/// POST /api/savefile — import a Gen III `.sav` snapshot.
///
/// The body must be the raw binary savefile bytes (Content-Type: application/octet-stream).
/// Extracts the player name from the save game section and seeds a new run.
/// Returns the detected player name and a success/error status.
async fn api_import_savefile(
    State(_state): State<WebState>,
    body: axum::body::Bytes,
) -> axum::Json<serde_json::Value> {
    if body.len() < 0x20000 {
        return axum::Json(serde_json::json!({ "error": "Savefile too small (expected ≥ 128 KiB)" }));
    }
    // Gen III save has two save slots of 57 KiB each (0xE000 bytes).
    // Each slot is 14 sections of 4096 bytes. Section 0 contains the
    // trainer info at offset 0: player_name (7 bytes, FF-terminated), gender, etc.
    let player_name = parse_gen3_player_name(&body).unwrap_or_else(|| "Unknown".to_string());

    axum::Json(serde_json::json!({
        "ok": true,
        "player_name": player_name,
        "size_bytes": body.len(),
        "note": "Savefile accepted. Start a new run to associate it.",
    }))
}

/// Parse the player name from a Gen III savefile.
///
/// Looks at slot 1 section 0 (offset 0x0000) for the trainer info block.
/// Player name is 7 bytes at offset 0, encoded in Gen III character encoding.
fn parse_gen3_player_name(sav: &[u8]) -> Option<String> {
    // Try section 0 of save slot 1 (offset 0) first, then slot 2 (offset 0xE000).
    for base in [0usize, 0xE000] {
        if base + 8 > sav.len() { continue; }
        let name_bytes = &sav[base..base + 7];
        let name: String = name_bytes.iter()
            .take_while(|&&b| b != 0xFF)
            .map(|&b| gen3_char(b))
            .collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn gen3_char(b: u8) -> char {
    // Partial Gen III character table for Latin letters/digits.
    match b {
        0xBB => 'A', 0xBC => 'B', 0xBD => 'C', 0xBE => 'D', 0xBF => 'E',
        0xC0 => 'F', 0xC1 => 'G', 0xC2 => 'H', 0xC3 => 'I', 0xC4 => 'J',
        0xC5 => 'K', 0xC6 => 'L', 0xC7 => 'M', 0xC8 => 'N', 0xC9 => 'O',
        0xCA => 'P', 0xCB => 'Q', 0xCC => 'R', 0xCD => 'S', 0xCE => 'T',
        0xCF => 'U', 0xD0 => 'V', 0xD1 => 'W', 0xD2 => 'X', 0xD3 => 'Y',
        0xD4 => 'Z',
        0xD5 => 'a', 0xD6 => 'b', 0xD7 => 'c', 0xD8 => 'd', 0xD9 => 'e',
        0xDA => 'f', 0xDB => 'g', 0xDC => 'h', 0xDD => 'i', 0xDE => 'j',
        0xDF => 'k', 0xE0 => 'l', 0xE1 => 'm', 0xE2 => 'n', 0xE3 => 'o',
        0xE4 => 'p', 0xE5 => 'q', 0xE6 => 'r', 0xE7 => 's', 0xE8 => 't',
        0xE9 => 'u', 0xEA => 'v', 0xEB => 'w', 0xEC => 'x', 0xED => 'y',
        0xEE => 'z',
        0xA1 => '0', 0xA2 => '1', 0xA3 => '2', 0xA4 => '3', 0xA5 => '4',
        0xA6 => '5', 0xA7 => '6', 0xA8 => '7', 0xA9 => '8', 0xAA => '9',
        _ => '?',
    }
}

const DEX_HTML: &str = include_str!("dex.html");
const TYPECHART_HTML: &str = include_str!("typechart.html");

async fn serve_dex_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(DEX_HTML, state.testing, theme))
}

async fn serve_typechart_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(TYPECHART_HTML, state.testing, theme))
}

// ---------------------------------------------------------------------------
// User auth helpers + handlers
// ---------------------------------------------------------------------------

/// Extract a bearer token from `Authorization: Bearer <token>`,
/// `X-Session-Token: <token>`, or the `frt_token` cookie.
/// Returns `None` if none is present.
fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("Authorization")
        && let Ok(s) = v.to_str()
        && let Some(tok) = s.strip_prefix("Bearer ") {
            return Some(tok.trim().to_string());
    }
    if let Some(v) = headers.get("X-Session-Token")
        && let Ok(s) = v.to_str() {
            return Some(s.trim().to_string());
    }
    // Cookie fallback — works for same-origin page loads and WS upgrades.
    if let Some(v) = headers.get(header::COOKIE)
        && let Ok(s) = v.to_str() {
        for part in s.split(';') {
            if let Some(val) = part.trim().strip_prefix("frt_token=") {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

/// Extract a `?token=<value>` query parameter from a URI.
/// Used by OBS browser sources that embed the token in the URL.
fn extract_query_token(uri: &axum::http::Uri) -> Option<String> {
    let q = uri.query()?;
    for pair in q.split('&') {
        if let Some(val) = pair.strip_prefix("token=")
            && !val.is_empty()
        {
            return Some(val.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Auth / access-control middleware
// ---------------------------------------------------------------------------

/// Routes that do not require a valid session.
fn is_public_route(path: &str, method: &axum::http::Method) -> bool {
    matches!(path, "/" | "/register" | "/interactions" | "/api/webhook/donation")
        || path == "/api/login"
        || path.starts_with("/share/")
        // POST /api/users = register endpoint
        || (path == "/api/users" && method == axum::http::Method::POST)
}

/// Global authentication middleware — validates the session on every
/// non-public route and injects [`User`] into request extensions.
/// Unauthenticated page requests are redirected to `/`; API/WS requests
/// receive `401 Unauthorized`.
async fn auth_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path   = request.uri().path().to_string();
    let method = request.method().clone();

    if is_public_route(&path, &method) {
        return next.run(request).await;
    }

    let token = extract_query_token(request.uri())
        .or_else(|| extract_bearer(request.headers()));

    let user: Option<User> = if let Some(tok) = token {
        tokio::task::spawn_blocking(move || fire_red_database::validate_session(&tok))
            .await
            .unwrap_or(Ok(None))
            .unwrap_or(None)
    } else {
        None
    };

    match user {
        Some(u) => {
            let mut req = request;
            req.extensions_mut().insert(u);
            next.run(req).await
        }
        None => {
            if path.starts_with("/api/") || path == "/ws" {
                axum::response::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(r#"{"error":"authentication required"}"#))
                    .unwrap()
            } else {
                axum::response::Redirect::to("/").into_response()
            }
        }
    }
}

/// Per-run access middleware — checks `user_can_access_run` for any path
/// that looks like `/api/run/<numeric-id>/…`.
///
/// Exceptions: invite-flow paths where the user doesn't yet have access
/// (`/invite/accept`, `/invite/decline`, `/invite/request`).
async fn run_access_middleware(
    Extension(user): Extension<User>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path();

    // Invite-flow routes: caller may not have access yet.
    if path.ends_with("/invite/accept")
        || path.ends_with("/invite/decline")
        || (path.ends_with("/invite/request") && request.method() == axum::http::Method::POST)
    {
        return next.run(request).await;
    }

    // Extract numeric run_id from /api/run/<id>/…
    let run_id: Option<u32> = path
        .strip_prefix("/api/run/")
        .and_then(|s| s.split('/').next())
        .and_then(|s| s.parse().ok());

    if let Some(rid) = run_id {
        let uid = user.id;
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);

        if !can {
            return axum::response::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(r#"{"error":"access denied"}"#))
                .unwrap();
        }
    }

    next.run(request).await
}

/// Per-slot access middleware — for all requests to `/api/slot/<idx>/…`,
/// verifies the authenticated user has access to that slot's run.
async fn slot_access_middleware(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path();
    let slot_idx: Option<usize> = path
        .strip_prefix("/api/slot/")
        .and_then(|s| s.split('/').next())
        .and_then(|s| s.parse().ok());

    if let Some(idx) = slot_idx {
        let run_id = {
            let lock = state.live_slots.lock_or_recover();
            lock.get(idx).and_then(|s| s.db.as_ref().and_then(|db| db.get_run_id()))
        };

        if let Some(rid) = run_id {
            let uid = user.id;
            let can = tokio::task::spawn_blocking(move || {
                fire_red_database::user_can_access_run(rid, uid)
            })
            .await
            .unwrap_or(Ok(false))
            .unwrap_or(false);

            if !can {
                return axum::response::Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(r#"{"error":"access denied"}"#))
                    .unwrap();
            }
        }
    }

    next.run(request).await
}

/// Filter a JSON slot-array string for `user_id`.
///
/// Array positions are **preserved** — inaccessible slots are replaced with
/// `null` rather than removed.  This ensures that overlay URLs such as
/// `/1/alerts` (which index into the array by position) still work correctly
/// when multiple users share the same server.
///
/// A slot with no `active_run_id` is kept as-is (accessible to all
/// authenticated users, e.g. unlinked tracker-TCP connections).
async fn filter_slots_for_user(json: &str, user_id: u32) -> String {
    let arr: serde_json::Value =
        serde_json::from_str(json).unwrap_or(serde_json::Value::Array(vec![]));
    let slots = match arr.as_array() {
        Some(s) => s.clone(),
        None => return "[]".to_string(),
    };

    let accessible: HashSet<u32> =
        tokio::task::spawn_blocking(move || fire_red_database::get_accessible_run_ids(user_id))
            .await
            .unwrap_or(Ok(HashSet::new()))
            .unwrap_or_default();

    // Replace inaccessible slots with null (preserve position).
    let filtered: Vec<serde_json::Value> = slots
        .into_iter()
        .map(|slot| {
            match slot.get("active_run_id").and_then(|v| v.as_u64()) {
                None => slot,                                           // unlinked → keep
                Some(rid) if accessible.contains(&(rid as u32)) => slot, // owned → keep
                _ => serde_json::Value::Null,                          // forbidden → null
            }
        })
        .collect();

    serde_json::to_string(&serde_json::Value::Array(filtered))
        .unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Deserialize)]
struct RegisterBody {
    username: String,
    password: String,
}

/// `POST /api/users` — register a new user account.
///
/// Body: `{ "username": "...", "password": "..." }` (password ≥ 8 chars)
/// Returns: `{ "id": N, "username": "..." }` or `{ "error": "..." }` with
/// `409 Conflict` when the username is already taken.
async fn api_register_user(
    axum::Json(body): axum::Json<RegisterBody>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::create_user(&body.username, &body.password)
    }).await;
    match result {
        Ok(Ok(user)) => (
            StatusCode::CREATED,
            axum::Json(serde_json::json!({
                "id": user.id,
                "username": user.username,
                "created_at": user.created_at,
            })),
        ),
        Ok(Err(e)) if e.contains("already taken") => (
            StatusCode::CONFLICT,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "Task panicked" })),
        ),
    }
}

/// `GET /api/users` — list all registered users (admin view).
///
/// Returns: `[{ "id": N, "username": "...", "created_at": N }, ...]`
async fn api_list_users() -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(fire_red_database::list_users).await;
    match result {
        Ok(Ok(users)) => {
            let arr: Vec<_> = users.iter().map(|u| serde_json::json!({
                "id": u.id,
                "username": u.username,
                "created_at": u.created_at,
            })).collect();
            (StatusCode::OK, axum::Json(serde_json::json!(arr)))
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "Task panicked" })),
        ),
    }
}

/// `POST /api/login` — authenticate and get a session token.
///
/// Body: `{ "username": "...", "password": "..." }`
/// Returns: `{ "token": "...", "user": { "id": N, "username": "..." } }` and
/// sets an `HttpOnly` `frt_token` cookie so browser page-loads are authenticated
/// automatically. Returns `401` on bad credentials.
async fn api_login(
    axum::Json(body): axum::Json<RegisterBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let username_for_log = body.username.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<(fire_red_database::User, String)>, String> {
        let user = fire_red_database::authenticate_user(&body.username, &body.password)?;
        match user {
            Some(u) => {
                let token = fire_red_database::create_session(u.id)?;
                Ok(Some((u, token)))
            }
            None => Ok(None),
        }
    }).await;
    match result {
        Ok(Ok(Some((user, token)))) => {
            let cookie = format!(
                "frt_token={}; HttpOnly; Path=/; SameSite=Lax; Max-Age=2592000",
                token
            );
            (
                StatusCode::OK,
                [(header::SET_COOKIE, cookie)],
                axum::Json(serde_json::json!({
                    "token": token,
                    "user": { "id": user.id, "username": user.username },
                })),
            ).into_response()
        }
        Ok(Ok(None)) => {
            tracing::warn!(username = %username_for_log, "POST /api/login → 401");
            (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "invalid username or password" })),
            ).into_response()
        }
        Ok(Err(e)) => {
            tracing::error!(username = %username_for_log, error = %e, "POST /api/login → 500 (DB error)");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": e })),
            ).into_response()
        }
        Err(_) => {
            tracing::error!(username = %username_for_log, "POST /api/login → 500 (task panicked)");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "Task panicked" })),
            ).into_response()
        }
    }
}

/// `POST /api/logout` — invalidate the current session token.
///
/// Requires `Authorization: Bearer <token>` or `X-Session-Token: <token>`.
/// Returns `200` whether or not the token existed.
async fn api_logout(
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = extract_bearer(&headers) {
        tokio::task::spawn_blocking(move || fire_red_database::delete_session(&token))
            .await
            .ok();
    }
    // Clear the frt_token cookie in the browser.
    (
        StatusCode::OK,
        [(header::SET_COOKIE, "frt_token=; HttpOnly; Path=/; SameSite=Lax; Max-Age=0")],
    )
}

/// `GET /api/me` — return the currently authenticated user.
///
/// Requires `Authorization: Bearer <token>` or `X-Session-Token: <token>`.
/// Returns `{ "id": N, "username": "..." }` or `401` if the token is missing
/// or expired.
async fn api_me(
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "no session token provided" })),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::validate_session(&token)
    }).await;
    match result {
        Ok(Ok(Some(user))) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "id": user.id,
                "username": user.username,
                "created_at": user.created_at,
            })),
        ),
        Ok(Ok(None)) => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "session expired or invalid" })),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        ),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "Task panicked" })),
        ),
    }
}

/// `GET /api/me/active_run` — return the run_id this user most recently connected to.
///
/// Returns `{ "run_id": N }` or `{ "run_id": null }` if none recorded.
async fn api_me_active_run(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "authentication required"})));
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::validate_session(&token)).await;
    let user = match result {
        Ok(Ok(Some(u))) => u,
        Ok(Ok(None)) => return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "session expired"}))),
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e}))),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "Task panicked"}))),
    };
    let run_id = state.user_active_run.lock().unwrap().get(&user.id).copied();
    (StatusCode::OK, axum::Json(serde_json::json!({"run_id": run_id})))
}

/// `PUT /api/me/active_run` — explicitly set the caller's active run.
///
/// Body: `{ "run_id": N }`.  Used by the page-selector dropdown so that
/// selecting a run page on the join/dashboard also updates auto-detect.
async fn api_me_set_active_run(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "authentication required"})));
    };
    let run_id = match body.get("run_id").and_then(|v| v.as_u64()) {
        Some(id) => id as u32,
        None => return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error": "run_id required"}))),
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::validate_session(&token)).await;
    let user = match result {
        Ok(Ok(Some(u))) => u,
        Ok(Ok(None)) => return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "session expired"}))),
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e}))),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "Task panicked"}))),
    };
    state.user_active_run.lock().unwrap().insert(user.id, run_id);
    (StatusCode::OK, axum::Json(serde_json::json!({"ok": true})))
}

/// `GET /api/user/:id/runs` — list runs for a user (own account only).
async fn api_user_runs(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(user_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    if user.id != user_id {
        return axum::Json(serde_json::json!({ "error": "access denied" }));
    }
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::list_runs_for_user_json(&conn, user_id)
    })
    .await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

// ---------------------------------------------------------------------------
// Login / landing page  (served at "/")
// ---------------------------------------------------------------------------

const LOGIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;padding:2rem 1rem}
.card{background:#16213e;border:1px solid #0f3460;border-radius:10px;padding:2rem;width:100%;max-width:380px}
h1{font-size:1.4rem;color:#e94560;margin-bottom:.3rem;text-align:center}
.subtitle{font-size:.8rem;color:#556;text-align:center;margin-bottom:1.8rem}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input{width:100%;padding:.55rem .75rem;background:#0f3460;border:1px solid #444;border-radius:5px;color:#eee;font-size:.9rem;margin-bottom:1rem}
input:focus{outline:none;border-color:#e94560}
.btn{display:block;width:100%;padding:.55rem;border:none;border-radius:5px;font-size:.9rem;cursor:pointer;text-align:center;text-decoration:none;line-height:1.4}
.btn-primary{background:#e94560;color:#fff;margin-bottom:.7rem}
.btn-primary:hover{background:#c73652}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499;margin-bottom:.5rem}
.btn-secondary:hover{background:#253d6a}
.msg{margin-top:.5rem;padding:.5rem;border-radius:4px;text-align:center;font-size:.82rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.divider{border:none;border-top:1px solid #1e3a6e;margin:1.2rem 0}
.links{display:flex;flex-direction:column;gap:.5rem}
.user-info{text-align:center;margin-bottom:1rem;font-size:.9rem;color:#7dce7d}
.hint{font-size:.75rem;color:#556;text-align:center;margin-top:.4rem}
</style>
</head>
<body>
<div class="card">
  <h1>🔴 Fire Red Tracker</h1>
  <p class="subtitle">Nuzlocke run tracker</p>

  <!-- Logged-out state -->
  <div id="login-wrap">
    <label for="uname">Username</label>
    <input id="uname" type="text" placeholder="your username" autocomplete="username">
    <label for="upass">Password</label>
    <input id="upass" type="password" placeholder="••••••••" autocomplete="current-password" onkeydown="if(event.key==='Enter')doLogin()">
    <button class="btn btn-primary" onclick="doLogin()">Log In</button>
    <p class="hint">No account? <a href="/register" style="color:#5090e0">Register here</a></p>
    <div id="msg-login" class="msg"></div>
    <hr class="divider">
    <div class="links">
      <a class="btn btn-secondary" href="/overlay">Overlay (anonymous)</a>
      <a class="btn btn-secondary" href="/join">Join / Run Select</a>
    </div>
  </div>

  <!-- Logged-in state -->
  <div id="loggedin-wrap" style="display:none">
    <div class="user-info" id="user-info"></div>
    <div class="links">
      <a class="btn btn-primary" href="/overlay" id="overlay-link">Overlay</a>
      <a class="btn btn-secondary" href="/dashboard">Dashboard</a>
      <a class="btn btn-secondary" href="/join">Join / Run Select</a>
      <a class="btn btn-secondary" href="/history">Run History</a>
    </div>
    <hr class="divider">
    <button class="btn btn-secondary" onclick="doLogout()">Log Out</button>
  </div>
</div>

<script>
const TOKEN_KEY='frt_session';
let SESSION=localStorage.getItem(TOKEN_KEY)||null;

function authHdr(){return SESSION?{'Authorization':'Bearer '+SESSION}:{};}
function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}

async function init(){
  if(!SESSION){return;}
  const r=await fetch('/api/me',{headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    const me=await r.json();
    showLoggedIn(me);
  }else{
    SESSION=null;localStorage.removeItem(TOKEN_KEY);
  }
}

function showLoggedIn(me){
  document.getElementById('login-wrap').style.display='none';
  document.getElementById('loggedin-wrap').style.display='';
  document.getElementById('user-info').textContent='Logged in as '+esc(me.username);
}

async function doLogin(){
  const u=document.getElementById('uname').value.trim();
  const p=document.getElementById('upass').value;
  const msg=document.getElementById('msg-login');
  msg.className='msg';
  if(!u||!p){msg.className='msg err';msg.textContent='Enter username and password.';return;}
  const r=await fetch('/api/login',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({username:u,password:p})}).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    SESSION=d.token;
    localStorage.setItem(TOKEN_KEY,SESSION);
    showLoggedIn(d.user);
  }else{
    msg.className='msg err';msg.textContent=d.error||'Login failed.';
  }
}

async function doLogout(){
  await fetch('/api/logout',{method:'POST',headers:authHdr()}).catch(()=>null);
  SESSION=null;localStorage.removeItem(TOKEN_KEY);
  document.getElementById('loggedin-wrap').style.display='none';
  document.getElementById('login-wrap').style.display='';
  document.getElementById('upass').value='';
}

init();
</script>
</body>
</html>"#;

async fn serve_login_page() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        LOGIN_HTML,
    )
}

// ---------------------------------------------------------------------------
// Dashboard page
// ---------------------------------------------------------------------------

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Dashboard – Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;padding:2rem 1rem}
.container{max-width:900px;margin:0 auto}
h1{font-size:1.5rem;color:#e94560;margin-bottom:1.5rem;display:flex;align-items:center;justify-content:space-between}
h1 a{font-size:.85rem;color:#5090e0;text-decoration:none}
h1 a:hover{text-decoration:underline}
.section{background:#16213e;border:1px solid #0f3460;border-radius:8px;padding:1.5rem;margin-bottom:1.5rem}
.section-title{font-size:.95rem;font-weight:700;color:#ccc;margin-bottom:1rem;padding-bottom:.5rem;border-bottom:1px solid #1e3a6e}
.stat-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(130px,1fr));gap:1rem;margin-bottom:.5rem}
.stat-card{background:#0f3460;border-radius:6px;padding:1rem;text-align:center}
.stat-num{font-size:1.8rem;font-weight:700;color:#e94560;line-height:1}
.stat-label{font-size:.75rem;color:#888;margin-top:.3rem;text-transform:uppercase;letter-spacing:.5px}
.btn{display:inline-block;padding:.4rem 1rem;border:none;border-radius:4px;font-size:.85rem;cursor:pointer;text-decoration:none;line-height:1.4}
.btn-primary{background:#e94560;color:#fff}
.btn-primary:hover{background:#c73652}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499}
.btn-secondary:hover{background:#253d6a}
.btn-success{background:#1a5c2e;color:#7dce7d;border:1px solid #2d8a2d}
.btn-success:hover{background:#1e6a34}
.btn-danger{background:#5c1a1a;color:#ce7d7d;border:1px solid #8a2d2d}
.btn-danger:hover{background:#6a1e1e}
.btn-connect{background:#0f3a4a;color:#7dd;border:1px solid #1a6a7a}
.btn-connect:hover{background:#145060}
.btn-sm{padding:.28rem .65rem;font-size:.78rem}
.btn-xs{padding:.18rem .5rem;font-size:.72rem}
.page-select{background:#1e3a6e;color:#aad;border:1px solid #2d5499;border-radius:4px;padding:.18rem .5rem;font-size:.72rem;cursor:pointer}
.page-select:focus{outline:none;border-color:#e94560}
table{width:100%;border-collapse:collapse;font-size:.85rem}
th{text-align:left;color:#888;font-weight:600;font-size:.72rem;text-transform:uppercase;letter-spacing:.4px;padding:.4rem .6rem;border-bottom:1px solid #1e3a6e}
td{padding:.42rem .6rem;border-bottom:1px solid rgba(255,255,255,0.04);vertical-align:middle}
tr:hover td{background:rgba(255,255,255,0.03)}
.run-id{color:#5090e0;font-weight:600}
.deaths{color:#e06060}
.catches{color:#60d060}
.badge-owner{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a5c;color:#5090e0;border:1px solid #2d5499;vertical-align:middle;margin-left:.35rem}
.badge-invited{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a1a;color:#7dce7d;border:1px solid #2d8a2d;vertical-align:middle;margin-left:.35rem}
.party-grid{display:flex;flex-wrap:wrap;gap:.6rem}
.party-mon{background:#0f3460;border-radius:6px;padding:.6rem .9rem;min-width:110px;font-size:.82rem}
.mon-name{font-weight:600;color:#eee}
.mon-species{color:#888;font-size:.75rem}
.mon-level{color:#5090e0;font-size:.75rem}
.mon-shiny{color:#f0d060;font-size:.7rem;margin-left:.3rem}
.invite-row{display:flex;align-items:center;gap:.7rem;padding:.6rem 0;border-bottom:1px solid rgba(255,255,255,0.05);flex-wrap:wrap}
.invite-row:last-child{border-bottom:none}
.invite-info{flex:1;font-size:.85rem}
.invite-run{color:#5090e0;font-weight:600}
.invite-from{color:#888;font-size:.78rem}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input[type=text]{width:100%;padding:.5rem .7rem;background:#0f3460;border:1px solid #444;border-radius:4px;color:#eee;font-size:.9rem;margin-bottom:.8rem}
input:focus{outline:none;border-color:#e94560}
.form-row{display:flex;gap:.6rem;align-items:flex-end}
.form-row>*{flex:1;margin-bottom:0}
.form-row .btn{flex:0 0 auto}
.msg{margin-top:.6rem;padding:.5rem;border-radius:4px;text-align:center;font-size:.85rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.empty{color:#666;font-size:.85rem;text-align:center;padding:1.5rem 0}
.loading{color:#888;font-size:.85rem}
.td-actions{text-align:right;white-space:nowrap;display:flex;justify-content:flex-end;gap:3px;flex-wrap:wrap}
</style>
</head>
<body>
<div class="container">
<h1><span id="page-title">Dashboard</span> <a href="/join">← Back to Join</a></h1>

<!-- ── Stats overview ──────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">Overview</div>
  <div class="stat-grid" id="stat-grid">
    <div class="stat-card"><div class="stat-num" id="stat-runs">—</div><div class="stat-label">Total Runs</div></div>
    <div class="stat-card"><div class="stat-num" id="stat-catches">—</div><div class="stat-label">Caught</div></div>
    <div class="stat-card"><div class="stat-num" id="stat-deaths">—</div><div class="stat-label">Deaths</div></div>
    <div class="stat-card"><div class="stat-num" id="stat-encounters">—</div><div class="stat-label">Encounters</div></div>
  </div>
</div>

<!-- ── Open runs ───────────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">Open Runs</div>
  <div id="open-runs-status" class="loading">Loading…</div>
  <table id="open-runs-table" style="display:none">
    <thead><tr><th>#</th><th>Player</th><th>Started</th><th>Caught</th><th>Deaths</th><th>Invite</th><th></th></tr></thead>
    <tbody id="open-runs-body"></tbody>
  </table>
</div>

<!-- ── Most recent party ───────────────────────────────────────────── -->
<div class="section" id="party-section" style="display:none">
  <div class="section-title">Current Party <span id="party-run-label" style="color:#666;font-size:.8rem;font-weight:400"></span></div>
  <div class="party-grid" id="party-grid"></div>
</div>

<!-- ── Pending invites ─────────────────────────────────────────────── -->
<div class="section" id="invites-section" style="display:none">
  <div class="section-title">Pending Run Invites</div>
  <div id="invites-list"></div>
</div>

</div><!-- /container -->

<!-- Invite modal overlay -->
<div id="invite-modal" style="display:none;position:fixed;inset:0;background:rgba(0,0,0,.7);z-index:100;align-items:center;justify-content:center">
  <div style="background:#16213e;border:1px solid #0f3460;border-radius:8px;padding:1.5rem;width:340px;max-width:95vw">
    <div style="font-size:.95rem;font-weight:700;color:#ccc;margin-bottom:1rem">Invite User to Run <span id="modal-run-id" style="color:#5090e0"></span></div>
    <label for="invite-username">Username to invite</label>
    <input id="invite-username" type="text" placeholder="their username" autocomplete="off">
    <div id="msg-invite" class="msg"></div>
    <div style="display:flex;gap:.6rem;margin-top:.5rem">
      <button class="btn btn-primary" style="flex:1" onclick="submitInvite()">Send Invite</button>
      <button class="btn btn-secondary" onclick="closeInviteModal()">Cancel</button>
    </div>
  </div>
</div>

<script>
const TOKEN_KEY='frt_session';
const CLIENT_IP='__CLIENT_IP__';
const DIRECT_PORT=DEFAULT_PORT;
const DIRECT_ACTIVE=DIRECT_MODE_ACTIVE;
let SESSION=localStorage.getItem(TOKEN_KEY)||null;
let MODAL_RUN_ID=null;

function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function fmtDate(iso){if(!iso)return'—';try{return new Date(iso).toLocaleDateString();}catch{return iso;}}
function authHdr(){return SESSION?{'Authorization':'Bearer '+SESSION}:{};}
function openRunPage(runId,sel){
  const p=sel.value;sel.value='';if(!p)return;
  const url=p==='stats'?'/run/'+runId+'/stats':'/'+p+'?run='+runId;
  const tok=localStorage.getItem(TOKEN_KEY);
  if(tok)fetch('/api/me/active_run',{method:'PUT',headers:{'Content-Type':'application/json','Authorization':'Bearer '+tok},body:JSON.stringify({run_id:runId})}).catch(()=>{});
  window.open(url,'_blank');
}

async function quickConnect(runId){
  const r=await fetch('/api/direct/connect',{
    method:'POST',
    headers:{'Content-Type':'application/json',...authHdr()},
    body:JSON.stringify({host:CLIENT_IP,port:DIRECT_PORT,run_id:runId}),
  }).catch(()=>null);
  if(!r){alert('Network error.');return;}
  const d=await r.json();
  if(r.ok){window.open('/overlay?run='+runId,'_blank');}
  else{alert(d.error||'Connection failed.');}
}

async function init(){
  if(!SESSION){window.location.href='/join';return;}
  const r=await fetch('/api/me',{headers:authHdr()}).catch(()=>null);
  if(!r||!r.ok){window.location.href='/join';return;}
  const me=await r.json();
  document.getElementById('page-title').textContent='Dashboard — '+esc(me.username);
  loadDashboard();
}

async function loadDashboard(){
  const r=await fetch('/api/me/dashboard',{headers:authHdr()}).catch(()=>null);
  if(!r||!r.ok){
    document.getElementById('open-runs-status').textContent='Could not load dashboard.';
    return;
  }
  const d=await r.json();
  if(d.error){document.getElementById('open-runs-status').textContent=d.error;return;}

  // Stats
  const s=d.stats||{};
  document.getElementById('stat-runs').textContent=s.runs??0;
  document.getElementById('stat-catches').textContent=s.catches??0;
  document.getElementById('stat-deaths').textContent=s.deaths??0;
  document.getElementById('stat-encounters').textContent=s.encounters??0;

  // Open runs
  const runs=d.open_runs||[];
  const st=document.getElementById('open-runs-status');
  const tbl=document.getElementById('open-runs-table');
  const tbody=document.getElementById('open-runs-body');
  if(!runs.length){st.textContent='No open runs.';st.style.display='';tbl.style.display='none';}
  else{
    st.style.display='none';tbl.style.display='';
    tbody.innerHTML='';
    for(const run of runs){
      const tr=document.createElement('tr');
      const ownerBadge=run.is_owner
        ?'<span class="badge-owner">owner</span>'
        :'<span class="badge-invited">invited</span>';
      const inviteBtn=run.is_owner
        ?'<button class="btn btn-secondary btn-xs" onclick="openInviteModal('+run.id+')">Invite</button>'
        :'';
      tr.innerHTML=
        '<td><span class="run-id">#'+run.id+'</span>'+ownerBadge+'</td>'
        +'<td>'+esc(run.player_name||'—')+'</td>'
        +'<td style="color:#888;font-size:.8rem">'+fmtDate(run.started_at)+'</td>'
        +'<td><span class="catches">'+(run.catches??0)+'</span></td>'
        +'<td><span class="deaths">'+(run.deaths??0)+'</span></td>'
        +'<td>'+inviteBtn+'</td>'
        +'<td class="td-actions">'
          +'<select class="page-select" onchange="openRunPage('+run.id+',this)"><option value="">Open page…</option><option value="overlay">Overlay</option><option value="history">History</option><option value="stats">Stats</option><option value="shiny">Shiny</option><option value="memorial">Memorial</option><option value="trainers">Trainers</option><option value="timeline">Timeline</option></select>'
          +(DIRECT_ACTIVE?' <button class="btn btn-connect btn-xs" onclick="quickConnect('+run.id+')" title="Connect your RetroArch and open overlay">Quick Connect</button>':'')
        +'</td>';
      tbody.appendChild(tr);
    }
  }

  // Recent party
  const party=d.recent_party||[];
  if(party.length&&runs.length){
    const ps=document.getElementById('party-section');
    const pg=document.getElementById('party-grid');
    const rl=document.getElementById('party-run-label');
    rl.textContent='(Run #'+runs[0].id+')';
    pg.innerHTML='';
    for(const mon of party){
      const div=document.createElement('div');
      div.className='party-mon';
      div.innerHTML=
        '<div class="mon-name">'+esc(mon.nickname)+(mon.is_shiny?'<span class="mon-shiny">★</span>':'')+'</div>'
        +'<div class="mon-species">'+esc(mon.species_name)+'</div>'
        +'<div class="mon-level">Lv. '+mon.level+'</div>';
      pg.appendChild(div);
    }
    ps.style.display='';
  }

  // Pending invites
  const invites=d.pending_invites||[];
  if(invites.length){
    const sec=document.getElementById('invites-section');
    const list=document.getElementById('invites-list');
    list.innerHTML='';
    for(const inv of invites){
      const row=document.createElement('div');
      row.className='invite-row';
      row.id='invite-row-'+inv.invite_id;
      row.innerHTML=
        '<div class="invite-info">'
          +'<span class="invite-run">Run #'+inv.run_id+'</span>'
          +' <span style="color:#ccc">'+esc(inv.player_name)+'</span>'
          +'<div class="invite-from">Invited by '+esc(inv.invited_by)+' · '+fmtDate(inv.created_at)+'</div>'
        +'</div>'
        +'<button class="btn btn-success btn-sm" onclick="respondInvite('+inv.run_id+',true,'+inv.invite_id+')">Accept</button>'
        +'<button class="btn btn-danger btn-sm" onclick="respondInvite('+inv.run_id+',false,'+inv.invite_id+')">Decline</button>';
      list.appendChild(row);
    }
    sec.style.display='';
  }
}

function openInviteModal(runId){
  MODAL_RUN_ID=runId;
  document.getElementById('modal-run-id').textContent='#'+runId;
  document.getElementById('invite-username').value='';
  document.getElementById('msg-invite').className='msg';
  document.getElementById('invite-modal').style.display='flex';
  setTimeout(()=>document.getElementById('invite-username').focus(),50);
}
function closeInviteModal(){
  document.getElementById('invite-modal').style.display='none';
  MODAL_RUN_ID=null;
}
async function submitInvite(){
  const uname=document.getElementById('invite-username').value.trim();
  const msg=document.getElementById('msg-invite');
  if(!uname){msg.className='msg err';msg.textContent='Enter a username.';return;}
  const r=await fetch('/api/run/'+MODAL_RUN_ID+'/invite',{
    method:'POST',
    headers:{'Content-Type':'application/json',...authHdr()},
    body:JSON.stringify({username:uname}),
  }).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    msg.className='msg ok';msg.textContent='Invite sent to '+esc(uname)+'.';
    setTimeout(closeInviteModal,1400);
  }else{
    msg.className='msg err';msg.textContent=d.error||'Failed.';
  }
}
async function respondInvite(runId,accept,inviteId){
  const endpoint=accept?'accept':'decline';
  const r=await fetch('/api/run/'+runId+'/invite/'+endpoint,{
    method:'POST',
    headers:authHdr(),
  }).catch(()=>null);
  if(r&&r.ok){
    const row=document.getElementById('invite-row-'+inviteId);
    if(row)row.remove();
    const sec=document.getElementById('invites-section');
    const list=document.getElementById('invites-list');
    if(!list.children.length)sec.style.display='none';
    if(accept)loadDashboard();
  }
}

init();
</script>
</body>
</html>"#;

async fn serve_dashboard(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<WebState>,
) -> impl IntoResponse {
    let client_ip = addr.ip().to_string();
    let default_port = state.connector.as_ref().map(|c| c.default_port).unwrap_or(55355);
    let direct_active = if state.connector.is_some() { "true" } else { "false" };
    let html = DASHBOARD_HTML
        .replace("DIRECT_MODE_ACTIVE", direct_active)
        .replace("DEFAULT_PORT", &default_port.to_string())
        .replace("__CLIENT_IP__", &client_ip);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

/// `GET /api/me/dashboard` — full dashboard JSON for the authenticated user.
async fn api_me_dashboard(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::user_dashboard_json(&conn, user.id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

#[derive(serde::Deserialize)]
struct InviteBody { username: String }

/// `POST /api/run/:id/invite` — invite a user (by username) to a run.
///
/// Requires auth. The caller must own the run.
/// Body: `{ "username": "..." }`
async fn api_run_invite(
    State(_state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
    axum::Json(body): axum::Json<InviteBody>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let username = body.username.trim().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<u32, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::invite_user_to_run(run_id, user.id, &username)
    }).await;
    match result {
        Ok(Ok(invite_id)) => (StatusCode::OK, axum::Json(serde_json::json!({ "invite_id": invite_id }))),
        Ok(Err(e)) if e.contains("do not own") || e.contains("not found") => {
            (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "error": e })))
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/run/:id/invites` — list all invites for a run (owner view).
async fn api_run_invites(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::get_run_invites_json(&conn, run_id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/invite/accept` — accept an invite to a run.
async fn api_run_invite_accept(
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    api_run_invite_respond(headers, run_id, true).await
}

/// `POST /api/run/:id/invite/decline` — decline an invite to a run.
async fn api_run_invite_decline(
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    api_run_invite_respond(headers, run_id, false).await
}

async fn api_run_invite_respond(
    headers: axum::http::HeaderMap,
    run_id: u32,
    accept: bool,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::respond_to_invite(run_id, user.id, accept)
    }).await;
    match result {
        Ok(Ok(())) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/invite/request` — request access to a run.
///
/// Any authenticated user who does not own the run may call this.
async fn api_run_invite_request(
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<u32, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::request_run_invite(run_id, user.id)
    }).await;
    match result {
        Ok(Ok(invite_id)) => (StatusCode::OK, axum::Json(serde_json::json!({ "invite_id": invite_id }))),
        Ok(Err(e)) if e.contains("already own") => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e }))),
        Ok(Err(e)) if e.contains("not found") => (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": e }))),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/run/:id/invite/requests` — list pending access requests (owner only).
async fn api_run_invite_requests(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::get_run_invite_requests_json(&conn, run_id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/invite/request/:uid/approve` — approve an access request.
async fn api_run_invite_request_approve(
    headers: axum::http::HeaderMap,
    Path((run_id, requester_id)): Path<(u32, u32)>,
) -> impl IntoResponse {
    api_run_invite_request_respond(headers, run_id, requester_id, true).await
}

/// `POST /api/run/:id/invite/request/:uid/deny` — deny an access request.
async fn api_run_invite_request_deny(
    headers: axum::http::HeaderMap,
    Path((run_id, requester_id)): Path<(u32, u32)>,
) -> impl IntoResponse {
    api_run_invite_request_respond(headers, run_id, requester_id, false).await
}

async fn api_run_invite_request_respond(
    headers: axum::http::HeaderMap,
    run_id: u32,
    requester_id: u32,
    approve: bool,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::respond_to_invite_request(run_id, requester_id, user.id, approve)
    }).await;
    match result {
        Ok(Ok(())) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))),
        Ok(Err(e)) if e.contains("do not own") => (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "error": e }))),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/me/run_statuses` — map of run_id → access status for the caller.
async fn api_me_run_statuses(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::get_my_run_statuses_json(&conn, user.id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/me/run_requests` — all pending access requests on runs the caller owns.
async fn api_me_run_requests(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::get_my_run_requests_json(&conn, user.id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_HTML: &str =
        r#"<!DOCTYPE html><html><head><!-- THEME_SLOT --></head><body>__VERSION__</body></html>"#;

    #[test]
    fn apply_page_replaces_version() {
        let out = apply_page(SAMPLE_HTML, false);
        assert!(out.contains(VERSION), "VERSION not injected");
        assert!(!out.contains("__VERSION__"), "__VERSION__ not replaced");
    }

    #[test]
    fn apply_page_no_theme_removes_slot() {
        let out = apply_page(SAMPLE_HTML, false);
        assert!(
            !out.contains("<!-- THEME_SLOT -->"),
            "theme slot should be removed"
        );
        assert!(!out.contains("data-theme"), "no theme attr expected");
    }

    #[test]
    fn apply_page_with_theme_dark_removes_slot() {
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("dark"));
        assert!(!out.contains("<!-- THEME_SLOT -->"));
        assert!(!out.contains("data-theme"));
    }

    #[test]
    fn apply_page_with_theme_light_injects_attr() {
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("light"));
        assert!(!out.contains("<!-- THEME_SLOT -->"));
        assert!(
            out.contains(r#"dataset.theme="light""#),
            "light theme not injected: {out}"
        );
    }

    #[test]
    fn apply_page_with_theme_rejects_invalid_input() {
        // Themes containing characters outside [a-zA-Z0-9_-] are rejected entirely
        // rather than being stripped and concatenated, which would produce confusing output.
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("light<script>alert(1)</script>"));
        assert!(!out.contains("<script>alert"), "XSS not sanitized");
        assert!(
            !out.contains("lightscript"),
            "stripped-and-concatenated theme should not appear"
        );
        assert!(
            !out.contains("data-theme"),
            "rejected theme should not inject any attribute"
        );
    }

    #[test]
    fn apply_page_with_theme_rejects_oversized_input() {
        let long = "a".repeat(33);
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some(&long));
        assert!(
            !out.contains("data-theme"),
            "theme longer than 32 chars should be rejected"
        );
    }

    #[test]
    fn apply_page_with_theme_accepts_hyphen_and_underscore() {
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("my_custom-theme"));
        assert!(
            out.contains(r#"dataset.theme="my_custom-theme""#),
            "valid theme with - and _ rejected"
        );
    }

    #[test]
    fn apply_page_testing_injects_banner() {
        let out = apply_page(SAMPLE_HTML, true);
        assert!(out.contains("[TESTING]"), "testing banner missing");
    }

    #[test]
    fn apply_page_theme_and_testing_both_applied() {
        let out = apply_page_with_theme(SAMPLE_HTML, true, Some("light"));
        assert!(out.contains("[TESTING]"));
        assert!(out.contains(r#"dataset.theme="light""#));
    }

    // ── API integration tests ────────────────────────────────────────────────

    fn empty_web_state() -> WebState {
        let (tx, _rx) = tokio::sync::watch::channel(String::new());
        let live_slots: SharedSlots = Arc::new(Mutex::new(vec![]));
        WebState {
            tx,
            live_slots,
            db_conn: None,
            testing: true,
            allow_injections: false,
            connector: None,
            discord_slash: None,
            config_path: None,
        }
    }

    #[tokio::test]
    async fn api_state_empty_slots_returns_ok() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body[..], b"[]");
    }

    #[tokio::test]
    async fn api_slot_out_of_range_returns_404() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/slot/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serve_html_root_returns_ok() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn serve_about_returns_ok() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/about")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn api_runs_no_db_returns_error_json() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/runs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).expect("body must be JSON");
        assert!(v.get("error").is_some(), "expected error field when no DB");
    }

    #[tokio::test]
    async fn api_catch_rate_missing_params_returns_error_json() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/catch_rate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).expect("body must be JSON");
        assert!(v.get("error").is_some(), "expected error field for missing params");
    }
}
