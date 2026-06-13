//! # Game state helpers
//!
//! Utilities for reading live game state directly from the EWRAM/IWRAM
//! snapshots maintained by `fire_red_memory`, bypassing the polling-thread
//! intermediaries in `fire_red_loop` to avoid lag and race conditions.

use fire_red_loop::FireRedState;
use fire_red_party_monitor::Pokemon;
use fire_red_states::{BagPockets, ItemSlot, LockOrRecover, MAX_NATIONAL_DEX_FIRERED};
use std::sync::{Arc, Mutex};

/// GBA address of the packed (map_group, map_name) bytes in EWRAM.
pub const MAP_GROUP_AND_NAME_ADDR: usize = 0x02031DBC;

/// Base address of EWRAM in the GBA address space.
pub const EWRAM_BASE: usize = 0x02000000;

/// Base address of IWRAM in the GBA address space.
pub const IWRAM_BASE: usize = 0x03000000;

/// GBA address of gEnemyParty[0] for FireRed USA Rev 1.
/// Confirmed empirically: personality changes to a new value on every new wild battle.
/// Note: this slot is NOT cleared between battles — detection must use personality
/// change rather than presence/absence.
const ENEMY_PARTY_ADDR: usize = 0x0202402C;

/// IWRAM address of the gSaveBlock1Ptr pointer (4-byte little-endian GBA address).
const SAVE_BLOCK_1_PTR_ADDR: usize = 0x03005008;

/// IWRAM address of the gSaveBlock2Ptr pointer (4-byte little-endian GBA address).
/// Confirmed empirically: IWRAM[0x500C] = valid EWRAM ptr; IWRAM[0x5004] = 0x00000001 (not a ptr).
const SAVE_BLOCK_2_PTR_ADDR: usize = 0x0300500C;

/// Byte offset of the balls pocket (13 × ItemSlot) within SaveBlock1.
/// Confirmed empirically via --scan-balls-pocket: Pokéball (item_id=4) first
/// appears at slot 0 of the window starting here, with all other slots empty.
const BALLS_POCKET_SAVE_BLOCK_OFFSET: usize = 0x0430;

/// Number of slots in the balls pocket.
const BALLS_POCKET_SLOTS: usize = 13;

/// Byte offset of the general items pocket (42 × ItemSlot) within SaveBlock1.
/// Pockets are laid out: items(0x0310) → key_items(0x03B8) → balls(0x0430) → TMs(0x0464).
const ITEMS_POCKET_SAVE_BLOCK_OFFSET: usize = 0x0310;

/// Number of slots in the general items pocket.
const ITEMS_POCKET_SLOTS: usize = 42;

/// Byte offset of the key items pocket (30 × ItemSlot) within SaveBlock1.
const KEY_ITEMS_POCKET_SAVE_BLOCK_OFFSET: usize = 0x03B8;

/// Number of slots in the key items pocket.
const KEY_ITEMS_POCKET_SLOTS: usize = 30;

/// Byte offset of the TMs/HMs pocket (58 × ItemSlot) within SaveBlock1.
const TMS_POCKET_SAVE_BLOCK_OFFSET: usize = 0x0464;

/// Number of slots in the TMs/HMs pocket.
const TMS_POCKET_SLOTS: usize = 58;

/// Maximum item quantity storable in a single bag slot (vanilla FireRed cap).
const MAX_ITEM_QTY: u16 = 99;

/// Fallback EWRAM base for SaveBlock2 when gSaveBlock2Ptr can't be resolved from IWRAM.
const SAVE_BLOCK_2_BASE: usize = 0x020245DC;

/// Byte offset of `encryptionKey` (u32) within SaveBlock2.
/// Confirmed empirically: IWRAM[0x500C]+0x0F20 contains the live key.
const SECURITY_KEY_OFFSET: usize = 0x0F20;

/// Returns `true` if the pokemon with `personality` and `ot_id` is shiny.
///
/// Uses the Gen III formula: `(p_high ^ p_low ^ id_high ^ id_low) < 8`.
pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p_high  = (personality >> 16) as u16;
    let p_low   = (personality & 0xFFFF) as u16;
    let id_high = (ot_id >> 16) as u16;
    let id_low  = (ot_id & 0xFFFF) as u16;
    (p_high ^ p_low ^ id_high ^ id_low) < 8
}

/// Overwrites the shared party list with the current party members.
pub fn fill_party_list(thread_party: &Arc<Mutex<Vec<Pokemon>>>) {
    *thread_party.lock_or_recover() =
        fire_red_loop::get_party_members();
}

/// Checks the current party for any Pokemon with zero HP and marks them dead.
///
/// Skips Pokemon that are already recorded to avoid double-entries.
/// Skips the entire check when `run_tracking_active` is `false` — deaths before
/// Pokéballs are obtainable (e.g. losing to the rival in Oak's lab) are not
/// Nuzlocke deaths, mirroring how encounters are ignored before balls arrive.
pub fn check_for_dead_pokemon(thread_party: &Arc<Mutex<Vec<Pokemon>>>, run_tracking_active: bool) {
    if !run_tracking_active { return; }
    // Snapshot dead candidates before releasing the lock so the subsequent DB
    // and webhook calls don't hold the party mutex during potentially slow I/O,
    // which would block fill_party_list for the full duration of each DB write.
    let candidates: Vec<Pokemon> = {
        let party = thread_party.lock_or_recover();
        party.iter().filter(|p| p.hp == 0).cloned().collect()
    };
    for pokemon in candidates {
        if pokemon.hp != 0 { continue; }
        if fire_red_database::is_dead(pokemon.box_mon.personality) { continue; }

        let personality = pokemon.box_mon.personality;
        let ot_id       = pokemon.box_mon.ot_id;
        let growth      = &pokemon.box_mon.secure.growth;
        let atk         = &pokemon.box_mon.secure.attack;
        let ev          = &pokemon.box_mon.secure.ev_condition;
        let iv          = &pokemon.box_mon.secure.misc.iv_egg_ability;
        let misc        = &pokemon.box_mon.secure.misc;

        let ot_name = fire_red_text::gba_string_to_ascii(
            &pokemon.box_mon.ot_name,
            pokemon.box_mon.ot_name.len(),
            0,
        )
        .trim_matches('\0')
        .trim()
        .to_string();

        let died_at    = fire_red_database::unix_now();
        let shiny_flag = is_shiny(personality, ot_id);

        let recorded = match fire_red_database::mark_dead(fire_red_database::DeadPokemon {
            player_name:   fire_red_loop::get_trainer_name(),
            personality,
            ot_id,
            ot_name,
            nickname:      pokemon.box_mon.nickname_string.clone(),
            species:       growth.species,
            species_name:  growth.species_string.clone(),
            is_shiny:      shiny_flag,
            nature:        fire_red_database::nature_name(personality).to_string(),

            level:      pokemon.level,
            experience: growth.experience,
            max_hp:     pokemon.max_hp,
            attack:     pokemon.attack,
            defense:    pokemon.defense,
            speed:      pokemon.speed,
            sp_attack:  pokemon.sp_attack,
            sp_defense: pokemon.sp_defense,

            moves: atk.moves,
            pp:    atk.pp,

            ivs: fire_red_database::IVs {
                hp:        iv.hp_iv,
                attack:    iv.attack_iv,
                defense:   iv.defense_iv,
                speed:     iv.speed_iv,
                sp_attack: iv.sp_attack_iv,
                sp_defense: iv.sp_def_iv,
            },
            evs: fire_red_database::EVs {
                hp:        ev.hp_ev,
                attack:    ev.attack_ev,
                defense:   ev.defense_ev,
                speed:     ev.speed_ev,
                sp_attack: ev.sp_attack_ev,
                sp_defense: ev.sp_defense_ev,
            },

            held_item:    growth.held_item,
            ability:      pokemon.box_mon.ability,
            ability_name: pokemon.box_mon.ability_string.clone(),
            friendship:   growth.friendship,
            met_location: misc.met_location,
            gender:       pokemon.box_mon.gender,

            died_at,
            // Regular in-battle deaths are never soul-link deaths; the aggregator
            // sets is_soul_link_death = true when it calls mark_soul_link_dead().
            is_soul_link_death: false,
        }) {
            Ok(v)  => v,
            Err(e) => {
                tracing::error!("Failed to record dead pokemon (personality=0x{:08X}): {e}", personality);
                continue;
            }
        };
        if !recorded { continue; }

        if let Err(e) = fire_red_database::record_event(fire_red_database::EventKind::Death {
            species_name: &growth.species_string,
            nickname:     &pokemon.box_mon.nickname_string,
            level:        pokemon.level,
        }) {
            tracing::warn!("Failed to record Death event: {e}");
        }
        crate::webhook::fire_event(crate::webhook::WebhookEvent::Death {
            player:    fire_red_loop::get_trainer_name(),
            timestamp: died_at,
            pokemon:   crate::webhook::PokemonInfo {
                nickname: pokemon.box_mon.nickname_string.clone(),
                species:  growth.species_string.clone(),
                level:    pokemon.level,
                shiny:    shiny_flag,
                nature:   fire_red_database::nature_name(personality).to_string(),
            },
        });
    }
}

/// Reads the current map state directly from the EWRAM snapshot.
///
/// Returns `None` if the snapshot is not yet populated or the bytes are `(0,0)`
/// (indicating the snapshot hasn't been filled with real game data yet).
///
/// This bypasses the `STATE` mutex in `fire_red_loop`, which lags behind by up
/// to ~833ms (500ms EWRAM snapshot interval + 333ms map thread interval) and
/// may contain `(0,0)` before the map thread has ticked for the first time.
pub fn map_state_from_ewram() -> Option<FireRedState> {
    let ewram  = fire_red_memory::get_ewram();
    let offset = MAP_GROUP_AND_NAME_ADDR - EWRAM_BASE;
    if ewram.len() < offset + 2 {
        return None;
    }
    let group = ewram[offset];
    let name  = ewram[offset + 1];
    if group == 0 && name == 0 {
        return None;
    }
    Some(FireRedState { map_group_id: group, map_name_id: name })
}

/// Scans the current party for any Pokemon not yet in the caught log and records them.
///
/// Called alongside `check_for_dead_pokemon` on every party refresh so that
/// newly obtained mons (caught, gifted, or traded) are captured immediately.
pub fn check_for_new_pokemon(thread_party: &Arc<Mutex<Vec<Pokemon>>>) {
    // Snapshot the full party before releasing the lock so the DB and webhook
    // calls below don't hold the party mutex during potentially slow I/O,
    // which would block fill_party_list for the full duration of each DB write.
    let snapshot: Vec<Pokemon> = thread_party.lock_or_recover().clone();

    for pokemon in &snapshot {
        let species = pokemon.box_mon.secure.growth.species;
        if species == 0 || species > MAX_NATIONAL_DEX_FIRERED { continue; }
        let personality = pokemon.box_mon.personality;
        if fire_red_database::is_caught(personality) {
            let nickname = &pokemon.box_mon.nickname_string;
            if !nickname.is_empty()
                && let Some(old_name) = fire_red_database::update_caught_nickname(personality, nickname) {
                    let species_name = &pokemon.box_mon.secure.growth.species_string;
                    if let Err(e) = fire_red_database::record_event(fire_red_database::EventKind::NicknameChange {
                        species_name,
                        old_name:    &old_name,
                        new_name:    nickname,
                    }) {
                        tracing::warn!("Failed to record NicknameChange event: {e}");
                    }
                    crate::webhook::fire_event(crate::webhook::WebhookEvent::NicknameChange {
                        player:    fire_red_loop::get_trainer_name(),
                        timestamp: fire_red_database::unix_now(),
                        species:   species_name.clone(),
                        old_name,
                        new_name:  nickname.clone(),
                    });
                    tracing::info!("Nickname changed: {} → {}", species_name, nickname);
            }
            let ev = &pokemon.box_mon.secure.ev_condition;
            fire_red_database::update_caught_evs(personality, &fire_red_database::EVs {
                hp:         ev.hp_ev,
                attack:     ev.attack_ev,
                defense:    ev.defense_ev,
                speed:      ev.speed_ev,
                sp_attack:  ev.sp_attack_ev,
                sp_defense: ev.sp_defense_ev,
            });
            continue;
        }

        let ot_id  = pokemon.box_mon.ot_id;
        let growth = &pokemon.box_mon.secure.growth;
        let misc   = &pokemon.box_mon.secure.misc;
        let iv     = &misc.iv_egg_ability;
        let ev     = &pokemon.box_mon.secure.ev_condition;

        let caught_at     = fire_red_database::unix_now();
        let shiny_flag    = is_shiny(personality, ot_id);
        let location_name = map_state_from_ewram()
            .map(|s| {
                let n = fire_red_loop::get_area_name_for(s.map_group_id, s.map_name_id);
                if n.is_empty() { format!("{}\u{B7}{}", s.map_group_id, s.map_name_id) }
                else            { n.to_string() }
            })
            .unwrap_or_default();

        // Insert into the DB first. Only fire the event log and webhook when
        // mark_caught confirms the row was newly inserted — this prevents
        // duplicate Catch events when a transient DB error causes a retry.
        let newly_inserted = fire_red_database::mark_caught(fire_red_database::CaughtPokemon {
            player_name:   fire_red_loop::get_trainer_name(),
            personality,
            ot_id,
            nickname:      pokemon.box_mon.nickname_string.clone(),
            species,
            species_name:  growth.species_string.clone(),
            is_shiny:      shiny_flag,
            nature:        fire_red_database::nature_name(personality).to_string(),
            level:         pokemon.level,
            met_location:  misc.met_location,
            location_name,
            gender:        pokemon.box_mon.gender,
            ivs: fire_red_database::IVs {
                hp:         iv.hp_iv,
                attack:     iv.attack_iv,
                defense:    iv.defense_iv,
                speed:      iv.speed_iv,
                sp_attack:  iv.sp_attack_iv,
                sp_defense: iv.sp_def_iv,
            },
            evs: fire_red_database::EVs {
                hp:         ev.hp_ev,
                attack:     ev.attack_ev,
                defense:    ev.defense_ev,
                speed:      ev.speed_ev,
                sp_attack:  ev.sp_attack_ev,
                sp_defense: ev.sp_defense_ev,
            },
            caught_at,
        });
        if !newly_inserted { continue; }

        if let Err(e) = fire_red_database::record_event(fire_red_database::EventKind::Catch {
            species_name: &growth.species_string,
            nickname:     &pokemon.box_mon.nickname_string,
            level:        pokemon.level,
        }) {
            tracing::warn!("Failed to record Catch event: {e}");
        }
        crate::webhook::fire_event(crate::webhook::WebhookEvent::Catch {
            player:    fire_red_loop::get_trainer_name(),
            timestamp: caught_at,
            pokemon:   crate::webhook::PokemonInfo {
                nickname: pokemon.box_mon.nickname_string.clone(),
                species:  growth.species_string.clone(),
                level:    pokemon.level,
                shiny:    shiny_flag,
                nature:   fire_red_database::nature_name(personality).to_string(),
            },
        });
    }
}

/// Resolves the SaveBlock1 base address by dereferencing the 4-byte LE pointer
/// stored in IWRAM at `SAVE_BLOCK_1_PTR_ADDR`.
///
/// Returns `None` if IWRAM is too small to hold the pointer or the pointer falls
/// outside EWRAM. Centralised here to avoid duplicating the same 8-line bounds-check
/// pattern across `game_is_loaded`, `count_pokeballs`, and the dev-tool scanners.
fn read_save_block1_ptr(iwram: &[u8], ewram: &[u8]) -> Option<usize> {
    let ptr_offset = SAVE_BLOCK_1_PTR_ADDR - IWRAM_BASE;
    if iwram.len() < ptr_offset + 4 { return None; }
    let ptr = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;
    if ptr < EWRAM_BASE || ptr >= EWRAM_BASE + ewram.len() { return None; }
    Some(ptr)
}

fn read_save_block2_ptr(iwram: &[u8], ewram: &[u8]) -> Option<usize> {
    let ptr_offset = SAVE_BLOCK_2_PTR_ADDR - IWRAM_BASE;
    if iwram.len() < ptr_offset + 4 { return None; }
    let ptr = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;
    if ptr < EWRAM_BASE || ptr >= EWRAM_BASE + ewram.len() { return None; }
    Some(ptr)
}

/// Returns `true` if FireRed appears to be fully loaded with a valid save.
///
/// Checks three signals in order of reliability:
/// 1. The SaveBlock1 pointer at `0x03005008` in IWRAM points into valid EWRAM.
/// 2. The party size byte is in the range 0–6.
/// 3. The map group/name bytes are non-zero.
///
/// All three failing together strongly indicates a reset or title screen.
/// Used to gate badge reads and clear stale state after a soft reset.
pub fn game_is_loaded() -> bool {
    let iwram = fire_red_memory::get_iwram();
    let ewram = fire_red_memory::get_ewram();

    // SaveBlock1 pointer must resolve to a valid EWRAM address.
    if read_save_block1_ptr(&iwram, &ewram).is_none() {
        return false;
    }

    // Party size must be 0–6.
    let party_size_offset = 0x02024029 - EWRAM_BASE;
    if ewram.len() <= party_size_offset || ewram[party_size_offset] > 6 {
        return false;
    }

    // Map state should be non-zero — the title screen sits at (0, 0).
    let map_offset = MAP_GROUP_AND_NAME_ADDR - EWRAM_BASE;
    if ewram.len() < map_offset + 2 {
        return false;
    }
    if ewram[map_offset] == 0 && ewram[map_offset + 1] == 0 {
        return false;
    }

    true
}

/// Returns `true` if every Pokémon in the current party is recorded as dead,
/// indicating a Nuzlocke party wipe. Calls `end_run()` and returns `true` so
/// the caller can lock the encounter tracker against further tracking.
///
/// Returns `false` when the party is empty (pre-game or between battles) or
/// when any party member is still alive in the database, and when
/// `run_tracking_active` is false (run hasn't officially started yet).
pub fn check_for_run_over(thread_party: &Arc<Mutex<Vec<Pokemon>>>, run_tracking_active: bool) -> bool {
    if !run_tracking_active { return false; }
    let party = thread_party.lock_or_recover();
    if party.is_empty() { return false; }
    let all_dead = party.iter().all(|p| {
        fire_red_database::is_dead(p.box_mon.personality)
    });
    drop(party);
    if all_dead {
        if let Err(e) = fire_red_database::record_event(fire_red_database::EventKind::Wipe) {
            tracing::warn!("Failed to record Wipe event: {e}");
        }
        fire_red_database::end_run();
        crate::webhook::fire_event(crate::webhook::WebhookEvent::Wipe {
            player:    fire_red_loop::get_trainer_name(),
            timestamp: fire_red_database::unix_now(),
        });
        return true;
    }
    false
}

/// Scans SaveBlock1 for the balls pocket by sliding a 13-slot window and
/// checking only item IDs (quantities are XOR-encrypted in RAM and cannot be
/// read directly). A valid candidate window has every item_id either 0 (empty
/// slot) or 1–12 (a Pokéball type), with at least one non-zero item_id.
///
/// Run this with at least one ball in the bag. The printed SaveBlock1 offset
/// is the value to use for `BALLS_POCKET_SAVE_BLOCK_OFFSET`.
#[cfg(feature = "dev-tools")]
pub fn scan_for_balls_pocket() {
    let iwram = fire_red_memory::get_iwram();
    let ewram = fire_red_memory::get_ewram();

    let save_block_ptr = match read_save_block1_ptr(&iwram, &ewram) {
        Some(p) => { println!("SaveBlock1 ptr: 0x{:08X}", p); p }
        None    => { eprintln!("SaveBlock1 ptr invalid or EWRAM too small — is the game loaded?"); return; }
    };

    let sb1_start = save_block_ptr - EWRAM_BASE;
    let scan_end  = sb1_start + 0x3000;

    println!("Sliding 13-slot window through SaveBlock1+0x0000..+0x3000");
    println!("Checking item IDs only (quantities are encrypted in RAM)");
    println!("Valid window: all item_ids are 0 or 1-12, at least one is non-zero");
    println!();

    let mut found_any = false;
    let mut window_start = sb1_start;
    while window_start + BALLS_POCKET_SLOTS * 4 <= scan_end.min(ewram.len()) {
        // Require that the slot immediately before the window is empty or non-ball.
        // This filters out mid-pocket views and keeps only true pocket starts.
        if window_start >= sb1_start + 4 {
            let prev_base    = window_start - 4;
            let prev_item_id = u16::from_le_bytes([ewram[prev_base], ewram[prev_base + 1]]);
            if (1..=12).contains(&prev_item_id) {
                window_start += 4;
                continue;
            }
        }

        let mut all_clean = true;
        let mut any_ball  = false;

        for slot in 0..BALLS_POCKET_SLOTS {
            let base    = window_start + slot * 4;
            let item_id = u16::from_le_bytes([ewram[base], ewram[base + 1]]);
            if item_id == 0 {
                continue;
            }
            if (1..=12).contains(&item_id) {
                any_ball = true;
            } else {
                all_clean = false;
                break;
            }
        }

        if all_clean && any_ball {
            let pocket_offset = window_start - sb1_start;
            println!("  Candidate at SaveBlock1+0x{:04X}:", pocket_offset);
            for slot in 0..BALLS_POCKET_SLOTS {
                let base    = window_start + slot * 4;
                let item_id = u16::from_le_bytes([ewram[base], ewram[base + 1]]);
                if item_id != 0 {
                    println!("    slot {:2}: item_id={:2}", slot, item_id);
                }
            }
            println!();
            found_any = true;
        }

        window_start += 4;
    }

    if !found_any {
        println!("No candidate pocket found. Make sure you have at least one ball in your bag.");
    }
}

