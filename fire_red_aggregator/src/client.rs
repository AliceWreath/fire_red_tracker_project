//! Aggregator tracker connection handler.
//!
//! Each tracker that connects to the aggregator gets one [`MonitorSlot`] (reused
//! on reconnect if available) and one call to [`handle_tracker_connection`].
//!
//! ## Thread model
//!
//! [`handle_tracker_connection`] starts an inner **writer thread** that drains
//! `texture_request_queue` and `command_queue` every 50 ms, while the calling
//! thread's **reader loop** receives [`ServerMessage::State`],
//! [`ServerMessage::Textures`], and [`ServerMessage::RunChanged`] messages.
//!
//! When the connection drops the reader loop exits, sets `state` to `None`, and
//! returns. The caller (TCP listener loop) can then accept the next connection
//! and reuse the same slot.

use fire_red_states::{
    BagPockets, ClientMessage, GameState, LockOrRecover, ServerMessage, recv_message, send_message,
};
use image::ImageEncoder;
use image::codecs::png::PngEncoder;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared list of tracker slots, grown dynamically as trackers connect.
pub type SharedSlots = Arc<Mutex<Vec<Arc<MonitorSlot>>>>;

// ---------------------------------------------------------------------------
// Sprite cache
// ---------------------------------------------------------------------------

/// PNG-encoded sprite cache shared between the TCP reader thread and the HTTP
/// sprite endpoint. Keyed by `(species, is_shiny)`.
pub type PngSpriteCache = Arc<Mutex<HashMap<(u16, bool), Vec<u8>>>>;

/// Encodes raw RGBA pixels directly to PNG bytes using the PNG codec.
pub fn encode_png(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if pixels.len() < (width as usize) * (height as usize) * 4 {
        return None;
    }
    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(buf)
}

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
    /// Which sprite image this packet carries (front or back).
    pub variant: fire_red_states::SpriteVariant,
    /// Decompressed RGBA pixel data (width x height x 4 bytes)
    pub pixels: Vec<u8>,
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
/// the background network thread. The GUI reads from these arcs each frame.
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
    /// preventing duplicate requests across reconnects.
    pub known_species: Arc<Mutex<HashSet<u16>>>,
    /// Outbound texture request batches produced by the GUI and consumed by the
    /// network writer thread every 50 ms.
    pub texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>>,
    /// Connection string used to open `db` (kept for diagnostics).
    pub _db_path: Option<String>,
    /// Read-only handle to this player's nuzlocke database.
    /// Opened lazily on first successful access; `None` if no path was given or
    /// the file does not yet exist.
    pub db: Option<fire_red_database::DbReader>,
    /// Optional sprite cache set by web mode. When populated, the `spawn_client`
    /// reader thread encodes received sprites directly into this cache so the
    /// HTTP sprite endpoint can serve them without a separate drain step.
    pub sprite_cache: Arc<Mutex<Option<PngSpriteCache>>>,
    /// Commands queued by the web server to be forwarded to the tracker over TCP.
    pub command_queue: Arc<Mutex<VecDeque<ClientMessage>>>,
    /// Injection events queued by API endpoints and drained by BroadcastLoop each
    /// tick into the WebSocket JSON so alerts.html can show toasts.
    pub injection_events: Arc<Mutex<VecDeque<serde_json::Value>>>,
    /// Set to `true` when the tracker confirms a run change (EndRun / NewRun),
    /// so the BroadcastLoop can mark the DB reader dirty and re-sync.
    pub run_changed: Arc<AtomicBool>,
    /// Latest PC box snapshot received from the tracker (~5 s cadence).
    pub box_data: Arc<Mutex<Vec<fire_red_states::BoxEntry>>>,
    /// Latest bag pockets snapshot received from the tracker (~2 s cadence).
    pub bag_data: Arc<Mutex<Option<BagPockets>>>,
    /// `"host:port"` of the RetroArch instance this slot is polling in direct
    /// mode, or `None` for tracker-TCP slots.  Read-only after construction.
    pub direct_host: Option<String>,
    /// Raw ROM bytes used by the direct-mode sprite loader.  Replaced in-place
    /// when the caller triggers a ROM force-refresh via the API.
    pub rom_bytes: Arc<Mutex<Vec<u8>>>,
    /// Identity string of the ROM currently loaded into `rom_bytes`, formatted
    /// as `"<title>/<code>/<version>"` (e.g. `"POKEMON FIRE RED/BPRE/1"`).
    /// Empty until the first ROM is loaded.  Used by the refresh endpoint to
    /// detect whether the game in RetroArch changed between refreshes.
    pub rom_identity: Arc<Mutex<String>>,
    /// Handle to the game loop's live encounter-table buffer for the current
    /// map area.  `Some` only for direct-mode slots; wired up in `direct.rs`
    /// after the game loop is started.  Reset to default by the refresh
    /// endpoint so stale encounter tables from the old ROM are evicted.
    pub game_encounters: Arc<Mutex<Option<Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>>>>,
    /// Signals the game loop, sprite loader, and bridge threads for this slot
    /// to exit.  Set by `DirectConnector::disconnect`; always `false` for
    /// tracker-TCP slots (those stop when the connection drops).
    pub shutdown: Arc<AtomicBool>,
}

