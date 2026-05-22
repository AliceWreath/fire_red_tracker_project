//! # Fire Red Tracker
//!
//! A real-time Pokemon FireRed party and encounter monitor with an egui GUI.
//!
//! ## Modes
//!
//! The application supports three operating modes selected via command-line arguments:
//!
//! - **Standalone** - reads the ROM and game memory locally, renders the GUI.
//! - **Server* - like standalone but also accept TCP client connections and
//!     streams [`GameState`] updates + sprite data to them. Runs Headless.
//! - **Client** - connects to a server over TCP, receives state and sprites,
//!     and renders the GUI without needing the ROM locally.
//!
//! ## Usage
//!
//! ```text
//! tracker /path/to/file.gba [--clean]                     # standalone
//! tracker /path/to/file.gba --server [port]               # server (default port 7878)
//! tracker --client [host] [port]                          # client (default 127.0.0.1:7878)
//! ```
//!
use colored::Colorize;
use fire_red_loop::*;
use fire_red_party_monitor::get_is_clean;
use fire_red_states::*;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------
// Window / image size constants
// ---------------------------------------------------------

/// Holds the default target window size for party, in logical pixels.
const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);

/// Holds the size for the pokemon images, in logical pixels
const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// Holds the default target window size for encounters, in logical pixels
const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);

/// Holds the size for encounter images, in logical pixels
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

// -----------------------------------------------------------
// Global state
// -----------------------------------------------------------

/// How often (in seconds) the party list is force-refreshed even when the
/// party size has not changed, to catch in-place changes.
static FORCE_PARTY_CHECK_TIME_IN_SECS: u64 = 5;

/// Handle to the main game-polling thread, kept alive for the process lifetime.
static MAIN_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Handle to the client network thread (client mode only)
static CLIENT_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Handle to the TCP listener thread (server mode only)
static SERVER_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Set to 'false' by teh Ctrl-C handler to trigger a clean shutdown in server mode.
static RUNNING: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// Sprite compression helpers
// ---------------------------------------------------------------------------

/// Compresses raw RGBA pixel data using zlib (fast preset).
///
/// Sprites can be several KB uncompressed; compression reduces the amount of
/// data sent over the TCP connection when the server streams sprites to clients.
///
/// # Arguments
/// * `data` - Raw RGBA bytes (width * height * 4).
///
/// # Returns
/// Zlib-compressed byte vector.
fn compress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::{Compression, write::ZlibEncoder};
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// Decompresses zlib-compressed pixel data back to raw RGBA bytes.
///
/// Called on the client after receiving a [`SpriteData`] packet from the server.
/// On failure an empty 'Vec' is returned so the texture pipeline can still run
/// without panicking.
///
/// # Arguments
/// * `data` - Zlib-compressed bytes as produced by [`compress_pixels`]
///
/// # Returns
/// Decompressed RGBA bytes, or an empty 'Vec' on error.
fn decompress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::read::ZlibDecoder;
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).unwrap_or(0);
    out
}

// ---------------------------------------------------------------------------
// Server-side sprite cache
// ---------------------------------------------------------------------------

/// Extracts a pokemon sprite from teh ROM, compresses it, and returns a
/// [`SpriteData`] ready to send to a client.
///
/// Returns `None` if the species index is invalid or the sprite cannot be
/// decoded from the ROM.
///
/// # Arguments
/// * `rom`         - Full ROM byte slice (must already be loaded in memory).
/// * `species`     - National Pokedex number (1 - 386 for FireRed)
/// * `shiny`       - `true` to return the alternate shiny palette sprite.
fn build_sprite_data(rom: &[u8], species: u16, shiny: bool) -> Option<SpriteData> {
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, shiny).ok()?;
    let width = img.width();
    let height = img.height();
    let pixels = compress_pixels(&img.into_raw());
    Some(SpriteData {
        species,
        shiny,
        pixels,
        width,
        height,
    })
}

// ---------------------------------------------------------------------------
// GUI state
// ---------------------------------------------------------------------------

/// A sprite that has been received from the server and is waiting to be
/// uploaded to the GPU as an egui texture.
///
/// Decompression happens on teh network thread; the GUI thread only needs to
/// call [`egui::Context::load_texture`] and store the handle.
struct PendingTexture {
    /// National Pokedex Number
    species: u16,
    /// Whether this is the shiny variant
    shiny: bool,
    /// Decompressed RGBA pixel data (width * height * 4 bytes).0
    pixels: Vec<u8>,
    /// Image width in pixels
    width: u32,
    /// Image height in pixels
    height: u32,
}