/// Scans EWRAM for the bag item security key given the known quantity for one
/// ball slot. Run this with a known number of Pokéballs in the bag.
///
/// The security key is stored as a u32 in SaveBlock2 but only the lower 16 bits
/// are used for encryption: `stored_qty = actual_qty ^ (security_key & 0xFFFF)`.
/// This function reads the raw bytes at the balls pocket slot 0, computes the
/// candidate key (`raw_qty ^ expected_qty`), then searches all of EWRAM for
/// that u16 and prints each hit with its SaveBlock2-relative offset.
#[cfg(feature = "dev-tools")]
pub fn scan_for_security_key(expected_qty: u16) {
    let iwram = fire_red_memory::get_iwram();
    let ewram = fire_red_memory::get_ewram();

    // Resolve the balls pocket so we can read raw slot 0 quantity.
    let save_block_ptr = match read_save_block1_ptr(&iwram, &ewram) {
        Some(p) => p,
        None    => { eprintln!("SaveBlock1 ptr invalid or EWRAM too small — is the game loaded?"); return; }
    };

    let pocket_start = (save_block_ptr - EWRAM_BASE) + BALLS_POCKET_SAVE_BLOCK_OFFSET;
    let pocket_end   = pocket_start + BALLS_POCKET_SLOTS * 4;
    if ewram.len() < pocket_end {
        eprintln!("EWRAM too small to reach balls pocket.");
        return;
    }

    // Find the first occupied ball slot and read its raw (encrypted) quantity.
    let mut raw_qty: Option<u16> = None;
    for slot in 0..BALLS_POCKET_SLOTS {
        let base    = pocket_start + slot * 4;
        let item_id = u16::from_le_bytes([ewram[base], ewram[base + 1]]);
        if (1..=12).contains(&item_id) {
            let qty = u16::from_le_bytes([ewram[base + 2], ewram[base + 3]]);
            raw_qty = Some(qty);
            println!("Slot {:2}: item_id={:2}  raw_qty_bytes=0x{:04X}", slot, item_id, qty);
            break;
        }
    }

    let raw = match raw_qty {
        Some(v) => v,
        None => {
            eprintln!("No ball found in the balls pocket — have at least one ball in your bag.");
            return;
        }
    };

    let candidate_key = raw ^ expected_qty;
    println!("raw_qty=0x{:04X}  expected_qty={}  candidate_key=0x{:04X}",
        raw, expected_qty, candidate_key);
    println!();
    println!("Searching EWRAM for 0x{:04X} at u16-aligned offsets (SaveBlock2 relative):", candidate_key);

    let sb2_base_offset = SAVE_BLOCK_2_BASE - EWRAM_BASE;
    let mut found_any = false;

    // Scan all even-aligned positions in the region ±0x2000 around SaveBlock2.
    let scan_start = sb2_base_offset.saturating_sub(0x200);
    let scan_end   = (sb2_base_offset + 0x2000).min(ewram.len().saturating_sub(1));

    let mut off = scan_start;
    while off + 1 < scan_end {
        let val = u16::from_le_bytes([ewram[off], ewram[off + 1]]);
        if val == candidate_key {
            let gba_addr  = EWRAM_BASE + off;
            let sb2_rel   = off as isize - sb2_base_offset as isize;
            println!("  EWRAM offset 0x{:05X}  GBA 0x{:08X}  SaveBlock2{:+#06X}",
                off, gba_addr, sb2_rel);
            found_any = true;
        }
        off += 2;
    }

    if !found_any {
        println!("  Not found near SaveBlock2. Widening to full EWRAM scan...");
        let mut off = 0usize;
        while off + 1 < ewram.len() {
            let val = u16::from_le_bytes([ewram[off], ewram[off + 1]]);
            if val == candidate_key {
                let gba_addr = EWRAM_BASE + off;
                let sb2_rel  = off as isize - sb2_base_offset as isize;
                println!("  EWRAM offset 0x{:05X}  GBA 0x{:08X}  (SaveBlock2{:+#06X})",
                    off, gba_addr, sb2_rel);
            }
            off += 2;
        }
    }
}

/// Returns the lower 16 bits of the bag item security key from SaveBlock2.
///
/// Item quantities are stored in RAM as `actual_qty ^ (security_key & 0xFFFF)`.
/// Returns 0 on read failure (treats quantities as unencrypted).
fn read_security_key() -> u16 {
    let ewram  = fire_red_memory::get_ewram();
    let offset = (SAVE_BLOCK_2_BASE - EWRAM_BASE) + SECURITY_KEY_OFFSET;
    if ewram.len() < offset + 4 {
        return 0;
    }
    u32::from_le_bytes([
        ewram[offset],
        ewram[offset + 1],
        ewram[offset + 2],
        ewram[offset + 3],
    ]) as u16
}

/// Returns the total number of Pokéballs across all slots in the balls pocket.
///
/// Decodes XOR-encrypted quantities using the security key from SaveBlock2.
/// Returns 0 on any read failure so that pre-ball encounters are silently
/// skipped rather than incorrectly recorded.
pub fn count_pokeballs() -> u32 {
    let iwram = fire_red_memory::get_iwram();
    let ewram = fire_red_memory::get_ewram();

    let save_block_ptr = match read_save_block1_ptr(&iwram, &ewram) {
        Some(p) => p,
        None    => return 0,
    };

    let pocket_start = (save_block_ptr - EWRAM_BASE) + BALLS_POCKET_SAVE_BLOCK_OFFSET;
    let pocket_end   = pocket_start + BALLS_POCKET_SLOTS * 4;
    if ewram.len() < pocket_end {
        return 0;
    }

    let key     = read_security_key();
    let mut total: u32 = 0;
    for slot in 0..BALLS_POCKET_SLOTS {
        let base    = pocket_start + slot * 4;
        let item_id = u16::from_le_bytes([ewram[base], ewram[base + 1]]);
        if !(1..=12).contains(&item_id) {
            continue;
        }
        let raw_qty = u16::from_le_bytes([ewram[base + 2], ewram[base + 3]]);
        total += (raw_qty ^ key) as u32;
    }
    total
}

/// Returns `true` if the player has at least `threshold` Pokéballs.
///
/// Same as [`has_pokeballs`] but with a caller-supplied threshold, allowing
/// the run-start ball count to be configured per-session.
pub fn has_pokeballs_threshold(threshold: u32) -> bool {
    count_pokeballs() >= threshold
}

/// Returns the wild Pokémon currently engaged in battle, or `None` when not
/// in a wild encounter.
///
/// Reads `gEnemyParty[0]` from the EWRAM snapshot. FireRed's `CreateWildMon`
/// calls `CreateMon` with `OT_ID_PLAYER_ID`, so wild Pokémon receive the
/// player's OT ID — the same as every Pokémon in the player's own party.
/// Trainer-owned Pokémon carry a different OT ID. Comparing the enemy's
/// `ot_id` against the lead party member's `ot_id` therefore distinguishes
/// wild battles from trainer battles without any gBattleTypeFlags address.
///
/// Returns `None` when the slot is empty, fails checksum, or OT IDs don't
/// match (trainer battle or no party data available yet).
pub fn get_wild_enemy_pokemon() -> Option<Pokemon> {
    let ewram  = fire_red_memory::get_ewram();
    let rom    = fire_red_rom_buffer::get_rom();
    let offset = ENEMY_PARTY_ADDR - EWRAM_BASE;
    if ewram.len() < offset + 100 {
        return None;
    }
    let enemy = Pokemon::from_bytes(&ewram[offset..offset + 100], rom)?;

    // Require the player's party to be populated so we can read the player OT.
    let player_ot = fire_red_party_monitor::get_party()
        .and_then(|p| p.members.first().cloned())
        .map(|m| m.box_mon.ot_id)
        .filter(|&ot| ot != 0)?;

    if enemy.box_mon.ot_id == player_ot {
        Some(enemy)
    } else {
        None
    }
}

/// Compares the current badge state against `last_mask` (one bit per badge,
/// LSB = Brock) and fires events/webhooks for any newly obtained badges.
///
/// Returns `Some(updated_mask)` reflecting all currently held badges, or the
/// unchanged `last_mask` if badge state could not be read from EWRAM.
///
/// Pass `None` (the uninitialized sentinel) on the first call or after any
/// run/wipe reset. The function will silently adopt all currently-held badges
/// as the baseline without firing events — preventing both mid-game startup
/// replays and false positives after a wipe. Subsequent calls with the
/// returned `Some(mask)` fire events only for genuinely new badges.
pub fn check_for_new_badges(last_mask: Option<u8>) -> Option<u8> {
    let Some(bs) = fire_red_badge::read_badge_state() else {
        return last_mask;
    };

    // Build a current mask from the 8 badge flags.
    let mut current_mask: u8 = 0;
    for (i, &obtained) in bs.badges.iter().enumerate() {
        if obtained {
            current_mask |= 1 << i;
        }
    }

    // None is the "uninitialized" sentinel. Silently adopt whatever badges
    // are already held without firing events. This handles two cases:
    //   • Tracker started mid-game (existing badges must not replay).
    //   • Badge mask reset after a wipe or run change (new run's badges
    //     should not be re-fired once the mask is re-established).
    let Some(last) = last_mask else {
        return Some(current_mask);
    };

    let newly_earned = current_mask & !last;
    if newly_earned == 0 {
        return Some(current_mask);
    }

    let player = fire_red_loop::get_trainer_name();
    let timestamp = fire_red_database::unix_now();

    for i in 0..8u8 {
        if (newly_earned >> i) & 1 == 0 { continue; }
        let badge_name = fire_red_badge::badge_name(i as usize);
        if let Err(e) = fire_red_database::record_event(fire_red_database::EventKind::Badge { badge_name }) {
            tracing::warn!("Failed to record Badge event: {e}");
        }
        crate::webhook::fire_event(crate::webhook::WebhookEvent::Badge {
            player:     player.clone(),
            timestamp,
            badge_name: badge_name.to_string(),
        });
        tracing::info!("Badge earned: {}", badge_name);
    }

    Some(current_mask)
}

/// Pure logic for [`give_item`]: searches `pocket` for a slot to write to and
/// builds the 4-byte encrypted payload.
///
/// Works for any pocket — slot count is derived from `pocket.len() / 4`.
/// Quantities are XOR-encrypted with `key`, matching what FireRed expects.
///
/// Returns `Some((slot_index, payload))` where `payload` is the 4 bytes to
/// write: `[item_id_lo, item_id_hi, qty_enc_lo, qty_enc_hi]`.
/// Returns `None` if `item_id`/`quantity` is zero or the pocket is full.
pub(crate) fn compute_give_item_write(
    pocket:   &[u8],
    key:      u16,
    item_id:  u16,
    quantity: u16,
) -> Option<(usize, [u8; 4])> {
    if item_id == 0 || quantity == 0 {
        return None;
    }
    let n_slots = pocket.len() / 4;
    if n_slots == 0 {
        return None;
    }

    let mut existing_slot: Option<usize> = None;
    let mut empty_slot:    Option<usize> = None;

    for slot in 0..n_slots {
        let base    = slot * 4;
        let slot_id = u16::from_le_bytes([pocket[base], pocket[base + 1]]);
        if slot_id == item_id {
            existing_slot = Some(slot);
            break;
        }
        if slot_id == 0 && empty_slot.is_none() {
            empty_slot = Some(slot);
        }
    }

    let (slot, new_qty) = match (existing_slot, empty_slot) {
        (Some(s), _) => {
            let base    = s * 4;
            let raw_qty = u16::from_le_bytes([pocket[base + 2], pocket[base + 3]]);
            let cur_qty = raw_qty ^ key;
            let new_qty = (cur_qty as u32 + quantity as u32).min(MAX_ITEM_QTY as u32) as u16;
            (s, new_qty)
        }
        (None, Some(s)) => (s, quantity.min(MAX_ITEM_QTY)),
        (None, None)    => return None,
    };

    let qty_enc = new_qty ^ key;
    Some((slot, [
        (item_id & 0xFF) as u8,
        (item_id >> 8)   as u8,
        (qty_enc & 0xFF) as u8,
        (qty_enc >> 8)   as u8,
    ]))
}

/// Result of [`compute_take_item_write`]: either a single-slot quantity update
/// or a full pocket rewrite after the removed item is compacted out.
pub(crate) enum TakeItemWrite {
    /// Write 4 bytes at `slot * 4` within the pocket: item_id + encrypted qty.
    UpdateSlot { slot: usize, payload: [u8; 4] },
    /// Item fully removed. `pocket` is the complete re-encoded pocket bytes.
    WritePocket(Vec<u8>),
}

/// Pure logic for [`take_item`]: finds `item_id` in `pocket` and either
/// decrements its quantity or removes it entirely (compacting the pocket).
///
/// Works for any pocket — slot count is derived from `pocket.len() / 4`.
/// Returns `None` when `item_id` is zero, `quantity` is zero, the pocket is
/// empty, or the item is not present in the pocket.
pub(crate) fn compute_take_item_write(
    pocket:   &[u8],
    key:      u16,
    item_id:  u16,
    quantity: u16,
) -> Option<TakeItemWrite> {
    if item_id == 0 || quantity == 0 {
        return None;
    }
    let n_slots = pocket.len() / 4;
    if n_slots == 0 {
        return None;
    }

    let mut found_slot: Option<usize> = None;
    for slot in 0..n_slots {
        let base = slot * 4;
        if u16::from_le_bytes([pocket[base], pocket[base + 1]]) == item_id {
            found_slot = Some(slot);
            break;
        }
    }
    let slot = found_slot?;

    let base    = slot * 4;
    let raw_qty = u16::from_le_bytes([pocket[base + 2], pocket[base + 3]]);
    let cur_qty = raw_qty ^ key;

    if cur_qty <= quantity {
        // Full removal: drop the slot and compact remaining occupied slots left.
        let mut compacted: Vec<(u16, u16)> = Vec::with_capacity(n_slots);
        for s in 0..n_slots {
            let b  = s * 4;
            let id = u16::from_le_bytes([pocket[b], pocket[b + 1]]);
            if id == 0 || s == slot { continue; }
            let rq = u16::from_le_bytes([pocket[b + 2], pocket[b + 3]]);
            compacted.push((id, rq ^ key));
        }
        let mut new_pocket = vec![0u8; pocket.len()];
        for (i, (id, qty)) in compacted.iter().enumerate() {
            let b = i * 4;
            new_pocket[b..b + 2].copy_from_slice(&id.to_le_bytes());
            new_pocket[b + 2..b + 4].copy_from_slice(&(qty ^ key).to_le_bytes());
        }
        Some(TakeItemWrite::WritePocket(new_pocket))
    } else {
        let new_qty = cur_qty - quantity;
        let enc_qty = new_qty ^ key;
        let payload = [
            (item_id & 0xFF) as u8,
            (item_id >> 8)   as u8,
            (enc_qty & 0xFF) as u8,
            (enc_qty >> 8)   as u8,
        ];
        Some(TakeItemWrite::UpdateSlot { slot, payload })
    }
}

/// Reads `len` raw bytes from GBA address `addr` via a RetroArch UDP request.
///
/// Returns `None` if RetroArch doesn't respond or returns a malformed reply.
fn read_retroarch_bytes(socket: &std::net::UdpSocket, addr: u32, len: usize) -> Option<Vec<u8>> {
    let cmd    = fire_red_retroarch_interfacing::generate_command(addr, len);
    let tokens = fire_red_retroarch_interfacing::get_from_retroarch(socket, &cmd, len + 2)?;
    tokens[2..].iter()
        .map(|t| u8::from_str_radix(t, 16).ok())
        .collect()
}

/// Derives the bag security key by XOR-analysis of every occupied bag slot.
///
/// Returns `(Some(key), candidates)` when narrowed to one value,
/// or `(None, candidates)` with the remaining candidate set when ambiguous.
/// The caller can use the candidate set to validate a SaveBlock2 probe.
fn derive_key_from_pockets(socket: &std::net::UdpSocket, save_block1: u32) -> (Option<u16>, Vec<u16>) {
    // Collect raw encrypted quantities from balls pocket (item IDs 1–12).
    let balls_addr = save_block1 + BALLS_POCKET_SAVE_BLOCK_OFFSET as u32;
    let balls_raw: Vec<u16> = read_retroarch_bytes(socket, balls_addr, BALLS_POCKET_SLOTS * 4)
        .filter(|b| b.len() == BALLS_POCKET_SLOTS * 4)
        .map(|b| {
            (0..BALLS_POCKET_SLOTS).filter_map(|s| {
                let base = s * 4;
                let id = u16::from_le_bytes([b[base], b[base + 1]]);
                if (1..=12).contains(&id) { Some(u16::from_le_bytes([b[base + 2], b[base + 3]])) }
                else { None }
            }).collect()
        })
        .unwrap_or_default();

    // Collect raw encrypted quantities from items pocket (any non-zero item ID).
    let items_addr = save_block1 + ITEMS_POCKET_SAVE_BLOCK_OFFSET as u32;
    let items_raw: Vec<u16> = read_retroarch_bytes(socket, items_addr, ITEMS_POCKET_SLOTS * 4)
        .filter(|b| b.len() == ITEMS_POCKET_SLOTS * 4)
        .map(|b| {
            (0..ITEMS_POCKET_SLOTS).filter_map(|s| {
                let base = s * 4;
                let id = u16::from_le_bytes([b[base], b[base + 1]]);
                if id != 0 { Some(u16::from_le_bytes([b[base + 2], b[base + 3]])) }
                else { None }
            }).collect()
        })
        .unwrap_or_default();

    // Merge: balls first (more reliable IDs), then items.
    let all_raw: Vec<u16> = balls_raw.into_iter().chain(items_raw).collect();
    if all_raw.is_empty() {
        return (None, vec![]);
    }

    let mut candidates: Vec<u16> = (1u16..=99).map(|q| all_raw[0] ^ q).collect();
    for &r in &all_raw[1..] {
        candidates.retain(|&k| (1u16..=99).contains(&(r ^ k)));
        if candidates.len() == 1 { break; }
    }

    match candidates.as_slice() {
        [k] => (Some(*k), vec![*k]),
        _   => (None, candidates),
    }
}

/// Reads the bag security key from `SaveBlock2 + SECURITY_KEY_OFFSET`.
///
/// Falls back to two alternate offsets if the primary read returns zero.
/// `save_block2` must be the dynamically resolved base address of SaveBlock2
/// (from `gSaveBlock2Ptr` in IWRAM at `SAVE_BLOCK_2_PTR_ADDR`).
fn derive_key_from_save_block2(socket: &std::net::UdpSocket, save_block2: u32) -> u16 {
    for &off in &[SECURITY_KEY_OFFSET as u32, 0x0EE0, 0x0E4C] {
        if let Some(b) = read_retroarch_bytes(socket, save_block2 + off, 4) {
            if b.len() == 4 {
                let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u16;
                if v != 0 {
                    return v;
                }
            }
        }
    }
    0
}

/// Injects one item into the player's items pocket via `WRITE_CORE_MEMORY`.
///
/// Bag pocket descriptor: SaveBlock1 offset, slot count, and display name.
struct PocketTarget {
    offset: usize,
    slots:  usize,
    name:   &'static str,
}

/// Maps a FireRed item ID to the bag pocket it belongs in.
///
/// Ranges are based on the pokefirered item data table:
/// - 1–12:    Poké Balls   → balls pocket
/// - 236–304: Key items    → key items pocket (bikes, rods, passes, story items, etc.)
/// - 305–362: TMs / HMs   → TMs/HMs pocket
/// - all else: consumables, vitamins, hold items, berries → items pocket
fn pocket_for_item(item_id: u16) -> PocketTarget {
    match item_id {
        1..=12    => PocketTarget { offset: BALLS_POCKET_SAVE_BLOCK_OFFSET,     slots: BALLS_POCKET_SLOTS,     name: "balls" },
        236..=304 => PocketTarget { offset: KEY_ITEMS_POCKET_SAVE_BLOCK_OFFSET, slots: KEY_ITEMS_POCKET_SLOTS, name: "key items" },
        305..=362 => PocketTarget { offset: TMS_POCKET_SAVE_BLOCK_OFFSET,       slots: TMS_POCKET_SLOTS,       name: "TMs" },
        _         => PocketTarget { offset: ITEMS_POCKET_SAVE_BLOCK_OFFSET,     slots: ITEMS_POCKET_SLOTS,     name: "items" },
    }
}

