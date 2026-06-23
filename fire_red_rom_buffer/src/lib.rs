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

/// Optional user-supplied ROM offset for the gTrainers table.
///
/// When set via [`set_trainer_table_addr_override`] before the ROM is loaded,
/// all auto-detection is skipped and this value is used directly.
static TRAINER_TABLE_OVERRIDE: OnceLock<usize> = OnceLock::new();

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
    /// Fallback EWRAM address for SaveBlock2 when `save_block_2_ptr` cannot be
    /// resolved from IWRAM (e.g. before the game finishes booting).
    pub save_block_2_base: usize,
    pub map_group_and_name_addr: usize,

    // GBA bus addresses in IWRAM (0x03xxxxxx) — runtime pointers to SaveBlocks
    pub save_block_1_ptr: usize,
    /// IWRAM address of the `gSaveBlock2Ptr` pointer (4-byte LE GBA address).
    /// Dereference this at runtime to find the live SaveBlock2 in EWRAM.
    pub save_block_2_ptr: usize,
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

    /// ROM file offset of the `gTrainers` table.
    ///
    /// Each entry is 40 bytes (see [`trainer_entry_size`](crate)).
    /// Used by the vs-leader overlay to read gym leader party data from the ROM
    /// at runtime, making it randomizer-aware.
    /// `0` means the table address is unknown for this revision and the overlay
    /// should display no data.
    pub trainer_data_addr: usize,
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
    save_block_2_base: 0x020245DC,
    map_group_and_name_addr: 0x02031DBC,
    save_block_1_ptr: 0x03005008,
    save_block_2_ptr: 0x0300500C,
    save_block_3_ptr: 0x03005010,
    flags_offset: 0x0EE0,
    badge_flag_start: 0x820,
    box_data_offset: 0x4,
    e4_flag_start: 0x3E3,
    game_clear_flag: 0x083,
    trainer_data_addr: 0x23CAE0,
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
    save_block_2_base: 0x020245DC,
    map_group_and_name_addr: 0x02031DBC,
    save_block_1_ptr: 0x03005008,
    save_block_2_ptr: 0x0300500C,
    save_block_3_ptr: 0x03005010,
    flags_offset: 0x0EE0,
    badge_flag_start: 0x820,
    box_data_offset: 0x4,
    e4_flag_start: 0x3E3,
    game_clear_flag: 0x083,
    // LeafGreen trainer table address not yet confirmed; 0 disables vs_leader.
    trainer_data_addr: 0,
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
// gTrainers heuristic scanner
// ---------------------------------------------------------------------------

/// Size of one `gTrainers` entry in bytes (matches the C `struct Trainer`).
const TRAINER_ENTRY_SIZE: usize = 40;

/// Minimum run of consecutive valid entries to confirm a trainer table location.
const MIN_TRAINER_RUN: usize = 50;

/// Minimum number of entries (out of [`MIN_TRAINER_RUN`]) that must have a
/// non-empty party with a valid ROM pointer to confirm a gTrainers location.
/// Filters out data regions where all entries happen to have partySize == 0.
const MIN_TRAINER_WITH_PARTY: usize = 15;

/// Returns `true` if the entry at `offset` has the functional signature of
/// `TRAINER_NONE`: `partyFlags == 0` and `partySize == 0`.
///
/// The `partyPtr` field is intentionally **not** required to be zero.  In some
/// ROM builds (including certain retail FireRed dumps) the linker assigns a
/// non-null address to the empty `sParty_TrainerNone[]` array.  Requiring
/// `partyPtr == 0` would silently skip the real sentinel and only match false
/// positives inside zero-padded ROM regions.  When `partySize == 0` the game
/// engine never dereferences `partyPtr`, so its value is irrelevant.
fn is_trainer_none_entry(rom: &[u8], offset: usize) -> bool {
    offset + TRAINER_ENTRY_SIZE <= rom.len()
        && rom[offset] == 0         // partyFlags
        && rom[offset + 0x20] == 0  // partySize
}

