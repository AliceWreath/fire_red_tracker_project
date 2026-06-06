//! # Aggregator UI
//!
//! The egui application that displays game state from multiple connected
//! FireRed instances side-by-side in a responsive column layout.
//!
//! # Layout
//!
//! Each player occupies one column. The column layout is implemented using
//! `SidePanel::left` for all but the last player, with the last player
//! filling the `CentralPanel`. This gives each column a real finite rect
//! with a known height, which is necessary for bottom-pinning encounters.
//!
//! Within each column the available rect is manually split:
//! - The **bottom portion** is reserved for encounters via `allocate_ui_at_rect`.
//! - The **top portion** gets a scrollable region via `allocate_ui_at_rect`
//!   containing the party and the caught log.
//!
//! # Soul Link detection and propagation
//!
//! Caught entries are cross-referenced by `met_location` for display. In
//! addition, whenever a pokemon in any player's dead table has a partner in
//! another player's caught table at the same location, `mark_soul_link_dead`
//! is called automatically so both partners are shown as dead.

use crate::client::{MonitorSlot, SharedSlots};
use crate::config::{AggregatorConfig, save_config};
use std::sync::Arc;

trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for std::sync::Mutex<T> {
    #[track_caller]
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            let loc = std::panic::Location::caller();
            eprintln!("Warning: mutex poisoned at {}:{}: {e}", loc.file(), loc.line());
            e.into_inner()
        })
    }
}
use std::path::PathBuf;
use egui::Ui;
use fire_red_database::{CaughtPokemon, DeadPokemon};
use fire_red_party_monitor::Pokemon;
use fire_red_states::{GameState, MAX_NATIONAL_DEX_FIRERED, SpriteVariant};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

/// Size at which party member sprites are rendered, in logical pixels.
const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// Size at which wild encounter sprites are rendered, in logical pixels.
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (48.0, 48.0);

/// Width of each player column in logical pixels.
const COLUMN_WIDTH: f32 = 320.0;

/// Estimated height of the encounter section per non-empty encounter type row.
/// heading (20) + sprite row (ENCOUNTER_IMAGE_SIZE.1) + spacing (12)
const ENCOUNTER_ROW_HEIGHT: f32 = 20.0 + 48.0 + 12.0;

/// Fixed height reserved for the encounter section at the bottom of each column.
/// Enough for a heading, separator, and up to 3 encounter type rows.
const ENCOUNTER_PANEL_HEIGHT: f32 = 32.0 + 3.0 * ENCOUNTER_ROW_HEIGHT;

// ---------------------------------------------------------------------------
// DB cache
// ---------------------------------------------------------------------------

/// Per-slot snapshot of the caught list, refreshed at most once per second.
/// Dead records are NOT cached here — they are queried fresh every frame so
/// there is never a stale-cache delay when a pokemon dies or the run_id is
/// first resolved.
struct SlotDbCache {
    caught:       Vec<CaughtPokemon>,
    last_refresh: std::time::Instant,
}

impl SlotDbCache {
    fn new() -> Self {
        Self {
            caught:       Vec::new(),
            // Initialise far in the past so the very first frame triggers a refresh.
            last_refresh: std::time::Instant::now()
                - std::time::Duration::from_secs(60),
        }
    }
}

// ---------------------------------------------------------------------------
// App struct
// ---------------------------------------------------------------------------

struct SettingsDraft {
    listen_port_str: String,
    db:              String,
    db_enabled:      bool,
    ws_port_str:     String,
    ws_port_enabled: bool,
    default_test:    bool,
    test:            Option<crate::config::AggregatorTestOverrides>,
}

impl SettingsDraft {
    fn from_config(cfg: &AggregatorConfig) -> Self {
        Self {
            listen_port_str: cfg.listen_port.to_string(),
            db:              cfg.db.as_deref()
                .map(|s| s.trim_start_matches("postgresql://").trim_start_matches("postgres://").to_string())
                .unwrap_or_else(|| "localhost/nuzlocke".to_string()),
            db_enabled:      cfg.db.is_some(),
            ws_port_str:     cfg.ws_port.map(|p| p.to_string()).unwrap_or_else(|| "9090".to_string()),
            ws_port_enabled: cfg.ws_port.is_some(),
            default_test:    cfg.default_test,
            test:            cfg.test.clone(),
        }
    }
}

