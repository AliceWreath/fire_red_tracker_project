//! # FireRed Tracker
//!
//! A real-time Pokémon FireRed party and encounter monitor with an egui GUI.
//!
//! # Modes
//!
//! - **Standalone** — reads the ROM and game memory locally, renders the GUI.
//! - **Server** — like standalone but also accepts TCP client connections and
//!   streams [`GameState`] updates and sprite data to them. Runs headless.
//! - **Client** — connects to a server over TCP, receives state and sprites,
//!   and renders the GUI without needing the ROM locally.
//!
//! # Usage
//!
//! ```text
//! tracker /path/to/file.gba [--clean]              # standalone
//! tracker /path/to/file.gba --server [port]        # server (default port 7878)
//! tracker --client [host] [port]                   # client (default 127.0.0.1:7878)
//! ```

use colored::Colorize;
use fire_red_loop::*;
use fire_red_party_monitor::get_is_clean;
use fire_red_states::*;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Window / image size constants
// ---------------------------------------------------------------------------

/// Default target window size for the party panel, in logical pixels.
const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);

/// Size of party pokemon sprites, in logical pixels.
const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// Default target window size for the encounters panel, in logical pixels.
const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);

/// Size of encounter pokemon sprites, in logical pixels.
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// How often (in seconds) the party list is force-refreshed even when the
/// party size has not changed, to catch in-place changes (e.g. HP loss).
const FORCE_PARTY_CHECK_INTERVAL: u64 = 5;

/// GBA address of the packed (map_group, map_name) bytes in EWRAM.
const MAP_GROUP_AND_NAME_ADDR: usize = 0x02031DBC;

/// Base address of EWRAM in the GBA address space.
const EWRAM_BASE: usize = 0x02000000;

static MAIN_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static CLIENT_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static SERVER_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// Sprite compression helpers
// ---------------------------------------------------------------------------

fn compress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::{Compression, write::ZlibEncoder};
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

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

fn build_sprite_data(rom: &[u8], species: u16, shiny: bool) -> Option<SpriteData> {
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, shiny).ok()?;
    let width = img.width();
    let height = img.height();
    let pixels = compress_pixels(&img.into_raw());
    Some(SpriteData { species, shiny, pixels, width, height })
}

// ---------------------------------------------------------------------------
// GUI state
// ---------------------------------------------------------------------------

/// A sprite received from the server, waiting to be uploaded to the GPU.
struct PendingTexture {
    species: u16,
    shiny: bool,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

/// Top-level application state passed to [`eframe`].
struct WindowInfo {
    party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    textures: HashMap<String, egui::TextureHandle>,
    encounters_open: bool,
    pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
    texture_request_queue: Option<Arc<Mutex<VecDeque<Vec<u16>>>>>,
}

impl WindowInfo {
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
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();

