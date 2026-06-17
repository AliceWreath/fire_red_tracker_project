//! # GUI
//!
//! The egui application state and rendering for the tracker's party panel and
//! encounters child viewport.

use crate::config::{TrackerConfig, save_config};
use crate::game::is_shiny;
use crate::textures::{
    ENCOUNTER_IMAGE_SIZE, PARTY_IMAGE_SIZE, PendingTexture, load_texture, load_texture_back,
    load_texture_normal, make_placeholder,
};
use fire_red_states::LockOrRecover;
use fire_red_states::MAX_NATIONAL_DEX_FIRERED;
use fire_red_states::SpriteVariant;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Default target window size for the party panel, in logical pixels.
pub const PARTY_WINDOW: (f32, f32) = (400.0, 800.0);

/// Default target window size for the encounters panel, in logical pixels.
pub const ENCOUNTER_WINDOW: (f32, f32) = (600.0, 400.0);

struct SettingsDraft {
    rom: String,
    db: String,
    clean: bool,
    mode: crate::config::ConfigMode,
    aggregator_host: String,
    aggregator_port: String,
    preferred_player: String,
    // Run / polling
    poll_ms: String,
    dupes_clause: crate::config::DupesClauseMode,
    allow_species_repeats: bool,
    run_start_balls: String,
    // Test mode
    default_test: bool,
    test_db: String,
    test_agg_host: String,
    test_agg_port: String,
    test_player: String,
    // OBS clip trigger
    obs_host: String,
    obs_port: String,
    obs_password: String,
    obs_clip_death: bool,
    obs_clip_shiny: bool,
    obs_clip_wipe: bool,
    obs_clip_badge: bool,
    // Webhook URL and template fields
    death_url: String,
    death_url_enabled: bool,
    death_template: String,
    catch_url: String,
    catch_url_enabled: bool,
    catch_template: String,
    shiny_url: String,
    shiny_url_enabled: bool,
    shiny_template: String,
    wipe_url: String,
    wipe_url_enabled: bool,
    wipe_template: String,
    badge_url: String,
    badge_url_enabled: bool,
    badge_template: String,
    nickname_url: String,
    nickname_url_enabled: bool,
    nickname_template: String,
    // Pass-through fields — not exposed in GUI, preserved from config.
    nuzlocke_url: Option<String>,
    nuzlocke_template: Option<String>,
    notify_on_death: bool,
    notify_on_shiny: bool,
    notify_on_wipe: bool,
}

