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

    /// Enable ability name display (only reliable on unmodified ROMs).
    #[arg(long, default_value_t = false)]
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

    /// Check GitHub for a newer release and replace this binary if one is found.
    #[arg(long, default_value_t = false)]
    pub update: bool,

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
