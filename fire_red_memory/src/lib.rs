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
//! Both regions and all chunks within each region are read concurrently.
//! Each chunk runs on its own thread with its own UDP socket. Because RetroArch
//! includes the requested address in every response header, chunks can be
//! dispatched simultaneously and reassembled in order after all threads finish.
//!
//! Concurrency is bounded by [`MAX_CONCURRENT_CHUNKS`] to avoid overwhelming
//! RetroArch with too many simultaneous requests. Chunks are issued in batches
//! of that size, with each batch fully completing before the next is started.
//!
//! At ~16 ms per chunk with sufficient concurrency, total read time should
//! approach a single round-trip latency regardless of region size.

use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use arc_swap::ArcSwap;
use fire_red_retroarch_interfacing::{generate_command, get_from_retroarch, make_socket};

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
/// testing, 8 concurrent chunks is a reliable ceiling before responses start
/// being lost. Tune downward if retries increase, upward if RetroArch handles
/// more load without issue.
const MAX_CONCURRENT_CHUNKS: usize = 32;

/// GBA IWRAM address range (inclusive on both ends).
const IWRAM_START: u32 = 0x03000000;
const IWRAM_END:   u32 = 0x03007FFF;

/// GBA EWRAM address range (inclusive on both ends).
const EWRAM_START: u32 = 0x02000000;
const EWRAM_END:   u32 = 0x0203FFFF;

/// How long the background thread sleeps between full memory reads.
const SLEEP_DURATION: std::time::Duration = std::time::Duration::from_millis(250);

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
    fn start(&self) -> u32 { IWRAM_START }
    fn end(&self)   -> u32 { IWRAM_END   }
}