impl SettingsDraft {
    fn from_config(cfg: &TrackerConfig) -> Self {
        let wh = &cfg.webhooks;
        Self {
            rom: cfg.rom.clone(),
            db: cfg
                .db
                .trim_start_matches("postgresql://")
                .trim_start_matches("postgres://")
                .to_string(),
            clean: cfg.clean,
            mode: cfg.mode.clone(),
            aggregator_host: cfg.aggregator_host.clone(),
            aggregator_port: cfg.aggregator_port.to_string(),
            preferred_player: cfg
                .preferred_player
                .map(|n| n.to_string())
                .unwrap_or_default(),
            poll_ms: if cfg.poll_ms == 100 {
                String::new()
            } else {
                cfg.poll_ms.to_string()
            },
            dupes_clause: cfg.dupes_clause,
            allow_species_repeats: cfg.allow_species_repeats,
            run_start_balls: cfg
                .run_start_balls
                .map(|n| n.to_string())
                .unwrap_or_default(),
            default_test: cfg.default_test,
            test_db: cfg
                .test
                .as_ref()
                .and_then(|t| t.db.as_ref())
                .map(|s| {
                    s.trim_start_matches("postgresql://")
                        .trim_start_matches("postgres://")
                        .to_string()
                })
                .unwrap_or_default(),
            test_agg_host: cfg
                .test
                .as_ref()
                .and_then(|t| t.aggregator_host.clone())
                .unwrap_or_default(),
            test_agg_port: cfg
                .test
                .as_ref()
                .and_then(|t| t.aggregator_port)
                .map(|p| p.to_string())
                .unwrap_or_default(),
            test_player: cfg
                .test
                .as_ref()
                .and_then(|t| t.preferred_player)
                .map(|n| n.to_string())
                .unwrap_or_default(),
            obs_host: cfg.obs.host.clone(),
            obs_port: cfg.obs.port.to_string(),
            obs_password: cfg.obs.password.clone().unwrap_or_default(),
            obs_clip_death: cfg.obs.clip_on_death,
            obs_clip_shiny: cfg.obs.clip_on_shiny,
            obs_clip_wipe: cfg.obs.clip_on_wipe,
            obs_clip_badge: cfg.obs.clip_on_badge,
            death_url: wh.death_url.clone().unwrap_or_default(),
            death_url_enabled: wh.death_url.is_some(),
            death_template: wh.death_template.clone().unwrap_or_default(),
            catch_url: wh.catch_url.clone().unwrap_or_default(),
            catch_url_enabled: wh.catch_url.is_some(),
            catch_template: wh.catch_template.clone().unwrap_or_default(),
            shiny_url: wh.shiny_url.clone().unwrap_or_default(),
            shiny_url_enabled: wh.shiny_url.is_some(),
            shiny_template: wh.shiny_template.clone().unwrap_or_default(),
            wipe_url: wh.wipe_url.clone().unwrap_or_default(),
            wipe_url_enabled: wh.wipe_url.is_some(),
            wipe_template: wh.wipe_template.clone().unwrap_or_default(),
            badge_url: wh.badge_url.clone().unwrap_or_default(),
            badge_url_enabled: wh.badge_url.is_some(),
            badge_template: wh.badge_template.clone().unwrap_or_default(),
            nickname_url: wh.nickname_url.clone().unwrap_or_default(),
            nickname_url_enabled: wh.nickname_url.is_some(),
            nickname_template: wh.nickname_template.clone().unwrap_or_default(),
            nuzlocke_url: wh.nuzlocke_url.clone(),
            nuzlocke_template: wh.nuzlocke_template.clone(),
            notify_on_death: wh.notify_on_death,
            notify_on_shiny: wh.notify_on_shiny,
            notify_on_wipe: wh.notify_on_wipe,
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
    pub config_path: PathBuf,
    pub settings_open: bool,
    pub about_open: bool,
    settings: SettingsDraft,
    /// Latest release version string if a newer version is available, set by the
    /// background update-check thread.
    pub update_available: Arc<Mutex<Option<String>>>,
    title_set: bool,
}

impl WindowInfo {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        party_list: Arc<Mutex<Vec<fire_red_party_monitor::Pokemon>>>,
        encounter_list: Arc<Mutex<fire_red_pokemon_data::WildPokemonHeader>>,
        pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
        known_species: Arc<Mutex<std::collections::HashSet<u16>>>,
        texture_request_queue: Option<Arc<Mutex<VecDeque<Vec<u16>>>>>,
        config_path: PathBuf,
        config: &TrackerConfig,
        update_available: Arc<Mutex<Option<String>>>,
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
            about_open: false,
            settings: SettingsDraft::from_config(config),
            update_available,
            title_set: false,
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
        if !self.title_set
            && let Some(v) = &*self.update_available.lock_or_recover()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "Tracker — v{} available",
                v.trim_start_matches('v')
            )));
            self.title_set = true;
        }

        ctx.request_repaint();

        // ── 1. Upload textures received from the server ───────────────────────
        {
            let mut pending = self.pending_textures.lock_or_recover();
            for pt in pending.drain(..) {
                let palette = if pt.shiny { "shiny" } else { "normal" };
                let key = match pt.variant {
                    SpriteVariant::Front => format!("pokemon_{}_{}", pt.species, palette),
                    SpriteVariant::Back => format!("pokemon_{}_{}_back", pt.species, palette),
                };
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
            let list = self.party_list.lock_or_recover().clone();
            let encounter_list = self.encounter_list.lock_or_recover();
            let mut needed: Vec<u16> = Vec::new();

            // Encounter sprites are always the normal (non-shiny) palette.
            let all_encounter_mons = encounter_list
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

            for mon in all_encounter_mons {
                if mon.species == 0 || mon.species > MAX_NATIONAL_DEX_FIRERED {
                    continue;
                }
                let key = format!("pokemon_{}_normal", mon.species);
                if self.textures.contains_key(&key) {
                    continue;
                }
                if self.texture_request_queue.is_some() {
                    let known = self.known_species.lock_or_recover();
                    if !known.contains(&mon.species) {
                        needed.push(mon.species);
                    }
                } else {
                    let texture =
                        load_texture_normal(ctx, fire_red_rom_buffer::get_rom(), mon.species)
                            .unwrap_or_else(|_| make_placeholder(ctx, mon.species));
                    self.textures.insert(key, texture);
                }
            }

            drop(encounter_list);

            // Party sprites use the shiny variant when applicable.
            let missing_party: Vec<(u16, u32, u32)> = list
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
                        },
                    );
                    !self.textures.contains_key(&key)
                })
                .collect();

            for (species, personality, ot_id) in missing_party {
                if species == 0 || species > MAX_NATIONAL_DEX_FIRERED {
                    continue;
                }
                let key = format!(
                    "pokemon_{}_{}",
                    species,
                    if is_shiny(personality, ot_id) {
                        "shiny"
                    } else {
                        "normal"
                    },
                );
                if self.texture_request_queue.is_some() {
                    let known = self.known_species.lock_or_recover();
                    if !known.contains(&species) {
                        needed.push(species);
                    }
                } else {
                    let rom = fire_red_rom_buffer::get_rom();
                    let texture = load_texture(ctx, rom, species, personality, ot_id)
                        .unwrap_or_else(|_| make_placeholder(ctx, species));
                    self.textures.insert(key, texture);

                    let back_key = format!(
                        "pokemon_{}_{}_back",
                        species,
                        if is_shiny(personality, ot_id) {
                            "shiny"
                        } else {
                            "normal"
                        },
                    );
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        self.textures.entry(back_key)
                        && let Ok(tex) = load_texture_back(ctx, rom, species, personality, ot_id)
                    {
                        e.insert(tex);
                    }
                }
            }

            if !needed.is_empty() {
                needed.sort();
                needed.dedup();
                if let Some(queue) = &self.texture_request_queue {
                    queue.lock_or_recover().push_back(needed);
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
            let textures = &self.textures;

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("encounters_window"),
                egui::ViewportBuilder::default()
                    .with_title("Encounters")
                    .with_inner_size([ENCOUNTER_WINDOW.0, ENCOUNTER_WINDOW.1]),
                move |ctx, _class| {
                    #[allow(deprecated)]
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let enc = encounter_list.lock_or_recover();

                            ui.heading("Land Encounters");
                            ui.horizontal(|ui| {
                                for mon in enc.land_mon_encounters.wild_pokemon_list.iter() {
                                    let key = format!("pokemon_{}_normal", mon.species);
                                    if let Some(tex) = textures.get(&key) {
                                        ui.add(egui::Image::new(tex).fit_to_exact_size(
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
                                for mon in enc
                                    .water_mon_encounters
                                    .wild_pokemon_list
                                    .iter()
                                    .chain(enc.fishing_encounters.wild_pokemon_list.iter())
                                {
                                    let key = format!("pokemon_{}_normal", mon.species);
                                    if let Some(tex) = textures.get(&key) {
                                        ui.add(egui::Image::new(tex).fit_to_exact_size(
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

fn nature_mods(nature: &str) -> Option<(&'static str, &'static str)> {
    match nature {
        "Lonely" => Some(("Atk", "Def")),
        "Brave" => Some(("Atk", "Spe")),
        "Adamant" => Some(("Atk", "SpA")),
        "Naughty" => Some(("Atk", "SpD")),
        "Bold" => Some(("Def", "Atk")),
        "Relaxed" => Some(("Def", "Spe")),
        "Impish" => Some(("Def", "SpA")),
        "Lax" => Some(("Def", "SpD")),
        "Timid" => Some(("Spe", "Atk")),
        "Hasty" => Some(("Spe", "Def")),
        "Jolly" => Some(("Spe", "SpA")),
        "Naive" => Some(("Spe", "SpD")),
        "Modest" => Some(("SpA", "Atk")),
        "Mild" => Some(("SpA", "Def")),
        "Quiet" => Some(("SpA", "Spe")),
        "Rash" => Some(("SpA", "SpD")),
        "Calm" => Some(("SpD", "Atk")),
        "Gentle" => Some(("SpD", "Def")),
        "Sassy" => Some(("SpD", "Spe")),
        "Careful" => Some(("SpD", "SpA")),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn stat_row_job(
    nature: &str,
    hp: u16,
    atk: u16,
    def: u16,
    spe: u16,
    spa: u16,
    spd: u16,
    base: egui::Color32,
    size: f32,
) -> egui::text::LayoutJob {
    let mods = nature_mods(nature);
    let up_stat = mods.map(|(u, _)| u);
    let dn_stat = mods.map(|(_, d)| d);
    let stat_col = |label: &str| {
        if up_stat == Some(label) {
            egui::Color32::from_rgb(255, 153, 204)
        } else if dn_stat == Some(label) {
            egui::Color32::from_rgb(158, 200, 255)
        } else {
            base
        }
    };
    let sep = egui::text::TextFormat {
        color: base,
        font_id: egui::FontId::proportional(size),
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    for (i, (label, val)) in [
        ("HP", hp),
        ("Atk", atk),
        ("Def", def),
        ("Spe", spe),
        ("SpA", spa),
        ("SpD", spd),
    ]
    .iter()
    .enumerate()
    {
        if i > 0 {
            job.append(" | ", 0.0, sep.clone());
        }
        job.append(
            &format!("{} {}", label, val),
            0.0,
            egui::text::TextFormat {
                color: stat_col(label),
                font_id: egui::FontId::proportional(size),
                ..Default::default()
            },
        );
    }
    job
}

/// Returns the display symbol and color for a gender byte (0=male, 1=female, 2=genderless).
fn gender_label(gender: u8) -> (&'static str, egui::Color32) {
    match gender {
        0 => ("♂", egui::Color32::from_rgb(100, 160, 255)),
        1 => ("♀", egui::Color32::from_rgb(255, 130, 180)),
        _ => ("", egui::Color32::TRANSPARENT),
    }
}

impl WindowInfo {
    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        use crate::config::{
            ConfigMode, DupesClauseMode, ObsConfig, TrackerTestOverrides, WebhookConfig,
        };
        let s = &mut self.settings;

        egui::ScrollArea::vertical().id_salt("settings_scroll").max_height(500.0).show(ui, |ui| {
        egui::Grid::new("tracker_settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(110.0)
            .show(ui, |ui| {
                // ── ROM / database ────────────────────────────────────────────
                ui.label("ROM path:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.rom).desired_width(240.0).hint_text("path/to/firered.gba"));
                    if ui.button("Browse…").clicked()
                        && let Some(path) = rfd::FileDialog::new().add_filter("GBA ROM", &["gba"]).pick_file()
                    {
                        s.rom = path.display().to_string();
                    }
                });
                ui.end_row();

                ui.label("Database:");
                ui.add(egui::TextEdit::singleline(&mut s.db).desired_width(280.0).hint_text("localhost/nuzlocke"));
                ui.end_row();

                ui.label("Clean start:");
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.clean, "Wipe database on next launch");
                    ui.small("Deletes all run data at startup. Uncheck after use.");
                });
                ui.end_row();

                // ── Connection mode ───────────────────────────────────────────
                ui.separator();
                ui.end_row();
                ui.label("Default mode:");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.mode, ConfigMode::Standalone, "Standalone");
                    ui.selectable_value(&mut s.mode, ConfigMode::Connected,  "Connected");
                });
                ui.end_row();

                if s.mode == ConfigMode::Connected {
                    ui.label("Aggregator host:");
                    ui.add(egui::TextEdit::singleline(&mut s.aggregator_host).desired_width(200.0));
                    ui.end_row();

                    ui.label("Aggregator port:");
                    ui.add(egui::TextEdit::singleline(&mut s.aggregator_port).desired_width(80.0));
                    ui.end_row();

                    ui.label("Player number:");
                    ui.add(egui::TextEdit::singleline(&mut s.preferred_player).desired_width(60.0).hint_text("1, 2, …"));
                    ui.end_row();
                }

                // ── Run settings ──────────────────────────────────────────────
                ui.separator();
                ui.end_row();

                ui.label("Poll interval:");
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.poll_ms).desired_width(80.0).hint_text("100"));
                    ui.small("Game-polling interval in ms (20–2000). Blank = 100 ms default.");
                });
                ui.end_row();

                ui.label("Dupes clause:");
                ui.vertical(|ui| {
                    ui.selectable_value(&mut s.dupes_clause, DupesClauseMode::Off,       "Off — standard Nuzlocke");
                    ui.selectable_value(&mut s.dupes_clause, DupesClauseMode::PerPlayer, "Per Player — skip if you caught it");
                    ui.selectable_value(&mut s.dupes_clause, DupesClauseMode::Shared,    "Shared — skip if any player caught it (Soul Link)");
                });
                ui.end_row();

                ui.label("Randomizer mode:");
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.allow_species_repeats, "Allow same species on multiple routes");
                    ui.small("Skips the global species-seen check. Each route still allows one encounter, and the dupes clause still applies.");
                });
                ui.end_row();

                ui.label("Run-start balls:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut s.run_start_balls)
                            .desired_width(50.0)
                            .hint_text("5"),
                    );
                    ui.small("Pokéballs required before tracking begins (blank = 5).");
                });
                ui.end_row();

                // ── Test mode ─────────────────────────────────────────────────
                ui.separator();
                ui.end_row();
                ui.label(egui::RichText::new("Test mode").strong());
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.default_test, "Always run in test mode (same as --test)");
                    ui.small("Applies [test] overrides below on every launch.");
                });
                ui.end_row();

                ui.label("  Test DB:");
                ui.add(egui::TextEdit::singleline(&mut s.test_db).desired_width(260.0).hint_text("leave blank to use main DB"));
                ui.end_row();

                ui.label("  Test agg. host:");
                ui.add(egui::TextEdit::singleline(&mut s.test_agg_host).desired_width(200.0).hint_text("leave blank"));
                ui.end_row();

                ui.label("  Test agg. port:");
                ui.add(egui::TextEdit::singleline(&mut s.test_agg_port).desired_width(80.0).hint_text("leave blank"));
                ui.end_row();

                ui.label("  Test player #:");
                ui.add(egui::TextEdit::singleline(&mut s.test_player).desired_width(60.0).hint_text("leave blank"));
                ui.end_row();

                // ── OBS clip trigger ──────────────────────────────────────────
                ui.separator();
                ui.end_row();
                ui.label(egui::RichText::new("OBS clips").strong());
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.obs_clip_death, "Save replay buffer on death");
                    ui.checkbox(&mut s.obs_clip_shiny, "Save replay buffer on shiny");
                    ui.checkbox(&mut s.obs_clip_wipe,  "Save replay buffer on party wipe");
                    ui.checkbox(&mut s.obs_clip_badge, "Save replay buffer on badge earned");
                });
                ui.end_row();

                let obs_used = s.obs_clip_death || s.obs_clip_shiny || s.obs_clip_wipe || s.obs_clip_badge;
                ui.label("  OBS host:");
                ui.add_enabled_ui(obs_used, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.obs_host).desired_width(180.0).hint_text("localhost"));
                });
                ui.end_row();

                ui.label("  OBS port:");
                ui.add_enabled_ui(obs_used, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.obs_port).desired_width(80.0).hint_text("4455"));
                });
                ui.end_row();

                ui.label("  OBS password:");
                ui.add_enabled_ui(obs_used, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.obs_password).desired_width(180.0).hint_text("leave blank if disabled").password(true));
                });
                ui.end_row();

                // ── Webhooks ──────────────────────────────────────────────────
                ui.separator();
                ui.end_row();
                ui.label(egui::RichText::new("Webhooks").strong());
                ui.small("POST JSON on game events (Discord, stream alerts, etc.)");
                ui.end_row();

                ui.checkbox(&mut s.death_url_enabled, "Death URL:");
                ui.add_enabled_ui(s.death_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.death_url).desired_width(260.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(s.death_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.death_template).desired_width(260.0)
                        .hint_text(r#"{"content": "{player} lost {pokemon.nickname}!"} — blank = default JSON"#));
                });
                ui.end_row();

                ui.checkbox(&mut s.catch_url_enabled, "Catch URL:");
                ui.add_enabled_ui(s.catch_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.catch_url).desired_width(260.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(s.catch_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.catch_template).desired_width(260.0)
                        .hint_text(r#"{"content": "{player} caught {pokemon.species} (Lv.{pokemon.level})!"}"#));
                });
                ui.end_row();

                ui.checkbox(&mut s.shiny_url_enabled, "Shiny URL:");
                ui.add_enabled_ui(s.shiny_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.shiny_url).desired_width(260.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(s.shiny_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.shiny_template).desired_width(260.0)
                        .hint_text(r#"{"content": "✨ {player} encountered a shiny {pokemon.species}!"}"#));
                });
                ui.end_row();

                ui.checkbox(&mut s.wipe_url_enabled, "Wipe URL:");
                ui.add_enabled_ui(s.wipe_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.wipe_url).desired_width(260.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(s.wipe_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.wipe_template).desired_width(260.0)
                        .hint_text(r#"{"content": "{player}'s run has ended. RIP."}"#));
                });
                ui.end_row();

                ui.checkbox(&mut s.badge_url_enabled, "Badge URL:");
                ui.add_enabled_ui(s.badge_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.badge_url).desired_width(260.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(s.badge_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.badge_template).desired_width(260.0)
                        .hint_text(r#"{"content": "{player} earned the {badge.name}!"}"#));
                });
                ui.end_row();

                ui.checkbox(&mut s.nickname_url_enabled, "Rename URL:");
                ui.add_enabled_ui(s.nickname_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.nickname_url).desired_width(260.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(s.nickname_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.nickname_template).desired_width(260.0)
                        .hint_text(r#"{"content": "{player} renamed {pokemon.species} to {pokemon.new_name}!"}"#));
                });
                ui.end_row();
            });
        }); // ScrollArea

        ui.add_space(8.0);
        let rom_ok = !s.rom.trim().is_empty();
        let port_ok = s.mode != ConfigMode::Connected
            || s.aggregator_port
                .parse::<u16>()
                .map(|p| p > 0)
                .unwrap_or(false);
        let player_parse: Option<u8> = s
            .preferred_player
            .trim()
            .parse()
            .ok()
            .filter(|&n: &u8| n >= 1);
        let player_ok = s.preferred_player.trim().is_empty() || player_parse.is_some();
        ui.horizontal(|ui| {
            let saved = ui
                .add_enabled(rom_ok && port_ok && player_ok, egui::Button::new("Save"))
                .clicked();
            if !rom_ok {
                ui.label(
                    egui::RichText::new("ROM path is required")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            } else if !port_ok {
                ui.label(
                    egui::RichText::new("Invalid port")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            } else if !player_ok {
                ui.label(
                    egui::RichText::new("Player number must be 1 or higher")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            }
            if saved {
                let db_raw = s.db.trim().to_string();
                let db = if db_raw.starts_with("postgresql://") || db_raw.starts_with("postgres://")
                {
                    db_raw
                } else {
                    format!("postgresql://{}", db_raw)
                };
                let test_db_raw = s.test_db.trim().to_string();
                let test = {
                    let t = TrackerTestOverrides {
                        db: if test_db_raw.is_empty() {
                            None
                        } else if test_db_raw.starts_with("postgresql://")
                            || test_db_raw.starts_with("postgres://")
                        {
                            Some(test_db_raw)
                        } else {
                            Some(format!("postgresql://{}", test_db_raw))
                        },
                        aggregator_host: if s.test_agg_host.trim().is_empty() {
                            None
                        } else {
                            Some(s.test_agg_host.trim().to_string())
                        },
                        aggregator_port: s
                            .test_agg_port
                            .trim()
                            .parse()
                            .ok()
                            .filter(|&p: &u16| p > 0),
                        preferred_player: s
                            .test_player
                            .trim()
                            .parse()
                            .ok()
                            .filter(|&n: &u8| n >= 1),
                    };
                    if t.db.is_none()
                        && t.aggregator_host.is_none()
                        && t.aggregator_port.is_none()
                        && t.preferred_player.is_none()
                    {
                        None
                    } else {
                        Some(t)
                    }
                };
                let cfg = TrackerConfig {
                    rom: s.rom.trim().to_string(),
                    db,
                    clean: s.clean,
                    mode: s.mode.clone(),
                    aggregator_host: s.aggregator_host.trim().to_string(),
                    aggregator_port: s.aggregator_port.parse().unwrap_or(7878),
                    preferred_player: player_parse,
                    default_test: s.default_test,
                    test,
                    poll_ms: if s.poll_ms.trim().is_empty() {
                        100
                    } else {
                        s.poll_ms
                            .trim()
                            .parse::<u64>()
                            .unwrap_or(100)
                            .clamp(20, 2000)
                    },
                    webhooks: WebhookConfig {
                        death_url: if s.death_url_enabled && !s.death_url.trim().is_empty() {
                            Some(s.death_url.trim().to_string())
                        } else {
                            None
                        },
                        death_template: if s.death_url_enabled
                            && !s.death_template.trim().is_empty()
                        {
                            Some(s.death_template.trim().to_string())
                        } else {
                            None
                        },
                        catch_url: if s.catch_url_enabled && !s.catch_url.trim().is_empty() {
                            Some(s.catch_url.trim().to_string())
                        } else {
                            None
                        },
                        catch_template: if s.catch_url_enabled
                            && !s.catch_template.trim().is_empty()
                        {
                            Some(s.catch_template.trim().to_string())
                        } else {
                            None
                        },
                        shiny_url: if s.shiny_url_enabled && !s.shiny_url.trim().is_empty() {
                            Some(s.shiny_url.trim().to_string())
                        } else {
                            None
                        },
                        shiny_template: if s.shiny_url_enabled
                            && !s.shiny_template.trim().is_empty()
                        {
                            Some(s.shiny_template.trim().to_string())
                        } else {
                            None
                        },
                        wipe_url: if s.wipe_url_enabled && !s.wipe_url.trim().is_empty() {
                            Some(s.wipe_url.trim().to_string())
                        } else {
                            None
                        },
                        wipe_template: if s.wipe_url_enabled && !s.wipe_template.trim().is_empty() {
                            Some(s.wipe_template.trim().to_string())
                        } else {
                            None
                        },
                        badge_url: if s.badge_url_enabled && !s.badge_url.trim().is_empty() {
                            Some(s.badge_url.trim().to_string())
                        } else {
                            None
                        },
                        badge_template: if s.badge_url_enabled
                            && !s.badge_template.trim().is_empty()
                        {
                            Some(s.badge_template.trim().to_string())
                        } else {
                            None
                        },
                        nickname_url: if s.nickname_url_enabled && !s.nickname_url.trim().is_empty()
                        {
                            Some(s.nickname_url.trim().to_string())
                        } else {
                            None
                        },
                        nickname_template: if s.nickname_url_enabled
                            && !s.nickname_template.trim().is_empty()
                        {
                            Some(s.nickname_template.trim().to_string())
                        } else {
                            None
                        },
                        nuzlocke_url: s.nuzlocke_url.clone(),
                        nuzlocke_template: s.nuzlocke_template.clone(),
                        notify_on_death: s.notify_on_death,
                        notify_on_shiny: s.notify_on_shiny,
                        notify_on_wipe: s.notify_on_wipe,
                        discord_webhook_url: None,
                    },
                    obs: ObsConfig {
                        host: s.obs_host.trim().to_string(),
                        port: s.obs_port.trim().parse().unwrap_or(4455),
                        password: if s.obs_password.trim().is_empty() {
                            None
                        } else {
                            Some(s.obs_password.trim().to_string())
                        },
                        clip_on_death: s.obs_clip_death,
                        clip_on_shiny: s.obs_clip_shiny,
                        clip_on_wipe: s.obs_clip_wipe,
                        clip_on_badge: s.obs_clip_badge,
                        scene_on_death: None,
                        scene_on_wipe: None,
                        scene_on_shiny: None,
                        scene_on_badge: None,
                        scene_on_catch: None,
                    },
                    dupes_clause: s.dupes_clause,
                    allow_species_repeats: s.allow_species_repeats,
                    preset: None,
                    run_start_balls: s.run_start_balls.trim().parse::<u8>().ok(),
                    livesplit_host: None,
                    livesplit_port: None,
                    livesplit_split_on_badges: false,
                    livesplit_split_on_clear: true,
                    discord_client_id: None,
                    twitch_helix: None,
                };
                save_config(&cfg, &self.config_path);
                self.settings_open = false;
            }
        });
        ui.small("Changes take effect on next launch.");
    }

    fn draw_about(ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.heading("Fire Red Tracker");
            ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
            ui.add_space(8.0);
            ui.label("© 2026 AliceWreath");
            ui.label("MIT License");
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Third-party licenses").strong());
            ui.label("This binary includes open-source dependencies.");
            ui.label("See THIRD_PARTY_LICENSES.html bundled with this");
            ui.label("release for full attribution.");
        });
    }

    /// Draws the party panel.
    ///
    /// Renders badge summary, next gym info, then for each party member:
    /// sprite, nickname, level, HP (color-coded), met location, and ability.
    pub fn draw_party(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Party");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    self.settings_open = !self.settings_open;
                }
                if ui.button("ℹ").on_hover_text("About").clicked() {
                    self.about_open = !self.about_open;
                }
            });
        });

        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new("⚙ Settings")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    self.draw_settings(ui);
                });
            self.settings_open = open;
        }

        if self.about_open {
            let mut open = self.about_open;
            egui::Window::new("About")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ui.ctx(), |ui| {
                    Self::draw_about(ui);
                });
            self.about_open = open;
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
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
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

        // ── Party members ─────────────────────────────────────────────────────
        // Animate between front and back sprites at ~1 Hz (0.5 s per frame).
        let anim_time = ui.ctx().input(|i| i.time);
        let show_back = (anim_time * 2.0) as u64 % 2 == 1;

        // Level cap: the ace level of the next unchallenged gym/E4 member.
        let level_cap: Option<u8> = fire_red_badge::read_badge_state()
            .and_then(|bs| bs.next_gym)
            .map(|g| g.max_level);

        let list = self.party_list.lock_or_recover();
        for (idx, pokemon) in list.iter().enumerate() {
            let dead = fire_red_database::is_dead(pokemon.box_mon.personality);

            ui.horizontal(|ui| {
                let species = pokemon.box_mon.secure.growth.species;
                let personality = pokemon.box_mon.personality;
                let ot_id = pokemon.box_mon.ot_id;
                let palette = if is_shiny(personality, ot_id) {
                    "shiny"
                } else {
                    "normal"
                };
                let front_key = format!("pokemon_{}_{}", species, palette);
                let back_key = format!("pokemon_{}_{}_back", species, palette);

                let tex_key = if show_back && self.textures.contains_key(&back_key) {
                    &back_key
                } else {
                    &front_key
                };

                if let Some(tex) = self.textures.get(tex_key) {
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
                        let record =
                            fire_red_database::get_dead_pokemon(pokemon.box_mon.personality);
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
                            ui.label(stat_row_job(
                                &r.nature,
                                r.max_hp,
                                r.attack,
                                r.defense,
                                r.speed,
                                r.sp_attack,
                                r.sp_defense,
                                dim,
                                11.0,
                            ));
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
                            if let Some(cap) = level_cap
                                && pokemon.level >= cap
                            {
                                ui.label(
                                    egui::RichText::new("⚠ OVER CAP")
                                        .strong()
                                        .size(13.0)
                                        .color(egui::Color32::from_rgb(255, 100, 0)),
                                );
                            }
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
                                    egui::RichText::new(format!(
                                        "{}/{}",
                                        pokemon.hp, pokemon.max_hp
                                    ))
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

                        if !pokemon.box_mon.ability_string.is_empty() {
                            ui.label(format!("Ability: {}", pokemon.box_mon.ability_string));
                        }

                        let item_str = &pokemon.box_mon.secure.growth.held_item_string;
                        if !item_str.is_empty() && item_str != "None" {
                            ui.label(format!("Held: {}", item_str));
                        }

                        let growth = &pokemon.box_mon.secure.growth.growth_rate_string;
                        if !growth.is_empty() {
                            ui.label(format!("Growth: {}", growth));
                        }
                    }
                });
            });
            ui.separator();
        }

        // ── Type coverage ──────────────────────────────────────────────────────
        // Collect types for living (non-dead) party members from the ROM.
        let rom = fire_red_rom_buffer::get_rom();
        let member_types: Vec<(u8, u8)> = list
            .iter()
            .filter(|p| !fire_red_database::is_dead(p.box_mon.personality) && p.hp > 0)
            .map(|p| {
                fire_red_party_monitor::get_species_types(rom, p.box_mon.secure.growth.species)
            })
            .collect();

        if !member_types.is_empty() {
            let cov = crate::type_coverage::compute(&member_types);

            ui.separator();
            ui.label(egui::RichText::new("Type Coverage").strong().size(13.0));

            // Team types
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Types: ")
                        .size(11.0)
                        .color(egui::Color32::from_rgb(180, 180, 180)),
                );
                for &t in &cov.team_types {
                    ui.label(
                        egui::RichText::new(fire_red_party_monitor::type_name(t))
                            .size(11.0)
                            .color(egui::Color32::from_rgb(100, 200, 255)),
                    );
                }
            });

            // Weaknesses
            if !cov.team_weaknesses.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Weak to: ")
                            .size(11.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)),
                    );
                    for &t in &cov.team_weaknesses {
                        ui.label(
                            egui::RichText::new(fire_red_party_monitor::type_name(t))
                                .size(11.0)
                                .color(egui::Color32::from_rgb(255, 120, 80)),
                        );
                    }
                });
            }

            // Offensive coverage gaps (types we can NOT hit super-effectively)
            let coverage_gaps: Vec<u8> = (0..crate::type_coverage::NUM_TYPES as u8)
                .filter(|t| !cov.offensive_coverage.contains(t))
                .collect();
            if !coverage_gaps.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("No SE vs: ")
                            .size(11.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)),
                    );
                    for &t in &coverage_gaps {
                        ui.label(
                            egui::RichText::new(fire_red_party_monitor::type_name(t))
                                .size(11.0)
                                .color(egui::Color32::from_rgb(160, 160, 160)),
                        );
                    }
                });
            }
        }
    }
}