/// Returns `true` if the 40 bytes at `offset` resemble a valid `gTrainers` entry.
///
/// Checks:
/// - `partyFlags` byte (offset 0): bits 2-7 must be zero (value 0-3).
/// - `partySize` byte (offset 0x20): 0-6.
/// - If `partySize > 0`: `partyPtr` (offsets 0x24-0x27) must be a valid GBA ROM
///   bus address (0x08000000 – 0x09FFFFFF).
///
/// The `doubleBattle` bool32 (offset 0x18) and the three padding bytes
/// (offsets 0x21-0x23) are intentionally NOT checked.  Both are compiler-
/// generated fields in the original GBA binary that may contain uninitialized
/// values, even in the vanilla retail ROM.
fn looks_like_trainer_entry(rom: &[u8], offset: usize) -> bool {
    if offset + TRAINER_ENTRY_SIZE > rom.len() {
        return false;
    }
    if rom[offset] > 3 {
        return false;
    }
    let party_size = rom[offset + 0x20] as usize;
    if party_size > 6 {
        return false;
    }
    if party_size > 0 {
        let ptr = u32::from_le_bytes([
            rom[offset + 0x24],
            rom[offset + 0x25],
            rom[offset + 0x26],
            rom[offset + 0x27],
        ]);
        if !(0x0800_0000..=0x09FF_FFFF).contains(&ptr) {
            return false;
        }
    }
    true
}

/// Returns the number of entries starting at `start` (up to `count` checked)
/// where `partySize` > 0, `partyPtr` is in ROM range, AND the first Pokémon's
/// species at the party pointer is in the valid National Dex range (1–386).
///
/// In all four `TrainerMon` struct layouts (`iv`, `lvl`, `species` are always
/// the first three fields), so species is always at bytes 2-3 of the first mon.
/// Verifying that the pointer leads to plausible species data makes this check
/// highly discriminating against false-positive table candidates.
fn count_entries_with_valid_party(rom: &[u8], start: usize, count: usize) -> usize {
    (0..count)
        .filter(|&i| {
            let off = start + i * TRAINER_ENTRY_SIZE;
            if off + TRAINER_ENTRY_SIZE > rom.len() {
                return false;
            }
            let ps = rom[off + 0x20] as usize;
            if ps == 0 || ps > 6 {
                return false;
            }
            let ptr = u32::from_le_bytes([
                rom[off + 0x24],
                rom[off + 0x25],
                rom[off + 0x26],
                rom[off + 0x27],
            ]);
            if !(0x0800_0000..=0x09FF_FFFF).contains(&ptr) {
                return false;
            }
            let party_off = (ptr - 0x0800_0000) as usize;
            if party_off + 4 > rom.len() {
                return false;
            }
            // Species is at bytes 2-3 of the first mon in every TrainerMon layout.
            let species = u16::from_le_bytes([rom[party_off + 2], rom[party_off + 3]]);
            species > 0 && species <= 386
        })
        .count()
}

/// Known trainer indices for the eight Kanto gym leaders, Elite Four, and
/// Champion in vanilla FireRed.  At least [`MIN_VALID_LEADER_SPOTS`] of these
/// must have a non-empty party whose first Pokémon has level in [1,100] and
/// species in [1,386].  This rules out false-positive table offsets where one
/// or more of these slots contain level-0 or otherwise impossible data.
const LEADER_SPOT_INDICES: &[usize] = &[54, 55, 56, 57, 58, 59, 60, 61, 118, 119, 120, 121, 148];

/// Minimum number of [`LEADER_SPOT_INDICES`] that must pass the level+species
/// check inside [`validate_trainer_table_at`].
const MIN_VALID_LEADER_SPOTS: usize = 3;

/// Returns the number of gym-leader/E4 spot indices whose first party Pokémon
/// has level in [1,100] and species in [1,386].
fn count_valid_leader_spots(rom: &[u8], start: usize) -> usize {
    LEADER_SPOT_INDICES
        .iter()
        .filter(|&&idx| {
            let off = start + idx * TRAINER_ENTRY_SIZE;
            if off + TRAINER_ENTRY_SIZE > rom.len() {
                return false;
            }
            let ps = rom[off + 0x20] as usize;
            if ps == 0 || ps > 6 {
                return false;
            }
            let ptr = u32::from_le_bytes([
                rom[off + 0x24],
                rom[off + 0x25],
                rom[off + 0x26],
                rom[off + 0x27],
            ]);
            if !(0x0800_0000..=0x09FF_FFFF).contains(&ptr) {
                return false;
            }
            let po = (ptr - 0x0800_0000) as usize;
            if po + 4 > rom.len() {
                return false;
            }
            let lvl = rom[po + 1];
            let spc = u16::from_le_bytes([rom[po + 2], rom[po + 3]]);
            lvl >= 1 && lvl <= 100 && spc >= 1 && spc <= 386
        })
        .count()
}

