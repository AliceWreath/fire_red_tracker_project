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
//! # Soul Link detection
//!
//! Each caught entry is cross-referenced against the other players' caught
//! lists by `met_location`. A match means both players caught their
//! Nuzlocke mon on the same route — their soul link pair.

use crate::client::MonitorSlot;
use egui::Ui;
use fire_red_database::CaughtPokemon;
use fire_red_party_monitor::Pokemon;
use fire_red_states::GameState;
use std::collections::HashMap;

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
// App struct
// ---------------------------------------------------------------------------

/// The top-level eframe application for the multi-player aggregator view.
pub struct AggregatorApp {
    slots: Vec<MonitorSlot>,
    textures: HashMap<String, egui::TextureHandle>,
}

impl AggregatorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, slots: Vec<MonitorSlot>) -> Self {
        Self {
            slots,
            textures: HashMap::new(),
        }
    }

    /// Drains pending textures and enqueues requests for missing species.
    fn process_textures(&mut self, ctx: &egui::Context) {
        for slot in &self.slots {
            {
                let mut pending = slot
                    .pending_textures
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                for pt in pending.drain(..) {
                    let key = sprite_key(pt.species, pt.shiny);
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [pt.width as usize, pt.height as usize],
                        &pt.pixels,
                    );
                    let handle = ctx.load_texture(&key, image, egui::TextureOptions::NEAREST);
                    self.textures.insert(key, handle);
                }
            }
            {
                let state_guard = slot.state.lock().unwrap_or_else(|e| e.into_inner());
                let gs = match state_guard.as_ref() {
                    Some(gs) => gs,
                    None => continue,
                };
                let mut needed: Vec<u16> = Vec::new();
                let known = slot.known_species.lock().unwrap_or_else(|e| e.into_inner());

                for p in &gs.party {
                    let s = p.box_mon.secure.growth.species;
                    if s > 0 && s <= 386 && !known.contains(&s) {
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
                    if wild.species > 0 && wild.species <= 386 && !known.contains(&wild.species) {
                        needed.push(wild.species);
                    }
                }
                drop(known);

                if !needed.is_empty() {
                    needed.sort();
                    needed.dedup();
                    slot.texture_request_queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_back(needed);
                }
            }
        }
    }

    /// Draws one complete player column (party + caught log + encounters).
    fn draw_column(
        ui: &mut Ui,
        label: &str,
        state: &Option<GameState>,
        db: Option<&fire_red_database::DbReader>,
        textures: &HashMap<String, egui::TextureHandle>,
        all_states: &[(String, Option<GameState>)],
        all_caught: &[(String, Vec<CaughtPokemon>)],
    ) {
        let full_rect = ui.max_rect();
        let enc_height = ENCOUNTER_PANEL_HEIGHT;
        let party_height = (full_rect.height() - enc_height).max(50.0);

        // ── Party + caught log region (top, scrollable) ───────────────────────
        let party_rect =
            egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), party_height));
        ui.allocate_ui_at_rect(party_rect, |ui| {
            Self::draw_party_region(ui, label, state, db, textures, all_states, all_caught);
        });

        // ── Encounter region (bottom, pinned) ─────────────────────────────────
        let enc_min = egui::pos2(full_rect.min.x, full_rect.max.y - enc_height);
        let enc_rect =
            egui::Rect::from_min_size(enc_min, egui::vec2(full_rect.width(), enc_height));
        ui.allocate_ui_at_rect(enc_rect, |ui| {
            Self::draw_encounter_region(ui, state, textures);
        });
    }

    /// Draws the scrollable party + caught log region.
    fn draw_party_region(
        ui: &mut Ui,
        label: &str,
        state: &Option<GameState>,
        db: Option<&fire_red_database::DbReader>,
        textures: &HashMap<String, egui::TextureHandle>,
        all_states: &[(String, Option<GameState>)],
        all_caught: &[(String, Vec<CaughtPokemon>)],
    ) {
        ui.heading(label);
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
            .id_source(format!("{}_party_scroll", label))
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
                            egui::RichText::new("All badges obtained!")
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
                    let dead = db.map(|d| d.is_dead(pokemon.box_mon.personality)).unwrap_or(false);
                    Self::draw_party_member(ui, idx, pokemon, dead, textures, &others);
                    ui.separator();
                }

                // ── Caught log ────────────────────────────────────────────────
                if let Some(db) = db {
                    let caught = db.list_caught();
                    if !caught.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(format!("Caught ({})", caught.len()))
                                .strong()
                                .size(15.0),
                        );
                        ui.add_space(2.0);

                        // Other players' caught lists keyed by met_location for soul link lookup.
                        // Build a map: met_location -> Vec<(label, CaughtPokemon)>
                        let mut others_by_loc: HashMap<u8, Vec<(&str, &CaughtPokemon)>> =
                            HashMap::new();
                        for (other_label, other_list) in all_caught {
                            if other_label == label { continue; }
                            for cp in other_list {
                                others_by_loc
                                    .entry(cp.met_location)
                                    .or_default()
                                    .push((other_label, cp));
                            }
                        }

                        for cp in &caught {
                            let dead = db.is_dead(cp.personality);
                            Self::draw_caught_entry(ui, cp, dead, &others_by_loc);
                        }
                    }
                }
            });
    }

    /// Draws one entry in the caught log.
    fn draw_caught_entry(
        ui: &mut Ui,
        cp: &CaughtPokemon,
        dead: bool,
        others_by_loc: &HashMap<u8, Vec<(&str, &CaughtPokemon)>>,
    ) {
        let name_color = if dead {
            egui::Color32::from_rgb(150, 50, 50)
        } else {
            egui::Color32::WHITE
        };

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Loc.{:<3}", cp.met_location))
                    .color(egui::Color32::from_rgb(160, 160, 160))
                    .size(12.0),
            );
            ui.label(
                egui::RichText::new(&cp.nickname)
                    .strong()
                    .color(name_color)
                    .size(13.0),
            );
            ui.label(
                egui::RichText::new(format!("Lv.{}", cp.level))
                    .color(egui::Color32::from_rgb(180, 180, 180))
                    .size(12.0),
            );
            ui.label(
                egui::RichText::new(&cp.nature)
                    .color(egui::Color32::from_rgb(180, 180, 180))
                    .size(12.0),
            );
            if cp.is_shiny {
                ui.label(egui::RichText::new("★").color(egui::Color32::YELLOW).size(12.0));
            }
            if dead {
                ui.label(
                    egui::RichText::new("DEAD")
                        .strong()
                        .color(egui::Color32::RED)
                        .size(12.0),
                );
            }
        });

        // Soul link annotation for other players' catches on the same route.
        if let Some(linked) = others_by_loc.get(&cp.met_location) {
            for (other_label, other_cp) in linked {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "↳ {}: {}{}",
                            other_label,
                            other_cp.nickname,
                            if other_cp.is_shiny { " ★" } else { "" },
                        ))
                        .color(egui::Color32::from_rgb(191, 64, 191))
                        .size(12.0),
                    );
                });
            }
        }
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

    /// Draws a single party member row with Soul Link detection and dead marking.
    fn draw_party_member(
        ui: &mut Ui,
        _idx: usize,
        pokemon: &Pokemon,
        dead: bool,
        textures: &HashMap<String, egui::TextureHandle>,
        other_states: &[(String, Option<GameState>)],
    ) {
        let species = pokemon.box_mon.secure.growth.species;
        let personality = pokemon.box_mon.personality;
        let ot_id = pokemon.box_mon.ot_id;
        let met = pokemon.box_mon.secure.misc.met_location;
        let shiny = is_shiny(personality, ot_id);
        let key = sprite_key(species, shiny);

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
                ui.horizontal(|ui| {
                    let name_color = if dead {
                        egui::Color32::from_rgb(150, 50, 50)
                    } else {
                        egui::Color32::WHITE
                    };
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
                if !dead {
                    let hp_ratio = pokemon.hp as f32 / pokemon.max_hp as f32;
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

    /// Draws a labelled row of encounter sprites, skipping empty sections.
    fn draw_encounter_section(
        ui: &mut Ui,
        heading: &str,
        list: &[fire_red_pokemon_data::WildPokemon],
        textures: &HashMap<String, egui::TextureHandle>,
    ) {
        let valid: Vec<_> = list
            .iter()
            .filter(|w| w.species > 0 && w.species <= 386)
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
    /// Main per-frame callback.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        self.process_textures(ctx);

        let textures = &self.textures;

        let states: Vec<(String, Option<GameState>)> = self
            .slots
            .iter()
            .map(|slot| {
                let state = slot.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let label = slot.label.lock().unwrap_or_else(|e| e.into_inner()).clone();
                (label, state)
            })
            .collect();

        // Keep each DbReader pointed at the correct run for this player.
        // sync_player re-queries only when the name changes, so this is cheap.
        for (slot, (label, _)) in self.slots.iter().zip(states.iter()) {
            if let Some(db) = &slot.db {
                db.sync_player(label);
            }
        }

        // Read each player's caught list from their DB (empty if no DB configured).
        let all_caught: Vec<(String, Vec<CaughtPokemon>)> = self
            .slots
            .iter()
            .map(|slot| {
                let label = slot.label.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let caught = slot
                    .db
                    .as_ref()
                    .map(|db| db.list_caught())
                    .unwrap_or_default();
                (label, caught)
            })
            .collect();

        // Register a SidePanel for every player.
        for i in 0..self.slots.len() {
            let (label, state) = &states[i];
            let db = self.slots[i].db.as_ref();
            let panel_id = egui::Id::new(format!("player_col_{}", i));
            egui::SidePanel::left(panel_id)
                .exact_width(COLUMN_WIDTH)
                .resizable(false)
                .show(ctx, |ui| {
                    AggregatorApp::draw_column(
                        ui, label, state, db, textures, &states, &all_caught,
                    );
                });
        }

        // Consume remaining space so egui doesn't complain about a missing CentralPanel.
        egui::CentralPanel::default().show(ctx, |_ui| {});
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns the texture cache key for a given species and shininess.
pub fn sprite_key(species: u16, shiny: bool) -> String {
    format!(
        "pokemon_{}_{}",
        species,
        if shiny { "shiny" } else { "normal" }
    )
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
