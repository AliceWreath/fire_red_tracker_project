//! # Aggregator UI
//! 
//! The egui application that displays games state from multiple connected
//! FireRed instances side-by-side in a responsive column layout.
//! 
//! ## Purpose
//! 
//! This module is used in the **aggregator** binary, which connects to several 
//! tracker server instances simultanously and renders each player's party and
//! encounter data in its own column. It is designed for Soul Link runs, where
//! two or more players' pokemon are linked by their shared met-location.
//! 
//! ## Soul link detection
//! 
//! [`draw_party_member`] compares each pokemon's `met_location` against every 
//! other player's party. If two pokemon share the same met location, they are
//! considered linked and a label is shown above the sprite. This heuristic 
//! relies on met location beign unique per encounter area, which holds for
//! standard FireRed, but may produces false positives on heavily modified
//! ROMs.
//! 
//! ## Texture pipeline
//! 
//! Textures are managed identically to the single-player client: the network
//! thread for each [`MonitorSlot`] delivers decompressed sprites via
//! `pending_textures`, and [`process_textures`] uploads them to the GPU each
//! frame. Missing species are batched into `texture_request_queue` so they
//! are fetched from the server on the next network tick.

use crate::client::MonitorSlot;
use egui::Ui;
use fire_red_party_monitor::Pokemon;
use fire_red_states::GameState;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

/// Size at which party member sprites are rendered, in logical pixels
const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// Size at which wild encounter sprites are rendered, in logical pixels.
/// Slightly smaller than party sprites to fit more per row.
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (48.0, 48.0);

/// Minimum width of a player column before the layout drops to fewer columns.
/// Prevents columns from becoming too naroow to read on small windows.
const MIN_COLUMN_WIDTH: f32 = 300.0;

// ---------------------------------------------------------------------------
// App struct
// ---------------------------------------------------------------------------

/// The top-level eframe application for the multi-player aggregator view.
/// 
/// Owns the list of [`MonitorSlots`]s (one per connected server) and the shared
/// GPU texture cache. All rendering and texture management happens through this
/// struct's [`eframe::App::update`] implementation.
pub struct AggregatorApp {
    /// One slot per connected tracker server, in display order.
    slots: Vec<MonitorSlot>,
    /// Cache of GPU texture handles, keyed by [`sprite_key`].
    textures: HashMap<String, egui::TextureHandle>,
}

