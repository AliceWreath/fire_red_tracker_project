//! # Fire Red Badge Monitor
//!
//! Reads the player's badge flags from SaveBlock1 and exposes which badges
//! have been obtained, as well as the highest-level pokemon on the next
//! gym leader's team.
//!
//! ## Memory layout
//!
//! Badges are stored as individual bits in the flags array inside SaveBlock1.
//! The SaveBlock1 base address is read at runtime by dereferencing the pointer
//! at `0x03005008`. The flags array starts at offset `0x0EE0` within that block.
//!
//! Badge flags occupy flag indices `0x820` through `0x827`:
//! - Flag `0x820` = Boulder Badge (Brock)
//! - Flag `0x821` = Cascade Badge (Misty)
//! - Flag `0x822` = Thunder Badge (Lt. Surge)
//! - Flag `0x823` = Rainbow Badge (Erika)
//! - Flag `0x824` = Soul Badge (Koga)
//! - Flag `0x825` = Marsh Badge (Sabrina)
//! - Flag `0x826` = Volcano Badge (Blaine)
//! - Flag `0x827` = Earth Badge (Giovanni)
//!
//! Each flag index maps to: byte = index / 8, bit = index % 8 within the
//! flags array.

use fire_red_retroarch_interfacing::{generate_command, get_from_retroarch};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// IWRAM pointer to SaveBlock1 base address (dynamic, followed at runtime)
const SAVE_BLOCK_1_PTR: u32 = 0x03005008;

/// Offset of the flags array within SaveBlock1
const FLAGS_OFFSET: u32 = 0x0EE0;

/// Flag index of the first badge (Boulder Badge)
const BADGE_FLAG_START: u16 = 0x820;

/// Number of badges in the game
const NUM_BADGES: usize = 8;

// ---------------------------------------------------------------------------
// Badge data
// ---------------------------------------------------------------------------

/// Represents the full badge state for a player.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BadgeState {
    /// One entry per badge, in gym order. `true` = obtained.
    pub badges: [bool; 8],
    /// The next gym leader the player hasn't beaten yet, or `None` if all
    /// badges are obtained.
    pub next_gym: Option<GymInfo>,
}

/// Information about a gym leader.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GymInfo {
    /// Gym leader name
    pub leader: String,
    /// City name
    pub city: String,
    /// Badge awarded on victory
    pub badge: String,
    /// Highest level pokemon on the leader's team
    pub max_level: u8,
}

impl BadgeState {
    /// Returns how many badges the player currently has.
    pub fn count(&self) -> usize {
        self.badges.iter().filter(|&&b| b).count()
    }

    /// Returns `true` if all 8 badges have been obtained.
    pub fn all_obtained(&self) -> bool {
        self.badges.iter().all(|&b| b)
    }
}

// ---------------------------------------------------------------------------
// Gym leader table
// ---------------------------------------------------------------------------

/// Static table of all 8 Kanto gym leaders in order.
/// `max_level` is the highest level pokemon on their team in FireRed.
fn gym_leaders() -> [GymInfo; 8] {
    [
        GymInfo {
            leader: String::from("Brock"),
            city: String::from("Pewter City"),
            badge: String::from("Boulder Badge"),
            max_level: 14, // Onix Lv14
        },
        GymInfo {
            leader: String::from("Misty"),
            city: String::from("Cerulean City"),
            badge: String::from("Cascade Badge"),
            max_level: 21, // Starmie Lv21
        },
        GymInfo {
            leader: String::from("Lt. Surge"),
            city: String::from("Vermilion City"),
            badge: String::from("Thunder Badge"),
            max_level: 24, // Raichu Lv24
        },
        GymInfo {
            leader: String::from("Erika"),
            city: String::from("Celadon City"),
            badge: String::from("Rainbow Badge"),
            max_level: 29, // Vileplume Lv29
        },
        GymInfo {
            leader: String::from("Koga"),
            city: String::from("Fuchsia City"),
            badge: String::from("Soul Badge"),
            max_level: 43, // Weezing Lv43
        },
        GymInfo {
            leader: String::from("Sabrina"),
            city: String::from("Saffron City"),
            badge: String::from("Marsh Badge"),
            max_level: 50, // Alakazam Lv50
        },
        GymInfo {
            leader: String::from("Blaine"),
            city: String::from("Cinnabar Island"),
            badge: String::from("Volcano Badge"),
            max_level: 54, // Arcanine Lv54
        },
        GymInfo {
            leader: String::from("Giovanni"),
            city: String::from("Viridian City"),
            badge: String::from("Earth Badge"),
            max_level: 55, // Rhydon Lv55
        },
    ]
}

// ---------------------------------------------------------------------------
// RAM reading
// ---------------------------------------------------------------------------

/// Reads the SaveBlock1 base address by dereferencing the pointer at
/// `SAVE_BLOCK_1_PTR`. Returns `None` if the read fails.
fn get_save_block_1_base() -> Option<u32> {
    let command = generate_command(SAVE_BLOCK_1_PTR, 4);
    let res = get_from_retroarch(command.as_str(), 6)?;
    let bytes: Vec<u8> = res
        .iter()
        .skip(2)
        .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
        .collect();
    if bytes.len() >= 4 {
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    } else {
        None
    }
}

/// Reads a single flag from the flags array in SaveBlock1.
///
/// Each flag index maps to: byte offset = `FLAGS_OFFSET + flag_index / 8`,
/// bit = `flag_index % 8`.
///
/// Returns `false` on any read failure.
fn read_flag(save_block_base: u32, flag_index: u16) -> bool {
    let byte_offset = FLAGS_OFFSET + (flag_index / 8) as u32;
    let bit = flag_index % 8;
    let addr = save_block_base + byte_offset;

    let command = generate_command(addr, 1);
    let res = match get_from_retroarch(command.as_str(), 3) {
        Some(r) => r,
        None => return false,
    };

    let byte = match res
        .get(2)
        .and_then(|s| u8::from_str_radix(s.trim(), 16).ok())
    {
        Some(b) => b,
        None => return false,
    };

    (byte >> bit) & 1 == 1
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Reads the current badge state from game memory.
///
/// Returns `None` if the SaveBlock1 address cannot be resolved.
pub fn read_badge_state() -> Option<BadgeState> {
    let base = get_save_block_1_base()?;

    // Read 2 bytes covering all 8 badge flags (flags 0x820-0x827)
    let badge_byte_offset = FLAGS_OFFSET + (BADGE_FLAG_START / 8) as u32;
    let addr = base + badge_byte_offset;
    let command = generate_command(addr, 2);
    let res = get_from_retroarch(command.as_str(), 4)?;

    let b0 = u8::from_str_radix(res.get(2)?.trim(), 16).ok()?;
    let b1 = u8::from_str_radix(res.get(3)?.trim(), 16).ok()?;
    let both = (b0 as u16) | ((b1 as u16) << 8);

    let mut badges = [false; NUM_BADGES];
    for i in 0..NUM_BADGES {
        let bit_pos = (BADGE_FLAG_START % 8) as usize + i;
        badges[i] = (both >> bit_pos) & 1 == 1;
    }

    let next_gym = badges
        .iter()
        .enumerate()
        .find(|&(_, obtained)| !obtained)
        .map(|(i, _)| gym_leaders()[i].clone());

    Some(BadgeState {badges, next_gym})
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
