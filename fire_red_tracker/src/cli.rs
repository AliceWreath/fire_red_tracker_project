use clap::{Parser, Subcommand};

/// Real-time Pokémon FireRed party and encounter tracker.
///
/// Settings (ROM path, database, clean mode, default operating mode) are read
/// from the config file at first launch and saved for future runs.  Any value
/// can be overridden for a single run with the corresponding argument below.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Override the ROM path stored in the config file for this run.
    #[arg(value_name = "ROM")]
    pub rom: Option<String>,

    /// Path to the config file (default: ~/.config/fire_red_tracker/config.toml).
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,

    /// Override: enable ability name display (only reliable on unmodified ROMs).
    /// Merges with the config value — passing this flag enables clean mode even
    /// if the config has it set to false.
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

    /// Override the operating mode for this run only.
    /// Omit to use the mode stored in the config file.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Mode override subcommands — force a specific operating mode for this run.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Force server mode: run as a headless TCP server streaming game state to clients.
    Server {
        /// Port to listen on (overrides config server_port for this run).
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
    /// Force client mode: connect to a tracker server and display its game state.
    Client {
        /// Server hostname or IP address (overrides config client_host for this run).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Server port (overrides config client_port for this run).
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
}
