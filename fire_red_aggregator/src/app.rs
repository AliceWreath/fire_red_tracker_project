use crate::client::MonitorSlot;
use egui::Ui;
use fire_red_party_monitor::Pokemon;
use fire_red_states::GameState;
use std::collections::HashMap;

const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);
const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (48.0, 48.0);
const MIN_COLUMN_WIDTH: f32 = 300.0;

pub struct AggregatorApp {
    slots: Vec<MonitorSlot>,
    textures: HashMap<String, egui::TextureHandle>,
}

impl AggregatorApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        slots: Vec<MonitorSlot>,
    ) -> Self {
        Self {
            slots,
            textures: HashMap::new(),
        }
    }

    fn ensure_textures(&mut self, ctx: &egui::Context) {
        let rom = fire_red_rom_buffer::get_rom();

        for slot in &self.slots {
            let state_guard = slot.state.lock().unwrap_or_else(|e| e.into_inner());
            let gs = match state_guard.as_ref() {
                Some(gs) => gs,
                None => continue,
            };

            // Party sprites (may be shiny)
            for p in &gs.party {
                let species = p.box_mon.secure.growth.species;
                let personality = p.box_mon.personality;
                let ot_id = p.box_mon.ot_id;
                if species == 0 || species > 386 {
                    continue;
                }
                let shiny = is_shiny(personality, ot_id);
                let key = sprite_key(species, shiny);
                if !self.textures.contains_key(&key) {
                    if let Ok(tex) = load_texture(ctx, rom, species, personality, ot_id) {
                        self.textures.insert(key, tex);
                    }
                }
            }

            // Encounter sprites (always normal)
            let all_encounters = gs.encounters.land_mon_encounters.wild_pokemon_list.iter()
                .chain(gs.encounters.water_mon_encounters.wild_pokemon_list.iter())
                .chain(gs.encounters.rock_smash_encounters.wild_pokemon_list.iter())
                .chain(gs.encounters.fishing_encounters.wild_pokemon_list.iter());

            for wild in all_encounters {
                if wild.species == 0 || wild.species > 386 {
                    continue;
                }
                let key = sprite_key(wild.species, false);
                if !self.textures.contains_key(&key) {
                    if let Ok(tex) = load_texture_normal(ctx, rom, wild.species) {
                        self.textures.insert(key, tex);
                    }
                }
            }
        }
    }

    fn draw_player_column(
        ui: &mut Ui,
        label: &str,
        state: &Option<GameState>,
        textures: &HashMap<String, egui::TextureHandle>,
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
            Self::draw_party_member(ui, idx, pokemon, textures);
            ui.separator();
        }

        ui.add_space(8.0);

        // --- Encounters collapsing section ---
        egui::CollapsingHeader::new(egui::RichText::new("Encounters").strong().size(15.0))
            .default_open(true)
            .show(ui, |ui| {
                Self::draw_encounter_section(ui, "🌿 Grass", &gs.encounters.land_mon_encounters.wild_pokemon_list, textures);
                Self::draw_encounter_section(ui, "🌊 Water / Fishing", 
                    &gs.encounters.water_mon_encounters.wild_pokemon_list
                        .iter()
                        .chain(gs.encounters.fishing_encounters.wild_pokemon_list.iter())
                        .cloned()
                        .collect::<Vec<_>>(),
                    textures,
                );
                Self::draw_encounter_section(ui, "🪨 Rock Smash", &gs.encounters.rock_smash_encounters.wild_pokemon_list, textures);
            });
    }

    fn draw_party_member(
        ui: &mut Ui,
        _idx: usize,
        pokemon: &Pokemon,
        textures: &HashMap<String, egui::TextureHandle>,
    ) {
        let species = pokemon.box_mon.secure.growth.species;
        let personality = pokemon.box_mon.personality;
        let ot_id = pokemon.box_mon.ot_id;
        let shiny = is_shiny(personality, ot_id);
        let key = sprite_key(species, shiny);

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

    fn draw_encounter_section(
        ui: &mut Ui,
        heading: &str,
        list: &[fire_red_pokemon_data::WildPokemon],
        textures: &HashMap<String, egui::TextureHandle>,
    ) {
        let non_empty: Vec<_> = list.iter().filter(|w| w.species > 0 && w.species <= 386).collect();
        if non_empty.is_empty() {
            return;
        }

        ui.label(egui::RichText::new(heading).italics());
        ui.horizontal_wrapped(|ui| {
            for wild in &non_empty {
                let key = sprite_key(wild.species, false);
                if let Some(tex) = textures.get(&key) {
                    ui.add(
                        egui::Image::new(tex)
                            .fit_to_exact_size(egui::vec2(ENCOUNTER_IMAGE_SIZE.0, ENCOUNTER_IMAGE_SIZE.1)),
                    )
                    .on_hover_text(format!(
                        "Lv{}-{}",
                        wild.min_level, wild.max_level
                    ));
                }
            }
        });
        ui.add_space(4.0);
    }
}

impl eframe::App for AggregatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        self.ensure_textures(ctx);

        let slot_count = self.slots.len();
        let textures = &self.textures;

        // Snapshot states to avoid holding locks while drawing
        let states: Vec<(String, Option<GameState>)> = self
            .slots
            .iter()
            .map(|slot| {
                let state = slot.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                (slot.label.clone(), state)
            })
            .collect();

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let available = ui.available_width();
                // Respect a minimum column width so it doesn't get unreadably narrow
                let columns = slot_count
                    .min((available / MIN_COLUMN_WIDTH).floor().max(1.0) as usize)
                    .max(1);

                ui.columns(columns, |cols| {
                    for (i, (label, state)) in states.iter().enumerate() {
                        let col_idx = i % columns;
                        egui::ScrollArea::vertical()
                            .id_source(format!("col_scroll_{}", i))
                            .show(&mut cols[col_idx], |ui| {
                                AggregatorApp::draw_player_column(ui, label, state, textures);
                            });
                    }
                });
            });
        });
    }
}

// --- Helpers (mirrors what monitor has in main.rs) ---

pub fn sprite_key(species: u16, shiny: bool) -> String {
    format!("pokemon_{}_{}", species, if shiny { "shiny" } else { "normal" })
}

pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p1 = (personality >> 16) as u16;
    let p2 = (personality & 0xFFFF) as u16;
    let id1 = (ot_id >> 16) as u16;
    let id2 = (ot_id & 0xFFFF) as u16;
    (p1 ^ p2 ^ id1 ^ id2) < 8
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
    Ok(ctx.load_texture(sprite_key(species, shiny), color_image, egui::TextureOptions::NEAREST))
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
    Ok(ctx.load_texture(sprite_key(species, false), color_image, egui::TextureOptions::NEAREST))
}