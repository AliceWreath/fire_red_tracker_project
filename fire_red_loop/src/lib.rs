use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::os::raw::c_char;
use std::ffi::{CStr, c_uchar};
use std::os::raw::c_int;
use fire_red_party_monitor::*;
use fire_red_retroarch_interfacing::*;
use fire_red_rom_buffer::*;
use fire_red_get_values::*;

use fire_red_scanner::find_wild_headers;
use fire_red_pokemon_data::*;

#[repr(C)]
#[derive(Default, Debug, Eq, PartialEq)]
pub struct FireRedState {
    pub map_group_id: c_uchar,
    pub map_name_id: c_uchar,
}

const SLEEP_DURATION: u64 = 333;

static STATE: OnceLock<Mutex<FireRedState>> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
pub extern "C" fn c_start_loop(file_path: *const c_char, is_clean: bool) -> c_int {
        if file_path.is_null() {
            eprintln!("Must pass a path to the file!");
            return -1;
        }
    
        let c_str = unsafe { CStr::from_ptr(file_path) };
        let file_path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("Invalid UTF-8 string for file path!");
                return -1;
            }
        };
    start_loop(&file_path_str, is_clean)
}


pub fn start_loop(file_path: &str, is_clean: bool) -> c_int {    
    if file_path.is_empty() {
        eprintln!("Must pass a path to the file!");
        return -1;
    }

    let rom_path = file_path.to_string();

    let result = fill_rom(&rom_path);
    if result.is_err() {
        eprintln!("{:?}", result.err());
        return -2;
    }

    println!("Scanning for WildMonHeaders...");
    let start_wild_header_offset = find_wild_headers(&get_rom()).unwrap_or_else(|| {
        eprintln!("Could not locate WildMonHeaders\nQuitting");
        return 0;
    });
    if start_wild_header_offset == 0 {
        return -3;
    }
    println!("Found WildMonHeaders at 0x{:08X}!", start_wild_header_offset);

    fill_static_pokemon_header_list(&get_rom(), start_wild_header_offset);
    fill_static_name_repo(&get_rom(), fire_red_text::POKEMON_NAMES_ADDR as usize);
    initialize_static_party(is_clean);
    fire_red_party_monitor::start_loop();
    fire_red_box_monitor::start_loop();
    println!("box updated");

    // Prevent multiple loops
    if RUNNING.swap(true, Ordering::SeqCst) {
        return -4;
    }

    STATE.get_or_init(|| Mutex::new(FireRedState::default()));
    println!("spawning loop");

    let _handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            let data = get_map_info();
            let current_state = get_map_ground_and_id(&data);
            //update_party();

            

            let mut state = STATE.get().unwrap().lock().unwrap();
            state.map_group_id = current_state.map_group_id;
            state.map_name_id = current_state.map_name_id;

            let _ = std::thread::sleep(std::time::Duration::from_millis(SLEEP_DURATION));
        }
    });

    0
}

fn fill_static_name_repo(buffer: &[u8], offset: usize) {
    let names = fire_red_text::build_name_list(buffer, offset);
    fire_red_pokemon_name_buffer::fill_name_repo(names);
}

pub fn get_party_size() -> u8 {
    get_party().number_pokemon
}

pub fn get_party_members() -> Vec<Pokemon> {
    get_party().members.clone()
}

pub fn get_party_member(pos: usize) -> Pokemon {
    get_party().members[pos].clone()
}

pub fn get_box_list() -> Vec<BoxPokemon> {
    fire_red_box_monitor::get_storage_entries()
}

pub fn update_box_list() {
    fire_red_box_monitor::update_box_list();
}
/*
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ByteBuffer {
    pub data: *mut u8,
    pub len: usize,
    pub cap: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn get_rom_buffer() -> ByteBuffer {
    let buffer_cpy = get_rom();

    let mut buffer_cpy: Box<[u8]> = Box::from(buffer_cpy);
    let ptr = buffer_cpy.as_mut_ptr();
    let len = buffer_cpy.len();
    let cap = buffer_cpy.len(); //will be the same as len because it is a boxed slice
    std::mem::forget(buffer_cpy);

    ByteBuffer { data: ptr, len, cap }
}

#[unsafe(no_mangle)]
pub extern "C" fn free_byte_buffer(buf: ByteBuffer) {
    if !buf.data.is_null() {
        unsafe {
            let _vec = Vec::from_raw_parts(buf.data, buf.len, buf.cap);
        }
    }
}
*/
#[unsafe(no_mangle)]
pub extern "C" fn stop_loop() {
    RUNNING.store(false, Ordering::SeqCst);
    fire_red_party_monitor::end_loop();
    fire_red_box_monitor::end_loop();
}

#[unsafe(no_mangle)]
pub fn get_value() -> FireRedState {
    let state = STATE.get().unwrap().lock().unwrap();
    FireRedState { 
        map_group_id: state.map_group_id, 
        map_name_id: state.map_name_id,
    }
}

