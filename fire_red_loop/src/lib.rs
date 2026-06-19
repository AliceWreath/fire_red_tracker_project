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

use fire_red_get_values::read_u32;
use fire_red_map_data::{CurrentMapGroupAndName, MapHeader};
use fire_red_party_monitor::*;
use fire_red_pokemon_data::*;
use fire_red_rom_buffer::*;
use fire_red_scanner::{find_map_groups_table, find_wild_headers};
use fire_red_states::LockOrRecover;
use std::ffi::{CStr, CString, c_uchar};
use std::os::raw::c_char;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

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

/// ROM byte offset of `gMapGroupsAndMaps`, located by [`find_map_groups_table`]
/// at startup. `None` (unset) if the scan failed; callers fall back to the
/// hardcoded lookup table in that case.
static MAP_GROUPS_TABLE: OnceLock<usize> = OnceLock::new();

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
pub unsafe extern "C" fn c_start_loop(file_path: *const c_char, is_clean: bool) -> c_int {
    if file_path.is_null() {
        tracing::error!("Must pass a path to the file!");
        return -1;
    }
    let c_str = unsafe { CStr::from_ptr(file_path) };
    let file_path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("Invalid UTF-8 string for file path!");
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
/// * `file_path` — Path to the FireRed `.gba` ROM file. Point this at the
///   actual ROM you are playing — including a randomized ROM — so that
///   ability lookups read from the correct base stats table.
/// * `_is_clean` — Kept for C-ABI and config compatibility; no longer used
///   internally. Ability names are always resolved from the ROM.
///
/// # Returns
///
/// * `0`  — Loop started successfully.
/// * `-1` — Empty file path.
/// * `-2` — ROM failed to load (I/O or format error).
/// * `-3` — Wild-encounter headers could not be located in the ROM.
/// * `-4` — A loop is already running.
pub fn start_loop(file_path: &str, _is_clean: bool) -> c_int {
    // Prevent multiple concurrent loops.
    if RUNNING.swap(true, Ordering::SeqCst) {
        return -4;
    }

    if file_path.is_empty() {
        tracing::error!("Must pass a path to the file!");
        RUNNING.store(false, Ordering::SeqCst);
        return -1;
    }

    // Load the ROM into the global buffer — everything else depends on this.
    if let Err(e) = fill_rom(file_path) {
        tracing::error!("Failed to load ROM: {:?}", e);
        RUNNING.store(false, Ordering::SeqCst);
        return -2;
    }

    // Scan the ROM for the WildMonHeader table offset required by
    // `fill_static_pokemon_header_list`.
    tracing::info!("Scanning for WildMonHeaders...");
    let start_wild_header_offset = match find_wild_headers(get_rom()) {
        Some(offset) => {
            tracing::info!("Found WildMonHeaders at 0x{:08X}!", offset);
            offset
        }
        None => {
            tracing::error!("Could not locate WildMonHeaders — aborting.");
            RUNNING.store(false, Ordering::SeqCst);
            return -3;
        }
    };

    // Log the detected ROM revision so the user can confirm they loaded the
    // right ROM.  Detection happens inside fill_rom via fill_static_buffer.
    tracing::info!(
        "ROM revision: {:?}",
        fire_red_rom_buffer::get_rom_revision()
    );

    // Build all ROM-derived caches that subsystems read at runtime.
    fill_static_pokemon_header_list(get_rom(), start_wild_header_offset);
    fill_static_name_repo(
        get_rom(),
        fire_red_rom_buffer::get_rom_addresses().pokemon_names_addr,
    );

    // Locate gMapGroupsAndMaps using one pair per distinct group from the wild
    // encounter headers. This is non-fatal: zone names fall back to the
    // hardcoded lookup table if the scan fails.
    tracing::info!("Scanning for gMapGroupsAndMaps...");
    {
        // Take up to 20 pairs from across the encounter list. More pairs means
        // stronger validation; group diversity is not required — the 3-level
        // pointer chain check is discriminating enough on its own.
        let known_pairs: Vec<(u8, u8)> = get_pokemon_header_list()
            .iter()
            .take(20)
            .map(|h| (h.map_group, h.map_num))
            .collect();

        match find_map_groups_table(get_rom(), &known_pairs) {
            Some(offset) => {
                tracing::info!("Found gMapGroupsAndMaps at ROM offset 0x{:08X}", offset);
                MAP_GROUPS_TABLE.get_or_init(|| offset);
            }
            None => {
                tracing::warn!("gMapGroupsAndMaps not found — zone names will use fallback");
            }
        }
    }

    // Start the EWRAM/IWRAM snapshot loop FIRST so that the buffers are
    // populated before the party, trainer, and box monitors try to read them.
    fire_red_memory::start_loop();

    // Give the memory loop one full poll cycle (~500 ms) to populate the
    // buffers before subsystems initialize their statics from them.
    std::thread::sleep(std::time::Duration::from_millis(600));

    // Initialize subsystems that read from the EWRAM snapshot.
    initialize_static_party();
    fire_red_trainer_data::initialize_static_trainer_data();

    // Start the per-subsystem polling threads.
    fire_red_party_monitor::start_loop();
    fire_red_box_monitor::start_loop();
    fire_red_trainer_data::start_loop();

    STATE.get_or_init(|| Mutex::new(FireRedState::default()));
    tracing::info!("Spawning map-polling loop...");

    // Background thread: reads map state from the EWRAM snapshot every
    // SLEEP_DURATION ms and writes the result into STATE. No UDP calls.
    let handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            let current_state = get_map_state_from_ewram();
            {
                // Scope the guard so it is dropped before the sleep.
                // Previously the guard was held for the full SLEEP_DURATION,
                // which caused get_value() callers to block for up to 333 ms.
                let mut state = STATE
                    .get()
                    .expect("STATE not initialized")
                    .lock_or_recover();
                state.map_group_id = current_state.map_group_id;
                state.map_name_id = current_state.map_name_id;
            } // MutexGuard dropped here
            std::thread::sleep(std::time::Duration::from_millis(SLEEP_DURATION));
        }
    });

    *THREAD_HANDLE.lock_or_recover() = Some(handle);

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

    let mut handle_slot = THREAD_HANDLE.lock_or_recover();
    if let Some(handle) = handle_slot.take()
        && let Err(e) = handle.join()
    {
        tracing::error!("Error joining map-polling thread: {:?}", e);
    }
}