impl MemoryType for Ewram {
    fn start(&self) -> u32 { EWRAM_START }
    fn end(&self)   -> u32 { EWRAM_END   }
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
    std::thread::spawn(|| {
        while RUNNING.load(Ordering::SeqCst) {
            if let Err(e) = update_memory() {
                eprintln!("{}", e);
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
    println!("ending memory loop");
    RUNNING.store(false, Ordering::SeqCst);
}

/// Returns the most recent IWRAM snapshot.
///
/// Initializes the static to an empty `Vec` if [`start_loop`] has not been
/// called yet. The returned [`Arc`] is a consistent point-in-time snapshot;
/// the background thread may update the stored value concurrently without
/// affecting data visible through this handle.
pub fn get_iwram() -> Arc<Vec<u8>> {
    LOADED_IWRAM
        .get_or_init(|| ArcSwap::from_pointee(Vec::new()))
        .load_full()
}

/// Returns the most recent EWRAM snapshot.
///
/// See [`get_iwram`] for details.
pub fn get_ewram() -> Arc<Vec<u8>> {
    LOADED_EWRAM
        .get_or_init(|| ArcSwap::from_pointee(Vec::new()))
        .load_full()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Reads IWRAM and EWRAM concurrently and atomically stores both results.
///
/// Spawns one thread per region; within each region, chunks are also read
/// concurrently in batches of [`MAX_CONCURRENT_CHUNKS`]. Both region threads
/// are joined before storing results, so readers always see a consistent pair
/// of snapshots from the same polling cycle.
///
/// # Errors
///
/// Returns `Err` if either region fails after too many consecutive UDP
/// failures, or if either region thread panics.
fn update_memory() -> Result<(), &'static str> {
    let iwram_thread = std::thread::spawn(|| update_ram_type::<Iwram>());
    let ewram_thread = std::thread::spawn(|| update_ram_type::<Ewram>());

    let iwram = iwram_thread
        .join()
        .map_err(|_| "IWRAM thread panicked.")?
        .ok_or("Unable to update IWRAM.")?;

    let ewram = ewram_thread
        .join()
        .map_err(|_| "EWRAM thread panicked.")?
        .ok_or("Unable to update EWRAM.")?;

    LOADED_IWRAM.get().unwrap().store(Arc::new(iwram));
    LOADED_EWRAM.get().unwrap().store(Arc::new(ewram));

    Ok(())
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
    let socket = make_socket();
    let command = generate_command(start + chunk_start, chunk_size as usize);
    let mut retries = 0u32;

    loop {
        let Some(ret) = get_from_retroarch(
            &socket,
            command.as_str(),
            // +2 for the "READ_CORE_MEMORY <addr>" prefix tokens.
            (chunk_size + 2) as usize,
        ) else {
            eprintln!(
                "Failed to read memory at offset 0x{:08X} (attempt {}/{})",
                start + chunk_start,
                retries + 1,
                MAX_RETRIES,
            );
            retries += 1;
            if retries >= MAX_RETRIES {
                eprintln!("Too many consecutive failures at 0x{:08X}, aborting chunk.", start + chunk_start);
                return None;
            }
            std::thread::sleep(RETRY_BACKOFF * retries);
            continue;
        };

        // Validate response header
if ret[0] != "READ_CORE_MEMORY" {
    eprintln!("Unexpected response type: {}", ret[0]);
    retries += 1;
    continue;
}

let response_addr = u32::from_str_radix(ret[1].trim(), 16).ok();
if response_addr != Some(start + chunk_start) {
    eprintln!(
        "Address mismatch: expected 0x{:08X}, got {:?}",
        start + chunk_start, response_addr
    );
    retries += 1;
    continue;
}

// Strict hex parse — reject the chunk if any token is malformed
let Some(bytes) = ret.iter().skip(2)
    .map(|s| u8::from_str_radix(s.trim(), 16).ok())
    .collect::<Option<Vec<u8>>>()
else {
    eprintln!("Malformed hex in response at 0x{:08X}", start + chunk_start);
    retries += 1;
    continue;
};

if bytes.len() < chunk_size as usize {
    eprintln!("Short read at 0x{:08X}", start + chunk_start);
    retries += 1;
    continue;
}

        return Some((chunk_start, bytes));
    }
}

/// Reads the entire memory region described by `T` from RetroArch.
///
/// Splits the region into [`MAX_CHUNK_SIZE`]-byte chunks and issues them in
/// concurrent batches of up to [`MAX_CONCURRENT_CHUNKS`] threads. Each thread
/// calls [`read_chunk`] with its own UDP socket. After each batch completes,
/// the results are keyed by `chunk_start` offset so they can be assembled in
/// address order regardless of which thread finished first.
///
/// # Why batches instead of all chunks at once?
///
/// RetroArch processes network commands on a single internal thread. Sending
/// all 64 EWRAM chunks simultaneously risks overflowing its receive queue and
/// silently dropping requests. Batching limits concurrent load to a level
/// RetroArch can reliably handle.
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

    // Build the list of (chunk_start, chunk_size) pairs up front.
    let chunks: Vec<(u32, u32)> = (0..full_size)
        .step_by(MAX_CHUNK_SIZE)
        .map(|chunk_start| {
            let chunk_size = (full_size - chunk_start).min(MAX_CHUNK_SIZE as u32);
            (chunk_start, chunk_size)
        })
        .collect();

    let mut results: Vec<(u32, Vec<u8>)> = Vec::with_capacity(chunks.len());

    // Process chunks in batches to bound concurrency.
    for batch in chunks.chunks(MAX_CONCURRENT_CHUNKS) {
        let handles: Vec<_> = batch
            .iter()
            .map(|&(chunk_start, chunk_size)| {
                std::thread::spawn(move || read_chunk(start, chunk_start, chunk_size))
            })
            .collect();

        for handle in handles {
            match handle.join() {
                Ok(Some(result)) => results.push(result),
                Ok(None) => return None, // chunk exhausted its retries
                Err(_) => {
                    eprintln!("Chunk reader thread panicked.");
                    return None;
                }
            }
        }
    }

    // Sort by chunk offset and assemble. Chunks may have arrived out of order
    // relative to how they were dispatched, but each carries its offset as key.
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

    /// Integration test: reads both RAM regions from a live RetroArch instance.
    ///
    /// Requires RetroArch to be running with a FireRed ROM loaded and network
    /// commands enabled on port 55355. The test waits 2 seconds for the first
    /// poll to complete before asserting lengths.
    #[test]
    fn test_read() {
        // +1 because the address ranges are inclusive on both ends.
        let iwram_len = (IWRAM_END - IWRAM_START + 1) as usize;
        let ewram_len = (EWRAM_END - EWRAM_START + 1) as usize;

        start_loop();
        std::thread::sleep(std::time::Duration::from_millis(30));

        let iwram = get_iwram();
        let ewram = get_ewram();

        assert_eq!(iwram.len(), iwram_len, "IWRAM length mismatch");
        assert_eq!(ewram.len(), ewram_len, "EWRAM length mismatch");

        end_loop();
    }
}