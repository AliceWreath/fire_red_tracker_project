// Shared config types live in fire_red_game_loop; re-export them here so the
// rest of the tracker crate can still use `crate::config::WebhookConfig` etc.
pub use fire_red_game_loop::config::{
    DupesClauseMode, NuzlockePreset, ObsConfig, TwitchHelixConfig, WebhookConfig,
};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Tracker-specific types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConfigMode {
    #[default]
    Standalone,
    Connected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerConfig {
    pub rom: String,
    pub db: String,
    #[serde(default)]
    pub clean: bool,
    #[serde(default)]
    pub mode: ConfigMode,
    #[serde(default = "default_aggregator_host")]
    pub aggregator_host: String,
    #[serde(default = "default_aggregator_port")]
    pub aggregator_port: u16,
    /// Preferred display slot in the aggregator (1 = first column, 2 = second, …).
    /// Leave unset to let the aggregator assign order by connection time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_player: Option<u8>,
    /// When true, behaves as if `--test` is always passed. Can still be overridden per-run.
    #[serde(default)]
    pub default_test: bool,
    /// Settings applied when `--test` is passed (overrides base config; explicit CLI flags still win).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TrackerTestOverrides>,
    /// How often the game-polling loop ticks, in milliseconds.
    /// Lower values give more responsive HP/death detection at the cost of CPU.
    /// Defaults to 100 ms. Valid range: 20–2000.
    #[serde(
        default = "default_poll_ms",
        skip_serializing_if = "is_default_poll_ms"
    )]
    pub poll_ms: u64,
    /// Webhook URLs fired on game events.
    #[serde(default, skip_serializing_if = "WebhookConfig::is_empty")]
    pub webhooks: WebhookConfig,
    /// OBS WebSocket integration — save replay buffer clips on key events.
    #[serde(default, skip_serializing_if = "ObsConfig::is_default")]
    pub obs: ObsConfig,
    /// Controls how the dupes clause is applied when a new wild encounter is detected.
    /// Defaults to `Off` (standard Nuzlocke — first encounter per area, no species check).
    ///
    /// - `"off"` — no dupes check.
    /// - `"per_player"` — skip if *this* player has previously caught the species this run.
    /// - `"shared"` — skip if *any* player in the run has caught the species (Soul Link / co-op).
    ///
    /// Old boolean values are still accepted: `true` maps to `"shared"`, `false` to `"off"`.
    #[serde(default)]
    pub dupes_clause: DupesClauseMode,
    /// When true, skip the "already encountered this species anywhere in the run"
    /// check. Each area still allows only one encounter entry, and the dupes
    /// clause still applies independently. Useful when the same species can
    /// legitimately appear on multiple routes (e.g. randomized ROMs or certain
    /// Nuzlocke variants that don't restrict by species).
    #[serde(default)]
    pub allow_species_repeats: bool,
    /// Convenience preset that sets `dupes_clause` and `allow_species_repeats`
    /// together. Applied at load time; the individual fields can still be
    /// overridden afterward in code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<NuzlockePreset>,
    /// Minimum Pokéball count required for the run-start latch to trigger.
    /// Defaults to 5. Increase if your starter gift delays the first catch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_start_balls: Option<u8>,
    /// LiveSplit Server host. Leave unset to disable LiveSplit integration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub livesplit_host: Option<String>,
    /// LiveSplit Server TCP port. Defaults to 16834.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub livesplit_port: Option<u16>,
    /// Send a split to LiveSplit each time a new gym badge is earned.
    #[serde(default)]
    pub livesplit_split_on_badges: bool,
    /// Send a split to LiveSplit when the game is cleared (Champion defeated).
    #[serde(default = "default_true")]
    pub livesplit_split_on_clear: bool,
    /// Discord application client ID for Rich Presence. Leave unset to disable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_client_id: Option<u64>,
    /// Twitch Helix API integration — stream markers, polls, and predictions.
    /// Enable by adding a `[twitch_helix]` section to `config.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twitch_helix: Option<TwitchHelixConfig>,
}

fn default_aggregator_host() -> String {
    "127.0.0.1".to_string()
}
fn default_aggregator_port() -> u16 {
    7878
}
fn default_poll_ms() -> u64 {
    100
}
fn is_default_poll_ms(v: &u64) -> bool {
    *v == 100
}
fn default_true() -> bool {
    true
}


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackerTestOverrides {
    pub db: Option<String>,
    pub aggregator_host: Option<String>,
    pub aggregator_port: Option<u16>,
    pub preferred_player: Option<u8>,
}

// ---------------------------------------------------------------------------
// Config path
// ---------------------------------------------------------------------------

