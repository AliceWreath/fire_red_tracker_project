//! Aggregator Client
//! 
//! Network client for the multi-player aggregator. Each connected tracker
//! server gets one [`MonitorSlot`] and one background thread spawned by 
//! [`spawn_client`].
//! 
//! ## Thread model
//! 
//! [`spawn_client`] spawns an **outer reconnect loop** thread. On each
//! successful TCP connection it starts an inner **writer thread** that drains
//! `texture_request_queue` every 50 ms, while the other thread's **reader loop**
//! receives [`ServerMessage::State`] and [`ServerMessage::Textures`] messages.
//! 
//! When the connection drops:
//! 1. The reader loop exits and sets `state` to `None` (shown as "Disconnected
//!    in the UI").
//! 2. `connected` is set to `false`, signalling the writer thread to stop.
//! 3. The writer thread is joined.
//! 4. The outer loop sleeps for 3 seconds then reconnects.
//! 
//! This mirrors the single-player client in the main tracker binary but is
//! self-contained here so the aggregator has no dependency on that crate.

use fire_red_states::{GameState, ServerMessage, ClientMessage, send_message, recv_message};
use std::collections::{VecDeque, HashSet};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Pending texture
// ---------------------------------------------------------------------------

/// A sprite received from the server that has been decompressed and is waiting
/// to be uploaded to the GPU by the GUI thread.
/// 
/// Decompression happens on the network thread; the GUI thread only needs to
/// call [`egui::Context::load_texture`] and store the resulting handle.
pub struct PendingTexture {
    /// National pokedex number
    pub species: u16,
    /// `true` if this is the shiny palette
    pub shiny: bool,
    /// Decompressed RGBA pixel data (width x height x 4 bytes)
    pub pixels: Vec<u8>, // decompressed RGBA
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
}

// ---------------------------------------------------------------------------
// Monitor slot
// ---------------------------------------------------------------------------

/// All shared state for one connected tracker server.
/// 
/// A [`MonitorSlot`] is created for each server address before the GUI starts.
/// [`spawn_client`] is then called with clones of the inner `Arc`s to wire up
/// the background network thread.The GUI reads from these arcs each frame.
pub struct MonitorSlot {
    /// Display label shown as the column heading (e.g. `"Player 1"`)
    pub label: Arc<Mutex<String>>,
    /// Server address string (stored for diagnostics; prefixed `_` because the
    /// network thread captures `addr` by value instead of reading this field).
    pub _addr: String,
    /// Most recent [`GameState`] received from the server, or `None` while 
    /// disconnected
    pub state: Arc<Mutex<Option<GameState>>>,
    /// Sprites received from the server that have not yet been uploaded to the GPU.
    /// Drained by the GUI thread at the start of each frame.
    pub pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    /// Set of species IDs for which a texture request has already been sent,
    /// preventing dulicate requests across reconnects.
    pub known_species: Arc<Mutex<HashSet<u16>>>,
    /// Outbound texture request batches produced by the GUI and consumed by the
    /// network writer thread every 50 ms.
    pub texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>>,
}