/// The top-level eframe application for the multi-player aggregator view.
pub struct AggregatorApp {
    live_slots:           SharedSlots,
    /// Snapshot of slots taken at the start of each frame.
    slots:                Vec<Arc<MonitorSlot>>,
    textures:             HashMap<String, egui::TextureHandle>,
    db_caches:            Vec<SlotDbCache>,
    soul_link_propagated: HashSet<(usize, u32)>,
    frame_states:              Vec<(String, Option<GameState>)>,
    frame_all_dead:            Vec<HashMap<u32, DeadPokemon>>,
    frame_live_soul_link_dead: Vec<HashSet<u32>>,
    frame_db_connected:        Vec<bool>,
    config_path:   PathBuf,
    settings_open: bool,
    settings:      SettingsDraft,
    /// Latest release version string if a newer version is available, set by the
    /// background update-check thread.
    update_available: Arc<std::sync::Mutex<Option<String>>>,
    title_set: bool,
}

impl AggregatorApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        live_slots: SharedSlots,
        config_path: PathBuf,
        config: &AggregatorConfig,
        update_available: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            live_slots,
            slots:                     Vec::new(),
            textures:                  HashMap::new(),
            db_caches:                 Vec::new(),
            soul_link_propagated:      HashSet::new(),
            frame_states:              Vec::new(),
            frame_all_dead:            Vec::new(),
            frame_live_soul_link_dead: Vec::new(),
            frame_db_connected:        Vec::new(),
            config_path,
            settings_open: false,
            settings:      SettingsDraft::from_config(config),
            update_available,
            title_set: false,
        }
    }

    /// Drains pending textures and enqueues requests for missing species.
    fn process_textures(&mut self, ctx: &egui::Context) {
        for slot in &self.slots {
            {
                let mut pending = slot
                    .pending_textures
                    .lock_or_recover();
                for pt in pending.drain(..) {
                    let key = match pt.variant {
                        SpriteVariant::Front => sprite_key(pt.species, pt.shiny),
                        SpriteVariant::Back  => sprite_key_back(pt.species, pt.shiny),
                    };
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [pt.width as usize, pt.height as usize],
                        &pt.pixels,
                    );
                    let handle = ctx.load_texture(&key, image, egui::TextureOptions::NEAREST);
                    self.textures.insert(key, handle);
                }
            }
            {
                let state_guard = slot.state.lock_or_recover();
                let gs = match state_guard.as_ref() {
                    Some(gs) => gs,
                    None => continue,
                };
                let mut needed: Vec<u16> = Vec::new();
                let known = slot.known_species.lock_or_recover();

                for p in &gs.party {
                    let s = p.box_mon.secure.growth.species;
                    if s > 0 && s <= MAX_NATIONAL_DEX_FIRERED && !known.contains(&s) {
                        needed.push(s);
                    }
                }
                for wild in gs
                    .encounters
                    .land_mon_encounters
                    .wild_pokemon_list
                    .iter()
                    .chain(gs.encounters.water_mon_encounters.wild_pokemon_list.iter())
                    .chain(gs.encounters.rock_smash_encounters.wild_pokemon_list.iter())
                    .chain(gs.encounters.fishing_encounters.wild_pokemon_list.iter())
                {
                    if wild.species > 0 && wild.species <= MAX_NATIONAL_DEX_FIRERED && !known.contains(&wild.species) {
                        needed.push(wild.species);
                    }
                }
                drop(known);

                if !needed.is_empty() {
                    needed.sort();
                    needed.dedup();
                    slot.texture_request_queue
                        .lock_or_recover()
                        .push_back(needed);
                }
            }
        }
    }

    /// Draws one complete player column (party + caught log + encounters).
    #[allow(clippy::too_many_arguments)]
    fn draw_column(
        ui: &mut Ui,
        label: &str,
        state: &Option<GameState>,
        dead_records: &HashMap<u32, DeadPokemon>,
        soul_link_dead: &HashSet<u32>,
        db_connected: bool,
        textures: &HashMap<String, egui::TextureHandle>,
        all_states: &[(String, Option<GameState>)],
    ) {
        let full_rect = ui.max_rect();
        let enc_height = ENCOUNTER_PANEL_HEIGHT;
        let party_height = (full_rect.height() - enc_height).max(50.0);

        // ── Party + caught log region (top, scrollable) ───────────────────────
        let party_rect =
            egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), party_height));
        ui.scope_builder(egui::UiBuilder::new().max_rect(party_rect), |ui| {
            Self::draw_party_region(
                ui, label, state, dead_records, soul_link_dead, db_connected, textures, all_states,
            );
        });

        // ── Encounter region (bottom, pinned) ─────────────────────────────────
        let enc_min = egui::pos2(full_rect.min.x, full_rect.max.y - enc_height);
        let enc_rect =
            egui::Rect::from_min_size(enc_min, egui::vec2(full_rect.width(), enc_height));
        ui.scope_builder(egui::UiBuilder::new().max_rect(enc_rect), |ui| {
            Self::draw_encounter_region(ui, state, textures);
        });
    }

    /// Draws the scrollable party + caught log region.
    #[allow(clippy::too_many_arguments)]
    fn draw_party_region(
        ui: &mut Ui,
        label: &str,
        state: &Option<GameState>,
        dead_records: &HashMap<u32, DeadPokemon>,
        soul_link_dead: &HashSet<u32>,
        db_connected: bool,
        textures: &HashMap<String, egui::TextureHandle>,
        all_states: &[(String, Option<GameState>)],
    ) {
        ui.horizontal(|ui| {
            ui.heading(label);
            if db_connected {
                ui.label(
                    egui::RichText::new("● DB")
                        .color(egui::Color32::from_rgb(80, 200, 80))
                        .size(11.0),
                );
            }
        });
        ui.separator();

        let gs = match state {
            None => {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("⚠ Disconnected")
                        .color(egui::Color32::from_rgb(200, 80, 80))
                        .size(16.0),
                );
                return;
            }
            Some(gs) => gs,
        };

        egui::ScrollArea::vertical()
            .id_salt(format!("{}_party_scroll", label))
            .show(ui, |ui| {
                // Badge summary
                if let Some(badge_state) = &gs.badge_state {
                    ui.horizontal(|ui| {
                        ui.label(format!("Badges: {}/8", badge_state.count()));
                        for obtained in &badge_state.badges {
                            let color = if *obtained {
                                egui::Color32::from_rgb(80, 200, 80)
                            } else {
                                egui::Color32::from_rgb(80, 80, 80)
                            };
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
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
                            egui::RichText::new("Champion defeated!")
                                .color(egui::Color32::from_rgb(80, 200, 80)),
                        );
                    }
                    ui.separator();
                }

                ui.label(egui::RichText::new("Party").strong().size(15.0));
                ui.add_space(4.0);

                if gs.party.is_empty() {
                    ui.label("No party data");
                }

                for (idx, pokemon) in gs.party.iter().enumerate() {
                    let others: Vec<_> = all_states
                        .iter()
                        .filter(|(l, _)| l != label)
                        .cloned()
                        .collect();
                    let dead_record = dead_records.get(&pokemon.box_mon.personality);
                    let is_soul_link_dead = soul_link_dead.contains(&pokemon.box_mon.personality);
                    Self::draw_party_member(ui, idx, pokemon, dead_record, is_soul_link_dead, textures, &others);
                    ui.separator();
                }

            });
    }

    /// Draws the encounter section (always visible at the bottom of the column).
    fn draw_encounter_region(
        ui: &mut Ui,
        state: &Option<GameState>,
        textures: &HashMap<String, egui::TextureHandle>,
    ) {
        let gs = match state {
            Some(gs) => gs,
            None => return,
        };
        ui.separator();
        ui.label(egui::RichText::new("Encounters").strong().size(15.0));

        Self::draw_encounter_section(
            ui,
            "Land",
            &gs.encounters.land_mon_encounters.wild_pokemon_list,
            textures,
        );
        Self::draw_encounter_section(
            ui,
            "Water / Fishing",
            &gs.encounters
                .water_mon_encounters
                .wild_pokemon_list
                .iter()
                .chain(gs.encounters.fishing_encounters.wild_pokemon_list.iter())
                .cloned()
                .collect::<Vec<_>>(),
            textures,
        );
        Self::draw_encounter_section(
            ui,
            "Rock Smash",
            &gs.encounters.rock_smash_encounters.wild_pokemon_list,
            textures,
        );
    }

    /// Draws a single party member row with soul link annotation and full dead record.
    fn draw_party_member(
        ui: &mut Ui,
        _idx: usize,
        pokemon: &Pokemon,
        dead_record: Option<&DeadPokemon>,
        soul_link_dead: bool,
        textures: &HashMap<String, egui::TextureHandle>,
        other_states: &[(String, Option<GameState>)],
    ) {
        let species = pokemon.box_mon.secure.growth.species;
        let personality = pokemon.box_mon.personality;
        let ot_id = pokemon.box_mon.ot_id;
        let met = pokemon.box_mon.secure.misc.met_location;
        let shiny = is_shiny(personality, ot_id);
        let front_key = sprite_key(species, shiny);
        let back_key  = sprite_key_back(species, shiny);
        let show_back = (ui.ctx().input(|i| i.time) * 2.0) as u64 % 2 == 1;
        let key = if show_back && textures.contains_key(&back_key) { back_key } else { front_key };
        let dead = dead_record.is_some() || pokemon.hp == 0 || soul_link_dead;

        // Soul-link annotation based on live party state.
        for (other_label, other_state) in other_states {
            if let Some(gs) = other_state {
                for other_mon in &gs.party {
                    if other_mon.box_mon.secure.misc.met_location == met {
                        ui.label(
                            egui::RichText::new(format!(
                                "Soul-link: {} ({})",
                                other_mon.get_nickname_string(),
                                other_label,
                            ))
                            .color(egui::Color32::from_rgb(191, 64, 191))
                            .size(13.0),
                        );
                    }
                }
            }
        }

        let sprite_tint = if dead {
            egui::Color32::from_rgba_unmultiplied(80, 80, 80, 180)
        } else {
            egui::Color32::WHITE
        };

        ui.horizontal(|ui| {
            if let Some(tex) = textures.get(&key) {
                ui.add(
                    egui::Image::new(tex)
                        .fit_to_exact_size(egui::vec2(PARTY_IMAGE_SIZE.0, PARTY_IMAGE_SIZE.1))
                        .tint(sprite_tint),
                );
            }
            ui.vertical(|ui| {
                let name_color = if dead {
                    egui::Color32::from_rgb(150, 50, 50)
                } else {
                    egui::Color32::WHITE
                };
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(pokemon.get_nickname_string())
                            .strong()
                            .size(16.0)
                            .color(name_color),
                    );
                    if shiny {
                        ui.label(egui::RichText::new("✨").size(14.0));
                    }
                    if dead {
                        ui.label(
                            egui::RichText::new("DEAD")
                                .strong()
                                .size(14.0)
                                .color(egui::Color32::RED),
                        );
                    } else {
                        ui.label(format!("Lv{}", pokemon.level));
                    }
                });

                if let Some(r) = dead_record {
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
                    if r.max_hp > 0 {
                        ui.label(stat_row_job(&r.nature, r.max_hp, r.attack, r.defense, r.speed, r.sp_attack, r.sp_defense, dim, 11.0));
                    } else {
                        ui.label(
                            egui::RichText::new("Soul Link").color(dim).size(11.0),
                        );
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "Died: {}",
                            fire_red_database::format_timestamp(r.died_at),
                        ))
                        .color(dim)
                        .size(11.0),
                    );
                } else if pokemon.hp == 0 || soul_link_dead {
                    let dim = egui::Color32::from_rgb(140, 140, 140);
                    let nature = fire_red_database::nature_name(personality);
                    let species_name = &pokemon.box_mon.secure.growth.species_string;
                    ui.label(
                        egui::RichText::new(format!(
                            "Lv.{} {} — {}{}",
                            pokemon.level,
                            species_name,
                            nature,
                            if shiny { " ★" } else { "" },
                        ))
                        .color(dim),
                    );
                    ui.label(stat_row_job(nature, pokemon.max_hp, pokemon.attack, pokemon.defense, pokemon.speed, pokemon.sp_attack, pokemon.sp_defense, dim, 11.0));
                    if soul_link_dead {
                        ui.label(egui::RichText::new("Soul Link").color(dim).size(11.0));
                    }
                } else {
                    let hp_ratio = pokemon.hp as f32 / pokemon.max_hp.max(1) as f32;
                    let hp_color = if hp_ratio < 0.3 {
                        egui::Color32::RED
                    } else if hp_ratio < 0.8 {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::WHITE
                    };
                    ui.label(
                        egui::RichText::new(format!("{}/{}", pokemon.hp, pokemon.max_hp))
                            .color(hp_color)
                            .size(14.0),
                    );
                    ui.label(format!("Exp: {}", pokemon.box_mon.secure.growth.experience));
                }
            });
        });
    }

    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.settings;
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                ui.label("Listen port:");
                ui.add(egui::TextEdit::singleline(&mut s.listen_port_str).desired_width(80.0));
                ui.end_row();

                ui.checkbox(&mut s.db_enabled, "Database:");
                ui.add_enabled_ui(s.db_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.db).desired_width(280.0).hint_text("localhost/nuzlocke"));
                });
                ui.end_row();

                ui.checkbox(&mut s.ws_port_enabled, "WebSocket overlay:");
                ui.add_enabled_ui(s.ws_port_enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("port:");
                        ui.add(egui::TextEdit::singleline(&mut s.ws_port_str).desired_width(70.0));
                    });
                });
                ui.end_row();

                ui.checkbox(&mut s.default_test, "Default to test mode:");
                ui.small("Uses [test] config overrides on every launch (same as always passing --test).");
                ui.end_row();
            });

        ui.add_space(8.0);
        let port_ok = s.listen_port_str.trim().parse::<u16>().is_ok();
        let ws_ok   = !s.ws_port_enabled || s.ws_port_str.trim().parse::<u16>().is_ok();
        ui.horizontal(|ui| {
            let saved = ui.add_enabled(port_ok && ws_ok, egui::Button::new("Save")).clicked();
            if !port_ok {
                ui.label(egui::RichText::new("Invalid listen port").color(egui::Color32::from_rgb(220, 80, 80)).small());
            } else if !ws_ok {
                ui.label(egui::RichText::new("Invalid WebSocket port").color(egui::Color32::from_rgb(220, 80, 80)).small());
            }
            if saved {
                let db = if s.db_enabled {
                    let raw = s.db.trim().to_string();
                    Some(if raw.starts_with("postgresql://") || raw.starts_with("postgres://") { raw } else { format!("postgresql://{}", raw) })
                } else { None };
                let cfg = AggregatorConfig {
                    listen_port:  s.listen_port_str.trim().parse().unwrap_or(7878),
                    db,
                    ws_port:      if s.ws_port_enabled { s.ws_port_str.trim().parse().ok() } else { None },
                    default_test: s.default_test,
                    test:         s.test.clone(),
                };
                save_config(&cfg, &self.config_path);
                self.settings_open = false;
            }
        });
        ui.small("Changes take effect on next launch.");
    }

    /// Draws a labelled row of encounter sprites, skipping empty sections.
    fn draw_encounter_section(
        ui: &mut Ui,
        heading: &str,
        list: &[fire_red_pokemon_data::WildPokemon],
        textures: &HashMap<String, egui::TextureHandle>,
    ) {
        let valid: Vec<_> = list
            .iter()
            .filter(|w| w.species > 0 && w.species <= MAX_NATIONAL_DEX_FIRERED)
            .collect();
        if valid.is_empty() {
            return;
        }

        ui.label(egui::RichText::new(heading).italics());
        ui.horizontal_wrapped(|ui| {
            for wild in &valid {
                let key = sprite_key(wild.species, false);
                if let Some(tex) = textures.get(&key) {
                    ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(
                        ENCOUNTER_IMAGE_SIZE.0,
                        ENCOUNTER_IMAGE_SIZE.1,
                    )))
                    .on_hover_text(format!("Lv{}-{}", wild.min_level, wild.max_level));
                }
            }
        });
        ui.add_space(4.0);
    }
}

