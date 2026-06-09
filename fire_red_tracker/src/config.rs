use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How the dupes clause is applied when recording wild encounters.
///
/// Backward-compatible: existing `dupes_clause = true` in config deserialises as `Shared`;
/// `dupes_clause = false` (or absent) deserialises as `Off`.
///
/// New configs should use the explicit string form:
/// ```toml
/// dupes_clause = "off"        # no dupes check (default)
/// dupes_clause = "per_player" # skip if THIS player already caught this species
/// dupes_clause = "shared"     # skip if ANY player in the run has caught this species
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum DupesClauseMode {
    /// No dupes check — each player records all first encounters independently (default).
    #[default]
    Off,
    /// Per-player: skip if this player has previously caught this species anywhere in the run.
    PerPlayer,
    /// Shared / cross-player: skip if **any** player in the shared run has caught this species.
    /// In Soul Link or co-op runs this means one catch exempts all other players from needing
    /// to catch the same species.
    Shared,
}

impl Serialize for DupesClauseMode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            DupesClauseMode::Off       => "off",
            DupesClauseMode::PerPlayer => "per_player",
            DupesClauseMode::Shared    => "shared",
        })
    }
}

impl<'de> Deserialize<'de> for DupesClauseMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = DupesClauseMode;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, r#"bool or one of "off", "per_player", "shared""#)
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<DupesClauseMode, E> {
                Ok(if v { DupesClauseMode::Shared } else { DupesClauseMode::Off })
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<DupesClauseMode, E> {
                match v {
                    "off"        => Ok(DupesClauseMode::Off),
                    "per_player" => Ok(DupesClauseMode::PerPlayer),
                    "shared"     => Ok(DupesClauseMode::Shared),
                    _            => Err(E::unknown_variant(v, &["off", "per_player", "shared"])),
                }
            }
        }
        d.deserialize_any(V)
    }
}

impl fmt::Display for DupesClauseMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DupesClauseMode::Off       => "Off",
            DupesClauseMode::PerPlayer => "Per Player",
            DupesClauseMode::Shared    => "Shared",
        })
    }
}

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
    #[serde(default = "default_poll_ms", skip_serializing_if = "is_default_poll_ms")]
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
}

fn default_aggregator_host() -> String { "127.0.0.1".to_string() }
fn default_aggregator_port() -> u16 { 7878 }
fn default_poll_ms() -> u64 { 100 }
fn is_default_poll_ms(v: &u64) -> bool { *v == 100 }

// ---------------------------------------------------------------------------
// Webhook config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// POSTed when a party member dies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_url: Option<String>,
    /// Custom body template for death events (placeholders: {player}, {timestamp}, {pokemon.nickname}, etc.).
    /// When set, the rendered string is POSTed verbatim instead of the default JSON schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_template: Option<String>,
    /// POSTed when a new pokemon is added to the party (caught/gifted/traded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_url: Option<String>,
    /// Custom body template for catch events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_template: Option<String>,
    /// POSTed when a shiny wild pokemon is first encountered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shiny_url: Option<String>,
    /// Custom body template for shiny events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shiny_template: Option<String>,
    /// POSTed when the entire party is wiped and the run ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wipe_url: Option<String>,
    /// Custom body template for wipe events (pokemon placeholders expand to empty string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wipe_template: Option<String>,
}

impl WebhookConfig {
    pub fn is_empty(&self) -> bool {
        self.death_url.is_none()
            && self.death_template.is_none()
            && self.catch_url.is_none()
            && self.catch_template.is_none()
            && self.shiny_url.is_none()
            && self.shiny_template.is_none()
            && self.wipe_url.is_none()
            && self.wipe_template.is_none()
    }
}

// ---------------------------------------------------------------------------
// OBS WebSocket config
// ---------------------------------------------------------------------------

/// Optional OBS WebSocket integration — saves a replay buffer clip on game events.
///
/// Enable in `config.toml`:
/// ```toml
/// [obs]
/// clip_on_death = true
/// clip_on_shiny = true
/// clip_on_wipe  = true
/// # host = "localhost"   # default
/// # port = 4455          # default (OBS v5 WebSocket)
/// # password = "secret"  # only if authentication is enabled in OBS
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsConfig {
    #[serde(default = "default_obs_host")]
    pub host: String,
    #[serde(default = "default_obs_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub clip_on_death: bool,
    #[serde(default)]
    pub clip_on_shiny: bool,
    #[serde(default)]
    pub clip_on_wipe: bool,
}

fn default_obs_host() -> String { "localhost".to_string() }
fn default_obs_port() -> u16 { 4455 }

impl Default for ObsConfig {
    fn default() -> Self {
        Self {
            host:         default_obs_host(),
            port:         default_obs_port(),
            password:     None,
            clip_on_death: false,
            clip_on_shiny: false,
            clip_on_wipe:  false,
        }
    }
}

