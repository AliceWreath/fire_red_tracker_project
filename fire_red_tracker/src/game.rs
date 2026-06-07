//! # Game state helpers
//!
//! Utilities for reading live game state directly from the EWRAM/IWRAM
//! snapshots maintained by `fire_red_memory`, bypassing the polling-thread
//! intermediaries in `fire_red_loop` to avoid lag and race conditions.

use fire_red_loop::FireRedState;
use fire_red_party_monitor::Pokemon;
use fire_red_states::MAX_NATIONAL_DEX_FIRERED;
use std::sync::{Arc, Mutex};

trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for Mutex<T> {
    #[track_caller]
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            let loc = std::panic::Location::caller();
            eprintln!("Warning: mutex poisoned at {}:{}: {e}", loc.file(), loc.line());
            e.into_inner()
        })
    }
}

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

/// Byte offset of the balls pocket (13 × ItemSlot) within SaveBlock1.
/// Confirmed empirically via --scan-balls-pocket: Pokéball (item_id=4) first
/// appears at slot 0 of the window starting here, with all other slots empty.
const BALLS_POCKET_SAVE_BLOCK_OFFSET: usize = 0x0430;

/// Number of slots in the balls pocket.
const BALLS_POCKET_SLOTS: usize = 13;

/// Fixed EWRAM base address of SaveBlock2 for FireRed USA Rev 1.
/// Same address used by `fire_red_trainer_data`.
const SAVE_BLOCK_2_BASE: usize = 0x02024298;

/// Byte offset of `securityKey` (u32) within SaveBlock2.
/// Confirmed empirically via --scan-security-key: raw_qty 0x91B5 ^ key_low 0x91B0 = 5.
const SECURITY_KEY_OFFSET: usize = 0x0E4C;

/// Minimum number of Pokéballs required for the run-start latch to trigger.
const RUN_START_BALL_THRESHOLD: u32 = 5;

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
    let party = thread_party.lock_or_recover();
    for pokemon in party.iter() {
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

        let died_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let shiny_flag = is_shiny(personality, ot_id);
        fire_red_database::mark_dead(fire_red_database::DeadPokemon {
            player_name:   String::new(), // populated from DbState::current_player by mark_dead
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
        });
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
    let party = thread_party.lock_or_recover();
    for pokemon in party.iter() {
        let species = pokemon.box_mon.secure.growth.species;
        if species == 0 || species > MAX_NATIONAL_DEX_FIRERED { continue; }
        let personality = pokemon.box_mon.personality;
        if fire_red_database::is_caught(personality) {
            let nickname = &pokemon.box_mon.nickname_string;
            if !nickname.is_empty() {
                fire_red_database::update_caught_nickname(personality, nickname);
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

        let ot_id    = pokemon.box_mon.ot_id;
        let growth   = &pokemon.box_mon.secure.growth;
        let misc     = &pokemon.box_mon.secure.misc;
        let iv       = &misc.iv_egg_ability;
        let ev       = &pokemon.box_mon.secure.ev_condition;

        let caught_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let location_name = map_state_from_ewram()
            .map(|s| {
                let n = fire_red_loop::get_area_name_for(s.map_group_id, s.map_name_id);
                if n.is_empty() {
                    format!("{}\u{B7}{}", s.map_group_id, s.map_name_id)
                } else {
                    n.to_string()
                }
            })
            .unwrap_or_default();

        let shiny_flag = is_shiny(personality, ot_id);
        fire_red_database::mark_caught(fire_red_database::CaughtPokemon {
            player_name:   String::new(), // populated from DbState::current_player by mark_caught
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

    // The SaveBlock1 pointer lives in IWRAM at 0x03005008.
    let ptr_offset = SAVE_BLOCK_1_PTR_ADDR - IWRAM_BASE;
    if iwram.len() < ptr_offset + 4 {
        return false;
    }
    let save_block_ptr = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;

    // Pointer must fall within EWRAM.
    if save_block_ptr < EWRAM_BASE || save_block_ptr >= EWRAM_BASE + ewram.len() {
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
        fire_red_database::end_run();
        crate::webhook::fire_event(crate::webhook::WebhookEvent::Wipe {
            player:    fire_red_loop::get_trainer_name(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
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

    let ptr_offset = SAVE_BLOCK_1_PTR_ADDR - IWRAM_BASE;
    if iwram.len() < ptr_offset + 4 {
        eprintln!("IWRAM too small to read SaveBlock1 pointer.");
        return;
    }

    let save_block_ptr = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;

    println!("SaveBlock1 ptr: 0x{:08X}", save_block_ptr);

    if save_block_ptr < EWRAM_BASE || save_block_ptr >= EWRAM_BASE + ewram.len() {
        eprintln!("SaveBlock1 ptr 0x{:08X} is outside EWRAM — is the game loaded?", save_block_ptr);
        return;
    }

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
    let ptr_offset = SAVE_BLOCK_1_PTR_ADDR - IWRAM_BASE;
    if iwram.len() < ptr_offset + 4 {
        eprintln!("IWRAM too small to read SaveBlock1 pointer.");
        return;
    }
    let save_block_ptr = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;
    if save_block_ptr < EWRAM_BASE || save_block_ptr >= EWRAM_BASE + ewram.len() {
        eprintln!("SaveBlock1 ptr 0x{:08X} is outside EWRAM — is the game loaded?", save_block_ptr);
        return;
    }

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
            raw_qty = Some(u16::from_le_bytes([ewram[base + 2], ewram[base + 3]]));
            println!("Slot {:2}: item_id={:2}  raw_qty_bytes=0x{:04X}",
                slot, item_id, raw_qty.unwrap());
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
            println!("  EWRAM offset 0x{:05X}  GBA 0x{:08X}  SaveBlock2+0x{:04X}",
                off, gba_addr, sb2_rel as usize);
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

    let ptr_offset = SAVE_BLOCK_1_PTR_ADDR - IWRAM_BASE;
    if iwram.len() < ptr_offset + 4 {
        return 0;
    }

    let save_block_ptr = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;

    if save_block_ptr < EWRAM_BASE || save_block_ptr >= EWRAM_BASE + ewram.len() {
        return 0;
    }

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

/// Returns `true` if the player has at least `RUN_START_BALL_THRESHOLD` Pokéballs.
///
/// Used to gate encounter and death tracking — the run officially begins once
/// the player has accumulated enough balls to be considered ready.
/// Returns `false` on read failure so pre-ball encounters are silently skipped.
pub fn has_pokeballs() -> bool {
    count_pokeballs() >= RUN_START_BALL_THRESHOLD
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

#[cfg(test)]
mod tests {
    use super::is_shiny;

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
}