impl AggregatorApp {
    /// Creates a new [`AggregatorApp`] from a list of pre-configured monitor slots.
    /// 
    /// # Arguments
    /// * `_cc`     - eframe creation context (unused; reserved for future font setup).
    /// * `slots`   - One [`MonitorSlot`] per server the aggregator is connected to.
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        slots: Vec<MonitorSlot>,
    ) -> Self {
        Self {
            slots,
            textures: HashMap::new(),
        }
    }

    /// Drains pending textures from each slot's network thread and uploads them
    /// to the GPU, then esqueues requests for any species not yet cached.
    /// 
    /// Called once per frame from [`update`](eframe::App::update) before any
    /// drawing occurs, so textures are always available when the draw methods run.
    /// 
    /// For each slot this method:
    /// 1. Drains `pending_textures` and call [`egui::Context::load_texture`] for each.
    /// 2. Walks the current party and encounter lists, collecting species IDs that are
    ///    not yet in `known_species`
    /// 3. Pushes a deduplicated batch into `texture_request_queue` for the network
    ///    thread to send to the server.
    /// 
    /// Species 0 and species > 386 are always skipped as they are sentinel / 
    /// out-of-range values. The server always sends both normal and shiny variants
    /// for each requested species, so only one request per species is needed.
    fn process_textures(&mut self, ctx: &egui::Context) {
        for slot in &self.slots {
            // ── Drain arrived textures ───────────────────────────────────────
            {
                let mut pending =
                    slot.pending_textures.lock().unwrap_or_else(|e| e.into_inner());
                for pt in pending.drain(..) {
                    let key = sprite_key(pt.species, pt.shiny);
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [pt.width as usize, pt.height as usize],
                        &pt.pixels,
                    );
                    let handle =
                        ctx.load_texture(&key, color_image, egui::TextureOptions::NEAREST);
                    self.textures.insert(key, handle);
                }
            }

            // ── Request any species we don't have yet ────────────────────────
            {
                let state_guard = slot.state.lock().unwrap_or_else(|e| e.into_inner());
                let gs = match state_guard.as_ref() {
                    Some(gs) => gs,
                    None => continue,
                };

                let mut needed: Vec<u16> = Vec::new();
                let known = slot.known_species.lock().unwrap_or_else(|e| e.into_inner());

                // Party sprites
                for p in &gs.party {
                    let species = p.box_mon.secure.growth.species;
                    if species == 0 || species > 386 {
                        continue;
                    }
                    // We request by species; server sends both normal + shiny
                    if !known.contains(&species) {
                        needed.push(species);
                    }
                }

                // Encounter sprites
                let all_encounters = gs
                    .encounters
                    .land_mon_encounters
                    .wild_pokemon_list
                    .iter()
                    .chain(gs.encounters.water_mon_encounters.wild_pokemon_list.iter())
                    .chain(
                        gs.encounters
                            .rock_smash_encounters
                            .wild_pokemon_list
                            .iter(),
                    )
                    .chain(gs.encounters.fishing_encounters.wild_pokemon_list.iter());

                for wild in all_encounters {
                    if wild.species == 0 || wild.species > 386 {
                        continue;
                    }
                    if !known.contains(&wild.species) {
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

    /// Draws a single player's column: party members followed by a collapsible
    /// encounter section.
    /// 
    /// If `state` is `None` (server disconnected), a warning label is shown
    /// instead and the method returns early.
    /// 
    /// `all_states` is passed to [`draw_party_member`] so that Soul Link
    /// matches can be detected against every other player's party.
    /// 
    /// # Arguments
    /// * `ui`                  - The egui [`Ui`] for this column.
    /// * `label`               - Player / server label shown as the column heading.
    /// * `state`               - Current [`GameState`] snapshot for this player, or `None`.
    /// * `textures`            - Shared texture cache.
    /// * `all_states`          - Snapshots of all players' states, used for Soul Link matching.
    fn draw_player_column(
        ui: &mut Ui,
        label: &str,
        state: &Option<GameState>,
        textures: &HashMap<String, egui::TextureHandle>,
        all_states: &[(String, Option<GameState>)],
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

        // --- Party ---
        ui.label(egui::RichText::new("Party").strong().size(15.0));
        ui.add_space(4.0);

        if gs.party.is_empty() {
            ui.label("No party data");
        }

        for (idx, pokemon) in gs.party.iter().enumerate() {
            let other_states: Vec<(String, Option<GameState>)> = all_states
                .iter()
                .filter(|(l, _)| l != label)
                .cloned()
                .collect();

            Self::draw_party_member(ui, idx, pokemon, textures, &other_states);
            ui.separator();
        }

        ui.add_space(8.0);

        // --- Encounters collapsing section ---
        egui::CollapsingHeader::new(egui::RichText::new("Encounters").strong().size(15.0))
            .default_open(true)
            .show(ui, |ui| {
                Self::draw_encounter_section(
                    ui,
                    "Grass",
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
                    "🪨 Rock Smash",
                    &gs.encounters.rock_smash_encounters.wild_pokemon_list,
                    textures,
                );
            });
    }

    /// Draws a single party member row: optional Soul Link label, sprite, nickname,
    /// shiny indicator, level, and HP bar.
    /// 
    /// ## Soul Link detection
    /// Before rendering the sprite, this method checks whether the pokemon's
    /// `met_location` matches any pokemon in another player's party. If a match
    /// is found a purple label is shown above the sprite identifying the linked
    /// partner and which player owns it.
    /// 
    /// ## Hp color coding
    /// - **Red**    - below 30 % of max HP.
    /// - **Yellow** - below 80 % of max HP.
    /// - **White**  - 80 % or above.
    /// 
    /// # Arguments
    /// * `ui`              - The egui [`Ui`] to render into.
    /// * `_idx`            - Slot index within the party (unused; reserved for future use)
    /// * `pokemon`         - The pokemon to render
    /// * `textures`        - Shared texture cache.
    /// * `other_states`    - All other players' game states, used for Soul Link matching
    fn draw_party_member(
        ui: &mut Ui,
        _idx: usize,
        pokemon: &Pokemon,
        textures: &HashMap<String, egui::TextureHandle>,
        other_states: &[(String, Option<GameState>)],
    ) {
        let species = pokemon.box_mon.secure.growth.species;
        let personality = pokemon.box_mon.personality;
        let met = pokemon.box_mon.secure.misc.met_location;
        let ot_id = pokemon.box_mon.ot_id;
        let shiny = is_shiny(personality, ot_id);
        let key = sprite_key(species, shiny);
dbg!(&other_states);
        for (other_label, other_state) in other_states {
            if let Some(gs) = other_state {
                for other_mon in &gs.party {
                    dbg!(&other_mon, &met);
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

        ui.horizontal(|ui| {
            if let Some(tex) = textures.get(&key) {
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
                            .size(16.0)
                            .color(egui::Color32::WHITE),
                    );
                    if shiny {
                        ui.label(egui::RichText::new("✨").size(14.0));
                    }
                    ui.label(format!("Lv{}", pokemon.level));
                });

                let hp_color = if (pokemon.hp as f32) < (pokemon.max_hp as f32 * 0.3) {
                    egui::Color32::RED
                } else if (pokemon.hp as f32) < (pokemon.max_hp as f32 * 0.8) {
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
            });
        });
    }

    /// Draws a labelled horizontal row of encounter sprites.
    /// 
    /// Skips the section entirely if `list` contains no valid species (species 0
    /// and species > 386 are treated as empty). Each sprite shows a tooltip with
    /// the encounter's min and max level range when hovered.
    /// 
    /// # Arguments
    /// * `ui`              - The egui [`Ui`] to render into.
    /// * `heading`         - Section label shown above the sprite row (e.g. "Water")
    /// * `list`            - The wild encounter list to display.
    /// * `textures`        - Shared texture cache.
    fn draw_encounter_section(
        ui: &mut Ui,
        heading: &str,
        list: &[fire_red_pokemon_data::WildPokemon],
        textures: &HashMap<String, egui::TextureHandle>,
    ) {
        let non_empty: Vec<_> = list
            .iter()
            .filter(|w| w.species > 0 && w.species <= 386)
            .collect();
        if non_empty.is_empty() {
            return;
        }

        ui.label(egui::RichText::new(heading).italics());
        ui.horizontal_wrapped(|ui| {
            for wild in &non_empty {
                let key = sprite_key(wild.species, false);
                if let Some(tex) = textures.get(&key) {
                    ui.add(
                        egui::Image::new(tex).fit_to_exact_size(egui::vec2(
                            ENCOUNTER_IMAGE_SIZE.0,
                            ENCOUNTER_IMAGE_SIZE.1,
                        )),
                    )
                    .on_hover_text(format!("Lv{}-{}", wild.min_level, wild.max_level));
                }
            }
        });
        ui.add_space(4.0);
    }
}

impl eframe::App for AggregatorApp {
    /// Main per-frame callback. Processes textures, snapshots all slot states,
    /// then renders a responsive multi-column layout - one column per connected player.
    /// 
    /// ## Column count
    /// The number of columns is determined by dividing the available window width
    /// by [`MIN_COLUMN_WIDHT`], clamped to `[1, slot_count]`. This means the layout
    /// adapts automatically when the window is resized.
    /// 
    /// ## Lock discipline
    /// Each slot's `state` mutex is locked briefly to clone the [`GameState`]
    /// snapshot, then released before any drawing begins. This prevents the GUI
    /// from blocking the network thread during rendering.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        self.process_textures(ctx);

        let slot_count = self.slots.len();
        let textures = &self.textures;

        // Snapshot states to avoid holding locks while drawing
        let states: Vec<(String, Option<GameState>)> = self
            .slots
            .iter()
            .map(|slot| {
                let state = slot.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let label = slot.label.lock().unwrap_or_else(|e| e.into_inner()).clone();
                (label, state)
            })
            .collect();

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let available = ui.available_width();
                let columns = slot_count
                    .min((available / MIN_COLUMN_WIDTH).floor().max(1.0) as usize)
                    .max(1);

                ui.columns(columns, |cols| {
                    for (i, (label, state)) in states.iter().enumerate() {
                        let col_idx = i % columns;
                        egui::ScrollArea::vertical()
                            .id_source(format!("col_scroll_{}", i))
                            .show(&mut cols[col_idx], |ui| {
                                AggregatorApp::draw_player_column(
                                    ui, label, state, textures, &states,
                                );
                            });
                    }
                });
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns the texture cache key for a given speices and shininess.
/// 
/// Keys follow the forma `"pokmeon_{species}_{normal|shiny}"` and are used
/// consistently across both the aggregator and the single-player client so
/// that textures received from any server can be looked up with the same key.
/// 
/// # Arguments
/// * `species`         - National pokedex number
/// * `shiny`           - `true` for shiny variant.
pub fn sprite_key(species: u16, shiny: bool) -> String {
    format!(
        "pokemon_{}_{}",
        species,
        if shiny { "shiny" } else { "normal" }
    )
}

/// Returns `true` if a pokemon within the given `personality` and `ot_id` is shiny.
/// 
/// Uses the Gen III shiny formula:
/// `(p_high XOR p_low XOR id_high XOR id_low) < 8`
/// 
/// This is a local copy of the same function in the main tracker crate; it is
/// duplicated here so this module has no dependency on the tracker binary.
/// 
/// # Arguments
/// * `personality`         - 32-bit peronality value (PID)
/// * `ot_id`               - Combined 32-bit OT ID (public ID in low 16 bits, secret in high 16 bits)
pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p1 = (personality >> 16) as u16;
    let p2 = (personality & 0xFFFF) as u16;
    let id1 = (ot_id >> 16) as u16;
    let id2 = (ot_id & 0xFFFF) as u16;
    (p1 ^ p2 ^ id1 ^ id2) < 8
}
