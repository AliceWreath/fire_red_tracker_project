//! Global ROM buffer, revision detection, and address tables for FireRed/LeafGreen.
//!
//! Call [`fill_rom`] (or [`init_rom`]) once at startup.  Every subsequent call to
//! [`get_rom`] returns a `&'static [u8]` to the loaded bytes. [`get_rom_revision`]
//! and [`get_rom_addresses`] expose the detected revision and the corresponding
//! per-revision address table.

use std::sync::OnceLock;

/// Global immutable ROM buffer.
///
/// The ROM data is loaded once during initialization and then shared
/// as a static byte slice for the lifetime of the program.
///
/// Internally uses [`OnceLock`] to guarantee thread-safe, one-time
/// initialization.
static ROM_BUFFER: OnceLock<Vec<u8>> = OnceLock::new();

/// Detected ROM revision, stored alongside the ROM buffer.
static ROM_REVISION: OnceLock<RomRevision> = OnceLock::new();

/// Address table for the detected revision, stored alongside the ROM buffer.
static ROM_ADDRESSES: OnceLock<RomAddresses> = OnceLock::new();

// ---------------------------------------------------------------------------
// ROM header constants
// ---------------------------------------------------------------------------

/// Byte offset of the 4-byte ASCII game code in the GBA ROM header.
const GAME_CODE_OFFSET: usize = 0xAC;

/// Byte offset of the 1-byte software version in the GBA ROM header.
///
/// 0x00 = Rev 0 (v1.0), 0x01 = Rev 1 (v1.1).
const REVISION_OFFSET: usize = 0xBC;

// ---------------------------------------------------------------------------
// RomRevision
// ---------------------------------------------------------------------------

/// Identifies which ROM revision is loaded.
///
/// Used to select the correct address table for ROM data and EWRAM addresses.
/// Add a new variant (and a matching [`RomAddresses`] constant) when address
/// tables for an additional revision have been confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomRevision {
    /// Pokémon FireRed (USA) Rev 1 — the primary supported revision.
    FireRedUsaRev1,

    /// Pokémon FireRed (USA) Rev 0.
    ///
    /// Uses the same address table as Rev 1; both revisions share identical
    /// data-table and EWRAM-variable layouts. Only minor code-section patches
    /// differ between the two.
    FireRedUsaRev0,

    /// Pokémon LeafGreen (USA) Rev 1 (game code `BPGE`, revision byte `0x01`).
    ///
    /// EWRAM/IWRAM runtime addresses and save-block offsets are identical to
    /// FireRed.  ROM data-table addresses (Pokémon names, base stats, ability
    /// names, item data) have been confirmed by ROM scan against the retail
    /// LeafGreen USA Rev 1 ROM and differ from FireRed by small fixed offsets.
    LeafGreenUsaRev1,

    /// Pokémon LeafGreen (USA) Rev 0 (game code `BPGE`, revision byte `0x00`).
    ///
    /// Uses the same address table as Rev 1; Rev 0 and Rev 1 share identical
    /// data-table and EWRAM-variable layouts.
    LeafGreenUsaRev0,

    /// ROM header did not match any known game code, or the ROM was too small
    /// to read the header.  Addresses fall back to [`FireRedUsaRev1`].
    Unknown,
}

impl RomRevision {
    /// Returns `true` if this revision is a LeafGreen ROM.
    pub fn is_leaf_green(self) -> bool {
        matches!(
            self,
            RomRevision::LeafGreenUsaRev1 | RomRevision::LeafGreenUsaRev0
        )
    }

    /// Returns `true` if this revision is a FireRed ROM.
    pub fn is_fire_red(self) -> bool {
        matches!(
            self,
            RomRevision::FireRedUsaRev1 | RomRevision::FireRedUsaRev0
        )
    }
}

// ---------------------------------------------------------------------------
// RomAddresses
// ---------------------------------------------------------------------------

