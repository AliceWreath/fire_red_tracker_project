use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorConfig {
    /// TCP port to listen on for incoming tracker connections.
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
    /// PostgreSQL connection string (optional).
    pub db: Option<String>,
    /// WebSocket overlay port (optional — omit for GUI window mode).
    pub ws_port: Option<u16>,
    /// When true, behaves as if `--test` is always passed. Can still be overridden per-run.
    #[serde(default)]
    pub default_test: bool,
    /// Settings applied when `--test` is passed (overrides base config; explicit CLI flags still win).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<AggregatorTestOverrides>,
    /// Whether the injection API endpoints (give_item, make_shiny, etc.) are enabled.
    /// Defaults to true. Set to false (or pass --no-injections) to disable all injection commands.
    #[serde(default = "default_true")]
    pub allow_injections: bool,
    /// Optional Twitch chat bot — responds to `!party`, `!deaths`, `!shinies`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twitch: Option<TwitchConfig>,
}

/// Twitch IRC chat bot configuration.
///
/// Enable by adding a `[twitch]` section to `~/.config/fire_red_aggregator/config.toml`:
/// ```toml
/// [twitch]
/// channel = "mychannel"          # channel name without #
/// nick    = "my_bot_account"     # Twitch username for the bot account
/// token   = "oauth:xxxxxxxxxx"   # OAuth token — get one at twitchapps.com/tmi
/// # slot  = 0                    # which tracker slot to read (default: 0)
///
/// # Optional: Channel Points EventSub (reward → command mapping).
/// # Requires the OAuth token to have the channel:read:redemptions scope.
/// # client_id    — your Twitch app's Client ID (from dev.twitch.tv)
/// # broadcaster_id — numeric Twitch user ID of the channel (not the username)
/// # [twitch.reward_commands]
/// # "00000000-0000-0000-0000-000000000001" = "heal_all"
/// # "00000000-0000-0000-0000-000000000002" = "new_run"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchConfig {
    /// Twitch channel to join (without the leading `#`).
    pub channel: String,
    /// Twitch username used for the bot account.
    pub nick: String,
    /// OAuth token in the form `oauth:xxxxxxxxxx`.
    pub token: String,
    /// Tracker slot index to read live state from (default 0).
    #[serde(default)]
    pub slot: usize,
    /// Twitch app Client-ID (required for Channel Points EventSub).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Numeric Twitch user ID of the broadcaster's channel (required for EventSub).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcaster_id: Option<String>,
    /// Maps channel-point reward UUIDs to aggregator commands (`heal_all`, `new_run`, `end_run`).
    /// Omit or leave empty to disable Channel Points redemption handling.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub reward_commands: std::collections::HashMap<String, String>,
}

fn default_listen_port() -> u16 {
    7878
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregatorTestOverrides {
    pub listen_port: Option<u16>,
    pub db: Option<String>,
    pub ws_port: Option<u16>,
}

// ---------------------------------------------------------------------------
// Config path
// ---------------------------------------------------------------------------

pub fn default_config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("fire_red_aggregator")
            .join("config.toml")
    } else {
        PathBuf::from("aggregator.toml")
    }
}

// ---------------------------------------------------------------------------
// Load / save
// ---------------------------------------------------------------------------

pub fn load_or_prompt(path: &PathBuf) -> AggregatorConfig {
    if path.exists() {
        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
            tracing::error!("Failed to read config file {}: {}", path.display(), e);
            std::process::exit(1);
        });
        toml::from_str(&content).unwrap_or_else(|e| {
            tracing::error!("Failed to parse config file {}: {}", path.display(), e);
            std::process::exit(1);
        })
    } else {
        let config = show_setup_dialog();
        save_config(&config, path);
        config
    }
}

pub fn save_config(config: &AggregatorConfig, path: &PathBuf) {
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
    listen_port_str: String,
    db: String,
    db_enabled: bool,
    ws_port_str: String,
    ws_port_enabled: bool,
    result: Arc<Mutex<Option<AggregatorConfig>>>,
    should_close: bool,
    heading: &'static str,
    default_test: bool,
    test: Option<AggregatorTestOverrides>,
    allow_injections: bool,
}

impl SetupApp {
    fn new(result: Arc<Mutex<Option<AggregatorConfig>>>) -> Self {
        Self {
            listen_port_str: "7878".to_string(),
            db: "localhost/nuzlocke".to_string(),
            db_enabled: false,
            ws_port_str: "9090".to_string(),
            ws_port_enabled: false,
            result,
            should_close: false,
            heading: "First-Run Setup",
            default_test: false,
            test: None,
            allow_injections: true,
        }
    }

