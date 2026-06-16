//! Aggregator configuration types, TOML loading, and the first-run setup UI.

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
    // ── Direct mode (aggregator polls RetroArch directly) ──────────────────
    /// Single RetroArch host shorthand (legacy / convenience).
    /// If both this and `retroarch_hosts` are set, this is merged into the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retroarch_host: Option<String>,
    /// RetroArch hosts to poll directly (one slot per entry).
    /// Example: `retroarch_hosts = ["192.168.1.50", "192.168.1.51"]`
    /// Requires `rom_path` and (for headless) `ws_port` to also be set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retroarch_hosts: Vec<String>,
    /// RetroArch network-commands UDP port (applies to all hosts). Defaults to 55355.
    #[serde(default = "default_retroarch_port", skip_serializing_if = "is_default_retroarch_port")]
    pub retroarch_port: u16,
    /// Path to the FireRed ROM on the aggregator's machine (required for direct mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rom_path: Option<String>,
    /// Game-polling interval in ms for direct mode (default 100, range 20–2000).
    #[serde(default = "default_poll_ms_agg", skip_serializing_if = "is_default_poll_ms_agg")]
    pub poll_ms: u64,
    /// How the dupes clause is applied in direct mode.
    #[serde(default)]
    pub dupes_clause: fire_red_game_loop::config::DupesClauseMode,
    /// Allow same species on multiple routes (randomizer mode).
    #[serde(default)]
    pub allow_species_repeats: bool,
    /// Minimum Pokéball count before run tracking starts (default 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_start_balls: Option<u8>,
    /// Enable direct mode even when no hosts are pre-configured.
    /// Activates the /join page so players can connect on demand.
    #[serde(default)]
    pub direct_mode: bool,
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

fn default_listen_port() -> u16 { 7878 }
fn default_true() -> bool { true }
fn default_retroarch_port() -> u16 { 55355 }
fn is_default_retroarch_port(v: &u16) -> bool { *v == 55355 }
fn default_poll_ms_agg() -> u64 { 100 }
fn is_default_poll_ms_agg(v: &u64) -> bool { *v == 100 }

