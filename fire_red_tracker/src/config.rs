use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Types
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
    /// Webhook URLs fired on game events.
    #[serde(default, skip_serializing_if = "WebhookConfig::is_empty")]
    pub webhooks: WebhookConfig,
}

fn default_aggregator_host() -> String { "127.0.0.1".to_string() }
fn default_aggregator_port() -> u16 { 7878 }

// ---------------------------------------------------------------------------
// Webhook config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// POSTed when a party member dies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_url: Option<String>,
    /// POSTed when a new pokemon is added to the party (caught/gifted/traded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_url: Option<String>,
    /// POSTed when a shiny wild pokemon is first encountered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shiny_url: Option<String>,
    /// POSTed when the entire party is wiped and the run ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wipe_url: Option<String>,
}

impl WebhookConfig {
    pub fn is_empty(&self) -> bool {
        self.death_url.is_none()
            && self.catch_url.is_none()
            && self.shiny_url.is_none()
            && self.wipe_url.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackerTestOverrides {
    pub db:               Option<String>,
    pub aggregator_host:  Option<String>,
    pub aggregator_port:  Option<u16>,
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
// Load / save
// ---------------------------------------------------------------------------

pub fn load_or_prompt(path: &PathBuf) -> TrackerConfig {
    if path.exists() {
        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Failed to read config file {}: {}", path.display(), e);
            std::process::exit(1);
        });
        toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!("Failed to parse config file {}: {}", path.display(), e);
            std::process::exit(1);
        })
    } else {
        let config = show_setup_dialog();
        save_config(&config, path);
        config
    }
}

pub fn save_config(config: &TrackerConfig, path: &PathBuf) {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent) {
        eprintln!("Warning: could not create config directory: {}", e);
    }
    match toml::to_string_pretty(config) {
        Ok(content) => {
            if let Err(e) = std::fs::write(path, &content) {
                eprintln!("Warning: could not write config file: {}", e);
            } else {
                println!("Config saved to {}", path.display());
            }
        }
        Err(e) => eprintln!("Warning: could not serialize config: {}", e),
    }
}

// ---------------------------------------------------------------------------
// First-run egui setup window
// ---------------------------------------------------------------------------

struct SetupApp {
    rom:              String,
    db:               String,
    clean:            bool,
    mode:             ConfigMode,
    aggregator_host:  String,
    aggregator_port:  String,
    preferred_player: String,
    result:           Arc<Mutex<Option<TrackerConfig>>>,
    should_close:     bool,
    heading:          &'static str,
    default_test:     bool,
    test:             Option<TrackerTestOverrides>,
    // Webhook URL fields
    death_url:         String,
    death_url_enabled: bool,
    catch_url:         String,
    catch_url_enabled: bool,
    shiny_url:         String,
    shiny_url_enabled: bool,
    wipe_url:          String,
    wipe_url_enabled:  bool,
}

impl SetupApp {
    fn new(result: Arc<Mutex<Option<TrackerConfig>>>) -> Self {
        Self {
            rom:              String::new(),
            db:               "localhost/nuzlocke".to_string(),
            clean:            false,
            mode:             ConfigMode::Standalone,
            aggregator_host:  "127.0.0.1".to_string(),
            aggregator_port:  "7878".to_string(),
            preferred_player: String::new(),
            result,
            should_close:     false,
            heading:          "First-Run Setup",
            default_test:     false,
            test:             None,
            death_url:         String::new(),
            death_url_enabled: false,
            catch_url:         String::new(),
            catch_url_enabled: false,
            shiny_url:         String::new(),
            shiny_url_enabled: false,
            wipe_url:          String::new(),
            wipe_url_enabled:  false,
        }
    }

