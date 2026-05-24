//! # FireRed Tracker Loop
//!
//! Central coordinator for the FireRed tracker. Owns the main polling loop
//! that keeps [`FireRedState`] (current map group / name IDs) up to date, and
//! exposes public functions that the GUI and network layers use to query party,
//! box, trainer, and wild-encounter data.
//!
//! # Startup sequence
//!
//! 1. [`start_loop`] (or its C-ABI wrapper [`c_start_loop`]) is called with a
//!    path to the FireRed ROM and an `is_clean` flag.
//! 2. The ROM is loaded into the global buffer via `fill_rom`.
//! 3. Wild-encounter headers are scanned from the ROM and cached.
//! 4. The pokemon name table is built and cached.
//! 5. `fire_red_memory::start_loop()` is started **first** so that the EWRAM
//!    and IWRAM snapshots are available before any subsystem tries to read them.
//! 6. A short sleep gives the memory loop one full poll cycle to populate the
//!    buffers before dependent subsystems initialize from them.
//! 7. Party, box, and trainer monitors are started on their own threads.
//! 8. A background thread is spawned that reads the current map state from the
//!    EWRAM snapshot every [`SLEEP_DURATION`] ms and updates [`STATE`].
//!
//! # Shutdown
//!
//! Call [`stop_loop`] (or its C export) to signal all background threads to
//! exit and join the map-polling thread.
//!
//! # Data sources
//!
//! All live game data — party, trainer info, badges, box contents, and map
//! state — is read from the EWRAM/IWRAM snapshots maintained by
//! `fire_red_memory`. No subsystem in this crate issues UDP calls directly.

use fire_red_party_monitor::*;
use fire_red_rom_buffer::*;
use std::ffi::{CStr, c_uchar};
use std::os::raw::c_char;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use fire_red_pokemon_data::*;
use fire_red_scanner::find_wild_headers;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The minimal game state derived from the EWRAM snapshot on every tick.
///
/// The two IDs together uniquely identify the map the player is on. They are
/// compared against the ROM's wild-encounter header table to determine which
/// pokemon can be encountered in the current area.
#[repr(C)]
#[derive(Default, Debug, Eq, PartialEq, Clone, Copy)]
pub struct FireRedState {
    /// Map group index (roughly corresponds to a town / route cluster).
    pub map_group_id: c_uchar,

    /// Map name index within the group.
    pub map_name_id: c_uchar,
}

// ---------------------------------------------------------------------------
// Globals
// ---------------------------------------------------------------------------

/// How often the background thread reads map state from the EWRAM snapshot.
const SLEEP_DURATION: u64 = 333;

/// Base address of EWRAM in the GBA address space.
const EWRAM_BASE: usize = 0x02000000;

/// GBA address of the two-byte packed (map_group, map_name) field.
const MAP_GROUP_AND_NAME_ADDR: usize = 0x02031DBC;

/// Shared [`FireRedState`] updated by the background thread and read by callers.
/// Initialized once on the first call to [`start_loop`].
static STATE: OnceLock<Mutex<FireRedState>> = OnceLock::new();

/// `true` while the background thread should keep running.
/// Flipped to `false` by [`stop_loop`].
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Handle to the background map-polling thread so [`stop_loop`] can join it.
static THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// C FFI entry points
// ---------------------------------------------------------------------------

/// C-ABI entry point for starting the monitor loop.
///
/// Converts the raw C string `file_path` to a Rust `&str` and delegates to
/// [`start_loop`]. Intended for use from C/C++ hosts or Python via `ctypes`.
///
/// # Safety
///
/// `file_path` must be a valid, non-null, null-terminated UTF-8 C string for
/// the duration of this call.
///
/// # Returns
///
/// * `0`  — success
/// * `-1` — null pointer or invalid UTF-8
/// * See [`start_loop`] for other error codes.
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
    start_loop(file_path_str, is_clean)
}

// ---------------------------------------------------------------------------
// Core loop management
// ---------------------------------------------------------------------------