impl ObsConfig {
    pub fn is_default(&self) -> bool {
        !self.clip_on_death && !self.clip_on_shiny && !self.clip_on_wipe
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
// Validation
// ---------------------------------------------------------------------------

/// A single validation error returned by [`validate_config`].
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub field:   &'static str,
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
        errors.push(ConfigError { field: "rom", message: "path is empty".to_string() });
    } else {
        let path = std::path::Path::new(&cfg.rom);
        if !path.exists() {
            errors.push(ConfigError {
                field:   "rom",
                message: format!("file not found: {}", cfg.rom),
            });
        } else if std::fs::File::open(path).is_err() {
            errors.push(ConfigError {
                field:   "rom",
                message: format!("file exists but cannot be opened: {}", cfg.rom),
            });
        }
    }

    // Webhook URLs
    let wh_fields: &[(&'static str, &Option<String>)] = &[
        ("webhooks.death_url",  &cfg.webhooks.death_url),
        ("webhooks.catch_url",  &cfg.webhooks.catch_url),
        ("webhooks.shiny_url",  &cfg.webhooks.shiny_url),
        ("webhooks.wipe_url",   &cfg.webhooks.wipe_url),
    ];
    for (field, url_opt) in wh_fields {
        if let Some(url) = url_opt
            && !url.starts_with("http://") && !url.starts_with("https://")
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
    dupes_clause: DupesClauseMode,
    // Webhook URL and template fields
    death_url:         String,
    death_url_enabled: bool,
    death_template:    String,
    catch_url:         String,
    catch_url_enabled: bool,
    catch_template:    String,
    shiny_url:         String,
    shiny_url_enabled: bool,
    shiny_template:    String,
    wipe_url:          String,
    wipe_url_enabled:  bool,
    wipe_template:     String,
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
            dupes_clause:     DupesClauseMode::Off,
            death_url:         String::new(),
            death_url_enabled: false,
            death_template:    String::new(),
            catch_url:         String::new(),
            catch_url_enabled: false,
            catch_template:    String::new(),
            shiny_url:         String::new(),
            shiny_url_enabled: false,
            shiny_template:    String::new(),
            wipe_url:          String::new(),
            wipe_url_enabled:  false,
            wipe_template:     String::new(),
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
            dupes_clause:     cfg.dupes_clause,
            death_url:         wh.death_url.clone().unwrap_or_default(),
            death_url_enabled: wh.death_url.is_some(),
            death_template:    wh.death_template.clone().unwrap_or_default(),
            catch_url:         wh.catch_url.clone().unwrap_or_default(),
            catch_url_enabled: wh.catch_url.is_some(),
            catch_template:    wh.catch_template.clone().unwrap_or_default(),
            shiny_url:         wh.shiny_url.clone().unwrap_or_default(),
            shiny_url_enabled: wh.shiny_url.is_some(),
            shiny_template:    wh.shiny_template.clone().unwrap_or_default(),
            wipe_url:          wh.wipe_url.clone().unwrap_or_default(),
            wipe_url_enabled:  wh.wipe_url.is_some(),
            wipe_template:     wh.wipe_template.clone().unwrap_or_default(),
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

                // Dupes clause
                ui.separator();
                ui.end_row();
                ui.label(egui::RichText::new("Dupes clause").strong());
                ui.vertical(|ui| {
                    ui.selectable_value(&mut self.dupes_clause, DupesClauseMode::Off,       "Off — standard Nuzlocke (first encounter per area)");
                    ui.selectable_value(&mut self.dupes_clause, DupesClauseMode::PerPlayer, "Per Player — skip if you already caught this species");
                    ui.selectable_value(&mut self.dupes_clause, DupesClauseMode::Shared,    "Shared — skip if any player caught this species (Soul Link / co-op)");
                });
                ui.end_row();

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

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        let rom_ok = !self.rom.trim().is_empty();
        let port_val: Option<u16> = self.aggregator_port.trim().parse().ok().filter(|&p| p > 0);
        let port_ok = self.mode != ConfigMode::Connected || port_val.is_some();
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
                    aggregator_port:  port_val.unwrap_or(7878),
                    preferred_player: player_parse,
                    default_test:     self.default_test,
                    test:             self.test.clone(),
                    poll_ms:  default_poll_ms(),
                    webhooks: WebhookConfig {
                        death_url:      if self.death_url_enabled && !self.death_url.trim().is_empty() { Some(self.death_url.trim().to_string()) } else { None },
                        death_template: if self.death_url_enabled && !self.death_template.trim().is_empty() { Some(self.death_template.trim().to_string()) } else { None },
                        catch_url:      if self.catch_url_enabled && !self.catch_url.trim().is_empty() { Some(self.catch_url.trim().to_string()) } else { None },
                        catch_template: if self.catch_url_enabled && !self.catch_template.trim().is_empty() { Some(self.catch_template.trim().to_string()) } else { None },
                        shiny_url:      if self.shiny_url_enabled && !self.shiny_url.trim().is_empty() { Some(self.shiny_url.trim().to_string()) } else { None },
                        shiny_template: if self.shiny_url_enabled && !self.shiny_template.trim().is_empty() { Some(self.shiny_template.trim().to_string()) } else { None },
                        wipe_url:       if self.wipe_url_enabled  && !self.wipe_url.trim().is_empty()  { Some(self.wipe_url.trim().to_string())  } else { None },
                        wipe_template:  if self.wipe_url_enabled  && !self.wipe_template.trim().is_empty()  { Some(self.wipe_template.trim().to_string())  } else { None },
                    },
                    obs:          ObsConfig::default(),
                    dupes_clause: self.dupes_clause,
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
        assert!(!serialized.contains("poll_ms"), "poll_ms should be omitted when default");
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
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("dupes_clause = \"per_player\"\n")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::PerPlayer);
    }

