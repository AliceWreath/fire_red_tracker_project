//! Shared slot state for direct-mode connections.

use fire_red_states::{BagPockets, ClientMessage, GameState};
use image::ImageEncoder;
use image::codecs::png::PngEncoder;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// Shared list of active game slots.
pub type SharedSlots = Arc<Mutex<Vec<Arc<MonitorSlot>>>>;

// ---------------------------------------------------------------------------
// Sprite cache
// ---------------------------------------------------------------------------

/// PNG-encoded sprite cache shared between the game-polling thread and the HTTP
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

/// All shared state for one active RetroArch game slot.
///
/// A [`MonitorSlot`] is created for each RetroArch host before the game loop
/// starts. The game-polling threads write into the inner `Arc`s; the GUI and
/// web layer read from them each frame / request.
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
    /// Commands queued by the web server to be dispatched to the slot's game loop.
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
    /// `"host:port"` of the RetroArch instance this slot is polling.
    /// `None` only if the slot was constructed without a host (uncommon).
    /// Read-only after construction.
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
    /// to exit.  Set by `DirectConnector::disconnect`.
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
    /// * `addr`        - Display address string (e.g. `"direct:<host>"`).
    /// * `db_path`     - Optional path to this player's SQLite nuzlocke database.
    /// * `direct_host` - `Some("host:port")` for the RetroArch instance to poll; `None` if unset.
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