impl MonitorSlot {
    /// Creates a new [`MonitorSlot`] for the server at `addr`.
    ///
    /// All shared state is initialized to empty / `None`. Call [`spawn_client`]
    /// with clones of the inner arcs to start the background network thread.
    ///
    /// # Arguments
    /// * `index`       - Zero-based slot index; used to generate the display label.
    /// * `addr`        - TCP address of the tracker server (or `"direct:<host>"` in direct mode).
    /// * `db_path`     - Optional path to this player's SQLite nuzlocke database.
    /// * `direct_host` - `Some("host:port")` for direct-mode slots; `None` for tracker-TCP slots.
    pub fn new(
        index: usize,
        addr: String,
        db_path: Option<String>,
        direct_host: Option<String>,
    ) -> Self {
        let db = db_path
            .as_deref()
            .and_then(fire_red_database::DbReader::open);
        Self {
            label: Arc::new(Mutex::new(format!("Player {}", index + 1))),
            _addr: addr,
            state: Arc::new(Mutex::new(None)),
            pending_textures: Arc::new(Mutex::new(Vec::new())),
            known_species: Arc::new(Mutex::new(HashSet::new())),
            texture_request_queue: Arc::new(Mutex::new(VecDeque::new())),
            _db_path: db_path,
            db,
            sprite_cache: Arc::new(Mutex::new(None)),
            command_queue: Arc::new(Mutex::new(VecDeque::new())),
            injection_events: Arc::new(Mutex::new(VecDeque::new())),
            run_changed: Arc::new(AtomicBool::new(false)),
            box_data: Arc::new(Mutex::new(Vec::new())),
            bag_data: Arc::new(Mutex::new(None)),
            direct_host,
            rom_bytes: Arc::new(Mutex::new(Vec::new())),
            rom_identity: Arc::new(Mutex::new(String::new())),
            game_encounters: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(AtomicBool::new(false)),
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
/// `pending_textures`. On failure (truncated data, bad checksum) an empty
/// `Vec` is returned so the texture pipeline can continue without panicking.
///
/// # Arguments
/// * `data`    - Zlib-compressed bytes as sent by the server.
fn decompress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    if let Err(e) = decoder.read_to_end(&mut out) {
        tracing::warn!("sprite decompression failed: {e}");
    }
    out
}

// ---------------------------------------------------------------------------
// Client thread
// ---------------------------------------------------------------------------

/// Handles a live TCP connection from a tracker.
///
/// Blocks until the connection is lost, then returns. The caller is responsible
/// for accepting the next connection and calling this function again.
///
/// # Arguments
/// * `stream`                - Connected TCP stream from the accepted tracker.
/// * `state`                 - Shared game state updated on every `State` message.
/// * `pending_textures`      - Decompressed sprites queued for GPU upload.
/// * `known_species`         - Species IDs already requested; prevents duplicates.
/// * `texture_request_queue` - Texture request batches queued by the GUI.
/// * `label`                 - Display label updated from the tracker's player name.
/// * `sprite_cache`          - Optional shared PNG cache for web/overlay mode.
/// * `command_queue`         - `EndRun`/`NewRun` commands forwarded to the tracker.
/// * `run_changed`           - Set when the tracker confirms a run change.
/// * `box_data`              - Updated when a BoxData message arrives (~5 s cadence).
/// * `bag_data`              - Updated when a Bag message arrives (~2 s cadence).
#[allow(clippy::too_many_arguments)]
pub fn handle_tracker_connection(
    stream: TcpStream,
    state: Arc<Mutex<Option<GameState>>>,
    pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    known_species: Arc<Mutex<HashSet<u16>>>,
    texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>>,
    label: Arc<Mutex<String>>,
    sprite_cache: Arc<Mutex<Option<PngSpriteCache>>>,
    command_queue: Arc<Mutex<VecDeque<ClientMessage>>>,
    run_changed: Arc<AtomicBool>,
    box_data: Arc<Mutex<Vec<fire_red_states::BoxEntry>>>,
    bag_data: Arc<Mutex<Option<BagPockets>>>,
) {
    let mut write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to clone stream: {}", e);
            return;
        }
    };
    let mut read_stream = stream;