// ---------------------------------------------------------------------------
// Per-connection loop management
// ---------------------------------------------------------------------------

/// Initializes all ROM-derived caches and starts per-connection subsystem
/// threads.
///
/// Unlike [`start_loop`], this function does **not** check the global `RUNNING`
/// flag, so it can be called concurrently for multiple RetroArch connections.
/// Each connection supplies its own context objects; the global singleton
/// storage is never written.
///
/// ROM loading and ROM scanning are protected by [`OnceLock`] internally, so
/// calling this from multiple threads simultaneously is safe — only the first
/// caller performs the I/O; subsequent callers return immediately.
///
/// # Arguments
///
/// * `file_path`   — Path to the FireRed ROM.
/// * `_is_clean`   — Kept for call-site compatibility; not used.
/// * `mem_ctx`     — Per-connection EWRAM/IWRAM snapshot context.
/// * `party_ctx`   — Per-connection party data context.
/// * `trainer_ctx` — Per-connection trainer/player data context.
///
/// # Returns
///
/// `Ok(box_running)` where `box_running` is the shutdown flag for the
/// per-connection box monitor thread.  Store `false` into it (via the
/// returned `Arc<AtomicBool>`) to stop that thread when disconnecting.
///
/// `Err(code)` on failure:
/// * `-1` — Empty file path.
/// * `-2` — ROM failed to load.
/// * `-3` — Wild-encounter headers could not be located.
pub fn start_loop_ctx(
    file_path: &str,
    _is_clean: bool,
    mem_ctx: Arc<fire_red_memory::MemoryContext>,
    party_ctx: Arc<fire_red_party_monitor::PartyContext>,
    trainer_ctx: Arc<fire_red_trainer_data::TrainerContext>,
) -> Result<Arc<AtomicBool>, i32> {
    if file_path.is_empty() {
        tracing::error!("Must pass a path to the file!");
        return Err(-1);
    }

    if let Err(e) = fill_rom(file_path) {
        tracing::error!("Failed to load ROM: {:?}", e);
        return Err(-2);
    }

    tracing::info!("Scanning for WildMonHeaders...");
    let start_wild_header_offset = match find_wild_headers(get_rom()) {
        Some(offset) => {
            tracing::info!("Found WildMonHeaders at 0x{:08X}!", offset);
            offset
        }
        None => {
            tracing::error!("Could not locate WildMonHeaders — aborting.");
            return Err(-3);
        }
    };

    tracing::info!(
        "ROM revision: {:?}",
        fire_red_rom_buffer::get_rom_revision()
    );

    fill_static_pokemon_header_list(get_rom(), start_wild_header_offset);
    fill_static_name_repo(
        get_rom(),
        fire_red_rom_buffer::get_rom_addresses().pokemon_names_addr,
    );

    {
        let known_pairs: Vec<(u8, u8)> = get_pokemon_header_list()
            .iter()
            .take(20)
            .map(|h| (h.map_group, h.map_num))
            .collect();
        match find_map_groups_table(get_rom(), &known_pairs) {
            Some(offset) => {
                tracing::info!("Found gMapGroupsAndMaps at ROM offset 0x{:08X}", offset);
                MAP_GROUPS_TABLE.get_or_init(|| offset);
            }
            None => {
                tracing::warn!("gMapGroupsAndMaps not found — zone names will use fallback");
            }
        }
    }

    // Start the per-connection memory polling thread.
    fire_red_memory::start_loop_ctx(mem_ctx.clone());

    // Wait for the first EWRAM poll to complete.
    std::thread::sleep(std::time::Duration::from_millis(600));

    // Initialize per-connection party and trainer data from the first snapshot.
    fire_red_party_monitor::update_party_ctx(&party_ctx);
    fire_red_trainer_data::update_trainer_data_ctx(&trainer_ctx);

    // Initialize trainer static data using the per-connection EWRAM.
    fire_red_trainer_data::initialize_static_trainer_data();

    // Start per-connection monitor threads.
    fire_red_party_monitor::start_loop_ctx(mem_ctx.clone(), party_ctx);
    let box_running = fire_red_box_monitor::start_loop_ctx(mem_ctx.clone());
    fire_red_trainer_data::start_loop_ctx(mem_ctx, trainer_ctx);

    Ok(box_running)
}

