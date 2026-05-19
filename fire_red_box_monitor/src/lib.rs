//! # FireRed Box Monitor
//! 
//! Monitors the player's PC box storage and maintains a deduplicated, in-memory
//! cache of every [`BoxPokemon`] seen across all 14 boxes.
//! 
//! ## Memory layout
//! 
//! FireRed stores PC box data in a structure called `gPokemonStorage` in WRAM.
//! Each slot is [`SLOT_SIZE`] (0x50) bytes laid out as:
//! 
//! ```text
//! slot address = box_0_base + (box * NUMBER_SLOTS + slot) * SLOT_SIZE
//! ```
//! 
//! The base address of box 0 is **not** fixed - it is found at runtime by
//! dereferncing the `SaveBlock3` pointer at [`SAVE_BLOCK_3_PTR`] and adding
//! [`BOX_DATA_OFFSET`]. This indirection is necessary becasue the storage
//! address can shift between saves.
//! 
//! ## Background thread
//! 
//! [`start_loop`] spawns a thread that calls [`update_box_list`] every
//! [`SLEEP_TIMER_IN_SECS`] seconds. The thread wraps each update in
//! [`std::panic::catch_unwind`] so a single bad memory read cannot bring down
//! the whole process. Call [`end_loop`] to stop it.
//! 
//! ## Deduplication
//! 
//! [`PokemonStorage`] tracks both a `Vec` of entries and a `HashSet` of seen
//! species IDs. [`check_for_new_entry`] uses the set as a fast guard so that a 
//! species already observed in the box is not added a second time. This means
//! the storage list reflects *unique species*, not every individual pokemon.

// Each PC slot is 80 bytes (0x50)
// there are 14 boxes x 30 slots
// starting at gPokemonStorage in WRAM
// slot address = POKEMON_STORAGE_ADDR + ((box * 30 + slot) * 0x50);

use fire_red_party_monitor::BoxPokemon;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// GBA addresss of the `SaveBlock3` pointer in IWAM
/// 
/// Dereferencing this 4-byte little-endian pointer yields the base address of
/// the `gPokemonStorage` struct in EWRAM. The base address itself can shift
/// between saves, so this indirection must be followed at runtime.
static SAVE_BLOCK_3_PTR: usize = 0x03005010;

/// Byte offset from teh `SaveBlock3` base address to the start of box slot data.
/// 
/// Added to the dereferenced `SaveBlock3` pointer to get the address of box 0,
/// slot 0.
static BOX_DATA_OFFSET: usize = 0x4;

/// Size of one PC box slot in bytes.
/// 
/// Each `BoxPokemon` occupies exactly 80 (0x50) bytes in memory.
static SLOT_SIZE: usize = 0x50;

/// Total number of PC boxews in FireRed's storage system.
static NUMBER_BOXES: usize = 14;

/// Number of pokemon slots per PC box.
static NUMBER_SLOTS: usize = 30;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// The in-memory cache of all PC box pokemon, populated by the background thread.
/// Initialized lazily on first access.
static POKEMON_STORAGE_LIST: OnceLock<Mutex<PokemonStorage>> = OnceLock::new();

/// How often the background thead refreshes the box cache, in seconds.
static SLEEP_TIMER_IN_SECS: u64 = 5;

/// Set to `true` while the background thread should keep running.
/// Flipped to `false` by [`end_loop`]
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Handle to the background polling thread, stored so [`end_loop`] can join it.
static THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Thread lifecycle
// ---------------------------------------------------------------------------

/// Starts the background box-monitoring thread.
/// 
/// The thread calls [`update_box_list`] every [`SLEEP_TIMER_IN_SECS`] seconds.
/// Each cell is wrapped in [`std::panic::catch_unwind`] so a panicking read
/// does not propogate and crash the process.
/// 
/// Calling this while the loop is already running will spawn a second thread;
/// callers should ensure it is only called once.
pub fn start_loop() {
    RUNNING.store(true, Ordering::SeqCst);

    let handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            let result = std::panic::catch_unwind(|| update_box_list());
            if let Err(_) = result {
                eprintln!("Panic occurred while updating box list");
            }
            std::thread::sleep(std::time::Duration::from_secs(SLEEP_TIMER_IN_SECS));
        }
    });

    *THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
}