/// Config overrides applied when `--test` is active (or `default_test = true`).
/// All fields are optional; omit a field to inherit the base config value.
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
    // Direct mode
    direct_mode: bool,
    retroarch_hosts_str: String,
    retroarch_port_str: String,
    rom_path: String,
    poll_ms_str: String,
    dupes_clause: fire_red_game_loop::config::DupesClauseMode,
    allow_species_repeats: bool,
    run_start_balls_str: String,
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
            direct_mode: false,
            retroarch_hosts_str: String::new(),
            retroarch_port_str: "55355".to_string(),
            rom_path: String::new(),
            poll_ms_str: "100".to_string(),
            dupes_clause: fire_red_game_loop::config::DupesClauseMode::default(),
            allow_species_repeats: false,
            run_start_balls_str: "5".to_string(),
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
        // Merge legacy single host into the list for display.
        let mut all_hosts = cfg.retroarch_hosts.clone();
        if let Some(h) = &cfg.retroarch_host {
            if !all_hosts.contains(h) { all_hosts.push(h.clone()); }
        }
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
            direct_mode: cfg.direct_mode,
            retroarch_hosts_str: all_hosts.join("\n"),
            retroarch_port_str: cfg.retroarch_port.to_string(),
            rom_path: cfg.rom_path.clone().unwrap_or_default(),
            poll_ms_str: cfg.poll_ms.to_string(),
            dupes_clause: cfg.dupes_clause,
            allow_species_repeats: cfg.allow_species_repeats,
            run_start_balls_str: cfg.run_start_balls.unwrap_or(5).to_string(),
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
        use fire_red_game_loop::config::DupesClauseMode;

        ui.add_space(8.0);
        ui.heading(format!("FireRed Aggregator — {}", self.heading));
        ui.separator();
        ui.add_space(4.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("setup_grid")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .min_col_width(140.0)
                .show(ui, |ui| {
                    // Listen port
                    ui.label("Listen port:");
                    ui.vertical(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.listen_port_str).desired_width(80.0));
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
                                ui.add(egui::TextEdit::singleline(&mut self.ws_port_str).desired_width(70.0));
                            });
                            ui.small("enables headless mode for OBS browser source");
                        });
                    });
                    ui.end_row();

                    // Injection API toggle
                    ui.checkbox(&mut self.allow_injections, "Allow injections:");
                    ui.small("enable give_item, make_shiny, change_species, etc.");
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.label(egui::RichText::new("Direct Mode (optional — polls RetroArch directly)").strong());
            ui.small("Enable to allow players to connect via the /join page without pre-configuring hosts.");
            ui.add_space(4.0);

            egui::Grid::new("direct_grid")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .min_col_width(140.0)
                .show(ui, |ui| {
                    // Direct mode toggle
                    ui.checkbox(&mut self.direct_mode, "Enable direct mode:");
                    ui.small("activates /join page — required if no hosts are pre-configured");
                    ui.end_row();

                    // ROM path
                    ui.label("ROM path:");
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.rom_path)
                                    .desired_width(260.0)
                                    .hint_text("/path/to/firered.gba"),
                            );
                            if ui.small_button("Browse…").clicked() {
                                if let Some(p) = rfd::FileDialog::new()
                                    .add_filter("GBA ROM", &["gba"])
                                    .pick_file()
                                {
                                    self.rom_path = p.to_string_lossy().into_owned();
                                }
                            }
                        });
                        ui.small("required for direct mode");
                    });
                    ui.end_row();

                    // RetroArch hosts
                    ui.label("RetroArch hosts:");
                    ui.vertical(|ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.retroarch_hosts_str)
                                .desired_width(300.0)
                                .desired_rows(3)
                                .hint_text("192.168.1.50\n192.168.1.51"),
                        );
                        ui.small("one IP per line — players can also connect via /join");
                    });
                    ui.end_row();

                    // RetroArch port
                    ui.label("RetroArch port:");
                    ui.vertical(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.retroarch_port_str).desired_width(70.0));
                        ui.small("default 55355");
                    });
                    ui.end_row();

                    // Poll interval
                    ui.label("Poll interval (ms):");
                    ui.vertical(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.poll_ms_str).desired_width(70.0));
                        ui.small("range 20–2000, default 100");
                    });
                    ui.end_row();

                    // Dupes clause
                    ui.label("Dupes clause:");
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.dupes_clause, DupesClauseMode::Off, "Off");
                            ui.radio_value(&mut self.dupes_clause, DupesClauseMode::PerPlayer, "Per player");
                            ui.radio_value(&mut self.dupes_clause, DupesClauseMode::Shared, "Shared");
                        });
                    });
                    ui.end_row();

                    // Allow species repeats
                    ui.checkbox(&mut self.allow_species_repeats, "Allow species repeats:");
                    ui.small("for randomizers");
                    ui.end_row();

                    // Run start balls
                    ui.label("Run-start Pokéballs:");
                    ui.vertical(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.run_start_balls_str).desired_width(50.0));
                        ui.small("minimum balls before run tracking starts (default 5)");
                    });
                    ui.end_row();
                });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);

            let listen_parse: Result<u16, _> = self.listen_port_str.trim().parse();
            let ws_parse: Result<u16, _> = self.ws_port_str.trim().parse();
            let port_parse: Result<u16, _> = self.retroarch_port_str.trim().parse();
            let listen_ok = listen_parse.is_ok();
            let ws_ok = !self.ws_port_enabled || ws_parse.is_ok();
            let port_ok = port_parse.is_ok();

            ui.horizontal(|ui| {
                let btn = ui.add_enabled(
                    listen_ok && ws_ok && port_ok,
                    egui::Button::new("Save & Continue"),
                );
                if btn.clicked() {
                    let db = if self.db_enabled {
                        let raw = self.db.trim().to_string();
                        Some(if raw.starts_with("postgresql://") || raw.starts_with("postgres://") {
                            raw
                        } else {
                            format!("postgresql://{}", raw)
                        })
                    } else {
                        None
                    };
                    let ws_port = if self.ws_port_enabled { ws_parse.ok() } else { None };
                    let hosts: Vec<String> = self.retroarch_hosts_str
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect();
                    let rom = if self.rom_path.trim().is_empty() {
                        None
                    } else {
                        Some(self.rom_path.trim().to_string())
                    };

                    *self.result.lock().unwrap() = Some(AggregatorConfig {
                        listen_port: listen_parse.unwrap_or(7878),
                        db,
                        ws_port,
                        default_test: self.default_test,
                        test: self.test.clone(),
                        allow_injections: self.allow_injections,
                        twitch: None,
                        retroarch_host: None,
                        retroarch_hosts: hosts,
                        retroarch_port: port_parse.unwrap_or(55355),
                        rom_path: rom,
                        poll_ms: self.poll_ms_str.trim().parse::<u64>().unwrap_or(100).clamp(20, 2000),
                        dupes_clause: self.dupes_clause,
                        allow_species_repeats: self.allow_species_repeats,
                        run_start_balls: self.run_start_balls_str.trim().parse().ok(),
                        direct_mode: self.direct_mode,
                    });
                    self.should_close = true;
                }

                for msg in [
                    (!listen_ok).then_some("Invalid listen port"),
                    (!ws_ok).then_some("Invalid WebSocket port"),
                    (!port_ok).then_some("Invalid RetroArch port"),
                ].into_iter().flatten() {
                    ui.label(
                        egui::RichText::new(format!("  {}", msg))
                            .color(egui::Color32::from_rgb(220, 80, 80))
                            .small(),
                    );
                }
            });
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
                .with_inner_size([560.0, 620.0])
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