/// All revision-specific memory addresses for a FireRed ROM.
///
/// **ROM data table fields** (`pokemon_names_addr` … `base_stats_addr`) are
/// byte offsets into the ROM file (i.e. GBA bus address minus `0x08000000`).
///
/// **EWRAM fields** (`party_size_addr` … `map_group_and_name_addr`) and
/// **IWRAM fields** (`save_block_1_ptr`, `save_block_3_ptr`) are absolute GBA
/// bus addresses.
///
/// **SaveBlock internal offsets** (`flags_offset`, `badge_flag_start`,
/// `box_data_offset`) are byte offsets within the respective save-block
/// structure.  They are stable across all known FireRed revisions but are kept
/// here so ROM hacks that restructure save blocks can be supported without
/// touching call sites.
#[derive(Debug, Clone, Copy)]
pub struct RomAddresses {
    // ROM data table offsets (ROM file offset = bus address − 0x08000000)
    pub pokemon_names_addr: usize,
    pub ability_names_addr: usize,
    pub item_data_addr: usize,
    pub base_stats_addr: usize,
    pub move_data_addr: usize,

    // GBA bus addresses in EWRAM (0x02xxxxxx)
    pub party_size_addr: usize,
    pub party_addr: usize,
    pub player_data_addr: usize,
    pub map_group_and_name_addr: usize,

    // GBA bus addresses in IWRAM (0x03xxxxxx) — runtime pointers to SaveBlocks
    pub save_block_1_ptr: usize,
    pub save_block_3_ptr: usize,

    // Byte offsets within SaveBlock structures
    pub flags_offset: usize,
    pub badge_flag_start: usize,
    pub box_data_offset: usize,

    // Elite 4 and game-clear flag indices within gSaveBlock1.flags[]
    /// Flag index for Lorelei's first defeat (enables rematch).
    /// Bruno, Agatha, and Lance follow at e4_flag_start+1/+2/+3 respectively.
    /// Source: pokefirered FLAG_REMATCH_LORELEI = 0x3E3
    pub e4_flag_start: usize,
    /// Flag index set when the player first enters the Hall of Fame
    /// (i.e., defeats the Champion for the first time).
    /// Source: pokefirered FLAG_SYS_GAME_CLEAR = 0x083
    pub game_clear_flag: usize,
}

/// Address table confirmed for Pokémon FireRed USA Rev 1.
///
/// ROM data addresses were verified against the `pokefirered` decompilation.
/// EWRAM addresses were confirmed by live scan on Rev 1 (see project notes).
const FIRERED_USA_REV1: RomAddresses = RomAddresses {
    pokemon_names_addr: 0x245F5B,
    ability_names_addr: 0x24FCB0,
    item_data_addr: 0x3DB098,
    base_stats_addr: 0x2547F4,
    move_data_addr: 0x250C04,
    party_size_addr: 0x02024029,
    party_addr: 0x02024284,
    player_data_addr: 0x02024298,
    map_group_and_name_addr: 0x02031DBC,
    save_block_1_ptr: 0x03005008,
    save_block_3_ptr: 0x03005010,
    flags_offset: 0x0EE0,
    badge_flag_start: 0x820,
    box_data_offset: 0x4,
    e4_flag_start: 0x3E3,
    game_clear_flag: 0x083,
};

/// Address table for Pokémon FireRed USA Rev 0.
///
/// Rev 0 and Rev 1 differ only in code-section patches; all data tables and
/// EWRAM variable addresses are identical between the two revisions.
const FIRERED_USA_REV0: RomAddresses = FIRERED_USA_REV1;

/// Address table for Pokémon LeafGreen (USA).
///
/// ROM data-table offsets confirmed by live scan of the retail LeafGreen USA
/// Rev 1 ROM (BPGE, rev byte 0x01).  All four tables sit 0x24 bytes earlier
/// than FireRed for the name/stats tables; the item table has a larger shift.
/// EWRAM/IWRAM runtime addresses and save-block offsets are identical to FireRed.
const LEAFGREEN_USA_REV1: RomAddresses = RomAddresses {
    pokemon_names_addr: 0x245F37,
    ability_names_addr: 0x24FC8C,
    item_data_addr: 0x3DAED4,
    base_stats_addr: 0x2547D0,
    move_data_addr: 0x250BE0,
    party_size_addr: 0x02024029,
    party_addr: 0x02024284,
    player_data_addr: 0x02024298,
    map_group_and_name_addr: 0x02031DBC,
    save_block_1_ptr: 0x03005008,
    save_block_3_ptr: 0x03005010,
    flags_offset: 0x0EE0,
    badge_flag_start: 0x820,
    box_data_offset: 0x4,
    e4_flag_start: 0x3E3,
    game_clear_flag: 0x083,
};

