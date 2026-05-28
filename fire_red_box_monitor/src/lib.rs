//! # FireRed Box Monitor
//!
//! Monitors the player's PC box storage and maintains a deduplicated, in-memory
//! cache of every [`BoxPokemon`] seen across all 14 boxes.
//!
//! # Memory layout
//!
//! FireRed stores PC box data in `gPokemonStorage` in EWRAM. The base address
//! is not fixed — it must be resolved at runtime by dereferencing the
//! `SaveBlock3` pointer stored in IWRAM at [`SAVE_BLOCK_3_PTR`] and adding
//! [`BOX_DATA_OFFSET`].
//!
//! Each slot is [`SLOT_SIZE`] (0x50) bytes:
//!
//! ```text
//! slot address = box_0_base + (box * NUMBER_SLOTS + slot) * SLOT_SIZE
//! ```
//!
//! # Background thread
//!
//! [`start_loop`] spawns a thread that calls [`update_box_list`] every
//! [`SLEEP_TIMER`] seconds. Each call is wrapped in
//! [`std::panic::catch_unwind`] so a single bad read cannot crash the process.
//! Call [`end_loop`] to stop it.
//!
//! # Deduplication
//!
//! [`PokemonStorage`] tracks both a `Vec` of entries and a `HashSet` of seen
//! species IDs. [`check_for_new_entry`] uses the set as a fast guard so a
//! species already in the cache is never added twice. The cache therefore
//! reflects *unique species*, not every individual pokemon.

use fire_red_party_monitor::BoxPokemon;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Address constants
// ---------------------------------------------------------------------------

/// Base address of IWRAM in the GBA address space.
const IWRAM_BASE: usize = 0x03000000;

/// Base address of EWRAM in the GBA address space.
const EWRAM_BASE: usize = 0x02000000;

/// IWRAM address of the `SaveBlock3` pointer.
///
/// Dereferencing this 4-byte little-endian value yields the runtime base
/// address of `gPokemonStorage` in EWRAM.
const SAVE_BLOCK_3_PTR: usize = 0x03005010;

/// Byte offset from the `SaveBlock3` base to box 0, slot 0.
const BOX_DATA_OFFSET: usize = 0x4;

/// Size in bytes of one PC box slot (`BoxPokemon` on-disk format).
const SLOT_SIZE: usize = 0x50;

/// Total number of PC boxes in FireRed.
const NUMBER_BOXES: usize = 14;

/// Number of pokemon slots per box.
const NUMBER_SLOTS: usize = 30;

/// Total number of slots across all boxes.
const TOTAL_SLOTS: usize = NUMBER_BOXES * NUMBER_SLOTS;

/// Total bytes occupied by all box slot data.
const TOTAL_BOX_BYTES: usize = TOTAL_SLOTS * SLOT_SIZE;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// In-memory cache of all PC box pokemon, populated by the background thread.
static POKEMON_STORAGE_LIST: OnceLock<Mutex<PokemonStorage>> = OnceLock::new();

/// How often the background thread refreshes the box cache.
const SLEEP_TIMER: std::time::Duration = std::time::Duration::from_millis(250);

/// `true` while the background thread should keep running.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Handle to the background polling thread so [`end_loop`] can join it.
static THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Offset helpers
// ---------------------------------------------------------------------------

/// Converts an absolute GBA IWRAM address to a byte offset within the IWRAM
/// snapshot buffer.
#[inline]
fn iwram_offset(addr: usize) -> usize {
    debug_assert!(addr >= IWRAM_BASE, "address 0x{:08X} is below IWRAM_BASE", addr);
    addr - IWRAM_BASE
}

/// Converts an absolute GBA EWRAM address to a byte offset within the EWRAM
/// snapshot buffer.
#[inline]
fn ewram_offset(addr: usize) -> usize {
    debug_assert!(addr >= EWRAM_BASE, "address 0x{:08X} is below EWRAM_BASE", addr);
    addr - EWRAM_BASE
}

// ---------------------------------------------------------------------------
// Thread lifecycle
// ---------------------------------------------------------------------------