// ---------------------------------------------------------------------------
// eframe::App
// ---------------------------------------------------------------------------

impl eframe::App for AggregatorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Fire Red Aggregator").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙ Settings").clicked() {
                        self.settings_open = !self.settings_open;
                    }
                });
            });
        });

        let n = self.slots.len();
        if self.frame_states.len() < n { return; }

        let textures              = &self.textures;
        let frame_states          = &self.frame_states;
        let frame_all_dead        = &self.frame_all_dead;
        let frame_soul_link_dead  = &self.frame_live_soul_link_dead;
        let db_connected          = &self.frame_db_connected;

        for i in 0..n {
            let (label, state)   = &frame_states[i];
            let dead_records     = &frame_all_dead[i];
            let soul_link_dead   = &frame_soul_link_dead[i];

            let panel_id = egui::Id::new(format!("player_col_{}", i));
            egui::Panel::left(panel_id)
                .exact_size(COLUMN_WIDTH)
                .resizable(false)
                .show_inside(ui, |ui| {
                    AggregatorApp::draw_column(
                        ui, label, state, dead_records, soul_link_dead,
                        db_connected[i], textures, frame_states,
                    );
                });
        }

        // Settings modal window
        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new("⚙ Settings")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    self.draw_settings(ui);
                });
            self.settings_open = open;
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.title_set
            && let Some(v) = &*self.update_available.lock_or_recover()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                format!("Fire Red Aggregator — v{} available", v.trim_start_matches('v')),
            ));
            self.title_set = true;
        }

        // Snapshot the live slot list for this frame.
        self.slots = self.live_slots.lock_or_recover().clone();
        while self.db_caches.len() < self.slots.len() {
            self.db_caches.push(SlotDbCache::new());
        }

        ctx.request_repaint();
        self.process_textures(ctx);

        // ── Collect live states ───────────────────────────────────────────────
        let states: Vec<(String, Option<GameState>)> = self
            .slots
            .iter()
            .map(|slot| {
                let state = slot.state.lock_or_recover().clone();
                let label = slot.label.lock_or_recover().clone();
                (label, state)
            })
            .collect();

        // ── Sync each DbReader to the correct run for the current player ──────
        // sync_player returns true when the run_id was just resolved (first
        // success or name change). We use that to trigger an immediate refresh
        // of the caught cache rather than waiting up to a second.
        let mut run_id_changed: Vec<bool> = vec![false; self.slots.len()];
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(db) = &slot.db {
                run_id_changed[i] = db.sync_player(&states[i].0);
            }
        }

        // ── Refresh caught cache: immediately on run_id change, else every second
        let now = std::time::Instant::now();
        for i in 0..self.slots.len() {
            let should_refresh = run_id_changed[i]
                || now.duration_since(self.db_caches[i].last_refresh)
                    >= std::time::Duration::from_secs(1);
            if should_refresh
                && let Some(db) = &self.slots[i].db {
                self.db_caches[i].caught = db.list_caught(&states[i].0);
                self.db_caches[i].last_refresh = now;
            }
        }

        // ── Dead records: queried fresh every frame ───────────────────────────
        // One query per slot (not per party member) is cheaper than the old
        // per-member is_dead() approach, and eliminates any cache-staleness delay.
        let all_dead: Vec<HashMap<u32, DeadPokemon>> = self
            .slots
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                slot.db
                    .as_ref()
                    .map(|db| db.list_dead_with_records(&states[i].0))
                    .unwrap_or_default()
            })
            .collect();

        // ── Soul link death propagation ───────────────────────────────────────
        let n = self.slots.len();
        let caught_by_slot: Vec<Vec<CaughtPokemon>> = self.db_caches
            .iter()
            .map(|c| c.caught.clone())
            .collect();
        for (j, partner) in soul_link_kill_candidates(
            &all_dead,
            &caught_by_slot,
            &self.soul_link_propagated,
        ) {
            let wrote = self.slots[j].db.as_ref()
                .map(|db| db.mark_soul_link_dead(&partner))
                .unwrap_or(false);
            if wrote {
                self.soul_link_propagated.insert((j, partner.personality));
            }
        }

        // ── Live soul link dead detection ─────────────────────────────────────
        // For each slot, collect personalities whose soul link partner already
        // has hp == 0 in another slot's live game state. This makes the partner
        // show as dead instantly without waiting for the DB write (which can lag
        // up to 5 seconds due to the tracker's FORCE_PARTY_CHECK_INTERVAL).
        let mut live_soul_link_dead: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        for i in 0..n {
            let Some(gs_i) = &states[i].1 else { continue };
            for pokemon_i in &gs_i.party {
                if pokemon_i.hp != 0 { continue; }
                let met_i = pokemon_i.box_mon.secure.misc.met_location;
                if met_i == 0 { continue; }
                for j in 0..n {
                    if j == i { continue; }
                    let Some(gs_j) = &states[j].1 else { continue };
                    for pokemon_j in &gs_j.party {
                        if pokemon_j.box_mon.secure.misc.met_location == met_i {
                            live_soul_link_dead[j].insert(pokemon_j.box_mon.personality);
                        }
                    }
                }
            }
        }

        // ── Store frame data for ui() ─────────────────────────────────────────
        self.frame_db_connected        = self.slots.iter().map(|s| s.db.is_some()).collect();
        self.frame_states              = states;
        self.frame_all_dead            = all_dead;
        self.frame_live_soul_link_dead = live_soul_link_dead;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `(boosted_stat, dropped_stat)` label pairs for a nature, or `None` for neutral.
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

