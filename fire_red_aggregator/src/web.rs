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
use crate::client::{MonitorSlot, SharedSlots, PngSpriteCache, encode_png};
use fire_red_states::{is_shiny, ClientMessage, GameState, LockOrRecover, MAX_NATIONAL_DEX_FIRERED};
use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use fire_red_database::{CaughtPokemon, DeadPokemon};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet};
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
    is_shiny:       bool,
    encountered_at: String,
    area:           String,
    sprite:         Option<String>,
    map_group:      u8,
    map_name:       u8,
}

#[derive(serde::Serialize, Clone)]
struct SlotDto {
    label:               String,
    connected:           bool,
    db_connected:        bool,
    active_run_id:       Option<u32>,
    run_summary:         Option<RunSummaryDto>,
    db_encounters:       Vec<DbEncounterDto>,
    badges:              Vec<bool>,
    next_gym:            Option<GymDto>,
    party:               Vec<MemberDto>,
    encounters:          Vec<EncounterGroupDto>,
    dead:                Vec<DeadMonDto>,
    caught:              Vec<CaughtMonDto>,
    box_pokemon:         Vec<BoxMonDto>,
    /// map_group of the current wild-encounter zone (0 if no encounter area).
    current_map_group:   u8,
    /// map_name of the current wild-encounter zone (0 if no encounter area).
    current_map_name:    u8,
    /// Human-readable name for the current zone, empty when not in a wild area.
    current_zone_name:   String,
    /// Encounters from the most recently completed run, for cross-run hints.
    prev_run_encounters: Vec<DbEncounterDto>,
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
    ability:           String,
    held_item:         String,
    held_item_id:      u16,
    growth_rate:       String,
    ev_hp:             u8,
    ev_atk:            u8,
    ev_def:            u8,
    ev_spe:            u8,
    ev_spa:            u8,
    ev_spd:            u8,
    iv_hp:             u8,
    iv_atk:            u8,
    iv_def:            u8,
    iv_spe:            u8,
    iv_spa:            u8,
    iv_spd:            u8,
    /// Base64 PNG data URI for the sprite, e.g. `data:image/png;base64,...`.
    /// `None` while the sprite is still in transit from the tracker server.
    sprite:            Option<String>,
    /// Unique personality value — used by the overlay to detect death transitions.
    personality:       u32,
    /// Status condition bitmask (Gen III encoding):
    /// bits 0-2 = sleep turns, bit 3 = PSN, bit 4 = BRN, bit 5 = FRZ, bit 6 = PAR, bit 7 = TOX.
    status:            u32,
    /// Current move names (empty string for empty slots).
    moves:             [String; 4],
    /// Current PP for each move slot.
    pp:                [u8; 4],
}

// ---------------------------------------------------------------------------
// DB + soul-link state (mirrors AggregatorApp in app.rs)
// ---------------------------------------------------------------------------

struct SlotCache {
    caught:           Vec<CaughtPokemon>,
    encounters:       Vec<fire_red_database::Encounter>,
    prev_encounters:  Vec<fire_red_database::Encounter>,
    last_refresh:     Instant,
}

impl SlotCache {
    fn new() -> Self {
        Self {
            caught:          Vec::new(),
            encounters:      Vec::new(),
            prev_encounters: Vec::new(),
            last_refresh:    Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
        }
    }
}

struct BroadcastLoop {
    live_slots:           SharedSlots,
    caches:               Vec<SlotCache>,
    soul_link_propagated: HashSet<(usize, u32)>,
    last_json:            String,
    sprites:              PngSpriteCache,
}

