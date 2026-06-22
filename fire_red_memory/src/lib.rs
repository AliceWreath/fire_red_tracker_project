//! Fire Red Memory
//!
//! Maintains an up-to-date in-memory snapshot of the IWRAM and EWRAM from a
//! Pokémon FireRed ROM running in RetroArch with the mGBA core.
//!
//! # Usage
//!
//! Call [`start_loop`] once to spawn a background thread that reads both RAM
//! regions from RetroArch every [`SLEEP_DURATION`] milliseconds and stores the
//! results atomically. Then call [`get_iwram`] or [`get_ewram`] from any thread
//! at any time to get a consistent snapshot without blocking the reader.
//!
//! Call [`end_loop`] to shut down the background thread gracefully.
//!
//! # Memory layout
//!
//! | Region | GBA address range       | Size    |
//! |--------|-------------------------|---------|
//! | IWRAM  | 0x03000000–0x03007FFF   | 32 KiB  |
//! | EWRAM  | 0x02000000–0x0203FFFF   | 256 KiB |
//!
//! # Chunk size
//!
//! RetroArch's network command interface silently drops responses that exceed
//! its internal send buffer (~12 KB of ASCII, or ~4,096 bytes of GBA data).
//! Reads are therefore broken into [`MAX_CHUNK_SIZE`]-byte chunks.
//!
//! # Parallelism
//!
//! Both regions are read on separate threads and each stores its result
//! independently as soon as it finishes — IWRAM readers (1 round-trip, ~16 ms)
//! no longer wait for EWRAM (4 round-trips, ~64 ms).
//!
//! Within each region, chunks are dispatched with a **sliding window** bounded
//! by [`MAX_CONCURRENT_CHUNKS`]. Each chunk runs on its own thread with its own
//! UDP socket. A counting semaphore (Mutex + Condvar) releases a slot as soon
//! as any chunk finishes, so the next chunk starts immediately rather than
//! waiting for an entire batch to drain. This eliminates the head-of-line stall
//! that strict batching causes when one chunk is slow or retrying.
//!
//! At ~16 ms per round-trip with 16 concurrent chunks, EWRAM (64 chunks) takes
//! approximately 4 × 16 ms = 64 ms; IWRAM (8 chunks) takes ~16 ms.

use arc_swap::ArcSwap;
use fire_red_retroarch_interfacing::{generate_command, get_from_retroarch, make_socket};
pub use fire_red_retroarch_interfacing::{get_thread_addr_string, set_thread_addr_string};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Statics
// ---------------------------------------------------------------------------

/// Holds the most recently read IWRAM snapshot.
///
/// Initialized to an empty `Vec` by [`start_loop`] (or lazily on first read).
/// Updated atomically by the background thread so readers never block.
static LOADED_IWRAM: OnceLock<ArcSwap<Vec<u8>>> = OnceLock::new();

/// Holds the most recently read EWRAM snapshot.
///
/// See [`LOADED_IWRAM`] for details.
static LOADED_EWRAM: OnceLock<ArcSwap<Vec<u8>>> = OnceLock::new();

/// Controls the background polling loop.
///
/// Set to `true` by [`start_loop`] and `false` by [`end_loop`].
static RUNNING: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Per-connection context
// ---------------------------------------------------------------------------

/// Per-connection EWRAM/IWRAM snapshot state.
///
/// Create one per RetroArch connection with [`MemoryContext::new`], pass it to
/// [`start_loop_ctx`], and register it on the game-loop thread with
/// [`set_thread_memory_context`] so that [`get_ewram`] / [`get_iwram`]
/// automatically return this connection's data on that thread.
pub struct MemoryContext {
    pub ewram: ArcSwap<Vec<u8>>,
    pub iwram: ArcSwap<Vec<u8>>,
    pub running: AtomicBool,
    /// Persistent IWRAM worker: send the RetroArch address to trigger a read.
    iwram_tx: mpsc::Sender<String>,
    iwram_rx: Mutex<mpsc::Receiver<Result<Vec<u8>, &'static str>>>,
    /// Persistent EWRAM worker: send the RetroArch address to trigger a read.
    ewram_tx: mpsc::Sender<String>,
    ewram_rx: Mutex<mpsc::Receiver<Result<Vec<u8>, &'static str>>>,
}

