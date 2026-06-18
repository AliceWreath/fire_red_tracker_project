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
use std::cell::RefCell;
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

// ---------------------------------------------------------------------------
// Per-connection context
// ---------------------------------------------------------------------------

/// Per-connection trainer/player data state.
///
/// Create one per direct-mode connection.  Pass it to [`start_loop_ctx`] to
/// start a background thread that writes to it, and register it on the
/// game-loop thread with [`set_thread_trainer_context`] so that
/// [`get_static_trainer_data`] / [`get_player_data`] return this connection's data.
pub struct TrainerContext {
    pub data: ArcSwap<PlayerData>,
    pub running: AtomicBool,
}

impl TrainerContext {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            data: ArcSwap::from_pointee(PlayerData::default()),
            running: AtomicBool::new(false),
        })
    }
}

impl Default for TrainerContext {
    fn default() -> Self {
        Self {
            data: ArcSwap::from_pointee(PlayerData::default()),
            running: AtomicBool::new(false),
        }
    }
}

thread_local! {
    static THREAD_TRAINER_CTX: RefCell<Option<Arc<TrainerContext>>> = const { RefCell::new(None) };
}

/// Registers `ctx` as this thread's trainer context.
///
/// After this call, [`get_static_trainer_data`] and [`get_player_data`] on the
/// calling thread will return data from `ctx` instead of the global singleton.
pub fn set_thread_trainer_context(ctx: Arc<TrainerContext>) {
    THREAD_TRAINER_CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

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

/// Returns the trainer data ArcSwap for the current thread's connection.
///
/// If a per-connection [`TrainerContext`] has been registered on this thread
/// via [`set_thread_trainer_context`], its current value is mirrored into the
/// global singleton (so the `&'static` reference remains valid) and the global
/// is returned.  Otherwise falls back to the global singleton directly.
pub fn get_static_trainer_data() -> &'static ArcSwap<PlayerData> {
    let thread_data =
        THREAD_TRAINER_CTX.with(|c| c.borrow().as_ref().map(|ctx| ctx.data.load_full()));
    if let Some(data) = thread_data {
        let global =
            PLAYER_DATA.get_or_init(|| ArcSwap::from_pointee(PlayerData::default()));
        global.store(data);
        return global;
    }
    initialize_static_trainer_data()
}

/// Returns the current player data snapshot.
///
/// If a per-connection [`TrainerContext`] has been registered on this thread
/// via [`set_thread_trainer_context`], its data is returned.  Otherwise falls
/// back to the global singleton populated by [`start_loop`].
pub fn get_player_data() -> Option<Arc<PlayerData>> {
    let thread =
        THREAD_TRAINER_CTX.with(|c| c.borrow().as_ref().map(|ctx| ctx.data.load_full()));
    if let Some(data) = thread {
        return Some(data);
    }
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

/// Reads per-connection trainer data from the EWRAM snapshot into `ctx`.
///
/// Calls [`fire_red_memory::get_ewram`], which returns the per-connection
/// snapshot when a [`fire_red_memory::MemoryContext`] is registered on the
/// calling thread.
pub fn update_trainer_data_ctx(ctx: &TrainerContext) {
    if let Some(player) = read_player_data_from_ewram() {
        ctx.data.store(Arc::new(player));
    }
}

/// Starts a per-connection trainer data polling loop.
///
/// Spawns a background thread that registers `mem_ctx` as its memory context
/// and calls [`update_trainer_data_ctx`] every [`SLEEP_TIMER`], writing
/// results into `trainer_ctx`.
///
/// Stop it with [`end_loop_ctx`].
pub fn start_loop_ctx(
    mem_ctx: Arc<fire_red_memory::MemoryContext>,
    trainer_ctx: Arc<TrainerContext>,
) {
    trainer_ctx.running.store(true, Ordering::SeqCst);
    std::thread::spawn(move || {
        fire_red_memory::set_thread_memory_context(mem_ctx);
        while trainer_ctx.running.load(Ordering::SeqCst) {
            update_trainer_data_ctx(&trainer_ctx);
            std::thread::sleep(SLEEP_TIMER);
        }
    });
}

/// Signals the per-connection trainer data polling loop to stop.
pub fn end_loop_ctx(ctx: &TrainerContext) {
    ctx.running.store(false, Ordering::SeqCst);
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