#[allow(clippy::too_many_arguments)]
fn stat_row_job(
    nature: &str,
    hp: u16, atk: u16, def: u16, spe: u16, spa: u16, spd: u16,
    base: egui::Color32,
    size: f32,
) -> egui::text::LayoutJob {
    let mods     = nature_mods(nature);
    let up_stat  = mods.map(|(u, _)| u);
    let dn_stat  = mods.map(|(_, d)| d);
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

/// For every dead pokemon across slots, finds partners in other slots caught
/// at the same `met_location` that are not yet dead and have not already been
/// propagated this session.
///
/// Returns `(slot_j, partner)` pairs; the caller is responsible for writing
/// the DB record and updating `soul_link_propagated`.
fn soul_link_kill_candidates(
    all_dead: &[HashMap<u32, DeadPokemon>],
    caught_by_slot: &[Vec<CaughtPokemon>],
    already_propagated: &HashSet<(usize, u32)>,
) -> Vec<(usize, CaughtPokemon)> {
    let n = all_dead.len();
    let mut out = Vec::new();
    for i in 0..n {
        for &dead_p in all_dead[i].keys() {
            let met_loc = caught_by_slot[i]
                .iter()
                .find(|c| c.personality == dead_p)
                .map(|c| c.met_location)
                .unwrap_or(0);
            if met_loc == 0 { continue; }
            for j in 0..n {
                if j == i { continue; }
                if let Some(p) = caught_by_slot[j]
                    .iter()
                    .find(|c| c.met_location == met_loc && c.personality != dead_p)
                    .cloned()
                    && !all_dead[j].contains_key(&p.personality)
                    && !already_propagated.contains(&(j, p.personality))
                {
                    out.push((j, p));
                }
            }
        }
    }
    out
}

/// Returns the front-sprite texture cache key for a given species and shininess.
pub fn sprite_key(species: u16, shiny: bool) -> String {
    format!("pokemon_{}_{}", species, if shiny { "shiny" } else { "normal" })
}

/// Returns the back-sprite texture cache key for a given species and shininess.
pub fn sprite_key_back(species: u16, shiny: bool) -> String {
    format!("pokemon_{}_{}_back", species, if shiny { "shiny" } else { "normal" })
}

/// Returns `true` if the pokemon with `personality` and `ot_id` is shiny.
///
/// Uses the Gen III formula: `(p_high ^ p_low ^ id_high ^ id_low) < 8`.
pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p_high = (personality >> 16) as u16;
    let p_low = (personality & 0xFFFF) as u16;
    let id_high = (ot_id >> 16) as u16;
    let id_low = (ot_id & 0xFFFF) as u16;
    (p_high ^ p_low ^ id_high ^ id_low) < 8
}

