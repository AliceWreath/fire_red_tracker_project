//! # Game state helpers
//!
//! Utilities for reading live game state directly from the EWRAM/IWRAM
//! snapshots maintained by `fire_red_memory`, bypassing the polling-thread
//! intermediaries in `fire_red_loop` to avoid lag and race conditions.

use fire_red_loop::FireRedState;
use fire_red_party_monitor::Pokemon;
use std::sync::{Arc, Mutex};

/// GBA address of the packed (map_group, map_name) bytes in EWRAM.
pub const MAP_GROUP_AND_NAME_ADDR: usize = 0x02031DBC;

/// Base address of EWRAM in the GBA address space.
pub const EWRAM_BASE: usize = 0x02000000;

/// Base address of IWRAM in the GBA address space.
pub const IWRAM_BASE: usize = 0x03000000;

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
    *thread_party.lock().unwrap_or_else(|e| e.into_inner()) =
        fire_red_loop::get_party_members();
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
    let ptr_offset = 0x03005008 - IWRAM_BASE;
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
