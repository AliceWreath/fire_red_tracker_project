//! RetroArch UDP network command interface.
//!
//! Provides low-level communication with a running RetroArch instance via its
//! UDP network command protocol. Supports reading arbitrary ranges of emulated
//! GBA memory by address and length.
//!
//! # Setup
//!
//! RetroArch must have "Network Commands" enabled:
//! Settings → Network → Network Commands → ON
//! The default port is 55355, which matches [`RETROARCH_ADDR`].
//!
//! # Protocol
//!
//! Commands are sent as ASCII strings over UDP. RetroArch responds with a
//! space-separated string of the form:
//! ```text
//! READ_CORE_MEMORY <addr> <byte0> <byte1> ...
//! ```
//! where each byte is a two-character uppercase hex value.
//!
//! # Important: chunk size limit
//!
//! RetroArch silently drops responses that exceed its internal send buffer.
//! In practice this caps useful reads at around 4,096 bytes of GBA memory per
//! request (~12 KB of ASCII response). Callers should chunk larger reads; see
//! the `fire_red_memory` crate for an example.
//!
//! # Thread safety
//!
//! This crate does not use a shared global socket. Instead, [`make_socket`]
//! creates a fresh bound UDP socket each time it is called. Callers that
//! perform concurrent reads should call [`make_socket`] once per thread and
//! pass the resulting socket to [`get_from_retroarch`]. This avoids the
//! response-stealing problem that arises when multiple threads share a single
//! UDP socket.

use std::net::UdpSocket;

/// RetroArch UDP network command interface address.
///
/// Must match the port configured in RetroArch under
/// Settings → Network → Network Commands Port.
static RETROARCH_ADDR: &str = "127.0.0.1:55355";

/// GBA memory address holding the current map group and map number.
///
/// The value is a packed `u32`: high byte = map group, low byte = map number.
/// Used to determine the player's current location in FireRed.
static MAP_GROUP_AND_NAME_ADDR: u32 = 0x02031DBC;

/// UDP receive buffer size in bytes.
///
/// Sized to comfortably exceed the largest RetroArch response we expect.
/// At 4,096 bytes of data per chunk, each byte is returned as "XX " (3 chars)
/// plus a ~30-byte header, giving a maximum response of roughly 12,318 bytes.
/// 16 KiB provides comfortable headroom.
const BUFFER_SIZE: usize = 16_384;

/// Generates a RetroArch `READ_CORE_MEMORY` command string.
///
/// # Arguments
///
/// * `ptr` - GBA memory address to read from.
/// * `len` - Number of bytes to read. Must not exceed ~4,096 or RetroArch
///   will silently drop the response without returning an error.
///
/// # Returns
///
/// A formatted command string ready to be sent to [`get_from_retroarch`].
///
/// # Example
///
/// ```
/// use fire_red_retroarch_interfacing::generate_command;
/// let cmd = generate_command(0x02024284, 4);
/// assert_eq!(cmd, "READ_CORE_MEMORY 0x02024284 4");
/// ```
pub fn generate_command(ptr: u32, len: usize) -> String {
    format!("READ_CORE_MEMORY 0x{:08X} {}", ptr, len)
}

/// Creates a new UDP socket bound to the loopback interface for RetroArch
/// communication.
///
/// Each socket is bound to `127.0.0.1:0` (OS-assigned ephemeral port) with a
/// 500 ms read timeout.
///
/// # Why one socket per thread?
///
/// UDP is connectionless — when two threads share one socket and both call
/// `recv_from`, either thread can receive the other's response. By giving each
/// thread its own socket on its own port, RetroArch's reply is guaranteed to
/// arrive at the correct socket.
///
/// # Why not `connect()`?
///
/// Calling `connect()` on a UDP socket in Linux causes the kernel to filter
/// incoming datagrams to only those from the connected address. In testing,
/// this caused all `recv` calls to time out even though RetroArch was replying
/// correctly. Using `send_to`/`recv_from` on an unconnected socket avoids
/// this issue.
///
/// # Errors
///
/// Returns an `Err` if the OS cannot bind the socket or set the read timeout.
pub fn make_socket() -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
    Ok(socket)
}

