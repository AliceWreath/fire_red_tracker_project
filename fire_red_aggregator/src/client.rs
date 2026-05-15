use fire_red_states::{GameState, recv_state};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct MonitorSlot {
    pub label: String,
    pub addr: String,
    pub state: Arc<Mutex<Option<GameState>>>,
}

impl MonitorSlot {
    pub fn new(index: usize, addr: String) -> Self {
        Self {
            label: format!("Player {} ({})", index + 1, addr),
            addr,
            state: Arc::new(Mutex::new(None)),
        }
    }
}

pub fn spawn_client(addr: String, state: Arc<Mutex<Option<GameState>>>) {
    std::thread::spawn(move || loop {
        println!("Connecting to monitor at {}...", addr);
        match TcpStream::connect(&addr) {
            Ok(mut stream) => {
                println!("Connected to {}", addr);
                loop {
                    match recv_state(&mut stream) {
                        Ok(gs) => {
                            *state.lock().unwrap_or_else(|e| e.into_inner()) = Some(gs);
                        }
                        Err(e) => {
                            eprintln!("Lost connection to {}: {}", addr, e);
                            *state.lock().unwrap_or_else(|e| e.into_inner()) = None;
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to {}: {}", addr, e);
                *state.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    });
}