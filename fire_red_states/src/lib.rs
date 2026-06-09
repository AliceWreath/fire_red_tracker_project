use std::net::TcpStream;
use std::io::{Read, Write};

/// Maximum allowed network message size: 20 MB.
///
/// Guards against malformed or malicious packets that would otherwise cause
/// runaway heap allocation.  The 4-byte length prefix could theoretically
/// claim up to ~4 GB; this constant is the application-layer sanity cap.
///
/// **Why 20 MB?**  The largest legitimate `ServerMessage` variant is
/// `Textures(Vec<SpriteData>)`.  In the worst case the tracker sends all 386
/// species × 2 (normal + shiny) as zlib-compressed 64×64 RGBA sprites; even
/// at a conservative 2 KB per sprite that is under 1.6 MB.  20 MB leaves a
/// comfortable margin for future sprite-count growth or higher-resolution
/// assets without admitting absurdly large allocations from a buggy or
/// hostile peer.
const MAX_MESSAGE_SIZE: usize = 20 * 1024 * 1024;

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
///   3 = Hello
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub enum ClientMessage {
    RequestTextures(Vec<u16>), // index 0 — do not reorder
    EndRun,                    // index 1 — do not reorder
    NewRun,                    // index 2 — do not reorder
    Hello(String),             // index 3 — do not reorder
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

/// Which sprite image a [`SpriteData`] packet carries.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum SpriteVariant {
    /// Standard front-facing battle sprite (default).
    #[default]
    Front,
    /// Rear-facing sprite used on the player's side of battle.
    Back,
}

/// Serialized Pokemon sprite texture data for network transmission.
///
/// Pixel data is stored as zlib-compressed RGBA bytes.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SpriteData {
    pub species: u16,
    pub shiny: bool,
    /// Which image this packet carries; defaults to [`SpriteVariant::Front`] when
    /// deserializing packets from older server versions that lack the field.
    #[serde(default)]
    pub variant: SpriteVariant,
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

/// Serializes and sends a message over a TCP stream.
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
/// * `T` - Message type implementing [`serde::de::DeserializeOwned`].
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

// ---------------------------------------------------------------------------
// Mutex poison recovery
// ---------------------------------------------------------------------------

/// Extension trait for [`std::sync::Mutex`] that recovers from poison instead
/// of propagating it. A poisoned mutex means a thread panicked while holding
/// the lock; for this tracker's display-only state, stale data is safer than
/// crashing.
pub trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for std::sync::Mutex<T> {
    #[track_caller]
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            let loc = std::panic::Location::caller();
            tracing::warn!("mutex poisoned at {}:{}: {e}", loc.file(), loc.line());
            e.into_inner()
        })
    }
}

// ---------------------------------------------------------------------------
// Base64 encoding
// ---------------------------------------------------------------------------

/// Encodes `data` as standard Base64 (RFC 4648 alphabet, `=` padding).
///
/// Hand-rolled to avoid adding a dependency for a single, tight use-site.
/// Used by the OBS WebSocket auth flow in the tracker and by the web overlay
/// sprite pipeline in the aggregator — kept here so both crates share one copy.
pub fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n  = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >>  6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[( n        & 63) as usize] as char } else { '=' });
    }
    out
}

// ---------------------------------------------------------------------------
// GBA value helpers
// ---------------------------------------------------------------------------

/// Returns `true` if the Pokémon with `personality` and `ot_id` is shiny.
///
/// Gen III formula: `(p_high ^ p_low ^ id_high ^ id_low) < 8`.
pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p_high  = (personality >> 16) as u16;
    let p_low   = (personality & 0xFFFF) as u16;
    let id_high = (ot_id >> 16) as u16;
    let id_low  = (ot_id & 0xFFFF) as u16;
    (p_high ^ p_low ^ id_high ^ id_low) < 8
}

#[cfg(test)]
mod base64_tests {
    use super::base64_encode;

    #[test]
    fn empty_input() {
        assert_eq!(base64_encode(&[]), "");
    }

    #[test]
    fn three_bytes_no_padding() {
        // RFC 4648 test vector: "Man" → "TWFu"
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn one_byte_two_padding_chars() {
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn two_bytes_one_padding_char() {
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn output_always_multiple_of_four() {
        for len in 0..=9usize {
            let data: Vec<u8> = (0..len as u8).collect();
            let encoded = base64_encode(&data);
            assert_eq!(encoded.len() % 4, 0, "length {len} gave non-multiple-of-4 output");
        }
    }

    #[test]
    fn hello_world() {
        assert_eq!(base64_encode(b"Hello, World!"), "SGVsbG8sIFdvcmxkIQ==");
    }
}
