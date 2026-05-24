//! # GUI
//!
//! The egui application state and rendering for the tracker's party panel and
//! encounters child viewport.

use crate::game::is_shiny;
use crate::textures::{
    PARTY_IMAGE_SIZE, ENCOUNTER_IMAGE_SIZE,
    PendingTexture, load_texture, load_texture_normal, make_placeholder,
};
use fire_red_party_monitor::get_is_clean;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Default target window size for the party panel, in logical pixels.
pub const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);

/// Default target window size for the encounters panel, in logical pixels.
pub const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);

/// Top-level application state passed to [`eframe`].
pub struct WindowInfo {
    /// Current party pokemon, updated by the game-polling or network thread.
    pub party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
    /// Current area wild-encounter table, updated by the game-polling or network thread.
    pub encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
    /// GPU texture handles keyed by `"pokemon_{species}_{normal|shiny}"`.
    pub textures: HashMap<String, egui::TextureHandle>,
    /// Whether the encounters child viewport is currently open.
    pub encounters_open: bool,
    /// Sprites received from the server that have not yet been uploaded to the GPU.
    /// Drained at the start of each frame.
    pub pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    /// Species IDs for which a texture request has already been sent to the
    /// server, preventing duplicate requests.
    pub known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
    /// Queue of texture request batches produced by the GUI and consumed by
    /// the network writer thread. `None` in standalone/server mode.
    pub texture_request_queue: Option<Arc<Mutex<VecDeque<Vec<u16>>>>>,
}

impl WindowInfo {
    pub fn new(
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

    /// Main per-frame callback.
    ///
    /// 1. Drain pending textures received from the server.
    /// 2. Load or request any missing textures for visible species.
    /// 3. Draw the party panel.
    /// 4. Draw the encounters child viewport.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();

        // ── 1. Upload textures received from the server ───────────────────────
        {
            let mut pending = self.pending_textures.lock().unwrap_or_else(|e| e.into_inner());
            for pt in pending.drain(..) {
                let key = format!(
                    "pokemon_{}_{}",
                    pt.species,
                    if pt.shiny { "shiny" } else { "normal" },
                );
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [pt.width as usize, pt.height as usize],
                    &pt.pixels,
                );
                let handle = ctx.load_texture(&key, image, egui::TextureOptions::NEAREST);
                self.textures.insert(key, handle);
            }
        }

        // ── 2. Load / request missing textures ───────────────────────────────
        {
            let list           = self.party_list.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let encounter_list = self.encounter_list.lock().unwrap_or_else(|e| e.into_inner());
            let mut needed: Vec<u16> = Vec::new();

            // Encounter sprites are always the normal (non-shiny) palette.
            let all_encounter_mons = encounter_list.land_mon_encounters.wild_pokemon_list.iter()
                .chain(encounter_list.water_mon_encounters.wild_pokemon_list.iter())
                .chain(encounter_list.rock_smash_encounters.wild_pokemon_list.iter())
                .chain(encounter_list.fishing_encounters.wild_pokemon_list.iter());

            for mon in all_encounter_mons {
                if mon.species == 0 || mon.species > 386 { continue; }
                let key = format!("pokemon_{}_normal", mon.species);
                if self.textures.contains_key(&key) { continue; }
                if self.texture_request_queue.is_some() {
                    let known = self.known_species.lock().unwrap_or_else(|e| e.into_inner());
                    if !known.contains(&mon.species) { needed.push(mon.species); }
                } else {
                    let texture = load_texture_normal(ctx, fire_red_rom_buffer::get_rom(), mon.species)
                        .unwrap_or_else(|_| make_placeholder(ctx, mon.species));
                    self.textures.insert(key, texture);
                }
            }

            drop(encounter_list);

            // Party sprites use the shiny variant when applicable.
            let missing_party: Vec<(u16, u32, u32)> = list
                .iter()
                .map(|p| (p.box_mon.secure.growth.species, p.box_mon.personality, p.box_mon.ot_id))
                .filter(|(species, personality, ot_id)| {
                    let key = format!(
                        "pokemon_{}_{}",
                        species,
                        if is_shiny(*personality, *ot_id) { "shiny" } else { "normal" },
                    );
                    !self.textures.contains_key(&key)
                })
                .collect();

            for (species, personality, ot_id) in missing_party {
                if species == 0 || species > 386 { continue; }
                let key = format!(
                    "pokemon_{}_{}",
                    species,
                    if is_shiny(personality, ot_id) { "shiny" } else { "normal" },
                );
                if self.texture_request_queue.is_some() {
                    let known = self.known_species.lock().unwrap_or_else(|e| e.into_inner());
                    if !known.contains(&species) { needed.push(species); }
                } else {
                    let texture = load_texture(ctx, fire_red_rom_buffer::get_rom(), species, personality, ot_id)
                        .unwrap_or_else(|_| make_placeholder(ctx, species));
                    self.textures.insert(key, texture);
                }
            }

            if !needed.is_empty() {
                needed.sort();
                needed.dedup();
                if let Some(queue) = &self.texture_request_queue {
                    queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(needed);
                }
            }
        }

