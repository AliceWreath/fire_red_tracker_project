//! # GUI
//!
//! The egui application state and rendering for the tracker's party panel and
//! encounters child viewport.

use crate::config::{TrackerConfig, save_config};
use crate::game::is_shiny;
use crate::textures::{
    PARTY_IMAGE_SIZE, ENCOUNTER_IMAGE_SIZE,
    PendingTexture, load_texture, load_texture_normal, make_placeholder,
};
use fire_red_party_monitor::get_is_clean;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Default target window size for the party panel, in logical pixels.
pub const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);

/// Default target window size for the encounters panel, in logical pixels.
pub const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);

struct SettingsDraft {
    rom:             String,
    db:              String,
    clean:           bool,
    mode:            crate::config::ConfigMode,
    aggregator_host: String,
    aggregator_port: String,
}

impl SettingsDraft {
    fn from_config(cfg: &TrackerConfig) -> Self {
        Self {
            rom:             cfg.rom.clone(),
            db:              cfg.db.trim_start_matches("postgresql://").trim_start_matches("postgres://").to_string(),
            clean:           cfg.clean,
            mode:            cfg.mode.clone(),
            aggregator_host: cfg.aggregator_host.clone(),
            aggregator_port: cfg.aggregator_port.to_string(),
        }
    }
}

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
    pub config_path:   PathBuf,
    pub settings_open: bool,
    settings:          SettingsDraft,
}

