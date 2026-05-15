use libc::size_t;
use std::os::raw::c_char;

use fire_red_get_values::*;

pub static LAST_POKEMON_ID_NUMBER: size_t = 0x019B;
pub static POKEMON_NAMES_ADDR: u32 = 0x245F5B;

/// # Safety
/// 
/// This function is considered unsafe because it takes in a c_uchar (u8) from C
/*pub unsafe extern "C" fn c_char_gba_to_ascii(character: c_uchar) -> c_char {
    char_gba_to_ascii(character) as c_char
}*/

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

pub fn get_pokemon_name_by_number(species: usize) -> Result<String, String> {
    if species > fire_red_pokemon_name_buffer::get_name_repo().len() - 1 {
        return Err(String::from(" "));
    }
    Ok(String::from(fire_red_pokemon_name_buffer::get_name_repo()[species].clone().trim()))
}

pub fn gba_string_to_ascii(buffer: &[u8], len: usize, offset: usize) -> String {
    let mut result = String::new();
    for i in 0..len {
        result.push(char_gba_to_ascii(buffer[offset + i]));
    }
    result
}

#[repr(C)]
#[derive(Default)]
pub struct StringArray {
    arr: *mut *mut c_char,
    len: size_t,
    pub capacity: size_t,       //total allocation size in bytes
}

/// # Safety
/// 
/// this function is unsafe because it requires that the caller free the memory
/// by calling c_free_string_array after its done using the created array.
/*
#[unsafe(no_mangle)]
pub unsafe extern "C" fn c_build_name_list(
    buffer: *const c_uchar, 
    buffer_len: size_t,
    offset: size_t,
) -> StringArray {
    let buffer: &[u8] = if buffer.is_null() { 
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(buffer.add(offset), buffer_len) }
    };

    let names = build_name_list(buffer, 0);
    let len = names.len();

    // Total bytes needed for all strings (+1 for null terminator in each)
    let total_str_bytes: size_t = names
        .iter()
        .map(|s| s.len() + 1)
        .sum();

    // space for pointer array
    let ptr_array_size = len * std::mem::size_of::<*mut c_char>();

    // total allocaiton size
    let total_size = ptr_array_size + total_str_bytes;

    let align = std::mem::align_of::<*mut c_char>();
    let layout = Layout::from_size_align(total_size, align).unwrap();

    let raw = unsafe { alloc(layout) };
    if raw.is_null() {
        handle_alloc_error(layout);
    }

    // pointer array at start
    let ptr_array = raw as *mut *mut c_char;

    // string data comes after pointer array
    let mut str_data = unsafe { raw.add(ptr_array_size) };

    for (i, name) in names.into_iter().enumerate() {
        let cstr = CString::new(name).unwrap();
        let bytes = cstr.as_bytes_with_nul();

        // copy string bytes into buffer
        unsafe { 
            ptr::copy_nonoverlapping(bytes.as_ptr(), str_data, bytes.len()); 
 
            // point to this string
            *ptr_array.add(i) = str_data as *mut c_char;

            // advance pointer
            str_data = str_data.add(bytes.len());
        }
    }

    StringArray {
        arr: ptr_array,
        len,
        capacity: total_size,
    }
}

/// # Safety
/// 
/// this is considered unsafe because it takes data from C
pub unsafe extern "C" fn c_free_string_array(arr: StringArray) {
    if arr.arr.is_null() {
        return;
    }

    let align = std::mem::align_of::<*mut c_char>();
    let layout = Layout::from_size_align(arr.capacity, align).unwrap();

    unsafe { dealloc(arr.arr as *mut c_uchar, layout); }
}
*/


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