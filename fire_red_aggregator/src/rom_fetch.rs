//! Fetch the FireRed ROM directly from RetroArch's emulated memory.
//!
//! The GBA ROM is mapped at `0x08000000` in RetroArch's emulated address
//! space and is readable via the same UDP network-command interface used for
//! EWRAM polling.  This lets the aggregator run in direct mode with no ROM
//! file on the server machine: it reads the ROM from RetroArch once, caches
//! it, and reuses the cached copy on every subsequent launch.
//!
//! # Cache layout
//!
//! ```text
//! ~/.cache/fire_red_aggregator/
//!   runs/<run_id>/<host>_<port>/<title>_<code>_<version>.gba   — with a DB run
//!   connections/<host>_<port>/<title>_<code>_<version>.gba      — no DB run
//! ```
//!
//! The host:port pair is used as the connection-level key (not a slot index)
//! because it is stable across reconnects for the same device and is always
//! unique between different devices, even when the aggregator reuses a slot
//! index for a reconnecting host.
//!
//! # Download strategy
//!
//! Instead of the naive serial approach (send one `READ_CORE_MEMORY` request,
//! wait for its reply, send the next), the downloader uses a **sliding window**:
//! up to [`PIPELINE_WINDOW`] requests are sent in a burst before waiting for
//! replies.  Because RetroArch queues incoming UDP commands and sends replies
//! immediately, the replies stream back in quick succession.  Responses are
//! matched to their chunk index via the address field embedded in every reply,
//! so out-of-order delivery is handled correctly.  Any chunk that does not
//! arrive before the per-window deadline is retried in a subsequent pass.
//!
//! # Timing
//!
//! FireRed is 16 MiB.  At 4 096 bytes per request and a window of 32, that is
//! 128 windows of 32 reads each.  On a typical LAN the full download takes
//! roughly 0.5–2 s (vs. 5–15 s with serial reads).

use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const ROM_BASE: u32 = 0x0800_0000;
const CHUNK: usize = 4_096;
const ROM_SIZE: usize = 16 * 1024 * 1024;

// Sequential constants — used only for the small header reads.
const TIMEOUT_MS: u64 = 800;
const RETRIES: usize = 5;

// Pipelined download constants.
/// In-flight `READ_CORE_MEMORY` requests per burst.
const PIPELINE_WINDOW: usize = 32;
/// How long to wait for all responses in one window before declaring misses.
const WINDOW_TIMEOUT_MS: u64 = 400;
/// Per-`recv` socket timeout; kept short so the wall-clock deadline is polled
/// frequently without busy-looping.
const RECV_POLL_MS: u64 = 10;
/// Maximum retry passes before the download is abandoned as failed.
const MAX_PASSES: usize = 8;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Delete the cached ROM for `host:port` / `run_id` and re-download it from RetroArch.
///
/// Removes any `.gba` file in the per-connection cache directory, then
/// performs a full fresh download.  Blocks until the download completes.
///
/// Returns the path of the newly written cache file, or an error string.
pub fn force_fetch_rom(host: &str, port: u16, run_id: Option<u32>) -> Result<PathBuf, String> {
    let dir = conn_dir(host, port, run_id)?;
    if dir.exists()
        && let Ok(entries) = std::fs::read_dir(&dir)
    {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("gba") {
                if let Err(e) = std::fs::remove_file(&p) {
                    return Err(format!("force refresh: cannot delete cached ROM: {}", e));
                }
                tracing::info!("ROM force-refresh: deleted cached ROM at {}", p.display());
            }
        }
    }

    // fetch_or_load_rom will see no cache and do a full download.
    fetch_or_load_rom(host, port, run_id)
}

