use std::net::UdpSocket;
use std::sync::{Arc, OnceLock};

static UDP_SOCKET: OnceLock<Arc<UdpSocket>> = OnceLock::new();
static RETROARCH_ADDR: &str = "127.0.0.1:55355";
static MAP_GROUP_AND_NAME_ADDR: u32 = 0x02031DBC;
const BUFFER_SIZE: usize = 40000;

pub fn generate_command(ptr: u32, len: usize) -> String {
    format!("READ_CORE_MEMORY 0x{:08X} {}", ptr, len)
}

pub fn get_socket() -> Arc<UdpSocket> {
    UDP_SOCKET 
        .get_or_init(|| {
            let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind udpsocket.");
            socket.set_read_timeout(Some(std::time::Duration::from_millis(500))).ok();
            Arc::new(socket)
        })
        .clone()
}


pub fn get_map_info() -> Option<Vec<String>> {
    let command = generate_command(MAP_GROUP_AND_NAME_ADDR, std::mem::size_of::<u32>());
    get_from_retroarch(command.as_str(), std::mem::size_of::<u32>() + 2) // +2 for the READ_CORE_MEOMORY prefix
}

pub fn get_from_retroarch(command: &str, expected_len_data: usize) -> Option<Vec<String>> {
    let socket = get_socket();
    let _ = socket.send_to(&command.as_bytes(), RETROARCH_ADDR);
    let _ = std::thread::sleep(std::time::Duration::from_millis(50));
    let mut buf = [0u8; BUFFER_SIZE];

    match socket.recv_from(&mut buf) {
        Ok((n, src)) => {
            let resp = String::from_utf8_lossy(&buf[..n]);
            let parts: Vec<&str> = resp.split_whitespace().collect();
    
            let parts: Vec<String> = parts.iter().map(|s| s.to_string()).collect();
            if parts.len() < expected_len_data {
                return None;
            }
            return Some(parts);
        }
        Err(e)  if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {
            eprintln!("Timeout while waiting for response from RetroArch: {}", e);
            return None;
        }
        Err(e) => {
            eprintln!("Unexpected error while receiving from RetroArch: {}", e);
            return None;
        }
    }
}