//! # FireRed Monitor Loop
//! 
//! This crate is the central coordinator for the FireRed tracker. It owns the
//! main polling loop that keeps [`FireRedState`] (current map group / name)
//! up to date, and exposes a set of public functions that the GUI and network
//! layers use to query party, box, and wild-encounter data.
//! 
//! ## Startup sequence
//! 
//! 1. [`start_loop`] (or its C-ABI wrapper [`c_start_loop`]) is called with a 
//!     path to the FireRed ROM and an `is_clean` flag.
//! 2. The ROM is loaded into the global buffer via `fill_rom`
//! 3. Wild-encounter headers are scanned form the ROM and cached.
//! 4. The pokemon name table is built and cached.
//! 5. Party and box monitors are started on thier own threads.
//! 6. A background thread is spawned that polls RetroArch every
//!     [`SLEEP_DURATION`] ms and  updates the shared [`FireRedState`]
//! 
//! ## Shutdown
//! 
//! Call [`stop_loop`] (or its C export) to signal the background thread to exit,
//! stop the party / box monitors, and join all threads.

use fire_red_get_values::*;
use fire_red_party_monitor::*;
use fire_red_retroarch_interfacing::*;
use fire_red_rom_buffer::*;
use std::ffi::{CStr, c_uchar};
use std::os::raw::c_char;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use fire_red_pokemon_data::*;
use fire_red_scanner::find_wild_headers;

// --------------------------------------------------------------------------------
// State
// --------------------------------------------------------------------------------

/// The minimal game state polled from RetroArch on every tick.
/// 
/// The two IDs together uniquely identify the map the player is currently on.
/// They are compared against the ROM's wild-encounter header table to determine
/// which pokemon can be encountered in the current area.
#[repr(C)]
#[derive(Default, Debug, Eq, PartialEq)]
pub struct FireRedState {
    /// Map group index (roughly corresponds to a town / route cluster)
    pub map_group_id: c_uchar,

    /// Map name index within the group.
    pub map_name_id: c_uchar,
}

// --------------------------------------------------------------------------------
// Global / thread state
// --------------------------------------------------------------------------------

/// Polling interval for the background map-state thread, in ms
const SLEEP_DURATION: u64 = 333;

/// Shared [`FireRedState`] updated by the background therad and read by the GUI.
/// Initialised once on the first call to [`start_loop`]
static STATE: OnceLock<Mutex<FireRedState>> = OnceLock::new();

/// Set to `true` while the background thread should keep running
/// Flipped to `false` by [`stop_loop`]
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Handle to the background map-polling thread, stored so [`stop_loop`] can join it
static THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

// --------------------------------------------------------------------------------
// C FFI entry points
// --------------------------------------------------------------------------------

/// C-ABI entry point for starting the monitor loop.
/// 
/// Converts the raw C string `file_path` to a Rust `&str` and delegates to 
/// [`start_loop`]. Intended for use from C/C++ hosts of Python via `ctypes`.
/// 
/// # Safety
/// `file_path` must be a valid, non-null, null-terminated UTF-8 C string for the
/// duration of this call.
/// 
/// # Returns
/// * `0` - success
/// * `1` - null or invalid UTF-8
/// * See [`start_loop`] for other error codes
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

// --------------------------------------------------------------------------------
// Core loop management
// --------------------------------------------------------------------------------

/// Initalizes all subsystems and starts the background map-polling thread.
/// 
/// Safe to call from Rust; see [`c_start_loop`] for the C-ABI wrapper.
/// 
/// Calling this while a loop is already running is a no-op that returns `-4`
/// 
/// # Arguments
/// * `file_path` - path to the firered `.gba` ROM file.
/// * `is_clean` - When `true`, enables ability display; only reliable on 
///                unmodified ("clean") ROMs.
/// 
/// # Returns
/// * `0`  - Loop started successfully
/// * `-1` - Empty file path.
/// * `-2` - ROM failed to load (I/O or format error)
/// * `-3` - Wild-encounter headers could not be located in the ROM
/// * `-4` - A loop is already running.
pub fn start_loop(file_path: &str, is_clean: bool) -> c_int {
    // Prevent multiple loops
    if RUNNING.swap(true, Ordering::SeqCst) {
        return -4;
    }

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

    // Scan the ROM for the start of the WildMonHeader table. This offset is
    // required for `fill_static_pokemon_header_list`
    println!("Scanning for WildMonHeaders...");
    let start_wild_header_offset = find_wild_headers(&get_rom()).unwrap_or_else(|| {
        eprintln!("Could not locate WildMonHeaders\nQuitting");
        0
    });
    if start_wild_header_offset == 0 {
        return -3;
    }
    println!(
        "Found WildMonHeaders at 0x{:08X}!",
        start_wild_header_offset
    );

    // Cache all static data that is read repeatedly at runtime.
    fill_static_pokemon_header_list(&get_rom(), start_wild_header_offset);
    fill_static_name_repo(&get_rom(), fire_red_text::POKEMON_NAMES_ADDR as usize);
    initialize_static_party(is_clean);
    fire_red_trainer_data::initialize_static_trainer_data();
    fire_red_party_monitor::start_loop();
    fire_red_box_monitor::start_loop();
    fire_red_trainer_data::start_loop();

    STATE.get_or_init(|| Mutex::new(FireRedState::default()));
    println!("spawning loop");

    // Background thread: polls the current map every SLEEP_DURATION ms and
    // writes the result into STATE
    let handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            let data = get_map_info();
            if data.is_none() {
                eprintln!("Failed to get map info from RetroArch, retrying...");
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
            let data = data.unwrap();
            let current_state = get_map_ground_and_id(&data);

            let mut state = STATE
                .get()
                .expect("STATE not initialized")
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.map_group_id = current_state.map_group_id;
            state.map_name_id = current_state.map_name_id;

            let _ = std::thread::sleep(std::time::Duration::from_millis(SLEEP_DURATION));
        }
    });

    *THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);

    0
}

