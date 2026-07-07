//! Aggregator configuration types, TOML loading, and the first-run setup UI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorConfig {
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
    /// Directory for automatic JSON run backups written when `game_cleared` is
    /// first detected (optional). The file is named `run_<id>_<timestamp>.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<String>,
    /// Hours between scheduled full-database JSON backups written to
    /// `backup_dir` as `db_backup_<timestamp>.json`. Unset or 0 = disabled.
    /// Game-clear backups are unaffected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_interval_hours: Option<u32>,
    /// How many scheduled backup files to retain in `backup_dir`; older
    /// `db_backup_*.json` files are deleted after each snapshot (default 10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_keep: Option<u32>,
    /// LiveSplit One TCP host for the aggregator-side split bridge. Splits fire
    /// on badge events (when `livesplit_split_on_badges` is true) and on game
    /// clear (always, when this host is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub livesplit_host: Option<String>,
    /// LiveSplit One TCP port (default 16834).
    #[serde(default = "default_livesplit_port", skip_serializing_if = "is_default_livesplit_port")]
    pub livesplit_port: u16,
    /// Fire a LiveSplit split every time a new gym badge is earned.
    #[serde(default)]
    pub livesplit_split_on_badges: bool,
    /// Discord Application Commands (slash commands) integration.
    /// Register with `POST /interactions` as your interactions endpoint URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_slash: Option<DiscordSlashConfig>,
    /// Discord persistent live-status embed. The bot edits a pinned message in a
    /// channel every `update_interval_secs` seconds with current party/run info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_live_embed: Option<DiscordLiveEmbedConfig>,
    /// Discord run thread: creates a new thread in a channel at run start and
    /// posts milestone messages (badge, death, shiny, game_cleared) as replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_run_thread: Option<DiscordRunThreadConfig>,
    /// YouTube Live chat bot. Polls the YouTube Data API for new chat messages
    /// and responds to `!party`, `!deaths`, `!shinies`, `!status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube_chat: Option<YouTubeChatConfig>,
    /// Manual gTrainers ROM offset override (byte offset within the file, not
    /// the GBA bus address).  When set, all auto-detection is skipped.
    /// Example: if a ROM tool reports the table at bus address `0x08240000`,
    /// set this to `0x240000`.  Leave unset to use auto-detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trainer_table_rom_offset: Option<usize>,
}

/// Discord Application Commands configuration.
///
/// Add a `[discord_slash]` section to `~/.config/fire_red_aggregator/config.toml`:
/// ```toml
/// [discord_slash]
/// app_id     = 123456789012345678       # Application ID from Discord dev portal
/// public_key = "abc123..."              # Ed25519 public key (hex) from dev portal
/// token      = "Bot MTc..."             # Bot token for registering commands
/// guild_id   = 987654321098765432       # Guild ID (omit for global commands)
/// ```
/// Then set your interactions endpoint URL in the Discord dev portal to
/// `https://your-aggregator-host/interactions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSlashConfig {
    /// Discord Application ID.
    pub app_id: u64,
    /// Ed25519 public key (lowercase hex) from the Discord dev portal.
    pub public_key: String,
    /// Bot token for registering slash commands at startup.
    pub token: String,
    /// Guild ID for guild-scoped commands (faster to update than global commands).
    /// If omitted, commands are registered globally (takes up to 1 hour to propagate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<u64>,
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

fn default_true() -> bool { true }
fn default_retroarch_port() -> u16 { 55355 }
fn is_default_retroarch_port(v: &u16) -> bool { *v == 55355 }
/// Discord persistent live-status embed configuration.
///
/// ```toml
/// [discord_live_embed]
/// bot_token     = "Bot MTc..."
/// channel_id    = 123456789012345678   # channel that holds the pinned message
/// message_id    = 987654321098765432   # ID of the existing message to edit
/// update_interval_secs = 30            # how often to refresh (default 30)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordLiveEmbedConfig {
    /// Discord bot token (must have `Send Messages` and `Read Message History` in the channel).
    pub bot_token: String,
    /// Channel ID where the status message lives.
    pub channel_id: u64,
    /// ID of the existing message to edit. Create the initial message manually, then copy its ID.
    pub message_id: u64,
    /// How often to update the embed in seconds (default 30, minimum 10).
    #[serde(default = "default_embed_interval")]
    pub update_interval_secs: u64,
}

