use colored::Colorize;
use fire_red_loop::*;
use fire_red_party_monitor::get_is_clean;
use fire_red_states::*;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);
const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);
const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);
const MAX_MESSAGE_SIZE: usize = 20 * 1024 * 1024; // 20 MB

static FORCE_PARTY_CHECK_TIME_IN_SECS: u64 = 5;
static MAIN_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static CLIENT_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static SERVER_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(true);

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize)]
pub enum ClientMessage {
    RequestTextures(Vec<u16>),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum ServerMessage {
    State(GameState),
    Textures(Vec<SpriteData>),
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SpriteData {
    pub species: u16,
    pub shiny: bool,
    pub pixels: Vec<u8>, // zlib-compressed RGBA bytes
    pub width: u32,
    pub height: u32,
}

// ---------------------------------------------------------------------------
// Wire helpers — length-prefixed bincode frames
// ---------------------------------------------------------------------------

fn send_message<T: serde::Serialize>(stream: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let encoded =
        bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

fn recv_message<T: serde::de::DeserializeOwned>(stream: &mut TcpStream) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("message too large: {} bytes", len),
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    bincode::deserialize(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

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

struct PendingTexture {
    species: u16,
    shiny: bool,
    pixels: Vec<u8>, // decompressed RGBA
    width: u32,
    height: u32,
}

struct WindowInfo {
    party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    textures: HashMap<String, egui::TextureHandle>,
    encounters_open: bool,
    // Client-mode texture pipeline
    pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
    // Shared queue so the GUI can request textures from the network thread.
    // None in standalone mode (textures loaded from ROM directly).
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
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // intentionally empty - rendering is done in update()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

fn fill_party_list(thread_party: &Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>) {
    let mut list = thread_party.lock().unwrap_or_else(|e| e.into_inner());
    *list = get_party_members();
}

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
            GameState {
                party: party.clone(),
                encounters: encounters.clone(),
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

    let shared_party: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>> =
        Arc::new(Mutex::new(Vec::new()));
    let shared_encounters: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>> = Arc::new(
        Mutex::new(fire_red_pokemon_data::WildPokemonHeader::default()),
    );

    // Shared sprite cache — keyed by (species, shiny), populated on demand
    let sprite_cache: Arc<Mutex<HashMap<(u16, bool), SpriteData>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Client-mode texture pipeline
    let pending_textures: Arc<Mutex<Vec<PendingTexture>>> = Arc::new(Mutex::new(Vec::new()));
    let known_species: Arc<Mutex<std::collections::HashSet<u16>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));

    // Shared queue so the GUI thread can enqueue texture requests that survive
    // reconnects — the network writer drains it each connection attempt.
    let texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>> =
        Arc::new(Mutex::new(VecDeque::new()));

    match &mode {
        Mode::Standalone | Mode::Server { .. } => {
            let rom_path = match args.get(1) {
                Some(path) => path.clone(),
                None => {
                    eprintln!("Missing ROM path argument");
                    std::process::exit(1);
                }
            };

            let is_clean = args.iter().any(|a| a == "--clean");

            let thread_party = shared_party.clone();
            let thread_encounters = shared_encounters.clone();

            let main_thread = std::thread::spawn(move || {
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

                    if old_party_size != current_party_size {
                        old_party_size = current_party_size;
                        update_box_list();
                        fill_party_list(&thread_party);
                    }

                    if current_fire_red_state != state {
                        current_fire_red_state = state;
                        let encounters = get_area_pokemon_id();
                        let mut enc = thread_encounters.lock().unwrap_or_else(|e| e.into_inner());
                        *enc = encounters;
                    }

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

        Mode::Client { host, port } => {
            // No ROM needed — all data including sprites comes from the server.
            let client_party = shared_party.clone();
            let client_encounters = shared_encounters.clone();
            let client_pending = pending_textures.clone();
            let client_known = known_species.clone();
            let client_queue = texture_request_queue.clone();
            let addr = format!("{}:{}", host, port);

            let client_thread = std::thread::spawn(move || {
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
                            let connected = Arc::new(AtomicBool::new(true));
                            let connected_writer = connected.clone();
                            let writer_queue = client_queue.clone();

                            // ── Writer thread: drains the shared request queue
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