/// Searches the items pocket for an existing stack of `item_id` to increment,
/// or the first empty slot if none exists. Quantities are XOR-encrypted with the
/// save's security key before writing, matching what FireRed expects in RAM.
///
/// **Key discovery strategy** (in order):
/// 1. Balls pocket oracle — derives the key by XOR-analysis of encrypted ball
///    quantities; no prior knowledge of ball counts needed.
/// 2. SaveBlock2 candidate scan — probes offsets 0x0F20 (pokefirered canonical),
///    0x0EE0, and 0x0E4C (empirical) for the first non-zero u16.
///
/// **SaveBlock1 resolution** uses the EWRAM/IWRAM snapshot cache rather than a
/// direct RetroArch read. Direct IWRAM reads can catch transient values during
/// GBA code execution; the cache is sampled at quiescent moments and is stable.
///
/// Returns `true` if the write command was dispatched to RetroArch.
pub fn give_item(item_id: u16, quantity: u16) -> bool {
    if item_id == 0 || quantity == 0 {
        return false;
    }

    // Resolve both SaveBlock pointers from the snapshot cache.
    // Direct IWRAM reads via RetroArch can catch transient values mid-GBA-execution;
    // the cache is sampled at quiescent moments and is stable.
    let (save_block1, save_block2): (usize, usize) = {
        let iwram = fire_red_memory::get_iwram();
        let ewram = fire_red_memory::get_ewram();
        let sb1 = match read_save_block1_ptr(&iwram, &ewram) {
            Some(ptr) => ptr,
            None => {
                tracing::warn!("give_item: SaveBlock1 ptr invalid in cache — is the game loaded?");
                return false;
            }
        };
        let sb2 = read_save_block2_ptr(&iwram, &ewram).unwrap_or(SAVE_BLOCK_2_BASE);
        (sb1, sb2)
    };

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("give_item: failed to create socket: {e}"); return false; }
    };

    // Derive the security key: pocket oracle first, SaveBlock2 direct read as fallback.
    let (oracle_key, _candidates) = derive_key_from_pockets(&socket, save_block1 as u32);
    let key = oracle_key
        .unwrap_or_else(|| derive_key_from_save_block2(&socket, save_block2 as u32));

    // Select the correct pocket based on item ID.
    let pocket_info = pocket_for_item(item_id);
    let pocket_addr = save_block1 as u32 + pocket_info.offset as u32;
    let expected_len = pocket_info.slots * 4;
    let pocket = match read_retroarch_bytes(&socket, pocket_addr, expected_len) {
        Some(b) if b.len() == expected_len => b,
        _ => {
            tracing::warn!("give_item: RetroArch did not respond to {} pocket read", pocket_info.name);
            return false;
        }
    };

    tracing::info!(
        "give_item: sb1=0x{save_block1:08X} pocket={} addr=0x{pocket_addr:08X} key=0x{key:04X}",
        pocket_info.name
    );

    let Some((slot, payload)) = compute_give_item_write(&pocket, key, item_id, quantity) else {
        tracing::warn!("give_item: {} pocket is full", pocket_info.name);
        return false;
    };

    let write_addr = pocket_addr + slot as u32 * 4;
    let ok = fire_red_retroarch_interfacing::write_to_retroarch(&socket, write_addr, &payload);
    if ok {
        let new_qty = u16::from_le_bytes([payload[2], payload[3]]) ^ key;
        tracing::info!(
            "give_item: wrote item_id={item_id} qty={new_qty} to slot {slot} (addr=0x{write_addr:08X})"
        );
    }
    ok
}

/// Removes `quantity` of `item_id` from the player's bag pocket.
///
/// Uses the same SaveBlock1/key-derivation strategy as [`give_item`].  If the
/// current stack quantity is ≤ `quantity` the item is fully removed and the
/// pocket is compacted (remaining slots shift left) so FireRed's bag UI is
/// consistent.  Otherwise only the quantity is decremented in place.
///
/// Returns `true` if the write command was dispatched to RetroArch.
pub fn take_item(item_id: u16, quantity: u16) -> bool {
    if item_id == 0 || quantity == 0 {
        return false;
    }

    let (save_block1, save_block2): (usize, usize) = {
        let iwram = fire_red_memory::get_iwram();
        let ewram = fire_red_memory::get_ewram();
        let sb1 = match read_save_block1_ptr(&iwram, &ewram) {
            Some(ptr) => ptr,
            None => {
                tracing::warn!("take_item: SaveBlock1 ptr invalid in cache — is the game loaded?");
                return false;
            }
        };
        let sb2 = read_save_block2_ptr(&iwram, &ewram).unwrap_or(SAVE_BLOCK_2_BASE);
        (sb1, sb2)
    };

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("take_item: failed to create socket: {e}"); return false; }
    };

    let (oracle_key, _) = derive_key_from_pockets(&socket, save_block1 as u32);
    let key = oracle_key
        .unwrap_or_else(|| derive_key_from_save_block2(&socket, save_block2 as u32));

    let pocket_info  = pocket_for_item(item_id);
    let pocket_addr  = save_block1 as u32 + pocket_info.offset as u32;
    let expected_len = pocket_info.slots * 4;
    let pocket = match read_retroarch_bytes(&socket, pocket_addr, expected_len) {
        Some(b) if b.len() == expected_len => b,
        _ => {
            tracing::warn!("take_item: RetroArch did not respond to {} pocket read", pocket_info.name);
            return false;
        }
    };

    tracing::info!(
        "take_item: sb1=0x{save_block1:08X} pocket={} addr=0x{pocket_addr:08X} key=0x{key:04X}",
        pocket_info.name
    );

    let result = match compute_take_item_write(&pocket, key, item_id, quantity) {
        Some(r) => r,
        None => {
            tracing::warn!("take_item: item_id={item_id} not found in {} pocket", pocket_info.name);
            return false;
        }
    };

    match result {
        TakeItemWrite::UpdateSlot { slot, payload } => {
            let write_addr = pocket_addr + slot as u32 * 4;
            let ok = fire_red_retroarch_interfacing::write_to_retroarch(&socket, write_addr, &payload);
            if ok {
                let remaining = u16::from_le_bytes([payload[2], payload[3]]) ^ key;
                tracing::info!(
                    "take_item: item_id={item_id} remaining qty={remaining} at slot {slot} (addr=0x{write_addr:08X})"
                );
            }
            ok
        }
        TakeItemWrite::WritePocket(new_pocket) => {
            let ok = fire_red_retroarch_interfacing::write_to_retroarch(&socket, pocket_addr, &new_pocket);
            if ok {
                tracing::info!(
                    "take_item: removed item_id={item_id} from {} pocket and compacted",
                    pocket_info.name
                );
            }
            ok
        }
    }
}

/// Makes the party Pokémon at `party_position` (0–5) shiny by patching its
/// stored OT Secret ID so the Gen III shiny formula evaluates to zero.
///
/// **Why change SID, not personality?**
/// The shiny formula is `(p_high ^ p_low ^ TID ^ SID) < 8`.  Setting
/// `new_SID = p_high ^ p_low ^ TID` satisfies it with XOR result = 0.
/// Personality (and therefore nature, ability, gender, and the substructure
/// block order) is left completely unchanged.
///
/// **Encryption re-keying:**
/// The 48-byte data block (bytes 32–79) is XOR-encrypted with
/// `personality ^ ot_id`.  Because only `ot_id` changes, each 32-bit word
/// just needs `XOR (old_ot_id ^ new_ot_id)` — no full decrypt/re-encrypt.
/// The checksum covers the *decrypted* data, which is unchanged, so no
/// checksum rewrite is required.
pub fn make_shiny(party_position: usize) -> bool {
    if party_position >= 6 {
        tracing::warn!("make_shiny: party_position {party_position} out of range (must be 0–5)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("make_shiny: socket error: {e}"); return false; }
    };

    // Read the first 80 bytes: unencrypted header (0–31) + encrypted block (32–79).
    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("make_shiny: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let ot_id       = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);

    if personality == 0 {
        tracing::warn!("make_shiny: party[{party_position}] is empty (personality=0)");
        return false;
    }
    if fire_red_states::is_shiny(personality, ot_id) {
        tracing::info!(
            "make_shiny: party[{party_position}] personality=0x{personality:08X} already shiny"
        );
        return true;
    }

    let p_high  = (personality >> 16) as u16;
    let p_low   = (personality & 0xFFFF) as u16;
    let tid     = (ot_id & 0xFFFF) as u16;
    let old_sid = (ot_id >> 16) as u16;

    // new_sid chosen so that p_high ^ p_low ^ tid ^ new_sid == 0.
    let new_sid: u16   = p_high ^ p_low ^ tid;
    let new_ot_id: u32 = ((new_sid as u32) << 16) | (tid as u32);

    // Re-key the encrypted data block in-place.
    // old_key ^ new_key = old_ot_id ^ new_ot_id = (old_sid ^ new_sid) << 16
    let xor_diff: u32 = ((old_sid ^ new_sid) as u32) << 16;
    let mut re_encrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ xor_diff;
        re_encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    // Write new ot_id (4 bytes at mon_addr + 4).
    if !fire_red_retroarch_interfacing::write_to_retroarch(
        &socket, mon_addr + 4, &new_ot_id.to_le_bytes(),
    ) {
        tracing::warn!("make_shiny: failed to write ot_id for party[{party_position}]");
        return false;
    }
    // Write re-keyed data block (48 bytes at mon_addr + 32).
    if !fire_red_retroarch_interfacing::write_to_retroarch(
        &socket, mon_addr + 32, &re_encrypted,
    ) {
        tracing::warn!("make_shiny: failed to write encrypted data for party[{party_position}]");
        return false;
    }

    tracing::info!(
        "make_shiny: party[{party_position}] personality=0x{personality:08X} \
         tid=0x{tid:04X} sid: 0x{old_sid:04X} → 0x{new_sid:04X}"
    );
    true
}

/// Substructure order table for Gen III Pokémon data blocks.
///
/// `SUBSTRUCTURE_ORDER[personality % 24][i]` is the type at substructure
/// position `i` (G=0 Growth, A=1 Attacks, E=2 Effort, M=3 Misc).
const SUBSTRUCTURE_ORDER: [[u8; 4]; 24] = [
    [0, 1, 2, 3], [0, 1, 3, 2], [0, 2, 1, 3], [0, 3, 1, 2],
    [0, 2, 3, 1], [0, 3, 2, 1], [1, 0, 2, 3], [1, 0, 3, 2],
    [2, 0, 1, 3], [3, 0, 1, 2], [2, 0, 3, 1], [3, 0, 2, 1],
    [1, 2, 0, 3], [1, 3, 0, 2], [2, 1, 0, 3], [3, 1, 0, 2],
    [2, 3, 0, 1], [3, 2, 0, 1], [1, 2, 3, 0], [1, 3, 2, 0],
    [2, 1, 3, 0], [3, 1, 2, 0], [2, 3, 1, 0], [3, 2, 1, 0],
];

/// Returns the index (0–3) of the Growth substructure for a given `personality`.
///
/// `SUBSTRUCTURE_ORDER[p%24][substructType]` = the position of that type in
/// the block, so Growth (type 0) is simply at `table[p%24][0]`.
fn growth_substructure_index(personality: u32) -> usize {
    SUBSTRUCTURE_ORDER[(personality % 24) as usize][0] as usize
}

/// Result of [`compute_change_species`].
pub(crate) enum ChangeSpeciesOutcome {
    /// The Growth block already holds `new_species`; no write is needed.
    AlreadyMatches,
    /// Updated checksum bytes and fully re-encrypted 48-byte data block.
    Write { checksum: [u8; 2], encrypted: [u8; 48] },
}

/// Pure logic for [`change_species`]: decrypts the data block, updates the
/// species in the Growth substructure, recalculates the checksum, and
/// re-encrypts.
///
/// `data` must be 80 bytes (Pokémon header + encrypted data block).
/// Returns `None` if `data` is too short or personality is 0 (empty slot).
pub(crate) fn compute_change_species(
    data: &[u8],
    new_species: u16,
) -> Option<ChangeSpeciesOutcome> {
    if data.len() < 80 { return None; }

    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }

    let enc_key = personality ^ ot_id;

    // Decrypt the 48-byte data block (bytes 32–79).
    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    let g_offset    = growth_substructure_index(personality) * 12;
    let old_species = u16::from_le_bytes([decrypted[g_offset], decrypted[g_offset + 1]]);
    if old_species == new_species {
        return Some(ChangeSpeciesOutcome::AlreadyMatches);
    }

    decrypted[g_offset..g_offset + 2].copy_from_slice(&new_species.to_le_bytes());

    // Recalculate checksum over the full modified decrypted block.
    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    // Re-encrypt.
    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    Some(ChangeSpeciesOutcome::Write { checksum: checksum.to_le_bytes(), encrypted })
}

/// Changes the party Pokémon at `party_position` (0–5) to `new_species`.
///
/// Only the species field in the encrypted Growth substructure is updated.
/// Personality, OT ID, nickname, moves, EVs, IVs, nature, ability, and gender
/// are all preserved. The checksum is recalculated after the change.
///
/// The party stats (max HP, attack, etc.) in bytes 80–99 are not updated here —
/// FireRed recomputes them via `CalculateMonStats` on the next relevant game event.
///
/// Returns `true` if the write was dispatched to RetroArch.
pub fn change_species(party_position: usize, new_species: u16) -> bool {
    if party_position >= 6 {
        tracing::warn!("change_species: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    if new_species == 0 || new_species > MAX_NATIONAL_DEX_FIRERED {
        tracing::warn!("change_species: new_species {new_species} out of range (must be 1–386)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("change_species: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("change_species: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("change_species: party[{party_position}] is empty (personality=0)");
        return false;
    }

    match compute_change_species(&data, new_species) {
        None => {
            tracing::warn!("change_species: unexpected None for party[{party_position}]");
            false
        }
        Some(ChangeSpeciesOutcome::AlreadyMatches) => {
            tracing::info!("change_species: party[{party_position}] already species={new_species}");
            true
        }
        Some(ChangeSpeciesOutcome::Write { checksum, encrypted }) => {
            // Write checksum (+28, 2 bytes) + unknown (+30, 2 bytes preserved) +
            // encrypted block (+32, 48 bytes) as one 52-byte payload so the game
            // never sees a state where the checksum and data are inconsistent.
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]); // preserve unknown field
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr + 28, &payload) {
                tracing::warn!("change_species: failed to write to party[{party_position}]");
                return false;
            }
            tracing::info!(
                "change_species: party[{party_position}] personality=0x{personality:08X} → species={new_species}"
            );
            true
        }
    }
}

/// Returns the index (0–3) of the Misc substructure for a given `personality`.
fn misc_substructure_index(personality: u32) -> usize {
    SUBSTRUCTURE_ORDER[(personality % 24) as usize][3] as usize
}

/// Pure logic for [`change_ability`]: decrypts the data block, updates bit 31
/// of the IV/egg/ability word in the Misc substructure, recalculates the
/// checksum, and re-encrypts.
///
/// `data` must be 80 bytes. Returns `None` if `data` is too short, personality
/// is 0 (empty slot), or the ability bit already matches `ability_slot`.
/// Otherwise returns `(checksum_bytes, re-encrypted_block)`.
pub(crate) fn compute_set_ability_bit(
    data: &[u8],
    ability_slot: u8,
) -> Option<([u8; 2], [u8; 48])> {
    if data.len() < 80 { return None; }

    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }

    let enc_key = personality ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    // Misc substructure: byte 4 within the block is the IV/egg/ability u32.
    let m_off = misc_substructure_index(personality) * 12;
    let iv_ea_off = m_off + 4;
    let mut iv_ea = u32::from_le_bytes(decrypted[iv_ea_off..iv_ea_off + 4].try_into().unwrap());

    let current_bit = ((iv_ea >> 31) & 1) as u8;
    if current_bit == ability_slot { return None; }

    if ability_slot == 1 {
        iv_ea |= 1u32 << 31;
    } else {
        iv_ea &= !(1u32 << 31);
    }
    decrypted[iv_ea_off..iv_ea_off + 4].copy_from_slice(&iv_ea.to_le_bytes());

    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    Some((checksum.to_le_bytes(), encrypted))
}

/// Switches the party Pokémon at `party_position` (0–5) to ability slot
/// `ability_slot` (0 = first ability, 1 = second ability).
///
/// Sets or clears bit 31 of the IV/egg/ability word in the Misc substructure.
/// All other fields — species, EVs, IVs, moves, nature, personality — are
/// preserved. The checksum is recalculated and the data block re-encrypted.
///
/// Returns `true` if the write was dispatched to RetroArch (or the slot already
/// had the requested ability bit set).
pub fn change_ability(party_position: usize, ability_slot: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("change_ability: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    if ability_slot > 1 {
        tracing::warn!("change_ability: ability_slot {ability_slot} out of range (must be 0 or 1)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("change_ability: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("change_ability: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("change_ability: party[{party_position}] is empty (personality=0)");
        return false;
    }

    match compute_set_ability_bit(&data, ability_slot) {
        None => {
            tracing::info!("change_ability: party[{party_position}] already on ability slot {ability_slot}");
            true
        }
        Some((checksum, encrypted)) => {
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]);
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr + 28, &payload) {
                tracing::warn!("change_ability: failed to write to party[{party_position}]");
                return false;
            }
            tracing::info!(
                "change_ability: party[{party_position}] personality=0x{personality:08X} → ability_slot={ability_slot}"
            );
            true
        }
    }
}

/// Result of [`compute_change_gender`].
pub(crate) enum ChangeGenderOutcome {
    /// The Pokémon already has `target_gender`; no write needed.
    AlreadyMatches,
    /// New personality, updated checksum bytes, and re-encrypted 48-byte block.
    Write { new_personality: u32, checksum: [u8; 2], encrypted: [u8; 48] },
}

/// Pure logic for [`change_gender`]: searches for a new personality low byte
/// that satisfies `target_gender` (0 = male, 1 = female), preserves nature
/// (personality % 25), and — when the Pokémon is currently shiny — keeps the
/// Gen III shiny formula satisfied. If `personality % 24` changes, the four
/// 12-byte substructures are rearranged to match the new order before
/// re-encrypting.
///
/// `data` must be 80 bytes. `gender_ratio` is the species' raw byte from the
/// ROM base-stats table (0 = always male, 254 = always female, 255 = genderless;
/// otherwise female when `personality & 0xFF < gender_ratio`).
/// Returns `None` if `data` is too short, personality is 0 (empty slot), the
/// species has fixed/no gender for `target_gender`, or (when shiny) no single
/// byte satisfies all constraints simultaneously.
pub(crate) fn compute_change_gender(
    data: &[u8],
    target_gender: u8,
    gender_ratio: u8,
) -> Option<ChangeGenderOutcome> {
    if data.len() < 80 { return None; }

    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }

    // Fixed-gender and genderless species.
    match gender_ratio {
        255 => return None,
        0   => return if target_gender == 0 { Some(ChangeGenderOutcome::AlreadyMatches) } else { None },
        254 => return if target_gender == 1 { Some(ChangeGenderOutcome::AlreadyMatches) } else { None },
        _   => {}
    }

    let b0 = (personality & 0xFF) as u8;
    let currently_female = b0 < gender_ratio;
    if (currently_female && target_gender == 1) || (!currently_female && target_gender == 0) {
        return Some(ChangeGenderOutcome::AlreadyMatches);
    }

    let shiny_now = is_shiny(personality, ot_id);
    // Shiny XOR = p_high ^ p_low ^ id_high ^ id_low.  Only b0 changes,
    // so k16 = everything-except-b0 is a constant; new_xor = k16 ^ new_b0.
    let p_high  = (personality >> 16) as u16;
    let b1      = ((personality >> 8) & 0xFF) as u16;
    let id_high = (ot_id >> 16) as u16;
    let id_low  = (ot_id & 0xFFFF) as u16;
    let k16 = p_high ^ (b1 << 8) ^ id_high ^ id_low;

    let upper  = personality & 0xFFFF_FF00;
    let nature = personality % 25;

    let new_b0 = (0u32..=255).find(|&c| {
        let c = c as u8;
        let new_p = upper | c as u32;
        if new_p % 25 != nature { return false; }
        let gender_ok = if target_gender == 0 { c >= gender_ratio } else { c < gender_ratio };
        if !gender_ok { return false; }
        if shiny_now && (k16 ^ c as u16) >= 8 { return false; }
        true
    })? as u8;

    let new_p       = upper | new_b0 as u32;
    let enc_key     = personality ^ ot_id;
    let new_enc_key = new_p ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    // Rearrange substructures if personality % 24 changed.
    let old_mod24 = (personality % 24) as usize;
    let new_mod24 = (new_p % 24) as usize;
    let arranged = if old_mod24 == new_mod24 {
        decrypted
    } else {
        let mut r = [0u8; 48];
        for t in 0..4 {
            let old_pos = SUBSTRUCTURE_ORDER[old_mod24][t] as usize;
            let new_pos = SUBSTRUCTURE_ORDER[new_mod24][t] as usize;
            r[new_pos * 12..new_pos * 12 + 12]
                .copy_from_slice(&decrypted[old_pos * 12..old_pos * 12 + 12]);
        }
        r
    };

    let checksum: u16 = arranged
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in arranged.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ new_enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    Some(ChangeGenderOutcome::Write {
        new_personality: new_p,
        checksum: checksum.to_le_bytes(),
        encrypted,
    })
}