/// Top-level application state passed to [`eframe`]
///
/// Holds all shared data needed to drive both the party panel (main window)
/// and the encounters panel (child viewport), as well as the client-mode
/// texture pipeline.
struct WindowInfo {
    /// Current party pokemon, updated by teh game-polling or network thread.
    party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    /// Current area wild encounter table, updated by the game-polling or network thread.
    encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    /// Cache of GPU texture handles, keyed by `"pokemon_{species}_{normal|shiny"`.
    textures: HashMap<String, egui::TextureHandle>,
    /// Whether the encounters child window is currently visible.
    encounters_open: bool,
    /// Sprites received from teh server that have not yet been uploaded to the GPU.
    /// Drained at the start of each frame.
    pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    /// Set of species IDs for which a texture request has already been sent to
    /// the server, preventing duplicate requests.
    known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
    /// Queue of texture request batches produces by the GUI and consumed by the
    /// network writer thread. `None` in standalone/server mode (textures are
    /// loaded from teh ROM directly without oging through the network).
    texture_request_queue: Option<Arc<Mutex<VecDeque<Vec<u16>>>>>,
}

impl WindowInfo {
    /// Creates a new [`WindowInfo`] from the shared arcs produced in `main`.
    ///
    /// # Arguments
    /// * `_cc`                         - eframe creation context (unused; reserved
    ///                                     for future font/style initialization).
    /// * `party_list`                  - Shared party pokemon list.
    /// * `encounter_list`              - Shared wild encounter table
    /// * `pending_textures`            - Shared pipeline for textures received from the server.
    /// * `known_species`               - Set of species already requested / received.
    /// * `texture_request_queue`       - Queue for outbound texture requests; `None` in
    ///                                     standalone mode.
    fn new(
        _cc: &eframe::CreationContext<'_>,
        party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
        encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
        pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
        known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
        texture_request_queue: Option<Arc<Mutex<VecDeque<Vec<u16>>>>>,
    ) -> Self {
        Self {
            party_list,
            encounter_list,
            textures: HashMap::new(),
            encounters_open: true,
            pending_textures,
            known_species,
            texture_request_queue,
        }
    }
}

