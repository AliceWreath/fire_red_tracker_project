//! # FireRed text
//! 
//! Converts pokemon/gba text bytes into human-readable text
use libc::size_t;
use std::os::raw::c_char;

use fire_red_get_values::*;

/// Highest valid Pokemon species ID in Pokemon FireRed.
///
/// Used when building the internal Pokemon name table from ROM data.
pub static LAST_POKEMON_ID_NUMBER: size_t = 0x019B;

/// Converts a single pokemon firered/gba text byte into a Unicode character.
///
/// The FireRed games use a custom text encoding instead of ASCII.
///
/// # Supported mappings
///
/// - `0xA1..=0xAA` → '0'–'9'
/// - `0xAB`        → '!'
/// - `0xAC`        → '?'
/// - `0xAD`        → '.'
/// - `0xAE`        → '-'
/// - `0xB1`        → '\'' (apostrophe, e.g. KING'S ROCK)
/// - `0xB7`        → ','
/// - `0xBB..=0xD4` → 'A'–'Z'
/// - `0xD5..=0xEE` → 'a'–'z'
/// - `0xB5`        → '♂'  (used in Pokémon names, e.g. NIDORAN♂)
/// - `0xB6`        → '♀'  (used in Pokémon names, e.g. NIDORAN♀)
/// - `0x1D`        → '♀'  (used in battle/UI text)
/// - `0x20`        → '♂'  (used in battle/UI text)
/// - `0xFF`        → '\0' (null terminator)
///
/// Any other byte maps to a space character.
pub fn char_gba_to_ascii(character: u8) -> char {
    match character {
        0xA1..=0xAA => char::from(character - 0xA1 + b'0'),
        0xAB => '!',
        0xAC => '?',
        0xAD => '.',
        0xAE => '-',
        0xB1 => '\'',
        0xB7 => ',',
        0xBB..=0xD4 => char::from(character - 0xBB + b'A'),
        0xD5..=0xEE => char::from(character - 0xD5 + b'a'),
        0xB5 => '♂',
        0xB6 => '♀',
        0x20 => '♂',
        0x1D => '♀',
        0xFF => '\0',
        _ => ' ',
    }
}

/// Retrieves the nmame of a pokemon from the cached name repository by its species ID.
/// 
/// Returns an error if the species index is out of bounds.
/// 
/// # Arguments
/// 
/// * 'species' - Pokemon species ID.
/// 
/// # Returns
/// 
/// - 'Ok(String)' contains the pokemon name as a String.
/// - 'Err(String)' contains an error message if the species ID is invalid.
/// 
/// # Examples
/// 
/// ```ignore
/// let name = get_pokemon_name_by_number(25).unwrap();
/// assert_eq!(name, "PIKACHU");
/// ```
pub fn get_pokemon_name_by_number(species: usize) -> Result<String, String> {
    if species > fire_red_pokemon_name_buffer::get_name_repo().len() - 1 {
        return Err(format!("species index {species} out of range"));
    }
    Ok(String::from(fire_red_pokemon_name_buffer::get_name_repo()[species].clone().trim()))
}


/// Converts a FireRed encoded byte slice into a UTF-8 string.
/// 
/// This function reads 'len' bytes starting from 'offset' and converts each character
/// using the ['char_gba_to_ascii'] function.
/// 
/// # Arguments
/// 
/// * 'buffer' - Raw ROM or memory data buffer.
/// * 'len' - Number of bytes to decode.
/// * 'offset' - Starting index in the buffer to read from.
/// 
/// # Examples
/// 
/// ```ignore
/// let ascii_string = gba_string_to_ascii(&rom_data, 10, 0x245F5B);
/// ```
pub fn gba_string_to_ascii(buffer: &[u8], len: usize, offset: usize) -> String {
    let mut result = String::new();
    for i in 0..len {
        match buffer.get(offset + i) {
            Some(&b) => result.push(char_gba_to_ascii(b)),
            None => break,
        }
    }
    result
}

/// FFI-safe arry of C strings.
/// 
/// Intended for interoperability with C or other foreign languages.
/// 
/// # Fields
/// 
/// * `arr` - Pointer to an array of `char*`
/// * 'len' - Number of strings stored
/// * 'capacity' - Total allocated buffer size in bytes.
/// 
/// # Safety
/// 
/// Memory ownership and deallocation must be handled carefully when passing
/// this struct across FFI boundaries to avoid leaks or undefined behavior.
#[repr(C)]
#[derive(Default)]
pub struct StringArray {
    arr: *mut *mut c_char,
    len: size_t,
    pub capacity: size_t,       //total allocation size in bytes
}