const LEAFGREEN_USA_REV0: RomAddresses = LEAFGREEN_USA_REV1;

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detects the ROM revision from the GBA ROM header.
///
/// Reads the 4-byte game code at offset `0xAC` and the 1-byte software
/// version at offset `0xBC`.  Prints a diagnostic message for unknown ROMs
/// so the user can tell whether their ROM is supported.
///
/// | Game code | Rev byte | Detected revision            |
/// |-----------|----------|------------------------------|
/// | `BPRE`    | `0x01`   | [`FireRedUsaRev1`]           |
/// | `BPRE`    | `0x00`   | [`FireRedUsaRev0`]           |
/// | `BPGE`    | `0x01`   | [`LeafGreenUsaRev1`]         |
/// | `BPGE`    | `0x00`   | [`LeafGreenUsaRev0`]         |
/// | anything else        | [`Unknown`] (falls back to FireRed Rev 1 addresses) |
///
/// [`FireRedUsaRev1`]: RomRevision::FireRedUsaRev1
/// [`FireRedUsaRev0`]: RomRevision::FireRedUsaRev0
/// [`LeafGreenUsaRev1`]: RomRevision::LeafGreenUsaRev1
/// [`LeafGreenUsaRev0`]: RomRevision::LeafGreenUsaRev0
/// [`Unknown`]: RomRevision::Unknown
pub fn detect_rom_revision(rom: &[u8]) -> RomRevision {
    let Some(game_code) = rom.get(GAME_CODE_OFFSET..GAME_CODE_OFFSET + 4) else {
        tracing::warn!(
            "ROM auto-detect: file too small to read header — defaulting to FireRed USA Rev 1 addresses."
        );
        return RomRevision::Unknown;
    };
    let revision = rom.get(REVISION_OFFSET).copied().unwrap_or(0xFF);

    match game_code {
        b"BPRE" => match revision {
            1 => RomRevision::FireRedUsaRev1,
            0 => RomRevision::FireRedUsaRev0,
            r => {
                tracing::warn!(
                    "ROM auto-detect: game code BPRE rev {:#04X} is not a known FireRed revision \
                     — defaulting to Rev 1 addresses.",
                    r
                );
                RomRevision::Unknown
            }
        },
        b"BPGE" => match revision {
            1 => {
                tracing::info!("ROM auto-detect: LeafGreen USA Rev 1 detected.");
                RomRevision::LeafGreenUsaRev1
            }
            0 => {
                tracing::info!("ROM auto-detect: LeafGreen USA Rev 0 detected.");
                RomRevision::LeafGreenUsaRev0
            }
            r => {
                tracing::warn!(
                    "ROM auto-detect: game code BPGE rev {:#04X} is not a known LeafGreen revision \
                     — defaulting to LeafGreen Rev 1 addresses.",
                    r
                );
                RomRevision::LeafGreenUsaRev1
            }
        },
        _ => {
            let code_str = std::str::from_utf8(game_code).unwrap_or("????");
            tracing::warn!(
                "ROM auto-detect: unrecognized game code {:?} rev {:#04X} \
                 — defaulting to FireRed USA Rev 1 addresses; some lookups may be wrong.",
                code_str,
                revision
            );
            RomRevision::Unknown
        }
    }
}

/// Returns the [`RomAddresses`] table for the given revision.
fn addresses_for(rev: RomRevision) -> RomAddresses {
    match rev {
        RomRevision::FireRedUsaRev1 => FIRERED_USA_REV1,
        RomRevision::FireRedUsaRev0 => FIRERED_USA_REV0,
        RomRevision::LeafGreenUsaRev1 => LEAFGREEN_USA_REV1,
        RomRevision::LeafGreenUsaRev0 => LEAFGREEN_USA_REV0,
        RomRevision::Unknown => FIRERED_USA_REV1,
    }
}

// ---------------------------------------------------------------------------
// Public accessors
// ---------------------------------------------------------------------------

/// Returns the detected [`RomRevision`].
///
/// Returns [`RomRevision::Unknown`] if the ROM buffer has not yet been
/// initialized via [`fill_rom`] / [`init_rom`].
pub fn get_rom_revision() -> RomRevision {
    ROM_REVISION.get().copied().unwrap_or(RomRevision::Unknown)
}

