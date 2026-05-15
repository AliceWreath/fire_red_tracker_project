use fire_red_get_values::*;

const HEADER_SIZE: usize = 20;

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn is_valid_gba_ptr(ptr: u32) -> bool {
    (ptr >= 0x08000000 && ptr < 0x09000000) || ptr == 0
}

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

pub fn find_wild_headers(rom: &[u8]) -> Option<usize> {
    let mut i = 0;

    while i < rom.len() - HEADER_SIZE {
        if looks_like_header(rom, i) {
            if validate_table(rom, i) {
                return Some(i);
            }
        }

        i += 4; // aligned scan
    }

    None
}