        // ── 1. Drain textures received from server (client mode) ─────────────
        {
            let mut pending = self.pending_textures.lock().unwrap_or_else(|e| e.into_inner());
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

        // ── 2. Load / request missing textures ───────────────────────────────
        {
            let list = self.party_list.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let encounter_list = self.encounter_list.lock().unwrap_or_else(|e| e.into_inner());
            let mut needed_for_request: Vec<u16> = Vec::new();

            // Encounter sprites (always normal palette).
            let all_encounters = encounter_list
                .land_mon_encounters.wild_pokemon_list.iter()
                .chain(encounter_list.water_mon_encounters.wild_pokemon_list.iter())
                .chain(encounter_list.rock_smash_encounters.wild_pokemon_list.iter())
                .chain(encounter_list.fishing_encounters.wild_pokemon_list.iter());

            for wild_pokemon in all_encounters {
                if wild_pokemon.species == 0 || wild_pokemon.species > 386 {
                    continue;
                }
                let key = format!("pokemon_{}_normal", wild_pokemon.species);
                if self.textures.contains_key(&key) {
                    continue;
                }
                if self.texture_request_queue.is_some() {
                    let known = self.known_species.lock().unwrap_or_else(|e| e.into_inner());
                    if !known.contains(&wild_pokemon.species) {
                        needed_for_request.push(wild_pokemon.species);
                    }
                } else {
                    let texture = load_texture_normal(
                        ctx,
                        fire_red_rom_buffer::get_rom(),
                        wild_pokemon.species,
                    )
                    .unwrap_or_else(|_| {
                        eprintln!("Failed to load texture for species {}.", wild_pokemon.species);
                        make_placeholder(ctx, wild_pokemon.species)
                    });
                    self.textures.insert(key, texture);
                }
            }

            drop(encounter_list);

            // Party sprites (shiny when applicable).
            let missing: Vec<(u16, u32, u32)> = list
                .iter()
                .map(|p| (
                    p.box_mon.secure.growth.species,
                    p.box_mon.personality,
                    p.box_mon.ot_id,
                ))
                .filter(|(species, personality, ot_id)| {
                    let key = format!(
                        "pokemon_{}_{}",
                        species,
                        if is_shiny(*personality, *ot_id) { "shiny" } else { "normal" }
                    );
                    !self.textures.contains_key(&key)
                })
                .collect();

            for (species, personality, ot_id) in missing {
                if species == 0 || species > 386 {
                    continue;
                }
                let key = format!(
                    "pokemon_{}_{}",
                    species,
                    if is_shiny(personality, ot_id) { "shiny" } else { "normal" }
                );
                if self.texture_request_queue.is_some() {
                    let known = self.known_species.lock().unwrap_or_else(|e| e.into_inner());
                    if !known.contains(&species) {
                        needed_for_request.push(species);
                    }
                } else {
                    let texture = load_texture(ctx, fire_red_rom_buffer::get_rom(), species, personality, ot_id)
                        .unwrap_or_else(|_| {
                            eprintln!("Failed to load texture for species {}.", species);
                            make_placeholder(ctx, species)
                        });
                    self.textures.insert(key, texture);
                }
            }

            if !needed_for_request.is_empty() {
                needed_for_request.sort();
                needed_for_request.dedup();
                if let Some(queue) = &self.texture_request_queue {
                    queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(needed_for_request);
                }
            }
        }

        // ── 3. Party panel ────────────────────────────────────────────────────
        #[allow(deprecated)]
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_ui(ui, ctx);
        });

        // ── 4. Encounters viewport ────────────────────────────────────────────
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
                            let encounters = encounter_list.lock().unwrap_or_else(|e| e.into_inner());

                            ui.heading("Land Encounters");
                            ui.horizontal(|ui| {
                                for mon in encounters.land_mon_encounters.wild_pokemon_list.iter() {
                                    let key = format!("pokemon_{}_normal", mon.species);
                                    if let Some(tex) = textures.get(&key) {
                                        ui.add(egui::Image::new(tex).fit_to_exact_size(
                                            egui::vec2(ENCOUNTER_IMAGE_SIZE.0, ENCOUNTER_IMAGE_SIZE.1),
                                        ));
                                    }
                                }
                            });

