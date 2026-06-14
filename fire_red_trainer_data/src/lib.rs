//! FireRed Trainer Data
//!
//! Reads and monitors trainer/player metadata from Pokémon FireRed running in
//! RetroArch, using the EWRAM snapshot maintained by `fire_red_memory`.
//!
//! # Architecture
//!
//! Rather than issuing individual UDP reads, this crate slices the relevant
//! bytes directly out of the EWRAM snapshot. [`update_player_data`] calls
//! [`fire_red_memory::get_ewram`] to obtain the latest snapshot and parses
//! [`PlayerData`] from the revision-appropriate SaveBlock2 address — no network I/O.
//!
//! The background loop re-reads on a longer interval than the party loop
//! because trainer data (name, gender, ID, play time) changes infrequently.

mod trainer_data;

use arc_swap::ArcSwap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

pub use trainer_data::{PLAYER_DATA_SIZE, PlayerData};

// ---------------------------------------------------------------------------
// Statics and constants
// ---------------------------------------------------------------------------

/// Global shared player data state.
///
/// Uses [`ArcSwap`] for lock-free reads while the background thread updates.
static PLAYER_DATA: OnceLock<ArcSwap<PlayerData>> = OnceLock::new();

/// Controls the background polling loop.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// How long the background thread sleeps between checks.
///
/// Trainer data (name, gender, ID, play time) changes infrequently, so a
/// longer interval is appropriate here than for party or RAM data.
const SLEEP_TIMER: std::time::Duration = std::time::Duration::from_secs(15);

/// Base address of EWRAM in the GBA address space.
const EWRAM_BASE: usize = 0x02000000;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initializes and returns the global [`PlayerData`] static.
///
/// The initial value is parsed from the current EWRAM snapshot. If the
/// snapshot is not yet populated, a default (zeroed) [`PlayerData`] is used.
pub fn initialize_static_trainer_data() -> &'static ArcSwap<PlayerData> {
    PLAYER_DATA.get_or_init(|| {
        let data = read_player_data_from_ewram().unwrap_or_default();
        ArcSwap::from_pointee(data)
    })
}

/// Returns the global [`PlayerData`] static, initializing it if necessary.
pub fn get_static_trainer_data() -> &'static ArcSwap<PlayerData> {
    initialize_static_trainer_data()
}

/// Returns the current player data snapshot.
///
/// Returns `None` if the static has not yet been initialized.
pub fn get_player_data() -> Option<Arc<PlayerData>> {
    PLAYER_DATA.get().map(|arc| arc.load_full())
}

/// Starts the background trainer data polling loop.
///
/// Spawns a thread that checks for changes in [`PlayerData`] every
/// [`SLEEP_TIMER`]. The store is only updated when the parsed data differs
/// from the current snapshot, avoiding unnecessary Arc allocations.
///
/// Ensure `fire_red_memory::start_loop()` is running before calling this so
/// the EWRAM snapshot is being kept up to date.
pub fn start_loop() {
    RUNNING.store(true, Ordering::SeqCst);
    std::thread::spawn(|| {
        while RUNNING.load(Ordering::SeqCst) {
            update_player_data();
            std::thread::sleep(SLEEP_TIMER);
        }
    });
}

/// Stops the background polling loop.
pub fn end_loop() {
    RUNNING.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Converts an absolute GBA EWRAM address to a byte offset within the
/// EWRAM snapshot buffer.
#[inline]
fn ewram_offset(addr: usize) -> usize {
    debug_assert!(
        addr >= EWRAM_BASE,
        "address 0x{:08X} is below EWRAM_BASE",
        addr
    );
    addr - EWRAM_BASE
}

/// Reads and parses [`PlayerData`] from the current EWRAM snapshot.
///
/// Returns `None` if the snapshot is too small or the data cannot be parsed.
fn read_player_data_from_ewram() -> Option<PlayerData> {
    let ewram = fire_red_memory::get_ewram();
    let offset = ewram_offset(trainer_data::player_data_addr());

    let end = offset + PLAYER_DATA_SIZE;
    if ewram.len() < end {
        return None;
    }

    PlayerData::from_bytes(&ewram[offset..end])
}

/// Reads the latest [`PlayerData`] from the EWRAM snapshot and stores it if
/// it differs from the current value.
///
/// Skips the store entirely when the data is unchanged to avoid unnecessary
/// Arc allocations on every poll cycle.
fn update_player_data() {
    let Some(player) = read_player_data_from_ewram() else {
        tracing::warn!("Failed to parse player data from EWRAM snapshot.");
        return;
    };

    let current = get_static_trainer_data().load();
    if player != **current {
        get_static_trainer_data().store(Arc::new(player));
    }
}

/// Searches the EWRAM snapshot for a known player name byte sequence.
///
/// Useful during development to locate the correct address for a player name
/// when the layout is uncertain. Searches the full EWRAM snapshot in memory
/// rather than issuing UDP reads.
///
/// # Parameters
///
/// * `needle` — Raw GBA-encoded bytes of the name to search for.
///
/// # Returns
///
/// The absolute GBA address of the first match, or `None` if not found.
pub fn find_player_name(needle: &[u8]) -> Option<u32> {
    let ewram = fire_red_memory::get_ewram();

    ewram
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|offset| EWRAM_BASE as u32 + offset as u32)
}
