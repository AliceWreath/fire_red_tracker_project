//! DB read-side state, soul-link state, and shared axum state.

use super::*;

// ---------------------------------------------------------------------------
// DB + soul-link state (mirrors AggregatorApp in app.rs)
// ---------------------------------------------------------------------------

pub(crate) struct SlotCache {
    pub(crate) caught: Vec<CaughtPokemon>,
    pub(crate) encounters: Vec<fire_red_database::Encounter>,
    pub(crate) prev_encounters: Vec<fire_red_database::Encounter>,
    pub(crate) last_refresh: Instant,
    /// Owner-pinned display column (1 = leftmost). `None` = no preference.
    pub(crate) slot_index: Option<u8>,
}

impl SlotCache {
    pub(crate) fn new() -> Self {
        Self {
            caught: Vec::new(),
            encounters: Vec::new(),
            prev_encounters: Vec::new(),
            last_refresh: Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
            slot_index: None,
        }
    }
}

/// Maps a gym leader / Elite 4 member name to their primary type ID (Gen III).
/// Returns 0 (Normal) for unrecognised names such as the post-game Champion rematch.
pub(crate) fn leader_type_id(leader: &str) -> u8 {
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
pub(crate) fn leader_trainer_index(leader: &str) -> Option<usize> {
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
pub(crate) const TRAINER_ENTRY_SIZE: usize = 40;

/// GBA ROM bus base address; subtract to get ROM file offset.
pub(crate) const ROM_BUS_BASE: u32 = 0x0800_0000;

/// Reads the gym leader's party from the loaded ROM and builds the DTO list.
///
/// Handles all four party struct layouts (no-item/custom-moves combinations).
/// Returns an empty vec if the ROM is not loaded, the trainer index is unknown,
/// or the ROM file is too small to contain the expected data.
pub(crate) fn build_leader_party(leader_name: &str) -> Vec<LeaderPartyMonDto> {
    let trainer_idx = match leader_trainer_index(leader_name) {
        Some(i) => i,
        None => {
            tracing::warn!("vs_leader: no trainer index for {:?}", leader_name);
            return vec![];
        }
    };
    let rom = match fire_red_rom_buffer::try_get_rom() {
        Some(r) => r,
        None => {
            tracing::warn!("vs_leader: ROM buffer not yet initialized");
            return vec![];
        }
    };
    let trainer_table = fire_red_rom_buffer::get_rom_addresses().trainer_data_addr;
    if trainer_table == 0 {
        // Log once per process; the overlay polls every 100 ms so without this
        // guard the warning would fill the log immediately.
        static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        WARNED.get_or_init(|| {
            tracing::warn!(
                "vs_leader: trainer table could not be located for this ROM \
                 revision ({:?}) — overlay disabled (this message won't repeat)",
                fire_red_rom_buffer::get_rom_revision()
            );
        });
        return vec![];
    }
    let entry_off = trainer_table + trainer_idx * TRAINER_ENTRY_SIZE;
    if rom.len() < entry_off + TRAINER_ENTRY_SIZE {
        tracing::warn!(
            "vs_leader: ROM too small for trainer entry — \
             rom_len={} entry_off={:#X} need={}",
            rom.len(), entry_off, entry_off + TRAINER_ENTRY_SIZE
        );
        return vec![];
    }
    let entry = &rom[entry_off..entry_off + TRAINER_ENTRY_SIZE];

    let party_flags  = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
    let party_size   = entry[0x20] as usize;
    let party_ptr    = u32::from_le_bytes([entry[0x24], entry[0x25], entry[0x26], entry[0x27]]);

    if party_ptr < ROM_BUS_BASE || party_size == 0 || party_size > 6 {
        tracing::warn!(
            "vs_leader: invalid trainer entry for {:?} (idx {}) at ROM offset {:#X} — \
             party_flags={:#010X} party_size={} party_ptr={:#010X} ROM_BUS_BASE={:#010X}",
            leader_name, trainer_idx, entry_off,
            party_flags, party_size, party_ptr, ROM_BUS_BASE
        );
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
        tracing::warn!(
            "vs_leader: party data out of ROM bounds for {:?} — \
             party_ptr={:#010X} party_off={:#X} party_size={} entry_bytes={} rom_len={}",
            leader_name, party_ptr, party_off, party_size, entry_bytes, rom.len()
        );
        return vec![];
    }

    let result: Vec<LeaderPartyMonDto> = (0..party_size)
        .filter_map(|i| {
            let base = party_off + i * entry_bytes;
            let b = &rom[base..base + entry_bytes];
            let level   = b[1];
            let species = u16::from_le_bytes([b[2], b[3]]);
            if level == 0 {
                // Level 0 is never valid for a trainer's Pokémon; reject the
                // entire leader entry rather than silently showing garbage data.
                tracing::warn!(
                    "vs_leader: {:?} slot {} has level 0 (species={}) — \
                     trainer table address is likely wrong",
                    leader_name, i, species
                );
                return None;
            }
            if species == 0 || species > fire_red_states::MAX_NATIONAL_DEX_FIRERED {
                tracing::warn!(
                    "vs_leader: {:?} slot {} has out-of-range species {} (max {})",
                    leader_name, i, species, fire_red_states::MAX_NATIONAL_DEX_FIRERED
                );
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
        .collect();
    if result.is_empty() {
        tracing::warn!(
            "vs_leader: build_leader_party({:?}) produced no mons \
             (party_size={} entry_bytes={} has_item={} has_moves={})",
            leader_name, party_size, entry_bytes, has_item, has_moves
        );
    } else {
        tracing::debug!(
            "vs_leader: built {} mons for {:?}",
            result.len(), leader_name
        );
    }
    result
}

pub(crate) struct BroadcastLoop {
    pub(crate) live_slots: SharedSlots,
    pub(crate) caches: Vec<SlotCache>,
    pub(crate) soul_link_propagated: HashSet<(usize, u32)>,
    /// Manual soul-link overrides for the current run (personality → partner_personality).
    /// Refreshed alongside the caught cache; consulted before automatic met_location pairing.
    pub(crate) soul_link_overrides: HashMap<u32, u32>,
    pub(crate) last_json: String,
    pub(crate) sprites: PngSpriteCache,
    /// Per-slot: set of (run_id) for which we have already triggered a backup so we
    /// don't fire again on subsequent ticks.
    pub(crate) backup_done: HashSet<u32>,
    /// Per-slot: badge count observed on the previous tick, for LiveSplit split detection.
    pub(crate) prev_badge_counts: Vec<usize>,
    /// DB connection string (for auto-backup).
    pub(crate) db_conn: Option<String>,
    /// Directory to write auto-backup files into.
    pub(crate) backup_dir: Option<String>,
    /// Whether to fire a LiveSplit split on each new badge.
    pub(crate) livesplit_split_on_badges: bool,
}

impl BroadcastLoop {
    pub(crate) fn new(
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
    pub(crate) fn request_sprites(&self, slots: &[Arc<MonitorSlot>], states: &[(String, Option<GameState>)]) {
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

    /// Drains any pending textures from the game-polling pipeline into the sprite cache.
    /// Also wires the shared sprite cache into any slot that was added after
    /// `run()` started (identified by having `sprite_cache = None`).
    pub(crate) fn drain_sprites(&mut self, slots: &[Arc<MonitorSlot>]) {
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
    pub(crate) fn sprite_uri(&self, species: u16, shiny: bool) -> Option<String> {
        let cache = self.sprites.lock_or_recover();
        cache
            .get(&(species, shiny))
            .map(|png| format!("data:image/png;base64,{}", base64_encode(png)))
    }

    /// Propagates soul-link deaths across slots (DB-persisted and live) and
    /// returns the set of personality values that are soul-link-dead per slot.
    pub(crate) fn propagate_soul_links(
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
    pub(crate) fn build_party_dto(
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
    pub(crate) fn build_dead_dto(&self, dead_records: &HashMap<u32, DeadPokemon>) -> Vec<DeadMonDto> {
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
    pub(crate) fn tick(&mut self) -> Option<String> {
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
                self.caches[i].slot_index = db.query_player_slot_index(label);
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

        // Determine display order: sort by (slot_index, player_name).
        // slot_index is the owner-pinned column (DB); falls back to preferred_player
        // from game state, then u32::MAX (sorts last with alphabetical tiebreak).
        let mut display_order: Vec<usize> = (0..n).collect();
        display_order.sort_by(|&i, &j| {
            let pi = self.caches[i].slot_index
                .or_else(|| states[i].1.as_ref().and_then(|gs| gs.preferred_player))
                .map(u32::from)
                .unwrap_or(u32::MAX);
            let pj = self.caches[j].slot_index
                .or_else(|| states[j].1.as_ref().and_then(|gs| gs.preferred_player))
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
                    pinned_slot_index: self.caches[i].slot_index,
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

pub(crate) type IntegrationManager = Arc<Mutex<HashMap<u32, HashMap<String, Arc<AtomicBool>>>>>;

#[derive(Clone)]
pub(crate) struct WebState {
    pub(crate) tx: watch::Sender<String>,
    pub(crate) live_slots: SharedSlots,
    pub(crate) db_conn: Option<String>,
    pub(crate) testing: bool,
    pub(crate) allow_injections: bool,
    pub(crate) connector: Option<Arc<crate::direct::DirectConnector>>,
    /// Directory for JSON backups; used by the manual POST /api/backup trigger.
    pub(crate) backup_dir: Option<String>,
    /// Retention count for scheduled/manual snapshots in `backup_dir`.
    pub(crate) backup_keep: usize,
    pub(crate) discord_slash: Option<crate::config::DiscordSlashConfig>,
    /// Path to the TOML config file, used by the hot-reload endpoint.
    pub(crate) config_path: Option<Arc<String>>,
    /// In-memory map from user_id to their most recently connected run_id.
    pub(crate) user_active_run: Arc<Mutex<HashMap<u32, u32>>>,
    /// Stop flags for per-user integration threads: user_id → kind → stop flag.
    pub(crate) integration_manager: IntegrationManager,
}
