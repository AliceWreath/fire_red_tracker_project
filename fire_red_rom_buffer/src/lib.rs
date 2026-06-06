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

    /// ROM header did not match any known game code, or the ROM was too small
    /// to read the header.  Addresses fall back to [`FireRedUsaRev1`].
    Unknown,
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
    pub item_data_addr:     usize,
    pub base_stats_addr:    usize,

    // GBA bus addresses in EWRAM (0x02xxxxxx)
    pub party_size_addr:         usize,
    pub party_addr:              usize,
    pub player_data_addr:        usize,
    pub map_group_and_name_addr: usize,

    // GBA bus addresses in IWRAM (0x03xxxxxx) — runtime pointers to SaveBlocks
    pub save_block_1_ptr: usize,
    pub save_block_3_ptr: usize,

    // Byte offsets within SaveBlock structures
    pub flags_offset:     usize,
    pub badge_flag_start: usize,
    pub box_data_offset:  usize,
}

/// Address table confirmed for Pokémon FireRed USA Rev 1.
///
/// ROM data addresses were verified against the `pokefirered` decompilation.
/// EWRAM addresses were confirmed by live scan on Rev 1 (see project notes).
const FIRERED_USA_REV1: RomAddresses = RomAddresses {
    pokemon_names_addr:      0x245F5B,
    ability_names_addr:      0x24FCB0,
    item_data_addr:          0x3DB098,
    base_stats_addr:         0x2547F4,
    party_size_addr:         0x02024029,
    party_addr:              0x02024284,
    player_data_addr:        0x02024298,
    map_group_and_name_addr: 0x02031DBC,
    save_block_1_ptr:        0x03005008,
    save_block_3_ptr:        0x03005010,
    flags_offset:            0x0EE0,
    badge_flag_start:        0x820,
    box_data_offset:         0x4,
};

/// Address table for Pokémon FireRed USA Rev 0.
///
/// Rev 0 and Rev 1 differ only in code-section patches; all data tables and
/// EWRAM variable addresses are identical between the two revisions.
const FIRERED_USA_REV0: RomAddresses = FIRERED_USA_REV1;

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detects the ROM revision from the GBA ROM header.
///
/// Reads the 4-byte game code at offset `0xAC` and the 1-byte software
/// version at offset `0xBC`.  Prints a diagnostic message for unknown ROMs
/// so the user can tell whether their ROM is supported.
///
/// | Game code | Rev byte | Detected revision        |
/// |-----------|----------|--------------------------|
/// | `BPRE`    | `0x01`   | [`FireRedUsaRev1`]       |
/// | `BPRE`    | `0x00`   | [`FireRedUsaRev0`]       |
/// | anything else        | [`Unknown`] (falls back to Rev 1 addresses) |
///
/// [`FireRedUsaRev1`]: RomRevision::FireRedUsaRev1
/// [`FireRedUsaRev0`]: RomRevision::FireRedUsaRev0
/// [`Unknown`]: RomRevision::Unknown
pub fn detect_rom_revision(rom: &[u8]) -> RomRevision {
    let Some(game_code) = rom.get(GAME_CODE_OFFSET..GAME_CODE_OFFSET + 4) else {
        eprintln!("ROM auto-detect: file too small to read header — defaulting to FireRed USA Rev 1 addresses.");
        return RomRevision::Unknown;
    };
    let revision = rom.get(REVISION_OFFSET).copied().unwrap_or(0xFF);

    if game_code == b"BPRE" {
        match revision {
            1 => RomRevision::FireRedUsaRev1,
            0 => RomRevision::FireRedUsaRev0,
            r => {
                eprintln!(
                    "ROM auto-detect: game code BPRE rev {:#04X} is not a known FireRed revision \
                     — defaulting to Rev 1 addresses.",
                    r
                );
                RomRevision::Unknown
            }
        }
    } else {
        let code_str = std::str::from_utf8(game_code).unwrap_or("????");
        eprintln!(
            "ROM auto-detect: unrecognized game code {:?} rev {:#04X} \
             — defaulting to FireRed USA Rev 1 addresses; some lookups may be wrong.",
            code_str, revision
        );
        RomRevision::Unknown
    }
}

/// Returns the [`RomAddresses`] table for the given revision.
fn addresses_for(rev: RomRevision) -> RomAddresses {
    match rev {
        RomRevision::FireRedUsaRev1 => FIRERED_USA_REV1,
        RomRevision::FireRedUsaRev0 => FIRERED_USA_REV0,
        RomRevision::Unknown        => FIRERED_USA_REV1,
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

    let rom = std::fs::read(path_to_file);
    if rom.is_err() {
        return Err(format!("Unable to open file {}, check the path.\nROM static not initialized!", path_to_file));
    }

    let rom = rom.unwrap();
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
    ROM_BUFFER.get().expect("Vector not intialized")
}

/// Returns the ROM buffer if it has been initialized, or `None` otherwise.
pub fn try_get_rom() -> Option<&'static [u8]> {
    ROM_BUFFER.get().map(Vec::as_slice)
}
