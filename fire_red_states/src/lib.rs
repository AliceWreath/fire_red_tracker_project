use std::net::TcpStream;
use std::io::{Read, Write};

/// Maximum allowed network message size.
/// 
/// Used as a safeguard against malformed or malicious packets that could
/// otherwise allocate excessive memory.
/// 
/// Current limit: 20 MB.
const MAX_MESSAGE_SIZE: usize = 20 * 1024 * 1024; // 20 MB

/// Messages sent from a client to the server.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub enum ClientMessage {
    /// Request sprite textures for a list of Pokémon species IDs.
    RequestTextures(Vec<u16>),
    /// End the current active run (sets ended_at, stops recording data).
    EndRun,
    /// Start a new run and make it the active run.
    NewRun,
}

/// Messages sent from the server to connected clients.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ServerMessage {
    /// Full game state update.
    State(GameState),

    /// Collection of sprite textures requested by client.
    Textures(Vec<SpriteData>),

    /// Confirmation that the active run changed.
    /// `None` = run ended (no active run); `Some(id)` = new run ID.
    RunChanged(Option<u32>),
}

/// Serialized Pokemon sprite texture data for network transmission.
/// 
/// Pixel data is stored as zlib-compressed RGBA bytes.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SpriteData {
    pub species: u16,
    pub shiny: bool,
    pub pixels: Vec<u8>, // zlib-compressed RGBA bytes
    pub width: u32,
    pub height: u32,
}

/// Shared game state transmitted between server and clients.
/// 
/// Contains both the current player party and wild encounter data.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct GameState {
    /// Current player party pokemon.
    pub party: Vec<fire_red_party_monitor::Pokemon>,

    /// Wild encounter table/header data.
    pub encounters: fire_red_pokemon_data::WildPokemonHeader,

    /// Trainer name
    pub player_name: String,

    /// Current collected badges
    pub badge_state: Option<fire_red_badge::BadgeState>,
}

/// Network operating mode for the tracker.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum Mode {
    /// Run entirely locally with no networking.
    Standalone,

    /// Connect to an aggregator and stream game state to it.
    Connected {
        /// Aggregator host or IP address.
        host: String,
        /// Aggregator TCP port.
        port: u16,
    },
}

// ---------------------------------------------------------------------------
// Wire helpers — length-prefixed bincode frames
// ---------------------------------------------------------------------------

/// Serialilzes and sends a message over a TCP stream.
/// 
/// Messages are encoded using 'bincode' and prefixed with a 4-byte
/// big-endian length header.
/// 
/// # Arguments
/// 
/// * 'stream' - Connected TCP stream.
/// * 'msg' - Serializable message to send.
/// 
/// # Errors
/// 
/// Returns an error if serialization or network I/O fails.
/// 
/// # Protocol
/// 
/// Packet layout:
/// 
/// ```
/// [4-byte big-endian length][bincode-encoded message bytes]
/// ```
pub fn send_message<T: serde::Serialize>(stream: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let encoded =
        bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

/// Receives and deserializes a message from a TCP stream.
/// 
/// Reads a 4-byte big-endian prefix followed by a bincode message of the specified length.
/// 
/// # Type Parameters
/// 
/// * 'T' - Message type implementing 'DeserializedOwned'.
/// 
/// # Arguments
/// 
/// * 'stream' - Connected TCP stream.
/// 
/// # Errors
/// 
/// returns an error if:
/// 
/// - The connection closes unexpectedly.
/// - The packet exceeds ['MAX_MESSAGE_SIZE'].
/// - Deserialization fails.
/// 
/// # Security
/// 
/// Incoming packet sizes are validated before allocation to avoid 
/// excessive memory usage from malformed or malicious packets.
pub fn recv_message<T: serde::de::DeserializeOwned>(stream: &mut TcpStream) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("message too large: {} bytes", len),
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    bincode::deserialize(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

/*
/// Sends a serialized [`GameState`] packet over a TCP stream.
///
/// Deprecated in favor of the generic [`send_message`] helper.
pub fn send_state(stream: &mut TcpStream, state: &GameState) -> std::io::Result<()> {
    let encoded =
        bincode::serialize(state).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

/// Receives a serialized [`GameState`] packet from a TCP stream.
///
/// Deprecated in favor of the generic [`recv_message`] helper.
pub fn recv_state(stream: &mut TcpStream) -> std::io::Result<GameState> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "state packet too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;

    bincode::deserialize(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}
*/