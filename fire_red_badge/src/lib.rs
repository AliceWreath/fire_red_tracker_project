//! # Fire Red Badge Monitor
//!
//! Reads the player's badge flags from SaveBlock1 and exposes which badges
//! have been obtained, as well as information about the next gym leader.
//!
//! # Memory layout
//!
//! SaveBlock1 is dynamically allocated — its base address is not fixed. At
//! runtime the GBA stores a pointer to SaveBlock1 at `SAVE_BLOCK_1_PTR`
//! (`0x03005008`) in IWRAM. Reading badge state therefore requires two steps:
//!
//! 1. Read 4 bytes from the IWRAM snapshot at the IWRAM offset of
//!    `SAVE_BLOCK_1_PTR` to obtain the SaveBlock1 base address.
//! 2. Read the flags array from the EWRAM snapshot at
//!    `(base - EWRAM_BASE) + FLAGS_OFFSET`.
//!
//! The flags array starts at offset `0x0EE0` within SaveBlock1. Badge flags
//! occupy flag indices `0x820`–`0x827`, one bit per badge:
//!
//! | Index  | Badge         | Leader    |
//! |--------|---------------|-----------|
//! | 0x820  | Boulder Badge | Brock     |
//! | 0x821  | Cascade Badge | Misty     |
//! | 0x822  | Thunder Badge | Lt. Surge |
//! | 0x823  | Rainbow Badge | Erika     |
//! | 0x824  | Soul Badge    | Koga      |
//! | 0x825  | Marsh Badge   | Sabrina   |
//! | 0x826  | Volcano Badge | Blaine    |
//! | 0x827  | Earth Badge   | Giovanni  |
//!
//! Each flag index maps to: `byte = index / 8`, `bit = index % 8` within the
//! flags array.

// ---------------------------------------------------------------------------
// Address constants
// ---------------------------------------------------------------------------

/// Base address of IWRAM in the GBA address space.
const IWRAM_BASE: usize = 0x03000000;

/// Base address of EWRAM in the GBA address space.
const EWRAM_BASE: usize = 0x02000000;

/// IWRAM address of the pointer to SaveBlock1.
///
/// Dereferencing this 4-byte little-endian pointer yields the runtime base
/// address of SaveBlock1, which lies somewhere in EWRAM.
const SAVE_BLOCK_1_PTR: usize = 0x03005008;

/// Byte offset of the flags array within SaveBlock1.
const FLAGS_OFFSET: usize = 0x0EE0;

/// Flag index of the first badge (Boulder Badge / Brock).
const BADGE_FLAG_START: usize = 0x820;

/// Total number of badges.
const NUM_BADGES: usize = 8;

// ---------------------------------------------------------------------------
// Offset helpers
// ---------------------------------------------------------------------------

/// Converts an absolute GBA IWRAM address to a byte offset within the IWRAM
/// snapshot buffer.
#[inline]
fn iwram_offset(addr: usize) -> usize {
    debug_assert!(addr >= IWRAM_BASE, "address 0x{:08X} is below IWRAM_BASE", addr);
    addr - IWRAM_BASE
}

/// Converts an absolute GBA EWRAM address to a byte offset within the EWRAM
/// snapshot buffer.
#[inline]
fn ewram_offset(addr: usize) -> usize {
    debug_assert!(addr >= EWRAM_BASE, "address 0x{:08X} is below EWRAM_BASE", addr);
    addr - EWRAM_BASE
}

// ---------------------------------------------------------------------------
// Badge data
// ---------------------------------------------------------------------------

/// The full badge state for the current player.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BadgeState {
    /// One entry per badge, in gym order. `true` = obtained.
    pub badges: [bool; NUM_BADGES],

    /// The next gym leader the player hasn't beaten yet, or `None` if all
    /// badges have been obtained.
    pub next_gym: Option<GymInfo>,
}

impl BadgeState {
    /// Returns how many badges the player currently holds.
    pub fn count(&self) -> usize {
        self.badges.iter().filter(|&&b| b).count()
    }

    /// Returns `true` if all 8 badges have been obtained.
    pub fn all_obtained(&self) -> bool {
        self.badges.iter().all(|&b| b)
    }
}

/// Information about a single gym leader.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GymInfo {
    /// Gym leader name.
    pub leader: String,
    /// City the gym is located in.
    pub city: String,
    /// Name of the badge awarded on victory.
    pub badge: String,
    /// Highest level pokemon on the leader's team in FireRed.
    pub max_level: u8,
}

// ---------------------------------------------------------------------------
// Gym leader table
// ---------------------------------------------------------------------------