#[cfg(test)]
mod tests {
    use super::*;
    use fire_red_database::{IVs, EVs};

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn stub_caught(personality: u32, met_location: u8) -> CaughtPokemon {
        CaughtPokemon {
            player_name: String::new(), personality, ot_id: 0,
            nickname: String::new(), species: 0, species_name: String::new(),
            is_shiny: false, nature: String::new(), level: 5, met_location,
            location_name: String::new(),
            ivs: IVs::default(), evs: EVs::default(), caught_at: 0, gender: 2,
        }
    }

    fn stub_dead() -> DeadPokemon {
        DeadPokemon {
            player_name: String::new(), personality: 0, ot_id: 0, ot_name: String::new(),
            nickname: String::new(), species: 0, species_name: String::new(), is_shiny: false,
            nature: String::new(), level: 5, experience: 0, max_hp: 0, attack: 0, defense: 0,
            speed: 0, sp_attack: 0, sp_defense: 0, moves: [0; 4], pp: [0; 4],
            ivs: IVs::default(), evs: EVs::default(), held_item: 0, ability: 0,
            ability_name: String::new(), friendship: 0, met_location: 0, died_at: 0, gender: 2,
        }
    }

    fn dead_map(personalities: &[u32]) -> HashMap<u32, DeadPokemon> {
        personalities.iter().map(|&p| (p, stub_dead())).collect()
    }