fn default_embed_interval() -> u64 { 30 }

/// Discord run thread configuration.
///
/// ```toml
/// [discord_run_thread]
/// bot_token  = "Bot MTc..."
/// channel_id = 123456789012345678   # channel to create threads in
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordRunThreadConfig {
    /// Discord bot token (must have `Create Public Threads` in the channel).
    pub bot_token: String,
    /// Channel ID where run threads will be created.
    pub channel_id: u64,
}

/// YouTube Live chat bot configuration.
///
/// ```toml
/// [youtube_chat]
/// api_key      = "AIza..."                # YouTube Data API v3 key
/// broadcast_id = "LIVE_BROADCAST_ID"      # YouTube Live broadcast ID (from the URL)
/// slot         = 0                        # tracker slot index (default 0)
/// poll_secs    = 15                       # polling interval in seconds (default 15)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeChatConfig {
    /// YouTube Data API v3 key (restricted to YouTube Data API v3).
    pub api_key: String,
    /// YouTube Live broadcast ID. Found in the live broadcast URL:
    /// `https://www.youtube.com/watch?v=<broadcast_id>`.
    pub broadcast_id: String,
    /// Tracker slot index to read live state from (default 0).
    #[serde(default)]
    pub slot: usize,
    /// How often to poll the YouTube API in seconds (default 15, min 5).
    #[serde(default = "default_yt_poll_secs")]
    pub poll_secs: u64,
}

fn default_yt_poll_secs() -> u64 { 15 }

fn default_poll_ms_agg() -> u64 { 100 }
fn is_default_poll_ms_agg(v: &u64) -> bool { *v == 100 }
fn default_livesplit_port() -> u16 { 16834 }
fn is_default_livesplit_port(v: &u16) -> bool { *v == 16834 }

/// Config overrides applied when `--test` is active (or `default_test = true`).
/// All fields are optional; omit a field to inherit the base config value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregatorTestOverrides {
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

