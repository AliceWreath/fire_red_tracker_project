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
        [k] => {
            tracing::info!("give_item: key=0x{k:04X} from pocket oracle ({} slots)", all_raw.len());
            (Some(*k), vec![*k])
        }
        _ => {
            tracing::warn!(
                "give_item: pocket oracle: {} candidates from {} slots",
                candidates.len(), all_raw.len()
            );
            (None, candidates)
        }
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
                    tracing::info!("give_item: key=0x{v:04X} from SaveBlock2+0x{off:04X}");
                    return v;
                }
            }
        }
    }
    tracing::warn!("give_item: all SaveBlock2 key reads returned zero, using key=0x0000");
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
    use super::{compute_give_item_write, is_shiny, ITEMS_POCKET_SLOTS, MAX_ITEM_QTY};

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