/// Static table of all 8 Kanto gym leaders in order.
fn gym_leaders() -> [GymInfo; NUM_BADGES] {
    [
        GymInfo { leader: "Brock".into(),     city: "Pewter City".into(),    badge: "Boulder Badge".into(), max_level: 14 },
        GymInfo { leader: "Misty".into(),     city: "Cerulean City".into(),  badge: "Cascade Badge".into(), max_level: 21 },
        GymInfo { leader: "Lt. Surge".into(), city: "Vermilion City".into(), badge: "Thunder Badge".into(), max_level: 24 },
        GymInfo { leader: "Erika".into(),     city: "Celadon City".into(),   badge: "Rainbow Badge".into(), max_level: 29 },
        GymInfo { leader: "Koga".into(),      city: "Fuchsia City".into(),   badge: "Soul Badge".into(),    max_level: 43 },
        GymInfo { leader: "Sabrina".into(),   city: "Saffron City".into(),   badge: "Marsh Badge".into(),   max_level: 50 },
        GymInfo { leader: "Blaine".into(),    city: "Cinnabar Island".into(),badge: "Volcano Badge".into(), max_level: 54 },
        GymInfo { leader: "Giovanni".into(),  city: "Viridian City".into(),  badge: "Earth Badge".into(),   max_level: 55 },
    ]
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Reads the current badge state from the IWRAM and EWRAM snapshots.
///
/// # How it works
///
/// 1. Reads 4 bytes from the IWRAM snapshot at the offset of
///    [`SAVE_BLOCK_1_PTR`] to obtain the runtime SaveBlock1 base address.
/// 2. Validates that the resolved address falls within EWRAM.
/// 3. Reads 2 bytes from the EWRAM snapshot at
///    `(base - EWRAM_BASE) + FLAGS_OFFSET + (BADGE_FLAG_START / 8)`,
///    which covers all 8 badge bits in a single slice.
///
/// # Returns
///
/// `None` if either snapshot is unpopulated, the pointer is out of range,
/// or the resolved address falls outside EWRAM.
pub fn read_badge_state() -> Option<BadgeState> {
    let iwram = fire_red_memory::get_iwram();
    let ewram = fire_red_memory::get_ewram();

    // Step 1: read the SaveBlock1 pointer from IWRAM.
    let ptr_offset = iwram_offset(SAVE_BLOCK_1_PTR);
    if iwram.len() < ptr_offset + 4 {
        return None;
    }
    let save_block_base = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;

    // Step 2: validate that the pointer points into EWRAM.
    if save_block_base < EWRAM_BASE || save_block_base >= EWRAM_BASE + ewram.len() {
        eprintln!(
            "SaveBlock1 pointer 0x{:08X} is outside EWRAM — snapshot may not be ready.",
            save_block_base
        );
        return None;
    }

    // Step 3: locate the two badge flag bytes within the EWRAM snapshot.
    // Badge flags 0x820–0x827 occupy bits 0–7 of the two bytes starting at
    // flags_array[0x820 / 8] = flags_array[0x104].
    let badge_byte_index = BADGE_FLAG_START / 8; // = 0x104
    let flags_offset_in_ewram = ewram_offset(save_block_base) + FLAGS_OFFSET + badge_byte_index;

    if ewram.len() < flags_offset_in_ewram + 2 {
        return None;
    }

    let b0 = ewram[flags_offset_in_ewram];
    let b1 = ewram[flags_offset_in_ewram + 1];
    let both = (b0 as u16) | ((b1 as u16) << 8);

    // Step 4: extract one bit per badge.
    // BADGE_FLAG_START (0x820) is already 8-aligned, so the bit position
    // within `both` for badge i is simply i (bits 0–7).
    let bit_start = BADGE_FLAG_START % 8; // = 0
    let mut badges = [false; NUM_BADGES];
    for i in 0..NUM_BADGES {
        badges[i] = (both >> (bit_start + i)) & 1 == 1;
    }

    // Step 5: find the first unearned badge to identify the next gym.
    let next_gym = badges
        .iter()
        .position(|&obtained| !obtained)
        .map(|i| gym_leaders()[i].clone());

    Some(BadgeState { badges, next_gym })
}

/// Returns the name of badge N (0-indexed), or `"Unknown"` if out of range.
pub fn badge_name(index: usize) -> &'static str {
    match index {
        0 => "Boulder Badge",
        1 => "Cascade Badge",
        2 => "Thunder Badge",
        3 => "Rainbow Badge",
        4 => "Soul Badge",
        5 => "Marsh Badge",
        6 => "Volcano Badge",
        7 => "Earth Badge",
        _ => "Unknown",
    }
}