impl WindowInfo {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
        encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
        pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
        known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
        texture_request_queue: Option<Arc<Mutex<VecDeque<Vec<u16>>>>>,
        config_path: PathBuf,
        config: &TrackerConfig,
    ) -> Self {
        Self {
            party_list,
            encounter_list,
            textures: HashMap::new(),
            encounters_open: true,
            pending_textures,
            known_species,
            texture_request_queue,
            config_path,
            settings_open: false,
            settings: SettingsDraft::from_config(config),
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

fn nature_mods(nature: &str) -> Option<(&'static str, &'static str)> {
    match nature {
        "Lonely"  => Some(("Atk", "Def")), "Brave"   => Some(("Atk", "Spe")),
        "Adamant" => Some(("Atk", "SpA")), "Naughty" => Some(("Atk", "SpD")),
        "Bold"    => Some(("Def", "Atk")), "Relaxed" => Some(("Def", "Spe")),
        "Impish"  => Some(("Def", "SpA")), "Lax"     => Some(("Def", "SpD")),
        "Timid"   => Some(("Spe", "Atk")), "Hasty"   => Some(("Spe", "Def")),
        "Jolly"   => Some(("Spe", "SpA")), "Naive"   => Some(("Spe", "SpD")),
        "Modest"  => Some(("SpA", "Atk")), "Mild"    => Some(("SpA", "Def")),
        "Quiet"   => Some(("SpA", "Spe")), "Rash"    => Some(("SpA", "SpD")),
        "Calm"    => Some(("SpD", "Atk")), "Gentle"  => Some(("SpD", "Def")),
        "Sassy"   => Some(("SpD", "Spe")), "Careful" => Some(("SpD", "SpA")),
        _         => None,
    }
}

fn stat_row_job(
    nature: &str,
    hp: u16, atk: u16, def: u16, spe: u16, spa: u16, spd: u16,
    base: egui::Color32,
    size: f32,
) -> egui::text::LayoutJob {
    let mods    = nature_mods(nature);
    let up_stat = mods.map(|(u, _)| u);
    let dn_stat = mods.map(|(_, d)| d);
    let stat_col = |label: &str| {
        if up_stat == Some(label)  { egui::Color32::from_rgb(255, 153, 204) }
        else if dn_stat == Some(label) { egui::Color32::from_rgb(158, 200, 255) }
        else { base }
    };
    let sep = egui::text::TextFormat {
        color: base, font_id: egui::FontId::proportional(size), ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    for (i, (label, val)) in [("HP", hp), ("Atk", atk), ("Def", def), ("Spe", spe), ("SpA", spa), ("SpD", spd)].iter().enumerate() {
        if i > 0 { job.append(" | ", 0.0, sep.clone()); }
        job.append(&format!("{} {}", label, val), 0.0, egui::text::TextFormat {
            color: stat_col(label), font_id: egui::FontId::proportional(size), ..Default::default()
        });
    }
    job
}

/// Returns the display symbol and color for a gender byte (0=male, 1=female, 2=genderless).
fn gender_label(gender: u8) -> (&'static str, egui::Color32) {
    match gender {
        0 => ("♂", egui::Color32::from_rgb(100, 160, 255)),
        1 => ("♀", egui::Color32::from_rgb(255, 130, 180)),
        _ => ("",  egui::Color32::TRANSPARENT),
    }
}

impl WindowInfo {
    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        egui::Grid::new("tracker_settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(110.0)
            .show(ui, |ui| {
                ui.label("ROM path:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.rom).desired_width(260.0).hint_text("path/to/firered.gba"));
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("GBA ROM", &["gba"]).pick_file() {
                            s.rom = path.display().to_string();
                        }
                    }
                });
                ui.end_row();

                ui.label("Database:");
                ui.add(egui::TextEdit::singleline(&mut s.db).desired_width(300.0).hint_text("localhost/nuzlocke"));
                ui.end_row();

                ui.label("Clean ROM:");
                ui.checkbox(&mut s.clean, "Enable ability name display");
                ui.end_row();

                ui.label("Default mode:");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.mode, crate::config::ConfigMode::Standalone, "Standalone");
                    ui.selectable_value(&mut s.mode, crate::config::ConfigMode::Connected,  "Connected");
                });
                ui.end_row();

                if s.mode == crate::config::ConfigMode::Connected {
                    ui.label("Aggregator host:");
                    ui.add(egui::TextEdit::singleline(&mut s.aggregator_host).desired_width(200.0));
                    ui.end_row();

                    ui.label("Aggregator port:");
                    ui.add(egui::TextEdit::singleline(&mut s.aggregator_port).desired_width(80.0));
                    ui.end_row();
                }
            });

        ui.add_space(8.0);
        let rom_ok  = !s.rom.trim().is_empty();
        let port_ok = s.mode != crate::config::ConfigMode::Connected || s.aggregator_port.parse::<u16>().is_ok();
        ui.horizontal(|ui| {
            let saved = ui.add_enabled(rom_ok && port_ok, egui::Button::new("Save")).clicked();
            if !rom_ok {
                ui.label(egui::RichText::new("ROM path is required").color(egui::Color32::from_rgb(220, 80, 80)).small());
            } else if !port_ok {
                ui.label(egui::RichText::new("Invalid port").color(egui::Color32::from_rgb(220, 80, 80)).small());
            }
            if saved {
                let db_raw = s.db.trim().to_string();
                let db = if db_raw.starts_with("postgresql://") || db_raw.starts_with("postgres://") { db_raw } else { format!("postgresql://{}", db_raw) };
                let cfg = TrackerConfig {
                    rom:             s.rom.trim().to_string(),
                    db,
                    clean:           s.clean,
                    mode:            s.mode.clone(),
                    aggregator_host: s.aggregator_host.trim().to_string(),
                    aggregator_port: s.aggregator_port.parse().unwrap_or(7878),
                };
                save_config(&cfg, &self.config_path);
                self.settings_open = false;
            }
        });
        ui.small("Changes take effect on next launch.");
    }

    /// Draws the party panel.
    ///
    /// Renders badge summary, next gym info, then for each party member:
    /// sprite, nickname, level, HP (color-coded), met location, and ability
    /// (if `--clean` is active).
    pub fn draw_party(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Party");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    self.settings_open = !self.settings_open;
                }
            });
        });

        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new("⚙ Settings")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| { self.draw_settings(ui); });
            self.settings_open = open;
        }

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
            let dead = fire_red_database::is_dead(pokemon.box_mon.personality);

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
                    let tint = if dead {
                        egui::Color32::from_rgba_unmultiplied(80, 80, 80, 180)
                    } else {
                        egui::Color32::WHITE
                    };
                    ui.add(
                        egui::Image::new(tex)
                            .fit_to_exact_size(egui::vec2(PARTY_IMAGE_SIZE.0, PARTY_IMAGE_SIZE.1))
                            .tint(tint),
                    );
                }

                ui.vertical(|ui| {
                    let (gender_sym, gender_color) = gender_label(pokemon.box_mon.gender);

                    if dead {
                        let record = fire_red_database::get_dead_pokemon(
                            pokemon.box_mon.personality,
                        );
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(pokemon.get_nickname_string())
                                    .strong()
                                    .size(18.0)
                                    .color(egui::Color32::from_rgb(150, 50, 50)),
                            );
                            if !gender_sym.is_empty() {
                                ui.label(
                                    egui::RichText::new(gender_sym)
                                        .strong()
                                        .size(15.0)
                                        .color(gender_color),
                                );
                            }
                            ui.label(
                                egui::RichText::new("DEAD")
                                    .strong()
                                    .size(18.0)
                                    .color(egui::Color32::RED),
                            );
                        });
                        if let Some(r) = &record {
                            let dim = egui::Color32::from_rgb(140, 140, 140);
                            ui.label(
                                egui::RichText::new(format!(
                                    "Lv.{} {} — {}{}",
                                    r.level,
                                    r.species_name,
                                    r.nature,
                                    if r.is_shiny { " ★" } else { "" },
                                ))
                                .color(dim),
                            );
                            ui.label(stat_row_job(&r.nature, r.max_hp, r.attack, r.defense, r.speed, r.sp_attack, r.sp_defense, dim, 11.0));
                            ui.label(
                                egui::RichText::new(format!(
                                    "Died: {}",
                                    fire_red_database::format_timestamp(r.died_at),
                                ))
                                .color(dim)
                                .size(11.0),
                            );
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(pokemon.get_nickname_string())
                                    .strong()
                                    .size(18.0)
                                    .color(egui::Color32::WHITE),
                            );
                            if !gender_sym.is_empty() {
                                ui.label(
                                    egui::RichText::new(gender_sym)
                                        .strong()
                                        .size(15.0)
                                        .color(gender_color),
                                );
                            }
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
                            "Caught: {}",
                            fire_red_location_names::location_name(
                                pokemon.box_mon.secure.misc.met_location,
                            ),
                        ));

                        if get_is_clean() {
                            ui.label(format!("Ability: {}", pokemon.box_mon.ability_string));
                        }
                    }
                });
            });
            ui.separator();
        }
    }
}