impl MemoryContext {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl Default for MemoryContext {
    fn default() -> Self {
        let (iwram_cmd_tx, iwram_cmd_rx) = mpsc::channel::<String>();
        let (iwram_res_tx, iwram_res_rx) = mpsc::sync_channel::<Result<Vec<u8>, &'static str>>(1);
        let (ewram_cmd_tx, ewram_cmd_rx) = mpsc::channel::<String>();
        let (ewram_res_tx, ewram_res_rx) = mpsc::sync_channel::<Result<Vec<u8>, &'static str>>(1);

        std::thread::spawn(move || {
            while let Ok(addr) = iwram_cmd_rx.recv() {
                set_thread_addr_string(addr);
                let result = update_ram_type::<Iwram>().ok_or("Unable to update IWRAM.");
                if iwram_res_tx.send(result).is_err() { break; }
            }
        });
        std::thread::spawn(move || {
            while let Ok(addr) = ewram_cmd_rx.recv() {
                set_thread_addr_string(addr);
                let result = update_ram_type::<Ewram>().ok_or("Unable to update EWRAM.");
                if ewram_res_tx.send(result).is_err() { break; }
            }
        });

        Self {
            ewram: ArcSwap::from_pointee(Vec::new()),
            iwram: ArcSwap::from_pointee(Vec::new()),
            running: AtomicBool::new(false),
            iwram_tx: iwram_cmd_tx,
            iwram_rx: Mutex::new(iwram_res_rx),
            ewram_tx: ewram_cmd_tx,
            ewram_rx: Mutex::new(ewram_res_rx),
        }
    }
}

thread_local! {
    static THREAD_MEM_CTX: RefCell<Option<Arc<MemoryContext>>> = const { RefCell::new(None) };
}

/// Registers `ctx` as this thread's memory context.
///
/// After this call, [`get_ewram`] and [`get_iwram`] on the calling thread will
/// return data from `ctx` rather than the global singleton.  Call this at the
/// top of every thread that belongs to a specific direct-mode connection
/// (game-loop thread, party monitor thread, box monitor thread, etc.).
pub fn set_thread_memory_context(ctx: Arc<MemoryContext>) {
    THREAD_MEM_CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of GBA memory bytes to request in a single RetroArch UDP
/// command.
///
/// RetroArch silently drops responses whose ASCII encoding exceeds its internal
/// send buffer. Empirically this limit sits at ~12,313 bytes of response, which
/// corresponds to 4,096 bytes of data (each byte encodes as "XX ", 3 chars,
/// plus a ~30-byte header). Requests larger than this receive no reply at all.
const MAX_CHUNK_SIZE: usize = 4_096;

/// Maximum number of chunk-reader threads active at the same time per region.
///
/// RetroArch processes network commands on a single thread, so flooding it
/// with more concurrent requests than it can queue will cause drops. In
/// testing, 16 concurrent chunks is a reliable ceiling before responses start
/// being lost. Tune downward if retries increase, upward if RetroArch handles
/// more load without issue.
const MAX_CONCURRENT_CHUNKS: usize = 16;

/// GBA IWRAM address range (inclusive on both ends).
const IWRAM_START: u32 = 0x03000000;
const IWRAM_END: u32 = 0x03007FFF;

/// GBA EWRAM address range (inclusive on both ends).
const EWRAM_START: u32 = 0x02000000;
const EWRAM_END: u32 = 0x0203FFFF;

/// How long the background thread sleeps between full memory reads.
const SLEEP_DURATION: std::time::Duration = std::time::Duration::from_millis(100);

/// How many consecutive UDP failures to tolerate before aborting a chunk read.
const MAX_RETRIES: u32 = 5;

/// Base backoff duration multiplied by the retry count on each failure,
/// giving an escalating delay: 50 ms, 100 ms, 150 ms, 200 ms.
const RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Memory type trait and implementations
// ---------------------------------------------------------------------------

/// Describes a contiguous GBA memory region with a start and end address.
///
/// Both addresses are inclusive. Implementors are expected to be zero-sized
/// marker types constructed via [`Default`].
trait MemoryType {
    fn start(&self) -> u32;
    fn end(&self) -> u32;
}

/// Marker type for GBA Internal Work RAM (IWRAM).
///
/// 32 KiB, mapped at 0x03000000–0x03007FFF. Fast RAM used for the game stack,
/// variables, and interrupt handlers.
#[derive(Default)]
struct Iwram;

/// Marker type for GBA External Work RAM (EWRAM).
///
/// 256 KiB, mapped at 0x02000000–0x0203FFFF. Used for bulk game data, save
/// state buffers, and overlay code.
#[derive(Default)]
struct Ewram;

impl MemoryType for Iwram {
    fn start(&self) -> u32 {
        IWRAM_START
    }
    fn end(&self) -> u32 {
        IWRAM_END
    }
}

impl MemoryType for Ewram {
    fn start(&self) -> u32 {
        EWRAM_START
    }
    fn end(&self) -> u32 {
        EWRAM_END
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Starts the background memory polling loop.
///
/// Spawns a thread that reads IWRAM and EWRAM from RetroArch every
/// [`SLEEP_DURATION`] and stores the results in [`LOADED_IWRAM`] and
/// [`LOADED_EWRAM`]. Both regions and all chunks within each region are read
/// concurrently, bounded by [`MAX_CONCURRENT_CHUNKS`] per region.
///
/// Safe to call multiple times — the statics are initialised only once.
///
/// Call [`end_loop`] to stop the background thread.
pub fn start_loop() {
    LOADED_EWRAM.get_or_init(|| ArcSwap::from_pointee(Vec::new()));
    LOADED_IWRAM.get_or_init(|| ArcSwap::from_pointee(Vec::new()));
    RUNNING.store(true, Ordering::SeqCst);
    let ra_addr = get_thread_addr_string();
    std::thread::spawn(move || {
        set_thread_addr_string(ra_addr);
        let wait_interval = std::time::Duration::from_secs(5);
        let mut connected = false;
        let mut last_waiting_print = std::time::Instant::now()
            .checked_sub(wait_interval)
            .unwrap_or_else(std::time::Instant::now);

        while RUNNING.load(Ordering::SeqCst) {
            match update_memory() {
                Ok(()) => {
                    if !connected {
                        tracing::info!("RetroArch connected.");
                        connected = true;
                    }
                }
                Err(_) => {
                    if connected {
                        tracing::warn!("Lost connection to RetroArch. Waiting...");
                        connected = false;
                    }
                    let now = std::time::Instant::now();
                    if now.duration_since(last_waiting_print) >= wait_interval {
                        tracing::debug!("Waiting for RetroArch...");
                        last_waiting_print = now;
                    }
                }
            }
            std::thread::sleep(SLEEP_DURATION);
        }
    });
}

/// Signals the background polling loop to stop.
///
/// The background thread will finish its current read cycle before exiting.
/// This function returns immediately without joining the thread.
pub fn end_loop() {
    tracing::info!("ending memory loop");
    RUNNING.store(false, Ordering::SeqCst);
}

/// Returns the most recent IWRAM snapshot.
///
/// If a per-connection [`MemoryContext`] has been registered on this thread
/// via [`set_thread_memory_context`], its IWRAM is returned.  Otherwise falls
/// back to the global singleton populated by [`start_loop`].
pub fn get_iwram() -> Arc<Vec<u8>> {
    let thread = THREAD_MEM_CTX.with(|c| c.borrow().as_ref().map(|ctx| ctx.iwram.load_full()));
    thread.unwrap_or_else(|| {
        LOADED_IWRAM
            .get_or_init(|| ArcSwap::from_pointee(Vec::new()))
            .load_full()
    })
}

/// Returns the most recent EWRAM snapshot.
///
/// If a per-connection [`MemoryContext`] has been registered on this thread
/// via [`set_thread_memory_context`], its EWRAM is returned.  Otherwise falls
/// back to the global singleton populated by [`start_loop`].
pub fn get_ewram() -> Arc<Vec<u8>> {
    let thread = THREAD_MEM_CTX.with(|c| c.borrow().as_ref().map(|ctx| ctx.ewram.load_full()));
    thread.unwrap_or_else(|| {
        LOADED_EWRAM
            .get_or_init(|| ArcSwap::from_pointee(Vec::new()))
            .load_full()
    })
}

/// Starts a per-connection memory polling loop.
///
/// Spawns a background thread that polls the RetroArch instance whose address
/// is set on the **calling** thread via
/// [`fire_red_retroarch_interfacing::set_thread_addr`] and writes results into
/// `ctx`.  Multiple connections can run concurrently because each has its own
/// [`MemoryContext`]; the global [`LOADED_EWRAM`] / [`LOADED_IWRAM`] are not
/// touched.
///
/// Stop the thread with [`end_loop_ctx`].
pub fn start_loop_ctx(ctx: Arc<MemoryContext>) {
    ctx.running.store(true, Ordering::SeqCst);
    let ra_addr = get_thread_addr_string();
    let ctx2 = ctx.clone();
    std::thread::spawn(move || {
        set_thread_addr_string(ra_addr);
        let wait_interval = std::time::Duration::from_secs(5);
        let mut connected = false;
        let mut last_waiting_print = std::time::Instant::now()
            .checked_sub(wait_interval)
            .unwrap_or_else(std::time::Instant::now);
        while ctx2.running.load(Ordering::SeqCst) {
            match update_memory_ctx(&ctx2) {
                Ok(()) => {
                    if !connected {
                        tracing::info!("RetroArch connected.");
                        connected = true;
                    }
                }
                Err(_) => {
                    if connected {
                        tracing::warn!("Lost connection to RetroArch. Waiting...");
                        connected = false;
                    }
                    let now = std::time::Instant::now();
                    if now.duration_since(last_waiting_print) >= wait_interval {
                        tracing::debug!("Waiting for RetroArch...");
                        last_waiting_print = now;
                    }
                }
            }
            std::thread::sleep(SLEEP_DURATION);
        }
    });
}

/// Signals the per-connection memory polling loop to stop.
pub fn end_loop_ctx(ctx: &MemoryContext) {
    ctx.running.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Per-connection variant of [`update_memory`]: writes results into `ctx`
/// instead of the global [`LOADED_EWRAM`] / [`LOADED_IWRAM`] singletons.
///
/// Delegates to the persistent IWRAM/EWRAM worker threads that were spawned in
/// [`MemoryContext::default`], avoiding OS thread creation overhead per tick.
fn update_memory_ctx(ctx: &Arc<MemoryContext>) -> Result<(), &'static str> {
    let ra_addr = get_thread_addr_string();
    // Trigger both workers concurrently before waiting on either result.
    ctx.iwram_tx.send(ra_addr.clone()).map_err(|_| "IWRAM worker disconnected.")?;
    ctx.ewram_tx.send(ra_addr).map_err(|_| "EWRAM worker disconnected.")?;
    let iwram_data = ctx.iwram_rx.lock().unwrap()
        .recv()
        .map_err(|_| "IWRAM worker disconnected.")??;
    let ewram_data = ctx.ewram_rx.lock().unwrap()
        .recv()
        .map_err(|_| "EWRAM worker disconnected.")??;
    ctx.iwram.store(Arc::new(iwram_data));
    ctx.ewram.store(Arc::new(ewram_data));
    Ok(())
}

/// Reads IWRAM and EWRAM concurrently, storing each region as soon as it
/// completes rather than waiting for both before storing either.
///
/// Spawns one thread per region. Each thread reads its region with a sliding
/// window of up to [`MAX_CONCURRENT_CHUNKS`] concurrent chunk requests and
/// stores the result immediately on success, so IWRAM readers (1 round-trip)
/// see fresh data without waiting for EWRAM (4 round-trips at 16 chunks).
///
/// # Errors
///
/// Returns `Err` if either region fails after too many consecutive UDP
/// failures, or if either region thread panics.
fn update_memory() -> Result<(), &'static str> {
    let ra_addr = get_thread_addr_string();
    let ra_addr2 = ra_addr.clone();
    let iwram_thread = std::thread::spawn(move || -> Option<()> {
        set_thread_addr_string(ra_addr);
        let data = update_ram_type::<Iwram>()?;
        LOADED_IWRAM.get()?.store(Arc::new(data));
        Some(())
    });
    let ewram_thread = std::thread::spawn(move || -> Option<()> {
        set_thread_addr_string(ra_addr2);
        let data = update_ram_type::<Ewram>()?;
        LOADED_EWRAM.get()?.store(Arc::new(data));
        Some(())
    });

    let iwram_ok = iwram_thread.join().map_err(|_| "IWRAM thread panicked.")?;
    let ewram_ok = ewram_thread.join().map_err(|_| "EWRAM thread panicked.")?;

    match (iwram_ok, ewram_ok) {
        (Some(()), Some(())) => Ok(()),
        (None, _) => Err("Unable to update IWRAM."),
        (_, None) => Err("Unable to update EWRAM."),
    }
}

/// Reads a single chunk of GBA memory from RetroArch, retrying on failure.
///
/// Creates its own UDP socket so it can be called from concurrent threads
/// without responses being stolen across sockets.
///
/// # Arguments
///
/// * `start`      - Base address of the memory region (e.g. `IWRAM_START`).
/// * `chunk_start` - Byte offset within the region for this chunk.
/// * `chunk_size`  - Number of bytes to request.
///
/// # Returns
///
/// - `Some((chunk_start, bytes))` on success.
/// - `None` if [`MAX_RETRIES`] consecutive failures occur.
fn read_chunk(start: u32, chunk_start: u32, chunk_size: u32) -> Option<(u32, Vec<u8>)> {
    let socket = match make_socket() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to create UDP socket: {e}");
            return None;
        }
    };
    let command = generate_command(start + chunk_start, chunk_size as usize);
    let mut retries = 0u32;

    loop {
        let Some(ret) = get_from_retroarch(
            &socket,
            command.as_str(),
            // +2 for the "READ_CORE_MEMORY <addr>" prefix tokens.
            (chunk_size + 2) as usize,
        ) else {
            retries += 1;
            if retries >= MAX_RETRIES {
                return None;
            }
            std::thread::sleep(RETRY_BACKOFF * retries);
            continue;
        };

        // Validate response header
        if ret[0] != "READ_CORE_MEMORY" {
            retries += 1;
            continue;
        }

        let response_addr = u32::from_str_radix(ret[1].trim(), 16).ok();
        if response_addr != Some(start + chunk_start) {
            retries += 1;
            continue;
        }

        // Strict hex parse — reject the chunk if any token is malformed
        let Some(bytes) = ret
            .iter()
            .skip(2)
            .map(|s| u8::from_str_radix(s.trim(), 16).ok())
            .collect::<Option<Vec<u8>>>()
        else {
            retries += 1;
            continue;
        };

        if bytes.len() < chunk_size as usize {
            retries += 1;
            continue;
        }

        return Some((chunk_start, bytes));
    }
}

/// Reads the entire memory region described by `T` from RetroArch.
///
/// Splits the region into [`MAX_CHUNK_SIZE`]-byte chunks and dispatches them
/// with a sliding window bounded by [`MAX_CONCURRENT_CHUNKS`]. Unlike strict
/// batching, a new chunk is dispatched as soon as any in-flight chunk finishes,
/// keeping the concurrency level full at all times. This eliminates the
/// head-of-line stall where a single slow/retrying chunk holds up the whole
/// next batch.
///
/// Each chunk runs on its own thread with its own UDP socket. A counting
/// semaphore (Mutex + Condvar) throttles dispatch; results are collected via
/// an mpsc channel after all chunks have been dispatched.
///
/// # Returns
///
/// - `Some(Vec<u8>)` — the full region, `(end - start + 1)` bytes long.
/// - `None` if any chunk fails after [`MAX_RETRIES`] consecutive failures.
fn update_ram_type<T: MemoryType + Default>() -> Option<Vec<u8>> {
    let mem_type = T::default();
    let (start, end) = (mem_type.start(), mem_type.end());

    // +1 because the address range is inclusive on both ends.
    let full_size: u32 = (end - start) + 1;

    let chunks: Vec<(u32, u32)> = (0..full_size)
        .step_by(MAX_CHUNK_SIZE)
        .map(|chunk_start| {
            let chunk_size = (full_size - chunk_start).min(MAX_CHUNK_SIZE as u32);
            (chunk_start, chunk_size)
        })
        .collect();

    let total = chunks.len();
    let (tx, rx) = mpsc::channel::<Option<(u32, Vec<u8>)>>();

    // Counting semaphore: permits = MAX_CONCURRENT_CHUNKS.
    // The semaphore is released inside each chunk thread (before the send) so
    // the dispatch loop can acquire the next slot without waiting for rx.recv().
    let semaphore = Arc::new((Mutex::new(MAX_CONCURRENT_CHUNKS), Condvar::new()));

    let ra_addr = get_thread_addr_string();

    for (chunk_start, chunk_size) in chunks {
        // Acquire a slot — blocks until a running chunk releases one.
        {
            let (lock, cvar) = &*semaphore;
            let mut slots = lock.lock().unwrap_or_else(|p| p.into_inner());
            while *slots == 0 {
                slots = cvar.wait(slots).unwrap_or_else(|p| p.into_inner());
            }
            *slots -= 1;
        }

        let tx = tx.clone();
        let sem = semaphore.clone();
        let addr = ra_addr.clone();
        std::thread::spawn(move || {
            set_thread_addr_string(addr);
            let result = read_chunk(start, chunk_start, chunk_size);
            // Release slot before sending so the dispatcher can proceed
            // without waiting for the channel recv.
            let (lock, cvar) = &*sem;
            *lock.lock().unwrap_or_else(|p| p.into_inner()) += 1;
            cvar.notify_one();
            let _ = tx.send(result); // ignored if caller already returned None
        });
    }

    // Drop the dispatch-side sender so rx drains to completion once all
    // chunk threads have sent their results.
    drop(tx);

    let mut results = Vec::with_capacity(total);
    for result in rx {
        let r = result?;
        results.push(r);
    }

    // Sort by chunk offset and assemble.
    results.sort_unstable_by_key(|(chunk_start, _)| *chunk_start);
    let mut ram_holder: Vec<u8> = Vec::with_capacity(full_size as usize);
    for (_, bytes) in results {
        ram_holder.extend_from_slice(&bytes);
    }
    Some(ram_holder)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ewram_region_size_and_chunk_count() {
        let full_size = (EWRAM_END - EWRAM_START + 1) as usize;
        assert_eq!(full_size, 256 * 1024, "EWRAM must be 256 KiB");
        // Every chunk is MAX_CHUNK_SIZE except possibly the last.
        let chunk_count = full_size.div_ceil(MAX_CHUNK_SIZE);
        assert_eq!(
            chunk_count, 64,
            "EWRAM must split into 64 chunks at 4 KiB each"
        );
        // All chunks fit inside one sliding window: ceiling(64/16) = 4 rounds.
        let window_rounds = chunk_count.div_ceil(MAX_CONCURRENT_CHUNKS);
        assert_eq!(window_rounds, 4);
    }

    #[test]
    fn iwram_region_size_and_chunk_count() {
        let full_size = (IWRAM_END - IWRAM_START + 1) as usize;
        assert_eq!(full_size, 32 * 1024, "IWRAM must be 32 KiB");
        let chunk_count = full_size.div_ceil(MAX_CHUNK_SIZE);
        assert_eq!(
            chunk_count, 8,
            "IWRAM must split into 8 chunks at 4 KiB each"
        );
        // 8 chunks < 16 concurrent → fits in a single window round.
        assert!(chunk_count <= MAX_CONCURRENT_CHUNKS);
    }

    #[test]
    fn last_chunk_is_correctly_sized_when_region_not_divisible() {
        // Verify the min() clamp: a region one byte larger than MAX_CHUNK_SIZE
        // should produce two chunks — a full one and a 1-byte tail.
        let full_size: u32 = MAX_CHUNK_SIZE as u32 + 1;
        let chunks: Vec<(u32, u32)> = (0..full_size)
            .step_by(MAX_CHUNK_SIZE)
            .map(|chunk_start| {
                let chunk_size = (full_size - chunk_start).min(MAX_CHUNK_SIZE as u32);
                (chunk_start, chunk_size)
            })
            .collect();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], (0, MAX_CHUNK_SIZE as u32));
        assert_eq!(chunks[1], (MAX_CHUNK_SIZE as u32, 1));
    }

    #[test]
    fn out_of_order_results_assemble_correctly() {
        // Simulate chunks arriving in reverse order (as they would from a
        // sliding window) and verify sort-then-extend produces correct output.
        let mut results: Vec<(u32, Vec<u8>)> = vec![
            (8, vec![0x07, 0x08]),
            (0, vec![0x01, 0x02, 0x03, 0x04]),
            (4, vec![0x05, 0x06]),
        ];
        results.sort_unstable_by_key(|(chunk_start, _)| *chunk_start);
        let mut assembled: Vec<u8> = Vec::new();
        for (_, bytes) in results {
            assembled.extend_from_slice(&bytes);
        }
        assert_eq!(
            assembled,
            vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
    }

    /// Integration test: reads both RAM regions from a live RetroArch instance.
    ///
    /// Requires RetroArch to be running with a FireRed ROM loaded and network
    /// commands enabled on port 55355. The test waits 2 seconds for the first
    /// poll to complete before asserting lengths.
    #[test]
    #[ignore = "requires live RetroArch with UDP memory commands on port 55355"]
    fn test_read() {
        // +1 because the address ranges are inclusive on both ends.
        let iwram_len = (IWRAM_END - IWRAM_START + 1) as usize;
        let ewram_len = (EWRAM_END - EWRAM_START + 1) as usize;

        start_loop();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let iwram = get_iwram();
        let ewram = get_ewram();

        assert_eq!(iwram.len(), iwram_len, "IWRAM length mismatch");
        assert_eq!(ewram.len(), ewram_len, "EWRAM length mismatch");

        end_loop();
    }
}
