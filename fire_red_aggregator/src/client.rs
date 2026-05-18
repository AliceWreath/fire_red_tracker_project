use fire_red_states::{GameState, ServerMessage, ClientMessage, send_message, recv_message};
use std::collections::{VecDeque, HashSet};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct PendingTexture {
    pub species: u16,
    pub shiny: bool,
    pub pixels: Vec<u8>, // decompressed RGBA
    pub width: u32,
    pub height: u32,
}

pub struct MonitorSlot {
    pub label: String,
    pub _addr: String,
    pub state: Arc<Mutex<Option<GameState>>>,
    pub pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    pub known_species: Arc<Mutex<HashSet<u16>>>,
    pub texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>>,
}

impl MonitorSlot {
    pub fn new(index: usize, addr: String) -> Self {
        Self {
            label: format!("Player {}", index + 1),
            _addr: addr,
            state: Arc::new(Mutex::new(None)),
            pending_textures: Arc::new(Mutex::new(Vec::new())),
            known_species: Arc::new(Mutex::new(HashSet::new())),
            texture_request_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

fn decompress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).unwrap_or(0);
    out
}

pub fn spawn_client(
    addr: String,
    state: Arc<Mutex<Option<GameState>>>,
    pending_textures: Arc<Mutex<Vec<PendingTexture>>>,
    known_species: Arc<Mutex<HashSet<u16>>>,
    texture_request_queue: Arc<Mutex<VecDeque<Vec<u16>>>>,
) {
    std::thread::spawn(move || loop {
        println!("Connecting to monitor at {}...", addr);
        match TcpStream::connect(&addr) {
            Ok(stream) => {
                println!("Connected to {}", addr);

                let mut write_stream = match stream.try_clone() {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Failed to clone stream: {}", e);
                        break;
                    }
                };
                let mut read_stream = stream;

                let connected = Arc::new(AtomicBool::new(true));
                let connected_writer = connected.clone();
                let writer_queue = texture_request_queue.clone();

                // ── Writer thread: drains texture request queue ──────────────
                let writer = std::thread::spawn(move || {
                    while connected_writer.load(Ordering::SeqCst) {
                        let batch = {
                            let mut q = writer_queue.lock().unwrap_or_else(|e| e.into_inner());
                            let mut all: Vec<u16> = q.drain(..).flatten().collect();
                            all.sort();
                            all.dedup();
                            all
                        };

                        if !batch.is_empty() {
                            if send_message(
                                &mut write_stream,
                                &ClientMessage::RequestTextures(batch),
                            )
                            .is_err()
                            {
                                break;
                            }
                        }

                        std::thread::sleep(Duration::from_millis(50));
                    }
                });

                // ── Reader loop: receives State + Textures ───────────────────
                loop {
                    match recv_message::<ServerMessage>(&mut read_stream) {
                        Ok(ServerMessage::State(gs)) => {
                            *state.lock().unwrap_or_else(|e| e.into_inner()) = Some(gs);
                        }
                        Ok(ServerMessage::Textures(sprites)) => {
                            let mut pending =
                                pending_textures.lock().unwrap_or_else(|e| e.into_inner());
                            let mut known =
                                known_species.lock().unwrap_or_else(|e| e.into_inner());
                            for sprite in sprites {
                                known.insert(sprite.species);
                                pending.push(PendingTexture {
                                    species: sprite.species,
                                    shiny: sprite.shiny,
                                    pixels: decompress_pixels(&sprite.pixels),
                                    width: sprite.width,
                                    height: sprite.height,
                                });
                            }
                        }
                        Err(e) => {
                            eprintln!("Lost connection to {}: {}", addr, e);
                            *state.lock().unwrap_or_else(|e| e.into_inner()) = None;
                            break;
                        }
                    }
                }

                connected.store(false, Ordering::SeqCst);
                let _ = writer.join();
            }
            Err(e) => {
                eprintln!("Failed to connect to {}: {}", addr, e);
                *state.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    });
}