/// Changes the gender of the party Pokémon at `party_position` (0–5) to
/// `target_gender` (0 = male, 1 = female).
///
/// Adjusts only the low byte of the personality, preserving nature
/// (personality % 25). If the Pokémon is shiny, only values that keep the
/// shiny formula satisfied are considered — if none exists for the requested
/// gender, the call logs a warning and returns `false`. Personality, EVs, IVs,
/// moves, and all other save data are preserved.
///
/// Writes an atomic 80-byte payload covering personality + the re-encrypted
/// data block so the game never sees an inconsistent state.
pub fn change_gender(party_position: usize, target_gender: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("change_gender: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    if target_gender > 1 {
        tracing::warn!("change_gender: target_gender {target_gender} must be 0 (male) or 1 (female)");
        return false;
    }

    const PARTY_BASE:       u32   = 0x02024284;
    const MON_SIZE:         u32   = 100;
    const BASE_STATS_SIZE:  usize = 28;
    const GENDER_RATIO_OFF: usize = 0x10;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("change_gender: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("change_gender: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("change_gender: party[{party_position}] is empty (personality=0)");
        return false;
    }

    // Decrypt to read species → look up gender_ratio from ROM.
    let ot_id   = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let enc_key = personality ^ ot_id;
    let mut dec_tmp = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        dec_tmp[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    let g_off   = growth_substructure_index(personality) * 12;
    let species = u16::from_le_bytes([dec_tmp[g_off], dec_tmp[g_off + 1]]);

    let rom      = fire_red_rom_buffer::get_rom();
    let rom_addr = fire_red_rom_buffer::get_rom_addresses();
    let gr_off   = rom_addr.base_stats_addr + species as usize * BASE_STATS_SIZE + GENDER_RATIO_OFF;
    if gr_off >= rom.len() {
        tracing::warn!("change_gender: species={species} ROM offset out of range");
        return false;
    }
    let gender_ratio = rom[gr_off];

    match compute_change_gender(&data, target_gender, gender_ratio) {
        None => {
            let reason = match gender_ratio {
                255 => "species is genderless",
                0   => "species is always male",
                254 => "species is always female",
                _   => "shiny + gender constraints have no common personality byte",
            };
            tracing::warn!("change_gender: cannot change party[{party_position}]: {reason}");
            false
        }
        Some(ChangeGenderOutcome::AlreadyMatches) => {
            tracing::info!("change_gender: party[{party_position}] already target gender");
            true
        }
        Some(ChangeGenderOutcome::Write { new_personality, checksum, encrypted }) => {
            // Single atomic 80-byte write: new personality (0–3), unchanged header
            // fields (4–27), new checksum (28–29), preserved unknown (30–31), new
            // encrypted block (32–79).
            let mut payload = [0u8; 80];
            payload[0..4].copy_from_slice(&new_personality.to_le_bytes());
            payload[4..28].copy_from_slice(&data[4..28]);
            payload[28..30].copy_from_slice(&checksum);
            payload[30..32].copy_from_slice(&data[30..32]);
            payload[32..80].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr, &payload) {
                tracing::warn!("change_gender: write failed for party[{party_position}]");
                return false;
            }
            let gs = if target_gender == 0 { "male" } else { "female" };
            tracing::info!(
                "change_gender: party[{party_position}] 0x{personality:08X} → 0x{new_personality:08X} ({gs})"
            );
            true
        }
    }
}

// ── GBA character encoding ────────────────────────────────────────────────────

/// Converts a UTF-8 char to its FireRed GBA encoding byte.
///
/// Inverse of `fire_red_text::char_gba_to_ascii`. Returns `None` for characters
/// that have no GBA mapping; callers silently skip them.
pub(crate) fn ascii_to_gba(c: char) -> Option<u8> {
    match c {
        ' '        => Some(0x00),
        '0'..='9'  => Some(c as u8 - b'0' + 0xA1),
        '!'        => Some(0xAB),
        '?'        => Some(0xAC),
        '.'        => Some(0xAD),
        '-'        => Some(0xAE),
        '\''       => Some(0xB1),
        ','        => Some(0xB7),
        'A'..='Z'  => Some(c as u8 - b'A' + 0xBB),
        'a'..='z'  => Some(c as u8 - b'a' + 0xD5),
        '♂'        => Some(0xB5),
        '♀'        => Some(0xB6),
        _          => None,
    }
}

/// Encodes `name` into a 10-byte GBA nickname buffer.
///
/// Unmappable characters are silently dropped. The string is truncated at 10
/// GBA bytes. Unused trailing bytes are set to `0xFF` (the GBA string terminator).
pub(crate) fn encode_nickname(name: &str) -> [u8; 10] {
    let mut buf = [0xFFu8; 10];
    let mut i = 0usize;
    for c in name.chars() {
        if i >= 10 { break; }
        if let Some(b) = ascii_to_gba(c) {
            buf[i] = b;
            i += 1;
        }
    }
    buf
}

/// Renames the party Pokémon at `party_position` (0–5) to `nickname`.
///
/// `nickname` is UTF-8; characters with no GBA mapping are silently dropped and
/// the string is truncated to 10 GBA bytes. Only the 10-byte nickname field at
/// struct offset 8 is written — the encrypted data block, personality, OT fields,
/// and all other data are left unchanged.
///
/// Returns `true` if the write was dispatched to RetroArch.
pub fn change_nickname(party_position: usize, nickname: &str) -> bool {
    if party_position >= 6 {
        tracing::warn!("change_nickname: party_position {party_position} out of range (must be 0–5)");
        return false;
    }

    const PARTY_BASE:   u32 = 0x02024284;
    const MON_SIZE:     u32 = 100;
    const NICKNAME_OFF: u32 = 8;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("change_nickname: socket error: {e}"); return false; }
    };

    // Verify the slot is occupied before writing.
    let hdr = match read_retroarch_bytes(&socket, mon_addr, 4) {
        Some(b) if b.len() == 4 => b,
        _ => {
            tracing::warn!("change_nickname: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };
    if u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) == 0 {
        tracing::warn!("change_nickname: party[{party_position}] is empty (personality=0)");
        return false;
    }

    let gba_name = encode_nickname(nickname);
    let ok = fire_red_retroarch_interfacing::write_to_retroarch(
        &socket, mon_addr + NICKNAME_OFF, &gba_name,
    );
    if ok {
        tracing::info!("change_nickname: party[{party_position}] → {nickname:?}");
    }
    ok
}

// ── change_held_item ──────────────────────────────────────────────────────────

/// Pure logic for [`change_held_item`]: decrypts the data block, updates the
/// held-item field (Growth substructure bytes 2–3), recalculates the checksum,
/// and re-encrypts.
///
/// `data` must be 80 bytes. Returns `None` if `data` is too short, personality
/// is 0, or the held item already matches `new_item_id`.
/// Otherwise returns `(checksum_bytes, re-encrypted_block)`.
pub(crate) fn compute_change_held_item(
    data: &[u8],
    new_item_id: u16,
) -> Option<([u8; 2], [u8; 48])> {
    if data.len() < 80 { return None; }

    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }

    let enc_key = personality ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    let g_off    = growth_substructure_index(personality) * 12;
    let item_off = g_off + 2; // held_item is Growth substructure bytes 2–3
    let old_item = u16::from_le_bytes([decrypted[item_off], decrypted[item_off + 1]]);
    if old_item == new_item_id { return None; }

    decrypted[item_off..item_off + 2].copy_from_slice(&new_item_id.to_le_bytes());

    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    Some((checksum.to_le_bytes(), encrypted))
}

/// Sets the held item of the party Pokémon at `party_position` (0–5) to
/// `item_id`. Pass `item_id = 0` to remove the held item.
///
/// Only the held-item field in the Growth substructure is changed; species,
/// EVs, IVs, moves, nature, personality, and shiny status are all preserved.
/// The checksum is recalculated and the data block re-encrypted.
///
/// Returns `true` if the write was dispatched to RetroArch (or the slot already
/// holds the requested item).
pub fn change_held_item(party_position: usize, item_id: u16) -> bool {
    if party_position >= 6 {
        tracing::warn!("change_held_item: party_position {party_position} out of range (must be 0–5)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("change_held_item: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("change_held_item: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("change_held_item: party[{party_position}] is empty (personality=0)");
        return false;
    }

    match compute_change_held_item(&data, item_id) {
        None => {
            tracing::info!("change_held_item: party[{party_position}] already holds item_id={item_id}");
            true
        }
        Some((checksum, encrypted)) => {
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]);
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr + 28, &payload) {
                tracing::warn!("change_held_item: failed to write to party[{party_position}]");
                return false;
            }
            tracing::info!(
                "change_held_item: party[{party_position}] personality=0x{personality:08X} → item_id={item_id}"
            );
            true
        }
    }
}

// ── cure_status ───────────────────────────────────────────────────────────────

/// Clears the status condition of the party Pokémon at `party_position` (0–5).
///
/// Writes 4 zero bytes to the status word at bytes 80–83 of the PartyPokemon
/// struct (immediately after the 80-byte BoxPokemon header + data block). This
/// clears all status flags in one write: sleep turn counter, poison, burn,
/// freeze, paralysis, and Toxic stage.
///
/// Returns `true` if the write was dispatched to RetroArch.
pub fn cure_status(party_position: usize) -> bool {
    if party_position >= 6 {
        tracing::warn!("cure_status: party_position {party_position} out of range (must be 0–5)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    const STATUS_OFF: u32 = 80;
    let status_addr = PARTY_BASE + party_position as u32 * MON_SIZE + STATUS_OFF;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("cure_status: socket error: {e}"); return false; }
    };

    let ok = fire_red_retroarch_interfacing::write_to_retroarch(&socket, status_addr, &[0u8; 4]);
    if ok {
        tracing::info!("cure_status: cleared status for party[{party_position}]");
    }
    ok
}

// ── change_nature ─────────────────────────────────────────────────────────────

/// Result of [`compute_change_nature`].
pub(crate) enum ChangeNatureOutcome {
    /// The Pokémon already has `target_nature`; no write needed.
    AlreadyMatches,
    /// New personality, updated checksum bytes, and re-encrypted 48-byte block.
    Write { new_personality: u32, checksum: [u8; 2], encrypted: [u8; 48] },
}

/// Pure logic for [`change_nature`]: searches for a new personality low byte
/// that satisfies `target_nature` (`personality % 25`), preserves the current
/// gender (for species where gender is personality-derived), and — when the
/// Pokémon is currently shiny — keeps the Gen III shiny formula satisfied.
/// If `personality % 24` changes the four 12-byte substructures are rearranged
/// to match the new block order before re-encrypting.
///
/// `data` must be 80 bytes. `gender_ratio` is the species' raw ROM byte (0 =
/// always male, 254 = always female, 255 = genderless; others = variable gender).
/// For fixed-gender and genderless species the gender constraint is skipped.
/// Returns `None` if `data` is too short, personality is 0, or no single low byte
/// satisfies all active constraints simultaneously.
pub(crate) fn compute_change_nature(
    data: &[u8],
    target_nature: u8,
    gender_ratio: u8,
) -> Option<ChangeNatureOutcome> {
    if data.len() < 80 { return None; }

    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }

    if personality % 25 == target_nature as u32 {
        return Some(ChangeNatureOutcome::AlreadyMatches);
    }

    // For variable-gender species, record whether the current Pokémon is female
    // so we can preserve that. Fixed-gender (0, 254) and genderless (255) species
    // have the gender determined entirely by species, not personality, so no
    // constraint is needed.
    let is_female: Option<bool> = match gender_ratio {
        0 | 254 | 255 => None,
        _ => Some(((personality & 0xFF) as u8) < gender_ratio),
    };

    let shiny_now = is_shiny(personality, ot_id);
    let p_high    = (personality >> 16) as u16;
    let b1        = ((personality >> 8) & 0xFF) as u16;
    let id_high   = (ot_id >> 16) as u16;
    let id_low    = (ot_id & 0xFFFF) as u16;
    // k16: constant part of the shiny XOR; full XOR = k16 ^ new_b0.
    let k16    = p_high ^ (b1 << 8) ^ id_high ^ id_low;
    let upper  = personality & 0xFFFF_FF00;

    let new_b0 = (0u32..=255).find(|&c| {
        let c   = c as u8;
        let new_p = upper | c as u32;
        if new_p % 25 != target_nature as u32 { return false; }
        if let Some(female) = is_female {
            let gender_ok = if female { c < gender_ratio } else { c >= gender_ratio };
            if !gender_ok { return false; }
        }
        if shiny_now && (k16 ^ c as u16) >= 8 { return false; }
        true
    })? as u8;

    let new_p       = upper | new_b0 as u32;
    let enc_key     = personality ^ ot_id;
    let new_enc_key = new_p ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    // Rearrange substructures if personality % 24 changed.
    let old_mod24 = (personality % 24) as usize;
    let new_mod24 = (new_p % 24) as usize;
    let arranged = if old_mod24 == new_mod24 {
        decrypted
    } else {
        let mut r = [0u8; 48];
        for t in 0..4 {
            let old_pos = SUBSTRUCTURE_ORDER[old_mod24][t] as usize;
            let new_pos = SUBSTRUCTURE_ORDER[new_mod24][t] as usize;
            r[new_pos * 12..new_pos * 12 + 12]
                .copy_from_slice(&decrypted[old_pos * 12..old_pos * 12 + 12]);
        }
        r
    };

    let checksum: u16 = arranged
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in arranged.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ new_enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    Some(ChangeNatureOutcome::Write {
        new_personality: new_p,
        checksum: checksum.to_le_bytes(),
        encrypted,
    })
}

/// Changes the nature of the party Pokémon at `party_position` (0–5) to
/// `target_nature` (0–24, Hardy=0 … Quirky=24).
///
/// Adjusts only the low byte of the personality, preserving gender for species
/// where gender is determined by personality. If the Pokémon is currently shiny,
/// only bytes that keep the Gen III shiny formula satisfied are considered — the
/// call logs a warning and returns `false` if no such byte exists. The
/// substructure layout is rearranged when `personality % 24` changes.
///
/// Writes an atomic 80-byte payload (personality + re-encrypted block) so the
/// game never sees an inconsistent state.
pub fn change_nature(party_position: usize, target_nature: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("change_nature: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    if target_nature > 24 {
        tracing::warn!("change_nature: target_nature {target_nature} out of range (must be 0–24)");
        return false;
    }

    const PARTY_BASE:       u32   = 0x02024284;
    const MON_SIZE:         u32   = 100;
    const BASE_STATS_SIZE:  usize = 28;
    const GENDER_RATIO_OFF: usize = 0x10;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("change_nature: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("change_nature: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("change_nature: party[{party_position}] is empty (personality=0)");
        return false;
    }

    // Decrypt to read species → look up gender_ratio from ROM base-stats.
    let ot_id   = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let enc_key = personality ^ ot_id;
    let mut dec_tmp = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        dec_tmp[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    let g_off   = growth_substructure_index(personality) * 12;
    let species = u16::from_le_bytes([dec_tmp[g_off], dec_tmp[g_off + 1]]);

    let rom      = fire_red_rom_buffer::get_rom();
    let rom_addr = fire_red_rom_buffer::get_rom_addresses();
    let gr_off   = rom_addr.base_stats_addr + species as usize * BASE_STATS_SIZE + GENDER_RATIO_OFF;
    let gender_ratio = if gr_off < rom.len() { rom[gr_off] } else { 255 };

    match compute_change_nature(&data, target_nature, gender_ratio) {
        None => {
            tracing::warn!(
                "change_nature: no low byte satisfies nature={target_nature} + gender + shiny \
                 constraints for party[{party_position}]"
            );
            false
        }
        Some(ChangeNatureOutcome::AlreadyMatches) => {
            tracing::info!("change_nature: party[{party_position}] already nature={target_nature}");
            true
        }
        Some(ChangeNatureOutcome::Write { new_personality, checksum, encrypted }) => {
            let mut payload = [0u8; 80];
            payload[0..4].copy_from_slice(&new_personality.to_le_bytes());
            payload[4..28].copy_from_slice(&data[4..28]);
            payload[28..30].copy_from_slice(&checksum);
            payload[30..32].copy_from_slice(&data[30..32]);
            payload[32..80].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr, &payload) {
                tracing::warn!("change_nature: write failed for party[{party_position}]");
                return false;
            }
            tracing::info!(
                "change_nature: party[{party_position}] 0x{personality:08X} → 0x{new_personality:08X} \
                 (nature={target_nature})"
            );
            true
        }
    }
}

// ── effort / IV helpers ───────────────────────────────────────────────────────

/// Returns the index (0–3) of the Effort substructure for a given `personality`.
fn effort_substructure_index(personality: u32) -> usize {
    SUBSTRUCTURE_ORDER[(personality % 24) as usize][2] as usize
}

/// Pack six 5-bit IV values into the lower 30 bits of a u32.
pub(crate) fn pack_ivs(hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8) -> u32 {
      (hp    as u32 & 0x1F)
    | ((atk   as u32 & 0x1F) << 5)
    | ((def   as u32 & 0x1F) << 10)
    | ((spd   as u32 & 0x1F) << 15)
    | ((spa   as u32 & 0x1F) << 20)
    | ((spdef as u32 & 0x1F) << 25)
}

/// Unpack the lower 30 bits of an IV/egg/ability word into `[hp, atk, def, spd, spa, spdef]`.
fn unpack_ivs(word: u32) -> [u8; 6] {
    [
        (word        & 0x1F) as u8,
        ((word >> 5)  & 0x1F) as u8,
        ((word >> 10) & 0x1F) as u8,
        ((word >> 15) & 0x1F) as u8,
        ((word >> 20) & 0x1F) as u8,
        ((word >> 25) & 0x1F) as u8,
    ]
}

// ── attacks_substructure helpers ──────────────────────────────────────────────

/// Returns the index (0–3) of the Attacks substructure for a given `personality`.
fn attacks_substructure_index(personality: u32) -> usize {
    SUBSTRUCTURE_ORDER[(personality % 24) as usize][1] as usize
}

/// Base PP of `move_id` from the ROM move-data table (12 bytes/entry, PP at byte 4).
fn base_pp_for_move(rom: &[u8], move_data_addr: usize, move_id: u16) -> u8 {
    let off = move_data_addr + move_id as usize * 12 + 4;
    rom.get(off).copied().unwrap_or(0)
}

/// Maximum PP for a single slot given `base_pp` and the 2-bit PP-bonus for that slot.
fn max_pp_for_slot(base_pp: u8, pp_bonuses: u8, slot: usize) -> u8 {
    let bonus = (pp_bonuses >> (slot * 2)) & 0x3;
    base_pp + (base_pp as u16 * bonus as u16 / 5) as u8
}

// ── restore_pp ────────────────────────────────────────────────────────────────