impl eframe::App for WindowInfo {
    /// Intentionally empty - all rendering is handled in [`update`]
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // intentionally empty - rendering is done in update()
    }

    /// Main per-frame callback called by eframe on every repaint.
    ///
    /// Responsibilities (in order):
    /// 1. **Drain pending textures** - upload any sprites received from the
    ///     server since the last frame.
    /// 2. **Request / load missing textures** - for every species visibile in the
    ///     party or encounter list that has no cached texture, either load it
    ///     directly from ROM (standalone/server mode) or enqueue a request
    ///     to the network thread (client mode).
    /// 3. **Draw the party panel** in the central panel.
    /// 4. **Draw the encounters window** as an immediate child viewport.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request a repaint on the next frame so the UI stays live even when
        // no user input occurs (game state changes continuously).
        ctx.request_repaint();

        // ── 1. Drain textures received from server (client mode) ────────────
        {
            let mut pending = self
                .pending_textures
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            for pt in pending.drain(..) {
                let key = format!(
                    "pokemon_{}_{}",
                    pt.species,
                    if pt.shiny { "shiny" } else { "normal" }
                );
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [pt.width as usize, pt.height as usize],
                    &pt.pixels,
                );
                let handle = ctx.load_texture(&key, color_image, egui::TextureOptions::NEAREST);
                self.textures.insert(key, handle);
            }
        }

        // ── 2. Load / request missing textures ──────────────────────────────
        {
            let list = self
                .party_list
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let encounter_list = self
                .encounter_list
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            let mut needed_for_request: Vec<u16> = Vec::new();

            let missing: Vec<(u16, u32, u32)> = list
                .iter()
                .map(|p| {
                    (
                        p.box_mon.secure.growth.species,
                        p.box_mon.personality,
                        p.box_mon.ot_id,
                    )
                })
                .filter(|(species, personality, ot_id)| {
                    let key = format!(
                        "pokemon_{}_{}",
                        species,
                        if is_shiny(*personality, *ot_id) {
                            "shiny"
                        } else {
                            "normal"
                        }
                    );
                    !self.textures.contains_key(&key)
                })
                .collect();

            let encounter_iters = encounter_list
                .land_mon_encounters
                .wild_pokemon_list
                .iter()
                .chain(encounter_list.water_mon_encounters.wild_pokemon_list.iter())
                .chain(
                    encounter_list
                        .rock_smash_encounters
                        .wild_pokemon_list
                        .iter(),
                )
                .chain(encounter_list.fishing_encounters.wild_pokemon_list.iter());

            for wild_pokemon in encounter_iters {
                if wild_pokemon.species == 0 || wild_pokemon.species > 386 {
                    continue;
                }
                let key = format!("pokemon_{}_normal", wild_pokemon.species);
                if !self.textures.contains_key(&key) {
                    if self.texture_request_queue.is_some() {
                        // Client mode — request from server
                        let known = self.known_species.lock().unwrap_or_else(|e| e.into_inner());
                        if !known.contains(&wild_pokemon.species) {
                            needed_for_request.push(wild_pokemon.species);
                        }
                    } else {
                        // Standalone / server mode — load from ROM directly
                        let texture = load_texture_normal(
                            ctx,
                            fire_red_rom_buffer::get_rom(),
                            wild_pokemon.species,
                        )
                        .unwrap_or_else(|_| {
                            eprintln!(
                                "Failed to load texture for species {}. Using placeholder.",
                                wild_pokemon.species
                            );
                            make_placeholder(ctx, wild_pokemon.species)
                        });
                        self.textures.insert(key, texture);
                    }
                }
            }

            drop(encounter_list);

            for (species, personality, ot_id) in missing {
                if species == 0 || species > 386 {
                    continue;
                }
                let key = format!(
                    "pokemon_{}_{}",
                    species,
                    if is_shiny(personality, ot_id) {
                        "shiny"
                    } else {
                        "normal"
                    }
                );
                if self.texture_request_queue.is_some() {
                    let known = self.known_species.lock().unwrap_or_else(|e| e.into_inner());
                    if !known.contains(&species) {
                        needed_for_request.push(species);
                    }
                } else {
                    let texture = load_texture(
                        ctx,
                        fire_red_rom_buffer::get_rom(),
                        species,
                        personality,
                        ot_id,
                    )
                    .unwrap_or_else(|_| {
                        eprintln!(
                            "Failed to load texture for species {}. Using placeholder.",
                            species
                        );
                        make_placeholder(ctx, species)
                    });
                    self.textures.insert(key, texture);
                }
            }

            // Push any new requests into the shared queue
            if !needed_for_request.is_empty() {
                needed_for_request.sort();
                needed_for_request.dedup();
                if let Some(queue) = &self.texture_request_queue {
                    queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_back(needed_for_request);
                }
            }
        }

        #[allow(deprecated)]
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_ui(ui, ctx);
        });

        // separate independent window
        if self.encounters_open {
            let encounter_list = self.encounter_list.clone();
            let textures: &HashMap<String, egui::TextureHandle> = &self.textures;

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("encounters_window"),
                egui::ViewportBuilder::default()
                    .with_title("Encounters")
                    .with_inner_size([ENCOUNTER_WINDOW.0, ENCOUNTER_WINDOW.1]),
                move |ctx, _class| {
                    #[allow(deprecated)]
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let encounters =
                                encounter_list.lock().unwrap_or_else(|e| e.into_inner());
                            ui.heading("Land Encounters");
                            ui.horizontal(|ui| {
                                for wild_pokemon in
                                    encounters.land_mon_encounters.wild_pokemon_list.iter()
                                {
                                    let key = format!("pokemon_{}_normal", wild_pokemon.species);
                                    if let Some(texture) = textures.get(&key) {
                                        ui.add(egui::Image::new(texture).fit_to_exact_size(
                                            egui::vec2(
                                                ENCOUNTER_IMAGE_SIZE.0,
                                                ENCOUNTER_IMAGE_SIZE.1,
                                            ),
                                        ));
                                    }
                                }
                            });
                            ui.separator();
                            ui.heading("Water Encounters");
                            ui.horizontal(|ui| {
                                for wild_pokemon in
                                    encounters.water_mon_encounters.wild_pokemon_list.iter()
                                {
                                    let key = format!("pokemon_{}_normal", wild_pokemon.species);
                                    if let Some(texture) = textures.get(&key) {
                                        ui.add(egui::Image::new(texture).fit_to_exact_size(
                                            egui::vec2(
                                                ENCOUNTER_IMAGE_SIZE.0,
                                                ENCOUNTER_IMAGE_SIZE.1,
                                            ),
                                        ));
                                    }
                                }
                                for wild_pokemon in
                                    encounters.fishing_encounters.wild_pokemon_list.iter()
                                {
                                    let key = format!("pokemon_{}_normal", wild_pokemon.species);
                                    if let Some(texture) = textures.get(&key) {
                                        ui.add(egui::Image::new(texture).fit_to_exact_size(
                                            egui::vec2(
                                                ENCOUNTER_IMAGE_SIZE.0,
                                                ENCOUNTER_IMAGE_SIZE.1,
                                            ),
                                        ));
                                    }
                                }
                            });
                        });
                    });
                },
            );
        }
    }
}

