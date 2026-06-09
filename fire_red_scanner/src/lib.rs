use fire_red_get_values::*;

/// Size in bytes of a single wild encounter header entry.
/// 
/// Each header contains
/// 
/// - Map group ('u8')
/// - Map number ('u8')
/// - Padding ('u16')
/// - Four encounter table pointers ('u32' each)
/// 
/// total size: 20 bytes.
const HEADER_SIZE: usize = 20;

/// Reads a little-endian `u32` value from a byte slice.
/// 
/// # Arguments
/// 
/// * `bytes` - Source byte buffer.
/// * `offset` - Starting index to read from.
/// 
/// # Returns
/// 
/// - `u32` value read from the buffer, or 0 if the offset is out of bounds.
fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or_else(|_| 0u32.to_le_bytes()))
}

/// Checks whether a values looks like a valid GBA ROM pointer.
/// 
/// Valid pointers are expected to fall within the GBA ROM address range of
/// 
/// - 0x08000000 to 0x09FFFFFF
/// 
/// A null pointer (`0`) is also considered valid.
/// 
/// # Arguments
/// 
/// * `ptr` - Value to check.
fn is_valid_gba_ptr(ptr: u32) -> bool {
    (0x08000000..=0x09FFFFFF).contains(&ptr) || ptr == 0
}

/// Performs a heuristic check on a potential wild encounter header
/// 
/// This function checks:
/// 
/// - Header fits within ROM bounds
/// - Padding bytes are zero
/// - Map group and map number are within expected ranges
/// - Encounter tables pointers appear valid
/// 
/// # Arguments
/// 
/// * `rom` - Complete ROM byte buffer
/// * `offset` - Starting index of the potential header
/// 
/// # Returns
/// 
/// `true` if the data resembles a valid FireRed wild encounter header.
fn looks_like_header(rom: &[u8], offset: usize) -> bool {
    if offset + HEADER_SIZE > rom.len() {
        return false;
    }

    let map_group = rom[offset];
    let map_num = rom[offset + 1];
    let padding: u16 = read_u16(rom, offset + 2);

    if padding != 0 {
        return false;
    }

    // Basic sanity checks (tuned for FireRed)
    if map_group > 50 || map_num > 200 {
        return false;
    }

    let grass = read_u32_le(rom, offset + 4);
    let water = read_u32_le(rom, offset + 8);
    let rock  = read_u32_le(rom, offset + 12);
    let fish  = read_u32_le(rom, offset + 16);

    // All four encounter table pointers must be valid (zero = no encounters for that type)
    is_valid_gba_ptr(grass)
        && is_valid_gba_ptr(water)
        && is_valid_gba_ptr(rock)
        && is_valid_gba_ptr(fish)
}

/// Validates a potential wild encounter header table.
/// 
/// The table is scanned sequentially until either:
/// 
/// - An invalid header is found
/// - A terminating sentinel (`0xFF`) is found
/// 
/// # Sentinel Format
/// 
/// ```text
/// map_group == 0xFF
/// ```
/// 
/// # Arguments
/// 
/// * `rom` - Complete ROM byte buffer
/// * `start` - Starting index of the potential header table
/// 
/// # Returns
/// 
/// `true` if the structure appears to be a valid encounter table.
fn validate_table(rom: &[u8], start: usize) -> bool {
    let mut offset = start;
    let mut count = 0;

    while offset + HEADER_SIZE <= rom.len() {
        let map_group = rom[offset];

        // End of table sentinel
        if map_group == 0xFF {
            return count > 50; // must be large enough to be real
        }

        if !looks_like_header(rom, offset) {
            return false;
        }

        count += 1;
        offset += HEADER_SIZE;
    }

    false
}

// ---------------------------------------------------------------------------
// gMapGroupsAndMaps scanner
// ---------------------------------------------------------------------------

/// Converts a GBA ROM bus address to a byte offset within the ROM buffer.
///
/// Returns `None` if `ptr` is zero or outside the ROM address range
/// `0x08000000..=0x09FFFFFF`.
fn rom_ptr_to_offset(ptr: u32) -> Option<usize> {
    if ptr != 0 && (0x08000000..=0x09FFFFFF).contains(&ptr) {
        Some((ptr - 0x08000000) as usize)
    } else {
        None
    }
}

