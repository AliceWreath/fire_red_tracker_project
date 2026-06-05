use clap::{Parser, Subcommand};

/// Real-time Pokémon FireRed party and encounter tracker.
///
/// Settings are read from the config file at first launch and saved for future
/// runs. Any value can be overridden for a single run with the flags below.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Override the ROM path stored in the config file for this run.
    #[arg(value_name = "ROM")]
    pub rom: Option<String>,

    /// Path to the config file (default: ~/.config/fire_red_tracker/config.toml).
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Accepted for backward compatibility; no longer has any effect.
    /// Ability names are now always resolved from the provided ROM file.
    #[arg(long, default_value_t = false, hide = true)]
    pub clean: bool,

    /// Override the database connection string stored in the config file.
    #[arg(long, value_name = "CONN")]
    pub db: Option<String>,

    /// Start a brand-new run instead of resuming the most recent one.
    #[arg(long, default_value_t = false)]
    pub new_run: bool,

    /// Resume a specific run by its numeric ID (overrides --new-run).
    #[arg(long)]
    pub run_id: Option<u32>,

    /// Print all stored runs and exit without launching the tracker.
    #[arg(long, default_value_t = false)]
    pub list_runs: bool,

    /// Open the configuration editor and exit.
    #[arg(long, default_value_t = false)]
    pub config_editor: bool,

    /// Check GitHub for a newer release and replace this binary if one is found.
    #[arg(long, default_value_t = false)]
    pub update: bool,

    /// Scan SaveBlock1 for Pokéball item slots and print their offsets, then exit.
    /// Run with at least one ball in your bag to identify BALLS_POCKET_SAVE_BLOCK_OFFSET.
    #[arg(long, default_value_t = false)]
    pub scan_balls_pocket: bool,

    /// Scan EWRAM for the bag security key and print SaveBlock2-relative offsets, then exit.
    /// Pass the exact number of Pokéballs currently in your bag (e.g. --scan-security-key=5).
    /// Use the printed offset to update SECURITY_KEY_OFFSET in game.rs.
    #[arg(long, value_name = "QTY")]
    pub scan_security_key: Option<u16>,

    /// Preferred display column in the aggregator (1 = first, 2 = second, …).
    /// Overrides the value in the config file for this run only.
    #[arg(long, value_name = "N")]
    pub preferred_player: Option<u8>,

    /// Apply the [test] section from the config file on top of normal settings,
    /// and always start a new run (implies --new-run).
    /// Explicit flags (--db, --preferred-player, connect host/port) still win.
    #[arg(long, default_value_t = false)]
    pub test: bool,

    /// Override the operating mode for this run only.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Mode override subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Connect to an aggregator and stream game state to it (headless).
    Connect {
        /// Aggregator host or IP address.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Aggregator port.
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
}