pub fn load_or_prompt(path: &PathBuf) -> Result<AggregatorConfig, String> {
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file {}: {}", path.display(), e))?;
        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config file {}: {}", path.display(), e))
    } else {
        let config = show_setup_dialog();
        save_config(&config, path);
        Ok(config)
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

/// An `AggregatorConfig` with every field at its serde default — the same
/// values an empty config file would produce. Used as the base for configs
/// built by the setup/editor UIs so fields those UIs don't expose keep
/// correct defaults instead of hardcoded ones.
fn empty_config() -> AggregatorConfig {
    toml::from_str("").expect("empty AggregatorConfig must deserialize via serde defaults")
}

struct SetupApp {
    /// The config being edited. Fields without a UI control (twitch,
    /// backup_*, livesplit_*, discord_*, youtube_chat, trainer_table_rom_offset)
    /// are preserved from here on save instead of being reset.
    base: AggregatorConfig,
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
            base: empty_config(),
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
        if let Some(h) = &cfg.retroarch_host
            && !all_hosts.contains(h)
        {
            all_hosts.push(h.clone());
        }
        Self {
            base: cfg.clone(),
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
                            if ui.small_button("Browse…").clicked()
                                && let Some(p) = rfd::FileDialog::new()
                                    .add_filter("GBA ROM", &["gba"])
                                    .pick_file()
                            {
                                self.rom_path = p.to_string_lossy().into_owned();
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

            let ws_parse: Result<u16, _> = self.ws_port_str.trim().parse();
            let port_parse: Result<u16, _> = self.retroarch_port_str.trim().parse();
            let ws_ok = !self.ws_port_enabled || ws_parse.is_ok();
            let port_ok = port_parse.is_ok();

            ui.horizontal(|ui| {
                let btn = ui.add_enabled(
                    ws_ok && port_ok,
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

                    // Start from `base` so fields this UI doesn't expose
                    // (twitch, backup_*, livesplit_*, discord_*, youtube_chat,
                    // trainer_table_rom_offset) survive a save unchanged.
                    let mut cfg = self.base.clone();
                    cfg.db = db;
                    cfg.ws_port = ws_port;
                    cfg.default_test = self.default_test;
                    cfg.test = self.test.clone();
                    cfg.allow_injections = self.allow_injections;
                    cfg.retroarch_host = None; // legacy field, merged into the list
                    cfg.retroarch_hosts = hosts;
                    cfg.retroarch_port = port_parse.unwrap_or(55355);
                    cfg.rom_path = rom;
                    cfg.poll_ms = self.poll_ms_str.trim().parse::<u64>().unwrap_or(100).clamp(20, 2000);
                    cfg.dupes_clause = self.dupes_clause;
                    cfg.allow_species_repeats = self.allow_species_repeats;
                    cfg.run_start_balls = self.run_start_balls_str.trim().parse().ok();
                    cfg.direct_mode = self.direct_mode;
                    *self.result.lock().unwrap_or_else(|p| p.into_inner()) = Some(cfg);
                    self.should_close = true;
                }

                for msg in [
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

    result.lock().unwrap_or_else(|p| p.into_inner()).take().unwrap_or_else(|| {
        println!("Setup cancelled.");
        std::process::exit(0);
    })
}

/// Open the config editor window, pre-filled with the existing config if the
/// file exists, then save the result. Called by `--config-editor`.
pub fn run_config_editor(path: &PathBuf) -> Result<(), String> {
    let existing: Option<AggregatorConfig> = if path.exists() {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file {}: {}", path.display(), e))?;
        Some(toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config file {}: {}", path.display(), e))?)
    } else {
        None
    };

    let new_cfg = match existing {
        Some(ref cfg) => show_config_editor_from(cfg),
        None => show_setup_dialog(),
    };
    save_config(&new_cfg, path);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_has_serde_defaults() {
        let cfg = empty_config();
        assert_eq!(cfg.db, None);
        assert_eq!(cfg.ws_port, None);
        assert!(cfg.allow_injections, "allow_injections defaults to true");
        assert_eq!(cfg.retroarch_port, 55355);
        assert_eq!(cfg.poll_ms, 100);
        assert_eq!(cfg.livesplit_port, 16834);
        assert_eq!(cfg.backup_dir, None);
        assert_eq!(cfg.backup_interval_hours, None);
        assert_eq!(cfg.backup_keep, None);
    }

    /// Guards the editor-save fix: fields the settings UIs don't expose must
    /// survive a parse → (edit) → serialize cycle instead of being reset.
    #[test]
    fn non_gui_fields_survive_toml_roundtrip() {
        let toml_in = r#"
            db = "postgresql://example/nuzlocke"
            backup_dir = "/tmp/backups"
            backup_interval_hours = 6
            backup_keep = 4
            livesplit_host = "127.0.0.1"
            livesplit_split_on_badges = true
            trainer_table_rom_offset = 0x23CAE0
        "#;
        let cfg: AggregatorConfig = toml::from_str(toml_in).unwrap();

        // Simulate what the editors do on save: clone the base and overwrite
        // only a GUI-exposed field.
        let mut edited = cfg.clone();
        edited.direct_mode = true;

        let out = toml::to_string(&edited).unwrap();
        let reparsed: AggregatorConfig = toml::from_str(&out).unwrap();
        assert_eq!(reparsed.backup_dir.as_deref(), Some("/tmp/backups"));
        assert_eq!(reparsed.backup_interval_hours, Some(6));
        assert_eq!(reparsed.backup_keep, Some(4));
        assert_eq!(reparsed.livesplit_host.as_deref(), Some("127.0.0.1"));
        assert!(reparsed.livesplit_split_on_badges);
        assert_eq!(reparsed.trainer_table_rom_offset, Some(0x23CAE0));
        assert!(reparsed.direct_mode, "edited field applied");
        assert_eq!(reparsed.db.as_deref(), Some("postgresql://example/nuzlocke"));
    }
}
