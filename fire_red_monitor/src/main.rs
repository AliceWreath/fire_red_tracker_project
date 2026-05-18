use fire_red_loop::*;
use fire_red_party_monitor::get_is_clean;
use fire_red_states::*;
use image::codecs::png::CompressionType;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use colored::Colorize;

const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);
const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);
const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);
const MAX_MESSAGE_SIZE: usize = 20 * 1024 * 1024; // 20MB

static FORCE_PARTY_CHECK_TIME_IN_SECS: u64 = 5;
static MAIN_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static CLIENT_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static SERVER_THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(true);

// message types

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
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

// Send/receive data helpers

fn send_message<T: serde::Serialize>(stream: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let encoded = bincode::serialize(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

fn recv_message<T: serde::de::DeserializeOwned>(stream: &mut TcpStream) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_but) as usize;
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

// sprite compression helpers

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

// server side sprite cache

fn build_sprite_data(rom: &[u8], species: u16) -> Option<SpriteData> {
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, false).ok()?;
    let width = img.width();
    let height = img.height();
    let pixels = compress_pixels(&img.into_raw());
    Some(SpriteData { species, pixels, width, height })
}

// GUI state

struct PendingTexture {
    species: u16,
    pixels: Vec<u8>, // decompressed RGBA
    width: u32,
    height: u32,
}

struct WindowInfo {
    party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    textures: HashMap<String, egui::TextureHandle>,
    encounters_open: bool,
    // client-mode texture pipeline
    pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
    // shared queue so the gui can request textures from teh network thread
    // none is standalone mode
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
            textures: HashMap::new(), // start empty
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