/// Return a path to the ROM for `host:port`, downloading and caching it if needed.
///
/// The cache file lives at:
/// - `runs/<run_id>/<host>_<port>/<title>_<code>_<version>.gba` when a DB run is active
/// - `connections/<host>_<port>/<title>_<code>_<version>.gba` when there is no DB run
///
/// Blocks until the ROM is available (either from cache or freshly downloaded).
/// Returns an error string if RetroArch is unreachable or the download fails.
pub fn fetch_or_load_rom(host: &str, port: u16, run_id: Option<u32>) -> Result<PathBuf, String> {
    let socket = connect_socket(host, port)?;

    // Read probe A: first 4 KiB of the GBA ROM (header + ARM startup code).
    // The GBA header (bytes 0xA0-0xBF) uniquely identifies title/code/version.
    // Comparing 4 KiB instead of just 256 bytes also catches ROM hacks that
    // patch the startup code while leaving the header fields identical.
    let probe_a = read_retry(&socket, ROM_BASE, CHUNK)
        .ok_or_else(|| format!(
            "Could not read ROM header from RetroArch at {}:{}. \
             Is the game loaded and are network commands enabled?",
            host, port
        ))?;

    // Read probe B: 4 KiB from offset 0x23C000 (trainer-data region in vanilla
    // FireRed).  Data-only ROM hacks often leave the startup code intact but
    // modify ROM data starting at this range, so the second probe catches stale
    // caches that the startup-code probe misses.  Failure here is non-fatal —
    // if RetroArch can't supply this read we fall back to probe A only.
    let probe_b = read_retry(&socket, ROM_BASE + 0x0023_C000, CHUNK);

    // GBA header layout (all at ROM byte offsets):
    //   0xA0..0xAC  — game title (12 ASCII bytes, null-padded)
    //   0xAC..0xB0  — game code  (4 ASCII bytes, e.g. "BPRE")
    //   0xBC        — ROM version (u8)
    let title = std::str::from_utf8(&probe_a[0xA0..0xAC])
        .unwrap_or("")
        .trim_end_matches('\0')
        .trim()
        .to_string();
    let code = std::str::from_utf8(&probe_a[0xAC..0xB0])
        .unwrap_or("????")
        .trim_end_matches('\0')
        .to_string();
    let version = probe_a[0xBC];

    tracing::info!(
        "ROM fetch: identified \"{}\" ({}) v{} at {}:{}",
        title, code, version, host, port
    );

    let safe_title: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let safe_title = safe_title.trim_matches('_').to_string();
    let filename = format!("{}_{}_{}.gba", safe_title, code, version);
    let cache = conn_dir(host, port, run_id)?.join(&filename);
    if cache.exists() {
        // Validate the cached file against both probes.  Probe A checks the
        // startup code; probe B (trainer-data region) catches data-only ROM
        // hacks that leave the startup code identical to vanilla FireRed.
        let cache_ok = std::fs::File::open(&cache)
            .and_then(|mut f| {
                use std::io::{Read, Seek, SeekFrom};
                let mut buf_a = [0u8; CHUNK];
                f.read_exact(&mut buf_a)?;
                if buf_a.as_ref() != probe_a.as_slice() {
                    return Ok(false);
                }
                if let Some(ref pb) = probe_b {
                    let mut buf_b = [0u8; CHUNK];
                    f.seek(SeekFrom::Start(0x0023_C000))?;
                    f.read_exact(&mut buf_b)?;
                    Ok(buf_b.as_ref() == pb.as_slice())
                } else {
                    Ok(true)
                }
            })
            .unwrap_or(false);

        if cache_ok {
            tracing::info!("ROM fetch: using cached ROM at {}", cache.display());
            return Ok(cache);
        }
        tracing::warn!(
            "ROM fetch: cached ROM at {} does not match the running game \
             (probe mismatch — ROM was likely switched) — deleting stale \
             cache and re-downloading",
            cache.display()
        );
        let _ = std::fs::remove_file(&cache);
    }

    // Not in cache — download the full ROM via the pipelined downloader.
    let total_chunks = ROM_SIZE / CHUNK;
    tracing::info!(
        "ROM fetch: downloading {} MiB ({} chunks, window={}) from {}…",
        ROM_SIZE / 1024 / 1024,
        total_chunks,
        PIPELINE_WINDOW,
        host
    );

    let rom = download_pipelined(&socket)?;

    // Verify GBA header complement checksum (byte 0xBD).
    // Valid if: sum(ROM[0xA0..=0xBD]) + 0x19 == 0 (mod 256).
    if rom.len() > 0xBD {
        let chk: u8 = rom[0xA0..=0xBD]
            .iter()
            .fold(0u8, |acc, &b| acc.wrapping_add(b))
            .wrapping_add(0x19);
        if chk != 0 {
            return Err(format!(
                "ROM fetch: GBA header checksum invalid (got 0x{:02X}) — \
                 download may be corrupt or RetroArch did not supply the ROM correctly",
                rom[0xBD]
            ));
        }
    }

    // Persist to cache.
    std::fs::create_dir_all(conn_dir(host, port, run_id)?)
        .map_err(|e| format!("ROM fetch: cannot create cache dir: {}", e))?;
    std::fs::write(&cache, &rom)
        .map_err(|e| format!("ROM fetch: cannot write cache file: {}", e))?;

    tracing::info!("ROM fetch: saved to {}", cache.display());
    Ok(cache)
}