/// Stops the monitor loop and joins all background threads.
/// 
/// Signals the map-polling thread to exit, shuts down the party and box
/// monitors, then blocks until the polling thread has finished. Safe to call
/// from C via the `#[no_mangle]` export.
#[unsafe(no_mangle)]
pub extern "C" fn stop_loop() {
    RUNNING.store(false, Ordering::SeqCst);
    fire_red_party_monitor::end_loop();
    fire_red_box_monitor::end_loop();
    fire_red_trainer_data::end_loop();
    let mut handle_slot = THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = handle_slot.take() {
        if let Err(e) = handle.join() {
            eprintln!("Error joining thread: {:?}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// State accessors
// ---------------------------------------------------------------------------

/// Returns a snapshot of the current [`FireRedState`] (map group + name IDs)
/// 
/// Panics if called before [`start_loop`] has initiated `STATE`
#[unsafe(no_mangle)]
pub fn get_value() -> FireRedState {
    let state = STATE.get().unwrap().lock().unwrap();
    FireRedState {
        map_group_id: state.map_group_id,
        map_name_id: state.map_name_id,
    }
}

// ---------------------------------------------------------------------------
// Trainer data accessors
// ---------------------------------------------------------------------------

pub fn get_trainer_name() -> String {
    fire_red_trainer_data::get_static_trainer_data().load().trainer_name_string.clone()
}

pub fn get_rival_name() -> String {
    fire_red_trainer_data::get_static_trainer_data().load().rival_name_string.clone()
}

pub fn get_play_time() -> String {
    let data = fire_red_trainer_data::get_static_trainer_data().load();
    format!("{}:{}:{}:{}", data.player_time_hours, data.player_time_minutes, data.player_time_seconds, data.player_time_v_blanks)
}

// ---------------------------------------------------------------------------
// Party / box accessors
// ---------------------------------------------------------------------------

/// Returns the number of pokemon currently in the player's party (0-6)
/// 
/// Returns `0` if party data is unavailable.
pub fn get_party_size() -> u8 {
    match get_party() {
        Some(party) => party.number_pokemon,
        None => 0,
    }
}

/// Returns a cloned `Vec` of all pokemon currently in the player's party.
/// 
/// Returns an empty `Vec` if party data is unavailable
pub fn get_party_members() -> Vec<Pokemon> {
    match get_party() {
        Some(party) => party.members.clone(),
        None => Vec::new(),
    }
}

/// Returns the pokemon at position `pos` in the player's party, or `None`
/// if the index is out of range or party data is unavailable.
/// 
/// # Arguments
/// * `pos` - Zero-based party slot index (0-5)
pub fn get_party_member(pos: usize) -> Option<Pokemon> {
    get_party()?.members.get(pos).cloned()
}

/// Returns all pokemon currently stored in teh PC box system
pub fn get_box_list() -> Vec<BoxPokemon> {
    fire_red_box_monitor::get_storage_entries()
}

/// Triggers a synchronous refresh of the PC box data from game memory.
/// 
/// Call this after a deposit, withdrawl, or other box-state change is detected.
pub fn update_box_list() {
    fire_red_box_monitor::update_box_list();
}

// ---------------------------------------------------------------------------
// Wild-encounter data
// ---------------------------------------------------------------------------

/// Returns a reference to the static list of all wild-encounter headers parsed
/// from the ROM at startup
fn get_wild_headers() -> &'static Vec<WildPokemonHeaderROM> {
    get_pokemon_header_list()
}

/// Returns the [`WildPokemonHeader`] for the player's current map, or a 
/// default (empty) header if no matching entry is found.
/// 
/// Looks up the current [`FireRedState`] and scans the cached header list for
/// a matching `(map_group, map_num)` pair. If multiple headers match (which should
/// not happen in a well-formed ROM), the last one wins.
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

/// Returns the wild-encounter pokemon for the current map as resolved name strings
/// 
/// Performs the same header lookup as [`get_area_pokemon_id`] but resolves each
/// species number to its display name via the cached name table. Entries whose
/// species number has no corresponding name are silently skipped.
/// 
/// Results are grouped by encounter type in the returned [`AreaEncountersStringVectors`]
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
                    Err(_) => {}
                };
            }
            let mons = wild_pokemon_header.water_mon_encounters.wild_pokemon_list;
            for mon in mons {
                let mon_name = fire_red_text::get_pokemon_name_by_number(mon.species as usize);
                match mon_name {
                    Ok(name) => area.water.push(name),
                    Err(_) => {}
                };
            }
            let mons = wild_pokemon_header.rock_smash_encounters.wild_pokemon_list;
            for mon in mons {
                let mon_name = fire_red_text::get_pokemon_name_by_number(mon.species as usize);
                match mon_name {
                    Ok(name) => area.rock.push(name),
                    Err(_) => {}
                };
            }
            let mons = wild_pokemon_header.fishing_encounters.wild_pokemon_list;
            for mon in mons {
                let mon_name = fire_red_text::get_pokemon_name_by_number(mon.species as usize);
                match mon_name {
                    Ok(name) => area.fishing.push(name),
                    Err(_) => {}
                };
            }
        }
    }

    area
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Builds the pokmeon name list from teh ROM and stores it in the global name buffer.
/// 
/// # Arguments
/// * `buffer` - Full ROM byte slice
/// * `offset` - Byte offset of the pokemon name table within the ROM
fn fill_static_name_repo(buffer: &[u8], offset: usize) {
    let names = fire_red_text::build_name_list(buffer, offset);
    fire_red_pokemon_name_buffer::fill_name_repo(names);
}

