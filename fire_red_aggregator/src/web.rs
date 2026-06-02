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

use crate::app::is_shiny;
use crate::client::{MonitorSlot, SharedSlots, SpriteCache, encode_png};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use fire_red_database::{CaughtPokemon, DeadPokemon};
use fire_red_states::{ClientMessage, GameState, MAX_NATIONAL_DEX_FIRERED};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

// ---------------------------------------------------------------------------
// Base64 encoding (no extra dependency needed)
// ---------------------------------------------------------------------------

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(n & 63) as usize] as char } else { '=' });
    }
    out
}

// ---------------------------------------------------------------------------
// JSON DTOs
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, Clone)]
struct RunSummaryDto {
    run_id:      u32,
    player_name: String,
    started_at:  String,
    ended_at:    Option<String>,
    deaths:      usize,
    caught:      usize,
}

#[derive(serde::Serialize, Clone)]
struct DbEncounterDto {
    species_name:   String,
    level:          u8,
    caught:         bool,
    encountered_at: String,
    /// Formatted map area string. Full name lookup is a future improvement;
    /// currently "G·N" where G=map_group and N=map_name.
    area:           String,
    sprite:         Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct SlotDto {
    label:          String,
    connected:      bool,
    db_connected:   bool,
    active_run_id:  Option<u32>,
    run_summary:    Option<RunSummaryDto>,
    db_encounters:  Vec<DbEncounterDto>,
    badges:         Vec<bool>,
    next_gym:       Option<GymDto>,
    party:          Vec<MemberDto>,
    encounters:     Vec<EncounterGroupDto>,
    dead:           Vec<DeadMonDto>,
    caught:         Vec<CaughtMonDto>,
    box_pokemon:    Vec<BoxMonDto>,
}

#[derive(serde::Serialize, Clone)]
struct DeadMonDto {
    nickname:      String,
    species_name:  String,
    level:         u8,
    nature:        String,
    shiny:         bool,
    soul_link:     bool,
    died_at:       String,
    gender:        u8,
    max_hp:        u16,
    attack:        u16,
    defense:       u16,
    speed:         u16,
    sp_attack:     u16,
    sp_defense:    u16,
    iv_hp:         u8,
    iv_atk:        u8,
    iv_def:        u8,
    iv_spe:        u8,
    iv_spa:        u8,
    iv_spd:        u8,
    ev_hp:         u8,
    ev_atk:        u8,
    ev_def:        u8,
    ev_spe:        u8,
    ev_spa:        u8,
    ev_spd:        u8,
    sprite:        Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct CaughtMonDto {
    nickname:      String,
    species_name:  String,
    level:         u8,
    nature:        String,
    shiny:         bool,
    caught_at:         String,
    met_location_name: String,
    gender:            u8,
    iv_hp:             u8,
    iv_atk:        u8,
    iv_def:        u8,
    iv_spe:        u8,
    iv_spa:        u8,
    iv_spd:        u8,
    sprite:        Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct BoxMonDto {
    box_index:    u8,
    slot_index:   u8,
    species_name: String,
    nickname:     String,
    is_shiny:     bool,
    nature:       String,
    is_egg:       bool,
    iv_hp:        u8,
    iv_atk:       u8,
    iv_def:       u8,
    iv_spe:       u8,
    iv_spa:       u8,
    iv_spd:       u8,
    /// `0` = male, `1` = female, `2` = genderless.
    gender:       u8,
    sprite:       Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct EncounterGroupDto {
    label: String,
    mons:  Vec<EncounterMonDto>,
}

#[derive(serde::Serialize, Clone)]
struct EncounterMonDto {
    min_level: u8,
    max_level: u8,
    sprite:    Option<String>,
}

#[derive(serde::Serialize, Clone)]
struct GymDto {
    leader:    String,
    city:      String,
    max_level: u8,
}

#[derive(serde::Serialize, Clone)]
struct SoulLinkPartnerDto {
    nickname: String,
    player:   String,
}

#[derive(serde::Serialize, Clone)]
struct MemberDto {
    nickname:          String,
    species_name:      String,
    level:             u8,
    hp:                u16,
    max_hp:            u16,
    exp:               u32,
    nature:            String,
    shiny:             bool,
    dead:              bool,
    soul_link_kill:    bool,
    soul_link_partner: Option<SoulLinkPartnerDto>,
    died_at:           Option<String>,
    attack:            u16,
    defense:           u16,
    speed:             u16,
    sp_attack:         u16,
    sp_defense:        u16,
    /// `0` = male, `1` = female, `2` = genderless.
    gender:            u8,
    /// Base64 PNG data URI for the sprite, e.g. `data:image/png;base64,...`.
    /// `None` while the sprite is still in transit from the tracker server.
    sprite:            Option<String>,
}

// ---------------------------------------------------------------------------
// DB + soul-link state (mirrors AggregatorApp in app.rs)
// ---------------------------------------------------------------------------

struct SlotCache {
    caught:       Vec<CaughtPokemon>,
    encounters:   Vec<fire_red_database::Encounter>,
    last_refresh: Instant,
}

impl SlotCache {
    fn new() -> Self {
        Self {
            caught:       Vec::new(),
            encounters:   Vec::new(),
            last_refresh: Instant::now() - Duration::from_secs(60),
        }
    }
}

struct BroadcastLoop {
    live_slots:           SharedSlots,
    caches:               Vec<SlotCache>,
    soul_link_propagated: HashSet<(usize, u32)>,
    last_json:            String,
    sprites:              SpriteCache,
}

impl BroadcastLoop {
    fn new(live_slots: SharedSlots, sprites: SpriteCache) -> Self {
        Self {
            live_slots,
            caches:               Vec::new(),
            soul_link_propagated: HashSet::new(),
            last_json:            String::new(),
            sprites,
        }
    }

    /// Requests sprites for party members and encounter pokemon not yet cached.
    fn request_sprites(&self, slots: &[Arc<MonitorSlot>], states: &[(String, Option<GameState>)]) {
        let cache = self.sprites.lock().unwrap_or_else(|e| e.into_inner());
        for (i, slot) in slots.iter().enumerate() {
            let Some(gs) = &states[i].1 else { continue };
            let mut known = slot.known_species.lock().unwrap_or_else(|e| e.into_inner());
            let mut needed: Vec<u16> = Vec::new();

            // Party sprites (normal + shiny variant if shiny)
            for p in &gs.party {
                let s = p.box_mon.secure.growth.species;
                let shiny = is_shiny(p.box_mon.personality, p.box_mon.ot_id);
                if s == 0 || s > MAX_NATIONAL_DEX_FIRERED { continue; }
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
            let all_enc = enc.land_mon_encounters.wild_pokemon_list.iter()
                .chain(enc.water_mon_encounters.wild_pokemon_list.iter())
                .chain(enc.rock_smash_encounters.wild_pokemon_list.iter())
                .chain(enc.fishing_encounters.wild_pokemon_list.iter());
            for w in all_enc {
                let s = w.species;
                if s == 0 || s > MAX_NATIONAL_DEX_FIRERED { continue; }
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
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_back(needed);
            }
        }
    }

    /// Drains any pending textures from the TCP pipeline into the sprite cache.
    /// Also wires the shared sprite cache into any slot that connected after
    /// `run()` started (identified by having `sprite_cache = None`).
    fn drain_sprites(&mut self, slots: &[Arc<MonitorSlot>]) {
        for slot in slots {
            let mut sc = slot.sprite_cache.lock().unwrap_or_else(|e| e.into_inner());
            if sc.is_none() { *sc = Some(self.sprites.clone()); }
            drop(sc);

            let mut pending = slot.pending_textures.lock().unwrap_or_else(|e| e.into_inner());
            if pending.is_empty() { continue; }
            let drained: Vec<_> = pending.drain(..).collect();
            drop(pending);
            let mut cache = self.sprites.lock().unwrap_or_else(|e| e.into_inner());
            for pt in drained {
                let key = (pt.species, pt.shiny);
                if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(key)
                    && let Some(png) = encode_png(&pt.pixels, pt.width, pt.height) {
                    e.insert(png);
                }
            }
        }
    }

    /// Returns a `data:image/png;base64,...` URI for the given species/shiny
    /// if the sprite has been received and encoded, or `None` otherwise.
    fn sprite_uri(&self, species: u16, shiny: bool) -> Option<String> {
        let cache = self.sprites.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(&(species, shiny)).map(|png| {
            format!("data:image/png;base64,{}", base64_encode(png))
        })
    }

    /// Runs one tick: refreshes DB caches, propagates soul-link deaths, and
    /// returns a JSON string if the state has changed since the last tick.
    fn tick(&mut self) -> Option<String> {
        let slots: Vec<Arc<MonitorSlot>> =
            self.live_slots.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let n = slots.len();
        while self.caches.len() < n { self.caches.push(SlotCache::new()); }

        // Collect live states
        let states: Vec<(String, Option<GameState>)> = slots
            .iter()
            .map(|s| {
                let state = s.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let label = s.label.lock().unwrap_or_else(|e| e.into_inner()).clone();
                (label, state)
            })
            .collect();

        // Sprite pipeline
        self.request_sprites(&slots, &states);
        self.drain_sprites(&slots);

        // If the tracker confirmed a run change, mark the DB reader dirty so
        // sync_player re-queries even though the player name hasn't changed.
        for slot in &slots {
            if slot.run_changed.swap(false, std::sync::atomic::Ordering::AcqRel)
                && let Some(db) = &slot.db {
                db.mark_dirty();
            }
        }

        // Sync DB run IDs
        let mut run_id_changed = vec![false; n];
        for (i, slot) in slots.iter().enumerate() {
            if let Some(db) = &slot.db {
                run_id_changed[i] = db.sync_player(&states[i].0);
            }
        }

        // Refresh caught cache
        let now = Instant::now();
        for i in 0..n {
            let stale = now.duration_since(self.caches[i].last_refresh) >= Duration::from_secs(1);
            if (run_id_changed[i] || stale)
                && let Some(db) = &slots[i].db {
                let label = &states[i].0;
                self.caches[i].caught      = db.list_caught(label);
                self.caches[i].encounters  = db.list_encounters(label);
                self.caches[i].last_refresh = now;
            }
        }

        // Dead records (fresh every tick), filtered per player by name.
        let all_dead: Vec<HashMap<u32, DeadPokemon>> = (0..n)
            .map(|i| {
                slots[i].db.as_ref()
                    .map(|db| db.list_dead_with_records(&states[i].0))
                    .unwrap_or_default()
            })
            .collect();

        // Snapshot box data per slot for use in sprite requests and DTO building.
        let all_box: Vec<Vec<fire_red_states::BoxEntry>> = slots
            .iter()
            .map(|s| s.box_data.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .collect();

        // Request sprites for dead, caught, and box pokemon not yet in the cache
        {
            let cache = self.sprites.lock().unwrap_or_else(|e| e.into_inner());
            for (i, slot) in slots.iter().enumerate() {
                let mut known = slot.known_species.lock().unwrap_or_else(|e| e.into_inner());
                let mut needed: Vec<u16> = Vec::new();
                for dp in all_dead[i].values() {
                    let s = dp.species;
                    if s > 0 && s <= MAX_NATIONAL_DEX_FIRERED && !known.contains(&s) && !cache.contains_key(&(s, dp.is_shiny)) {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                for cp in &self.caches[i].caught {
                    let s = cp.species;
                    if s > 0 && s <= MAX_NATIONAL_DEX_FIRERED && !known.contains(&s) && !cache.contains_key(&(s, cp.is_shiny)) {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                for enc in &self.caches[i].encounters {
                    let s = enc.species;
                    if s > 0 && s <= MAX_NATIONAL_DEX_FIRERED && !known.contains(&s) && !cache.contains_key(&(s, false)) {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                for be in &all_box[i] {
                    let s = be.species;
                    if s > 0 && s <= MAX_NATIONAL_DEX_FIRERED && !known.contains(&s) && !cache.contains_key(&(s, be.is_shiny)) {
                        needed.push(s);
                        known.insert(s);
                    }
                }
                drop(known);
                if !needed.is_empty() {
                    needed.sort();
                    needed.dedup();
                    slot.texture_request_queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_back(needed);
                }
            }
        }

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
                if met_loc == 0 { continue; }
                for j in 0..n {
                    if j == i { continue; }
                    let partner = self.caches[j]
                        .caught
                        .iter()
                        .find(|c| c.met_location == met_loc && c.personality != dead_p)
                        .cloned();
                    if let Some(p) = partner {
                        let key = (j, p.personality);
                        let already_dead       = all_dead[j].contains_key(&p.personality);
                        let already_propagated = self.soul_link_propagated.contains(&key);
                        if !already_dead && !already_propagated {
                            let wrote = slots[j]
                                .db
                                .as_ref()
                                .map(|db| db.mark_soul_link_dead(&p))
                                .unwrap_or(false);
                            if wrote {
                                self.soul_link_propagated.insert(key);
                            }
                        }
                    }
                }
            }
        }

        // Live soul-link dead detection
        let mut live_soul_link_dead: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        for i in 0..n {
            let Some(gs_i) = &states[i].1 else { continue };
            for p_i in &gs_i.party {
                if p_i.hp != 0 { continue; }
                let met_i = p_i.box_mon.secure.misc.met_location;
                if met_i == 0 { continue; }
                for j in 0..n {
                    if j == i { continue; }
                    let Some(gs_j) = &states[j].1 else { continue };
                    for p_j in &gs_j.party {
                        if p_j.box_mon.secure.misc.met_location == met_i {
                            live_soul_link_dead[j].insert(p_j.box_mon.personality);
                        }
                    }
                }
            }
        }

        // Build JSON payload
        let slots_dto: Vec<SlotDto> = (0..n)
            .map(|i| {
                let (label, state) = &states[i];
                let dead_records   = &all_dead[i];
                let soul_link_dead = &live_soul_link_dead[i];
                let db_connected   = slots[i].db.is_some();
                let active_run_id  = slots[i].db.as_ref().and_then(|db| db.active_run_id());

                let run_summary = slots[i].db.as_ref().and_then(|db| db.run_summary())
                    .map(|(run_id, player_name, started_at, ended_at, deaths, caught)| RunSummaryDto {
                        run_id,
                        player_name,
                        started_at:  fire_red_database::format_timestamp(started_at),
                        ended_at:    ended_at.map(fire_red_database::format_timestamp),
                        deaths,
                        caught,
                    });

                let db_encounters: Vec<DbEncounterDto> = self.caches[i].encounters.iter()
                    .map(|enc| DbEncounterDto {
                        species_name:   enc.species_name.clone(),
                        level:          enc.level,
                        caught:         enc.caught,
                        encountered_at: fire_red_database::format_timestamp(enc.encountered_at),
                        area: {
                            let n = fire_red_location_names::map_area_name(enc.map_group, enc.map_name);
                            if n.is_empty() {
                                format!("{}\u{B7}{}", enc.map_group, enc.map_name)
                            } else {
                                n.to_string()
                            }
                        },
                        sprite:         self.sprite_uri(enc.species, false),
                    })
                    .collect();

                let (connected, badges, next_gym, party, encounters) = match state {
                    None => (false, vec![false; 8], None, vec![], vec![]),
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
                                leader:    g.leader.clone(),
                                city:      g.city.clone(),
                                max_level: g.max_level,
                            });

                        let party = gs
                            .party
                            .iter()
                            .map(|p| {
                                let personality    = p.box_mon.personality;
                                let ot_id          = p.box_mon.ot_id;
                                let shiny          = is_shiny(personality, ot_id);
                                let met            = p.box_mon.secure.misc.met_location;
                                let species        = p.box_mon.secure.growth.species;
                                let is_soul_link   = soul_link_dead.contains(&personality);
                                let dead_record    = dead_records.get(&personality);
                                let dead           = dead_record.is_some() || p.hp == 0 || is_soul_link;

                                // Soul-link partner annotation
                                let soul_link_partner = if met == 0 {
                                    None
                                } else {
                                    let mut found = None;
                                    'outer: for (j, (player_j, state_j)) in states.iter().enumerate().take(n) {
                                        if j == i { continue; }
                                        if let Some(gs_j) = state_j {
                                            for p_j in &gs_j.party {
                                                if p_j.box_mon.secure.misc.met_location == met {
                                                    found = Some(SoulLinkPartnerDto {
                                                        nickname: p_j.get_nickname_string(),
                                                        player:   player_j.clone(),
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
                                            r.max_hp == 0,
                                            r.attack, r.defense, r.speed, r.sp_attack, r.sp_defense,
                                        )
                                    } else {
                                        (
                                            None, false,
                                            p.attack, p.defense, p.speed, p.sp_attack, p.sp_defense,
                                        )
                                    };

                                // Embed sprite as data URI — avoids any HTTP / caching issues
                                let sprite = self.sprite_uri(species, shiny);

                                MemberDto {
                                    nickname:          p.get_nickname_string(),
                                    species_name:      p.box_mon.secure.growth.species_string.clone(),
                                    level:             p.level,
                                    hp:                p.hp,
                                    max_hp:            p.max_hp,
                                    exp:               p.box_mon.secure.growth.experience,
                                    nature:            fire_red_database::nature_name(personality).to_string(),
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
                                    gender:            p.box_mon.gender,
                                    sprite,
                                }
                            })
                            .collect();

                        // Build encounter groups (skip empty ones)
                        let enc = &gs.encounters;
                        let mut encounters: Vec<EncounterGroupDto> = Vec::new();

                        let land: Vec<EncounterMonDto> = enc.land_mon_encounters
                            .wild_pokemon_list.iter()
                            .filter(|w| w.species > 0 && w.species <= MAX_NATIONAL_DEX_FIRERED)
                            .map(|w| EncounterMonDto {
                                min_level: w.min_level,
                                max_level: w.max_level,
                                sprite:    self.sprite_uri(w.species, false),
                            })
                            .collect();
                        if !land.is_empty() {
                            encounters.push(EncounterGroupDto { label: "Land".into(), mons: land });
                        }

                        let water_fish: Vec<EncounterMonDto> = enc.water_mon_encounters
                            .wild_pokemon_list.iter()
                            .chain(enc.fishing_encounters.wild_pokemon_list.iter())
                            .filter(|w| w.species > 0 && w.species <= MAX_NATIONAL_DEX_FIRERED)
                            .map(|w| EncounterMonDto {
                                min_level: w.min_level,
                                max_level: w.max_level,
                                sprite:    self.sprite_uri(w.species, false),
                            })
                            .collect();
                        if !water_fish.is_empty() {
                            encounters.push(EncounterGroupDto { label: "Water / Fishing".into(), mons: water_fish });
                        }

                        let rock: Vec<EncounterMonDto> = enc.rock_smash_encounters
                            .wild_pokemon_list.iter()
                            .filter(|w| w.species > 0 && w.species <= MAX_NATIONAL_DEX_FIRERED)
                            .map(|w| EncounterMonDto {
                                min_level: w.min_level,
                                max_level: w.max_level,
                                sprite:    self.sprite_uri(w.species, false),
                            })
                            .collect();
                        if !rock.is_empty() {
                            encounters.push(EncounterGroupDto { label: "Rock Smash".into(), mons: rock });
                        }

                        (true, badges, next_gym, party, encounters)
                    }
                };

                // dead_records and caches are already filtered by player_name in
                // list_dead_with_records / list_caught, so no further filtering needed.
                let mut dead_sorted: Vec<&DeadPokemon> = dead_records.values().collect();
                dead_sorted.sort_by_key(|b| std::cmp::Reverse(b.died_at));
                let dead: Vec<DeadMonDto> = dead_sorted.iter().map(|dp| DeadMonDto {
                    nickname:     dp.nickname.clone(),
                    species_name: dp.species_name.clone(),
                    level:        dp.level,
                    nature:       dp.nature.clone(),
                    shiny:        dp.is_shiny,
                    soul_link:    dp.max_hp == 0,
                    gender:       dp.gender,
                    died_at:      fire_red_database::format_timestamp(dp.died_at),
                    max_hp:       dp.max_hp,
                    attack:       dp.attack,
                    defense:      dp.defense,
                    speed:        dp.speed,
                    sp_attack:    dp.sp_attack,
                    sp_defense:   dp.sp_defense,
                    iv_hp:        dp.ivs.hp,
                    iv_atk:       dp.ivs.attack,
                    iv_def:       dp.ivs.defense,
                    iv_spe:       dp.ivs.speed,
                    iv_spa:       dp.ivs.sp_attack,
                    iv_spd:       dp.ivs.sp_defense,
                    ev_hp:        dp.evs.hp,
                    ev_atk:       dp.evs.attack,
                    ev_def:       dp.evs.defense,
                    ev_spe:       dp.evs.speed,
                    ev_spa:       dp.evs.sp_attack,
                    ev_spd:       dp.evs.sp_defense,
                    sprite:       self.sprite_uri(dp.species, dp.is_shiny),
                }).collect();

                let caught: Vec<CaughtMonDto> = self.caches[i].caught.iter()
                    .rev()
                    .map(|cp| CaughtMonDto {
                    nickname:     cp.nickname.clone(),
                    species_name: cp.species_name.clone(),
                    level:        cp.level,
                    nature:       cp.nature.clone(),
                    shiny:        cp.is_shiny,
                    caught_at:         fire_red_database::format_timestamp(cp.caught_at),
                    met_location_name: fire_red_location_names::location_name(cp.met_location).to_string(),
                    gender:            cp.gender,
                    iv_hp:        cp.ivs.hp,
                    iv_atk:       cp.ivs.attack,
                    iv_def:       cp.ivs.defense,
                    iv_spe:       cp.ivs.speed,
                    iv_spa:       cp.ivs.sp_attack,
                    iv_spd:       cp.ivs.sp_defense,
                    sprite:       self.sprite_uri(cp.species, cp.is_shiny),
                }).collect();

                let box_pokemon: Vec<BoxMonDto> = all_box[i].iter()
                    .map(|be| BoxMonDto {
                        box_index:    be.box_index,
                        slot_index:   be.slot_index,
                        species_name: be.species_name.clone(),
                        nickname:     be.nickname.clone(),
                        is_shiny:     be.is_shiny,
                        nature:       be.nature.clone(),
                        is_egg:       be.is_egg,
                        gender:       be.gender,
                        iv_hp:        be.iv_hp,
                        iv_atk:       be.iv_atk,
                        iv_def:       be.iv_def,
                        iv_spe:       be.iv_spe,
                        iv_spa:       be.iv_spa,
                        iv_spd:       be.iv_spd,
                        sprite:       self.sprite_uri(be.species, be.is_shiny),
                    })
                    .collect();

                SlotDto { label: label.clone(), connected, db_connected, active_run_id, run_summary, db_encounters, badges, next_gym, party, encounters, dead, caught, box_pokemon }
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
    tx:         watch::Sender<String>,
    live_slots: SharedSlots,
    db_conn:    Option<String>,
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

const OVERLAY_HTML:  &str = include_str!("overlay.html");
const FOCUSED_HTML:  &str = include_str!("focused.html");
const DBVIEWER_HTML: &str = include_str!("db.html");
const HISTORY_HTML:  &str = include_str!("history.html");

async fn serve_html() -> Html<&'static str> {
    Html(OVERLAY_HTML)
}

async fn serve_focused() -> Html<&'static str> {
    Html(FOCUSED_HTML)
}

async fn serve_db_viewer() -> Html<&'static str> {
    Html(DBVIEWER_HTML)
}

async fn serve_history() -> Html<&'static str> {
    Html(HISTORY_HTML)
}

async fn serve_db_json(State(state): State<WebState>) -> axum::Json<serde_json::Value> {
    let conn = match state.db_conn {
        Some(s) => s,
        None    => return axum::Json(serde_json::json!({ "error": "No database configured" })),
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::dump_all(&conn)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Query failed" })))
}

async fn clear_db(State(state): State<WebState>) -> impl IntoResponse {
    let conn = match state.db_conn {
        Some(s) => s,
        None    => return (StatusCode::SERVICE_UNAVAILABLE, "No database configured".to_string()),
    };
    match tokio::task::spawn_blocking(move || fire_red_database::clear_all_records(&conn)).await {
        Ok(Ok(())) => (StatusCode::OK, "ok".to_string()),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(_)     => (StatusCode::INTERNAL_SERVER_ERROR, "Task panicked".to_string()),
    }
}

/// Returns the full current state as a JSON array of slot objects — same
/// payload the WebSocket would push on the next tick.
async fn api_state(State(state): State<WebState>) -> impl IntoResponse {
    let json = state.tx.borrow().clone();
    let body = if json.is_empty() { "[]".to_string() } else { json };
    ([(header::CONTENT_TYPE, "application/json")], body)
}

/// Returns a single slot object by zero-based index, or 404 if out of range.
async fn api_slot(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    let json = state.tx.borrow().clone();
    let slots: serde_json::Value = serde_json::from_str(&json)
        .unwrap_or(serde_json::Value::Array(vec![]));
    match slots.get(index) {
        Some(slot) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            slot.to_string(),
        ).into_response(),
        None => (StatusCode::NOT_FOUND, "slot index out of range").into_response(),
    }
}

async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<WebState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.tx.subscribe(), state.live_slots))
}

async fn handle_socket(
    socket: axum::extract::ws::WebSocket,
    mut rx: watch::Receiver<String>,
    live_slots: SharedSlots,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Send current state immediately so the browser isn't blank on connect.
    {
        let current = rx.borrow_and_update().clone();
        if !current.is_empty()
            && ws_tx
                .send(axum::extract::ws::Message::Text(current))
                .await
                .is_err()
        {
            return;
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
                    let slots = live_slots.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    for slot in &slots {
                        slot.command_queue
                            .lock().unwrap_or_else(|e| e.into_inner())
                            .push_back(msg.clone());
                    }
                }
            }
        }
    });

    // Push state updates whenever the broadcast channel changes.
    loop {
        if rx.changed().await.is_err() { break; }
        let msg = rx.borrow_and_update().clone();
        if ws_tx.send(axum::extract::ws::Message::Text(msg)).await.is_err() {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(live_slots: SharedSlots, port: u16, db_conn: Option<String>) {
    let sprites: SpriteCache = Arc::new(Mutex::new(HashMap::new()));

    // Wire the shared sprite cache into any already-connected slots and keep
    // it available for slots that connect later (BroadcastLoop sets it on drain).
    {
        let slots = live_slots.lock().unwrap_or_else(|e| e.into_inner());
        for slot in slots.iter() {
            *slot.sprite_cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(sprites.clone());
        }
    }

    let (tx, _rx) = watch::channel::<String>(String::new());
    let tx_bg        = tx.clone();
    let sprites_loop = sprites.clone();
    let loop_slots   = live_slots.clone();

    std::thread::spawn(move || {
        let mut bloop = BroadcastLoop::new(loop_slots, sprites_loop);
        loop {
            if let Some(json) = bloop.tick() {
                let _ = tx_bg.send(json);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    let web_state = WebState { tx, live_slots, db_conn };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let app = Router::new()
            .route("/", get(serve_html))
            .route("/ws", get(ws_handler))
            .route("/db", get(serve_db_viewer))
            .route("/db.json", get(serve_db_json))
            .route("/db/clear", post(clear_db))
            .route("/api/state", get(api_state))
            .route("/api/slot/:index", get(api_slot))
            .route("/history", get(serve_history))
            .route("/:index/party", get(serve_focused))
            .route("/:index/encounters", get(serve_focused))
            .route("/:index/dead", get(serve_focused))
            .route("/:index/caught", get(serve_focused))
            .route("/:index/box", get(serve_focused))
            .with_state(web_state);

        let addr = format!("0.0.0.0:{}", port);
        println!("WebSocket overlay listening on http://{}", addr);
        println!("Add in OBS as Browser Source: http://localhost:{}", port);

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("failed to bind WebSocket port");
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("WebSocket server error: {e}");
        }
    });
}
