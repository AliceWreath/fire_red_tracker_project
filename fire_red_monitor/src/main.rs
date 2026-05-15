use fire_red_loop::*;
use fire_red_party_monitor::get_is_clean;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);
const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);
const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

static FORCE_PARTY_CHECK_TIME_IN_SECS: u64 = 5;

struct WindowInfo {
    party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    textures: std::collections::HashMap<String, egui::TextureHandle>,
    encounters_open: bool,
}

impl WindowInfo {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
        encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    ) -> Self {
        Self {
            party_list,
            encounter_list,
            textures: std::collections::HashMap::new(), // start empty
            encounters_open: true,
        }
    }
}

impl eframe::App for WindowInfo {
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // intentionally empty - rendering is done in update()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();

        // load any missing textures before drawing
        {
            let list = self.party_list.lock().unwrap();
            let encounter_list = self.encounter_list.lock().unwrap();

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
                let key = format!("pokemon_{}_normal", wild_pokemon.species);
                if !self.textures.contains_key(&key) {
                    // Load and insert directly, bypassing the shiny logic
                    let texture = load_texture_normal(
                        ctx,
                        fire_red_rom_buffer::get_rom(),
                        wild_pokemon.species,
                    );
                    self.textures.insert(key, texture);
                }
            }

            drop(list);
            drop(encounter_list);

            for (species, personality, ot_id) in missing {
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
                );
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
            let textures: std::collections::HashMap<String, egui::TextureHandle> =
                self.textures.clone();

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("encounters_window"),
                egui::ViewportBuilder::default()
                    .with_title("Encounters")
                    .with_inner_size([ENCOUNTER_WINDOW.0, ENCOUNTER_WINDOW.1]),
                move |ctx, _class| {
                    #[allow(deprecated)]
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let encounters = encounter_list.lock().unwrap();
                            for wild_pokemon in
                                encounters.water_mon_encounters.wild_pokemon_list.iter()
                            {
                                let _key = format!("pokemon_{}_normal", wild_pokemon.species);
                            }
                            for wild_pokemon in
                                encounters.fishing_encounters.wild_pokemon_list.iter()
                            {
                                let _key = format!("pokemon_{}_normal", wild_pokemon.species);
                            }
                            for wild_pokemon in
                                encounters.rock_smash_encounters.wild_pokemon_list.iter()
                            {
                                let _key = format!("pokemon_{}_normal", wild_pokemon.species);
                            }
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

        let list = self.party_list.lock().unwrap();
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
                            } else {
                                if (pokemon.hp as f32) < (pokemon.max_hp as f32 * 0.8) {
                                    egui::Color32::YELLOW
                                } else {
                                    egui::Color32::WHITE
                                }
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
) -> egui::TextureHandle {
    let shiny = is_shiny(personality, ot_id);
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, shiny);
    let size = [img.width() as usize, img.height() as usize];
    let pixels: Vec<u8> = img.into_raw();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    ctx.load_texture(
        format!(
            "pokemon_{}_{}",
            species,
            if shiny { "shiny" } else { "normal" }
        ),
        color_image,
        egui::TextureOptions::NEAREST,
    )
}

fn fill_party_list(thread_party: &Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>) {
    let mut list = thread_party.lock().unwrap();
    *list = get_party_members();
}

pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p1 = (personality >> 16) as u16;
    let p2 = (personality & 0xFFFF) as u16;
    let id1 = (ot_id >> 16) as u16;
    let id2 = (ot_id & 0xFFFF) as u16;
    (p1 ^ p2 ^ id1 ^ id2) < 8
}

pub fn load_texture_normal(ctx: &egui::Context, rom: &[u8], species: u16) -> egui::TextureHandle {
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, false);
    let size = [img.width() as usize, img.height() as usize];
    let pixels: Vec<u8> = img.into_raw();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    ctx.load_texture(
        format!("pokemon_{}_normal", species),
        color_image,
        egui::TextureOptions::NEAREST,
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GameState {
    party: Vec<fire_red_party_monitor::Pokemon>,
    encounters: fire_red_pokemon_data::WildPokemonHeader,
}

enum Mode {
    Standalone,
    Server {
        port: u16,
    },
    Client {
        rom_path: String,
        host: String,
        port: u16,
    },
}

fn send_state(stream: &mut TcpStream, state: &GameState) -> std::io::Result<()> {
    let encoded =
        bincode::serialize(state).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

fn recv_state(stream: &mut TcpStream) -> std::io::Result<GameState> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;

    bincode::deserialize(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
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
            let rom_path = args[1].clone();
            let is_clean = args.iter().any(|a| a == "--clean");

            let thread_party = shared_party.clone();
            let thread_encounters = shared_encounters.clone();

            std::thread::spawn(move || {
                match start_loop(rom_path.as_str(), is_clean) {
                    0 => eprintln!("DEBUG: start_loop succeeded"),
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
                        let mut enc = thread_encounters.lock().unwrap();
                        *enc = encounters;
                    }

                    if start_refresh_party_timer.elapsed().unwrap().as_secs()
                        >= FORCE_PARTY_CHECK_TIME_IN_SECS
                    {
                        start_refresh_party_timer = std::time::SystemTime::now();
                        fill_party_list(&thread_party);
                    }

                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });

            if let Mode::Server { port } = &mode {
                let port = *port;
                let server_party = shared_party.clone();
                let server_encounters = shared_encounters.clone();

                std::thread::spawn(move || {
                    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
                        .expect("Failed to bind server port");
                    println!("Server listening on port {}", port);

                    for stream in listener.incoming() {
                        match stream {
                            Ok(mut stream) => {
                                println!("Client connected: {}", stream.peer_addr().unwrap());
                                loop {
                                    let state = {
                                        let party = server_party.lock().unwrap();
                                        let encounters = server_encounters.lock().unwrap();
                                        GameState {
                                            party: party.clone(),
                                            encounters: encounters.clone(),
                                        }
                                    };

                                    if send_state(&mut stream, &state).is_err() {
                                        println!("Client disconnected");
                                        break;
                                    }

                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                }
                            }
                            Err(e) => eprintln!("Connection error: {}", e),
                        }
                    }
                });
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

            std::thread::spawn(move || {
                loop {
                    println!("Connecting to server at {}...", addr);
                    match TcpStream::connect(&addr) {
                        Ok(mut stream) => {
                            println!("Connected!");
                            loop {
                                match recv_state(&mut stream) {
                                    Ok(state) => {
                                        *client_party.lock().unwrap() = state.party;
                                        *client_encounters.lock().unwrap() = state.encounters;
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
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([PARTY_WINDOW.0, PARTY_WINDOW.1]),
        ..Default::default()
    };

    let app_title = match &mode {
        Mode::Standalone => "Tracker".to_string(),
        Mode::Server { port } => format!("Tracker (Server :{}", port),
        Mode::Client {
            host,
            port,
            rom_path: _,
        } => format!("Tracker (client {}:{}", host, port),
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