        // drain textures received from server in client mode
        {
            let mut pending = self.pending_textures.lock().unwrap_or_else(|e| e.into_inner());
            for pt in pending.drain(..) {
                let key = format!("pokemon_{}_normal", pt_species);
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [pt.width as usize, pt.height as usize],
                    &pt.pixels);
                let handle = ctx.load_texture(&key, color_image, egui::TextureOptions::NEAREST);
                self.textures.insert(key, handle);

                // deal with shinies if need be
                let shiny_key = format!("pokemon_{}_shiny", pt.species);
                if !self.textures.contains_key(&shiny_key) {
                    let color_image2 = egui::ColorImage::from_rgba_unmultiplied(
                        [pt.width as usize, pt.height as usize],
                        &pt.pixels);
                    let handle2 = ctx.load_texture(&shiny_key, color_image2, egui::TextureOptions::NEAREST);
                        self.textures.insert(shiny_key, handle2);
                }
            }
        }
        // load / request missing textures
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
                    (   // see line 203
                        p.box
                    )
                })
        }
        // load any missing textures before drawing
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

            // In the missing vec, use a sentinel that won't be shiny
            // Replace the wild pokemon push:
            for wild_pokemon in encounter_iters {
                if wild_pokemon.species == 0 || wild_pokemon.species > 386 {
                    continue; // skip invalid species
                }
                let key = format!("pokemon_{}_normal", wild_pokemon.species);
                if !self.textures.contains_key(&key) {
                    // Load and insert directly, bypassing the shiny logic
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
                        // Create a simple placeholder texture (e.g., a red square)
                        let size = [PARTY_IMAGE_SIZE.0 as usize, PARTY_IMAGE_SIZE.1 as usize];
                        let pixels = vec![255u8, 0, 0, 255].repeat(size[0] * size[1]);
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                        ctx.load_texture(
                            format!("pokemon_{}_placeholder", wild_pokemon.species),
                            color_image,
                            egui::TextureOptions::NEAREST,
                        )
                    });
                    self.textures.insert(key, texture);
                }
            }

            drop(list);
            drop(encounter_list);

            for (species, personality, ot_id) in missing {
                if species == 0 || species > 386 {
                    continue; // skip invalid species
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
                    // Create a simple placeholder texture (e.g., a red square)
                    let size = [PARTY_IMAGE_SIZE.0 as usize, PARTY_IMAGE_SIZE.1 as usize];
                    let pixels = vec![255u8, 0, 0, 255].repeat(size[0] * size[1]);
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                    ctx.load_texture(
                        format!("pokemon_{}_placeholder", species),
                        color_image,
                        egui::TextureOptions::NEAREST,
                    )
                });
                self.textures.insert(key, texture);
            }
        }

        #[allow(deprecated)]
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_ui(ui, ctx);
        });

        // separate independent window
        if self.encounters_open {
            let encounter_list = self.encounter_list.clone();

            // Snapshot all encounter textures before the move closure
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

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mode = match args.get(1).map(|s| s.as_str()) {
        Some("--client") => {
            let rom_path = args.get(2).cloned().unwrap_or_else(|| {
                eprintln!("Usage: {} --client firered.gba [host] [port]", args[0]);
                std::process::exit(1);
            });
            let host = args
                .get(3)
                .cloned()
                .unwrap_or_else(|| "127.0.0.1".to_string());
            let port = args.get(4).and_then(|p| p.parse().ok()).unwrap_or(7878);
            Mode::Client {
                rom_path,
                host,
                port,
            }
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
                "  {} --client firered.gba [host] [port]       (default 127.0.0.1:7878)",
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

                let server_thread = std::thread::spawn(move || {
                    let listener = match TcpListener::bind(format!("0.0.0.0:{}", port)) {
                        Ok(listen) => listen,
                        Err(e) => {
                            eprintln!("Failed to start server on port {}: {}", port, e);
                            return;
                        }
                    };

                    println!("Server listening on port {}", port);

                    for stream in listener.incoming() {
                        match stream {
                            Ok(mut stream) => {
                                println!(
                                    "Client connected: {}",
                                    stream
                                        .peer_addr()
                                        .map_or_else(|_| "unknown".to_string(), |a| a.to_string())
                                );
                                let sprite_cache: HashMap<u16, SpriteData> = HashMap::new();
                                let sprite_cache = Arc::new(Mutex::new(sprite_cache));

                                //spawn a reader thread for incoming texture requests
                                let cache_clone = sprite_cache.clone();
                                let mut read_stream = stream.try_clone().unwrap();
                                std::thread::spawn(move || {
                                    loop {
                                        match recv_client_message(&mut read_stream) {
                                            Ok(ClientMessage::RequestTextures(species_list)) => {
                                                let rom = fire_red_rom_buffer::get_rom();
                                                let mut responses = Vec::new();

                                                for species in species_list {
                                                    let mut cache = cache_clone.lock().unwrap();
                                                    if !cache.contains_key(&species) {
                                                        if let Ok(img) = fire_red_image_data::get_pokemon_sprite(rom, species, false) {
                                                            let pixels = compress(img.into_raw());
                                                            let data = SpriteData {
                                                                species,
                                                                pixels,
                                                                width: img.width(),
                                                                height: img.height(),
                                                            };
                                                            cache.insert(species, data.clone());
                                                            responses.push(data);
                                                        }
                                                    } else {
                                                        responses.push(cache[&species].clone());
                                                    }

                                                    // send back - need to write_stream references here
                                                }
                                                Err()) => break,
                                            }
                                        }
                                    }
                                });
                                loop {
                                    let state = {
                                        let party =
                                            server_party.lock().unwrap_or_else(|e| e.into_inner());
                                        let encounters = server_encounters
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner());
                                        GameState {
                                            party: party.clone(),
                                            encounters: encounters.clone(),
                                        }
                                    };                                    

                                    if send_state(&mut stream, &state).is_err() {
                                        println!("Client disconnected");
                                        break;
                                    }

                                    if send_server_message(&mut stream, &ServerMessage::State(state)).is_err() {
                                        break;
                                    }

                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                }
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

        Mode::Client {
            rom_path,
            host,
            port,
        } => {
            fire_red_rom_buffer::init_rom(rom_path).expect("Failed to load ROM");

            let client_party = shared_party.clone();
            let client_encounters = shared_encounters.clone();
            let addr = format!("{}:{}", host, port);

            let client_thread = std::thread::spawn(move || {
                loop {
                    println!("Connecting to server at {}...", addr);
                    match TcpStream::connect(&addr) {
                        Ok(mut stream) => {
                            println!("Connected!");
                            loop {
                                match recv_state(&mut stream) {
                                    Ok(state) => {
                                        *client_party.lock().unwrap_or_else(|e| e.into_inner()) =
                                            state.party;
                                        *client_encounters
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner()) = state.encounters;
                                    }
                                    Err(e) => {
                                        eprintln!("Lost connection: {}", e);
                                        break;
                                    }
                                }
                            }
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

    if let Mode::Server { .. } = &mode {
        // No GUI needed just wait for the server thread to finish (which will be never in normal operation)
        println!("{}", "***** Server mode - no GUI. Press Ctrl-C to exit. *****".green().bold());

        ctrlc::set_handler(|| {
            RUNNING.store(false, Ordering::SeqCst);
        })
        .expect("Error setting Ctrl+C handler");

        while RUNNING.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        return;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([PARTY_WINDOW.0, PARTY_WINDOW.1]),
        ..Default::default()
    };

    let app_title = match &mode {
        Mode::Standalone => "Tracker".to_string(),
        Mode::Server { port } => format!("Tracker (Server :{})", port),
        Mode::Client {
            host,
            port,
            rom_path: _,
        } => format!("Tracker (client {}:{})", host, port),
    };

    let _ = eframe::run_native(
        &app_title,
        options,
        Box::new(|cc| {
            Ok(Box::new(WindowInfo::new(
                cc,
                shared_party,
                shared_encounters,
            )))
        }),
    );
}
