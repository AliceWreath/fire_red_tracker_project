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
            Arc::new(socket)
        })
        .clone()
}


pub fn get_map_info() -> Vec<String> {
    let command = generate_command(MAP_GROUP_AND_NAME_ADDR, std::mem::size_of::<u32>());
    get_from_retroarch(command.as_str(), std::mem::size_of::<u32>() + 2) // +2 for the READ_CORE_MEOMORY prefix
}

pub fn get_from_retroarch(command: &str, expected_len_data: usize) -> Vec<String> {
    let socket = get_socket();
    let _ = socket.send_to(&command.as_bytes(), RETROARCH_ADDR);
    let _ = std::thread::sleep(std::time::Duration::from_millis(50));
    let mut buf = [0u8; BUFFER_SIZE];
    let result = socket.recv_from(&mut buf);

    let (len, _) = result.unwrap();
    let resp = String::from_utf8_lossy(&buf[..len]);
    let parts: Vec<&str> = resp.split_whitespace().collect();
    
    let parts: Vec<String> = parts.iter().map(|s| s.to_string()).collect();
    if parts.len() < expected_len_data {
        return vec![" ".to_string(); expected_len_data];
    }
    parts
}