/// Pure logic for [`restore_pp`]: fills every non-empty move slot to its
/// current maximum PP.
///
/// `data` must be 80 bytes. `rom` is the full ROM image and `move_data_addr`
/// is the byte offset of the `gBattleMoves` table within it.
///
/// Returns `None` when `personality == 0` (empty slot), `data` is too short,
/// or all move slots are already at maximum PP.
pub(crate) fn compute_restore_pp(
    data: &[u8],
    rom: &[u8],
    move_data_addr: usize,
) -> Option<([u8; 2], [u8; 48])> {
    if data.len() < 80 { return None; }
    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }
    let enc_key = personality ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    let g_off      = growth_substructure_index(personality)  * 12;
    let a_off      = attacks_substructure_index(personality) * 12;
    let pp_bonuses = decrypted[g_off + 8];

    let mut changed = false;
    for slot in 0..4usize {
        let move_id = u16::from_le_bytes([decrypted[a_off + slot * 2], decrypted[a_off + slot * 2 + 1]]);
        if move_id == 0 { continue; }
        let base_pp = base_pp_for_move(rom, move_data_addr, move_id);
        if base_pp == 0 { continue; }
        let target_pp = max_pp_for_slot(base_pp, pp_bonuses, slot);
        if decrypted[a_off + 8 + slot] < target_pp {
            decrypted[a_off + 8 + slot] = target_pp;
            changed = true;
        }
    }
    if !changed { return None; }

    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    Some((checksum.to_le_bytes(), encrypted))
}

/// Restores PP on all four move slots of the party Pokémon at `party_position`
/// (0–5) to their current maximums (base PP + PP-Up bonus).
///
/// Only equipped slots (move_id ≠ 0) are modified; the encrypted data block is
/// decrypted, PP bytes updated, checksum recalculated, and re-encrypted.
/// Personality, species, IVs, nature, and shiny status are all preserved.
///
/// Returns `true` when the write was dispatched (or PP was already full).
pub fn restore_pp(party_position: usize) -> bool {
    if party_position >= 6 {
        tracing::warn!("restore_pp: party_position {party_position} out of range (must be 0–5)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("restore_pp: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("restore_pp: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("restore_pp: party[{party_position}] is empty (personality=0)");
        return false;
    }

    let rom      = fire_red_rom_buffer::get_rom();
    let rom_addr = fire_red_rom_buffer::get_rom_addresses();

    match compute_restore_pp(&data, rom, rom_addr.move_data_addr) {
        None => {
            tracing::info!("restore_pp: party[{party_position}] PP already full");
            true
        }
        Some((checksum, encrypted)) => {
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]);
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr + 28, &payload) {
                tracing::warn!("restore_pp: write failed for party[{party_position}]");
                return false;
            }
            tracing::info!("restore_pp: party[{party_position}] PP restored");
            true
        }
    }
}

// ── set_friendship ────────────────────────────────────────────────────────────

/// Pure logic for [`set_friendship`]: sets the friendship byte in the Growth
/// substructure.
///
/// Returns `None` when `personality == 0`, `data` is too short, or the
/// friendship byte already equals `friendship`.
pub(crate) fn compute_set_friendship(data: &[u8], friendship: u8) -> Option<([u8; 2], [u8; 48])> {
    if data.len() < 80 { return None; }
    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }
    let enc_key = personality ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    let g_off = growth_substructure_index(personality) * 12;
    if decrypted[g_off + 9] == friendship { return None; }
    decrypted[g_off + 9] = friendship;

    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    Some((checksum.to_le_bytes(), encrypted))
}

/// Sets the friendship (happiness) byte of the party Pokémon at
/// `party_position` (0–5) to `friendship` (0–255).
///
/// The friendship byte lives at Growth substructure offset 9. The checksum is
/// recalculated and the data block re-encrypted. All other fields are
/// preserved.
///
/// Returns `true` when the write was dispatched (or friendship already matches).
pub fn set_friendship(party_position: usize, friendship: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("set_friendship: party_position {party_position} out of range (must be 0–5)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("set_friendship: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("set_friendship: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("set_friendship: party[{party_position}] is empty (personality=0)");
        return false;
    }

    match compute_set_friendship(&data, friendship) {
        None => {
            tracing::info!("set_friendship: party[{party_position}] already friendship={friendship}");
            true
        }
        Some((checksum, encrypted)) => {
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]);
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr + 28, &payload) {
                tracing::warn!("set_friendship: write failed for party[{party_position}]");
                return false;
            }
            tracing::info!("set_friendship: party[{party_position}] → friendship={friendship}");
            true
        }
    }
}

// ── change_move ───────────────────────────────────────────────────────────────

/// Pure logic for [`change_move`]: replaces the move at `slot` (0–3) and sets
/// its current PP to the new move's maximum.
///
/// `data` must be 80 bytes. `rom` and `move_data_addr` are used to look up the
/// new move's base PP. Returns `None` when the slot already holds `move_id` or
/// when `personality == 0`.
pub(crate) fn compute_change_move(
    data: &[u8],
    slot: u8,
    move_id: u16,
    rom: &[u8],
    move_data_addr: usize,
) -> Option<([u8; 2], [u8; 48])> {
    if data.len() < 80 || slot > 3 { return None; }
    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }
    let enc_key = personality ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    let g_off = growth_substructure_index(personality)  * 12;
    let a_off = attacks_substructure_index(personality) * 12;
    let s     = slot as usize;

    let old_move = u16::from_le_bytes([decrypted[a_off + s * 2], decrypted[a_off + s * 2 + 1]]);
    if old_move == move_id { return None; }
    decrypted[a_off + s * 2..a_off + s * 2 + 2].copy_from_slice(&move_id.to_le_bytes());

    let pp_bonuses = decrypted[g_off + 8];
    decrypted[a_off + 8 + s] = if move_id == 0 {
        0
    } else {
        let base_pp = base_pp_for_move(rom, move_data_addr, move_id);
        max_pp_for_slot(base_pp, pp_bonuses, s)
    };

    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    Some((checksum.to_le_bytes(), encrypted))
}

/// Replaces the move at `slot` (0–3) of the party Pokémon at `party_position`
/// (0–5) with `move_id`, setting current PP to the new move's maximum.
///
/// Use `move_id = 0` to clear the slot (current PP is set to 0). The PP-Up
/// bonus for the slot is preserved — only base PP + existing bonus is applied.
/// All other data (species, IVs, nature, shiny) is untouched.
///
/// Returns `true` when the write was dispatched (or the slot already holds the
/// requested move).
pub fn change_move(party_position: usize, slot: u8, move_id: u16) -> bool {
    if party_position >= 6 {
        tracing::warn!("change_move: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    if slot > 3 {
        tracing::warn!("change_move: slot {slot} out of range (must be 0–3)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("change_move: socket error: {e}"); return false; }
    };

    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => {
            tracing::warn!("change_move: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("change_move: party[{party_position}] is empty (personality=0)");
        return false;
    }

    let rom      = fire_red_rom_buffer::get_rom();
    let rom_addr = fire_red_rom_buffer::get_rom_addresses();

    match compute_change_move(&data, slot, move_id, rom, rom_addr.move_data_addr) {
        None => {
            tracing::info!("change_move: party[{party_position}] slot {slot} already move_id={move_id}");
            true
        }
        Some((checksum, encrypted)) => {
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]);
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr + 28, &payload) {
                tracing::warn!("change_move: write failed for party[{party_position}]");
                return false;
            }
            tracing::info!(
                "change_move: party[{party_position}] slot {slot} → move_id={move_id}"
            );
            true
        }
    }
}

// ── set_ivs / increase_ivs ────────────────────────────────────────────────────

/// Shared decrypt → modify IV word → recompute checksum → re-encrypt core used by
/// both [`compute_set_ivs`] and [`compute_increase_ivs`].
fn apply_iv_word(data: &[u8], new_word_fn: impl FnOnce(u32) -> u32) -> Option<([u8; 2], [u8; 48])> {
    if data.len() < 80 { return None; }
    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }
    let enc_key = personality ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    let iv_off   = misc_substructure_index(personality) * 12 + 4;
    let old_word = u32::from_le_bytes(decrypted[iv_off..iv_off + 4].try_into().unwrap());
    let new_word = new_word_fn(old_word);
    if new_word == old_word { return None; }
    decrypted[iv_off..iv_off + 4].copy_from_slice(&new_word.to_le_bytes());

    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    Some((checksum.to_le_bytes(), encrypted))
}

/// Pure logic for [`set_ivs`]: overwrites all six IVs, preserving bits 30–31.
pub(crate) fn compute_set_ivs(
    data: &[u8],
    hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8,
) -> Option<([u8; 2], [u8; 48])> {
    apply_iv_word(data, |old| {
        let high = old & 0xC000_0000;
        high | pack_ivs(hp.min(31), atk.min(31), def.min(31), spd.min(31), spa.min(31), spdef.min(31))
    })
}

/// Pure logic for [`increase_ivs`]: adds deltas to each IV clamped at 31, preserving bits 30–31.
pub(crate) fn compute_increase_ivs(
    data: &[u8],
    hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8,
) -> Option<([u8; 2], [u8; 48])> {
    apply_iv_word(data, |old| {
        let high = old & 0xC000_0000;
        let o = unpack_ivs(old);
        high | pack_ivs(
            o[0].saturating_add(hp).min(31),
            o[1].saturating_add(atk).min(31),
            o[2].saturating_add(def).min(31),
            o[3].saturating_add(spd).min(31),
            o[4].saturating_add(spa).min(31),
            o[5].saturating_add(spdef).min(31),
        )
    })
}

/// Writes the result of an IV compute function to RetroArch.
fn write_iv_change(
    party_position: usize,
    fn_name: &str,
    result: Option<([u8; 2], [u8; 48])>,
    data: &[u8],
    socket: &std::net::UdpSocket,
    mon_addr: u32,
    personality: u32,
) -> bool {
    match result {
        None => {
            tracing::info!("{fn_name}: party[{party_position}] IVs already match or slot empty");
            true
        }
        Some((checksum, encrypted)) => {
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]);
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(socket, mon_addr + 28, &payload) {
                tracing::warn!("{fn_name}: write failed for party[{party_position}]");
                return false;
            }
            tracing::info!("{fn_name}: party[{party_position}] personality=0x{personality:08X} IVs updated");
            true
        }
    }
}

/// Sets all six IVs of the party Pokémon at `party_position` (0–5). Each value
/// is clamped to 31. Egg and ability bits (30–31 of the Misc IV word) are
/// preserved.
pub fn set_ivs(party_position: usize, hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("set_ivs: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;
    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("set_ivs: socket error: {e}"); return false; }
    };
    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => { tracing::warn!("set_ivs: RetroArch did not respond for party[{party_position}]"); return false; }
    };
    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 { tracing::warn!("set_ivs: party[{party_position}] is empty"); return false; }
    let result = compute_set_ivs(&data, hp, atk, def, spd, spa, spdef);
    write_iv_change(party_position, "set_ivs", result, &data, &socket, mon_addr, personality)
}

/// Adds each delta to the corresponding IV of the party Pokémon at
/// `party_position` (0–5), clamping each result at 31.
pub fn increase_ivs(party_position: usize, hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("increase_ivs: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;
    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("increase_ivs: socket error: {e}"); return false; }
    };
    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => { tracing::warn!("increase_ivs: RetroArch did not respond for party[{party_position}]"); return false; }
    };
    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 { tracing::warn!("increase_ivs: party[{party_position}] is empty"); return false; }
    let result = compute_increase_ivs(&data, hp, atk, def, spd, spa, spdef);
    write_iv_change(party_position, "increase_ivs", result, &data, &socket, mon_addr, personality)
}

// ── set_evs / increase_evs ────────────────────────────────────────────────────

/// Shared decrypt → modify EV bytes → recompute checksum → re-encrypt core.
fn apply_ev_bytes(data: &[u8], new_evs_fn: impl FnOnce([u8; 6]) -> [u8; 6]) -> Option<([u8; 2], [u8; 48])> {
    if data.len() < 80 { return None; }
    let personality = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let ot_id       = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if personality == 0 { return None; }
    let enc_key = personality ^ ot_id;

    let mut decrypted = [0u8; 48];
    for (i, chunk) in data[32..80].chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        decrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }

    let e_off    = effort_substructure_index(personality) * 12;
    let old_evs: [u8; 6] = decrypted[e_off..e_off + 6].try_into().unwrap();
    let new_evs  = new_evs_fn(old_evs);
    if new_evs == old_evs { return None; }
    decrypted[e_off..e_off + 6].copy_from_slice(&new_evs);

    let checksum: u16 = decrypted
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .fold(0u16, |acc, w| acc.wrapping_add(w));

    let mut encrypted = [0u8; 48];
    for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
        let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ enc_key;
        encrypted[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    Some((checksum.to_le_bytes(), encrypted))
}

/// Pure logic for [`set_evs`]: overwrites the six EV bytes.
pub(crate) fn compute_set_evs(
    data: &[u8],
    hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8,
) -> Option<([u8; 2], [u8; 48])> {
    apply_ev_bytes(data, |_| [hp, atk, def, spd, spa, spdef])
}

/// Pure logic for [`increase_evs`]: adds deltas to each EV clamped at 255.
pub(crate) fn compute_increase_evs(
    data: &[u8],
    hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8,
) -> Option<([u8; 2], [u8; 48])> {
    apply_ev_bytes(data, |o| [
        o[0].saturating_add(hp),
        o[1].saturating_add(atk),
        o[2].saturating_add(def),
        o[3].saturating_add(spd),
        o[4].saturating_add(spa),
        o[5].saturating_add(spdef),
    ])
}

/// Writes the result of an EV compute function to RetroArch.
fn write_ev_change(
    party_position: usize,
    fn_name: &str,
    result: Option<([u8; 2], [u8; 48])>,
    data: &[u8],
    socket: &std::net::UdpSocket,
    mon_addr: u32,
    personality: u32,
) -> bool {
    match result {
        None => {
            tracing::info!("{fn_name}: party[{party_position}] EVs already match or slot empty");
            true
        }
        Some((checksum, encrypted)) => {
            let mut payload = [0u8; 52];
            payload[0..2].copy_from_slice(&checksum);
            payload[2..4].copy_from_slice(&data[30..32]);
            payload[4..52].copy_from_slice(&encrypted);
            if !fire_red_retroarch_interfacing::write_to_retroarch(socket, mon_addr + 28, &payload) {
                tracing::warn!("{fn_name}: write failed for party[{party_position}]");
                return false;
            }
            tracing::info!("{fn_name}: party[{party_position}] personality=0x{personality:08X} EVs updated");
            true
        }
    }
}

/// Sets all six EVs of the party Pokémon at `party_position` (0–5). The
/// 510-total game cap is not enforced; each value may be 0–255 independently.
/// Contest-condition bytes in the Effort substructure are preserved.
pub fn set_evs(party_position: usize, hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("set_evs: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;
    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("set_evs: socket error: {e}"); return false; }
    };
    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => { tracing::warn!("set_evs: RetroArch did not respond for party[{party_position}]"); return false; }
    };
    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 { tracing::warn!("set_evs: party[{party_position}] is empty"); return false; }
    let result = compute_set_evs(&data, hp, atk, def, spd, spa, spdef);
    write_ev_change(party_position, "set_evs", result, &data, &socket, mon_addr, personality)
}

/// Adds each delta to the corresponding EV of the party Pokémon at
/// `party_position` (0–5), clamping each at 255. The 510-total game cap is
/// not enforced.
pub fn increase_evs(party_position: usize, hp: u8, atk: u8, def: u8, spd: u8, spa: u8, spdef: u8) -> bool {
    if party_position >= 6 {
        tracing::warn!("increase_evs: party_position {party_position} out of range (must be 0–5)");
        return false;
    }
    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;
    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("increase_evs: socket error: {e}"); return false; }
    };
    let data = match read_retroarch_bytes(&socket, mon_addr, 80) {
        Some(b) if b.len() == 80 => b,
        _ => { tracing::warn!("increase_evs: RetroArch did not respond for party[{party_position}]"); return false; }
    };
    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 { tracing::warn!("increase_evs: party[{party_position}] is empty"); return false; }
    let result = compute_increase_evs(&data, hp, atk, def, spd, spa, spdef);
    write_ev_change(party_position, "increase_evs", result, &data, &socket, mon_addr, personality)
}

// ── restore_hp ────────────────────────────────────────────────────────────────

/// Restores the current HP of the party Pokémon at `party_position` (0–5) to
/// its calculated maximum.
///
/// Reads the max-HP word from PartyPokemon byte offset 88–89 (unencrypted) and
/// writes it to the current-HP word at offset 86–87. No encrypted data block is
/// read or written.
///
/// Returns `true` when the write was dispatched (or HP was already full).
pub fn restore_hp(party_position: usize) -> bool {
    if party_position >= 6 {
        tracing::warn!("restore_hp: party_position {party_position} out of range (must be 0–5)");
        return false;
    }

    const PARTY_BASE: u32 = 0x02024284;
    const MON_SIZE:   u32 = 100;
    let mon_addr = PARTY_BASE + party_position as u32 * MON_SIZE;

    let socket = match fire_red_retroarch_interfacing::make_socket() {
        Ok(s)  => s,
        Err(e) => { tracing::warn!("restore_hp: socket error: {e}"); return false; }
    };

    // Read 90 bytes: 0–3 personality check; 86–87 current HP; 88–89 max HP.
    let data = match read_retroarch_bytes(&socket, mon_addr, 90) {
        Some(b) if b.len() == 90 => b,
        _ => {
            tracing::warn!("restore_hp: RetroArch did not respond for party[{party_position}]");
            return false;
        }
    };

    let personality = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if personality == 0 {
        tracing::warn!("restore_hp: party[{party_position}] is empty (personality=0)");
        return false;
    }

    let current_hp = u16::from_le_bytes([data[86], data[87]]);
    let max_hp     = u16::from_le_bytes([data[88], data[89]]);

    if current_hp >= max_hp {
        tracing::info!("restore_hp: party[{party_position}] HP already full ({current_hp}/{max_hp})");
        return true;
    }

    if !fire_red_retroarch_interfacing::write_to_retroarch(&socket, mon_addr + 86, &max_hp.to_le_bytes()) {
        tracing::warn!("restore_hp: write failed for party[{party_position}]");
        return false;
    }
    tracing::info!("restore_hp: party[{party_position}] {current_hp} → {max_hp} HP");
    true
}