    fn from_existing(result: Arc<Mutex<Option<TrackerConfig>>>, cfg: &TrackerConfig) -> Self {
        let db_display = cfg.db
            .trim_start_matches("postgresql://")
            .trim_start_matches("postgres://")
            .to_string();
        let wh = &cfg.webhooks;
        Self {
            rom:              cfg.rom.clone(),
            db:               db_display,
            clean:            cfg.clean,
            mode:             cfg.mode.clone(),
            aggregator_host:  cfg.aggregator_host.clone(),
            aggregator_port:  cfg.aggregator_port.to_string(),
            preferred_player: cfg.preferred_player.map(|n| n.to_string()).unwrap_or_default(),
            result,
            should_close:     false,
            heading:          "Edit Config",
            default_test:     cfg.default_test,
            test:             cfg.test.clone(),
            death_url:         wh.death_url.clone().unwrap_or_default(),
            death_url_enabled: wh.death_url.is_some(),
            catch_url:         wh.catch_url.clone().unwrap_or_default(),
            catch_url_enabled: wh.catch_url.is_some(),
            shiny_url:         wh.shiny_url.clone().unwrap_or_default(),
            shiny_url_enabled: wh.shiny_url.is_some(),
            wipe_url:          wh.wipe_url.clone().unwrap_or_default(),
            wipe_url_enabled:  wh.wipe_url.is_some(),
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

        egui::Grid::new("setup_grid")
            .num_columns(2)
            .spacing([12.0, 10.0])
            .min_col_width(110.0)
            .show(ui, |ui| {
                // ROM path
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

                // Database
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

                // Mode
                ui.label("Default mode:");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.mode, ConfigMode::Standalone, "Standalone");
                    ui.selectable_value(&mut self.mode, ConfigMode::Connected,  "Connected");
                });
                ui.end_row();

                // Aggregator address (only when Connected)
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

                // Webhooks
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

                ui.checkbox(&mut self.catch_url_enabled, "Catch URL:");
                ui.add_enabled_ui(self.catch_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.catch_url).desired_width(340.0).hint_text("https://…"));
                });
                ui.end_row();

                ui.checkbox(&mut self.shiny_url_enabled, "Shiny URL:");
                ui.add_enabled_ui(self.shiny_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.shiny_url).desired_width(340.0).hint_text("https://…"));
                });
                ui.end_row();

                ui.checkbox(&mut self.wipe_url_enabled, "Wipe URL:");
                ui.add_enabled_ui(self.wipe_url_enabled, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.wipe_url).desired_width(340.0).hint_text("https://…"));
                });
                ui.end_row();
            });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        let rom_ok = !self.rom.trim().is_empty();
        let port_parse: Result<u16, _> = self.aggregator_port.parse();
        let port_ok = self.mode != ConfigMode::Connected || port_parse.is_ok();
        let player_parse: Option<u8> = self.preferred_player.trim()
            .parse().ok()
            .filter(|&n: &u8| n >= 1);
        let player_ok = self.preferred_player.trim().is_empty() || player_parse.is_some();
        let can_save = rom_ok && port_ok && player_ok;

        ui.horizontal(|ui| {
            let btn = ui.add_enabled(can_save, egui::Button::new("Save & Continue"));
            if btn.clicked() {
                let db_raw = self.db.trim().to_string();
                let db = if db_raw.starts_with("postgresql://") || db_raw.starts_with("postgres://") {
                    db_raw
                } else {
                    format!("postgresql://{}", db_raw)
                };

                let config = TrackerConfig {
                    rom:              self.rom.trim().to_string(),
                    db,
                    clean:            self.clean,
                    mode:             self.mode.clone(),
                    aggregator_host:  self.aggregator_host.trim().to_string(),
                    aggregator_port:  port_parse.unwrap_or(7878),
                    preferred_player: player_parse,
                    default_test:     self.default_test,
                    test:             self.test.clone(),
                    webhooks: WebhookConfig {
                        death_url: if self.death_url_enabled && !self.death_url.trim().is_empty() { Some(self.death_url.trim().to_string()) } else { None },
                        catch_url: if self.catch_url_enabled && !self.catch_url.trim().is_empty() { Some(self.catch_url.trim().to_string()) } else { None },
                        shiny_url: if self.shiny_url_enabled && !self.shiny_url.trim().is_empty() { Some(self.shiny_url.trim().to_string()) } else { None },
                        wipe_url:  if self.wipe_url_enabled  && !self.wipe_url.trim().is_empty()  { Some(self.wipe_url.trim().to_string())  } else { None },
                    },
                };

                *self.result.lock().unwrap() = Some(config);
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
                    egui::RichText::new("  Invalid port (1–65535)")
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
        None      => SetupApp::new(result_for_app),
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
                .with_inner_size([560.0, 480.0])
                .with_resizable(true),
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(app))),
    );

    result.lock().unwrap().take().unwrap_or_else(|| {
        println!("Setup cancelled.");
        std::process::exit(0);
    })
}

/// Open the config editor window, pre-filled with the existing config if the
/// file exists, then save the result. Called by `--config-editor`.
pub fn run_config_editor(path: &PathBuf) {
    let existing: Option<TrackerConfig> = if path.exists() {
        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Failed to read config file {}: {}", path.display(), e);
            std::process::exit(1);
        });
        Some(toml::from_str(&content).unwrap_or_else(|e| {
            eprintln!("Failed to parse config file {}: {}", path.display(), e);
            std::process::exit(1);
        }))
    } else {
        None
    };

    let new_cfg = match existing {
        Some(ref cfg) => show_config_editor_from(cfg),
        None          => show_setup_dialog(),
    };
    save_config(&new_cfg, path);
}