/// Signals the background thread to stop and blocks until it exits.
/// 
/// If the thread has already exited or was never started, this is a no-op
pub fn end_loop() {
    RUNNING.store(false, Ordering::SeqCst);

    let mut handle_slot = THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = handle_slot.take() {
        if let Err(e) = handle.join() {
            eprintln!("Error joining thread: {:?}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Storage struct
// ---------------------------------------------------------------------------

/// In-memory cache of PC box pokemon.
/// 
/// Maintains two parallel data structures for 0(1) duplicate detections:
/// - `entries`         - the full list of unique [`BoxPokemon`] records.
/// - `species_set`     - the set of speciese IDs already present in `entries`
/// 
/// Both must be kept in sync; use [`check_for_new_entry`] and [`sync_storage`]
/// rather than mutating the fields directly.
#[derive(Debug)]
pub struct PokemonStorage {
    /// All unique box pokemon observed since the last full sync
    entries: Vec<BoxPokemon>,
    /// Species IDs present in `entries`, used as a fast existence check.
    species_set: HashSet<u16>,
}

impl PokemonStorage {
    /// Returns a reference to the global [`PokemonStorage`] mutex, initializing
    /// it with empty collections of first call.
    pub fn get_storage_list() -> &'static Mutex<PokemonStorage> {
        POKEMON_STORAGE_LIST.get_or_init(|| {
            Mutex::new(PokemonStorage {
                entries: Vec::new(),
                species_set: HashSet::new(),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// RAM address resolution
// ---------------------------------------------------------------------------

/// Resolves the runtime address of box 0, slot 0 in EWRAM.
/// 
/// Follows the `SaveBlock3` pointer indirection:
/// 1. Reads 4 bytes from [`SAVE_BLOCK_3_PTR`] to obtain the `SaveBlock3` base address.
/// 2. Adds [`BOX_DATA_OFFSET`] to get the first box slot address.
/// 
/// Retries up to 20 times on failed RetroArch reads before giving up.
/// 
/// # Returns
/// The GBA address of box 0, slot 0, or `None` if the address could not be 
/// resolved after all retries.
fn get_box_0_ram_location() -> Option<u32> {
    let max_retries: usize = 20;
    let mut retries = 0;
    let command = fire_red_retroarch_interfacing::generate_command((SAVE_BLOCK_3_PTR) as u32, 4);
    while retries < max_retries {
        match fire_red_retroarch_interfacing::get_from_retroarch(command.as_str(), 6) {
            Some(res) => {
                let bytes: Vec<u8> = res
                    .iter()
                    .skip(2)
                    .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
                    .collect();
                if bytes.len() >= 4 {
                    return Some(
                        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                            + BOX_DATA_OFFSET as u32,
                    );
                }
            }
            None => {
                retries += 1;
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Storage accessors
// ---------------------------------------------------------------------------

/// Returns a cloned snapshot of all unique box pokemon currently in the cache.
pub fn get_storage_entries() -> Vec<BoxPokemon> {
    PokemonStorage::get_storage_list()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .clone()
}

/// Returns a cloned snapshot of the set of species IDs currently in the cache.
pub fn get_storage_species_set() -> HashSet<u16> {
    PokemonStorage::get_storage_list()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .species_set
        .clone()
}

/// Returns a reference to the global [`PokemonStorage`] mutex
/// 
/// Equivalent to [`PokemonStorage::get_storage_list`]; provided as a free
/// function for call-site convenience.
pub fn get_storage_list() -> &'static Mutex<PokemonStorage> {
    POKEMON_STORAGE_LIST.get_or_init(|| {
        Mutex::new(PokemonStorage {
            entries: Vec::new(),
            species_set: HashSet::new(),
        })
    })
}

// ---------------------------------------------------------------------------
// Cache mutation
// ---------------------------------------------------------------------------

/// Adds `entry` to the storage cache if its species has not been seen before.
/// 
/// Species 0 is the empty-slot sentinel and is always ignored. If the species
/// is already in `species_set`, the entry is silently skipped.
/// 
/// # Returns
/// `Some(())` if the entry was inserted, `None` if it was skipped.
pub fn check_for_new_entry(entry: &BoxPokemon) -> Option<()> {
    if entry.secure.growth.species == 0 {
        return None;
    }

    let mut storage = PokemonStorage::get_storage_list()
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    if storage.species_set.contains(&entry.secure.growth.species) {
        return None;
    }

    storage.species_set.insert(entry.secure.growth.species);
    storage.entries.push(entry.clone());
    Some(())
}

/// Reconciles the in-memory cache against a freshly read list of box pokemon.
/// 
/// - If `list` is **empty**, the cache is fully cleared (entires and species set).
/// - Otherwise, any cached entry whose species is no longer present in `list`
///     is removed, and `species_set` is rebuilt to match.
/// 
/// This handles the case where a pokemon is released or moved out of the box
/// between refreshes
/// 
/// # Returns
/// The signed difference in cache size after the sync (negative = removals),
/// positive = additions, zero = no change).
pub fn sync_storage(list: &[BoxPokemon]) -> isize {
    let mut storage = PokemonStorage::get_storage_list()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let initial_size: isize = storage.species_set.len() as isize;

    if list.is_empty() {
        storage.entries = Vec::new();
        storage.species_set = HashSet::new();
    } else {
        let current_species: HashSet<u16> = list.iter().map(|i| i.secure.growth.species).collect();

        storage
            .entries
            .retain(|p| current_species.contains(&p.secure.growth.species));
        storage.species_set = storage
            .entries
            .iter()
            .map(|p| p.secure.growth.species)
            .collect();
    }
    storage.species_set.len() as isize - initial_size
}

// ---------------------------------------------------------------------------
// RAM reading
// ---------------------------------------------------------------------------

/// Reads all box slots from EWRAM and returns a `Vec` of non-empty [`BoxPokemon`]
/// 
/// Reads teh full box storage region (`NUMBER_BOXES x NUMBER_SLOTS x SLOT_SIZE`
/// bytes) in chunks of 5 boxes at a time to stay within RetroArch's practical
/// response sice limits. Each chunk is parsed slot-by-slot using
/// [`BoxPokemon::fill_struct_from_bytes`]; slots with zero chucksum (i.e empty
/// slots) are discarded.
/// 
/// ## Error handling
/// - If the box 0 address cannot be resolved, returns an empty `Vec`.
/// - Failed chunk reads are retried; after 5 consecutive failures the funtion
///   aborts early and returns whatever has been collected so far (or empty).
/// - Malformed or short responses are skipped without counting as a failure.
pub fn get_box_entries_from_ram() -> Vec<BoxPokemon> {
    use fire_red_retroarch_interfacing::*;

    let mut list: Vec<BoxPokemon> = Vec::new();

    let chunk_size = 5 * NUMBER_SLOTS * SLOT_SIZE;
    let full_size = NUMBER_BOXES * NUMBER_SLOTS * SLOT_SIZE;

    let box_0_location = match get_box_0_ram_location() {
        Some(loc) => loc,
        None => {
            println!("Unable to determine box data location in RAM.");
            return list; // Return empty list if we can't get the location
        }
    };

    let mut retries = 0;
    for chunk_start in (0..full_size).step_by(chunk_size) {
        let this_chunk_bytes = (full_size - chunk_start).min(chunk_size);

        let command = generate_command(
            box_0_location.saturating_add(chunk_start as u32),
            this_chunk_bytes,
        );
        let ret = get_from_retroarch(command.as_str(), this_chunk_bytes + 2);
        if ret.is_none() {
            println!(
                "Failed to read box data chunk starting at offset 0x{:X}",
                chunk_start
            );
            retries += 1;
            if retries >= 5 {
                println!("Too many consecutive failures reading box data, aborting.");
                return Vec::new();
            }
            continue;
        }
        let ret = ret.unwrap();
        let data: Vec<&str> = ret.iter().map(|s| s.as_str()).collect();

        // guard against malformed responses
        if data.len() < 3 {
            continue;
        }

        let bytes: Vec<u8> = data
            .iter()
            .skip(2)
            .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
            .collect();

        // guard against incomplete chunks
        if bytes.len() < SLOT_SIZE {
            continue;
        }

        for current_offset in (0..bytes.len()).step_by(SLOT_SIZE) {
            if current_offset + SLOT_SIZE > bytes.len() {
                break; // use break instead of panic
            }

            let res = BoxPokemon::fill_struct_from_bytes(
                &bytes,
                current_offset,
                fire_red_rom_buffer::get_rom(),
            );
            match res {
                Some(mon) => {
                    if mon.checksum != 0 {
                        list.push(mon);
                    }
                }
                None => continue,
            };
        }
    }

    list
}

/// Performs a full refresh of the box cache.
/// 
/// 1. Reads all box slots from RAM with [`get_box_entries_from_ram`]
/// 2. Reconciles the cache against the live data with [`sync_storage`] to
///    remove any stale entries.
/// 3. Reads RAM a second time and calls [`check_for_new_entry`] for each slot
///    to add any newly discovered species.
/// 
/// The double-read guards against race conditions where a pokemon appears
/// between teh sync and the new-entry check.
/// 
/// # Returns
/// `true` if at least one new species was added to the cache, `false` otherwise
pub fn update_box_list() -> bool {
    let list = get_box_entries_from_ram();
    sync_storage(&list);

    let mut change_occured = false;
    let list = get_box_entries_from_ram();
    for entry in list {
        let result = check_for_new_entry(&entry);
        if result.is_some() {
            change_occured = true;
        }
    }
    change_occured
}

// ---------------------------------------------------------------------------
// Diagnostic / debugging utilities
// ---------------------------------------------------------------------------

/// Scans all of EWRAM for a pokmeon with the given personality value.
/// 
/// Reads EWRAM (0x02000000-0x02040000) in 16 KB chunks and reports the absolute
/// GBA address of every 4-byte window that matches the little-endian encoding 
/// of `known_personality`. Useful for debugging when the expected box-slot
/// address is unknown or the storage layout has changed.
/// 
/// Results are printed to stdout; this function is intended as a diagnostic
/// tool and is not called during normal operations.
/// 
/// # Arguments
/// * `known_personality` - The 32-bit personality value (PID) to search for.
pub fn scan_for_pokemon(known_personality: u32) {
    use fire_red_retroarch_interfacing::*;

    // Scan all of EWRAM: 0x02000000 to 0x02040000
    // Do it in 16KB chunks to avoid huge requests
    let ewram_start = 0x02000000u32;
    let ewram_size = 0x00040000usize;
    let chunk = 0x4000usize;

    println!(
        "Scanning EWRAM for personality {:08X}...",
        known_personality
    );
    let target = known_personality.to_le_bytes();

    for offset in (0..ewram_size).step_by(chunk) {
        let addr = ewram_start + offset as u32;
        let command = generate_command(addr, chunk);
        let ret = get_from_retroarch(command.as_str(), chunk + 2);
        if ret.is_none() {
            println!("Failed to read EWRAM chunk starting at offset 0x{:X}", addr);
            continue;
        }
        let ret = ret.unwrap();
        let data: Vec<&str> = ret.iter().map(|s| s.as_str()).collect();
        let bytes: Vec<u8> = data
            .iter()
            .skip(2)
            .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
            .collect();

        for (i, window) in bytes.windows(4).enumerate() {
            if window == target {
                println!("HIT at absolute 0x{:08X}", addr.saturating_add(i as u32));
            }
        }
    }
    println!("Scan complete.");
}
