//! Heuristic scanner for wild encounter table headers in a FireRed ROM.
//!
//! [`find_wild_headers`] walks the ROM looking for byte patterns that match
//! the 20-byte wild encounter header struct.  Results feed
//! `fire_red_pokemon_data` so the overlay can show per-route encounter tables.

use fire_red_get_values::*;

/// Size in bytes of a single wild encounter header entry.
///
/// Each header contains
///
/// - Map group (`u8`)
/// - Map number (`u8`)
/// - Padding (`u16`)
/// - Four encounter table pointers (`u32` each)
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
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .unwrap_or_else(|_| 0u32.to_le_bytes()),
    )
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
    let rock = read_u32_le(rom, offset + 12);
    let fish = read_u32_le(rom, offset + 16);

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
            None => return false,
        };

        // Level 2: group_array[map] → pointer to the MapHeader.
        let l2 = group_offset + map as usize * 4;
        if l2 + 4 > rom.len() {
            return false;
        }
        let map_offset = match rom_ptr_to_offset(read_u32(rom, l2)) {
            Some(o) => o,
            None => return false,
        };

        // Level 3: MapHeader — first three pointer fields must be valid and non-null;
        // the fourth (connections) is allowed to be zero.
        if map_offset + 16 > rom.len() {
            return false;
        }
        let footer = read_u32(rom, map_offset);
        let events = read_u32(rom, map_offset + 4);
        let scripts = read_u32(rom, map_offset + 8);
        let conns = read_u32(rom, map_offset + 12);

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
/// around Pokemon FireRed ROM layouts.
pub fn find_wild_headers(rom: &[u8]) -> Option<usize> {
    let mut i = 0;

    while i + HEADER_SIZE <= rom.len() {
        if looks_like_header(rom, i) && validate_table(rom, i) {
            return Some(i);
        }

        i += 4; // aligned scan
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const ROM_BASE: u32 = 0x0800_0000;

    /// Filler byte for synthetic ROMs. 0x01 everywhere means any candidate
    /// header has non-zero padding and any candidate pointer reads as
    /// 0x01010101 (outside the ROM address range), so filler regions can
    /// never validate as a table.
    const FILLER: u8 = 0x01;

    fn write_ptr(rom: &mut [u8], offset: usize, ptr: u32) {
        rom[offset..offset + 4].copy_from_slice(&ptr.to_le_bytes());
    }

    /// Writes one 20-byte wild encounter header with all four table pointers
    /// pointing at the start of ROM (a valid GBA ROM pointer).
    fn write_wild_header(rom: &mut [u8], offset: usize, group: u8, map: u8) {
        rom[offset] = group;
        rom[offset + 1] = map;
        rom[offset + 2] = 0;
        rom[offset + 3] = 0;
        for i in 0..4 {
            write_ptr(rom, offset + 4 + i * 4, ROM_BASE);
        }
    }

    /// Builds a ROM containing `count` valid headers at `table_start`,
    /// followed by the 0xFF end-of-table sentinel.
    fn rom_with_wild_table(table_start: usize, count: usize) -> Vec<u8> {
        let mut rom = vec![FILLER; table_start + (count + 1) * HEADER_SIZE + 64];
        for n in 0..count {
            write_wild_header(&mut rom, table_start + n * HEADER_SIZE, (n / 8) as u8, (n % 8) as u8);
        }
        rom[table_start + count * HEADER_SIZE] = 0xFF; // sentinel
        rom
    }

    // ── rom_ptr_to_offset ────────────────────────────────────────────────

    #[test]
    fn rom_ptr_to_offset_maps_bus_address() {
        assert_eq!(rom_ptr_to_offset(0x0800_0000), Some(0));
        assert_eq!(rom_ptr_to_offset(0x0812_3456), Some(0x12_3456));
        assert_eq!(rom_ptr_to_offset(0x09FF_FFFF), Some(0x01FF_FFFF));
    }

    #[test]
    fn rom_ptr_to_offset_rejects_null_and_out_of_range() {
        assert_eq!(rom_ptr_to_offset(0), None);
        assert_eq!(rom_ptr_to_offset(0x07FF_FFFF), None);
        assert_eq!(rom_ptr_to_offset(0x0A00_0000), None);
        assert_eq!(rom_ptr_to_offset(0x0200_0000), None); // EWRAM, not ROM
    }

    // ── looks_like_header ────────────────────────────────────────────────

    #[test]
    fn looks_like_header_accepts_valid_header() {
        let mut rom = vec![FILLER; 64];
        write_wild_header(&mut rom, 0, 3, 40);
        assert!(looks_like_header(&rom, 0));
    }

    #[test]
    fn looks_like_header_accepts_null_table_pointers() {
        let mut rom = vec![FILLER; 64];
        write_wild_header(&mut rom, 0, 3, 40);
        write_ptr(&mut rom, 4, 0); // grass = null (no encounters of that type)
        assert!(looks_like_header(&rom, 0));
    }

    #[test]
    fn looks_like_header_rejects_nonzero_padding() {
        let mut rom = vec![FILLER; 64];
        write_wild_header(&mut rom, 0, 3, 40);
        rom[2] = 1;
        assert!(!looks_like_header(&rom, 0));
    }

    #[test]
    fn looks_like_header_rejects_out_of_range_group_and_map() {
        let mut rom = vec![FILLER; 64];
        write_wild_header(&mut rom, 0, 51, 40); // group > 50
        assert!(!looks_like_header(&rom, 0));
        write_wild_header(&mut rom, 0, 3, 201); // map > 200
        assert!(!looks_like_header(&rom, 0));
    }

    #[test]
    fn looks_like_header_rejects_invalid_table_pointer() {
        let mut rom = vec![FILLER; 64];
        write_wild_header(&mut rom, 0, 3, 40);
        write_ptr(&mut rom, 16, 0x0300_0000); // IWRAM address, not ROM
        assert!(!looks_like_header(&rom, 0));
    }

    #[test]
    fn looks_like_header_rejects_truncated_buffer() {
        let mut rom = vec![FILLER; HEADER_SIZE];
        write_wild_header(&mut rom, 0, 3, 40);
        assert!(!looks_like_header(&rom[..HEADER_SIZE - 1], 0));
    }

    // ── find_wild_headers ────────────────────────────────────────────────

    #[test]
    fn find_wild_headers_locates_table() {
        let rom = rom_with_wild_table(0x80, 60);
        assert_eq!(find_wild_headers(&rom), Some(0x80));
    }

    #[test]
    fn find_wild_headers_rejects_table_of_50_or_fewer_entries() {
        // validate_table requires strictly more than 50 headers before the
        // sentinel, so a 50-entry table must be treated as a false positive.
        let rom = rom_with_wild_table(0x80, 50);
        assert_eq!(find_wild_headers(&rom), None);
    }

    #[test]
    fn find_wild_headers_accepts_table_of_51_entries() {
        let rom = rom_with_wild_table(0x80, 51);
        assert_eq!(find_wild_headers(&rom), Some(0x80));
    }

    #[test]
    fn find_wild_headers_rejects_table_without_sentinel() {
        let mut rom = rom_with_wild_table(0x80, 60);
        let sentinel = 0x80 + 60 * HEADER_SIZE;
        rom[sentinel] = FILLER; // corrupt the 0xFF terminator
        assert_eq!(find_wild_headers(&rom), None);
    }

    #[test]
    fn find_wild_headers_rejects_corrupt_entry_mid_table() {
        let mut rom = rom_with_wild_table(0x80, 60);
        rom[0x80 + 30 * HEADER_SIZE + 2] = 1; // non-zero padding in entry 30
        assert_eq!(find_wild_headers(&rom), None);
    }

    #[test]
    fn find_wild_headers_empty_rom_returns_none() {
        assert_eq!(find_wild_headers(&[]), None);
        assert_eq!(find_wild_headers(&vec![FILLER; 4096]), None);
    }

    // ── find_map_groups_table ────────────────────────────────────────────

    const GROUPS_TABLE: usize = 0x100;
    const GROUP_ARRAYS: usize = 0x200;
    const MAP_HEADERS: usize = 0x400;

    /// Builds a ROM containing a valid gMapGroupsAndMaps structure:
    /// 4 groups of 8 maps, every map pointing at a valid MapHeader whose
    /// first three pointers are non-null and whose connections pointer is 0.
    fn rom_with_map_groups() -> Vec<u8> {
        let mut rom = vec![FILLER; 0x1000];
        for group in 0..4usize {
            let group_array = GROUP_ARRAYS + group * 0x40;
            write_ptr(&mut rom, GROUPS_TABLE + group * 4, ROM_BASE + group_array as u32);
            for map in 0..8usize {
                let header = MAP_HEADERS + (group * 8 + map) * 0x20;
                write_ptr(&mut rom, group_array + map * 4, ROM_BASE + header as u32);
                write_ptr(&mut rom, header, ROM_BASE + 0x800); // footer
                write_ptr(&mut rom, header + 4, ROM_BASE + 0x800); // events
                write_ptr(&mut rom, header + 8, ROM_BASE + 0x800); // scripts
                write_ptr(&mut rom, header + 12, 0); // connections may be null
            }
        }
        rom
    }

    #[test]
    fn find_map_groups_table_locates_table() {
        let rom = rom_with_map_groups();
        let pairs = [(0u8, 1u8), (2, 3)];
        assert_eq!(find_map_groups_table(&rom, &pairs), Some(GROUPS_TABLE));
    }

    #[test]
    fn find_map_groups_table_requires_known_pairs() {
        let rom = rom_with_map_groups();
        assert_eq!(find_map_groups_table(&rom, &[]), None);
    }

    #[test]
    fn find_map_groups_table_rejects_when_headers_invalid() {
        let mut rom = rom_with_map_groups();
        // Corrupt the footer pointer of every MapHeader so no candidate can
        // pass level-3 validation anywhere in the ROM.
        for n in 0..32usize {
            write_ptr(&mut rom, MAP_HEADERS + n * 0x20, 0);
        }
        assert_eq!(find_map_groups_table(&rom, &[(0, 1), (2, 3)]), None);
    }

    #[test]
    fn find_map_groups_table_rejects_pair_outside_table() {
        let rom = rom_with_map_groups();
        // Group 40 reads filler bytes as its group-array pointer, which is
        // not a valid ROM pointer, so validation must fail for this pair.
        assert_eq!(find_map_groups_table(&rom, &[(0, 1), (40, 0)]), None);
    }

    #[test]
    fn find_map_groups_table_empty_rom_returns_none() {
        assert_eq!(find_map_groups_table(&[], &[(0, 1)]), None);
        assert_eq!(find_map_groups_table(&vec![FILLER; 4096], &[(0, 1)]), None);
    }
}