impl WindowInfo {
    /// Draws the party panel into `ui`.
    ///
    /// For each pokemon in the party this renders:
    /// - Its sprite (shiny variant when applicable)
    /// - Nickname and level.
    /// - Current / map HP, color-coded by percentage.
    ///     - **Red** below 30%
    ///     - **Yellow** below 80%
    ///     - **White** otherwise.
    /// - Met location.
    /// - Ability (only whne running in "clean" ROM mode, i.e. `--clean` flag).
    fn draw_ui(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.heading("Party");

        let list = self.party_list.lock().unwrap_or_else(|e| e.into_inner());
        for (idx, pokemon) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                let species = pokemon.box_mon.secure.growth.species;
                let personality = pokemon.box_mon.personality;
                let ot_id = pokemon.box_mon.ot_id;
                let key = format!(
                    "pokemon_{}_{}",
                    species,
                    if is_shiny(personality, ot_id) {
                        "shiny"
                    } else {
                        "normal"
                    }
                );

                if let Some(texture) = self.textures.get(&key) {
                    ui.add(
                        egui::Image::new(texture)
                            .fit_to_exact_size(egui::vec2(PARTY_IMAGE_SIZE.0, PARTY_IMAGE_SIZE.1)),
                    );
                }

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(pokemon.get_nickname_string())
                                .strong()
                                .size(18.0)
                                .color(egui::Color32::WHITE),
                        );
                        ui.label(format!("Lvl: {}", pokemon.level));
                        ui.label(format!("Exp: {}", pokemon.box_mon.secure.growth.experience));
                    });

                    egui::Grid::new(format!("stats_{}", idx))
                        .min_col_width(80.0)
                        .spacing([10.0, 2.0])
                        .show(ui, |ui| {
                            let color = if (pokemon.hp as f32) < (pokemon.max_hp as f32 * 0.3) {
                                egui::Color32::RED
                            } else if (pokemon.hp as f32) < (pokemon.max_hp as f32 * 0.8) {
                                egui::Color32::YELLOW
                            } else {
                                egui::Color32::WHITE
                            };
                            ui.label(
                                egui::RichText::new(format!("{}/{}", pokemon.hp, pokemon.max_hp))
                                    .strong()
                                    .size(18.0)
                                    .color(color),
                            );
                        });
                    ui.label(format!(
                        "Caught Location : {}",
                        pokemon.box_mon.secure.misc.met_location
                    ));

                    if get_is_clean() {
                        ui.label(format!("Ability: {}", pokemon.box_mon.ability_string));
                    }
                });
            });
            ui.separator();
        }
    }
}

// ---------------------------------------------------------------------------
// Texture helpers
// ---------------------------------------------------------------------------

/// Loads a pokemon sprite from the ROM and uploads it as an egui texture
///
/// The shiny flag is derived automatically from `personality` and `ot_id` using
/// the Gen III shiny formula (see [`is_shiny`])
///
/// # Arguments
/// * `ctx`                 - egui context used to allocate the GPU texture.
/// * `rom`                 - Full ROM byte slice.
/// * `species`             - National Pokedex number.
/// * `personality`         - Pokemon's personality value (PID)
/// * `ot_id`               - Combined original trainer ID (public + secret)
///
/// # Errors
/// Returns an error if the sprite cannot be decoded from the ROM.
pub fn load_texture(
    ctx: &egui::Context,
    rom: &[u8],
    species: u16,
    personality: u32,
    ot_id: u32,
) -> Result<egui::TextureHandle, Box<dyn std::error::Error>> {
    let shiny = is_shiny(personality, ot_id);
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, shiny)?;
    let size = [img.width() as usize, img.height() as usize];
    let pixels: Vec<u8> = img.into_raw();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Ok(ctx.load_texture(
        format!(
            "pokemon_{}_{}",
            species,
            if shiny { "shiny" } else { "normal" }
        ),
        color_image,
        egui::TextureOptions::NEAREST,
    ))
}