                            ui.separator();
                            ui.heading("Water Encounters");
                            ui.horizontal(|ui| {
                                for mon in encounters.water_mon_encounters.wild_pokemon_list.iter()
                                    .chain(encounters.fishing_encounters.wild_pokemon_list.iter())
                                {
                                    let key = format!("pokemon_{}_normal", mon.species);
                                    if let Some(tex) = textures.get(&key) {
                                        ui.add(egui::Image::new(tex).fit_to_exact_size(
                                            egui::vec2(ENCOUNTER_IMAGE_SIZE.0, ENCOUNTER_IMAGE_SIZE.1),
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
    fn draw_ui(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.heading("Party");

        let list = self.party_list.lock().unwrap_or_else(|e| e.into_inner());
        for (idx, pokemon) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                let species     = pokemon.box_mon.secure.growth.species;
                let personality = pokemon.box_mon.personality;
                let ot_id       = pokemon.box_mon.ot_id;
                let shiny       = is_shiny(personality, ot_id);
                let key = format!(
                    "pokemon_{}_{}",
                    species,
                    if shiny { "shiny" } else { "normal" }
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
                            let hp_ratio = pokemon.hp as f32 / pokemon.max_hp as f32;
                            let color = if hp_ratio < 0.3 {
                                egui::Color32::RED
                            } else if hp_ratio < 0.8 {
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

                    ui.label(format!("Caught Location: {}", pokemon.box_mon.secure.misc.met_location));

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

/// Loads a pokemon sprite from the ROM and uploads it as an egui texture.
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
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Ok(ctx.load_texture(
        format!("pokemon_{}_{}", species, if shiny { "shiny" } else { "normal" }),
        color_image,
        egui::TextureOptions::NEAREST,
    ))
}

/// Loads the non-shiny sprite for a species and uploads it as an egui texture.
pub fn load_texture_normal(
    ctx: &egui::Context,
    rom: &[u8],
    species: u16,
) -> Result<egui::TextureHandle, Box<dyn std::error::Error>> {
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, false)?;
    let size = [img.width() as usize, img.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Ok(ctx.load_texture(
        format!("pokemon_{}_normal", species),
        color_image,
        egui::TextureOptions::NEAREST,
    ))
}

/// Creates a solid-red placeholder texture for species whose sprites could not be loaded.
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

/// Overwrites the shared party list with the current party members.
fn fill_party_list(thread_party: &Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>) {
    *thread_party.lock().unwrap_or_else(|e| e.into_inner()) = get_party_members();
}

/// Returns `true` if the pokemon with `personality` and `ot_id` is shiny.
///
/// Uses the Gen III shiny determination formula:
/// `(p_high ^ p_low ^ id_high ^ id_low) < 8`
pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p_high  = (personality >> 16) as u16;
    let p_low   = (personality & 0xFFFF) as u16;
    let id_high = (ot_id >> 16) as u16;
    let id_low  = (ot_id & 0xFFFF) as u16;
    (p_high ^ p_low ^ id_high ^ id_low) < 8
}

/// Reads the current map state directly from the EWRAM snapshot, bypassing
/// the `STATE` mutex which may not have been written by the map thread yet.
fn map_state_from_ewram() -> Option<FireRedState> {
    let ewram = fire_red_memory::get_ewram();
    let offset = MAP_GROUP_AND_NAME_ADDR - EWRAM_BASE;
    if ewram.len() < offset + 2 {
        return None;
    }
    let group = ewram[offset];
    let name  = ewram[offset + 1];
    if group == 0 && name == 0 {
        return None;
    }
    Some(FireRedState { map_group_id: group, map_name_id: name })
}

// ---------------------------------------------------------------------------
// Server: handle one connected client
// ---------------------------------------------------------------------------

fn handle_client(
    stream: TcpStream,
    server_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    server_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>>,
) {
    println!(
        "Client connected: {}",
        stream.peer_addr().map_or_else(|_| "unknown".to_string(), |a| a.to_string())
    );

    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to clone stream: {}", e); return; }
    };
    let write_stream = Arc::new(Mutex::new(stream));

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
                        if species == 0 || species > 386 { continue; }
                        for shiny in [false, true] {
                            let cache_key = (species, shiny);
                            let mut cache = sprite_cache_for_reader.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(data) = cache.get(&cache_key) {
                                sprites.push(data.clone());
                            } else if let Some(data) = build_sprite_data(rom, species, shiny) {
                                cache.insert(cache_key, data.clone());
                                sprites.push(data);
                            }
                        }
                    }
                    if !sprites.is_empty() {
                        let mut ws = write_stream_for_reader.lock().unwrap_or_else(|e| e.into_inner());
                        if send_message(&mut *ws, &ServerMessage::Textures(sprites)).is_err() { break; }
                    }
                }
                Err(_) => break,
            }
        }
    });

    loop {
        let state = {
            let party      = server_party.lock().unwrap_or_else(|e| e.into_inner());
            let encounters = server_encounters.lock().unwrap_or_else(|e| e.into_inner());
            GameState {
                party:       party.clone(),
                encounters:  encounters.clone(),
                player_name: fire_red_loop::get_trainer_name(),
                badge_state: fire_red_badge::read_badge_state(),
            }
        };
        let mut ws = write_stream.lock().unwrap_or_else(|e| e.into_inner());
        if send_message(&mut *ws, &ServerMessage::State(state)).is_err() {
            println!("Client disconnected.");
            break;
        }
        drop(ws);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mode = match args.get(1).map(|s| s.as_str()) {
        Some("--client") => {
            let host = args.get(2).cloned().unwrap_or_else(|| "127.0.0.1".to_string());
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
            eprintln!("  {} firered.gba [--clean]               (standalone)", args[0]);
            eprintln!("  {} firered.gba --server [port]         (default port 7878)", args[0]);
            eprintln!("  {} --client [host] [port]              (default 127.0.0.1:7878)", args[0]);
            return;
        }
    };

    let shared_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>> =
        Arc::new(Mutex::new(Vec::new()));
    let shared_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>> =
        Arc::new(Mutex::new(fire_red_pokemon_data::WildPokemonHeader::default()));
    let sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let pending_textures: Arc<Mutex<Vec<PendingTexture>>> = Arc::new(Mutex::new(Vec::new()));
    let known_species: Arc<Mutex<std::collections::HashSet<u16>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    let texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>> =
        Arc::new(Mutex::new(VecDeque::new()));

    match &mode {
        Mode::Standalone | Mode::Server { .. } => {
            let rom_path = match args.get(1) {
                Some(path) => path.clone(),
                None => { eprintln!("Missing ROM path argument."); std::process::exit(1); }
            };
            let is_clean = args.iter().any(|a| a == "--clean");

            let thread_party      = shared_party.clone();
            let thread_encounters = shared_encounters.clone();

            let main_thread = std::thread::spawn(move || {
                match start_loop(rom_path.as_str(), is_clean) {
                    0 => println!("Monitor loop started."),
                    code => {
                        eprintln!("Failed to start monitor loop (exit code: {}).", code);
                        std::process::exit(1);
                    }
                }

                // Wait for the EWRAM snapshot to contain a real non-zero map state.
                // The memory loop needs one full poll cycle (~500ms) after start_loop
                // before the snapshot is populated.
                println!("Waiting for initial map state...");
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    if map_state_from_ewram().is_some() {
                        break;
                    }
                    if std::time::Instant::now() > deadline {
                        eprintln!("Warning: map state did not populate within 5 seconds.");
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }

                // Read map state directly from EWRAM for the initial encounter load.
                // We cannot use get_value() here because STATE (written by the map-polling
                // background thread) may not have been updated yet, and would return (0,0),
                // causing get_area_pokemon_id_for_state to find no matching header.
                let initial_map_state = map_state_from_ewram()
                    .unwrap_or(FireRedState { map_group_id: 0, map_name_id: 0 });

                println!(
                    "Map state ready: group={} name={}",
                    initial_map_state.map_group_id,
                    initial_map_state.map_name_id,
                );

                *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                    get_area_pokemon_id_for_state(&initial_map_state);

                // Track whether STATE has been populated with a real non-zero value.
                // Until it has, we ignore get_value() to avoid overwriting the
                // encounters we just populated from EWRAM.
                let mut state_initialized = false;
                let mut current_state     = initial_map_state;
                let mut old_party_size    = get_party_size();
                let mut last_party_refresh = std::time::Instant::now();

                fill_party_list(&thread_party);

                loop {
    // Read map state directly from EWRAM rather than going through STATE,
    // which lags behind by up to 500ms (EWRAM snapshot interval) + 333ms
    // (map thread poll interval).
    let state = map_state_from_ewram()
        .unwrap_or(current_state); // if EWRAM not ready, assume no change

    let party_size = get_party_size();

    if state_initialized && current_state != state {
        current_state = state;
        *thread_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
            get_area_pokemon_id_for_state(&current_state);
    }

    if !state_initialized
        && (state.map_group_id != 0 || state.map_name_id != 0)
    {
        state_initialized = true;
        current_state = state;
    }

    if old_party_size != party_size {
        old_party_size = party_size;
        update_box_list();
        fill_party_list(&thread_party);
    }

    if last_party_refresh.elapsed().as_secs() >= FORCE_PARTY_CHECK_INTERVAL {
        last_party_refresh = std::time::Instant::now();
        fill_party_list(&thread_party);
    }

    std::thread::sleep(std::time::Duration::from_millis(100));
}
            });

            *MAIN_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(main_thread);

            if let Mode::Server { port } = &mode {
                let port              = *port;
                let server_party      = shared_party.clone();
                let server_encounters = shared_encounters.clone();
                let server_cache      = sprite_cache.clone();

                let server_thread = std::thread::spawn(move || {
                    let listener = match TcpListener::bind(format!("0.0.0.0:{}", port)) {
                        Ok(l) => l,
                        Err(e) => {
                            eprintln!("Failed to start server on port {}: {}", port, e);
                            return;
                        }
                    };
                    println!("Server listening on port {}.", port);
                    for stream in listener.incoming() {
                        if !RUNNING.load(Ordering::SeqCst) { break; }
                        match stream {
                            Ok(s) => {
                                let party      = server_party.clone();
                                let encounters = server_encounters.clone();
                                let cache      = server_cache.clone();
                                std::thread::spawn(move || handle_client(s, party, encounters, cache));
                            }
                            Err(e) => eprintln!("Connection error: {}", e),
                        }
                    }
                });

                *SERVER_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(server_thread);
            }
        }

        Mode::Client { host, port } => {
            let addr              = format!("{}:{}", host, port);
            let client_party      = shared_party.clone();
            let client_encounters = shared_encounters.clone();
            let client_pending    = pending_textures.clone();
            let client_known      = known_species.clone();
            let client_queue      = texture_request_queue.clone();

            let client_thread = std::thread::spawn(move || {
                loop {
                    println!("Connecting to server at {}...", addr);
                    match TcpStream::connect(&addr) {
                        Ok(stream) => {
                            println!("Connected to server.");
                            let mut write_stream = match stream.try_clone() {
                                Ok(s) => s,
                                Err(e) => { eprintln!("Failed to clone stream: {}", e); break; }
                            };
                            let mut read_stream  = stream;
                            let connected        = Arc::new(AtomicBool::new(true));
                            let connected_writer = connected.clone();
                            let writer_queue     = client_queue.clone();

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
                                        ).is_err() { break; }
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                            });

                            loop {
                                match recv_message::<ServerMessage>(&mut read_stream) {
                                    Ok(ServerMessage::State(state)) => {
                                        *client_party.lock().unwrap_or_else(|e| e.into_inner()) =
                                            state.party;
                                        *client_encounters.lock().unwrap_or_else(|e| e.into_inner()) =
                                            state.encounters;
                                    }
                                    Ok(ServerMessage::Textures(sprites)) => {
                                        let mut pending = client_pending.lock().unwrap_or_else(|e| e.into_inner());
                                        let mut known   = client_known.lock().unwrap_or_else(|e| e.into_inner());
                                        for sprite in sprites {
                                            known.insert(sprite.species);
                                            pending.push(PendingTexture {
                                                species: sprite.species,
                                                shiny:   sprite.shiny,
                                                pixels:  decompress_pixels(&sprite.pixels),
                                                width:   sprite.width,
                                                height:  sprite.height,
                                            });
                                        }
                                    }
                                    Err(e) => { eprintln!("Lost connection: {}", e); break; }
                                }
                            }

                            connected.store(false, Ordering::SeqCst);
                            let _ = writer.join();
                        }
                        Err(e) => eprintln!("Failed to connect: {}", e),
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            });

            *CLIENT_THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(client_thread);
        }
    }

    // ── Server mode: headless, park until Ctrl-C ─────────────────────────────
    if let Mode::Server { .. } = &mode {
        println!("{}", "***** Server mode — no GUI. Press Ctrl-C to exit. *****".green().bold());
        ctrlc::set_handler(|| {
            RUNNING.store(false, Ordering::SeqCst);
            println!("\nShutting down...");
            std::process::exit(0);
        }).expect("Error setting Ctrl-C handler.");
        while RUNNING.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        return;
    }

    // ── GUI (Standalone + Client) ─────────────────────────────────────────────
    let app_title = match &mode {
        Mode::Standalone            => "Tracker".to_string(),
        Mode::Server { port }       => format!("Tracker (Server :{})", port),
        Mode::Client { host, port } => format!("Tracker (Client {}:{})", host, port),
    };

    let queue_for_gui = match &mode {
        Mode::Client { .. } => Some(texture_request_queue),
        _                   => None,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([PARTY_WINDOW.0, PARTY_WINDOW.1]),
        ..Default::default()
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