impl BroadcastLoop {
    fn new(live_slots: SharedSlots, sprites: PngSpriteCache) -> Self {
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
        let cache = self.sprites.lock_or_recover();
        for (i, slot) in slots.iter().enumerate() {
            let Some(gs) = &states[i].1 else { continue };
            let mut known = slot.known_species.lock_or_recover();
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
            if sc.is_none() { *sc = Some(self.sprites.clone()); }
            drop(sc);

            let mut pending = slot.pending_textures.lock_or_recover();
            if pending.is_empty() { continue; }
            let drained: Vec<_> = pending.drain(..).collect();
            drop(pending);
            let mut cache = self.sprites.lock_or_recover();
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
        let cache = self.sprites.lock_or_recover();
        cache.get(&(species, shiny)).map(|png| {
            format!("data:image/png;base64,{}", base64_encode(png))
        })
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
        let sorted_gifts: Vec<Vec<&CaughtPokemon>> = self.caches.iter()
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
                if met_loc == 0 && gift_idx.is_none() { continue; }

                for j in 0..n {
                    if j == i { continue; }
                    let partner = if met_loc == 0 {
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
                        let already_dead       = all_dead[j].contains_key(&p.personality);
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
                if p_i.hp != 0 { continue; }
                let met_i = p_i.box_mon.secure.misc.met_location;
                for j in 0..n {
                    if j == i { continue; }
                    if met_i == 0 {
                        // Gift Pokémon: pair by receipt order — matches DB path.
                        let Some(idx) = sorted_gifts[i].iter()
                            .position(|c| c.personality == p_i.box_mon.personality)
                            else { continue };
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
        gs.party.iter().map(|p| {
            let personality    = p.box_mon.personality;
            let ot_id          = p.box_mon.ot_id;
            let shiny          = is_shiny(personality, ot_id);
            let met            = p.box_mon.secure.misc.met_location;
            let species        = p.box_mon.secure.growth.species;
            let is_soul_link   = soul_link_dead.contains(&personality);
            let dead_record    = dead_records.get(&personality);
            let dead           = dead_record.is_some() || p.hp == 0 || is_soul_link;

            let soul_link_partner = if met == 0 {
                None
            } else {
                let mut found = None;
                'outer: for (j, (player_j, state_j)) in states.iter().enumerate().take(n) {
                    if j == slot_idx { continue; }
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
                        r.is_soul_link_death,
                        r.attack, r.defense, r.speed, r.sp_attack, r.sp_defense,
                    )
                } else {
                    (
                        None, false,
                        p.attack, p.defense, p.speed, p.sp_attack, p.sp_defense,
                    )
                };

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
                ability:           p.box_mon.ability_string.clone(),
                held_item:         p.box_mon.secure.growth.held_item_string.clone(),
                held_item_id:      p.box_mon.secure.growth.held_item,
                growth_rate:       p.box_mon.secure.growth.growth_rate_string.clone(),
                iv_hp:             p.box_mon.secure.misc.iv_egg_ability.hp_iv,
                iv_atk:            p.box_mon.secure.misc.iv_egg_ability.attack_iv,
                iv_def:            p.box_mon.secure.misc.iv_egg_ability.defense_iv,
                iv_spe:            p.box_mon.secure.misc.iv_egg_ability.speed_iv,
                iv_spa:            p.box_mon.secure.misc.iv_egg_ability.sp_attack_iv,
                iv_spd:            p.box_mon.secure.misc.iv_egg_ability.sp_def_iv,
                ev_hp:             p.box_mon.secure.ev_condition.hp_ev,
                ev_atk:            p.box_mon.secure.ev_condition.attack_ev,
                ev_def:            p.box_mon.secure.ev_condition.defense_ev,
                ev_spe:            p.box_mon.secure.ev_condition.speed_ev,
                ev_spa:            p.box_mon.secure.ev_condition.sp_attack_ev,
                ev_spd:            p.box_mon.secure.ev_condition.sp_defense_ev,
                sprite,
                personality,
                status:            p.status,
                moves: {
                    let m = &p.box_mon.secure.attack.moves;
                    [
                        fire_red_database::move_name(m[0]).to_string(),
                        fire_red_database::move_name(m[1]).to_string(),
                        fire_red_database::move_name(m[2]).to_string(),
                        fire_red_database::move_name(m[3]).to_string(),
                    ]
                },
                pp:                p.box_mon.secure.attack.pp,
            }
        }).collect()
    }

    /// Builds the dead-mon DTO list for one slot, sorted newest-first.
    fn build_dead_dto(&self, dead_records: &HashMap<u32, DeadPokemon>) -> Vec<DeadMonDto> {
        let mut dead_sorted: Vec<&DeadPokemon> = dead_records.values().collect();
        dead_sorted.sort_by_key(|b| std::cmp::Reverse(b.died_at));
        dead_sorted.iter().map(|dp| DeadMonDto {
            nickname:     dp.nickname.clone(),
            species_name: dp.species_name.clone(),
            level:        dp.level,
            nature:       dp.nature.clone(),
            shiny:        dp.is_shiny,
            soul_link:    dp.is_soul_link_death,
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
        }).collect()
    }

    /// Runs one tick: refreshes DB caches, propagates soul-link deaths, and
    /// returns a JSON string if the state has changed since the last tick.
    fn tick(&mut self) -> Option<String> {
        let slots: Vec<Arc<MonitorSlot>> =
            self.live_slots.lock_or_recover().clone();
        let n = slots.len();
        while self.caches.len() < n { self.caches.push(SlotCache::new()); }

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
            if slot.run_changed.swap(false, std::sync::atomic::Ordering::AcqRel) {
                if let Some(db) = &slot.db {
                    db.mark_dirty();
                }
                self.soul_link_propagated.clear();
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
                self.caches[i].caught           = db.list_caught(label);
                self.caches[i].encounters       = db.list_encounters(label);
                self.caches[i].prev_encounters  = db.list_prev_run_encounters(label);
                self.caches[i].last_refresh     = now;
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
            let pi = states[i].1.as_ref().and_then(|gs| gs.preferred_player)
                .map(u32::from).unwrap_or(u32::MAX);
            let pj = states[j].1.as_ref().and_then(|gs| gs.preferred_player)
                .map(u32::from).unwrap_or(u32::MAX);
            pi.cmp(&pj)
                .then_with(|| states[i].0.to_lowercase().cmp(&states[j].0.to_lowercase()))
        });

        // Build JSON payload
        let slots_dto: Vec<SlotDto> = display_order.iter().copied()
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
                        is_shiny:       enc.is_shiny,
                        encountered_at: fire_red_database::format_timestamp(enc.encountered_at),
                        area: {
                            let n = fire_red_location_names::map_area_name(enc.map_group, enc.map_name);
                            if n.is_empty() {
                                format!("{}\u{B7}{}", enc.map_group, enc.map_name)
                            } else {
                                n.to_string()
                            }
                        },
                        sprite:    self.sprite_uri(enc.species, enc.is_shiny),
                        map_group: enc.map_group,
                        map_name:  enc.map_name,
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

                        let party = self.build_party_dto(i, gs, dead_records, soul_link_dead, &states);

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
                let dead = self.build_dead_dto(dead_records);

                let caught: Vec<CaughtMonDto> = self.caches[i].caught.iter()
                    .rev()
                    .map(|cp| CaughtMonDto {
                    nickname:     cp.nickname.clone(),
                    species_name: cp.species_name.clone(),
                    level:        cp.level,
                    nature:       cp.nature.clone(),
                    shiny:        cp.is_shiny,
                    caught_at:         fire_red_database::format_timestamp(cp.caught_at),
                    met_location_name: if cp.location_name.is_empty() {
                        fire_red_location_names::location_name(cp.met_location).to_string()
                    } else {
                        cp.location_name.clone()
                    },
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

                // True player position from EWRAM, not from the encounter header.
                // On randomized ROMs the encounter slot key may differ from the
                // physical map position; always use the EWRAM-derived value.
                let (current_map_group, current_map_name) = match state {
                    Some(gs) => (gs.current_map_group, gs.current_map_name),
                    None => (0u8, 0u8),
                };
                let current_zone_name = match state {
                    Some(gs) if !gs.zone_name.is_empty() => gs.zone_name.clone(),
                    _ => fire_red_location_names::map_area_name(current_map_group, current_map_name)
                        .to_string(),
                };

                // Encounters from the previous completed run for cross-run hints
                let prev_run_encounters: Vec<DbEncounterDto> = self.caches[i].prev_encounters.iter()
                    .map(|enc| DbEncounterDto {
                        species_name:   enc.species_name.clone(),
                        level:          enc.level,
                        caught:         enc.caught,
                        is_shiny:       enc.is_shiny,
                        encountered_at: fire_red_database::format_timestamp(enc.encountered_at),
                        area: {
                            let n = fire_red_location_names::map_area_name(enc.map_group, enc.map_name);
                            if n.is_empty() {
                                format!("{}\u{B7}{}", enc.map_group, enc.map_name)
                            } else {
                                n.to_string()
                            }
                        },
                        sprite:    self.sprite_uri(enc.species, enc.is_shiny),
                        map_group: enc.map_group,
                        map_name:  enc.map_name,
                    })
                    .collect();

                SlotDto { label: label.clone(), connected, db_connected, active_run_id, run_summary, db_encounters, badges, next_gym, party, encounters, dead, caught, box_pokemon, current_map_group, current_map_name, current_zone_name, prev_run_encounters }
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
    testing:    bool,
}

// ---------------------------------------------------------------------------
// Axum handlers
// ---------------------------------------------------------------------------

const OVERLAY_HTML:      &str = include_str!("overlay.html");
const FOCUSED_HTML:      &str = include_str!("focused.html");
const DBVIEWER_HTML:     &str = include_str!("db.html");
const HISTORY_HTML:      &str = include_str!("history.html");
const ALERTS_HTML:       &str = include_str!("alerts.html");
const ROUTES_HTML:       &str = include_str!("routes.html");
const PARTY_PLAIN_HTML:  &str = include_str!("party_plain.html");
const CMD_HTML:          &str = include_str!("cmd.html");
const DBQUERY_HTML:      &str = include_str!("dbquery.html");
const RUNSTATS_HTML:     &str = include_str!("run_stats.html");
const SHINY_HTML:        &str = include_str!("shiny.html");
const MEMORIAL_HTML:     &str = include_str!("memorial.html");
const SOULLINK_HTML:     &str = include_str!("soullink.html");
const ABOUT_HTML:        &str = include_str!("about.html");
const COMPARE_HTML:      &str = include_str!("compare.html");

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
            let all_safe = t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            let within_len = t.len() <= 32;
            if all_safe && within_len {
                let injection = format!(
                    r#"<script>document.documentElement.dataset.theme="{t}"</script>"#
                );
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

async fn serve_html(State(state): State<WebState>, Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(OVERLAY_HTML, state.testing, theme))
}

async fn serve_focused(State(state): State<WebState>, Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(FOCUSED_HTML, state.testing, theme))
}

async fn serve_party(State(state): State<WebState>, Query(params): Query<HashMap<String, String>>) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    if params.contains_key("plain-view") {
        Html(apply_page_with_theme(PARTY_PLAIN_HTML, state.testing, theme))
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
        None    => return axum::Json(serde_json::json!({ "error": "No database configured" })),
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
        return (StatusCode::BAD_REQUEST, "Add ?confirm=true to confirm database wipe".to_string());
    }
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
        None    => return axum::Json(serde_json::json!({ "error": "slot index out of range" })),
    };
    let gs = match slot.state.lock_or_recover().clone() {
        Some(gs) => gs,
        None     => return axum::Json(serde_json::json!({ "error": "slot not connected" })),
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
    Path(index): Path<usize>,
) -> String {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None    => return format!("Slot {index} not found"),
    };
    let gs = match slot.state.lock_or_recover().clone() {
        Some(gs) => gs,
        None     => return format!("Slot {index} not connected"),
    };
    let player = &gs.player_name;
    let map    = if gs.zone_name.is_empty() { "Unknown location" } else { &gs.zone_name };
    let (hp, max_hp) = gs.party.first()
        .map(|p| (p.hp, p.max_hp))
        .unwrap_or((0, 0));
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
        "box"       => &["party", "encounters", "dead", "caught", "db_encounters", "prev_run_encounters"],
        "dead"      => &["encounters", "box_pokemon", "caught", "prev_run_encounters"],
        "caught"    => &["encounters", "box_pokemon", "dead", "prev_run_encounters"],
        "memorial"  => &["encounters", "box_pokemon", "caught", "prev_run_encounters", "db_encounters"],
        "soullink"  => &["encounters", "box_pokemon", "db_encounters", "prev_run_encounters"],
        _           => return json.to_owned(),
    };
    let Ok(mut slots) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return json.to_owned();
    };
    for slot in &mut slots {
        if let Some(obj) = slot.as_object_mut() {
            for key in strip { obj.remove(*key); }
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
                None    => current,
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
                        slot.command_queue
                            .lock_or_recover()
                            .push_back(msg.clone());
                    }
                }
            }
        }
    });

    // Push state updates whenever the broadcast channel changes.
    loop {
        if rx.changed().await.is_err() { break; }
        let raw = rx.borrow_and_update().clone();
        let msg = match &show {
            Some(s) => filter_slots_json(&raw, s),
            None    => raw,
        };
        if ws_tx.send(axum::extract::ws::Message::Text(msg)).await.is_err() {
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

async fn serve_about(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(ABOUT_HTML, state.testing))
}

/// `GET /api/run/:id/stats` — per-run statistics JSON.
async fn api_run_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::run_stats(&conn, run_id)
    }).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/route_stats` — per-route catch-rate statistics JSON.
async fn api_run_route_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::route_stats(&conn, run_id)
    }).await;
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
        None    => return axum::Json(serde_json::json!({ "error": "No database configured" })).into_response(),
    };
    if params.get("format").map(|s| s.as_str()) == Some("csv") {
        let result = tokio::task::spawn_blocking(move || {
            fire_red_database::export_run_csv(&conn, run_id)
        }).await;
        match result {
            Ok(Ok(csv))  => (
                [("content-type", "text/csv"),
                 ("content-disposition", &format!("attachment; filename=\"run_{run_id}.csv\""))],
                csv,
            ).into_response(),
            Ok(Err(e))   => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
            Err(_)       => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Task panicked").into_response(),
        }
    } else {
        let result = tokio::task::spawn_blocking(move || {
            fire_red_database::export_run(&conn, run_id)
        }).await;
        axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" }))).into_response()
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
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::route_odds_json(&conn, run_id)
    }).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/webhook_log` — webhook delivery receipt log for a run.