/// Retrieves the current map group and map number from RetroArch.
///
/// Reads a `u32` from [`MAP_GROUP_AND_NAME_ADDR`] in GBA memory via the
/// UDP network command interface.
///
/// Creates its own socket internally. If you are making many calls in a loop,
/// prefer creating a socket with [`make_socket`] and calling
/// [`get_from_retroarch`] directly to avoid the overhead of binding a new
/// socket on every call.
///
/// # Returns
///
/// - `Some(Vec<String>)` — whitespace-separated response tokens. Index 0 is
///   `"READ_CORE_MEMORY"`, index 1 is the address, indices 2+ are byte values
///   as uppercase hex strings.
/// - `None` if communication fails or the response is malformed.
pub fn get_map_info() -> Option<Vec<String>> {
    let socket = make_socket().ok()?;
    let command = generate_command(MAP_GROUP_AND_NAME_ADDR, std::mem::size_of::<u32>());
    // +2 accounts for the "READ_CORE_MEMORY <addr>" prefix tokens in the response.
    get_from_retroarch(&socket, command.as_str(), std::mem::size_of::<u32>() + 2)
}

/// Sends a command to RetroArch over UDP and waits for a response.
///
/// # Arguments
///
/// * `socket` - A bound UDP socket to use for this request. Create one with
///   [`make_socket`]. Each concurrent caller must use its own socket.
/// * `command` - The command string to send (e.g. from [`generate_command`]).
/// * `expected_token_count` - Minimum number of whitespace-separated tokens
///   expected in the response. Responses with fewer tokens are treated as
///   malformed and `None` is returned.
///
/// # Returns
///
/// - `Some(Vec<String>)` — the full parsed response as whitespace-separated
///   tokens, including the `READ_CORE_MEMORY` and address prefix tokens.
/// - `None` if the send fails, the socket times out, or the response has
///   fewer than `expected_token_count` tokens.
///
/// # Errors
///
/// All errors are logged via `tracing`. The caller is expected to handle `None`
/// as a retryable failure — no panics occur here.
pub fn get_from_retroarch(
    socket: &UdpSocket,
    command: &str,
    expected_token_count: usize,
) -> Option<Vec<String>> {
    if let Err(e) = socket.send_to(command.as_bytes(), RETROARCH_ADDR) {
        tracing::warn!("Failed to send command to RetroArch: {}", e);
        return None;
    }

    let mut buf = vec![0u8; BUFFER_SIZE];
    match socket.recv_from(&mut buf) {
        Ok((n, _src)) => {
            let parts: Vec<String> = String::from_utf8_lossy(&buf[..n])
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();

            if parts.len() < expected_token_count {
                return None;
            }
            Some(parts)
        }
        Err(e)
            if e.kind() == std::io::ErrorKind::TimedOut
                || e.kind() == std::io::ErrorKind::WouldBlock =>
        {
            None
        }
        Err(e) => {
            tracing::warn!("Unexpected socket error: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── generate_command ─────────────────────────────────────────────────────

    #[test]
    fn generate_command_formats_address() {
        assert_eq!(generate_command(0x02024284, 4), "READ_CORE_MEMORY 0x02024284 4");
    }

    #[test]
    fn generate_command_zero_address() {
        assert_eq!(generate_command(0, 1), "READ_CORE_MEMORY 0x00000000 1");
    }

    #[test]
    fn generate_command_max_address() {
        assert_eq!(generate_command(0xFFFFFFFF, 16), "READ_CORE_MEMORY 0xFFFFFFFF 16");
    }

    #[test]
    fn generate_command_large_length() {
        assert_eq!(generate_command(0x02000000, 4096), "READ_CORE_MEMORY 0x02000000 4096");
    }

    // ── make_socket ──────────────────────────────────────────────────────────

    #[test]
    fn make_socket_returns_ok() {
        assert!(make_socket().is_ok());
    }

    #[test]
    fn make_socket_binds_to_loopback_with_ephemeral_port() {
        let sock = make_socket().expect("make_socket should succeed");
        let addr = sock.local_addr().expect("socket should have a local address");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_ne!(addr.port(), 0);
    }

    #[test]
    fn make_socket_two_calls_get_different_ports() {
        let s1 = make_socket().expect("first make_socket should succeed");
        let s2 = make_socket().expect("second make_socket should succeed");
        // OS assigns different ephemeral ports each time.
        assert_ne!(
            s1.local_addr().expect("s1 local_addr").port(),
            s2.local_addr().expect("s2 local_addr").port()
        );
    }
}