impl MonitorSlot {
    /// Creates a new [`MonitorSlot`] for the server at `addr`
    /// 
    /// All shared state is initialized to empty / `None`. Call [`spawn_client`]
    /// with clones of the inner arcs to start the background network thread.
    /// 
    /// # Arguments
    /// * `index` - Zero-based slot index; used to generate the display label
    ///             (`"Player 1"`, `"Player 2"`, ...).
    /// * `addr`  - TCP address of the tracker server (e.g. "192.168.1.10:7878").
    pub fn new(index: usize, addr: String) -> Self {
        Self {
            label: Arc::new(Mutex::new(format!("Player {}", index + 1))),
            _addr: addr,
            state: Arc::new(Mutex::new(None)),
            pending_textures: Arc::new(Mutex::new(Vec::new())),
            known_species: Arc::new(Mutex::new(HashSet::new())),
            texture_request_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decompresses a zlib-compressed sprite pixel blob into raw RGBA bytes.
/// 
/// Called on the network thread immediately after receiving a
/// [`ServerMessage::Textures`] packet, before the data is placed in
/// `pending_textures`. On failure (turncated data, bad checksum) an empty
/// `Vec` is returned so the texture pipeline can continue without panicking.
/// 
/// # Arguments
/// * `data`    - Zlib-compressed bytes as sent by the server.
fn decompress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).unwrap_or(0);
    out
}

// ---------------------------------------------------------------------------
// Client thread
// ---------------------------------------------------------------------------

/// Spawns a background thread that maintains a TCP connection to one tracker
/// server and keeps the shared state arcs up to date.
/// 
/// The spawned thread runs an outer reconnect loop that retries every 3 seconds
/// on connection failure. Inside each successful connection two activities run 
/// concurrently:
/// 
/// - **Writer thread** - flushes `texture_request_queue` to the server as
///   [`ClientMessage::RequestTextures`] message every 50 ms. All pending
///   batches are merged and deduplicated before sending.
/// - **Reader loop** (on the spawned thread itself) =  receives [`ServerMessage::State`]
///   and [`ServerMessage::Textures`] messages and updates teh shared arcs 
///   accordingly.
/// 
/// On diconnect, `state` is set to `None` so the UI can show a "Disconnected"
/// warning, the writer thread is signalled and joined, and the outer loop
/// sleeps before retrying.
/// 
/// # Arguments
/// * `addr`                  - Server address (e.g. "192.168.1.1:1234").
/// * `state`                 - Shared game state written on every received `State` message.
/// * `pending_textures`      - Decompressed sprites queued for GPU upload.
/// * `known_species`         - Species IDs already requested / received; prevents re-requests
/// * `texture_request_queue` - Batches of species IDs the GUI wants textures for.
pub fn spawn_client(
    addr: String,
    state: Arc<Mutex<Option<GameState>>>,
    pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    known_species: Arc<Mutex<HashSet<u16>>>,
    texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>>,
    label: Arc<Mutex<String>>,
) {
    std::thread::spawn(move || loop {
        println!("Connecting to monitor at {}...", addr);
        match TcpStream::connect(&addr) {
            Ok(stream) => {
                println!("Connected to {}", addr);

                let mut write_stream = match stream.try_clone() {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Failed to clone stream: {}", e);
                        break;
                    }
                };
                let mut read_stream = stream;

                let connected = Arc::new(AtomicBool::new(true));
                let connected_writer = connected.clone();
                let writer_queue = texture_request_queue.clone();

                // ── Writer thread: drains texture request queue ──────────────
                let writer = std::thread::spawn(move || {
                    while connected_writer.load(Ordering::SeqCst) {
                        let batch = {
                            let mut q = writer_queue.lock().unwrap_or_else(|e| e.into_inner());
                            let mut all: Vec<u16> = q.drain(..).flatten().collect();
                            all.sort();
                            all.dedup();
                            all
                        };

                        if !batch.is_empty() {
                            if send_message(
                                &mut write_stream,
                                &ClientMessage::RequestTextures(batch),
                            )
                            .is_err()
                            {
                                break;
                            }
                        }

                        std::thread::sleep(Duration::from_millis(50));
                    }
                });

                // ── Reader loop: receives State + Textures ───────────────────
                loop {
                    match recv_message::<ServerMessage>(&mut read_stream) {
                        Ok(ServerMessage::State(gs)) => {
                            *label.lock().unwrap_or_else(|e| e.into_inner()) = gs.player_name.clone();
                            *state.lock().unwrap_or_else(|e| e.into_inner()) = Some(gs);                            
                        }
                        Ok(ServerMessage::Textures(sprites)) => {
                            let mut pending =
                                pending_textures.lock().unwrap_or_else(|e| e.into_inner());
                            let mut known =
                                known_species.lock().unwrap_or_else(|e| e.into_inner());
                            for sprite in sprites {
                                known.insert(sprite.species);
                                pending.push(PendingTexture {
                                    species: sprite.species,
                                    shiny: sprite.shiny,
                                    pixels: decompress_pixels(&sprite.pixels),
                                    width: sprite.width,
                                    height: sprite.height,
                                });
                            }
                        }
                        Err(e) => {
                            eprintln!("Lost connection to {}: {}", addr, e);
                            *state.lock().unwrap_or_else(|e| e.into_inner()) = None;
                            break;
                        }
                    }
                }

                connected.store(false, Ordering::SeqCst);
                let _ = writer.join();
            }
            Err(e) => {
                eprintln!("Failed to connect to {}: {}", addr, e);
                *state.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    });
}