    // ── soul_link_kill_candidates ─────────────────────────────────────────────

    #[test]
    fn no_dead_pokemon_returns_empty() {
        let all_dead  = vec![HashMap::new(), HashMap::new()];
        let caught    = vec![vec![stub_caught(1, 10)], vec![stub_caught(2, 10)]];
        let propagated = HashSet::new();
        assert!(soul_link_kill_candidates(&all_dead, &caught, &propagated).is_empty());
    }

    #[test]
    fn dead_with_no_partner_at_same_location_returns_empty() {
        let all_dead = vec![dead_map(&[1]), HashMap::new()];
        let caught   = vec![
            vec![stub_caught(1, 10)],
            vec![stub_caught(2, 99)], // different location
        ];
        let propagated = HashSet::new();
        assert!(soul_link_kill_candidates(&all_dead, &caught, &propagated).is_empty());
    }

    #[test]
    fn partner_at_same_location_is_identified() {
        let all_dead = vec![dead_map(&[1]), HashMap::new()];
        let caught   = vec![
            vec![stub_caught(1, 10)], // dead pokemon, caught at 10 by slot 0
            vec![stub_caught(2, 10)], // soul-link partner, caught at 10 by slot 1
        ];
        let propagated = HashSet::new();
        let candidates = soul_link_kill_candidates(&all_dead, &caught, &propagated);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, 1);
        assert_eq!(candidates[0].1.personality, 2);
    }

    #[test]
    fn already_dead_partner_is_skipped() {
        // Both pokemon are already dead — no new kills needed.
        let all_dead = vec![dead_map(&[1]), dead_map(&[2])];
        let caught   = vec![
            vec![stub_caught(1, 10)],
            vec![stub_caught(2, 10)],
        ];
        let propagated = HashSet::new();
        assert!(soul_link_kill_candidates(&all_dead, &caught, &propagated).is_empty());
    }

    #[test]
    fn already_propagated_pair_is_skipped() {
        let all_dead = vec![dead_map(&[1]), HashMap::new()];
        let caught   = vec![
            vec![stub_caught(1, 10)],
            vec![stub_caught(2, 10)],
        ];
        let mut propagated = HashSet::new();
        propagated.insert((1usize, 2u32)); // already handled this session
        assert!(soul_link_kill_candidates(&all_dead, &caught, &propagated).is_empty());
    }

    #[test]
    fn met_location_zero_is_ignored() {
        let all_dead = vec![dead_map(&[1]), HashMap::new()];
        let caught   = vec![
            vec![stub_caught(1, 0)], // met_location = 0 → untracked
            vec![stub_caught(2, 0)],
        ];
        let propagated = HashSet::new();
        assert!(soul_link_kill_candidates(&all_dead, &caught, &propagated).is_empty());
    }

    #[test]
    fn dead_pokemon_not_in_caught_cache_is_skipped() {
        // Personality 99 is dead in slot 0 but has no caught record → met_loc = 0 → skip.
        let all_dead = vec![dead_map(&[99]), HashMap::new()];
        let caught   = vec![
            vec![stub_caught(1, 10)], // no entry for personality 99
            vec![stub_caught(2, 10)],
        ];
        let propagated = HashSet::new();
        assert!(soul_link_kill_candidates(&all_dead, &caught, &propagated).is_empty());
    }

    #[test]
    fn three_slots_cross_links_all_partners() {
        // Slot 0's pokemon 1 (met 10) links to slot 1's pokemon 2 AND slot 2's pokemon 3.
        let all_dead = vec![dead_map(&[1]), HashMap::new(), HashMap::new()];
        let caught   = vec![
            vec![stub_caught(1, 10)],
            vec![stub_caught(2, 10), stub_caught(5, 99)],
            vec![stub_caught(3, 10)],
        ];
        let propagated = HashSet::new();
        let mut candidates = soul_link_kill_candidates(&all_dead, &caught, &propagated);
        candidates.sort_by_key(|(j, _)| *j);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].0, 1);
        assert_eq!(candidates[0].1.personality, 2);
        assert_eq!(candidates[1].0, 2);
        assert_eq!(candidates[1].1.personality, 3);
    }

    #[test]
    fn unrelated_slot_catch_at_different_location_is_not_linked() {
        // Slot 1 has catches at two locations; only the one matching met_loc is linked.
        let all_dead = vec![dead_map(&[1]), HashMap::new()];
        let caught   = vec![
            vec![stub_caught(1, 10)],
            vec![stub_caught(2, 99), stub_caught(3, 10)], // only personality 3 links
        ];
        let propagated = HashSet::new();
        let candidates = soul_link_kill_candidates(&all_dead, &caught, &propagated);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].1.personality, 3);
    }
}