fn get_wild_headers() -> &'static Vec<WildPokemonHeaderROM> {
    get_pokemon_header_list()
}

fn get_map_ground_and_id(buffer: &[String]) -> FireRedState {
    let map_group_id = get_u8(&[buffer[2].as_str()]);
    let map_name_id = get_u8(&[buffer[3].as_str()]);
    
    FireRedState {
        map_group_id,
        map_name_id
    }
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct StringArray {
    pub data: *mut *mut c_char,
    pub len: usize,
    pub cap: usize,
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct AreaEncountersStringArrays {
    pub land: StringArray,
    pub water: StringArray,
    pub rock_smash: StringArray,
    pub fishing: StringArray,
}

#[derive(Default, Debug)]
pub struct AreaEncountersStringVectors {
    pub land: Vec<String>,
    pub water: Vec<String>,
    pub rock: Vec<String>,
    pub fishing: Vec<String>,
}

impl std::fmt::Display for AreaEncountersStringVectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut string = String::new();

        if self.land.len() > 0 {
            string = string + "grass encounters\n";
            for s in &self.land {
                string = string + &s + "\n";
            }
            string.truncate(string.len() - 1);
        }
        if self.water.len() > 0 {
            string = string + "\n\nwater encounters\n";

            for s in &self.water {
                string = string + &s + "\n";
            }
            string.truncate(string.len() - 1);
        }

        if self.rock.len() > 0 {
            string = string + "\n\nrock smash encounters\n";
            for s in &self.rock {
                string = string + &s + "\n";
            }
            string.truncate(string.len() - 1);
        }
        
        if self.fishing.len() > 0 {
            string = string + "\n\nfishing encounters\n";
            for s in &self.fishing {
                string = string + &s + "\n";
            }
            string.truncate(string.len() - 1);
        }


        write!(f, "{}", string)
    }
}

/*
pub unsafe fn string_array_to_vec(arr: StringArray) -> Vec<String> {
    if arr.data.is_null() || arr.len == 0 {
        return Vec::new();
    }

    let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };

    let mut result = Vec::with_capacity(arr.len);

    for &ptr in slice {
        if ptr.is_null() {
            continue;
        }

        let cstring = unsafe { std::ffi::CString::from_raw(ptr) };

        match cstring.into_string() {
            Ok(s) => result.push(s),
            Err(e) => result.push(e.into_cstring().to_string_lossy().into_owned()),
        }
    }

    // this will free the memory
    let _ = unsafe { Vec::from_raw_parts(arr.data, arr.len, arr.cap) };

    result
}*/

/*
pub unsafe fn convert_area(area: AreaEncountersStringArrays) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    unsafe {
    (
        string_array_to_vec(area.land),
        string_array_to_vec(area.water),
        string_array_to_vec(area.rock_smash),
        string_array_to_vec(area.fishing),
    )
    }
}*/


pub fn get_area_pokemon_id() -> WildPokemonHeader {
    let state = get_value();
    let mut area_header = WildPokemonHeader::default();

    for header in get_wild_headers() {
        if header.map_group == state.map_group_id && header.map_num == state.map_name_id {
            area_header = WildPokemonHeader::fill_head(&header, &get_rom());
        }
    }

    area_header
}

pub fn get_area_pokemon_strings() -> AreaEncountersStringVectors {
    let mut area = AreaEncountersStringVectors::default();
    let state = get_value();

    for header in get_wild_headers() {
        if header.map_group == state.map_group_id && header.map_num == state.map_name_id {
            let wild_pokemon_header = WildPokemonHeader::fill_head(&header, &get_rom());

            let mons = wild_pokemon_header.land_mon_encounters.wild_pokemon_list;
            for mon in mons {
                let mon_name = fire_red_text::get_pokemon_name_by_number(mon.species as usize);
                match mon_name {
                    Ok(name) => area.land.push(name),
                    Err(_) => {},
                };
            }
            let mons = wild_pokemon_header.water_mon_encounters.wild_pokemon_list;
            for mon in mons {
                let mon_name = fire_red_text::get_pokemon_name_by_number(mon.species as usize);
                match mon_name {
                    Ok(name) => area.water.push(name),
                    Err(_) => {},
                };
            }
            let mons = wild_pokemon_header.rock_smash_encounters.wild_pokemon_list;
            for mon in mons {
                let mon_name = fire_red_text::get_pokemon_name_by_number(mon.species as usize);
                match mon_name {
                    Ok(name) => area.rock.push(name),
                    Err(_) => {},
                };
            }
            let mons = wild_pokemon_header.fishing_encounters.wild_pokemon_list;
            for mon in mons {
                let mon_name = fire_red_text::get_pokemon_name_by_number(mon.species as usize);
                match mon_name {
                    Ok(name) => area.fishing.push(name),
                    Err(_) => {},
                };
            }
        }
    }

    area
}