pub fn default_config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("fire_red_tracker")
            .join("config.toml")
    } else {
        PathBuf::from("tracker.toml")
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// A single validation error returned by [`validate_config`].
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub field: &'static str,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// Validates a [`TrackerConfig`] for obvious early errors.
///
/// Returns a list of errors; an empty list means the config is valid.
/// Checks performed:
/// - ROM path is non-empty and the file exists and is readable.
/// - Each configured webhook URL starts with `http://` or `https://`.
pub fn validate_config(cfg: &TrackerConfig) -> Vec<ConfigError> {
    let mut errors = Vec::new();

    // ROM path
    if cfg.rom.trim().is_empty() {
        errors.push(ConfigError {
            field: "rom",
            message: "path is empty".to_string(),
        });
    } else {
        let path = std::path::Path::new(&cfg.rom);
        if !path.exists() {
            errors.push(ConfigError {
                field: "rom",
                message: format!("file not found: {}", cfg.rom),
            });
        } else if std::fs::File::open(path).is_err() {
            errors.push(ConfigError {
                field: "rom",
                message: format!("file exists but cannot be opened: {}", cfg.rom),
            });
        }
    }

    // Webhook URLs
    let wh_fields: &[(&'static str, &Option<String>)] = &[
        ("webhooks.death_url", &cfg.webhooks.death_url),
        ("webhooks.catch_url", &cfg.webhooks.catch_url),
        ("webhooks.shiny_url", &cfg.webhooks.shiny_url),
        ("webhooks.wipe_url", &cfg.webhooks.wipe_url),
    ];
    for (field, url_opt) in wh_fields {
        if let Some(url) = url_opt
            && !url.starts_with("http://")
            && !url.starts_with("https://")
        {
            errors.push(ConfigError {
                field,
                message: format!("URL must start with http:// or https:// (got: {url})"),
            });
        }
    }

    errors
}

// ---------------------------------------------------------------------------
// Load / save
// ---------------------------------------------------------------------------

/// Load the config file silently (no GUI, no prompts). Returns `None` on any error.
///
/// Used by the hot-reload thread — on failure the existing in-memory config stays active.
pub fn try_load_config(path: &std::path::Path) -> Option<TrackerConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut config: TrackerConfig = toml::from_str(&content).ok()?;
    if let Some(preset) = config.preset {
        let (dc, asr) = preset.settings();
        config.dupes_clause = dc;
        config.allow_species_repeats = asr;
    }
    Some(config)
}

pub fn load_or_prompt(path: &PathBuf) -> TrackerConfig {
    if path.exists() {
        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
            tracing::error!("Failed to read config file {}: {}", path.display(), e);
            std::process::exit(1);
        });
        let mut config: TrackerConfig = toml::from_str(&content).unwrap_or_else(|e| {
            tracing::error!("Failed to parse config file {}: {}", path.display(), e);
            std::process::exit(1);
        });
        if let Some(preset) = config.preset {
            let (dc, asr) = preset.settings();
            config.dupes_clause = dc;
            config.allow_species_repeats = asr;
        }
        config
    } else {
        let config = show_setup_dialog();
        save_config(&config, path);
        config
    }
}

pub fn save_config(config: &TrackerConfig, path: &PathBuf) {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!("could not create config directory: {}", e);
    }
    match toml::to_string_pretty(config) {
        Ok(content) => {
            if let Err(e) = std::fs::write(path, &content) {
                tracing::warn!("could not write config file: {}", e);
            } else {
                tracing::info!("Config saved to {}", path.display());
            }
        }
        Err(e) => tracing::warn!("could not serialize config: {}", e),
    }
}

// ---------------------------------------------------------------------------
// First-run egui setup window
// ---------------------------------------------------------------------------

struct SetupApp {
    rom: String,
    db: String,
    clean: bool,
    mode: ConfigMode,
    aggregator_host: String,
    aggregator_port: String,
    preferred_player: String,
    result: Arc<Mutex<Option<TrackerConfig>>>,
    should_close: bool,
    heading: &'static str,
    // Run / polling
    poll_ms: String,
    dupes_clause: DupesClauseMode,
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
    // Fields with no UI widget — preserved unchanged from the loaded config.
    badge_url: Option<String>,
    badge_template: Option<String>,
    nickname_url: Option<String>,
    nickname_template: Option<String>,
    nuzlocke_url: Option<String>,
    nuzlocke_template: Option<String>,
    notify_on_death: bool,
    notify_on_shiny: bool,
    notify_on_wipe: bool,
    obs_clip_badge: bool,
}