/// Returns `true` if `start` looks like the beginning of a `gTrainers` table.
///
/// Requires:
/// 1. Entry 0 has the functional `TRAINER_NONE` signature (partyFlags, partySize,
///    and partyPtr are all zero).
/// 2. At least [`MIN_TRAINER_RUN`] consecutive entries pass structural checks.
/// 3. At least [`MIN_TRAINER_WITH_PARTY`] of those entries have a non-empty party
///    whose first Pokémon's species is in the valid National Dex range (1–386).
/// 4. At least [`MIN_VALID_LEADER_SPOTS`] of the known gym-leader/E4/Champion
///    indices have a first Pokémon with level in [1,100] and species in [1,386].
///    This directly rejects false-positive offsets where those slots hold
///    level-0 or otherwise impossible trainer data.
fn validate_trainer_table_at(rom: &[u8], start: usize) -> bool {
    if !is_trainer_none_entry(rom, start) {
        return false;
    }
    if !(0..MIN_TRAINER_RUN)
        .all(|i| looks_like_trainer_entry(rom, start + i * TRAINER_ENTRY_SIZE))
    {
        return false;
    }
    if count_entries_with_valid_party(rom, start, MIN_TRAINER_RUN) < MIN_TRAINER_WITH_PARTY {
        return false;
    }
    count_valid_leader_spots(rom, start) >= MIN_VALID_LEADER_SPOTS
}

/// Scans the full ROM for the `gTrainers` table.
///
/// Advances in 4-byte aligned steps.  For each candidate position:
/// 1. Fast pre-check: entry 0 must have the `TRAINER_NONE` functional signature.
/// 2. Full [`validate_trainer_table_at`] check (run length + species-verified
///    party count + gym-leader spot-check).
///
/// Searches the entire ROM rather than just the first 8 MiB because some ROM
/// hacks expand the ROM and relocate `gTrainers` into the upper half.
///
/// Returns the ROM byte offset of the first entry in the table, or `None` if
/// the table cannot be located.
fn find_trainer_table(rom: &[u8]) -> Option<usize> {
    tracing::info!(
        "gTrainers scan: searching {} MiB of ROM",
        rom.len() / (1024 * 1024)
    );
    let mut candidates = 0u32;
    let mut i = 0usize;
    while i + TRAINER_ENTRY_SIZE <= rom.len() {
        if !is_trainer_none_entry(rom, i) {
            i += 4;
            continue;
        }
        candidates += 1;
        if validate_trainer_table_at(rom, i) {
            return Some(i);
        }
        // Sentinel matched but table validation failed — log the first few so
        // we can tell whether the scanner is seeing candidates at all.
        if candidates <= 8 {
            let run_ok = (0..MIN_TRAINER_RUN)
                .all(|j| looks_like_trainer_entry(rom, i + j * TRAINER_ENTRY_SIZE));
            let party_count = if run_ok {
                count_entries_with_valid_party(rom, i, MIN_TRAINER_RUN)
            } else { 0 };
            let leader_spots = if run_ok { count_valid_leader_spots(rom, i) } else { 0 };
            tracing::info!(
                "gTrainers scan: sentinel at {:#X} rejected \
                 (run_ok={} party_count={}/{} leader_spots={}/{})",
                i, run_ok,
                party_count, MIN_TRAINER_WITH_PARTY,
                leader_spots, MIN_VALID_LEADER_SPOTS,
            );
        }
        i += 4;
    }
    tracing::info!("gTrainers scan: {} sentinel candidates found, none passed validation", candidates);
    None
}