/// Starts the background box-monitoring thread.
///
/// The thread calls [`update_box_list`] every [`SLEEP_TIMER`]. Each call is
/// wrapped in [`std::panic::catch_unwind`] so a panicking read does not
/// propagate and crash the process.
///
/// Calling this while a loop is already running will spawn a second thread;
/// ensure it is only called once.
pub fn start_loop() {
    RUNNING.store(true, Ordering::SeqCst);

    let handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            if let Err(_) = std::panic::catch_unwind(|| update_box_list()) {
                eprintln!("Panic occurred while updating box list.");
            }
            std::thread::sleep(SLEEP_TIMER);
        }
    });

    *THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
}

/// Signals the background thread to stop and blocks until it exits.
///
/// If the thread has already exited or was never started, this is a no-op.
pub fn end_loop() {
    RUNNING.store(false, Ordering::SeqCst);
    let mut handle_slot = THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = handle_slot.take() {
        if let Err(e) = handle.join() {
            eprintln!("Error joining box monitor thread: {:?}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Storage struct
// ---------------------------------------------------------------------------

/// In-memory cache of PC box pokemon.
///
/// Maintains two parallel structures for O(1) duplicate detection:
/// - `entries`     — the full list of unique [`BoxPokemon`] records.
/// - `species_set` — species IDs already present in `entries`.
///
/// Mutate only via [`check_for_new_entry`] and [`sync_storage`].
#[derive(Debug)]
pub struct PokemonStorage {
    /// All unique box pokemon observed since the last full sync.
    pub entries: Vec<BoxPokemon>,
    /// Species IDs present in `entries`, used as a fast existence check.
    pub species_set: HashSet<u16>,
}

impl PokemonStorage {
    /// Returns a reference to the global [`PokemonStorage`] mutex, initializing
    /// it with empty collections on first call.
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
// Address resolution
// ---------------------------------------------------------------------------

/// Resolves the EWRAM byte offset of box 0, slot 0 from the current snapshots.
///
/// Reads the `SaveBlock3` pointer from the IWRAM snapshot, validates that it
/// points into EWRAM, and returns the corresponding EWRAM byte offset after
/// adding [`BOX_DATA_OFFSET`].
///
/// # Returns
///
/// `None` if the IWRAM snapshot is too small, the pointer is zero, or the
/// resolved address falls outside EWRAM.
fn get_box_0_ewram_offset() -> Option<usize> {
    let iwram = fire_red_memory::get_iwram();
    let ptr_offset = iwram_offset(SAVE_BLOCK_3_PTR);

    if iwram.len() < ptr_offset + 4 {
        return None;
    }

    let save_block_3_base = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;

    if save_block_3_base < EWRAM_BASE {
        eprintln!(
            "SaveBlock3 pointer 0x{:08X} is outside EWRAM — snapshot may not be ready.",
            save_block_3_base
        );
        return None;
    }

    let box_0_addr = save_block_3_base + BOX_DATA_OFFSET;
    if box_0_addr < EWRAM_BASE {
        return None;
    }

    Some(ewram_offset(box_0_addr))
}

// ---------------------------------------------------------------------------
// Storage accessors
// ---------------------------------------------------------------------------

/// Returns a reference to the global [`PokemonStorage`] mutex.
pub fn get_storage_list() -> &'static Mutex<PokemonStorage> {
    PokemonStorage::get_storage_list()
}

/// Returns a cloned snapshot of all unique box pokemon currently in the cache.
pub fn get_storage_entries() -> Vec<BoxPokemon> {
    PokemonStorage::get_storage_list()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .clone()
}

/// Returns a cloned snapshot of the species ID set currently in the cache.
pub fn get_storage_species_set() -> HashSet<u16> {
    PokemonStorage::get_storage_list()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .species_set
        .clone()
}

// ---------------------------------------------------------------------------
// Cache mutation
// ---------------------------------------------------------------------------

/// Adds `entry` to the cache if its species has not been seen before.
///
/// Species 0 is the empty-slot sentinel and is always ignored. Returns
/// `Some(())` if inserted, `None` if skipped.
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
/// - If `list` is **empty**, the cache is fully cleared.
/// - Otherwise, entries whose species no longer appear in `list` are removed
///   and `species_set` is rebuilt to match.
///
/// # Returns
///
/// The signed difference in cache size after the sync (negative = removals,
/// positive = additions, zero = no change).
pub fn sync_storage(list: &[BoxPokemon]) -> isize {
    let mut storage = PokemonStorage::get_storage_list()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let initial_size = storage.species_set.len() as isize;

    if list.is_empty() {
        storage.entries.clear();
        storage.species_set.clear();
    } else {
        let current_species: HashSet<u16> =
            list.iter().map(|p| p.secure.growth.species).collect();
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
// Snapshot reading
// ---------------------------------------------------------------------------

/// Reads all box slots from the current EWRAM snapshot and returns a `Vec` of
/// non-empty [`BoxPokemon`].
///
/// Resolves the box 0 offset from the IWRAM snapshot, then slices the full
/// `TOTAL_BOX_BYTES` region from the EWRAM snapshot and parses each
/// [`SLOT_SIZE`]-byte slot. Slots whose checksum is zero (empty slots) or
/// whose `get_bytes` returns `None` (bad checksum) are discarded.
///
/// Returns an empty `Vec` if the box address cannot be resolved or the EWRAM
/// snapshot is too small to contain the full storage region.
pub fn get_box_entries_from_ram() -> Vec<BoxPokemon> {
    let ewram = fire_red_memory::get_ewram();
    let rom = fire_red_rom_buffer::get_rom();

    let box_0_offset = match get_box_0_ewram_offset() {
        Some(offset) => offset,
        None => {
            eprintln!("Unable to determine box data location in EWRAM snapshot.");
            return Vec::new();
        }
    };

    let end_offset = box_0_offset + TOTAL_BOX_BYTES;
    if ewram.len() < end_offset {
        eprintln!(
            "EWRAM snapshot too small for full box data: have {} bytes, need {} at offset 0x{:X}.",
            ewram.len(),
            TOTAL_BOX_BYTES,
            box_0_offset,
        );
        return Vec::new();
    }

    let box_data = &ewram[box_0_offset..end_offset];
    let mut list = Vec::new();

    for slot in 0..TOTAL_SLOTS {
        let slot_offset = slot * SLOT_SIZE;
        let slot_bytes = &box_data[slot_offset..slot_offset + SLOT_SIZE];

        if let Some(mon) = BoxPokemon::from_bytes(slot_bytes, rom) {
            if mon.checksum != 0 {
                list.push(mon);
            }
        }
    }

    list
}

/// Performs a full refresh of the box cache from the current EWRAM snapshot.
///
/// 1. Reads all box slots from the snapshot with [`get_box_entries_from_ram`].
/// 2. Reconciles the cache with [`sync_storage`] to remove stale entries.
/// 3. Reads the snapshot a second time and calls [`check_for_new_entry`] for
///    each slot to add any newly discovered species.
///
/// The double-read guards against races where a pokemon appears in the snapshot
/// between the sync and the new-entry check.
///
/// # Returns
///
/// `true` if at least one new species was added to the cache.
pub fn update_box_list() -> bool {
    let list = get_box_entries_from_ram();
    sync_storage(&list);

    let list = get_box_entries_from_ram();
    let mut changed = false;
    for entry in list {
        if check_for_new_entry(&entry).is_some() {
            changed = true;
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// Diagnostic utilities
// ---------------------------------------------------------------------------

/// Scans the EWRAM snapshot for a pokemon with the given personality value.
///
/// Reports the absolute GBA address of every 4-byte window that matches the
/// little-endian encoding of `known_personality`. Useful for locating a
/// pokemon when the expected box-slot address is unknown or the storage layout
/// has changed.
///
/// Results are printed to stdout. This function is intended as a diagnostic
/// tool and is not called during normal operation.
///
/// # Arguments
///
/// * `known_personality` — The 32-bit personality value (PID) to search for.
pub fn scan_for_pokemon(known_personality: u32) {
    let ewram = fire_red_memory::get_ewram();
    let target = known_personality.to_le_bytes();

    println!("Scanning EWRAM snapshot for personality 0x{:08X}...", known_personality);

    for (offset, window) in ewram.windows(4).enumerate() {
        if window == target {
            println!("HIT at 0x{:08X}", EWRAM_BASE + offset);
        }
    }

    println!("Scan complete.");
}