impl SetupApp {
    fn new(result: Arc<Mutex<Option<TrackerConfig>>>) -> Self {
        Self {
            rom: String::new(),
            db: "localhost/nuzlocke".to_string(),
            clean: false,
            mode: ConfigMode::Standalone,
            aggregator_host: "127.0.0.1".to_string(),
            aggregator_port: "7878".to_string(),
            preferred_player: String::new(),
            result,
            should_close: false,
            heading: "First-Run Setup",
            poll_ms: String::new(),
            dupes_clause: DupesClauseMode::Off,
            allow_species_repeats: false,
            run_start_balls: String::new(),
            default_test: false,
            test_db: String::new(),
            test_agg_host: String::new(),
            test_agg_port: String::new(),
            test_player: String::new(),
            obs_host: "localhost".to_string(),
            obs_port: "4455".to_string(),
            obs_password: String::new(),
            obs_clip_death: false,
            obs_clip_shiny: false,
            obs_clip_wipe: false,
            death_url: String::new(),
            death_url_enabled: false,
            death_template: String::new(),
            catch_url: String::new(),
            catch_url_enabled: false,
            catch_template: String::new(),
            shiny_url: String::new(),
            shiny_url_enabled: false,
            shiny_template: String::new(),
            wipe_url: String::new(),
            wipe_url_enabled: false,
            wipe_template: String::new(),
            badge_url: None,
            badge_template: None,
            nickname_url: None,
            nickname_template: None,
            nuzlocke_url: None,
            nuzlocke_template: None,
            notify_on_death: false,
            notify_on_shiny: false,
            notify_on_wipe: false,
            obs_clip_badge: false,
        }
    }

    fn from_existing(result: Arc<Mutex<Option<TrackerConfig>>>, cfg: &TrackerConfig) -> Self {
        let db_display = cfg
            .db
            .trim_start_matches("postgresql://")
            .trim_start_matches("postgres://")
            .to_string();
        let wh = &cfg.webhooks;
        Self {
            rom: cfg.rom.clone(),
            db: db_display,
            clean: cfg.clean,
            mode: cfg.mode.clone(),
            aggregator_host: cfg.aggregator_host.clone(),
            aggregator_port: cfg.aggregator_port.to_string(),
            preferred_player: cfg
                .preferred_player
                .map(|n| n.to_string())
                .unwrap_or_default(),
            result,
            should_close: false,
            heading: "Edit Config",
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
            badge_url: wh.badge_url.clone(),
            badge_template: wh.badge_template.clone(),
            nickname_url: wh.nickname_url.clone(),
            nickname_template: wh.nickname_template.clone(),
            nuzlocke_url: wh.nuzlocke_url.clone(),
            nuzlocke_template: wh.nuzlocke_template.clone(),
            notify_on_death: wh.notify_on_death,
            notify_on_shiny: wh.notify_on_shiny,
            notify_on_wipe: wh.notify_on_wipe,
            obs_clip_badge: cfg.obs.clip_on_badge,
        }
    }
}