/// Loads the non-shiny sprite for a species and uploads it as an egui texture
///
/// Convenience wrapper around [`load_texture`] used for wild encounter sprites,
/// which are always shown in their normal palette regardless of hidden shiny
/// values in the encounter table.
///
/// # Arguments
/// * `ctx`                 - egui context used to allocate the GPU texture
/// * `rom                  - Full ROM byte slice.
/// * `species`             - National pokedex number.
pub fn load_texture_normal(
    ctx: &egui::Context,
    rom: &[u8],
    species: u16,
) -> Result<egui::TextureHandle, Box<dyn std::error::Error>> {
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, false)?;
    let size = [img.width() as usize, img.height() as usize];
    let pixels: Vec<u8> = img.into_raw();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Ok(ctx.load_texture(
        format!("pokemon_{}_normal", species),
        color_image,
        egui::TextureOptions::NEAREST,
    ))
}

/// Creates a solid red placeholder texture for species whose sprites could not be loaded.
///
/// Prevents a panic when a sprite is unavailable (e.g. invalid species index)
/// while making missing sprites visually obvious.
///
/// # Arguments
/// * `ctx`             - egui context used to allocate the GPU texture.
/// * `species`         - National pokdex number, used only to key the texture cache.
fn make_placeholder(ctx: &egui::Context, species: u16) -> egui::TextureHandle {
    let size = [PARTY_IMAGE_SIZE.0 as usize, PARTY_IMAGE_SIZE.1 as usize];
    let pixels = vec![255u8, 0, 0, 255].repeat(size[0] * size[1]);
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    ctx.load_texture(
        format!("pokemon_{}_placeholder", species),
        color_image,
        egui::TextureOptions::NEAREST,
    )
}

/// Refreshes the shared party list by reading current party data from the game.
///
/// Called whenever the party size changes or when the periodic force-refresh
/// timer fires. Overwrites teh entire list so stale entries are never shown.
///
/// # Arguments
/// * `thread_party` - Shared party list shared with the GUI thread.
fn fill_party_list(thread_party: &Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>) {
    let mut list = thread_party.lock().unwrap_or_else(|e| e.into_inner());
    *list = get_party_members();
}

/// Returns `true` if a pokemon with the given `personality` and `ot_id` is shiny.
///
/// Uses the Gen III shiny determination formula:
/// `(p_high XOR p_low XOR id_high XOR id_low) < 8`
///
/// where `p_high`/`p_low` are the upper/lower 16 bits of teh personality value.
/// and `id_high`/`id_low` are teh upper/lower 16 bits of the combined OT ID.
///
/// # Arguments
/// * `personality`     - pokemon's 32-bit personality value (PID).
/// * `ot_id            - combined 32-bit OT ID (public ID in low 16 bits, secret ID in high 16 bits).
pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p1 = (personality >> 16) as u16;
    let p2 = (personality & 0xFFFF) as u16;
    let id1 = (ot_id >> 16) as u16;
    let id2 = (ot_id & 0xFFFF) as u16;
    (p1 ^ p2 ^ id1 ^ id2) < 8
}

// ---------------------------------------------------------------------------
// Server: handle one connected client (bidirectional)
// ---------------------------------------------------------------------------