/// Scans the full ROM for the `gTrainers` table WITHOUT requiring a
/// `TRAINER_NONE` sentinel at entry 0.
///
/// Used as a fallback when the sentinel-based scanner fails, e.g. because the
/// ROM stores a non-null `partyPtr` for `TRAINER_NONE` AND that pointer doesn't
/// satisfy the (relaxed) two-byte sentinel check at a 4-byte boundary.
///
/// Instead of looking for a specific sentinel pattern, this scanner looks for
/// any 4-byte aligned offset where `MIN_TRAINER_RUN` consecutive 40-byte
/// blocks all pass the structural `looks_like_trainer_entry` check AND the
/// derived table passes the party-count and gym-leader spot-checks.
///
/// False-positive risk is very low: a run of 50 consecutive valid-looking
/// 40-byte blocks occurs with probability ≈ (4/256 × 7/256)^50 in random
/// data.  The party-count check (15 entries with non-empty, species-valid
/// party data) provides an additional independent filter.
fn find_trainer_table_no_sentinel(rom: &[u8]) -> Option<usize> {
    tracing::info!(
        "gTrainers no-sentinel scan: searching {} MiB of ROM",
        rom.len() / (1024 * 1024)
    );
    let mut i = 0usize;
    while i + MIN_TRAINER_RUN * TRAINER_ENTRY_SIZE <= rom.len() {
        // Quick pre-filter identical to the first two checks in
        // looks_like_trainer_entry, so we skip non-starters in O(1).
        if rom[i] > 3 { i += 4; continue; }          // partyFlags
        if rom[i + 0x20] > 6 { i += 4; continue; }   // partySize
        // Full run check (no sentinel required at i).
        if !(0..MIN_TRAINER_RUN)
            .all(|j| looks_like_trainer_entry(rom, i + j * TRAINER_ENTRY_SIZE))
        {
            i += 4;
            continue;
        }
        if count_entries_with_valid_party(rom, i, MIN_TRAINER_RUN) < MIN_TRAINER_WITH_PARTY {
            i += 4;
            continue;
        }
        if count_valid_leader_spots(rom, i) < MIN_VALID_LEADER_SPOTS {
            i += 4;
            continue;
        }
        tracing::info!("gTrainers no-sentinel: table found at {:#X}", i);
        return Some(i);
    }
    tracing::info!("gTrainers no-sentinel: table not found");
    None
}

/// Known vanilla FireRed gym-leader parties used as ROM-scan anchors.
///
/// Each entry is `(trainer_index, &[(level, species)])` for the leader's
/// full party.  The iv byte at the start of each mon struct is skipped
/// (wildcard).  All four party struct formats are tried automatically:
/// 4 bytes/mon (no item, default moves), 8 bytes/mon (has item, default
/// moves), 12 bytes/mon (no item, custom moves), 16 bytes/mon (both).
///
/// Anchors are tried in order; the first that resolves a valid table wins.
const LEADER_PARTY_ANCHORS: &[(usize, &[(u8, u16)])] = &[
    (54, &[(12, 74), (14, 95)]),    // Brock: Geodude L12, Onix L14
    (55, &[(18, 120), (21, 121)]),  // Misty: Staryu L18, Starmie L21
];

/// Per-mon byte stride and matching `partyFlags` value for each GBA trainer
/// party struct layout.
///
/// In every layout the first 4 bytes of each mon are `[iv, level,
/// species_lo, species_hi]`, so the per-mon search pattern is the same
/// regardless of stride; only the distance between consecutive mons differs.
const PARTY_STRIDES: &[(usize, u8)] = &[
    (4,  0), // no item, default moves
    (8,  2), // held item, default moves
    (12, 1), // no item, custom moves
    (16, 3), // held item, custom moves
];

/// Tries to locate the `gTrainers` table by finding a gym leader's known
/// party data in the ROM and working backwards via the `partyPtr` field.
///
/// Useful for ROM hacks that relocate `gTrainers` but keep vanilla
/// gym-leader Pokémon and levels, AND whose trainer table lacks the
/// canonical `TRAINER_NONE` sentinel at entry 0 that the regular scanner
/// requires.
///
/// For each anchor leader, all four party struct strides are attempted so
/// this works even if the ROM hack assigned held items or custom moves to
/// gym leaders.
fn find_trainer_table_by_party_anchor(rom: &[u8]) -> Option<usize> {
    for &(trainer_idx, party) in LEADER_PARTY_ANCHORS {
        for &(stride, expected_flags) in PARTY_STRIDES {
            if let Some(table) = try_anchor(rom, trainer_idx, party, stride, expected_flags) {
                return Some(table);
            }
        }
    }
    None
}