impl eframe::App for SetupApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.should_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.add_space(8.0);
        ui.heading(format!("FireRed Tracker — {}", self.heading));
        ui.label("These settings will be saved to your config file for future runs.");
        ui.separator();
        ui.add_space(4.0);

        // Reserve ~70 px for the separator + button row below the scroll area.
        let scroll_height = (ui.available_height() - 70.0).max(100.0);
        egui::ScrollArea::vertical().max_height(scroll_height).show(ui, |ui| {
        egui::Grid::new("setup_grid")
            .num_columns(2)
            .spacing([12.0, 10.0])
            .min_col_width(110.0)
            .show(ui, |ui| {
                // ── ROM / database ────────────────────────────────────────────
                ui.label("ROM path:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.rom)
                            .desired_width(280.0)
                            .hint_text("path/to/firered.gba"),
                    );
                    if ui.button("Browse…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("GBA ROM", &["gba"])
                            .pick_file()
                    {
                        self.rom = path.display().to_string();
                    }
                });
                ui.end_row();

                ui.label("Database:");
                ui.vertical(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.db)
                            .desired_width(340.0)
                            .hint_text("localhost/nuzlocke"),
                    );
                    ui.small("postgresql:// is added automatically if omitted");
                });
                ui.end_row();

                ui.label("Clean start:");
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.clean, "Wipe database on next launch");
                    ui.small("Deletes all run data at startup. Uncheck after use.");
                });
                ui.end_row();

                // ── Connection mode ───────────────────────────────────────────
                ui.separator();
                ui.end_row();
                ui.label("Default mode:");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.mode, ConfigMode::Standalone, "Standalone");
                    ui.selectable_value(&mut self.mode, ConfigMode::Connected,  "Connected");
                });
                ui.end_row();

                if self.mode == ConfigMode::Connected {
                    ui.label("Aggregator host:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.aggregator_host)
                            .desired_width(200.0),
                    );
                    ui.end_row();

                    ui.label("Aggregator port:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.aggregator_port)
                            .desired_width(80.0),
                    );
                    ui.end_row();

                    ui.label("Player number:");
                    ui.vertical(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.preferred_player)
                                .desired_width(60.0)
                                .hint_text("1, 2, …"),
                        );
                        ui.small("Display column in the aggregator (leave blank for auto)");
                    });
                    ui.end_row();
                }

                // ── Run settings ──────────────────────────────────────────────
                ui.separator();
                ui.end_row();

                ui.label("Poll interval:");
                ui.vertical(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.poll_ms)
                            .desired_width(80.0)
                            .hint_text("100"),
                    );
                    ui.small("Game-polling interval in ms (20–2000). Blank = 100 ms default.");
                });
                ui.end_row();

                ui.label("Dupes clause:");
                ui.vertical(|ui| {
                    ui.selectable_value(&mut self.dupes_clause, DupesClauseMode::Off,       "Off — standard Nuzlocke (first encounter per area)");
                    ui.selectable_value(&mut self.dupes_clause, DupesClauseMode::PerPlayer, "Per Player — skip if you already caught this species");
                    ui.selectable_value(&mut self.dupes_clause, DupesClauseMode::Shared,    "Shared — skip if any player caught this species (Soul Link / co-op)");
                });
                ui.end_row();

                ui.label("Randomizer mode:");
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.allow_species_repeats, "Allow same species on multiple routes");
                    ui.small("Skips the global species-seen check. Each route still allows one encounter, and the dupes clause still applies.");
                });
                ui.end_row();

                ui.label("Run-start balls:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.run_start_balls)
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
                    ui.checkbox(&mut self.default_test, "Always run in test mode (same as always passing --test)");
                    ui.small("When enabled, the [test] overrides below are applied on every launch.");
                });
                ui.end_row();

                ui.label("  Test DB:");
                ui.vertical(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.test_db)
                            .desired_width(300.0)
                            .hint_text("leave blank to use main DB"),
                    );
                    ui.small("Overrides the database connection when running in test mode.");
                });
                ui.end_row();

                ui.label("  Test agg. host:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.test_agg_host)
                        .desired_width(200.0)
                        .hint_text("leave blank to use main host"),
                );
                ui.end_row();

                ui.label("  Test agg. port:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.test_agg_port)
                        .desired_width(80.0)
                        .hint_text("leave blank to use main port"),
                );
                ui.end_row();

                ui.label("  Test player #:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.test_player)
                        .desired_width(60.0)
                        .hint_text("leave blank"),
                );
                ui.end_row();

                // ── OBS clip trigger ──────────────────────────────────────────
                ui.separator();
                ui.end_row();
                ui.label(egui::RichText::new("OBS clips").strong());
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.obs_clip_death, "Save replay buffer on death");
                    ui.checkbox(&mut self.obs_clip_shiny, "Save replay buffer on shiny encounter");
                    ui.checkbox(&mut self.obs_clip_wipe,  "Save replay buffer on party wipe");
                });
                ui.end_row();

                let obs_used = self.obs_clip_death || self.obs_clip_shiny || self.obs_clip_wipe;
                ui.label("  OBS host:");
                ui.add_enabled_ui(obs_used, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.obs_host).desired_width(200.0).hint_text("localhost"));
                });
                ui.end_row();

                ui.label("  OBS port:");
                ui.add_enabled_ui(obs_used, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.obs_port).desired_width(80.0).hint_text("4455"));
                });
                ui.end_row();

                ui.label("  OBS password:");
                ui.add_enabled_ui(obs_used, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.obs_password).desired_width(200.0).hint_text("leave blank if auth disabled").password(true));
                });
                ui.end_row();

                // ── Webhooks ──────────────────────────────────────────────────
                ui.separator();
                ui.end_row();
                ui.label(egui::RichText::new("Webhooks").strong());
                ui.small("POST JSON to a URL on game events (Discord, stream alerts, etc.)");
                ui.end_row();

                ui.checkbox(&mut self.death_url_enabled, "Death URL:");
                ui.add_enabled_ui(self.death_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.death_url).desired_width(340.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(self.death_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.death_template).desired_width(340.0)
                        .hint_text(r#"{"content": "{player} lost {pokemon.nickname}!"} — blank = default JSON"#));
                });
                ui.end_row();

                ui.checkbox(&mut self.catch_url_enabled, "Catch URL:");
                ui.add_enabled_ui(self.catch_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.catch_url).desired_width(340.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(self.catch_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.catch_template).desired_width(340.0)
                        .hint_text(r#"{"content": "{player} caught {pokemon.species} (Lv.{pokemon.level})!"}"#));
                });
                ui.end_row();

                ui.checkbox(&mut self.shiny_url_enabled, "Shiny URL:");
                ui.add_enabled_ui(self.shiny_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.shiny_url).desired_width(340.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(self.shiny_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.shiny_template).desired_width(340.0)
                        .hint_text(r#"{"content": "✨ {player} encountered a shiny {pokemon.species}!"}"#));
                });
                ui.end_row();

                ui.checkbox(&mut self.wipe_url_enabled, "Wipe URL:");
                ui.add_enabled_ui(self.wipe_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.wipe_url).desired_width(340.0).hint_text("https://…"));
                });
                ui.end_row();
                ui.label("  Template:");
                ui.add_enabled_ui(self.wipe_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.wipe_template).desired_width(340.0)
                        .hint_text(r#"{"content": "{player}'s run has ended. RIP."}"#));
                });
                ui.end_row();
            });
        }); // ScrollArea

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        let rom_ok = !self.rom.trim().is_empty();
        let port_val: Option<u16> = self.aggregator_port.trim().parse().ok().filter(|&p| p > 0);
        let port_ok = self.mode != ConfigMode::Connected || port_val.is_some();
        let player_parse: Option<u8> = self
            .preferred_player
            .trim()
            .parse()
            .ok()
            .filter(|&n: &u8| n >= 1);
        let player_ok = self.preferred_player.trim().is_empty() || player_parse.is_some();
        let can_save = rom_ok && port_ok && player_ok;

        ui.horizontal(|ui| {
            let btn = ui.add_enabled(can_save, egui::Button::new("Save & Continue"));
            if btn.clicked() {
                let db_raw = self.db.trim().to_string();
                let db = if db_raw.starts_with("postgresql://") || db_raw.starts_with("postgres://")
                {
                    db_raw
                } else {
                    format!("postgresql://{}", db_raw)
                };

                let test_db_raw = self.test_db.trim().to_string();
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
                        aggregator_host: if self.test_agg_host.trim().is_empty() {
                            None
                        } else {
                            Some(self.test_agg_host.trim().to_string())
                        },
                        aggregator_port: self
                            .test_agg_port
                            .trim()
                            .parse()
                            .ok()
                            .filter(|&p: &u16| p > 0),
                        preferred_player: self
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

                let config = TrackerConfig {
                    rom: self.rom.trim().to_string(),
                    db,
                    clean: self.clean,
                    mode: self.mode.clone(),
                    aggregator_host: self.aggregator_host.trim().to_string(),
                    aggregator_port: port_val.unwrap_or(7878),
                    preferred_player: player_parse,
                    default_test: self.default_test,
                    test,
                    poll_ms: if self.poll_ms.trim().is_empty() {
                        default_poll_ms()
                    } else {
                        self.poll_ms
                            .trim()
                            .parse::<u64>()
                            .unwrap_or(100)
                            .clamp(20, 2000)
                    },
                    webhooks: WebhookConfig {
                        death_url: if self.death_url_enabled && !self.death_url.trim().is_empty() {
                            Some(self.death_url.trim().to_string())
                        } else {
                            None
                        },
                        death_template: if self.death_url_enabled
                            && !self.death_template.trim().is_empty()
                        {
                            Some(self.death_template.trim().to_string())
                        } else {
                            None
                        },
                        catch_url: if self.catch_url_enabled && !self.catch_url.trim().is_empty() {
                            Some(self.catch_url.trim().to_string())
                        } else {
                            None
                        },
                        catch_template: if self.catch_url_enabled
                            && !self.catch_template.trim().is_empty()
                        {
                            Some(self.catch_template.trim().to_string())
                        } else {
                            None
                        },
                        shiny_url: if self.shiny_url_enabled && !self.shiny_url.trim().is_empty() {
                            Some(self.shiny_url.trim().to_string())
                        } else {
                            None
                        },
                        shiny_template: if self.shiny_url_enabled
                            && !self.shiny_template.trim().is_empty()
                        {
                            Some(self.shiny_template.trim().to_string())
                        } else {
                            None
                        },
                        wipe_url: if self.wipe_url_enabled && !self.wipe_url.trim().is_empty() {
                            Some(self.wipe_url.trim().to_string())
                        } else {
                            None
                        },
                        wipe_template: if self.wipe_url_enabled
                            && !self.wipe_template.trim().is_empty()
                        {
                            Some(self.wipe_template.trim().to_string())
                        } else {
                            None
                        },
                        badge_url: self.badge_url.clone(),
                        badge_template: self.badge_template.clone(),
                        nickname_url: self.nickname_url.clone(),
                        nickname_template: self.nickname_template.clone(),
                        nuzlocke_url: self.nuzlocke_url.clone(),
                        nuzlocke_template: self.nuzlocke_template.clone(),
                        notify_on_death: self.notify_on_death,
                        notify_on_shiny: self.notify_on_shiny,
                        notify_on_wipe: self.notify_on_wipe,
                    },
                    obs: ObsConfig {
                        host: self.obs_host.trim().to_string(),
                        port: self.obs_port.trim().parse().unwrap_or(4455),
                        password: if self.obs_password.trim().is_empty() {
                            None
                        } else {
                            Some(self.obs_password.trim().to_string())
                        },
                        clip_on_death: self.obs_clip_death,
                        clip_on_shiny: self.obs_clip_shiny,
                        clip_on_wipe: self.obs_clip_wipe,
                        clip_on_badge: self.obs_clip_badge,
                        scene_on_death: None,
                        scene_on_wipe: None,
                        scene_on_shiny: None,
                        scene_on_badge: None,
                        scene_on_catch: None,
                    },
                    dupes_clause: self.dupes_clause,
                    allow_species_repeats: self.allow_species_repeats,
                    preset: None,
                    run_start_balls: self.run_start_balls.trim().parse::<u8>().ok(),
                    livesplit_host: None,
                    livesplit_port: None,
                    livesplit_split_on_badges: false,
                    livesplit_split_on_clear: true,
                    discord_client_id: None,
                    twitch_helix: None,
                };

                *self.result.lock().unwrap_or_else(|p| p.into_inner()) = Some(config);
                self.should_close = true;
            }

            if !rom_ok {
                ui.label(
                    egui::RichText::new("  ROM path is required")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            } else if !port_ok {
                ui.label(
                    egui::RichText::new("  Invalid aggregator port (1–65535)")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            } else if !player_ok {
                ui.label(
                    egui::RichText::new("  Player number must be 1 or higher")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            }
        });
    }
}

fn show_setup_dialog() -> TrackerConfig {
    run_setup_window(None)
}

fn show_config_editor_from(existing: &TrackerConfig) -> TrackerConfig {
    run_setup_window(Some(existing))
}

fn run_setup_window(existing: Option<&TrackerConfig>) -> TrackerConfig {
    let result: Arc<Mutex<Option<TrackerConfig>>> = Arc::new(Mutex::new(None));
    let result_for_app = result.clone();

    let app: SetupApp = match existing {
        Some(cfg) => SetupApp::from_existing(result_for_app, cfg),
        None => SetupApp::new(result_for_app),
    };

    let title = if existing.is_some() {
        "FireRed Tracker — Edit Config"
    } else {
        "FireRed Tracker — Setup"
    };

    let _ = eframe::run_native(
        title,
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([580.0, 600.0])
                .with_resizable(true),
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(app))),
    );

    result.lock().unwrap_or_else(|p| p.into_inner()).take().unwrap_or_else(|| {
        println!("Setup cancelled.");
        std::process::exit(0);
    })
}