/// Builds the full pokemon name table from FireRed ROM data.
/// 
/// Names are read sequentially starting at 'offset' until
/// ['LAST_POKEMON_ID_NUMBER'] entries have been parsed.
/// 
/// Each string is terminated by the byte 0xFF.
/// 
/// The returned vector always includes an initial placeholder entry "_"
/// at index 0, so that the species ID can be used directly as an index into the vector.
/// 
/// # Arguments
/// 
/// * 'buffer' - ROM or emulator memory buffer.
/// * 'offset' - Starting offset of the pokemon name table.
/// 
/// # Returns
/// 
/// A vector containing all decoded pokemon names, indexed by species ID. The first entry (index 0) is a placeholder "_".
/// 
/// # Examples
/// 
/// ```ignore
/// let names = build_name_list(&rom_data, POKEMON_NAMES_ADDR as usize);
/// println!("{}", names[25]); // Should print "PIKACHU"
/// ```
pub fn build_name_list(buffer: &[u8], offset: usize) -> Vec<String> {
    let mut name: Vec<String> = Vec::new();
    let mut index = 0;    

    if name.is_empty() {
        name.push(String::from("_"));
    }
    
    while name.len() <= LAST_POKEMON_ID_NUMBER {
        let mut name_s = String::from("");
        while offset + index < buffer.len() && read_u8(buffer, offset + index) != 0xff {
            name_s.push(char_gba_to_ascii(read_u8(buffer, offset + index)));
            index += 1;
        }
        name.push(name_s);
        // Skip the 0xff terminator, guarding against a truncated buffer.
        if offset + index < buffer.len() {
            index += 1;
        } else {
            break;
        }
    }

    name
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── char_gba_to_ascii ─────────────────────────────────────────────────────

    #[test]
    fn uppercase_range() {
        assert_eq!(char_gba_to_ascii(0xBB), 'A');
        assert_eq!(char_gba_to_ascii(0xBB + 25), 'Z');
    }

    #[test]
    fn lowercase_range() {
        assert_eq!(char_gba_to_ascii(0xD5), 'a');
        assert_eq!(char_gba_to_ascii(0xD5 + 25), 'z');
    }

    #[test]
    fn null_terminator() {
        assert_eq!(char_gba_to_ascii(0xFF), '\0');
    }

    #[test]
    fn digit_range() {
        assert_eq!(char_gba_to_ascii(0xA1), '0');
        assert_eq!(char_gba_to_ascii(0xAA), '9');
        // spot-check a few in between
        assert_eq!(char_gba_to_ascii(0xA2), '1');
        assert_eq!(char_gba_to_ascii(0xA5), '4');
    }

    #[test]
    fn unmapped_byte_becomes_space() {
        assert_eq!(char_gba_to_ascii(0x00), ' ');
        assert_eq!(char_gba_to_ascii(0x42), ' ');
    }

    #[test]
    fn gender_symbols_in_names() {
        // 0xB5/0xB6 are the gender bytes used in Pokémon name table entries
        // (e.g. NIDORAN♂ and NIDORAN♀).
        assert_eq!(char_gba_to_ascii(0xB5), '♂');
        assert_eq!(char_gba_to_ascii(0xB6), '♀');
    }

    #[test]
    fn nidoran_names_are_distinct() {
        // NIDORAN♀  = C8 C3 BE C9 CC BB C8 B6 FF
        // NIDORAN♂  = C8 C3 BE C9 CC BB C8 B5 FF
        let nidoran_f = [0xC8u8, 0xC3, 0xBE, 0xC9, 0xCC, 0xBB, 0xC8, 0xB6, 0xFF];
        let nidoran_m = [0xC8u8, 0xC3, 0xBE, 0xC9, 0xCC, 0xBB, 0xC8, 0xB5, 0xFF];
        let name_f: String = nidoran_f.iter().copied()
            .take_while(|&b| b != 0xFF)
            .map(char_gba_to_ascii)
            .collect();
        let name_m: String = nidoran_m.iter().copied()
            .take_while(|&b| b != 0xFF)
            .map(char_gba_to_ascii)
            .collect();
        assert_eq!(name_f, "NIDORAN♀");
        assert_eq!(name_m, "NIDORAN♂");
        assert_ne!(name_f, name_m);
    }

    // ── gba_string_to_ascii ───────────────────────────────────────────────────

    #[test]
    fn decodes_pikachu() {
        // "PIKA" in GBA encoding: P=0xCA, I=0xC3, K=0xC5, A=0xBB
        let buf = [0xCA, 0xC3, 0xC5, 0xBB, 0xFF];
        assert_eq!(gba_string_to_ascii(&buf, 4, 0), "PIKA");
    }

    #[test]
    fn stops_at_len_before_null() {
        let buf = [0xBB, 0xBC, 0xFF, 0xBD]; // A, B, null, C
        assert_eq!(gba_string_to_ascii(&buf, 2, 0), "AB");
    }

    #[test]
    fn decodes_from_offset() {
        let buf = [0x00, 0x00, 0xBB, 0xBC]; // 2 padding, then A, B
        assert_eq!(gba_string_to_ascii(&buf, 2, 2), "AB");
    }

    #[test]
    fn empty_when_len_is_zero() {
        let buf = [0xBB, 0xBC];
        assert_eq!(gba_string_to_ascii(&buf, 0, 0), "");
    }
}