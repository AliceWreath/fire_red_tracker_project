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
    (ptr >= 0x08000000 && ptr <= 0x09FFFFFF) || ptr == 0
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
    let padding: u16 = read_u16(&rom, offset + 2);

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

    // At least one valid pointer
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
        if looks_like_header(rom, i) {
            if validate_table(rom, i) {
                return Some(i);
            }
        }

        i += 4; // aligned scan
    }

    None
}