/// Validates a candidate ROM offset as `gMapGroupsAndMaps` by following two
/// levels of pointers for each known `(group, map)` pair and checking that the
/// destination looks like a [`MapHeader`].
///
/// A `MapHeader` is expected to begin with three non-null ROM pointers (footer,
/// events, scripts) followed by a fourth that may be zero (connections).
fn validate_map_groups_table(rom: &[u8], offset: usize, known_pairs: &[(u8, u8)]) -> bool {
    for &(group, map) in known_pairs {
        // Level 1: table[group] → pointer to the group's map-header pointer array.
        let l1 = offset + group as usize * 4;
        if l1 + 4 > rom.len() {
            return false;
        }
        let group_offset = match rom_ptr_to_offset(read_u32(rom, l1)) {
            Some(o) => o,
            None    => return false,
        };

        // Level 2: group_array[map] → pointer to the MapHeader.
        let l2 = group_offset + map as usize * 4;
        if l2 + 4 > rom.len() {
            return false;
        }
        let map_offset = match rom_ptr_to_offset(read_u32(rom, l2)) {
            Some(o) => o,
            None    => return false,
        };

        // Level 3: MapHeader — first three pointer fields must be valid and non-null;
        // the fourth (connections) is allowed to be zero.
        if map_offset + 16 > rom.len() {
            return false;
        }
        let footer  = read_u32(rom, map_offset);
        let events  = read_u32(rom, map_offset + 4);
        let scripts = read_u32(rom, map_offset + 8);
        let conns   = read_u32(rom, map_offset + 12);

        if rom_ptr_to_offset(footer).is_none()
            || rom_ptr_to_offset(events).is_none()
            || rom_ptr_to_offset(scripts).is_none()
            || !is_valid_gba_ptr(conns)
        {
            return false;
        }
    }
    true
}

/// Scans a FireRed ROM for the `gMapGroupsAndMaps` pointer table.
///
/// Uses known valid `(map_group, map_num)` pairs as anchors: for each
/// candidate offset the scanner follows
/// `table[group]` → `group_array[map]` → `MapHeader`
/// for every pair and rejects the candidate if any level fails pointer
/// validation. Passing pairs from at least two different groups greatly
/// reduces false-positive risk.
///
/// # Arguments
///
/// * `rom`         — Complete ROM byte buffer.
/// * `known_pairs` — At least two valid `(map_group, map_num)` pairs, ideally
///   from different groups. Pairs from the wild-encounter header table work well.
///
/// # Returns
///
/// * `Some(offset)` — ROM byte offset of `gMapGroupsAndMaps`.
/// * `None`         — Table could not be located.
pub fn find_map_groups_table(rom: &[u8], known_pairs: &[(u8, u8)]) -> Option<usize> {
    if known_pairs.is_empty() {
        tracing::warn!("find_map_groups_table: no known (group, map) pairs supplied");
        return None;
    }

    let mut i = 0;
    while i + 4 <= rom.len() {
        // Fast pre-check: candidate must start with a valid non-null ROM pointer.
        if rom_ptr_to_offset(read_u32(rom, i)).is_some()
            && validate_map_groups_table(rom, i, known_pairs)
        {
            return Some(i);
        }
        i += 4; // ROM pointers are always 4-byte aligned.
    }

    None
}

// ---------------------------------------------------------------------------
// Wild encounter header scanner
// ---------------------------------------------------------------------------

/// Scans a FireRed ROM for the wild encounter header table
/// 
/// The ROM is scanned in 4-byte aligned increments searching for a sequence
/// of valid wild encounter headers followed by a valid sentinel.
/// 
/// # Arguments
/// 
/// * `rom` - Complete ROM byte buffer
/// 
/// # Returns
/// 
/// - `Some(offset)` if a valid wild encounter table is found.
/// - `None` if no valid table could be located.
/// 
/// # Notes
/// 
/// This function uses heuristic validation and is designed specifically
/// around Pokemon FirERed ROM layouts.
pub fn find_wild_headers(rom: &[u8]) -> Option<usize> {
    let mut i = 0;

    while i + HEADER_SIZE <= rom.len() {
        if looks_like_header(rom, i)
            && validate_table(rom, i) {
            return Some(i);
        }

        i += 4; // aligned scan
    }

    None
}