/// Returns the address table for the detected ROM revision.
///
/// Falls back to the FireRed USA Rev 1 table if the ROM has not yet been
/// initialized.  This fallback is safe for the common single-ROM use case
/// because `fill_rom` always runs before any subsystem reads an address.
pub fn get_rom_addresses() -> &'static RomAddresses {
    ROM_ADDRESSES.get().unwrap_or(&FIRERED_USA_REV1)
}

// ---------------------------------------------------------------------------
// ROM buffer management
// ---------------------------------------------------------------------------

/// Loads a ROM file from disk and initializes the global ROM buffer.
///
/// # Arguments
///
/// * `path_to_file` - Path to the ROM file.
///
/// # Errors
///
/// Returns an error if:
///
/// - The provided path is empty
/// - The file cannot be opened or read.
/// - The ROM buffer could not be initialized.
///
/// # Notes
///
/// The ROM buffer is only initialized once. Subsequent calls
/// will not replace the existing buffer.
pub fn fill_rom(path_to_file: &str) -> Result<(), String> {
    if path_to_file.is_empty() {
        return Err(String::from("Must pass a valid file path."));
    }

    let rom = std::fs::read(path_to_file).map_err(|_| {
        format!(
            "Unable to open file {}, check the path.\nROM static not initialized!",
            path_to_file
        )
    })?;
    fill_static_buffer(rom);

    Ok(())
}

/// Alias for [`fill_rom`] to provide a more intuitive API for users.
///
/// Initializes the global ROM buffer from a ROM file path.
///
/// # Arguments
///
/// * `path_to_file` - Path to the ROM file.
///
/// # Errors
///
/// Returns the same errors as [`fill_rom`]
pub fn init_rom(path_to_file: &str) -> Result<(), String> {
    fill_rom(path_to_file)
}

/// Initializes the global ROM buffer if it has not already been set,
/// then detects and stores the ROM revision and address table.
///
/// # Arguments
///
/// * `buffer` - ROM byte buffer to store.
///
/// # Returns
///
/// A static reference to the stored ROM data.
///
/// # Notes
///
/// If the buffer has already been initialized, the existing
/// buffer is preserved and returned instead of the new one.
fn fill_static_buffer(buffer: Vec<u8>) -> &'static [u8] {
    let rom = ROM_BUFFER.get_or_init(|| buffer);
    // Detect revision from the stored ROM and cache both the revision enum
    // and the corresponding address table.  These OnceLocks are a no-op on
    // subsequent calls, so there is no race between concurrent fill_rom calls.
    let rev = ROM_REVISION.get_or_init(|| detect_rom_revision(rom));
    ROM_ADDRESSES.get_or_init(|| addresses_for(*rev));
    rom
}

/// Returns a shared reference to the global ROM buffer.
///
/// # Panics
///
/// Panics if the ROM buffer has not yet been initialized.
///
/// # Examples
///
/// ```ignore
/// init_rom("firered.gba").unwrap();
///
/// let rom = get_rom();
/// println!("ROM size: {} bytes", rom.len());
/// ```
pub fn get_rom() -> &'static [u8] {
    ROM_BUFFER.get().expect("Vector not initialized")
}

