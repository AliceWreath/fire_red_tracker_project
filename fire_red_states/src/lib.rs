use std::net::TcpStream;
use std::io::{Read, Write};

const MAX_MESSAGE_SIZE: usize = 20 * 1024 * 1024; // 20 MB

#[derive(serde::Serialize, serde::Deserialize)]
pub enum ClientMessage {
    RequestTextures(Vec<u16>),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum ServerMessage {
    State(GameState),
    Textures(Vec<SpriteData>),
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SpriteData {
    pub species: u16,
    pub shiny: bool,
    pub pixels: Vec<u8>, // zlib-compressed RGBA bytes
    pub width: u32,
    pub height: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct GameState {
    pub party: Vec<fire_red_party_monitor::Pokemon>,
    pub encounters: fire_red_pokemon_data::WildPokemonHeader,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum Mode {
    Standalone,
    Server {
        port: u16,
    },
    Client {
        host: String,
        port: u16,
    },
}

// ---------------------------------------------------------------------------
// Wire helpers — length-prefixed bincode frames
// ---------------------------------------------------------------------------

pub fn send_message<T: serde::Serialize>(stream: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let encoded =
        bincode::serialize(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

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
pub fn send_state(stream: &mut TcpStream, state: &GameState) -> std::io::Result<()> {
    let encoded =
        bincode::serialize(state).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

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