/// Open the config editor window, pre-filled with the existing config if the
/// file exists, then save the result. Called by `--config-editor`.
pub fn run_config_editor(path: &PathBuf) {
    let existing: Option<TrackerConfig> = if path.exists() {
        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
            tracing::error!("Failed to read config file {}: {}", path.display(), e);
            std::process::exit(1);
        });
        Some(toml::from_str(&content).unwrap_or_else(|e| {
            tracing::error!("Failed to parse config file {}: {}", path.display(), e);
            std::process::exit(1);
        }))
    } else {
        None
    };

    let new_cfg = match existing {
        Some(ref cfg) => show_config_editor_from(cfg),
        None => show_setup_dialog(),
    };
    save_config(&new_cfg, path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml(extra: &str) -> String {
        format!("rom = \"test.gba\"\ndb = \"postgresql://localhost/test\"\n{extra}")
    }

    // ── poll_ms ───────────────────────────────────────────────────────────────

    #[test]
    fn poll_ms_defaults_to_100_when_absent() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        assert_eq!(cfg.poll_ms, 100);
    }

    #[test]
    fn poll_ms_parsed_when_present() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("poll_ms = 250\n")).unwrap();
        assert_eq!(cfg.poll_ms, 250);
    }

    #[test]
    fn poll_ms_not_serialized_at_default() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        let serialized = toml::to_string(&cfg).unwrap();
        assert!(
            !serialized.contains("poll_ms"),
            "poll_ms should be omitted when default"
        );
    }

    #[test]
    fn poll_ms_serialized_when_nondefault() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("poll_ms = 500\n")).unwrap();
        let serialized = toml::to_string(&cfg).unwrap();
        assert!(serialized.contains("poll_ms = 500"));
    }

    #[test]
    fn poll_ms_roundtrips() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("poll_ms = 200\n")).unwrap();
        let s = toml::to_string(&cfg).unwrap();
        let cfg2: TrackerConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg2.poll_ms, 200);
    }

    // ── dupes_clause ─────────────────────────────────────────────────────────

    #[test]
    fn dupes_clause_defaults_to_off_when_absent() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::Off);
    }

    #[test]
    fn dupes_clause_bool_true_maps_to_shared() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("dupes_clause = true\n")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::Shared);
    }

    #[test]
    fn dupes_clause_bool_false_maps_to_off() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("dupes_clause = false\n")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::Off);
    }

    #[test]
    fn dupes_clause_string_off() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("dupes_clause = \"off\"\n")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::Off);
    }

    #[test]
    fn dupes_clause_string_per_player() {
        let cfg: TrackerConfig =
            toml::from_str(&minimal_toml("dupes_clause = \"per_player\"\n")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::PerPlayer);
    }

    #[test]
    fn dupes_clause_string_shared() {
        let cfg: TrackerConfig =
            toml::from_str(&minimal_toml("dupes_clause = \"shared\"\n")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::Shared);
    }

    #[test]
    fn dupes_clause_serialises_as_string() {
        let cfg: TrackerConfig =
            toml::from_str(&minimal_toml("dupes_clause = \"per_player\"\n")).unwrap();
        let s = toml::to_string(&cfg).unwrap();
        assert!(
            s.contains("per_player"),
            "expected 'per_player' in serialised output: {s}"
        );
    }

    // ── validate_config ──────────────────────────────────────────────────────

    fn cfg_with_rom(rom: &str) -> TrackerConfig {
        TrackerConfig {
            rom: rom.to_string(),
            db: "postgresql://localhost/test".to_string(),
            clean: false,
            mode: ConfigMode::default(),
            aggregator_host: "127.0.0.1".to_string(),
            aggregator_port: 7878,
            preferred_player: None,
            default_test: false,
            test: None,
            poll_ms: 100,
            webhooks: WebhookConfig::default(),
            obs: ObsConfig::default(),
            dupes_clause: DupesClauseMode::Off,
            allow_species_repeats: false,
            preset: None,
            run_start_balls: None,
            livesplit_host: None,
            livesplit_port: None,
            livesplit_split_on_badges: false,
            livesplit_split_on_clear: true,
            discord_client_id: None,
            twitch_helix: None,
        }
    }

    #[test]
    fn validate_config_empty_rom_is_error() {
        let cfg = cfg_with_rom("");
        let errs = validate_config(&cfg);
        assert!(
            errs.iter().any(|e| e.field == "rom"),
            "expected rom error, got: {errs:?}"
        );
    }

    #[test]
    fn validate_config_nonexistent_rom_is_error() {
        let cfg = cfg_with_rom("/tmp/__nonexistent_rom_test_file__.gba");
        let errs = validate_config(&cfg);
        assert!(
            errs.iter().any(|e| e.field == "rom"),
            "expected rom error, got: {errs:?}"
        );
    }

    #[test]
    fn validate_config_valid_rom_no_error() {
        // Create a temporary file so the path exists.
        let path = "/tmp/__test_rom_validate__.gba";
        std::fs::write(path, b"FAKE").unwrap();
        let cfg = cfg_with_rom(path);
        let errs = validate_config(&cfg);
        std::fs::remove_file(path).ok();
        assert!(
            !errs.iter().any(|e| e.field == "rom"),
            "unexpected rom error: {errs:?}"
        );
    }

    #[test]
    fn validate_config_valid_webhook_url_no_error() {
        let mut cfg = cfg_with_rom("/tmp/__test_rom_validate2__.gba");
        std::fs::write("/tmp/__test_rom_validate2__.gba", b"FAKE").unwrap();
        cfg.webhooks.death_url = Some("https://discord.com/api/webhooks/123/abc".to_string());
        let errs = validate_config(&cfg);
        std::fs::remove_file("/tmp/__test_rom_validate2__.gba").ok();
        assert!(
            !errs.iter().any(|e| e.field.contains("death_url")),
            "unexpected error: {errs:?}"
        );
    }

    #[test]
    fn validate_config_invalid_webhook_url_is_error() {
        let mut cfg = cfg_with_rom("");
        cfg.webhooks.death_url = Some("ftp://bad-scheme.example.com".to_string());
        let errs = validate_config(&cfg);
        assert!(
            errs.iter().any(|e| e.field == "webhooks.death_url"),
            "expected webhook url error, got: {errs:?}"
        );
    }

    #[test]
    fn validate_config_http_webhook_url_is_valid() {
        let mut cfg = cfg_with_rom("");
        cfg.webhooks.wipe_url = Some("http://localhost:9000/hook".to_string());
        let errs = validate_config(&cfg);
        assert!(
            !errs.iter().any(|e| e.field == "webhooks.wipe_url"),
            "http:// should be valid, got: {errs:?}"
        );
    }

    #[test]
    fn validate_config_multiple_errors_collected() {
        let mut cfg = cfg_with_rom("");
        cfg.webhooks.death_url = Some("bad-url".to_string());
        cfg.webhooks.catch_url = Some("also-bad".to_string());
        let errs = validate_config(&cfg);
        // rom + death_url + catch_url = at least 3 errors
        assert!(errs.len() >= 3, "expected >= 3 errors, got: {errs:?}");
    }

    // ── WebhookConfig::is_empty ───────────────────────────────────────────────

    #[test]
    fn webhook_config_is_empty_when_default() {
        assert!(WebhookConfig::default().is_empty());
    }

    #[test]
    fn webhook_config_not_empty_when_death_url_set() {
        let cfg = WebhookConfig {
            death_url: Some("https://example.com".to_string()),
            ..Default::default()
        };
        assert!(!cfg.is_empty());
    }

    #[test]
    fn webhook_config_not_empty_when_template_set() {
        let cfg = WebhookConfig {
            death_template: Some("{event}".to_string()),
            ..Default::default()
        };
        assert!(!cfg.is_empty());
    }

    // ── ObsConfig::is_default ────────────────────────────────────────────────

    #[test]
    fn obs_config_is_default_when_no_clips_enabled() {
        assert!(ObsConfig::default().is_default());
    }

    #[test]
    fn obs_config_not_default_when_clip_on_death() {
        let cfg = ObsConfig {
            clip_on_death: true,
            ..Default::default()
        };
        assert!(!cfg.is_default());
    }

    // ── livesplit / discord config fields ─────────────────────────────────────

    #[test]
    fn livesplit_host_defaults_to_none_when_absent() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        assert!(cfg.livesplit_host.is_none());
    }

    #[test]
    fn livesplit_port_defaults_to_none_when_absent() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        assert!(cfg.livesplit_port.is_none());
    }

    #[test]
    fn livesplit_split_on_badges_defaults_to_false() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        assert!(!cfg.livesplit_split_on_badges);
    }

    #[test]
    fn livesplit_split_on_clear_defaults_to_true() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        assert!(cfg.livesplit_split_on_clear);
    }

    #[test]
    fn discord_client_id_defaults_to_none_when_absent() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        assert!(cfg.discord_client_id.is_none());
    }

    #[test]
    fn livesplit_host_parsed_when_present() {
        let cfg: TrackerConfig =
            toml::from_str(&minimal_toml("livesplit_host = \"localhost\"\n")).unwrap();
        assert_eq!(cfg.livesplit_host.as_deref(), Some("localhost"));
    }

    #[test]
    fn livesplit_port_parsed_when_present() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("livesplit_port = 16834\n")).unwrap();
        assert_eq!(cfg.livesplit_port, Some(16834));
    }

    #[test]
    fn livesplit_split_on_clear_can_be_disabled() {
        let cfg: TrackerConfig =
            toml::from_str(&minimal_toml("livesplit_split_on_clear = false\n")).unwrap();
        assert!(!cfg.livesplit_split_on_clear);
    }

    #[test]
    fn discord_client_id_parsed_when_present() {
        let cfg: TrackerConfig =
            toml::from_str(&minimal_toml("discord_client_id = 123456789\n")).unwrap();
        assert_eq!(cfg.discord_client_id, Some(123456789u64));
    }

    #[test]
    fn livesplit_discord_fields_not_serialized_when_none() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("")).unwrap();
        let s = toml::to_string(&cfg).unwrap();
        assert!(
            !s.contains("livesplit_host"),
            "livesplit_host should be omitted when None"
        );
        assert!(
            !s.contains("livesplit_port"),
            "livesplit_port should be omitted when None"
        );
        assert!(
            !s.contains("discord_client_id"),
            "discord_client_id should be omitted when None"
        );
    }

    // ── try_load_config ───────────────────────────────────────────────────────

    #[test]
    fn try_load_config_returns_some_for_valid_file() {
        let path = std::path::PathBuf::from("/tmp/__test_try_load_valid__.toml");
        std::fs::write(&path, minimal_toml("")).unwrap();
        let result = try_load_config(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_some());
    }

    #[test]
    fn try_load_config_returns_none_for_invalid_toml() {
        let path = std::path::PathBuf::from("/tmp/__test_try_load_invalid__.toml");
        std::fs::write(&path, "this is [[[not valid toml").unwrap();
        let result = try_load_config(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_none());
    }

    #[test]
    fn try_load_config_returns_none_for_missing_file() {
        let result = try_load_config(std::path::Path::new(
            "/tmp/__nonexistent_tracker_config__.toml",
        ));
        assert!(result.is_none());
    }

    #[test]
    fn try_load_config_applies_preset_overrides() {
        let path = std::path::PathBuf::from("/tmp/__test_try_load_preset__.toml");
        std::fs::write(&path, minimal_toml("preset = \"hardcore\"\n")).unwrap();
        let cfg = try_load_config(&path).expect("should parse");
        std::fs::remove_file(&path).ok();
        assert_eq!(
            cfg.dupes_clause,
            DupesClauseMode::PerPlayer,
            "hardcore preset should set per_player dupes clause"
        );
    }
}