/// Returns the ROM buffer if it has been initialized, or `None` otherwise.
pub fn try_get_rom() -> Option<&'static [u8]> {
    ROM_BUFFER.get().map(Vec::as_slice)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_header(game_code: &[u8; 4], revision: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x200];
        rom[GAME_CODE_OFFSET..GAME_CODE_OFFSET + 4].copy_from_slice(game_code);
        rom[REVISION_OFFSET] = revision;
        rom
    }

    #[test]
    fn detect_firered_rev1() {
        let rom = make_header(b"BPRE", 1);
        assert_eq!(detect_rom_revision(&rom), RomRevision::FireRedUsaRev1);
    }

    #[test]
    fn detect_firered_rev0() {
        let rom = make_header(b"BPRE", 0);
        assert_eq!(detect_rom_revision(&rom), RomRevision::FireRedUsaRev0);
    }

    #[test]
    fn detect_leafgreen_rev1() {
        let rom = make_header(b"BPGE", 1);
        assert_eq!(detect_rom_revision(&rom), RomRevision::LeafGreenUsaRev1);
    }

    #[test]
    fn detect_leafgreen_rev0() {
        let rom = make_header(b"BPGE", 0);
        assert_eq!(detect_rom_revision(&rom), RomRevision::LeafGreenUsaRev0);
    }

    #[test]
    fn detect_unknown_game_code() {
        let rom = make_header(b"AXVE", 0); // Ruby
        assert_eq!(detect_rom_revision(&rom), RomRevision::Unknown);
    }

    #[test]
    fn detect_too_small_rom_returns_unknown() {
        assert_eq!(detect_rom_revision(&[0u8; 4]), RomRevision::Unknown);
    }

    #[test]
    fn is_fire_red_true_for_firered_variants() {
        assert!(RomRevision::FireRedUsaRev1.is_fire_red());
        assert!(RomRevision::FireRedUsaRev0.is_fire_red());
        assert!(!RomRevision::LeafGreenUsaRev1.is_fire_red());
        assert!(!RomRevision::LeafGreenUsaRev0.is_fire_red());
        assert!(!RomRevision::Unknown.is_fire_red());
    }

    #[test]
    fn is_leaf_green_true_for_leafgreen_variants() {
        assert!(RomRevision::LeafGreenUsaRev1.is_leaf_green());
        assert!(RomRevision::LeafGreenUsaRev0.is_leaf_green());
        assert!(!RomRevision::FireRedUsaRev1.is_leaf_green());
        assert!(!RomRevision::FireRedUsaRev0.is_leaf_green());
        assert!(!RomRevision::Unknown.is_leaf_green());
    }

    #[test]
    fn leafgreen_rev1_ewram_addresses_match_firered() {
        // Runtime addresses must be identical between FireRed and LeafGreen.
        assert_eq!(
            LEAFGREEN_USA_REV1.party_size_addr,
            FIRERED_USA_REV1.party_size_addr
        );
        assert_eq!(LEAFGREEN_USA_REV1.party_addr, FIRERED_USA_REV1.party_addr);
        assert_eq!(
            LEAFGREEN_USA_REV1.player_data_addr,
            FIRERED_USA_REV1.player_data_addr
        );
        assert_eq!(
            LEAFGREEN_USA_REV1.map_group_and_name_addr,
            FIRERED_USA_REV1.map_group_and_name_addr
        );
        assert_eq!(
            LEAFGREEN_USA_REV1.save_block_1_ptr,
            FIRERED_USA_REV1.save_block_1_ptr
        );
        assert_eq!(
            LEAFGREEN_USA_REV1.save_block_3_ptr,
            FIRERED_USA_REV1.save_block_3_ptr
        );
        assert_eq!(
            LEAFGREEN_USA_REV1.flags_offset,
            FIRERED_USA_REV1.flags_offset
        );
        assert_eq!(
            LEAFGREEN_USA_REV1.badge_flag_start,
            FIRERED_USA_REV1.badge_flag_start
        );
        assert_eq!(
            LEAFGREEN_USA_REV1.box_data_offset,
            FIRERED_USA_REV1.box_data_offset
        );
    }

    #[test]
    fn leafgreen_rev1_rom_table_addresses_confirmed() {
        // ROM data-table addresses confirmed by live scan of LeafGreen USA Rev 1 ROM.
        // These must not equal the FireRed values (which were the old placeholders).
        assert_eq!(LEAFGREEN_USA_REV1.pokemon_names_addr, 0x245F37);
        assert_eq!(LEAFGREEN_USA_REV1.ability_names_addr, 0x24FC8C);
        assert_eq!(LEAFGREEN_USA_REV1.base_stats_addr, 0x2547D0);
        assert_eq!(LEAFGREEN_USA_REV1.item_data_addr, 0x3DAED4);
        assert_ne!(
            LEAFGREEN_USA_REV1.pokemon_names_addr,
            FIRERED_USA_REV1.pokemon_names_addr
        );
        assert_ne!(
            LEAFGREEN_USA_REV1.ability_names_addr,
            FIRERED_USA_REV1.ability_names_addr
        );
        assert_ne!(
            LEAFGREEN_USA_REV1.base_stats_addr,
            FIRERED_USA_REV1.base_stats_addr
        );
        assert_ne!(
            LEAFGREEN_USA_REV1.item_data_addr,
            FIRERED_USA_REV1.item_data_addr
        );
    }

    #[test]
    fn firered_rev0_rev1_unknown_revision_returns_unknown() {
        let rom = make_header(b"BPRE", 0xFF);
        assert_eq!(detect_rom_revision(&rom), RomRevision::Unknown);
    }
}