    let _ = send_message(
        &mut write_stream,
        &ClientMessage::Hello(env!("CARGO_PKG_VERSION").to_string()),
    );

    let connected = Arc::new(AtomicBool::new(true));
    let connected_writer = connected.clone();
    let writer_queue = texture_request_queue.clone();
    let writer_cmds = command_queue.clone();

    // ── Writer thread: drains texture requests and commands ──────────────────
    let writer = std::thread::spawn(move || {
        while connected_writer.load(Ordering::Acquire) {
            let cmds: Vec<ClientMessage> = {
                let mut q = writer_cmds.lock_or_recover();
                q.drain(..).collect()
            };
            for cmd in cmds {
                if send_message(&mut write_stream, &cmd).is_err() {
                    return;
                }
            }

            let batch = {
                let mut q = writer_queue.lock_or_recover();
                let mut all: Vec<u16> = q.drain(..).flatten().collect();
                all.sort();
                all.dedup();
                all
            };
            if !batch.is_empty()
                && send_message(&mut write_stream, &ClientMessage::RequestTextures(batch)).is_err()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    // ── Reader loop: receives State + Textures ───────────────────────────────
    loop {
        match recv_message::<ServerMessage>(&mut read_stream) {
            Ok(ServerMessage::State(gs)) => {
                *label.lock_or_recover() = gs.player_name.clone();
                *state.lock_or_recover() = Some(*gs);
            }
            Ok(ServerMessage::Textures(sprites)) => {
                let maybe_cache: Option<PngSpriteCache> = sprite_cache.lock_or_recover().clone();
                let mut pending = pending_textures.lock_or_recover();
                let mut known = known_species.lock_or_recover();
                for sprite in sprites {
                    known.insert(sprite.species);
                    let pixels = decompress_pixels(&sprite.pixels);
                    if let Some(ref cache) = maybe_cache {
                        let key = (sprite.species, sprite.shiny);
                        let mut c = cache.lock_or_recover();
                        if let std::collections::hash_map::Entry::Vacant(e) = c.entry(key)
                            && let Some(png) = encode_png(&pixels, sprite.width, sprite.height)
                        {
                            e.insert(png);
                        }
                    }
                    pending.push(PendingTexture {
                        species: sprite.species,
                        shiny: sprite.shiny,
                        variant: sprite.variant.clone(),
                        pixels,
                        width: sprite.width,
                        height: sprite.height,
                    });
                }
            }
            Ok(ServerMessage::RunChanged(_)) => {
                run_changed.store(true, Ordering::Release);
            }
            Ok(ServerMessage::BoxData(entries)) => {
                *box_data.lock_or_recover() = entries;
            }
            Ok(ServerMessage::Bag(pockets)) => {
                *bag_data.lock_or_recover() = Some(pockets);
            }
            Err(_) => {
                *state.lock_or_recover() = None;
                break;
            }
        }
    }

    connected.store(false, Ordering::Release);
    let _ = writer.join();
}