fn try_anchor(
    rom: &[u8],
    trainer_idx: usize,
    party: &[(u8, u16)],
    stride: usize,
    expected_flags: u8,
) -> Option<usize> {
    let mon_count = party.len();
    let party_bytes = stride * mon_count;

    // Step 1: scan the ROM for the party data.
    // Each mon in the struct is `stride` bytes; the first 4 bytes of every
    // mon are always [iv, level, species_lo, species_hi] regardless of
    // stride.  The iv byte (+0) is skipped (wildcard).
    // GBA ROM party data is 4-byte aligned, so we step by 4.
    let mut party_off = 0usize;
    'party: while party_off + party_bytes <= rom.len() {
        for (i, &(level, species)) in party.iter().enumerate() {
            let base = party_off + i * stride;
            if rom[base + 1] != level { party_off += 4; continue 'party; }
            if rom[base + 2] != (species & 0xFF) as u8 { party_off += 4; continue 'party; }
            if rom[base + 3] != (species >> 8) as u8 { party_off += 4; continue 'party; }
        }

        // Found a candidate match.  Compute the GBA bus address that a ROM
        // pointer to this party data would carry, then search for it stored
        // as the `partyPtr` field (offset +0x24 inside the 40-byte entry).
        let party_ptr = 0x0800_0000u32.wrapping_add(party_off as u32);
        let ptr_le = party_ptr.to_le_bytes();

        for ptr_off in 0..rom.len().saturating_sub(4) {
            if rom[ptr_off..ptr_off + 4] != ptr_le {
                continue;
            }

            let entry_start = match ptr_off.checked_sub(0x24) {
                Some(e) => e,
                None => continue,
            };
            let table_start = match entry_start.checked_sub(trainer_idx * TRAINER_ENTRY_SIZE) {
                Some(t) => t,
                None => continue,
            };

            if table_start % 4 != 0 { continue; }
            if entry_start + TRAINER_ENTRY_SIZE > rom.len() { continue; }

            let entry = &rom[entry_start..entry_start + TRAINER_ENTRY_SIZE];
            if entry[0] != expected_flags { continue; }         // partyFlags must match stride
            if entry[0x20] as usize != mon_count { continue; }  // partySize matches

            tracing::info!(
                "gTrainers anchor: trainer-{} party (stride={}, partyFlags={}) \
                 at ROM {:#X} (partyPtr={:#010X}) → entry at {:#X} → table at {:#X}",
                trainer_idx, stride, expected_flags,
                party_off, party_ptr, entry_start, table_start,
            );
            return Some(table_start);
        }

        party_off += 4;
    }
    None
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

/// Pins the `gTrainers` table to a specific ROM file offset, bypassing all
/// auto-detection.
///
/// Must be called **before** [`fill_rom`] / [`init_rom`].  Subsequent calls
/// are silently ignored (the value is set once).
///
/// `addr` is the byte offset *within the ROM file* (not the GBA bus address).
/// For example, if a ROM tool reports the table at bus address `0x08240000`,
/// pass `0x240000`.
pub fn set_trainer_table_addr_override(addr: usize) {
    let _ = TRAINER_TABLE_OVERRIDE.set(addr);
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
    ROM_ADDRESSES.get_or_init(|| {
        let mut addrs = addresses_for(*rev);

        // User-supplied override takes priority — skip all scanning.
        if let Some(&override_addr) = TRAINER_TABLE_OVERRIDE.get() {
            tracing::info!(
                "gTrainers: using user-supplied ROM offset {:#X} (auto-detection skipped)",
                override_addr
            );
            addrs.trainer_data_addr = override_addr;
            return addrs;
        }

        // Validate the hardcoded trainer_data_addr. ROM hacks often relocate
        // gTrainers; if the expected offset doesn't hold valid entries, scan
        // the first 8 MiB to find the real table.
        if addrs.trainer_data_addr != 0 {
            let hca = addrs.trainer_data_addr;

            // --- per-sub-check diagnostics (always logged at INFO) ----------
            let none_ok = is_trainer_none_entry(rom, hca);
            let run_fail_idx = if none_ok {
                (0..MIN_TRAINER_RUN)
                    .find(|&i| !looks_like_trainer_entry(rom, hca + i * TRAINER_ENTRY_SIZE))
            } else {
                None
            };
            let run_ok = none_ok && run_fail_idx.is_none();
            let party_count = if run_ok {
                count_entries_with_valid_party(rom, hca, MIN_TRAINER_RUN)
            } else {
                0
            };
            let leader_spots = if run_ok { count_valid_leader_spots(rom, hca) } else { 0 };
            tracing::info!(
                "gTrainers hardcoded-addr {:#X}: none_ok={} run_ok={} \
                 party_count={}/{} leader_spots={}/{}",
                hca, none_ok, run_ok,
                party_count, MIN_TRAINER_WITH_PARTY,
                leader_spots, MIN_VALID_LEADER_SPOTS,
            );
            // Dump 32 bytes starting at hca so the surrounding context is
            // visible, making it possible to identify what IS at that offset.
            {
                let end = (hca + 32).min(rom.len());
                let chunk = &rom[hca.min(rom.len())..end];
                let hex: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        if i > 0 && i % 8 == 0 { format!("  {:02X}", b) }
                        else { format!(" {:02X}", b) }
                    })
                    .collect();
                tracing::info!("gTrainers[0] ROM bytes +0x00..+0x1F:{}", hex);
            }
            // Also dump the key structural fields explicitly.
            if hca + TRAINER_ENTRY_SIZE <= rom.len() {
                tracing::info!(
                    "gTrainers[0] fields — partyFlags(+0x00)={:#04X} \
                     partySize(+0x20)={:#04X} partyPtr(+0x24)={:#010X}",
                    rom[hca],
                    rom[hca + 0x20],
                    u32::from_le_bytes([
                        rom[hca + 0x24], rom[hca + 0x25],
                        rom[hca + 0x26], rom[hca + 0x27],
                    ])
                );
            }
            if let Some(idx) = run_fail_idx {
                let off = hca + idx * TRAINER_ENTRY_SIZE;
                tracing::warn!(
                    "gTrainers: entry[{}] at {:#X} fails structural check — \
                     partyFlags={:#04X} partySize={} partyPtr={:#010X}",
                    idx, off,
                    rom[off],
                    rom[off + 0x20],
                    u32::from_le_bytes([
                        rom[off + 0x24], rom[off + 0x25],
                        rom[off + 0x26], rom[off + 0x27],
                    ])
                );
            }
            // ----------------------------------------------------------------

            if !validate_trainer_table_at(rom, hca) {
                match find_trainer_table(rom) {
                    Some(found) => {
                        tracing::info!(
                            "gTrainers scan: table found at ROM offset {:#X} \
                             (hardcoded {:#X} was invalid — likely a ROM hack)",
                            found, hca
                        );
                        addrs.trainer_data_addr = found;
                    }
                    None => {
                        tracing::info!(
                            "gTrainers scan: sentinel-based scan found nothing; \
                             trying sentinelless scan…"
                        );
                        let no_sentinel = find_trainer_table_no_sentinel(rom);
                        let fallback = no_sentinel.or_else(|| {
                            tracing::info!(
                                "gTrainers no-sentinel scan found nothing; \
                                 trying party-data anchor…"
                            );
                            find_trainer_table_by_party_anchor(rom)
                        });
                        match fallback {
                            Some(found) => {
                                tracing::info!(
                                    "gTrainers fallback: table found at ROM offset {:#X}",
                                    found
                                );
                                addrs.trainer_data_addr = found;
                            }
                            None => {
                                tracing::warn!(
                                    "gTrainers scan: could not locate trainer table — \
                                     vs_leader overlay will be unavailable for this ROM. \
                                     If you know the ROM offset (e.g. from a hex editor), \
                                     set `trainer_table_rom_offset = 0xXXXXXX` in the \
                                     aggregator config to bypass auto-detection."
                                );
                                addrs.trainer_data_addr = 0;
                            }
                        }
                    }
                }
            }
        }
        addrs
    });
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
            LEAFGREEN_USA_REV1.save_block_2_base,
            FIRERED_USA_REV1.save_block_2_base
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
            LEAFGREEN_USA_REV1.save_block_2_ptr,
            FIRERED_USA_REV1.save_block_2_ptr
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