/// Extracts the current map group and name IDs from a raw RetroArch memory-read response.
/// 
/// Expects `buffer` to contain at least 4 tokens: index 0 is the command echo, index 1 is
/// the address, index 2 is the map group byte, index 3 is the map name byte. Returns a default
/// [`FireRedState`] if the buffer is too short.
/// 
/// # Arguments
/// * `buffer` - Tokenised response from a `READ_CORE_MEMORY` command
fn get_map_ground_and_id(buffer: &[String]) -> FireRedState {
    if buffer.len() < 4 {
        return FireRedState::default();
    }
    let map_group_id = get_u8(&[buffer[2].as_str()]);
    let map_name_id = get_u8(&[buffer[3].as_str()]);

    FireRedState {
        map_group_id,
        map_name_id,
    }
}

// ---------------------------------------------------------------------------
// Area encounter string types
// ---------------------------------------------------------------------------

/// C-ABI-compatible wrapper around a heap-allocated array of C strings.
/// 
/// Ownership follows the same rules as a `Vec`: the caller is responsible for
/// freeing the memory.

#[repr(C)]
#[derive(Default, Debug)]
pub struct StringArray {
    pub data: *mut *mut c_char,
    pub len: usize,
    pub cap: usize,
}

/// C-ABI-compatible grouping of [`StringArray`]s for each encounter type
/// 
/// Mirrors [`AreaEncountersStringVectors`] but uses raw C pointers so the data
/// can be passed across the FFI boundary without copying
#[repr(C)]
#[derive(Default, Debug)]
pub struct AreaEncountersStringArrays {
    pub land: StringArray,
    pub water: StringArray,
    pub rock_smash: StringArray,
    pub fishing: StringArray,
}

/// Resolved wild-encouter pokemon names for the current map, grouped by type.
/// 
/// Used internally by the Rust GUI layer; see [`AreaEncountersStringArrays`] for
/// the equivalent C-ABI type
#[derive(Default, Debug)]
pub struct AreaEncountersStringVectors {
    pub land: Vec<String>,
    pub water: Vec<String>,
    pub rock: Vec<String>,
    pub fishing: Vec<String>,
}

impl std::fmt::Display for AreaEncountersStringVectors {
    /// Formats the encounter lists for human-readable output, printing each
    /// non-empty category with a heading followed by one name per line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.land.is_empty() {
            writeln!(f, "grass encounters")?;
            for s in &self.land {
                writeln!(f, "{}", s)?;
            }
        }
        if !self.water.is_empty() {
            writeln!(f, "water encounters")?;
            for s in &self.water {
                writeln!(f, "{}", s)?;
            }
        }

        if !self.rock.is_empty() {
            writeln!(f, "rock smash encounters")?;
            for s in &self.rock {
                writeln!(f, "{}", s)?;
            }
        }

        if !self.fishing.is_empty() {
            writeln!(f, "fishing encounters")?;
            for s in &self.fishing {
                writeln!(f, "{}", s)?;
            }
        }

        Ok(())
    }
}