/// Initializes all subsystems and starts the background map-polling thread.
///
/// Safe to call from Rust; see [`c_start_loop`] for the C-ABI wrapper.
/// Calling this while a loop is already running is a no-op that returns `-4`.
///
/// # Arguments
///
/// * `file_path` — Path to the FireRed `.gba` ROM file.
/// * `is_clean`  — When `true`, enables ability name display. Only reliable
///                 on unmodified ("clean") ROMs.
///
/// # Returns
///
/// * `0`  — Loop started successfully.
/// * `-1` — Empty file path.
/// * `-2` — ROM failed to load (I/O or format error).
/// * `-3` — Wild-encounter headers could not be located in the ROM.
/// * `-4` — A loop is already running.
pub fn start_loop(file_path: &str, is_clean: bool) -> c_int {
    // Prevent multiple concurrent loops.
    if RUNNING.swap(true, Ordering::SeqCst) {
        return -4;
    }

    if file_path.is_empty() {
        eprintln!("Must pass a path to the file!");
        RUNNING.store(false, Ordering::SeqCst);
        return -1;
    }

    // Load the ROM into the global buffer — everything else depends on this.
    if let Err(e) = fill_rom(file_path) {
        eprintln!("Failed to load ROM: {:?}", e);
        RUNNING.store(false, Ordering::SeqCst);
        return -2;
    }

    // Scan the ROM for the WildMonHeader table offset required by
    // `fill_static_pokemon_header_list`.
    println!("Scanning for WildMonHeaders...");
    let start_wild_header_offset = match find_wild_headers(get_rom()) {
        Some(offset) => {
            println!("Found WildMonHeaders at 0x{:08X}!", offset);
            offset
        }
        None => {
            eprintln!("Could not locate WildMonHeaders — aborting.");
            RUNNING.store(false, Ordering::SeqCst);
            return -3;
        }
    };

    // Build all ROM-derived caches that subsystems read at runtime.
    fill_static_pokemon_header_list(get_rom(), start_wild_header_offset);
    fill_static_name_repo(get_rom(), fire_red_text::POKEMON_NAMES_ADDR as usize);

    // Start the EWRAM/IWRAM snapshot loop FIRST so that the buffers are
    // populated before the party, trainer, and box monitors try to read them.
    fire_red_memory::start_loop();

    // Give the memory loop one full poll cycle (~500 ms) to populate the
    // buffers before subsystems initialize their statics from them.
    std::thread::sleep(std::time::Duration::from_millis(600));

    // Initialize subsystems that read from the EWRAM snapshot.
    initialize_static_party(is_clean);
    fire_red_trainer_data::initialize_static_trainer_data();

    // Start the per-subsystem polling threads.
    fire_red_party_monitor::start_loop();
    fire_red_box_monitor::start_loop();
    fire_red_trainer_data::start_loop();

    STATE.get_or_init(|| Mutex::new(FireRedState::default()));
    println!("Spawning map-polling loop...");

    // Background thread: reads map state from the EWRAM snapshot every
    // SLEEP_DURATION ms and writes the result into STATE. No UDP calls.
    let handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            let current_state = get_map_state_from_ewram();
            let mut state = STATE
                .get()
                .expect("STATE not initialized")
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            state.map_group_id = current_state.map_group_id;
            state.map_name_id  = current_state.map_name_id;
            std::thread::sleep(std::time::Duration::from_millis(SLEEP_DURATION));
        }
    });

    *THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);

    0
}

