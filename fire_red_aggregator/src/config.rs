use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct AggregatorConfig {
    /// Tracker server addresses in `host:port` format.
    pub addrs: Vec<String>,
    /// PostgreSQL connection string (optional).
    pub db: Option<String>,
    /// WebSocket overlay port (optional — omit for GUI window mode).
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

pub fn save_config(config: &AggregatorConfig, path: &PathBuf) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Warning: could not create config directory: {}", e);
        }
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
    /// One address per line.
    addrs_text: String,
    db: String,
    db_enabled: bool,
    ws_port_str: String,
    ws_port_enabled: bool,
    result: Arc<Mutex<Option<AggregatorConfig>>>,
    should_close: bool,
}

impl SetupApp {
    fn new(result: Arc<Mutex<Option<AggregatorConfig>>>) -> Self {
        Self {
            addrs_text: String::new(),
            db: "localhost/nuzlocke".to_string(),
            db_enabled: false,
            ws_port_str: "9090".to_string(),
            ws_port_enabled: false,
            result,
            should_close: false,
        }
    }

    fn addrs_valid(&self) -> bool {
        self.addrs_text.lines().any(|l| !l.trim().is_empty())
    }

    fn build_config(&self) -> AggregatorConfig {
        let addrs = self
            .addrs_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();

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
            self.ws_port_str.trim().parse().ok()
        } else {
            None
        };

        AggregatorConfig { addrs, db, ws_port }
    }
}

impl eframe::App for SetupApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.should_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        {
            ui.add_space(8.0);
            ui.heading("FireRed Aggregator — First-Run Setup");
            ui.label("These settings will be saved to your config file for future runs.");
            ui.separator();
            ui.add_space(4.0);

            egui::Grid::new("setup_grid")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .min_col_width(130.0)
                .show(ui, |ui| {
                    // Server addresses
                    ui.label("Server addresses:");
                    ui.vertical(|ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.addrs_text)
                                .desired_width(300.0)
                                .desired_rows(3)
                                .hint_text("localhost:7878\nlocalhost:7979"),
                        );
                        ui.small("one host:port per line");
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
                });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);

            let ok = self.addrs_valid();

            ui.horizontal(|ui| {
                let btn = ui.add_enabled(ok, egui::Button::new("Save & Continue"));
                if btn.clicked() {
                    *self.result.lock().unwrap() = Some(self.build_config());
                    self.should_close = true;
                }

                if !ok {
                    ui.label(
                        egui::RichText::new("  At least one server address is required")
                            .color(egui::Color32::from_rgb(220, 80, 80))
                            .small(),
                    );
                }
            });
        }
    }
}

fn show_setup_dialog() -> AggregatorConfig {
    let result: Arc<Mutex<Option<AggregatorConfig>>> = Arc::new(Mutex::new(None));
    let result_for_app = result.clone();

    let _ = eframe::run_native(
        "FireRed Aggregator — Setup",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("FireRed Aggregator — First-Run Setup")
                .with_inner_size([520.0, 340.0])
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
