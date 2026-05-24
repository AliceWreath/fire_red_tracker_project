use std::net::UdpSocket;
use std::sync::{Arc, OnceLock};

/// Global shared UDP socket used for RetroArch communication.
/// 
/// The socket is initialized lazily on first use and shared safely.
/// across threads through [`Arc`].
static UDP_SOCKET: OnceLock<Arc<UdpSocket>> = OnceLock::new();

/// Default RetroArch UDP command interface address.
/// 
/// Used for sending emulator memory read commands.
static RETROARCH_ADDR: &str = "127.0.0.1:55355";

/// Memory address containing the current map group and map number.
/// 
/// Used to determine the player's current locaiton in FireRed.
static MAP_GROUP_AND_NAME_ADDR: u32 = 0x02031DBC;

/// Maximum UDP receive buffer size.
/// 
/// Large enough to handle RetroArch responses safely.
const BUFFER_SIZE: usize = 40000;

/// Generates a RetroArch memory read command string.
/// 
/// # Arguments
/// 
/// * `ptr` - Target GBA memory address
/// * `len` - Number of bytes to read
/// 
/// # Returns
/// 
/// A formatted RetroArch command string.
/// 
/// # Example
/// 
/// ```ignore
/// let cmd = generate_command(0x02024284, 4);
/// assert_eq!(cmd, "READ_CORE_MEMORY 0x02024284 4");
/// ```
pub fn generate_command(ptr: u32, len: usize) -> String {
    format!("READ_CORE_MEMORY 0x{:08X} {}", ptr, len)
}

/// Returns the global shared UDP socket
/// 
/// The socket is lazily intialized the first time this function is called.
/// 
/// # Socket Configuration
/// 
/// - binds `0.0.0.0:0` to allow the OS to assign an available port
/// - Uses a 500ms read timeout to prevent blocking indefinitely when waiting for RetroArch responses.
/// 
/// # Panics
/// 
/// Panics if the UDP socket cannot be created.
pub fn get_socket() -> Arc<UdpSocket> {
    UDP_SOCKET 
        .get_or_init(|| {
            let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind udpsocket.");
            socket.set_read_timeout(Some(std::time::Duration::from_millis(500))).ok();
            Arc::new(socket)
        })
        .clone()
}

/// Retrieves the current map group and map number from RetroArch.
/// 
/// Internally sends a memory read request to the emulator.
/// 
/// # Returns
/// 
/// - `Some(Vec<String>)` containing the parsed response fields
/// - `None` if communication failes or the response is invalid.
/// 
/// # Notes
/// 
/// The returned vector contains whitespace-separated response tokens
/// from RetroArch.
pub fn get_map_info() -> Option<Vec<String>> {
    let command = generate_command(MAP_GROUP_AND_NAME_ADDR, std::mem::size_of::<u32>());
    get_from_retroarch(command.as_str(), std::mem::size_of::<u32>() + 2) // +2 for the READ_CORE_MEOMORY prefix
}

/// Sends a command to RetroArch and waits for a response.
/// 
/// # Arguments
/// 
/// * `command` - The command string.
/// * `expected_len_data` - Minimum expected number of response tokens (used for basic validation).
/// 
/// # Returns
/// 
/// - `Some(Vec<String>)` contianing parsed response tokens
/// - `None` if the request times out, fails, or returns invalid data.
/// 
/// # Errors
/// 
/// Timeout and socket errors are logged to stderr, but do not cause a panic. Instead, `None` is returned to indicate failure.
/// 
/// # Protocol
/// 
/// Commands are sent over UDP to the RetroArch network command interface.
/// The function waits for a response and parses it into whitespace-separated tokens, 
/// which are returned as a vector of strings.
pub fn get_from_retroarch(command: &str, expected_len_data: usize) -> Option<Vec<String>> {
    let socket = get_socket();
    let _ = socket.send_to(&command.as_bytes(), RETROARCH_ADDR);
    let _ = std::thread::sleep(std::time::Duration::from_millis(50));
    let mut buf = [0u8; BUFFER_SIZE];

    match socket.recv_from(&mut buf) {
        Ok((n, _src)) => {
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