        // ── 3. Party panel ────────────────────────────────────────────────────
        #[allow(deprecated)]
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_party(ui);
        });

        // ── 4. Encounters child viewport ──────────────────────────────────────
        if self.encounters_open {
            let encounter_list = self.encounter_list.clone();
            let textures       = &self.textures;

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("encounters_window"),
                egui::ViewportBuilder::default()
                    .with_title("Encounters")
                    .with_inner_size([ENCOUNTER_WINDOW.0, ENCOUNTER_WINDOW.1]),
                move |ctx, _class| {
                    #[allow(deprecated)]
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let enc = encounter_list.lock().unwrap_or_else(|e| e.into_inner());

                            ui.heading("Land Encounters");
                            ui.horizontal(|ui| {
                                for mon in enc.land_mon_encounters.wild_pokemon_list.iter() {
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
                                for mon in enc.water_mon_encounters.wild_pokemon_list.iter()
                                    .chain(enc.fishing_encounters.wild_pokemon_list.iter())
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
    /// Draws the party panel.
    ///
    /// Renders badge summary, next gym info, then for each party member:
    /// sprite, nickname, level, HP (color-coded), met location, and ability
    /// (if `--clean` is active).
    pub fn draw_party(&mut self, ui: &mut egui::Ui) {
        ui.heading("Party");

        // ── Badge summary ─────────────────────────────────────────────────────
        if let Some(badge_state) = fire_red_badge::read_badge_state() {
            ui.horizontal(|ui| {
                ui.label(format!("Badges: {}/8", badge_state.count()));
                for obtained in &badge_state.badges {
                    let color = if *obtained {
                        egui::Color32::from_rgb(80, 200, 80)
                    } else {
                        egui::Color32::from_rgb(80, 80, 80)
                    };
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(14.0, 14.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().circle_filled(rect.center(), 5.0, color);
                }
            });

            if let Some(gym) = &badge_state.next_gym {
                ui.label(
                    egui::RichText::new(format!(
                        "Next: {} ({}) — Lv.{}",
                        gym.leader, gym.city, gym.max_level,
                    ))
                    .color(egui::Color32::from_rgb(255, 200, 50)),
                );
            } else {
                ui.label(
                    egui::RichText::new("All badges obtained!")
                        .color(egui::Color32::from_rgb(80, 200, 80)),
                );
            }

            ui.separator();
        }

        // ── Party members ─────────────────────────────────────────────────────
        let list = self.party_list.lock().unwrap_or_else(|e| e.into_inner());
        for (idx, pokemon) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                let species     = pokemon.box_mon.secure.growth.species;
                let personality = pokemon.box_mon.personality;
                let ot_id       = pokemon.box_mon.ot_id;
                let key = format!(
                    "pokemon_{}_{}",
                    species,
                    if is_shiny(personality, ot_id) { "shiny" } else { "normal" },
                );

                if let Some(tex) = self.textures.get(&key) {
                    ui.add(
                        egui::Image::new(tex)
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

                    ui.label(format!(
                        "Caught Location: {}",
                        pokemon.box_mon.secure.misc.met_location,
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