    fn from_existing(result: Arc<Mutex<Option<AggregatorConfig>>>, cfg: &AggregatorConfig) -> Self {
        let (db, db_enabled) = match &cfg.db {
            Some(s) => (
                s.trim_start_matches("postgresql://")
                    .trim_start_matches("postgres://")
                    .to_string(),
                true,
            ),
            None => ("localhost/nuzlocke".to_string(), false),
        };
        let (ws_port_str, ws_port_enabled) = match cfg.ws_port {
            Some(p) => (p.to_string(), true),
            None => ("9090".to_string(), false),
        };
        Self {
            listen_port_str: cfg.listen_port.to_string(),
            db,
            db_enabled,
            ws_port_str,
            ws_port_enabled,
            result,
            should_close: false,
            heading: "Edit Config",
            default_test: cfg.default_test,
            test: cfg.test.clone(),
            allow_injections: cfg.allow_injections,
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
        ui.heading(format!("FireRed Aggregator — {}", self.heading));
        ui.label("Trackers connect to the aggregator — no addresses needed here.");
        ui.separator();
        ui.add_space(4.0);

        egui::Grid::new("setup_grid")
            .num_columns(2)
            .spacing([12.0, 10.0])
            .min_col_width(130.0)
            .show(ui, |ui| {
                // Listen port
                ui.label("Listen port:");
                ui.vertical(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.listen_port_str).desired_width(80.0),
                    );
                    ui.small("trackers connect to this port");
                });
                ui.end_row();

                // Database (optional)
                ui.checkbox(&mut self.db_enabled, "Database:");
                ui.add_enabled_ui(self.db_enabled, |ui| {
                    ui.vertical(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.db)
                                .desired_width(300.0)
                                .hint_text("localhost/nuzlocke"),
                        );
                        ui.small("postgresql:// is added automatically if omitted");
                    });
                });
                ui.end_row();

                // WebSocket overlay port (optional)
                ui.checkbox(&mut self.ws_port_enabled, "WebSocket overlay:");
                ui.add_enabled_ui(self.ws_port_enabled, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("port:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.ws_port_str)
                                    .desired_width(70.0),
                            );
                        });
                        ui.small("enables headless mode for OBS browser source");
                    });
                });
                ui.end_row();

                // Injection API toggle
                ui.checkbox(&mut self.allow_injections, "Allow injections:");
                ui.vertical(|ui| {
                    ui.small("enable give_item, make_shiny, change_species, etc.");
                });
                ui.end_row();
            });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        let listen_parse: Result<u16, _> = self.listen_port_str.trim().parse();
        let ws_parse: Result<u16, _> = self.ws_port_str.trim().parse();
        let listen_ok = listen_parse.is_ok();
        let ws_ok = !self.ws_port_enabled || ws_parse.is_ok();

        ui.horizontal(|ui| {
            let btn = ui.add_enabled(listen_ok && ws_ok, egui::Button::new("Save & Continue"));
            if btn.clicked() {
                let db = if self.db_enabled {
                    let raw = self.db.trim().to_string();
                    Some(
                        if raw.starts_with("postgresql://") || raw.starts_with("postgres://") {
                            raw
                        } else {
                            format!("postgresql://{}", raw)
                        },
                    )
                } else {
                    None
                };

                let ws_port = if self.ws_port_enabled {
                    ws_parse.ok()
                } else {
                    None
                };

                *self.result.lock().unwrap() = Some(AggregatorConfig {
                    listen_port: listen_parse.unwrap_or(7878),
                    db,
                    ws_port,
                    default_test: self.default_test,
                    test: self.test.clone(),
                    allow_injections: self.allow_injections,
                    twitch: None,
                });
                self.should_close = true;
            }

            if !listen_ok {
                ui.label(
                    egui::RichText::new("  Invalid listen port (1–65535)")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            } else if !ws_ok {
                ui.label(
                    egui::RichText::new("  Invalid WebSocket port (1–65535)")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .small(),
                );
            }
        });
    }
}

fn show_setup_dialog() -> AggregatorConfig {
    run_setup_window(None)
}

fn show_config_editor_from(existing: &AggregatorConfig) -> AggregatorConfig {
    run_setup_window(Some(existing))
}

fn run_setup_window(existing: Option<&AggregatorConfig>) -> AggregatorConfig {
    let result: Arc<Mutex<Option<AggregatorConfig>>> = Arc::new(Mutex::new(None));
    let result_for_app = result.clone();

    let app: SetupApp = match existing {
        Some(cfg) => SetupApp::from_existing(result_for_app, cfg),
        None => SetupApp::new(result_for_app),
    };

    let title = if existing.is_some() {
        "FireRed Aggregator — Edit Config"
    } else {
        "FireRed Aggregator — Setup"
    };

    let _ = eframe::run_native(
        title,
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([520.0, 280.0])
                .with_resizable(false),
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
    let existing: Option<AggregatorConfig> = if path.exists() {
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