    #[test]
    fn dupes_clause_string_shared() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("dupes_clause = \"shared\"\n")).unwrap();
        assert_eq!(cfg.dupes_clause, DupesClauseMode::Shared);
    }

    #[test]
    fn dupes_clause_serialises_as_string() {
        let cfg: TrackerConfig = toml::from_str(&minimal_toml("dupes_clause = \"per_player\"\n")).unwrap();
        let s = toml::to_string(&cfg).unwrap();
        assert!(s.contains("per_player"), "expected 'per_player' in serialised output: {s}");
    }

    // ── validate_config ──────────────────────────────────────────────────────

    fn cfg_with_rom(rom: &str) -> TrackerConfig {
        TrackerConfig {
            rom:             rom.to_string(),
            db:              "postgresql://localhost/test".to_string(),
            clean:           false,
            mode:            ConfigMode::default(),
            aggregator_host: "127.0.0.1".to_string(),
            aggregator_port: 7878,
            preferred_player: None,
            default_test:    false,
            test:            None,
            poll_ms:         100,
            webhooks:        WebhookConfig::default(),
            obs:             ObsConfig::default(),
            dupes_clause:    DupesClauseMode::Off,
        }
    }

    #[test]
    fn validate_config_empty_rom_is_error() {
        let cfg = cfg_with_rom("");
        let errs = validate_config(&cfg);
        assert!(errs.iter().any(|e| e.field == "rom"), "expected rom error, got: {errs:?}");
    }

    #[test]
    fn validate_config_nonexistent_rom_is_error() {
        let cfg = cfg_with_rom("/tmp/__nonexistent_rom_test_file__.gba");
        let errs = validate_config(&cfg);
        assert!(errs.iter().any(|e| e.field == "rom"), "expected rom error, got: {errs:?}");
    }

    #[test]
    fn validate_config_valid_rom_no_error() {
        // Create a temporary file so the path exists.
        let path = "/tmp/__test_rom_validate__.gba";
        std::fs::write(path, b"FAKE").unwrap();
        let cfg = cfg_with_rom(path);
        let errs = validate_config(&cfg);
        std::fs::remove_file(path).ok();
        assert!(!errs.iter().any(|e| e.field == "rom"), "unexpected rom error: {errs:?}");
    }

    #[test]
    fn validate_config_valid_webhook_url_no_error() {
        let mut cfg = cfg_with_rom("/tmp/__test_rom_validate2__.gba");
        std::fs::write("/tmp/__test_rom_validate2__.gba", b"FAKE").unwrap();
        cfg.webhooks.death_url = Some("https://discord.com/api/webhooks/123/abc".to_string());
        let errs = validate_config(&cfg);
        std::fs::remove_file("/tmp/__test_rom_validate2__.gba").ok();
        assert!(!errs.iter().any(|e| e.field.contains("death_url")), "unexpected error: {errs:?}");
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
        let cfg = WebhookConfig { death_url: Some("https://example.com".to_string()), ..Default::default() };
        assert!(!cfg.is_empty());
    }

    #[test]
    fn webhook_config_not_empty_when_template_set() {
        let cfg = WebhookConfig { death_template: Some("{event}".to_string()), ..Default::default() };
        assert!(!cfg.is_empty());
    }

    // ── ObsConfig::is_default ────────────────────────────────────────────────

    #[test]
    fn obs_config_is_default_when_no_clips_enabled() {
        assert!(ObsConfig::default().is_default());
    }

    #[test]
    fn obs_config_not_default_when_clip_on_death() {
        let cfg = ObsConfig { clip_on_death: true, ..Default::default() };
        assert!(!cfg.is_default());
    }
}