/// Stops the monitor loop and joins all background threads.
///
/// Signals all background threads to exit, then blocks until the map-polling
/// thread has finished. Safe to call from C via the `#[no_mangle]` export.
#[unsafe(no_mangle)]
pub extern "C" fn stop_loop() {
    RUNNING.store(false, Ordering::SeqCst);

    // Stop subsystem loops before joining the main thread so they don't try
    // to read from a snapshot that is about to stop updating.
    fire_red_party_monitor::end_loop();
    fire_red_box_monitor::end_loop();
    fire_red_trainer_data::end_loop();
    fire_red_memory::end_loop();

    let mut handle_slot = THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = handle_slot.take() {
        if let Err(e) = handle.join() {
            eprintln!("Error joining map-polling thread: {:?}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// State accessors
// ---------------------------------------------------------------------------

/// Returns a snapshot of the current [`FireRedState`] (map group + name IDs).
///
/// # Panics
///
/// Panics if called before [`start_loop`] has initialized `STATE`.
#[unsafe(no_mangle)]
pub fn get_value() -> FireRedState {
    let state = STATE
        .get()
        .expect("get_value called before start_loop")
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    FireRedState {
        map_group_id: state.map_group_id,
        map_name_id:  state.map_name_id,
    }
}

/// Returns the current badge state, or `None` if unavailable.
pub fn get_badge_state() -> Option<fire_red_badge::BadgeState> {
    fire_red_badge::read_badge_state()
}

// ---------------------------------------------------------------------------
// Trainer data accessors
// ---------------------------------------------------------------------------

/// Returns the player's trainer name as a `String`.
pub fn get_trainer_name() -> String {
    fire_red_trainer_data::get_static_trainer_data()
        .load()
        .trainer_name_string
        .clone()
}

/// Returns the rival's name as a `String`.
pub fn get_rival_name() -> String {
    fire_red_trainer_data::get_static_trainer_data()
        .load()
        .rival_name_string
        .clone()
}

/// Returns the current play time formatted as `"H:M:S:frames"`.
pub fn get_play_time() -> String {
    let data = fire_red_trainer_data::get_static_trainer_data().load();
    format!(
        "{}:{}:{}:{}",
        data.player_time_hours,
        data.player_time_minutes,
        data.player_time_seconds,
        data.player_time_v_blanks,
    )
}

// ---------------------------------------------------------------------------
// Party / box accessors
// ---------------------------------------------------------------------------

/// Returns the number of pokemon currently in the player's party (0–6).
///
/// Returns `0` if party data is unavailable.
pub fn get_party_size() -> u8 {
    get_party().map(|p| p.number_pokemon).unwrap_or(0)
}

/// Returns a cloned `Vec` of all pokemon currently in the player's party.
///
/// Returns an empty `Vec` if party data is unavailable.
pub fn get_party_members() -> Vec<Pokemon> {
    get_party().map(|p| p.members.clone()).unwrap_or_default()
}

/// Returns the pokemon at party slot `pos`, or `None` if out of range or
/// party data is unavailable.
///
/// # Arguments
///
/// * `pos` — Zero-based party slot index (0–5).
pub fn get_party_member(pos: usize) -> Option<Pokemon> {
    get_party()?.members.get(pos).cloned()
}

/// Returns all pokemon currently stored in the PC box system.
pub fn get_box_list() -> Vec<BoxPokemon> {
    fire_red_box_monitor::get_storage_entries()
}

/// Triggers a synchronous refresh of the PC box data from the EWRAM snapshot.
///
/// Call this after a deposit, withdrawal, or other box-state change is detected.
pub fn update_box_list() {
    fire_red_box_monitor::update_box_list();
}

// ---------------------------------------------------------------------------
// Wild-encounter data
// ---------------------------------------------------------------------------

/// Returns a reference to the static list of all wild-encounter headers parsed
/// from the ROM at startup.
fn get_wild_headers() -> &'static Vec<WildPokemonHeaderROM> {
    get_pokemon_header_list()
}

/// Returns the [`WildPokemonHeader`] for the player's current map.
///
/// Returns a default (empty) header if no matching entry is found. If multiple
/// headers match a given `(map_group, map_num)` pair (which should not occur in
/// a well-formed ROM), the last match wins.
pub fn get_area_pokemon_id() -> WildPokemonHeader {
    let state = get_value();
    let mut area_header = WildPokemonHeader::default();
    for header in get_wild_headers() {
        if header.map_group == state.map_group_id && header.map_num == state.map_name_id {
            area_header = WildPokemonHeader::fill_head(header, get_rom());
        }
    }
    area_header
}

/// Like [`get_area_pokemon_id`] but uses the provided state instead of
/// reading from [`STATE`]. Used for the initial encounter load before the
/// map-polling thread has ticked.
pub fn get_area_pokemon_id_for_state(state: &FireRedState) -> WildPokemonHeader {
    let mut area_header = WildPokemonHeader::default();
    for header in get_wild_headers() {
        if header.map_group == state.map_group_id && header.map_num == state.map_name_id {
            area_header = WildPokemonHeader::fill_head(header, &get_rom());
        }
    }
    area_header
}

/// Returns the wild-encounter pokemon for the current map as resolved name strings.
///
/// Performs the same header lookup as [`get_area_pokemon_id`] but resolves each
/// species number to its display name via the cached name table. Species with no
/// corresponding name entry are silently skipped.
///
/// Results are grouped by encounter type in the returned [`AreaEncountersStringVectors`].
pub fn get_area_pokemon_strings() -> AreaEncountersStringVectors {
    let mut area = AreaEncountersStringVectors::default();
    let state = get_value();

    for header in get_wild_headers() {
        if header.map_group != state.map_group_id || header.map_num != state.map_name_id {
            continue;
        }

        let wild_header = WildPokemonHeader::fill_head(header, get_rom());

        let push_names = |list: &mut Vec<String>, encounters: &[WildPokemon]| {
            for mon in encounters {
                if let Ok(name) = fire_red_text::get_pokemon_name_by_number(mon.species as usize) {
                    list.push(name);
                }
            }
        };

        push_names(&mut area.land,    &wild_header.land_mon_encounters.wild_pokemon_list);
        push_names(&mut area.water,   &wild_header.water_mon_encounters.wild_pokemon_list);
        push_names(&mut area.rock,    &wild_header.rock_smash_encounters.wild_pokemon_list);
        push_names(&mut area.fishing, &wild_header.fishing_encounters.wild_pokemon_list);
    }

    area
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Builds the pokemon name list from the ROM and stores it in the global name buffer.
fn fill_static_name_repo(buffer: &[u8], offset: usize) {
    let names = fire_red_text::build_name_list(buffer, offset);
    fire_red_pokemon_name_buffer::fill_name_repo(names);
}

/// Reads the current map group and name IDs directly from the EWRAM snapshot.
///
/// Returns a default [`FireRedState`] if the snapshot is not yet populated or
/// the address is out of range.
fn get_map_state_from_ewram() -> FireRedState {
    let ewram = fire_red_memory::get_ewram();
    let offset = MAP_GROUP_AND_NAME_ADDR - EWRAM_BASE;

    if ewram.len() < offset + 2 {
        eprintln!("EWRAM too small: {} bytes, need {}", ewram.len(), offset + 2);
        return FireRedState::default();
    }

    let state = FireRedState {
        map_group_id: ewram[offset],
        map_name_id:  ewram[offset + 1],
    };
    state
}

// ---------------------------------------------------------------------------
// Area encounter string types
// ---------------------------------------------------------------------------

/// C-ABI-compatible wrapper around a heap-allocated array of C strings.
///
/// Ownership follows `Vec` semantics: the caller is responsible for freeing
/// the memory when it is no longer needed.
#[repr(C)]
#[derive(Default, Debug)]
pub struct StringArray {
    pub data: *mut *mut c_char,
    pub len: usize,
    pub cap: usize,
}

/// C-ABI-compatible grouping of [`StringArray`]s for each encounter type.
///
/// Mirrors [`AreaEncountersStringVectors`] but uses raw C pointers so the data
/// can cross the FFI boundary without copying.
#[repr(C)]
#[derive(Default, Debug)]
pub struct AreaEncountersStringArrays {
    pub land: StringArray,
    pub water: StringArray,
    pub rock_smash: StringArray,
    pub fishing: StringArray,
}

/// Resolved wild-encounter pokemon names for the current map, grouped by type.
///
/// Used internally by the Rust GUI layer; see [`AreaEncountersStringArrays`]
/// for the equivalent C-ABI type.
#[derive(Default, Debug)]
pub struct AreaEncountersStringVectors {
    pub land: Vec<String>,
    pub water: Vec<String>,
    pub rock: Vec<String>,
    pub fishing: Vec<String>,
}

impl std::fmt::Display for AreaEncountersStringVectors {
    /// Formats encounter lists for human-readable output, printing each
    /// non-empty category with a heading followed by one name per line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sections = [
            ("grass encounters",      &self.land),
            ("water encounters",      &self.water),
            ("rock smash encounters", &self.rock),
            ("fishing encounters",    &self.fishing),
        ];
        for (heading, list) in &sections {
            if !list.is_empty() {
                writeln!(f, "{}", heading)?;
                for name in list.iter() {
                    writeln!(f, "{}", name)?;
                }
            }
        }
        Ok(())
    }
}