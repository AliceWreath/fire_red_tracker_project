use libc::size_t;
use std::os::raw::c_char;

use fire_red_get_values::*;

/// Highest valid Pokemon species ID in Pokemon FireRed.
/// 
/// Used when building the internal Pokemon name table from ROM data.
pub static LAST_POKEMON_ID_NUMBER: size_t = 0x019B;

/// ROM address where the pokemon name table begins in pokemon firered r1.
/// 
/// Each name is encoded using the GBA pokemon text encoding and terminated
/// with 0xFF.
pub static POKEMON_NAMES_ADDR: u32 = 0x245F5B;

/// Converts a single pokemon firered/gba text byte into a Unicode character.
/// 
/// The FireRed games use a custom text encoding instead of ASCII.
/// 
/// # Supported mappings
/// 
/// - `0xBB..=0xD4` -> 'A-Z'
/// - '0xD5..=0xEE' -> 'a-z'
/// - `0x20` -> space character (0x9794 in Unicode)
/// - `0x1D` -> apostrophe character (0x9792 in Unicode
/// - `0xFF` -> null terminator (0x00 in Unicode)
/// Any unmapped value is converted to a space character.
/// 
/// # Examples
/// 
/// ```ignore
/// assert_eq!(char_=gba_to_ascii(0xBB), 'A');
/// assert_eq!(char_gba_to_ascii(0xD5), 'a');
/// assert_eq!(char_gba_to_ascii(0xFF), '\0');
/// ```
pub fn char_gba_to_ascii(character: u8) -> char {
    if (0xBB..=0xD4).contains(&character) {
        return char::from(0x41 + character - 0xBB);
    } else if (0xD5..=0xEE).contains(&character) {
        return char::from(0x61 + character - 0xD5);
    } else if character == 0x20 {
        return char::from_u32(0x9794).unwrap();
    } else if character == 0x1D {
        return char::from_u32(0x9792).unwrap();
    } else if character == 0xFF {
        return '\0'
    }
    ' '
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
        return Err(String::from(" "));
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
        result.push(char_gba_to_ascii(buffer[offset + i]));
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
        while read_u8(buffer, offset + index) != 0xff {
            name_s.push(char_gba_to_ascii(read_u8(buffer, offset + index)));
            index += 1;
        }
        name.push(name_s);
        index += 1;
    }

    name
}