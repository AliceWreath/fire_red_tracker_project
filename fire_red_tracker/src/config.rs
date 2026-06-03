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
}

fn default_aggregator_host() -> String { "127.0.0.1".to_string() }
fn default_aggregator_port() -> u16 { 7878 }

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
    result:           Arc<Mutex<Option<TrackerConfig>>>,
    should_close:     bool,
}

impl SetupApp {
    fn new(result: Arc<Mutex<Option<TrackerConfig>>>) -> Self {
        Self {
            rom:             String::new(),
            db:              "localhost/nuzlocke".to_string(),
            clean:           false,
            mode:            ConfigMode::Standalone,
            aggregator_host: "127.0.0.1".to_string(),
            aggregator_port: "7878".to_string(),
            result,
            should_close:    false,
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
        ui.heading("FireRed Tracker — First-Run Setup");
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
                }
            });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        let rom_ok = !self.rom.trim().is_empty();
        let port_parse: Result<u16, _> = self.aggregator_port.parse();
        let port_ok = self.mode != ConfigMode::Connected || port_parse.is_ok();
        let can_save = rom_ok && port_ok;

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
                    rom:             self.rom.trim().to_string(),
                    db,
                    clean:           self.clean,
                    mode:            self.mode.clone(),
                    aggregator_host: self.aggregator_host.trim().to_string(),
                    aggregator_port: port_parse.unwrap_or(7878),
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
            }
        });
    }
}

fn show_setup_dialog() -> TrackerConfig {
    let result: Arc<Mutex<Option<TrackerConfig>>> = Arc::new(Mutex::new(None));
    let result_for_app = result.clone();

    let _ = eframe::run_native(
        "FireRed Tracker — Setup",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("FireRed Tracker — First-Run Setup")
                .with_inner_size([520.0, 300.0])
                .with_resizable(false),
            ..Default::default()
        },
        Box::new(move |_cc| Ok(Box::new(SetupApp::new(result_for_app)))),
    );

    result.lock().unwrap().take().unwrap_or_else(|| {
        println!("Setup cancelled.");
        std::process::exit(0);
    })
}