/// Stops all subsystem threads started by [`start_loop_ctx`].
///
/// Signals the memory, party, trainer, and box monitor threads for this
/// connection to exit.  Does not join the threads — they will exit after their
/// current poll cycle completes.
pub fn stop_loop_ctx(
    mem_ctx: &fire_red_memory::MemoryContext,
    party_ctx: &fire_red_party_monitor::PartyContext,
    trainer_ctx: &fire_red_trainer_data::TrainerContext,
    box_running: &AtomicBool,
) {
    fire_red_memory::end_loop_ctx(mem_ctx);
    fire_red_party_monitor::end_loop_ctx(party_ctx);
    fire_red_trainer_data::end_loop_ctx(trainer_ctx);
    box_running.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// State accessors
// ---------------------------------------------------------------------------

/// Returns a snapshot of the current [`FireRedState`] (map group + name IDs).
///
/// Returns `FireRedState::default()` (both IDs zero) if called before
/// [`start_loop`] has initialized `STATE`. This is safe — callers that read
/// the position before the game loop is ready simply see `(0, 0)`.
#[unsafe(no_mangle)]
pub fn get_value() -> FireRedState {
    let Some(mutex) = STATE.get() else {
        return FireRedState::default();
    };
    let state = mutex.lock_or_recover();
    FireRedState {
        map_group_id: state.map_group_id,
        map_name_id: state.map_name_id,
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

/// Returns the raw play-time components `(hours, minutes, seconds)` from the
/// current save-file clock. Returns `(0, 0, 0)` if trainer data is not yet loaded.
pub fn get_play_time_components() -> (u16, u8, u8) {
    let data = fire_red_trainer_data::get_static_trainer_data().load();
    (data.player_time_hours, data.player_time_minutes, data.player_time_seconds)
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

/// Reads the [`MapHeader`] for a given `(group, map)` pair directly from the
/// ROM via the `gMapGroupsAndMaps` pointer table.
///
/// Returns `None` if the map groups table was not found at startup, if either
/// pointer in the chain falls outside the ROM, or if the ROM buffer is too
/// short to hold the header.
pub fn get_map_header_from_rom(group: u8, map: u8) -> Option<MapHeader> {
    let table_offset = MAP_GROUPS_TABLE.get().copied()?;
    let rom = get_rom();

    // Follow table[group] → ROM pointer to the group's map-header array.
    let group_ptr = read_u32(rom, table_offset + group as usize * 4);
    if !(0x08000000..=0x09FFFFFF).contains(&group_ptr) {
        return None;
    }
    let group_offset = (group_ptr - 0x08000000) as usize;

    // Follow group_array[map] → ROM pointer to the MapHeader.
    let map_ptr = read_u32(rom, group_offset + map as usize * 4);
    if !(0x08000000..=0x09FFFFFF).contains(&map_ptr) {
        return None;
    }
    let map_offset = (map_ptr - 0x08000000) as usize;

    if map_offset + 28 > rom.len() {
        return None;
    }
    Some(MapHeader::fill_from_bytes(rom, map_offset))
}

/// Returns the human-readable zone name for a given `(group, map)` pair,
/// using the ROM's `MapHeader.name_index` (MAPSEC) when available.
///
/// Falls back to the hardcoded `map_area_name` lookup if the ROM table has
/// not been initialized or the pointer chain is invalid for the given pair.
pub fn get_area_name_for(group: u8, map: u8) -> &'static str {
    if let Some(header) = get_map_header_from_rom(group, map) {
        let name = fire_red_location_names::location_name(header.name_index);
        // Accept any ROM-derived name except the two sentinel values:
        // "—" = MAPSEC_NONE (interior with no banner)
        // "Unknown Location" = MAPSEC value not in our table
        // In both cases fall through to the hardcoded lookup or the
        // caller's formatted fallback.
        if name != "—" && name != "Unknown Location" {
            return name;
        }
    }
    fire_red_location_names::map_area_name(group, map)
}

/// Returns the human-readable zone name for the player's current map.
///
/// Reads the current `(map_group, map_name_id)` from [`STATE`] and delegates
/// to [`get_area_name_for`].
pub fn get_area_name() -> &'static str {
    let state = get_value();
    get_area_name_for(state.map_group_id, state.map_name_id)
}

/// Returns the [`WildPokemonHeader`] for the player's current map.
///
/// Returns a default (empty) header if no matching entry is found. If multiple
/// headers match a given `(map_group, map_num)` pair (which should not occur in
/// a well-formed ROM), the last match wins.
pub fn get_area_pokemon_id() -> WildPokemonHeader {
    let state = get_value();
    get_area_pokemon_id_for_state(&state)
}

/// Like [`get_area_pokemon_id`] but uses the provided state instead of
/// reading from [`STATE`]. Used for the initial encounter load before the
/// map-polling thread has ticked.
pub fn get_area_pokemon_id_for_state(state: &FireRedState) -> WildPokemonHeader {
    if let Some(h) = get_area_pokemon_id_live(state) {
        return h;
    }
    let mut area_header = WildPokemonHeader::default();
    for header in get_wild_headers() {
        if header.map_group == state.map_group_id && header.map_num == state.map_name_id {
            area_header = WildPokemonHeader::fill_head(header, get_rom());
        }
    }
    area_header
}

/// Reads the wild-encounter header for the current map directly from RetroArch's
/// ROM memory space (GBA bus addresses 0x08000000+).
///
/// This gives correct encounter data for randomized ROMs: the local ROM file on
/// disk is only used to determine encounter-table structure (which map maps to
/// which ROM offset), while the actual Pokémon species are read live from the
/// game instance running in RetroArch.
///
/// Returns `None` if RetroArch is unreachable or no header matches the current map.
pub fn get_area_pokemon_id_live(state: &FireRedState) -> Option<WildPokemonHeader> {
    use fire_red_retroarch_interfacing::{generate_command, get_from_retroarch, make_socket};

    let rom_header = get_wild_headers()
        .iter()
        .rev()
        .find(|h| h.map_group == state.map_group_id && h.map_num == state.map_name_id)?;

    // Read `len` bytes from a GBA bus address via RetroArch.
    let read_at = |gba_addr: u32, len: usize| -> Option<Vec<u8>> {
        let socket = make_socket().ok()?;
        let cmd = generate_command(gba_addr, len);
        let resp = get_from_retroarch(&socket, &cmd, len + 2)?;
        resp.iter()
            .skip(2)
            .map(|s| u8::from_str_radix(s.trim(), 16).ok())
            .collect()
    };

    // Read one encounter type: the WildPokemonInfoROM struct (8 bytes) followed
    // by the species list at the pointer it contains.
    let read_info = |rom_ptr: u32| -> WildPokemonInfo {
        if rom_ptr == 0 {
            return WildPokemonInfo::default();
        }
        let info_bytes = match read_at(rom_ptr | 0x08000000, 8) {
            Some(b) if b.len() >= 8 => b,
            _ => return WildPokemonInfo::default(),
        };
        let encounter_rate = info_bytes[0];
        let list_ptr = u32::from_le_bytes([
            info_bytes[4], info_bytes[5], info_bytes[6], info_bytes[7],
        ]) & 0x07FFFFFF;
        if list_ptr == 0 {
            return WildPokemonInfo::default();
        }

        // Read up to 200 entries × 4 bytes each.
        let list_bytes = match read_at(list_ptr | 0x08000000, 200 * 4) {
            Some(b) => b,
            None => return WildPokemonInfo::default(),
        };

        let sentinel = list_ptr | 0x08000000;
        let mut pokemon_list: Vec<WildPokemon> = Vec::new();
        for i in 0..200usize {
            let off = i * 4;
            if off + 4 > list_bytes.len() {
                break;
            }
            let word = u32::from_le_bytes([
                list_bytes[off],
                list_bytes[off + 1],
                list_bytes[off + 2],
                list_bytes[off + 3],
            ]);
            if word == sentinel {
                break;
            }
            let min_level = list_bytes[off];
            let max_level = list_bytes[off + 1];
            let species = u16::from_le_bytes([list_bytes[off + 2], list_bytes[off + 3]]);
            if species == 0 || max_level == 0 {
                break;
            }
            if !pokemon_list.iter().any(|m: &WildPokemon| m.species == species) {
                pokemon_list.push(WildPokemon { min_level, max_level, species });
            }
        }

        WildPokemonInfo {
            encounter_rate,
            pokemon_count: pokemon_list.len(),
            wild_pokemon_list: pokemon_list,
        }
    };

    let mut area_header = WildPokemonHeader {
        map_group: rom_header.map_group,
        map_num:   rom_header.map_num,
        ..Default::default()
    };

    area_header.land_mon_encounters  = read_info(rom_header.land_mon_encounters_rom_ptr);
    area_header.water_mon_encounters = read_info(rom_header.water_mon_encounters_rom_ptr);
    area_header.rock_smash_encounters = read_info(rom_header.rock_smash_encounters_rom_ptr);
    area_header.fishing_encounters   = read_info(rom_header.fishing_encounters_rom_ptr);

    Some(area_header)
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

        push_names(
            &mut area.land,
            &wild_header.land_mon_encounters.wild_pokemon_list,
        );
        push_names(
            &mut area.water,
            &wild_header.water_mon_encounters.wild_pokemon_list,
        );
        push_names(
            &mut area.rock,
            &wild_header.rock_smash_encounters.wild_pokemon_list,
        );
        push_names(
            &mut area.fishing,
            &wild_header.fishing_encounters.wild_pokemon_list,
        );
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
    let offset = fire_red_rom_buffer::get_rom_addresses().map_group_and_name_addr - EWRAM_BASE;

    if ewram.len() < offset + 2 {
        return FireRedState::default();
    }

    FireRedState {
        map_group_id: ewram[offset],
        map_name_id: ewram[offset + 1],
    }
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
            ("grass encounters", &self.land),
            ("water encounters", &self.water),
            ("rock smash encounters", &self.rock),
            ("fishing encounters", &self.fishing),
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

// ---------------------------------------------------------------------------
// C-ABI helpers — StringArray construction and teardown
// ---------------------------------------------------------------------------

/// Converts a `Vec<String>` into a heap-allocated [`StringArray`] of C strings.
///
/// Strings containing interior null bytes are silently dropped. Returns a
/// zeroed default if the resulting list is empty.
fn strings_to_string_array(strings: Vec<String>) -> StringArray {
    let mut c_strs: Vec<*mut c_char> = strings
        .into_iter()
        .filter_map(|s| CString::new(s).ok())
        .map(|cs| cs.into_raw())
        .collect();

    if c_strs.is_empty() {
        return StringArray::default();
    }

    let len = c_strs.len();
    let cap = c_strs.capacity();
    let data = c_strs.as_mut_ptr();
    std::mem::forget(c_strs);
    StringArray { data, len, cap }
}

/// Frees all C strings in a [`StringArray`] and their backing pointer array.
///
/// # Safety
///
/// `arr` must have been produced by [`strings_to_string_array`]. All contained
/// `*mut c_char` pointers must have been allocated by `CString::into_raw`.
unsafe fn free_string_array(arr: StringArray) {
    if arr.data.is_null() || arr.len == 0 {
        return;
    }
    let ptrs = unsafe { Vec::from_raw_parts(arr.data, arr.len, arr.cap) };
    for p in ptrs {
        if !p.is_null() {
            unsafe {
                drop(CString::from_raw(p));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// C-ABI entry points — area encounter strings
// ---------------------------------------------------------------------------

/// Returns the wild-encounter pokemon name strings for the current map as a
/// heap-allocated [`AreaEncountersStringArrays`].
///
/// The caller **must** free the returned pointer with
/// [`free_area_encounters_string_arrays`]. Returns `NULL` if the start loop
/// has not been called.
#[unsafe(no_mangle)]
pub extern "C" fn get_area_pokemon_strings_ffi() -> *mut AreaEncountersStringArrays {
    let vecs = get_area_pokemon_strings();
    let arrays = Box::new(AreaEncountersStringArrays {
        land: strings_to_string_array(vecs.land),
        water: strings_to_string_array(vecs.water),
        rock_smash: strings_to_string_array(vecs.rock),
        fishing: strings_to_string_array(vecs.fishing),
    });
    Box::into_raw(arrays)
}

/// Frees an [`AreaEncountersStringArrays`] returned by
/// [`get_area_pokemon_strings_ffi`].
///
/// # Safety
///
/// `ptr` must be a non-null pointer returned by `get_area_pokemon_strings_ffi`
/// and must not have already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_area_encounters_string_arrays(ptr: *mut AreaEncountersStringArrays) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let land = std::ptr::read(&(*ptr).land);
        let water = std::ptr::read(&(*ptr).water);
        let rock_smash = std::ptr::read(&(*ptr).rock_smash);
        let fishing = std::ptr::read(&(*ptr).fishing);
        free_string_array(land);
        free_string_array(water);
        free_string_array(rock_smash);
        free_string_array(fishing);
        drop(Box::from_raw(ptr));
    }
}

// ---------------------------------------------------------------------------
// C-ABI entry points — wild pokemon header FFI
// ---------------------------------------------------------------------------

/// Returns the [`WildPokemonHeaderFFI`] for the player's current map as a
/// heap-allocated pointer.
///
/// Returns `NULL` if no encounter header exists for the current map (e.g. towns
/// or maps with no wild encounters). The caller **must** free the returned
/// pointer with [`free_wild_pokemon_header_ffi`].
///
/// # Safety requirements for the caller
///
/// The returned pointer is valid until `free_wild_pokemon_header_ffi` is called.
/// Do not free the inner `*mut WildPokemonInfoFFI` fields independently.
#[unsafe(no_mangle)]
pub extern "C" fn get_area_pokemon_header_ffi() -> *mut WildPokemonHeaderFFI {
    let state = get_value();
    for header in get_wild_headers() {
        if header.map_group == state.map_group_id && header.map_num == state.map_name_id {
            let ffi = WildPokemonHeaderFFI::fill_head(header, get_rom());
            return Box::into_raw(Box::new(ffi));
        }
    }
    std::ptr::null_mut()
}

/// Frees a [`WildPokemonHeaderFFI`] returned by [`get_area_pokemon_header_ffi`].
///
/// Also frees all inner `*mut WildPokemonInfoFFI` allocations via the
/// `WildPokemonHeaderFFI` `Drop` implementation.
///
/// # Safety
///
/// `ptr` must be a non-null pointer returned by `get_area_pokemon_header_ffi`
/// and must not have already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_wild_pokemon_header_ffi(ptr: *mut WildPokemonHeaderFFI) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(ptr));
    }
}

// ---------------------------------------------------------------------------
// C-ABI entry point — current map position (fire_red_map_data integration)
// ---------------------------------------------------------------------------

/// Returns the current map group and name IDs as a [`CurrentMapGroupAndName`].
///
/// Reads from the live [`FireRedState`] snapshot maintained by the polling
/// thread. Returns a zeroed struct if called before [`start_loop`].
#[unsafe(no_mangle)]
pub extern "C" fn c_get_current_map_position() -> CurrentMapGroupAndName {
    let state = get_value();
    CurrentMapGroupAndName {
        group: state.map_group_id,
        name: state.map_name_id,
    }
}
