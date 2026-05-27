//! # CLI argument definitions
//!
//! Defines the command-line interface for the FireRed Tracker using [`clap`].
//! The top-level [`Cli`] struct is parsed in `main` and used to derive the
//! operating [`Mode`] and ROM path.

use clap::{Parser, Subcommand};

/// Real-time Pokémon FireRed party and encounter tracker.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to the FireRed ROM file. Wrap in quotes for paths containing spaces.
    /// Not required in client mode.
    #[arg(value_name = "ROM")]
    pub rom: Option<String>,

    /// Enable ability name display. Only reliable on unmodified ("clean") ROMs.
    #[arg(long, default_value_t = false)]
    pub clean: bool,

    /// Start a brand-new run instead of resuming the most recent one.
    #[arg(long, default_value_t = false)]
    pub new_run: bool,

    /// Resume a specific run by its numeric ID (overrides --new-run).
    #[arg(long)]
    pub run_id: Option<u32>,

    /// Print all stored runs and exit without launching the tracker.
    #[arg(long, default_value_t = false)]
    pub list_runs: bool,

    /// PostgreSQL connection string.
    /// Example: postgresql://user:password@host/dbname
    /// The database must already exist on the server.
    #[arg(long, default_value = "postgresql://localhost/nuzlocke")]
    pub db: String,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Operating mode subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run as a headless TCP server, streaming game state to connected clients.
    Server {
        /// Port to listen on.
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
    /// Connect to a tracker server and display its game state.
    Client {
        /// Server hostname or IP address.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Server port.
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
}
