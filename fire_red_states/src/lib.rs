use std::net::TcpStream;
use std::io::{Read, Write};

const MAX_STATE_SIZE: usize = 10 * 1024 * 1024; // 10 MB, should be enough for party + encounters, adjust as needed

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
        rom_path: String,
        host: String,
        port: u16,
    },
}

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
    if len > MAX_STATE_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "state packet too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;

    bincode::deserialize(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}