// ---------------------------------------------------------------------------
// Pipelined downloader
// ---------------------------------------------------------------------------

/// Download the full GBA ROM using a sliding-window pipeline.
///
/// Sends up to [`PIPELINE_WINDOW`] `READ_CORE_MEMORY` requests before waiting
/// for replies.  Responses are matched by the address embedded in each reply,
/// so out-of-order delivery is handled correctly.  Any chunk that misses the
/// per-window deadline is retried in a subsequent pass.
fn download_pipelined(socket: &UdpSocket) -> Result<Vec<u8>, String> {
    let total_chunks = ROM_SIZE / CHUNK;
    let mut rom = vec![0u8; ROM_SIZE];
    let mut received = vec![false; total_chunks];
    // Large enough for the ASCII-hex response of one full chunk:
    // "READ_CORE_MEMORY 0xADDRESS " + "xx " * 4096 ≈ 12 330 bytes.
    let mut buf = vec![0u8; 32_768];

    // Short per-recv timeout so we poll the wall-clock deadline without
    // blocking indefinitely when the socket goes quiet.
    socket
        .set_read_timeout(Some(Duration::from_millis(RECV_POLL_MS)))
        .map_err(|e| format!("ROM fetch: set_read_timeout: {}", e))?;

    let mut pending: Vec<usize> = (0..total_chunks).collect();

    for pass in 0..MAX_PASSES {
        if pending.is_empty() {
            break;
        }
        if pass > 0 {
            tracing::warn!(
                "ROM fetch: pass {}: retrying {} missed chunks",
                pass + 1,
                pending.len()
            );
        }

        // Process pending chunks in windows of PIPELINE_WINDOW.
        for window in pending.chunks(PIPELINE_WINDOW) {
            // Burst-send all requests in this window.
            for &i in window {
                let addr = ROM_BASE + (i * CHUNK) as u32;
                let cmd = format!("READ_CORE_MEMORY 0x{:08X} {}", addr, CHUNK);
                socket
                    .send(cmd.as_bytes())
                    .map_err(|e| format!("ROM fetch: send: {}", e))?;
            }

            // Count how many window chunks are still outstanding.  Chunks
            // already received (e.g. stray late replies from the previous
            // window) are skipped.
            let mut window_remaining: usize =
                window.iter().filter(|&&i| !received[i]).count();

            let deadline = Instant::now() + Duration::from_millis(WINDOW_TIMEOUT_MS);

            while window_remaining > 0 && Instant::now() < deadline {
                match socket.recv(&mut buf) {
                    Ok(n) => {
                        if let Some((addr, bytes)) = parse_read_response(&buf[..n], CHUNK)
                            && addr >= ROM_BASE
                        {
                            let idx = (addr - ROM_BASE) as usize / CHUNK;
                            if idx < total_chunks && !received[idx] {
                                rom[idx * CHUNK..(idx + 1) * CHUNK]
                                    .copy_from_slice(&bytes);
                                received[idx] = true;
                                // Advance the window counter only when a
                                // chunk from THIS window arrives; late
                                // replies from a previous window are still
                                // captured above but don't affect progress.
                                if window.contains(&idx) {
                                    window_remaining -= 1;
                                }
                            }
                        }
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // Per-recv timeout expired; loop back and check deadline.
                    }
                    Err(e) => return Err(format!("ROM fetch: recv: {}", e)),
                }
            }
        }

        // Log progress and rebuild pending from the global received state.
        let n_received = received.iter().filter(|&&r| r).count();
        tracing::info!(
            "ROM fetch: {}/{} chunks ({:.0}%)",
            n_received,
            total_chunks,
            n_received as f64 / total_chunks as f64 * 100.0
        );

        pending.retain(|&i| !received[i]);
    }

    if !pending.is_empty() {
        return Err(format!(
            "ROM fetch: {} / {} chunks not received after {} passes",
            pending.len(),
            total_chunks,
            MAX_PASSES
        ));
    }

    Ok(rom)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the per-connection cache directory for `host:port` / `run_id`.
///
/// Layout:
/// - `<cache_dir>/runs/<run_id>/<host>_<port>/`  when run_id is Some
/// - `<cache_dir>/connections/<host>_<port>/`     when run_id is None
fn conn_dir(host: &str, port: u16, run_id: Option<u32>) -> Result<PathBuf, String> {
    let safe_host: String = host
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let dir_name = format!("{}_{}", safe_host, port);
    let base = cache_dir();
    Ok(match run_id {
        Some(id) => base.join("runs").join(id.to_string()).join(&dir_name),
        None => base.join("connections").join(&dir_name),
    })
}

fn cache_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("fire_red_aggregator");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join("fire_red_aggregator");
    }
    if let Ok(data) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(data).join("fire_red_aggregator").join("cache");
    }
    PathBuf::from("rom_cache")
}