async fn api_run_webhook_log(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::get_webhook_log_json(&conn, run_id)
    }).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// `GET /api/run/:id/shiny` — shiny odds statistics JSON for a run.
async fn api_shiny_stats(
    State(state): State<WebState>,
    Path(run_id): Path<u32>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::shiny_stats(&conn, run_id)
    }).await;
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
) -> impl IntoResponse {
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({ "error": "No database configured" }))).into_response();
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::active_run_timeline_json(&conn)
    }).await
    .unwrap_or_else(|_| Err(fire_red_database::EventsError::QueryFailed("Task panicked".into())));

    match result {
        Ok(body) =>
            (StatusCode::OK, axum::Json(body)).into_response(),
        Err(fire_red_database::EventsError::NoActiveRun) =>
            (StatusCode::NOT_FOUND,
             axum::Json(serde_json::json!({ "error": "no active run" }))).into_response(),
        Err(e) =>
            (StatusCode::INTERNAL_SERVER_ERROR,
             axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
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
        return (StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({ "error": "No database configured" }))).into_response();
    };
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::list_events_json(&conn, run_id)
    }).await
    .unwrap_or_else(|_| Err(fire_red_database::EventsError::QueryFailed("Task panicked".into())));

    match result {
        Ok(body) =>
            (StatusCode::OK, axum::Json(body)).into_response(),
        Err(e) =>
            (StatusCode::INTERNAL_SERVER_ERROR,
             axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

/// `GET /api/runs` — summary list of all runs (id, player, dates, deaths, catches, encounters).
async fn api_runs(
    State(state): State<WebState>,
) -> axum::Json<serde_json::Value> {
    let conn = require_db!(state);
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::list_all_runs_json(&conn)
    }).await;
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
    let result = tokio::task::spawn_blocking(move || {
        fire_red_database::import_run(&conn, &body)
    }).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

/// Broadcasts `end_run` or `new_run` to all connected tracker slots.
async fn api_command(
    State(state): State<WebState>,
    Path(cmd): Path<String>,
) -> impl IntoResponse {
    let msg = match cmd.as_str() {
        "end_run" => ClientMessage::EndRun,
        "new_run" => ClientMessage::NewRun,
        other => return (StatusCode::BAD_REQUEST, format!("Unknown command: {other}")),
    };
    let slots = state.live_slots.lock_or_recover().clone();
    let count = slots.len();
    for slot in &slots {
        slot.command_queue
            .lock_or_recover()
            .push_back(msg.clone());
    }
    (StatusCode::OK, format!("Command '{cmd}' sent to {count} slot(s)"))
}

/// Runs arbitrary SQL against the database and returns results as JSON.
///
/// Restricted to loopback connections — returns 403 for any remote caller.
async fn api_db_query(
    State(state): State<WebState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    if !addr.ip().is_loopback() {
        return axum::Json(serde_json::json!({ "error": "Forbidden: endpoint only available on localhost" }));
    }
    let conn = require_db!(state);
    let sql = match body["sql"].as_str() {
        Some(s) => s.to_string(),
        None    => return axum::Json(serde_json::json!({ "error": "Missing 'sql' field" })),
    };
    let result = tokio::task::spawn_blocking(move || fire_red_database::run_sql(&conn, &sql)).await;
    axum::Json(result.unwrap_or_else(|_| serde_json::json!({ "error": "Task panicked" })))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(live_slots: SharedSlots, port: u16, db_conn: Option<String>, testing: bool) {
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

    let web_state = WebState { tx, live_slots, db_conn, testing };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let app = Router::new()
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
            .route("/api/bot/:index", get(api_bot_summary))
            .route("/api/command/:cmd", post(api_command))
            .route("/api/db/query", post(api_db_query))
            .route("/api/runs",            get(api_runs))
            .route("/api/run/import",      post(api_run_import))
            .route("/api/run/:id/stats",        get(api_run_stats))
            .route("/api/run/:id/route_stats",   get(api_run_route_stats))
            .route("/api/run/:id/route_odds",    get(api_run_route_odds))
            .route("/api/run/:id/webhook_log",   get(api_run_webhook_log))
            .route("/api/run/:id/shiny",         get(api_shiny_stats))
            .route("/api/run/:id/export",  get(api_run_export))
            .route("/api/run/:id/events",  get(api_run_events))
            .route("/api/timeline",        get(api_active_timeline))
            .route("/history", get(serve_history))
            .route("/shiny", get(serve_shiny))
            .route("/memorial", get(serve_memorial))
            .route("/soullink", get(serve_soullink))
            .route("/alerts", get(serve_alerts))
            .route("/:index/alerts", get(serve_alerts))
            .route("/:index/routes", get(serve_routes))
            .route("/:index/party", get(serve_party))
            .route("/:index/encounters", get(serve_focused))
            .route("/:index/dead", get(serve_focused))
            .route("/:index/caught", get(serve_focused))
            .route("/:index/box", get(serve_focused))
            .route("/run/:id/stats", get(serve_run_stats))
            .route("/run/:id/memorial", get(serve_memorial))
            .route("/about", get(serve_about))
            .route("/compare", get(serve_compare))
            .with_state(web_state);

        let addr = format!("0.0.0.0:{}", port);
        tracing::info!("WebSocket overlay listening on http://{}", addr);
        tracing::info!("Add in OBS as Browser Source: http://localhost:{}", port);

        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("failed to bind WebSocket port");
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        ).await {
            tracing::error!("WebSocket server error: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_HTML: &str = r#"<!DOCTYPE html><html><head><!-- THEME_SLOT --></head><body>__VERSION__</body></html>"#;

    #[test]
    fn apply_page_replaces_version() {
        let out = apply_page(SAMPLE_HTML, false);
        assert!(out.contains(VERSION), "VERSION not injected");
        assert!(!out.contains("__VERSION__"), "__VERSION__ not replaced");
    }

    #[test]
    fn apply_page_no_theme_removes_slot() {
        let out = apply_page(SAMPLE_HTML, false);
        assert!(!out.contains("<!-- THEME_SLOT -->"), "theme slot should be removed");
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
        assert!(out.contains(r#"dataset.theme="light""#), "light theme not injected: {out}");
    }

    #[test]
    fn apply_page_with_theme_rejects_invalid_input() {
        // Themes containing characters outside [a-zA-Z0-9_-] are rejected entirely
        // rather than being stripped and concatenated, which would produce confusing output.
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("light<script>alert(1)</script>"));
        assert!(!out.contains("<script>alert"), "XSS not sanitized");
        assert!(!out.contains("lightscript"), "stripped-and-concatenated theme should not appear");
        assert!(!out.contains("data-theme"), "rejected theme should not inject any attribute");
    }

    #[test]
    fn apply_page_with_theme_rejects_oversized_input() {
        let long = "a".repeat(33);
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some(&long));
        assert!(!out.contains("data-theme"), "theme longer than 32 chars should be rejected");
    }

    #[test]
    fn apply_page_with_theme_accepts_hyphen_and_underscore() {
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("my_custom-theme"));
        assert!(out.contains(r#"dataset.theme="my_custom-theme""#), "valid theme with - and _ rejected");
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
}