/// Manages the full lifecycle of a single TCP client connection in server mode.
///
/// The function spawns two concurrent activities:
///
/// * **Reader thread** - listens for [`ClientMessage::RequestTextures`] packets
///     and responds with [`ServerMessage::Textures`] containing compressed sprite
///     data for the requested species. Both normal and shiny variants are always
///     sent so the client never needs the ROM locally. Results are cached in
///     `sprite_cache` to avoid re-decoding the ROM for repeated requests.
///
/// * **Writer loop** (runs on the calling thread) - broadcasts a
///     [`ServerMessage::State`] snapshot every 100 ms containing the current
///     party and encounter data. The loop exits on any write error (client disconnect).
///
/// # Arguments
/// * `stream`                      - Connected TCP stream.
/// * `server_party`                - Shared reference to the current party data.
/// * `server_encounters`           - Shared reference to the current area encounter data.
/// * `sprite_cache`                - Per-process sprite cache to amortise ROM decoding cost
///                                     across multiple clients.
fn handle_client(
    stream: TcpStream,
    server_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    server_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>>,
) {
    println!("handle_client started");
    println!(
        "Client connected: {}",
        stream
            .peer_addr()
            .map_or_else(|_| "unknown".to_string(), |a| a.to_string())
    );

    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to clone stream: {}", e);
            return;
        }
    };
    let write_stream = Arc::new(Mutex::new(stream));

    // ── Reader thread: handles ClientMessage::RequestTextures ────────────────
    let write_stream_for_reader = write_stream.clone();
    let sprite_cache_for_reader = sprite_cache.clone();
    std::thread::spawn(move || {
        let mut read_stream = read_stream;
        loop {
            match recv_message::<ClientMessage>(&mut read_stream) {
                Ok(ClientMessage::RequestTextures(species_list)) => {
                    let rom = fire_red_rom_buffer::get_rom();
                    let mut sprites: Vec<SpriteData> = Vec::new();

                    for species in species_list {
                        if species == 0 || species > 386 {
                            continue;
                        }
                        // Send both normal and shiny variants so the client
                        // never needs the ROM.
                        for shiny in [false, true] {
                            // Cache key encodes both species and shiny flag
                            let cache_key = (species, shiny);
                            let mut cache = sprite_cache_for_reader
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            if !cache.contains_key(&cache_key) {
                                if let Some(data) = build_sprite_data(rom, species, shiny) {
                                    cache.insert(cache_key, data.clone());
                                    sprites.push(data);
                                }
                            } else {
                                sprites.push(cache[&cache_key].clone());
                            }
                        }
                    }

                    if !sprites.is_empty() {
                        let mut ws = write_stream_for_reader
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if send_message(&mut *ws, &ServerMessage::Textures(sprites)).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    // ── Writer loop: pushes GameState every 100 ms ───────────────────────────
    loop {
        let state = {
            let party = server_party.lock().unwrap_or_else(|e| e.into_inner());
            let encounters = server_encounters.lock().unwrap_or_else(|e| e.into_inner());
            let trainer_name = fire_red_loop::get_trainer_name();
            GameState {
                party: party.clone(),
                encounters: encounters.clone(),
                player_name: trainer_name,
                badge_state: fire_red_badge::read_badge_state(),
            }
        };

        let mut ws = write_stream.lock().unwrap_or_else(|e| e.into_inner());
        if send_message(&mut *ws, &ServerMessage::State(state)).is_err() {
            println!("Client disconnected");
            break;
        }
        drop(ws);

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

/// Entry point.
///
/// Parses command-line arguments to determine the operating [`Mode`], then
/// sets up the appropriate combination of threads and (optionally) launches
/// the egui GUI.
///
/// ## Thread architecture
///
/// | Mode          | Thread created|
/// |---------------|---------------|
/// | Standalone    | game-polling thread + GUI (main thread) |
/// | Server        | game-polling thread + TCP listener thread (headless) |
/// | Client        | network thread (connect -> read/write loop) + GUI (main thread) |
///
/// All inter-thread data is passed via `Arc<Mutex<_>>`
fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mode = match args.get(1).map(|s| s.as_str()) {
        Some("--client") => {
            let host = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| "127.0.0.1".to_string());
            let port = args.get(3).and_then(|p| p.parse().ok()).unwrap_or(7878);
            Mode::Client { host, port }
        }
        Some(_) => match args.get(2).map(|s| s.as_str()) {
            Some("--server") => {
                let port = args.get(3).and_then(|p| p.parse().ok()).unwrap_or(7878);
                Mode::Server { port }
            }
            _ => Mode::Standalone,
        },
        None => {
            eprintln!("Usage:");
            eprintln!(
                "  {} firered.gba [--clean]                    (standalone)",
                args[0]
            );
            eprintln!(
                "  {} firered.gba --server [port]              (default port 7878)",
                args[0]
            );
            eprintln!(
                "  {} --client [host] [port]                    (default 127.0.0.1:7878)",
                args[0]
            );
            return;
        }
    };

    // Shared state between the game-polling / network threads and the GUI.
    let shared_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>> =
        Arc::new(Mutex::new(Vec::new()));
    let shared_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>> = Arc::new(
        Mutex::new(fire_red_pokemon_data::WildPokemonHeader::default()),
    );

    // Sprite cache shared across all clients on the server; avoids decoding
    // the same ROM sprite more than once per process lifetime.
    let sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Client-mode texture pipeline
    // Spirte arrives from teh server as compressed blobs on the network thread.
    // They are decompressed threre and placed in `pending_textures`. The GUI
    // thread drains this vec each from and uploads them to the GPU.
    let pending_textures: Arc<Mutex<Vec<PendingTexture>>> = Arc::new(Mutex::new(Vec::new()));

    // Tracks which species have already been requested so the GUI does not
    // flood the server with duplicate requests.
    let known_species: Arc<Mutex<std::collections::HashSet<u16>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));

    // The GUI thread pushes batches of species IDs there; the network writer
    // thread drains and sends them as `ClientMessage::RequestTextures`.
    // The queue survives reconnects so no request is lost on a dropped connection.
    let texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>> =
        Arc::new(Mutex::new(VecDeque::new()));

    match &mode {
        // -- Standalone & Server: load ROM and start the game-polling thread --
        Mode::Standalone | Mode::Server { .. } => {
            let rom_path = match args.get(1) {
                Some(path) => path.clone(),
                None => {
                    eprintln!("Missing ROM path argument");
                    std::process::exit(1);
                }
            };

            // `--clean` enables ability display; only works on unmodified ROMs.
            let is_clean = args.iter().any(|a| a == "--clean");

            let thread_party = shared_party.clone();
            let thread_encounters = shared_encounters.clone();

            let main_thread = std::thread::spawn(move || {
                // Initialize the memory-reading backend (mmap / process attach).
                match start_loop(rom_path.as_str(), is_clean) {
                    0 => println!("DEBUG: start_loop succeeded"),
                    code => {
                        eprintln!("Failed to start monitor loop (exit code: {})", code);
                        std::process::exit(1);
                    }
                }

                let mut current_fire_red_state = FireRedState::default();
                let mut old_party_size = get_party_size();
                let mut start_refresh_party_timer = std::time::SystemTime::now();
                fill_party_list(&thread_party);

                loop {
                    let state = get_value();
                    let current_party_size = get_party_size();

                    // Refresh party immediately whenever the party size changes
                    // (Pokemon caught, deposited, etc.)
                    if old_party_size != current_party_size {
                        old_party_size = current_party_size;
                        update_box_list();
                        fill_party_list(&thread_party);
                    }

                    // Refresh encounter table when the player moves to a new map.
                    if current_fire_red_state != state {
                        current_fire_red_state = state;
                        let encounters = get_area_pokemon_id();
                        let mut enc = thread_encounters.lock().unwrap_or_else(|e| e.into_inner());
                        *enc = encounters;
                    }

                    // Periodic force-refresh catches changes that would otherwise
                    // not be detected.
                    if start_refresh_party_timer
                        .elapsed()
                        .unwrap_or(std::time::Duration::ZERO)
                        .as_secs()
                        >= FORCE_PARTY_CHECK_TIME_IN_SECS
                    {
                        start_refresh_party_timer = std::time::SystemTime::now();
                        fill_party_list(&thread_party);
                    }

                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });

            *MAIN_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(main_thread);

            // Server mode: also start the TCP listener.
            if let Mode::Server { port } = &mode {
                let port = *port;
                let server_party = shared_party.clone();
                let server_encounters = shared_encounters.clone();
                let server_sprite_cache = sprite_cache.clone();

                let server_thread = std::thread::spawn(move || {
                    let listener = match TcpListener::bind(format!("0.0.0.0:{}", port)) {
                        Ok(l) => l,
                        Err(e) => {
                            eprintln!("Failed to start server on port {}: {}", port, e);
                            return;
                        }
                    };

                    println!("Server listening on port {}", port);

                    for stream in listener.incoming() {
                        if !RUNNING.load(Ordering::SeqCst) {
                            break;
                        }
                        match stream {
                            Ok(stream) => {
                                let party = server_party.clone();
                                let encounters = server_encounters.clone();
                                let cache = server_sprite_cache.clone();
                                // Each client gets its own thread; `handle_client` blocks until disconnect.
                                std::thread::spawn(move || {
                                    handle_client(stream, party, encounters, cache);
                                });
                            }
                            Err(e) => eprintln!("Connection error: {}", e),
                        }
                    }
                });

                *SERVER_THREAD_HANDLE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(server_thread);
            }
        }

        // -- Client: connect to server, receive state + sprites --
        Mode::Client { host, port } => {
            // No ROM needed — all data including sprites comes from the server.
            let client_party = shared_party.clone();
            let client_encounters = shared_encounters.clone();
            let client_pending = pending_textures.clone();
            let client_known = known_species.clone();
            let client_queue = texture_request_queue.clone();
            let addr = format!("{}:{}", host, port);

            let client_thread = std::thread::spawn(move || {
                // Outer reconnect loop - keeps retrying every 3 seconds on failure.
                loop {
                    println!("Connecting to server at {}...", addr);
                    match TcpStream::connect(&addr) {
                        Ok(stream) => {
                            println!("Connected to server!");

                            let mut write_stream = match stream.try_clone() {
                                Ok(s) => s,
                                Err(e) => {
                                    eprintln!("Failed to clone stream: {}", e);
                                    break;
                                }
                            };
                            let mut read_stream = stream;

                            // Flag so the reader can signal the writer to stop
                            // when teh connection drops.
                            let connected = Arc::new(AtomicBool::new(true));
                            let connected_writer = connected.clone();
                            let writer_queue = client_queue.clone();

                            // ── Writer thread: drains the shared request queue
                            // Batches all pending species requests into a single
                            // `ClientMessage::RequestTextures` per 50ms tick.
                            let writer = std::thread::spawn(move || {
                                while connected_writer.load(Ordering::SeqCst) {
                                    let batch = {
                                        let mut q =
                                            writer_queue.lock().unwrap_or_else(|e| e.into_inner());
                                        // Flatten all pending batches into one message
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

                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                            });

                            // ── Reader loop: receives State + Textures ────────
                            loop {
                                match recv_message::<ServerMessage>(&mut read_stream) {
                                    Ok(ServerMessage::State(state)) => {
                                        *client_party.lock().unwrap_or_else(|e| e.into_inner()) =
                                            state.party;
                                        *client_encounters
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner()) = state.encounters;
                                    }
                                    Ok(ServerMessage::Textures(sprites)) => {
                                        let mut pending = client_pending
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner());
                                        let mut known =
                                            client_known.lock().unwrap_or_else(|e| e.into_inner());
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
                                        eprintln!("Lost connection: {}", e);
                                        break;
                                    }
                                }
                            }

                            // Signal the writer to stop and wait for it
                            connected.store(false, Ordering::SeqCst);
                            let _ = writer.join();
                        }
                        Err(e) => eprintln!("Failed to connect: {}", e),
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            });

            *CLIENT_THREAD_HANDLE
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(client_thread);
        }
    }

    // ── Server mode: headless, park until Ctrl+C ────────────────────────────
    if let Mode::Server { .. } = &mode {
        println!(
            "{}",
            "***** Server mode - no GUI. Press Ctrl-C to exit. *****"
                .green()
                .bold()
        );

        ctrlc::set_handler(|| {
            RUNNING.store(false, Ordering::SeqCst);
            println!("\nShutting down...");
            std::process::exit(0);
        })
        .expect("Error setting Ctrl+C handler");

        while RUNNING.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        return;
    }

    // ── GUI (Standalone + Client) ────────────────────────────────────────────
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([PARTY_WINDOW.0, PARTY_WINDOW.1]),
        ..Default::default()
    };

    let app_title = match &mode {
        Mode::Standalone => "Tracker".to_string(),
        Mode::Server { port } => format!("Tracker (Server :{})", port),
        Mode::Client { host, port } => format!("Tracker (client {}:{})", host, port),
    };

    // Client mode: pass the queue so the GUI can request textures from the
    // network thread. Standalone: None — textures are loaded from ROM directly.
    let queue_for_gui = match &mode {
        Mode::Client { .. } => Some(texture_request_queue),
        _ => None,
    };

    let _ = eframe::run_native(
        &app_title,
        options,
        Box::new(|cc| {
            Ok(Box::new(WindowInfo::new(
                cc,
                shared_party,
                shared_encounters,
                pending_textures,
                known_species,
                queue_for_gui,
            )))
        }),
    );
}