/// Open a UDP socket "connected" to `host:port`.
fn connect_socket(host: &str, port: u16) -> Result<UdpSocket, String> {
    let is_local = host == "127.0.0.1" || host == "::1" || host == "localhost";
    let bind = if is_local { "127.0.0.1:0" } else { "0.0.0.0:0" };

    let socket = UdpSocket::bind(bind)
        .map_err(|e| format!("ROM fetch: bind failed: {}", e))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(TIMEOUT_MS)))
        .map_err(|e| format!("ROM fetch: set_read_timeout failed: {}", e))?;
    socket
        .connect(format!("{}:{}", host, port))
        .map_err(|e| format!("ROM fetch: connect to {}:{} failed: {}", host, port, e))?;

    Ok(socket)
}

/// Send a `READ_CORE_MEMORY` command and return the decoded bytes.
/// Retries up to [`RETRIES`] times on timeout.  Used only for small
/// sequential reads (e.g. the ROM header).
fn read_retry(socket: &UdpSocket, addr: u32, len: usize) -> Option<Vec<u8>> {
    let cmd = format!("READ_CORE_MEMORY 0x{:08X} {}", addr, len);
    let mut buf = vec![0u8; 32_768];

    for _ in 0..RETRIES {
        if socket.send(cmd.as_bytes()).is_err() {
            return None;
        }
        match socket.recv(&mut buf) {
            Ok(n) => {
                if let Some((_addr, bytes)) = parse_read_response(&buf[..n], len) {
                    return Some(bytes);
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(_) => return None,
        }
    }
    None
}

/// Parse a `READ_CORE_MEMORY 0xADDR b0 b1 …` response.
///
/// Returns `Some((address, bytes))` on success, `None` if the response is
/// malformed or contains a different number of bytes than `expected`.
fn parse_read_response(buf: &[u8], expected: usize) -> Option<(u32, Vec<u8>)> {
    let s = std::str::from_utf8(buf).ok()?;
    let mut parts = s.split_ascii_whitespace();

    if parts.next()? != "READ_CORE_MEMORY" {
        return None;
    }
    let addr_str = parts.next()?;
    let addr = u32::from_str_radix(addr_str.trim_start_matches("0x"), 16).ok()?;

    let bytes: Vec<u8> = parts
        .filter_map(|t| u8::from_str_radix(t, 16).ok())
        .collect();

    if bytes.len() == expected {
        Some((addr, bytes))
    } else {
        None
    }
}