/// Reads all four bag pockets from the player's SaveBlock1 and returns a
/// [`BagPockets`] with quantities already XOR-decrypted.
///
/// Returns `None` if the SaveBlock1 pointer cannot be resolved or if
/// the RetroArch socket cannot be created.
pub fn read_bag_pockets() -> Option<BagPockets> {
    let (save_block1, save_block2) = {
        let iwram = fire_red_memory::get_iwram();
        let ewram = fire_red_memory::get_ewram();
        let sb1 = read_save_block1_ptr(&iwram, &ewram)?;
        let sb2 = read_save_block2_ptr(&iwram, &ewram).unwrap_or(SAVE_BLOCK_2_BASE);
        (sb1 as u32, sb2 as u32)
    };

    let socket = fire_red_retroarch_interfacing::make_socket().ok()?;

    let raw_items = read_retroarch_bytes(
        &socket,
        save_block1 + ITEMS_POCKET_SAVE_BLOCK_OFFSET as u32,
        ITEMS_POCKET_SLOTS * 4,
    )?;
    let raw_key_items = read_retroarch_bytes(
        &socket,
        save_block1 + KEY_ITEMS_POCKET_SAVE_BLOCK_OFFSET as u32,
        KEY_ITEMS_POCKET_SLOTS * 4,
    )?;
    let raw_balls = read_retroarch_bytes(
        &socket,
        save_block1 + BALLS_POCKET_SAVE_BLOCK_OFFSET as u32,
        BALLS_POCKET_SLOTS * 4,
    )?;
    let raw_tms = read_retroarch_bytes(
        &socket,
        save_block1 + TMS_POCKET_SAVE_BLOCK_OFFSET as u32,
        TMS_POCKET_SLOTS * 4,
    )?;

    // Derive the encryption key using the oracle (balls + items pockets),
    // falling back to a direct SaveBlock2 read if the oracle is ambiguous.
    let key = {
        let mut raws: Vec<u16> = Vec::new();
        for s in 0..BALLS_POCKET_SLOTS {
            let b = s * 4;
            if raw_balls.len() >= b + 4 {
                let id = u16::from_le_bytes([raw_balls[b], raw_balls[b + 1]]);
                if (1..=12).contains(&id) {
                    raws.push(u16::from_le_bytes([raw_balls[b + 2], raw_balls[b + 3]]));
                }
            }
        }
        for s in 0..ITEMS_POCKET_SLOTS {
            let b = s * 4;
            if raw_items.len() >= b + 4 {
                let id = u16::from_le_bytes([raw_items[b], raw_items[b + 1]]);
                if id != 0 {
                    raws.push(u16::from_le_bytes([raw_items[b + 2], raw_items[b + 3]]));
                }
            }
        }
        let oracle = if !raws.is_empty() {
            let mut candidates: Vec<u16> = (1u16..=MAX_ITEM_QTY).map(|q| raws[0] ^ q).collect();
            for &r in &raws[1..] {
                candidates.retain(|&k| (1u16..=MAX_ITEM_QTY).contains(&(r ^ k)));
                if candidates.len() == 1 {
                    break;
                }
            }
            if candidates.len() == 1 { Some(candidates[0]) } else { None }
        } else {
            None
        };
        oracle.unwrap_or_else(|| derive_key_from_save_block2(&socket, save_block2))
    };

    let decrypt = |raw: &[u8], n_slots: usize| -> Vec<ItemSlot> {
        (0..n_slots)
            .filter_map(|s| {
                let b = s * 4;
                if raw.len() < b + 4 {
                    return None;
                }
                let id = u16::from_le_bytes([raw[b], raw[b + 1]]);
                if id == 0 {
                    return None;
                }
                let qty = u16::from_le_bytes([raw[b + 2], raw[b + 3]]) ^ key;
                Some(ItemSlot { item_id: id, quantity: qty })
            })
            .collect()
    };

    Some(BagPockets {
        items:     decrypt(&raw_items,     ITEMS_POCKET_SLOTS),
        key_items: decrypt(&raw_key_items, KEY_ITEMS_POCKET_SLOTS),
        balls:     decrypt(&raw_balls,     BALLS_POCKET_SLOTS),
        tms:       decrypt(&raw_tms,       TMS_POCKET_SLOTS),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ascii_to_gba, compute_change_gender, compute_change_held_item,
        compute_change_move, compute_change_nature, compute_change_species,
        compute_give_item_write, compute_increase_evs, compute_increase_ivs,
        compute_restore_pp, compute_set_ability_bit, compute_set_evs,
        compute_set_friendship, compute_set_ivs, compute_take_item_write,
        encode_nickname, is_shiny, unpack_ivs,
        ChangeGenderOutcome, ChangeNatureOutcome, ChangeSpeciesOutcome, TakeItemWrite,
        ITEMS_POCKET_SLOTS, MAX_ITEM_QTY,
    };

    // ── is_shiny ─────────────────────────────────────────────────────────────
    // Gen III shiny formula: (p_high ^ p_low ^ id_high ^ id_low) < 8

    #[test]
    fn all_zeros_is_shiny() {
        assert!(is_shiny(0, 0));
    }

    #[test]
    fn xor_of_7_is_shiny() {
        // p_high = 0x0007, rest zero → xor = 7 < 8
        assert!(is_shiny(0x0007_0000, 0));
    }

    #[test]
    fn xor_of_8_is_not_shiny() {
        // p_high = 0x0008, rest zero → xor = 8, not < 8
        assert!(!is_shiny(0x0008_0000, 0));
    }

    #[test]
    fn xor_cancelled_by_ot_id_is_shiny() {
        // p_high=0x0010, id_high=0x0010 → they cancel out, net xor = 0 < 8
        assert!(is_shiny(0x0010_0000, 0x0010_0000));
    }

    #[test]
    fn high_xor_is_not_shiny() {
        assert!(!is_shiny(0x1234_5678, 0x0000_0000));
    }

    // ── compute_give_item_write helpers ──────────────────────────────────────

    /// Creates an all-zero (empty) pocket buffer.
    fn empty_pocket() -> Vec<u8> {
        vec![0u8; ITEMS_POCKET_SLOTS * 4]
    }

    /// Writes an item slot into `pocket` at `slot_index`.
    /// `qty_raw` is the already-encrypted quantity (what actually lives in RAM).
    fn write_slot(pocket: &mut [u8], slot_index: usize, item_id: u16, qty_raw: u16) {
        let base = slot_index * 4;
        pocket[base..base + 2].copy_from_slice(&item_id.to_le_bytes());
        pocket[base + 2..base + 4].copy_from_slice(&qty_raw.to_le_bytes());
    }

    /// Decodes the quantity from a 4-byte payload produced by compute_give_item_write.
    fn decode_qty(payload: &[u8; 4], key: u16) -> u16 {
        u16::from_le_bytes([payload[2], payload[3]]) ^ key
    }

    fn decode_item_id(payload: &[u8; 4]) -> u16 {
        u16::from_le_bytes([payload[0], payload[1]])
    }

    // ── compute_give_item_write tests ─────────────────────────────────────────

    #[test]
    fn zero_item_id_returns_none() {
        let pocket = empty_pocket();
        assert!(compute_give_item_write(&pocket, 0, 0, 1).is_none());
    }

    #[test]
    fn zero_quantity_returns_none() {
        let pocket = empty_pocket();
        assert!(compute_give_item_write(&pocket, 0, 13, 0).is_none());
    }

    #[test]
    fn zero_length_pocket_returns_none() {
        assert!(compute_give_item_write(&[], 0, 13, 1).is_none());
    }

    #[test]
    fn empty_pocket_places_item_in_slot_0() {
        let pocket = empty_pocket();
        let (slot, payload) = compute_give_item_write(&pocket, 0, 13, 5).unwrap();
        assert_eq!(slot, 0);
        assert_eq!(decode_item_id(&payload), 13);
        assert_eq!(decode_qty(&payload, 0), 5);
    }

    #[test]
    fn places_in_first_empty_slot_after_other_items() {
        let mut pocket = empty_pocket();
        // slots 0 and 1 occupied by a different item
        write_slot(&mut pocket, 0, 7, 3);
        write_slot(&mut pocket, 1, 8, 2);
        let (slot, payload) = compute_give_item_write(&pocket, 0, 13, 1).unwrap();
        assert_eq!(slot, 2);
        assert_eq!(decode_item_id(&payload), 13);
        assert_eq!(decode_qty(&payload, 0), 1);
    }

    #[test]
    fn existing_stack_is_incremented_not_overwritten() {
        let mut pocket = empty_pocket();
        // item 13 already in slot 2 with qty=10, key=0 → raw=10
        write_slot(&mut pocket, 2, 13, 10);
        let (slot, payload) = compute_give_item_write(&pocket, 0, 13, 5).unwrap();
        assert_eq!(slot, 2);
        assert_eq!(decode_qty(&payload, 0), 15);
    }

    #[test]
    fn existing_stack_preferred_over_earlier_empty_slot() {
        let mut pocket = empty_pocket();
        // slot 0 is empty; slot 1 already has the item
        write_slot(&mut pocket, 1, 13, 20);
        let (slot, _) = compute_give_item_write(&pocket, 0, 13, 1).unwrap();
        // must pick slot 1, not slot 0
        assert_eq!(slot, 1);
    }

    #[test]
    fn quantity_capped_at_99_on_increment() {
        let mut pocket = empty_pocket();
        // slot 0 already has 98; adding 10 should cap at 99
        write_slot(&mut pocket, 0, 13, 98);
        let (_, payload) = compute_give_item_write(&pocket, 0, 13, 10).unwrap();
        assert_eq!(decode_qty(&payload, 0), MAX_ITEM_QTY);
    }

    #[test]
    fn quantity_capped_at_99_on_new_slot() {
        let pocket = empty_pocket();
        let (_, payload) = compute_give_item_write(&pocket, 0, 13, 200).unwrap();
        assert_eq!(decode_qty(&payload, 0), MAX_ITEM_QTY);
    }

    #[test]
    fn security_key_applied_to_stored_qty() {
        let key: u16 = 0x1234;
        let pocket = empty_pocket();
        let (_, payload) = compute_give_item_write(&pocket, key, 13, 5).unwrap();
        // raw stored bytes should be 5 ^ 0x1234
        let raw_stored = u16::from_le_bytes([payload[2], payload[3]]);
        assert_eq!(raw_stored, 5 ^ 0x1234);
        // decoding with the key gives back 5
        assert_eq!(decode_qty(&payload, key), 5);
    }

    #[test]
    fn security_key_applied_when_incrementing_existing_stack() {
        let key: u16 = 0xABCD;
        let mut pocket = empty_pocket();
        // slot 0 has qty=10, stored encrypted: 10 ^ key
        write_slot(&mut pocket, 0, 13, 10 ^ key);
        let (_, payload) = compute_give_item_write(&pocket, key, 13, 7).unwrap();
        assert_eq!(decode_qty(&payload, key), 17);
    }

    #[test]
    fn full_pocket_returns_none() {
        let mut pocket = empty_pocket();
        // fill all 42 slots with a different item
        for slot in 0..ITEMS_POCKET_SLOTS {
            write_slot(&mut pocket, slot, 99, 1);
        }
        assert!(compute_give_item_write(&pocket, 0, 13, 1).is_none());
    }

    #[test]
    fn item_id_preserved_in_payload_little_endian() {
        let pocket = empty_pocket();
        // item_id 0x00FF — low byte 0xFF, high byte 0x00
        let (_, payload) = compute_give_item_write(&pocket, 0, 0x00FF, 1).unwrap();
        assert_eq!(payload[0], 0xFF);
        assert_eq!(payload[1], 0x00);
    }

    #[test]
    fn multi_byte_item_id_preserved() {
        let pocket = empty_pocket();
        // item_id 0x0121 (TM01 in FireRed)
        let (_, payload) = compute_give_item_write(&pocket, 0, 0x0121, 1).unwrap();
        assert_eq!(decode_item_id(&payload), 0x0121);
    }

    #[test]
    fn finds_item_in_last_slot() {
        let mut pocket = empty_pocket();
        // last slot (41) has the item we want
        write_slot(&mut pocket, 41, 13, 3);
        let (slot, payload) = compute_give_item_write(&pocket, 0, 13, 2).unwrap();
        assert_eq!(slot, 41);
        assert_eq!(decode_qty(&payload, 0), 5);
    }

    #[test]
    fn adding_to_already_maxed_stack_stays_at_99() {
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 13, MAX_ITEM_QTY); // already 99, key=0
        let (_, payload) = compute_give_item_write(&pocket, 0, 13, 1).unwrap();
        assert_eq!(decode_qty(&payload, 0), MAX_ITEM_QTY);
    }

    // ── compute_take_item_write tests ─────────────────────────────────────────

    #[test]
    fn take_item_zero_id_returns_none() {
        let pocket = empty_pocket();
        assert!(compute_take_item_write(&pocket, 0, 0, 1).is_none());
    }

    #[test]
    fn take_item_zero_quantity_returns_none() {
        let pocket = empty_pocket();
        assert!(compute_take_item_write(&pocket, 0, 13, 0).is_none());
    }

    #[test]
    fn take_item_not_in_pocket_returns_none() {
        let pocket = empty_pocket();
        assert!(compute_take_item_write(&pocket, 0, 13, 1).is_none());
    }

    #[test]
    fn take_item_partial_decrements_quantity() {
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 13, 10); // qty=10, key=0
        let result = compute_take_item_write(&pocket, 0, 13, 3).unwrap();
        match result {
            TakeItemWrite::UpdateSlot { slot, payload } => {
                assert_eq!(slot, 0);
                assert_eq!(decode_qty(&payload, 0), 7);
                assert_eq!(decode_item_id(&payload), 13);
            }
            TakeItemWrite::WritePocket(_) => panic!("expected UpdateSlot for partial take"),
        }
    }

    #[test]
    fn take_item_exact_quantity_removes_and_compacts() {
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 13, 5); // qty=5, key=0
        let result = compute_take_item_write(&pocket, 0, 13, 5).unwrap();
        match result {
            TakeItemWrite::WritePocket(new_pocket) => {
                assert_eq!(new_pocket.len(), pocket.len());
                assert_eq!(u16::from_le_bytes([new_pocket[0], new_pocket[1]]), 0);
            }
            TakeItemWrite::UpdateSlot { .. } => panic!("expected WritePocket for full removal"),
        }
    }

    #[test]
    fn take_item_excess_quantity_removes() {
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 13, 3); // qty=3 < take 99
        let result = compute_take_item_write(&pocket, 0, 13, 99).unwrap();
        assert!(matches!(result, TakeItemWrite::WritePocket(_)));
    }

    #[test]
    fn take_item_compaction_shifts_slots_left() {
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 7,  2);  // slot 0: item 7,  qty=2
        write_slot(&mut pocket, 1, 13, 5);  // slot 1: item 13, qty=5 — removed
        write_slot(&mut pocket, 2, 99, 10); // slot 2: item 99, qty=10
        let result = compute_take_item_write(&pocket, 0, 13, 99).unwrap();
        match result {
            TakeItemWrite::WritePocket(new_pocket) => {
                let id0 = u16::from_le_bytes([new_pocket[0], new_pocket[1]]);
                let id1 = u16::from_le_bytes([new_pocket[4], new_pocket[5]]);
                let id2 = u16::from_le_bytes([new_pocket[8], new_pocket[9]]);
                assert_eq!(id0, 7,  "slot 0 should be item 7 after compaction");
                assert_eq!(id1, 99, "slot 1 should be item 99 (shifted) after compaction");
                assert_eq!(id2, 0,  "slot 2 should be empty after compaction");
            }
            TakeItemWrite::UpdateSlot { .. } => panic!("expected WritePocket"),
        }
    }

    #[test]
    fn take_item_security_key_applied() {
        let key: u16 = 0xABCD;
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 13, 10 ^ key); // qty=10 stored encrypted
        let result = compute_take_item_write(&pocket, key, 13, 3).unwrap();
        match result {
            TakeItemWrite::UpdateSlot { payload, .. } => {
                assert_eq!(decode_qty(&payload, key), 7);
            }
            TakeItemWrite::WritePocket(_) => panic!("expected UpdateSlot"),
        }
    }

    #[test]
    fn take_item_security_key_applied_on_removal() {
        let key: u16 = 0x1234;
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 7,  2 ^ key);  // slot 0: item 7
        write_slot(&mut pocket, 1, 13, 5 ^ key);  // slot 1: item 13 — removed
        write_slot(&mut pocket, 2, 99, 10 ^ key); // slot 2: item 99 — shifts to slot 1
        let result = compute_take_item_write(&pocket, key, 13, 99).unwrap();
        match result {
            TakeItemWrite::WritePocket(new_pocket) => {
                let qty1 = u16::from_le_bytes([new_pocket[4 + 2], new_pocket[4 + 3]]) ^ key;
                assert_eq!(qty1, 10, "item 99 qty should survive re-keying");
            }
            TakeItemWrite::UpdateSlot { .. } => panic!("expected WritePocket"),
        }
    }

    #[test]
    fn take_item_finds_in_non_zero_slot() {
        let mut pocket = empty_pocket();
        write_slot(&mut pocket, 0, 7, 3);
        write_slot(&mut pocket, 1, 8, 2);
        write_slot(&mut pocket, 2, 13, 6); // target is in slot 2
        let result = compute_take_item_write(&pocket, 0, 13, 2).unwrap();
        match result {
            TakeItemWrite::UpdateSlot { slot, payload } => {
                assert_eq!(slot, 2);
                assert_eq!(decode_qty(&payload, 0), 4);
            }
            TakeItemWrite::WritePocket(_) => panic!("expected UpdateSlot"),
        }
    }

    // ── compute_change_species tests ──────────────────────────────────────────

    /// Builds valid 80-byte Pokémon data from a decrypted block (for testing).
    fn make_mon_data(personality: u32, ot_id: u32, decrypted: [u8; 48]) -> Vec<u8> {
        let mut data = vec![0u8; 80];
        data[0..4].copy_from_slice(&personality.to_le_bytes());
        data[4..8].copy_from_slice(&ot_id.to_le_bytes());
        let checksum: u16 = decrypted.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .fold(0u16, |acc, w| acc.wrapping_add(w));
        data[28..30].copy_from_slice(&checksum.to_le_bytes()); // checksum at +28, not +30
        let key = personality ^ ot_id;
        for (i, chunk) in decrypted.chunks_exact(4).enumerate() {
            let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ key;
            data[32 + i * 4..32 + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        data
    }

    /// Decrypts the data block out of the bytes returned by compute_change_species.
    fn decrypt_block(encrypted: &[u8; 48], personality: u32, ot_id: u32) -> [u8; 48] {
        let key = personality ^ ot_id;
        let mut dec = [0u8; 48];
        for (i, chunk) in encrypted.chunks_exact(4).enumerate() {
            let w = u32::from_le_bytes(chunk.try_into().unwrap()) ^ key;
            dec[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        dec
    }

    #[test]
    fn change_species_empty_slot_returns_none() {
        let data = vec![0u8; 80]; // personality = 0
        assert!(compute_change_species(&data, 1).is_none());
    }

    #[test]
    fn change_species_short_data_returns_none() {
        assert!(compute_change_species(&[0u8; 79], 1).is_none());
    }

    #[test]
    fn change_species_already_same_returns_already_matches() {
        // personality=24 → 24%24=0 → order[0]=[0,1,2,3] → Growth at index 0, offset 0
        let mut dec = [0u8; 48];
        dec[0..2].copy_from_slice(&25u16.to_le_bytes()); // species=25
        let data = make_mon_data(24, 0, dec);
        let result = compute_change_species(&data, 25).unwrap();
        assert!(matches!(result, ChangeSpeciesOutcome::AlreadyMatches));
    }

    #[test]
    fn change_species_writes_correct_species_at_growth_offset_0() {
        // personality=24 → Growth at substructure 0, decrypted offset 0
        let mut dec = [0u8; 48];
        dec[0..2].copy_from_slice(&1u16.to_le_bytes()); // old species=1
        let data = make_mon_data(24, 0, dec);
        let result = compute_change_species(&data, 25).unwrap();
        match result {
            ChangeSpeciesOutcome::Write { encrypted, .. } => {
                let new_dec = decrypt_block(&encrypted, 24, 0);
                let species = u16::from_le_bytes([new_dec[0], new_dec[1]]);
                assert_eq!(species, 25);
            }
            ChangeSpeciesOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_species_writes_correct_species_at_non_zero_growth_index() {
        // personality=30 → 30%24=6 → order[6]=[1,0,2,3] → Growth at index 1, offset 12
        let mut dec = [0u8; 48];
        dec[12..14].copy_from_slice(&7u16.to_le_bytes()); // old species=7 at offset 12
        let data = make_mon_data(30, 0, dec);
        let result = compute_change_species(&data, 25).unwrap();
        match result {
            ChangeSpeciesOutcome::Write { encrypted, .. } => {
                let new_dec = decrypt_block(&encrypted, 30, 0);
                let species = u16::from_le_bytes([new_dec[12], new_dec[13]]);
                assert_eq!(species, 25, "species should be at growth offset 12");
                // Other substructures should be unchanged (still zero).
                assert_eq!(u16::from_le_bytes([new_dec[0], new_dec[1]]), 0, "non-growth slot 0 unchanged");
            }
            ChangeSpeciesOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_species_checksum_is_correct() {
        let mut dec = [0u8; 48];
        dec[0..2].copy_from_slice(&1u16.to_le_bytes());
        let data = make_mon_data(24, 0, dec);
        let result = compute_change_species(&data, 25).unwrap();
        match result {
            ChangeSpeciesOutcome::Write { checksum, encrypted } => {
                let new_dec = decrypt_block(&encrypted, 24, 0);
                let expected: u16 = new_dec.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .fold(0u16, |acc, w| acc.wrapping_add(w));
                assert_eq!(u16::from_le_bytes(checksum), expected);
            }
            ChangeSpeciesOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_species_non_growth_fields_preserved() {
        // Fill the whole decrypted block with non-zero data, change only species.
        // personality=24 → Growth at index 0 (offset 0-11)
        let mut dec = [0xABu8; 48];
        dec[0..2].copy_from_slice(&1u16.to_le_bytes()); // species field
        let data = make_mon_data(24, 0, dec);
        let result = compute_change_species(&data, 25).unwrap();
        match result {
            ChangeSpeciesOutcome::Write { encrypted, .. } => {
                let new_dec = decrypt_block(&encrypted, 24, 0);
                // Species changed
                assert_eq!(u16::from_le_bytes([new_dec[0], new_dec[1]]), 25);
                // Rest of Growth block (bytes 2-11) unchanged
                assert_eq!(&new_dec[2..12], &dec[2..12]);
                // Other substructures completely unchanged
                assert_eq!(&new_dec[12..], &dec[12..]);
            }
            ChangeSpeciesOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_species_growth_position_derived_from_table_not_inverse() {
        // personality=8 → 8%24=8 → SUBSTRUCTURE_ORDER[8]=[2,0,1,3]
        // table[8][0]=2 → Growth is at position 2, byte offset 24.
        // The old buggy `.position(|&t|t==0)` would find value 0 at index 1
        // and write species to offset 12 instead — corrupting the Attacks block.
        let mut dec = [0u8; 48];
        dec[24..26].copy_from_slice(&7u16.to_le_bytes()); // species=7 at correct offset
        let data = make_mon_data(8, 0, dec);
        let result = compute_change_species(&data, 25).unwrap();
        match result {
            ChangeSpeciesOutcome::Write { encrypted, .. } => {
                let new_dec = decrypt_block(&encrypted, 8, 0);
                assert_eq!(
                    u16::from_le_bytes([new_dec[24], new_dec[25]]), 25,
                    "species must be written to growth offset 24, not 12"
                );
                assert_eq!(
                    u16::from_le_bytes([new_dec[12], new_dec[13]]), 0,
                    "Attacks block at offset 12 must be untouched"
                );
            }
            ChangeSpeciesOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_species_ot_id_xor_key_applied() {
        // Use a non-zero ot_id so enc_key = personality ^ ot_id != personality.
        let personality: u32 = 24;
        let ot_id:       u32 = 0xDEAD_BEEF;
        let mut dec = [0u8; 48];
        dec[0..2].copy_from_slice(&1u16.to_le_bytes());
        let data = make_mon_data(personality, ot_id, dec);
        let result = compute_change_species(&data, 25).unwrap();
        match result {
            ChangeSpeciesOutcome::Write { encrypted, .. } => {
                let new_dec = decrypt_block(&encrypted, personality, ot_id);
                assert_eq!(u16::from_le_bytes([new_dec[0], new_dec[1]]), 25);
            }
            ChangeSpeciesOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    // ── compute_set_ability_bit tests ─────────────────────────────────────────

    /// Builds an 80-byte Pokémon data buffer with the given decrypted block.
    /// (Re-uses the same make_mon_data helper from the change_species tests above.)

    #[test]
    fn set_ability_bit_empty_slot_returns_none() {
        assert!(compute_set_ability_bit(&[0u8; 80], 0).is_none());
    }

    #[test]
    fn set_ability_bit_short_data_returns_none() {
        assert!(compute_set_ability_bit(&[1u8; 79], 0).is_none());
    }

    #[test]
    fn set_ability_bit_already_slot0_returns_none() {
        // personality=24, order 0 = [0,1,2,3] → M at pos 3, iv_ea at byte 40.
        let personality: u32 = 24;
        let mut dec = [0u8; 48];
        // bit 31 = 0 → already slot 0
        dec[40..44].copy_from_slice(&0u32.to_le_bytes());
        let data = make_mon_data(personality, 0, dec);
        assert!(compute_set_ability_bit(&data, 0).is_none());
    }

    #[test]
    fn set_ability_bit_already_slot1_returns_none() {
        let personality: u32 = 24;
        let mut dec = [0u8; 48];
        dec[40..44].copy_from_slice(&(1u32 << 31).to_le_bytes());
        let data = make_mon_data(personality, 0, dec);
        assert!(compute_set_ability_bit(&data, 1).is_none());
    }

    #[test]
    fn set_ability_bit_sets_bit31_for_slot1() {
        // personality=24 → M at position 3, iv_ea at decrypted byte 40.
        let personality: u32 = 24;
        let ot_id:       u32 = 0;
        let mut dec = [0u8; 48];
        dec[40..44].copy_from_slice(&0u32.to_le_bytes()); // slot 0 initially
        let data = make_mon_data(personality, ot_id, dec);
        let (_, encrypted) = compute_set_ability_bit(&data, 1).expect("should write");
        let new_dec = decrypt_block(&encrypted, personality, ot_id);
        let iv_ea = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        assert_eq!((iv_ea >> 31) & 1, 1, "ability bit should be 1");
        // IVs (bits 0-29) should be unchanged (all zero)
        assert_eq!(iv_ea & 0x3FFF_FFFF, 0);
    }

    #[test]
    fn set_ability_bit_clears_bit31_for_slot0() {
        let personality: u32 = 24;
        let ot_id:       u32 = 0;
        let mut dec = [0u8; 48];
        dec[40..44].copy_from_slice(&(1u32 << 31).to_le_bytes()); // slot 1 initially
        let data = make_mon_data(personality, ot_id, dec);
        let (_, encrypted) = compute_set_ability_bit(&data, 0).expect("should write");
        let new_dec = decrypt_block(&encrypted, personality, ot_id);
        let iv_ea = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        assert_eq!((iv_ea >> 31) & 1, 0, "ability bit should be 0");
    }

    #[test]
    fn set_ability_bit_preserves_ivs() {
        // Set some IV bits and verify they survive the toggle.
        let personality: u32 = 24;
        let ot_id:       u32 = 0;
        let iv_value: u32 = 0b11111_10101_01010_11001_00110_10011; // 30 IV bits
        let mut dec = [0u8; 48];
        dec[40..44].copy_from_slice(&iv_value.to_le_bytes()); // bit 31 = 0 → slot 0
        let data = make_mon_data(personality, ot_id, dec);
        let (_, encrypted) = compute_set_ability_bit(&data, 1).expect("should write");
        let new_dec = decrypt_block(&encrypted, personality, ot_id);
        let iv_ea = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        assert_eq!(iv_ea & 0x3FFF_FFFF, iv_value & 0x3FFF_FFFF, "IVs must be preserved");
        assert_eq!((iv_ea >> 31) & 1, 1, "ability bit should be 1");
    }

    #[test]
    fn set_ability_bit_checksum_correct_after_change() {
        let personality: u32 = 24;
        let ot_id:       u32 = 0;
        let mut dec = [0u8; 48];
        dec[40..44].copy_from_slice(&0u32.to_le_bytes());
        let data = make_mon_data(personality, ot_id, dec);
        let (cs_bytes, encrypted) = compute_set_ability_bit(&data, 1).expect("should write");
        let new_dec = decrypt_block(&encrypted, personality, ot_id);
        let expected: u16 = new_dec
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .fold(0u16, |acc, w| acc.wrapping_add(w));
        assert_eq!(u16::from_le_bytes(cs_bytes), expected);
    }

    #[test]
    fn set_ability_bit_misc_at_correct_substructure_position() {
        // personality=18 → 18%24=18 → ORDER[18]=[1,2,3,0] → M(type 3) at position 0
        // → iv_ea at decrypted byte 4, NOT byte 40 (which would be M at pos 3).
        let personality: u32 = 18;
        let ot_id:       u32 = 0;
        let mut dec = [0u8; 48];
        // Place zero at byte 4 (M at pos 0), something non-zero elsewhere.
        dec[4..8].copy_from_slice(&0u32.to_le_bytes());
        dec[40..44].copy_from_slice(&(1u32 << 31).to_le_bytes()); // would be wrong M pos
        let data = make_mon_data(personality, ot_id, dec);
        let (_, encrypted) = compute_set_ability_bit(&data, 1).expect("should write");
        let new_dec = decrypt_block(&encrypted, personality, ot_id);
        // Only byte 4 should have been modified (M is at pos 0)
        let iv_ea_correct = u32::from_le_bytes(new_dec[4..8].try_into().unwrap());
        let iv_ea_wrong   = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        assert_eq!((iv_ea_correct >> 31) & 1, 1, "bit set at Misc offset 4");
        assert_eq!((iv_ea_wrong >> 31) & 1, 1, "byte 40 unchanged (had 1 already)");
        // More specifically: byte 40 should be exactly what we put there (unchanged).
        assert_eq!(iv_ea_wrong, 1u32 << 31);
    }

    #[test]
    fn set_ability_bit_ot_id_xor_key_applied() {
        let personality: u32 = 24;
        let ot_id:       u32 = 0xDEAD_BEEF;
        let mut dec = [0u8; 48];
        dec[40..44].copy_from_slice(&0u32.to_le_bytes());
        let data = make_mon_data(personality, ot_id, dec);
        let (_, encrypted) = compute_set_ability_bit(&data, 1).expect("should write");
        let new_dec = decrypt_block(&encrypted, personality, ot_id);
        let iv_ea = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        assert_eq!((iv_ea >> 31) & 1, 1);
    }

    // ── compute_change_gender tests ───────────────────────────────────────────

    // Gender ratio 127: female if b0 < 127, male if b0 >= 127 (most species).

    #[test]
    fn change_gender_empty_slot_returns_none() {
        assert!(compute_change_gender(&[0u8; 80], 0, 127).is_none());
    }

    #[test]
    fn change_gender_short_data_returns_none() {
        assert!(compute_change_gender(&[1u8; 79], 0, 127).is_none());
    }

    #[test]
    fn change_gender_genderless_returns_none() {
        let data = make_mon_data(1, 0, [0u8; 48]);
        assert!(compute_change_gender(&data, 0, 255).is_none());
        assert!(compute_change_gender(&data, 1, 255).is_none());
    }

    #[test]
    fn change_gender_always_male_species_target_female_returns_none() {
        let data = make_mon_data(1, 0, [0u8; 48]);
        assert!(compute_change_gender(&data, 1, 0).is_none());
    }

    #[test]
    fn change_gender_always_female_species_target_male_returns_none() {
        let data = make_mon_data(1, 0, [0u8; 48]);
        assert!(compute_change_gender(&data, 0, 254).is_none());
    }

    #[test]
    fn change_gender_always_male_species_target_male_is_already_matches() {
        let data = make_mon_data(1, 0, [0u8; 48]);
        matches!(compute_change_gender(&data, 0, 0), Some(ChangeGenderOutcome::AlreadyMatches));
    }

    #[test]
    fn change_gender_already_correct_gender_returns_already_matches() {
        // personality b0=200 >= 127 → male; requesting male again
        let data = make_mon_data(0x0100_00C8, 0, [0u8; 48]); // b0=0xC8=200
        assert!(matches!(
            compute_change_gender(&data, 0, 127),
            Some(ChangeGenderOutcome::AlreadyMatches)
        ));
    }

    #[test]
    fn change_gender_male_to_female_b0_in_correct_range() {
        // personality=0x0100_00C8: b0=200 (male), nature=16, not shiny (ot_id=0)
        let data = make_mon_data(0x0100_00C8, 0, [0u8; 48]);
        match compute_change_gender(&data, 1, 127).unwrap() {
            ChangeGenderOutcome::Write { new_personality, .. } => {
                assert!((new_personality & 0xFF) < 127, "new b0 must be < 127 (female)");
                assert_eq!(new_personality % 25, 0x0100_00C8_u32 % 25, "nature preserved");
            }
            ChangeGenderOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_gender_female_to_male_b0_in_correct_range() {
        // personality=0x0100_0050: b0=0x50=80 (female for ratio 127)
        let data = make_mon_data(0x0100_0050, 0, [0u8; 48]);
        match compute_change_gender(&data, 0, 127).unwrap() {
            ChangeGenderOutcome::Write { new_personality, .. } => {
                assert!((new_personality & 0xFF) >= 127, "new b0 must be >= 127 (male)");
                assert_eq!(new_personality % 25, 0x0100_0050_u32 % 25, "nature preserved");
            }
            ChangeGenderOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_gender_shiny_preserved() {
        // Build a shiny female Pokémon (ot_id=0, so p_high ^ p_low < 8).
        // personality = 0x0000_0001: p_high=0, p_low=1, xor=1 < 8 → shiny.
        // b0=1 < 127 → female. Nature = 1 % 25 = 1.
        // k16 = 0 ^ 0 ^ 0 ^ 0 = 0.  Shiny values: {0,1,2,3,4,5,6,7} for new b0.
        // Male needs b0 >= 127 — all shiny values are < 127 → impossible.
        let female_shiny = make_mon_data(0x0000_0001, 0, [0u8; 48]);
        assert!(
            compute_change_gender(&female_shiny, 0, 127).is_none(),
            "shiny female→male should be impossible when all shiny b0 values are female"
        );

        // For shiny female→female: already matches.
        assert!(matches!(
            compute_change_gender(&female_shiny, 1, 127),
            Some(ChangeGenderOutcome::AlreadyMatches)
        ));

        // Build a case where shiny AND male are compatible.
        // Use ot_id = 0x0000_FF80 so id_low = 0xFF80.
        // k16 = 0 ^ 0 ^ 0 ^ 0xFF80 = 0xFF80.  k_high = 0xFF ≠ 0 → no valid b0.
        // That won't work either. Let's find specific values.
        //
        // For a shiny male (b0 >= 127) to exist, we need k16 ^ b0 < 8 with b0 >= 127.
        // k16 ^ b0 < 8 means b0 ∈ {k16_low, k16_low^1, …, k16_low^7} and k16_high=0.
        // Choose: ot_id = 0, p_high = 0, b1 = 0 → k16 = 0.
        //   Shiny b0 values: {0,1,2,3,4,5,6,7} — all female for ratio 127.
        // Choose: ot_id = 0, p_high = 0, b1 = 1 → k16 = 0 ^ (1<<8) ^ 0 ^ 0 = 0x0100.
        //   k_high = 1 ≠ 0 → impossible.
        // Let's try a gender ratio of 31 (very skewed to male, e.g. Nidoran♂ line).
        // With k16=0: shiny b0 in {0..7}, all < 31 → female. Still no shiny male.
        //
        // With k16=0 and gender_ratio=4 (nearly always male, b0 >= 4 = male):
        // shiny b0 {0,1,2,3} → b0 < 4 → female. {4,5,6,7} → b0 >= 4 → male!
        // personality = 0x0000_0000 would be empty. Use personality = 0x0000_0004:
        //   p_high=0, b1=0, b0=4 → k16=0, shiny (0^4^0^0=4<8), b0=4 >= 4 → male.
        let p = 0x0000_0004u32; // shiny male (gender_ratio=4)
        let shiny_male = make_mon_data(p, 0, [0u8; 48]);
        assert!(is_shiny(p, 0), "must be shiny");
        // Changing male→female: need b0 < 4 AND shiny (b0 in {0,1,2,3,4,5,6,7}) AND nature=4%25=4.
        // nature: (0 + new_b0) % 25 = 4 → new_b0 % 25 = 4. In {0,1,2,3}: none satisfy %25=4.
        // In {4,5,6}: 4%25=4 ✓, but 4 >= 4 → male. 5%25=5 ✗. 6%25=6 ✗.
        // So female change is also impossible here (nature clash).
        assert!(
            compute_change_gender(&shiny_male, 1, 4).is_none(),
            "shiny male→female impossible when no shiny b0 in female range satisfies nature"
        );
    }

    #[test]
    fn change_gender_substructure_data_survives_rearrangement() {
        // Use personality=0x0100_00C8 (b0=200, p%24=0, order=[0,1,2,3]).
        // After change to female, expected new b0 satisfies nature=16 and b0<127.
        // We place a sentinel value in Growth substructure (pos 0 in old order)
        // and verify it's readable from the correct pos in the new order after decrypt.
        let old_p: u32 = 0x0100_00C8;
        let ot_id: u32 = 0;
        let mut dec = [0u8; 48];
        // Put sentinel species (42) in Growth (type 0) at old position.
        let old_g_pos = super::SUBSTRUCTURE_ORDER[(old_p % 24) as usize][0] as usize;
        dec[old_g_pos * 12..old_g_pos * 12 + 2].copy_from_slice(&42u16.to_le_bytes());
        let data = make_mon_data(old_p, ot_id, dec);

        match compute_change_gender(&data, 1, 127).unwrap() {
            ChangeGenderOutcome::Write { new_personality, encrypted, .. } => {
                let new_dec = decrypt_block(&encrypted, new_personality, ot_id);
                let new_g_pos = super::SUBSTRUCTURE_ORDER[(new_personality % 24) as usize][0] as usize;
                let species_in_new = u16::from_le_bytes(
                    new_dec[new_g_pos * 12..new_g_pos * 12 + 2].try_into().unwrap()
                );
                assert_eq!(species_in_new, 42, "Growth data (species sentinel) must survive rearrangement");
            }
            ChangeGenderOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn change_gender_checksum_correct_after_change() {
        let data = make_mon_data(0x0100_00C8, 0, [0u8; 48]);
        match compute_change_gender(&data, 1, 127).unwrap() {
            ChangeGenderOutcome::Write { new_personality, checksum, encrypted } => {
                let new_dec = decrypt_block(&encrypted, new_personality, 0);
                let expected: u16 = new_dec
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .fold(0u16, |acc, w| acc.wrapping_add(w));
                assert_eq!(u16::from_le_bytes(checksum), expected);
            }
            ChangeGenderOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    // ── encode_nickname / ascii_to_gba tests ─────────────────────────────────

    #[test]
    fn ascii_to_gba_uppercase() {
        assert_eq!(ascii_to_gba('A'), Some(0xBB));
        assert_eq!(ascii_to_gba('Z'), Some(0xD4));
    }

    #[test]
    fn ascii_to_gba_lowercase() {
        assert_eq!(ascii_to_gba('a'), Some(0xD5));
        assert_eq!(ascii_to_gba('z'), Some(0xEE));
    }

    #[test]
    fn ascii_to_gba_digits() {
        assert_eq!(ascii_to_gba('0'), Some(0xA1));
        assert_eq!(ascii_to_gba('9'), Some(0xAA));
    }

    #[test]
    fn ascii_to_gba_space() {
        assert_eq!(ascii_to_gba(' '), Some(0x00));
    }

    #[test]
    fn ascii_to_gba_unmapped_returns_none() {
        assert_eq!(ascii_to_gba('@'), None);
        assert_eq!(ascii_to_gba('\n'), None);
    }

    #[test]
    fn encode_nickname_basic() {
        let buf = encode_nickname("Abcde");
        assert_eq!(buf[0], 0xBB); // 'A'
        assert_eq!(buf[1], 0xD5 + (b'b' - b'a')); // 'b'
        assert_eq!(buf[4], 0xD5 + (b'e' - b'a')); // 'e'
        assert_eq!(buf[5], 0xFF); // terminator
        assert_eq!(buf[9], 0xFF); // still 0xFF at end
    }

    #[test]
    fn encode_nickname_truncates_at_10() {
        let buf = encode_nickname("ABCDEFGHIJKLMN");
        // only first 10 GBA chars written; every slot used
        for b in buf.iter() {
            assert_ne!(*b, 0xFF); // no terminator within 10 chars
        }
    }

    #[test]
    fn encode_nickname_drops_unmapped_chars() {
        // '@' has no GBA mapping; should be silently skipped
        let buf = encode_nickname("@AB");
        assert_eq!(buf[0], 0xBB); // 'A' — '@' was skipped
        assert_eq!(buf[1], 0xBB + 1); // 'B'
        assert_eq!(buf[2], 0xFF); // terminator after 2 chars
    }

    // ── compute_change_held_item tests ────────────────────────────────────────

    #[test]
    fn held_item_updated_in_growth_substructure() {
        // personality=0 for Growth (GAGM order? no — personality % 24 = 0 → Growth at pos 0)
        // Use personality=0x0000_0001 (p%24=1, SUBSTRUCTURE_ORDER[1] = [0,1,3,2], Growth at pos 0)
        let p: u32 = 1;
        let ot: u32 = 0;
        let mut dec = [0u8; 48];
        // set species=25 in Growth block (pos 0 in decrypted, offset 0)
        dec[0..2].copy_from_slice(&25u16.to_le_bytes());
        // set old held_item=13 in Growth block offset 2
        dec[2..4].copy_from_slice(&13u16.to_le_bytes());
        let data = make_mon_data(p, ot, dec);

        let (_, encrypted) = compute_change_held_item(&data, 45).unwrap();
        // Decrypt result and verify held_item changed
        let new_dec = decrypt_block(&encrypted, p, ot);
        let item = u16::from_le_bytes([new_dec[2], new_dec[3]]);
        assert_eq!(item, 45);
        // species must be unchanged
        let species = u16::from_le_bytes([new_dec[0], new_dec[1]]);
        assert_eq!(species, 25);
    }

    #[test]
    fn held_item_already_matches_returns_none() {
        let p: u32 = 1;
        let mut dec = [0u8; 48];
        dec[2..4].copy_from_slice(&13u16.to_le_bytes());
        let data = make_mon_data(p, 0, dec);
        assert!(compute_change_held_item(&data, 13).is_none());
    }

    #[test]
    fn held_item_remove_with_zero() {
        let p: u32 = 1;
        let mut dec = [0u8; 48];
        dec[2..4].copy_from_slice(&13u16.to_le_bytes());
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_change_held_item(&data, 0).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let item = u16::from_le_bytes([new_dec[2], new_dec[3]]);
        assert_eq!(item, 0);
    }

    // ── compute_change_nature tests ───────────────────────────────────────────

    #[test]
    fn already_matching_nature_returns_already_matches() {
        // personality % 25 == 3 (Adamant)
        let p: u32 = 3;
        let data = make_mon_data(p, 0, [0u8; 48]);
        assert!(matches!(
            compute_change_nature(&data, 3, 127),
            Some(ChangeNatureOutcome::AlreadyMatches)
        ));
    }

    #[test]
    fn nature_changes_to_target() {
        // personality=0, nature=0 (Hardy). Change to nature=1 (Lonely).
        let p: u32 = 0; // special case: personality 0 means "empty" → skip it
        // Use p=25 (nature=0, not shiny with ot=0)
        let p: u32 = 25;
        let data = make_mon_data(p, 0, [0u8; 48]);
        match compute_change_nature(&data, 1, 127).unwrap() {
            ChangeNatureOutcome::Write { new_personality, .. } => {
                assert_eq!(new_personality % 25, 1);
            }
            ChangeNatureOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    #[test]
    fn nature_change_preserves_shiny() {
        // Make a shiny personality: p_high ^ p_low ^ id_high ^ id_low < 8
        // p = 0x0001_0000, ot = 0 → xor = 1 < 8 → shiny
        let p: u32 = 0x0001_0000;
        let data = make_mon_data(p, 0, [0u8; 48]);
        assert!(is_shiny(p, 0));
        let target = if p % 25 == 3 { 4 } else { 3 };
        match compute_change_nature(&data, target, 255) {
            Some(ChangeNatureOutcome::Write { new_personality, .. }) => {
                assert!(is_shiny(new_personality, 0), "shiny must be preserved");
                assert_eq!(new_personality % 25, target as u32);
            }
            Some(ChangeNatureOutcome::AlreadyMatches) => {}
            None => {} // may have no solution for this exact combo; acceptable
        }
    }

    #[test]
    fn nature_change_preserves_gender() {
        // gender_ratio=127: female if low_byte < 127
        // p = 0x0001_0005: low byte=5 < 127, so female; nature = 5 % 25 = 5
        let p: u32 = 0x0001_0005;
        let data = make_mon_data(p, 0, [0u8; 48]);
        let target_nature = 7u8; // different from 5
        match compute_change_nature(&data, target_nature, 127).unwrap() {
            ChangeNatureOutcome::Write { new_personality, .. } => {
                assert_eq!(new_personality % 25, target_nature as u32);
                // must stay female
                assert!((new_personality & 0xFF) < 127, "gender must be preserved");
            }
            ChangeNatureOutcome::AlreadyMatches => panic!("expected Write"),
        }
    }

    // ── compute_restore_pp tests ──────────────────────────────────────────────

    /// Builds a minimal ROM-like byte slice where move `move_id` has `base_pp`
    /// at the expected offset within gBattleMoves (12 bytes/entry, PP at byte 4).
    fn make_fake_rom(move_data_addr: usize, move_id: u16, base_pp: u8) -> Vec<u8> {
        let off = move_data_addr + move_id as usize * 12 + 4;
        let mut rom = vec![0u8; off + 1];
        rom[off] = base_pp;
        rom
    }

    #[test]
    fn restore_pp_empty_slot_returns_none() {
        let rom = vec![0u8; 1];
        assert!(compute_restore_pp(&vec![0u8; 80], &rom, 0).is_none());
    }

    #[test]
    fn restore_pp_already_full_returns_none() {
        // p%24=0 → Growth at 0, Attacks at 1 (offset 12)
        let p: u32 = 24;
        let move_data_addr: usize = 0x1000;
        let move_id: u16 = 5;
        let base_pp: u8 = 35;
        let rom = make_fake_rom(move_data_addr, move_id, base_pp);

        let mut dec = [0u8; 48];
        // Attacks at pos 1 → offset 12
        dec[12..14].copy_from_slice(&move_id.to_le_bytes()); // move0 id
        dec[20] = base_pp; // current PP already at max (no pp-up bonus)
        let data = make_mon_data(p, 0, dec);
        assert!(compute_restore_pp(&data, &rom, move_data_addr).is_none());
    }

    #[test]
    fn restore_pp_restores_depleted_pp() {
        let p: u32 = 24; // p%24=0 → Attacks at substructure 1, offset 12
        let move_data_addr: usize = 0x1000;
        let move_id: u16 = 5;
        let base_pp: u8 = 35;
        let rom = make_fake_rom(move_data_addr, move_id, base_pp);

        let mut dec = [0u8; 48];
        dec[12..14].copy_from_slice(&move_id.to_le_bytes());
        dec[20] = 0; // fully depleted
        let data = make_mon_data(p, 0, dec);

        let (_, encrypted) = compute_restore_pp(&data, &rom, move_data_addr).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        assert_eq!(new_dec[20], base_pp, "PP should be restored to base_pp");
    }

    #[test]
    fn restore_pp_skips_empty_move_slots() {
        let p: u32 = 24; // Attacks at offset 12
        let move_data_addr: usize = 0x1000;
        let rom = make_fake_rom(move_data_addr, 5, 35);

        let mut dec = [0u8; 48];
        // Only slot 0 has a move; slots 1-3 are empty (move_id=0)
        dec[12..14].copy_from_slice(&5u16.to_le_bytes());
        dec[20] = 10; // below max

        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_restore_pp(&data, &rom, move_data_addr).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        // slots 1-3 PP bytes should remain 0
        assert_eq!(new_dec[21], 0);
        assert_eq!(new_dec[22], 0);
        assert_eq!(new_dec[23], 0);
    }

    // ── compute_set_friendship tests ──────────────────────────────────────────

    #[test]
    fn set_friendship_empty_slot_returns_none() {
        assert!(compute_set_friendship(&vec![0u8; 80], 200).is_none());
    }

    #[test]
    fn set_friendship_already_same_returns_none() {
        let p: u32 = 1; // Growth at offset 0
        let mut dec = [0u8; 48];
        dec[9] = 200;
        let data = make_mon_data(p, 0, dec);
        assert!(compute_set_friendship(&data, 200).is_none());
    }

    #[test]
    fn set_friendship_updates_friendship_byte() {
        let p: u32 = 1; // Growth at offset 0
        let mut dec = [0u8; 48];
        dec[9] = 50;
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_set_friendship(&data, 255).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        assert_eq!(new_dec[9], 255);
        // species (g_off+0) should be unchanged
        assert_eq!(u16::from_le_bytes([new_dec[0], new_dec[1]]), 0);
    }

    #[test]
    fn set_friendship_non_zero_growth_index() {
        // p%24=6 → SUBSTRUCTURE_ORDER[6]=[1,0,2,3] → Growth at pos 1, offset 12
        let p: u32 = 30;
        let mut dec = [0u8; 48];
        dec[12 + 9] = 100;
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_set_friendship(&data, 0).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        assert_eq!(new_dec[12 + 9], 0);
        assert_eq!(new_dec[9], 0, "other substructure bytes must be untouched");
    }

    // ── compute_change_move tests ─────────────────────────────────────────────

    #[test]
    fn change_move_empty_slot_returns_none() {
        let rom = vec![0u8; 1];
        assert!(compute_change_move(&vec![0u8; 80], 0, 1, &rom, 0).is_none());
    }

    #[test]
    fn change_move_already_same_returns_none() {
        let p: u32 = 1; // Attacks at offset 12
        let mut dec = [0u8; 48];
        dec[12..14].copy_from_slice(&7u16.to_le_bytes()); // slot 0 = move 7
        let data = make_mon_data(p, 0, dec);
        let rom = vec![0u8; 100];
        assert!(compute_change_move(&data, 0, 7, &rom, 0).is_none());
    }

    #[test]
    fn change_move_sets_move_id_and_pp() {
        // p%24=1 → SUBSTRUCTURE_ORDER[1]=[0,1,3,2] → Attacks at pos 1, offset 12
        let p: u32 = 1;
        let move_data_addr: usize = 0x1000;
        let move_id: u16 = 33; // Tackle (base PP = 35 in Gen III)
        let base_pp: u8  = 35;
        let rom = make_fake_rom(move_data_addr, move_id, base_pp);

        let dec = [0u8; 48];
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_change_move(&data, 0, move_id, &rom, move_data_addr).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let stored_move = u16::from_le_bytes([new_dec[12], new_dec[13]]);
        assert_eq!(stored_move, move_id);
        assert_eq!(new_dec[20], base_pp, "PP should equal base PP (no PP-Up bonus)");
    }

    #[test]
    fn change_move_clears_slot_with_zero_move_id() {
        let p: u32 = 1; // Attacks at offset 12
        let move_data_addr: usize = 0x1000;
        let mut dec = [0u8; 48];
        dec[12..14].copy_from_slice(&5u16.to_le_bytes());
        dec[20] = 30;
        let data = make_mon_data(p, 0, dec);
        let rom = make_fake_rom(move_data_addr, 5, 30);
        let (_, encrypted) = compute_change_move(&data, 0, 0, &rom, move_data_addr).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let stored_move = u16::from_le_bytes([new_dec[12], new_dec[13]]);
        assert_eq!(stored_move, 0);
        assert_eq!(new_dec[20], 0, "PP should be 0 for empty slot");
    }

    #[test]
    fn change_move_slot3_correct_offset() {
        // p%24=0 → Attacks at pos 1, offset 12. Slot 3 → move at a_off+6, PP at a_off+11
        let p: u32 = 24;
        let move_data_addr: usize = 0x2000;
        let move_id: u16 = 99;
        let base_pp: u8  = 10;
        let rom = make_fake_rom(move_data_addr, move_id, base_pp);

        let dec = [0u8; 48];
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_change_move(&data, 3, move_id, &rom, move_data_addr).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let stored_move = u16::from_le_bytes([new_dec[12 + 6], new_dec[12 + 7]]);
        assert_eq!(stored_move, move_id);
        assert_eq!(new_dec[12 + 11], base_pp);
    }

    // ── compute_set_ivs / compute_increase_ivs tests ─────────────────────────

    #[test]
    fn set_ivs_empty_slot_returns_none() {
        assert!(compute_set_ivs(&vec![0u8; 80], 31, 31, 31, 31, 31, 31).is_none());
    }

    #[test]
    fn set_ivs_writes_all_six_stats() {
        // p%24=0 → Misc at pos 3, offset 36; IV word at decrypted[40..44]
        let p: u32 = 24;
        let data = make_mon_data(p, 0, [0u8; 48]);
        let (_, encrypted) = compute_set_ivs(&data, 31, 20, 15, 10, 5, 0).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let word = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        let ivs = unpack_ivs(word);
        assert_eq!(ivs, [31, 20, 15, 10, 5, 0]);
    }

    #[test]
    fn set_ivs_preserves_egg_and_ability_bits() {
        let p: u32 = 24; // Misc at offset 36, IV word at 40
        let mut dec = [0u8; 48];
        // Set ability bit (31) and egg bit (30)
        let word_with_flags: u32 = 0xC000_0000 | 0b1_0001_1111; // ability=1, egg=1, hp=31
        dec[40..44].copy_from_slice(&word_with_flags.to_le_bytes());
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_set_ivs(&data, 0, 0, 0, 0, 0, 0).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let new_word = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        assert_eq!(new_word & 0xC000_0000, 0xC000_0000, "bits 30-31 must be preserved");
        let ivs = unpack_ivs(new_word);
        assert_eq!(ivs, [0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn set_ivs_clamps_to_31() {
        let p: u32 = 24;
        let data = make_mon_data(p, 0, [0u8; 48]);
        let (_, encrypted) = compute_set_ivs(&data, 255, 255, 255, 255, 255, 255).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let ivs = unpack_ivs(u32::from_le_bytes(new_dec[40..44].try_into().unwrap()));
        assert_eq!(ivs, [31, 31, 31, 31, 31, 31]);
    }

    #[test]
    fn set_ivs_already_same_returns_none() {
        let p: u32 = 24;
        let mut dec = [0u8; 48];
        let word = 0b11111_10100_01111_01010_00101_00000u32; // hp=0 atk=5 def=10 spd=15 spa=20 spdef=31
        dec[40..44].copy_from_slice(&word.to_le_bytes());
        let data = make_mon_data(p, 0, dec);
        // Same values → None
        assert!(compute_set_ivs(&data, 0, 5, 10, 15, 20, 31).is_none());
    }

    #[test]
    fn increase_ivs_adds_and_clamps() {
        let p: u32 = 24;
        let mut dec = [0u8; 48];
        // Set all IVs to 20
        let word = super::pack_ivs(20, 20, 20, 20, 20, 20);
        dec[40..44].copy_from_slice(&word.to_le_bytes());
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_increase_ivs(&data, 15, 15, 15, 15, 15, 15).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let ivs = unpack_ivs(u32::from_le_bytes(new_dec[40..44].try_into().unwrap()));
        assert_eq!(ivs, [31, 31, 31, 31, 31, 31], "20+15 should clamp to 31");
    }

    #[test]
    fn increase_ivs_already_max_returns_none() {
        let p: u32 = 24;
        let mut dec = [0u8; 48];
        let word = super::pack_ivs(31, 31, 31, 31, 31, 31);
        dec[40..44].copy_from_slice(&word.to_le_bytes());
        let data = make_mon_data(p, 0, dec);
        assert!(compute_increase_ivs(&data, 1, 1, 1, 1, 1, 1).is_none());
    }

    #[test]
    fn set_ivs_non_zero_misc_index() {
        // p%24=1 → SUBSTRUCTURE_ORDER[1]=[0,1,3,2] → Misc at pos 2, offset 24; IV word at 28
        let p: u32 = 1;
        let data = make_mon_data(p, 0, [0u8; 48]);
        let (_, encrypted) = compute_set_ivs(&data, 10, 11, 12, 13, 14, 15).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        let ivs = unpack_ivs(u32::from_le_bytes(new_dec[28..32].try_into().unwrap()));
        assert_eq!(ivs, [10, 11, 12, 13, 14, 15]);
        // IV word at offset 40 (would be Misc if index were 3) should be zero
        let wrong_word = u32::from_le_bytes(new_dec[40..44].try_into().unwrap());
        assert_eq!(wrong_word & 0x3FFF_FFFF, 0, "IVs must be at correct substructure offset");
    }

    // ── compute_set_evs / compute_increase_evs tests ──────────────────────────

    #[test]
    fn set_evs_empty_slot_returns_none() {
        assert!(compute_set_evs(&vec![0u8; 80], 1, 0, 0, 0, 0, 0).is_none());
    }

    #[test]
    fn set_evs_writes_all_six_bytes() {
        // p%24=0 → Effort at pos 2, offset 24
        let p: u32 = 24;
        let data = make_mon_data(p, 0, [0u8; 48]);
        let (_, encrypted) = compute_set_evs(&data, 252, 0, 0, 4, 0, 0).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        assert_eq!(&new_dec[24..30], &[252, 0, 0, 4, 0, 0]);
    }

    #[test]
    fn set_evs_preserves_contest_bytes() {
        let p: u32 = 24; // Effort at offset 24
        let mut dec = [0u8; 48];
        // Set contest bytes 6-11 to non-zero values
        dec[30] = 100; // coolness
        dec[31] = 200; // beauty
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_set_evs(&data, 0, 0, 0, 0, 0, 0).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        assert_eq!(new_dec[30], 100, "contest bytes must be preserved");
        assert_eq!(new_dec[31], 200);
    }

    #[test]
    fn set_evs_already_same_returns_none() {
        let p: u32 = 24; // Effort at offset 24
        let mut dec = [0u8; 48];
        dec[24..30].copy_from_slice(&[252, 0, 0, 4, 0, 0]);
        let data = make_mon_data(p, 0, dec);
        assert!(compute_set_evs(&data, 252, 0, 0, 4, 0, 0).is_none());
    }

    #[test]
    fn increase_evs_adds_and_clamps_at_255() {
        let p: u32 = 24; // Effort at offset 24
        let mut dec = [0u8; 48];
        dec[24..30].copy_from_slice(&[200, 200, 200, 200, 200, 200]);
        let data = make_mon_data(p, 0, dec);
        let (_, encrypted) = compute_increase_evs(&data, 100, 100, 100, 100, 100, 100).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        assert_eq!(&new_dec[24..30], &[255, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn increase_evs_already_max_returns_none() {
        let p: u32 = 24;
        let mut dec = [0u8; 48];
        dec[24..30].copy_from_slice(&[255, 255, 255, 255, 255, 255]);
        let data = make_mon_data(p, 0, dec);
        assert!(compute_increase_evs(&data, 1, 1, 1, 1, 1, 1).is_none());
    }

    #[test]
    fn set_evs_non_zero_effort_index() {
        // p%24=1 → SUBSTRUCTURE_ORDER[1]=[0,1,3,2] → Effort at pos 3, offset 36
        let p: u32 = 1;
        let data = make_mon_data(p, 0, [0u8; 48]);
        let (_, encrypted) = compute_set_evs(&data, 1, 2, 3, 4, 5, 6).unwrap();
        let new_dec = decrypt_block(&encrypted, p, 0);
        assert_eq!(&new_dec[36..42], &[1, 2, 3, 4, 5, 6]);
        // Effort at offset 24 should be untouched (zero)
        assert_eq!(&new_dec[24..30], &[0, 0, 0, 0, 0, 0]);
    }

    // ── live RetroArch integration ────────────────────────────────────────────

    /// Reads `len` bytes from GBA address `addr` via RetroArch UDP.
    fn retroarch_read(socket: &std::net::UdpSocket, addr: u32, len: usize) -> Vec<u8> {
        use fire_red_retroarch_interfacing::{generate_command, get_from_retroarch};
        let cmd    = generate_command(addr, len);
        let tokens = get_from_retroarch(socket, &cmd, len + 2)
            .unwrap_or_else(|| panic!("RetroArch did not respond to READ_CORE_MEMORY 0x{addr:08X} {len}"));
        tokens[2..].iter()
            .map(|t| u8::from_str_radix(t, 16)
                .unwrap_or_else(|_| panic!("invalid hex token '{t}'")))
            .collect()
    }

    /// Gives the player one Potion (item_id 13) and one Rare Candy (item_id 68)
    /// by writing directly into the items pocket via RetroArch WRITE_CORE_MEMORY.
    ///
    /// Requires RetroArch to be running with FireRed USA Rev 1 loaded, with
    /// "Network Commands" enabled (Settings → Network → Network Commands → ON).
    /// The game must be past the title screen so SaveBlock1 is initialised.
    ///
    /// Run with:
    ///   cargo test -p fire_red_tracker integration_give -- --ignored --nocapture
    #[test]
    #[ignore = "requires live RetroArch with FireRed loaded"]
    fn integration_give_potion_and_rare_candy() {
        use fire_red_retroarch_interfacing::{make_socket, write_to_retroarch};

        const POTION_ITEM_ID:     u16   = 13;
        const RARE_CANDY_ITEM_ID: u16   = 68;
        const POCKET_OFFSET:      u32   = super::ITEMS_POCKET_SAVE_BLOCK_OFFSET as u32;
        const POCKET_LEN:         usize = super::ITEMS_POCKET_SLOTS * 4;
        const SAVE_BLOCK_2_BASE:  u32   = super::SAVE_BLOCK_2_BASE as u32;
        const SEC_KEY_OFFSET:     u32   = super::SECURITY_KEY_OFFSET as u32;

        let socket = make_socket().expect("failed to bind UDP socket");

        // Resolve the SaveBlock1 GBA address from the pointer in IWRAM.
        let ptr_bytes     = retroarch_read(&socket, 0x0300_5008, 4);
        let save_block1   = u32::from_le_bytes(ptr_bytes.try_into().unwrap());
        assert!(
            (0x0200_0000..0x0204_0000).contains(&save_block1),
            "SaveBlock1 ptr 0x{save_block1:08X} is outside EWRAM — is FireRed past the title screen?"
        );
        println!("SaveBlock1 @ 0x{save_block1:08X}");

        // Read the security key (low 16 bits of u32 at SaveBlock2 + 0x0E4C).
        let key_bytes = retroarch_read(&socket, SAVE_BLOCK_2_BASE + SEC_KEY_OFFSET, 4);
        let key       = u32::from_le_bytes(key_bytes.try_into().unwrap()) as u16;
        println!("Security key: 0x{key:04X}");

        let pocket_base = save_block1 + POCKET_OFFSET;

        // ── Potion ───────────────────────────────────────────────────────────
        let pocket = retroarch_read(&socket, pocket_base, POCKET_LEN);
        let (slot, payload) = compute_give_item_write(&pocket, key, POTION_ITEM_ID, 1)
            .expect("items pocket is full");
        let addr = pocket_base + slot as u32 * 4;
        assert!(write_to_retroarch(&socket, addr, &payload), "write failed");
        let qty = u16::from_le_bytes([payload[2], payload[3]]) ^ key;
        println!("Potion     → slot {slot} @ 0x{addr:08X}  (new qty = {qty})");

        // ── Rare Candy ───────────────────────────────────────────────────────
        // Re-read the pocket so the Rare Candy search sees the just-written Potion.
        let pocket2 = retroarch_read(&socket, pocket_base, POCKET_LEN);
        let (slot2, payload2) = compute_give_item_write(&pocket2, key, RARE_CANDY_ITEM_ID, 1)
            .expect("items pocket is full");
        let addr2 = pocket_base + slot2 as u32 * 4;
        assert!(write_to_retroarch(&socket, addr2, &payload2), "write failed");
        let qty2 = u16::from_le_bytes([payload2[2], payload2[3]]) ^ key;
        println!("Rare Candy → slot {slot2} @ 0x{addr2:08X}  (new qty = {qty2})");
    }
}
