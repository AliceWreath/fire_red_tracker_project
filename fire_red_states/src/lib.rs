use std::net::TcpStream;
use std::io::{Read, Write};

/// Maximum allowed network message size.
///
/// Used as a safeguard against malformed or malicious packets that could
/// otherwise allocate excessive memory.
///
/// Current limit: 20 MB.
const MAX_MESSAGE_SIZE: usize = 20 * 1024 * 1024; // 20 MB

/// The highest valid National Pokédex number in FireRed (Generation III cap).
///
/// Used to filter out placeholder or sentinel species values that appear in
/// ROM tables and EWRAM slots but do not correspond to real Pokémon.
pub const MAX_NATIONAL_DEX_FIRERED: u16 = 386;

/// Messages sent from a client to the server.
///
/// # IMPORTANT — bincode variant ordering
/// bincode encodes enum variants by their **positional index** (0, 1, 2, …).
/// Inserting a new variant anywhere other than the end silently breaks
/// deserialization between old and new binaries.  New variants MUST be
/// appended at the end only.  Current stable indices:
///   0 = RequestTextures
///   1 = EndRun
///   2 = NewRun
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub enum ClientMessage {
    RequestTextures(Vec<u16>), // index 0 — do not reorder
    EndRun,                    // index 1 — do not reorder
    NewRun,                    // index 2 — do not reorder
    // Append new variants here only.
}

/// Messages sent from the server to connected clients.
///
/// # IMPORTANT — bincode variant ordering
/// Same constraint as [`ClientMessage`].  Current stable indices:
///   0 = State
///   1 = Textures
///   2 = RunChanged
///   3 = BoxData
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ServerMessage {
    State(Box<GameState>),      // index 0 — do not reorder
    Textures(Vec<SpriteData>),  // index 1 — do not reorder
    RunChanged(Option<u32>),    // index 2 — do not reorder
    BoxData(Vec<BoxEntry>),     // index 3 — do not reorder
    // Append new variants here only.
}

/// A compact snapshot of one PC box slot for network transmission.
///
/// Built by the tracker from the live EWRAM snapshot and sent to the aggregator
/// every ~5 seconds so the web overlay can display the full box contents.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct BoxEntry {
    /// Zero-based index of the PC box (0–13).
    pub box_index:    u8,
    /// Zero-based slot within the box (0–29).
    pub slot_index:   u8,
    pub species:      u16,
    pub species_name: String,
    pub nickname:     String,
    pub personality:  u32,
    pub ot_id:        u32,
    pub is_shiny:     bool,
    pub nature:       String,
    pub iv_hp:        u8,
    pub iv_atk:       u8,
    pub iv_def:       u8,
    pub iv_spe:       u8,
    pub iv_spa:       u8,
    pub iv_spd:       u8,
    pub is_egg:       bool,
    /// `0` = male, `1` = female, `2` = genderless.
    pub gender:       u8,
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

    /// Human-readable name for the current wild-encounter zone, resolved by the
    /// tracker from the ROM's `gMapGroupsAndMaps` table. Empty when the current
    /// map has no wild encounters.
    pub zone_name: String,

    /// Actual player map-group ID read directly from EWRAM (0x02031DBC[0]).
    /// This is the true map position and is independent of the encounter header,
    /// so it must be used — not `encounters.map_group` — to key zone transitions.
    pub current_map_group: u8,

    /// Actual player map-name ID read directly from EWRAM (0x02031DBC[1]).
    pub current_map_name: u8,

    /// Preferred display slot index (1 = first column, 2 = second, …).
    /// `None` means no preference; the aggregator places those slots last,
    /// then breaks ties alphabetically by player name.
    pub preferred_player: Option<u8>,
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
/// ```text
/// [4-byte big-endian length][bincode-encoded message bytes]
/// ```
pub fn send_message<T: serde::Serialize>(stream: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let encoded =
        bincode::serialize(msg).map_err(std::io::Error::other)?;
    let len = u32::try_from(encoded.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "message too large"))?;
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
    bincode::deserialize(&buf).map_err(std::io::Error::other)
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