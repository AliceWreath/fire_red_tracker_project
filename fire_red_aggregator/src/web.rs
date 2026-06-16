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
    Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse},
    routing::{delete, get, patch, post},
};
use fire_red_database::{CaughtPokemon, DeadPokemon};
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

            if game_cleared {
                if let (Some(rid), Some(conn), Some(dir)) =
                    (run_id, self.db_conn.as_ref(), self.backup_dir.as_ref())
                {
                    if self.backup_done.insert(rid) {
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
async fn api_state(State(state): State<WebState>) -> impl IntoResponse {
    let json = state.tx.borrow().clone();
    let body = if json.is_empty() {
        "[]".to_string()
    } else {
        json
    };
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
async fn api_bot_summary(State(state): State<WebState>, Path(index): Path<usize>) -> String {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return format!("Slot {index} not found"),
    };
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
) -> impl IntoResponse {
    let show = params.get("show").cloned();
    ws.on_upgrade(move |socket| handle_socket(socket, state.tx.subscribe(), state.live_slots, show))
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
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Send current state immediately so the browser isn't blank on connect.
    {
        let current = rx.borrow_and_update().clone();
        if !current.is_empty() {
            let msg = match &show {
                Some(s) => filter_slots_json(&current, s),
                None => current,
            };
            if ws_tx
                .send(axum::extract::ws::Message::Text(msg))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // Spawn a task to forward incoming browser messages as tracker commands.
    // end_run and new_run are broadcast to every connected slot so all trackers
    // stay in sync.
    tokio::spawn(async move {
        while let Some(Ok(axum::extract::ws::Message::Text(text))) = ws_rx.next().await {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                let msg = match val["cmd"].as_str().unwrap_or("") {
                    "end_run" => Some(ClientMessage::EndRun),
                    "new_run" => Some(ClientMessage::NewRun),
                    _ => None,
                };
                if let Some(msg) = msg {
                    let slots = live_slots.lock_or_recover().clone();
                    for slot in &slots {
                        slot.command_queue.lock_or_recover().push_back(msg.clone());
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
        let msg = match &show {
            Some(s) => filter_slots_json(&raw, s),
            None => raw,
        };
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
async fn api_active_timeline(State(state): State<WebState>) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({ "error": "No database configured" })),
        )
            .into_response();
    };
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::active_run_timeline_json(&conn))
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

/// `GET /api/runs` — summary list of all runs (id, player, dates, deaths, catches, encounters).
async fn api_runs(State(state): State<WebState>) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::list_all_runs_json(&conn)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `POST /api/run/import` — import a run from the JSON format produced by `/api/run/:id/export`.
///
/// Creates a new run with a fresh id and re-inserts caught, dead, and encounter records.
/// Returns `{ "run_id": <new_id> }` on success.
async fn api_run_import(
    State(state): State<WebState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result =
        tokio::task::spawn_blocking(move || fire_red_database::import_run(&conn, &body)).await;
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
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let ids_str = match params.get("ids") {
        Some(s) => s.clone(),
        None => return axum::Json(serde_json::json!({ "error": "Missing 'ids' query parameter" })),
    };
    let run_ids: Vec<u32> = ids_str
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .take(20)
        .collect();
    if run_ids.is_empty() {
        return axum::Json(serde_json::json!({ "error": "No valid run IDs provided" }));
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

/// Broadcasts a command to all connected tracker slots.
///
/// Supported commands (no request body needed — suitable for Stream Deck buttons):
///
/// | `cmd`       | Effect                                                   |
/// |-------------|----------------------------------------------------------|
/// | `end_run`   | End the active run for every connected player.           |
/// | `new_run`   | Start a new run for every connected player.              |
/// | `heal_all`  | Heal HP/PP/status of every party Pokémon for all slots.  |
async fn api_command(State(state): State<WebState>, Path(cmd): Path<String>) -> impl IntoResponse {
    let msg = match cmd.as_str() {
        "end_run"  => ClientMessage::EndRun,
        "new_run"  => ClientMessage::NewRun,
        "heal_all" => ClientMessage::HealParty,
        other => return (StatusCode::BAD_REQUEST, format!("Unknown command: {other}")),
    };
    let slots = state.live_slots.lock_or_recover().clone();
    let count = slots.len();
    for slot in &slots {
        slot.command_queue.lock_or_recover().push_back(msg.clone());
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
    Path(goal_id): Path<i32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
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
    Path(goal_id): Path<i32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
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
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    if !state.allow_injections {
        return axum::Json(serde_json::json!({ "error": "injection commands are disabled" }));
    }
    let items = match body.as_array() {
        Some(a) => a,
        None => return axum::Json(serde_json::json!({ "error": "body must be a JSON array" })),
    };

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
// Router construction (extracted for testability)
// ---------------------------------------------------------------------------

fn build_router(web_state: WebState) -> Router {
    Router::new()
        .route("/", get(serve_html))
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
        .route("/api/direct/connect", post(api_direct_connect))
        .route("/api/direct/hosts", get(api_direct_hosts))
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
        .with_state(web_state)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(
    live_slots: SharedSlots,
    port: u16,
    db_conn: Option<String>,
    testing: bool,
    allow_injections: bool,
    connector: Option<Arc<crate::direct::DirectConnector>>,
    backup_dir: Option<String>,
    livesplit_split_on_badges: bool,
) {
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
// Direct-mode join page
// ---------------------------------------------------------------------------

const JOIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Connect – Fire Red Tracker</title>
<style>
  *{box-sizing:border-box;margin:0;padding:0}
  body{font-family:sans-serif;background:#1a1a2e;color:#eee;display:flex;justify-content:center;align-items:center;min-height:100vh}
  .card{background:#16213e;border:1px solid #0f3460;border-radius:8px;padding:2rem;width:360px}
  h1{font-size:1.4rem;color:#e94560;margin-bottom:.75rem}
  p{color:#aaa;font-size:.9rem;line-height:1.5;margin-bottom:1.2rem}
  label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
  input{width:100%;padding:.5rem .7rem;background:#0f3460;border:1px solid #444;border-radius:4px;color:#eee;font-size:1rem;margin-bottom:1rem}
  input:focus{outline:none;border-color:#e94560}
  button{width:100%;padding:.6rem;background:#e94560;border:none;border-radius:4px;color:#fff;font-size:1rem;cursor:pointer}
  button:hover{background:#c73652}
  button:disabled{background:#555;cursor:default}
  .msg{margin-top:1rem;padding:.6rem;border-radius:4px;text-align:center;font-size:.9rem;display:none}
  .ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
  .err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
  .active-hosts{margin-top:1.2rem;font-size:.8rem;color:#888}
  .active-hosts ul{margin-top:.4rem;padding-left:1.2rem;color:#aaa}
</style>
</head>
<body>
<div class="card">
  <h1>Connect to Tracker</h1>
  <p>Enter the IP address of the machine where RetroArch is running and make sure <strong>Network Commands</strong> are enabled in RetroArch settings.</p>
  <form id="f">
    <label for="host">RetroArch IP address</label>
    <input id="host" name="host" type="text" placeholder="192.168.1.x" required>
    <label for="port">Network Commands port</label>
    <input id="port" name="port" type="number" value="DEFAULT_PORT" min="1" max="65535" required>
    <button id="btn" type="submit">Connect</button>
  </form>
  <div id="msg" class="msg"></div>
  <div class="active-hosts" id="active" style="display:none">
    <strong>Currently connected hosts:</strong>
    <ul id="host-list"></ul>
  </div>
</div>
<script>
(async function(){
  try{
    const r=await fetch('/api/direct/hosts');
    if(r.ok){
      const d=await r.json();
      if(d.hosts&&d.hosts.length>0){
        const el=document.getElementById('active');
        const ul=document.getElementById('host-list');
        d.hosts.forEach(h=>{const li=document.createElement('li');li.textContent=h;ul.appendChild(li);});
        el.style.display='';
      }
    }
  }catch(e){}
})();

document.getElementById('f').onsubmit=async function(e){
  e.preventDefault();
  const host=document.getElementById('host').value.trim();
  const port=parseInt(document.getElementById('port').value,10);
  const msg=document.getElementById('msg');
  const btn=document.getElementById('btn');
  msg.className='msg';
  msg.textContent='Connecting…';
  btn.disabled=true;
  try{
    const r=await fetch('/api/direct/connect',{
      method:'POST',
      headers:{'Content-Type':'application/json'},
      body:JSON.stringify({host,port})
    });
    const d=await r.json();
    if(r.ok){
      msg.className='msg ok';
      msg.textContent=d.message||'Connection request sent. Your slot will appear shortly.';
    }else{
      msg.className='msg err';
      msg.textContent=d.error||'Connection failed.';
    }
  }catch(err){
    msg.className='msg err';
    msg.textContent='Request failed: '+err.message;
  }
  btn.disabled=false;
};
</script>
</body>
</html>"#;

async fn serve_join(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<WebState>,
) -> impl IntoResponse {
    if state.connector.is_none() {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<h1>Direct mode is not active.</h1>".to_string(),
        );
    }
    let default_port = state.connector.as_ref().map(|c| c.default_port).unwrap_or(55355);
    let client_ip = addr.ip().to_string();
    let html = JOIN_HTML
        .replace("DEFAULT_PORT", &default_port.to_string())
        .replace("192.168.1.x", &client_ip);
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
}

async fn api_direct_connect(
    State(state): State<WebState>,
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

    let port = body.port.unwrap_or(connector.default_port);
    let accepted = connector.connect(host.clone(), port);

    if accepted {
        tracing::info!("Direct mode: /join accepted {}:{}", host, port);
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

async fn api_direct_hosts(State(state): State<WebState>) -> impl IntoResponse {
    let hosts = state
        .connector
        .as_ref()
        .map(|c| c.active_hosts())
        .unwrap_or_default();
    axum::Json(serde_json::json!({"hosts": hosts}))
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
