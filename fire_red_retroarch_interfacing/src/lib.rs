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
//! The default port is 55355.
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

use std::cell::RefCell;
use std::net::UdpSocket;

// Per-thread RetroArch UDP address. Threads that never call set_thread_addr
// default to "127.0.0.1:55355" (same machine, tracker case).
thread_local! {
    static RETROARCH_ADDR: RefCell<String> = RefCell::new("127.0.0.1:55355".to_string());
}

/// Set the RetroArch address for the **current thread only**.
///
/// Call this at the very start of each game-polling thread, before any socket
/// operation. Multiple threads (one per RetroArch host) each call this with
/// their own host, so they never share or overwrite each other's address.
///
/// # Arguments
/// * `host` — hostname or IP address of the machine running RetroArch.
/// * `port` — RetroArch network commands port (default 55355).
pub fn set_thread_addr(host: &str, port: u16) {
    RETROARCH_ADDR.with(|a| *a.borrow_mut() = format!("{}:{}", host, port));
}

/// Returns a clone of the current thread's RetroArch address string.
///
/// Capture this before spawning a child thread, then pass the value to
/// [`set_thread_addr_string`] at the start of the child thread so the child
/// inherits the parent's address (thread-locals are not inherited automatically).
pub fn get_thread_addr_string() -> String {
    RETROARCH_ADDR.with(|a| a.borrow().clone())
}

/// Set the RetroArch address from a pre-formatted `"host:port"` string.
///
/// Use in spawned threads together with [`get_thread_addr_string`] to propagate
/// the parent thread's address.
pub fn set_thread_addr_string(addr: String) {
    RETROARCH_ADDR.with(|a| *a.borrow_mut() = addr);
}

/// Calls `f` with a borrow of the current thread's RetroArch address string.
fn with_addr<R>(f: impl FnOnce(&str) -> R) -> R {
    RETROARCH_ADDR.with(|a| f(a.borrow().as_str()))
}

/// Returns true if the current thread's RetroArch target is on the local machine.
fn is_local() -> bool {
    with_addr(|addr| {
        addr.starts_with("127.") || addr.starts_with("[::1]") || addr.starts_with("localhost")
    })
}

/// GBA memory address holding the current map group and map number.
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

/// Generates a RetroArch `WRITE_CORE_MEMORY` command string.
///
/// # Arguments
///
/// * `ptr`   - GBA memory address to write to.
/// * `bytes` - Bytes to write; each is formatted as an uppercase two-hex-digit
///   token separated by spaces.
///
/// # Example
///
/// ```
/// use fire_red_retroarch_interfacing::generate_write_command;
/// let cmd = generate_write_command(0x02001234, &[0x0D, 0x00, 0x01, 0x00]);
/// assert_eq!(cmd, "WRITE_CORE_MEMORY 0x02001234 0D 00 01 00");
/// ```
pub fn generate_write_command(ptr: u32, bytes: &[u8]) -> String {
    let hex: Vec<String> = bytes.iter().map(|b| format!("{:02X}", b)).collect();
    format!("WRITE_CORE_MEMORY 0x{:08X} {}", ptr, hex.join(" "))
}

/// Sends a `WRITE_CORE_MEMORY` command to RetroArch and waits for its
/// acknowledgement.
///
/// RetroArch responds with `WRITE_CORE_MEMORY <addr> <bytes_written>` on
/// success. The acknowledgement is read and discarded; the caller does not need
/// to inspect it.
///
/// # Arguments
///
/// * `socket` - A bound UDP socket. Create one with [`make_socket`].
/// * `addr`   - GBA memory address to write to.
/// * `bytes`  - Bytes to write.
///
/// # Returns
///
/// `true` if the command was sent (and the ack drained without a hard error).
/// `false` on send failure.
pub fn write_to_retroarch(socket: &UdpSocket, addr: u32, bytes: &[u8]) -> bool {
    let command = generate_write_command(addr, bytes);
    if let Err(e) = with_addr(|ra| socket.send_to(command.as_bytes(), ra)) {
        tracing::warn!("Failed to send WRITE_CORE_MEMORY to RetroArch: {}", e);
        return false;
    }
    // Drain the ack to keep the socket receive buffer clean. Errors are ignored —
    // a timeout just means RetroArch didn't reply, which doesn't indicate write
    // failure on the emulator side.
    let mut buf = [0u8; 64];
    let _ = socket.recv_from(&mut buf);
    true
}

/// Creates a new UDP socket for RetroArch communication.
///
/// Binds to `0.0.0.0:0` when the configured RetroArch host is a remote
/// machine, or `127.0.0.1:0` when it is localhost, so the OS assigns an
/// ephemeral port on the correct interface. Read timeout is 500 ms.
///
/// One socket per thread is required: UDP is connectionless so sharing a
/// socket between threads causes responses to land on the wrong receiver.
///
/// # Errors
///
/// Returns an `Err` if the OS cannot bind the socket or set the read timeout.
pub fn make_socket() -> std::io::Result<UdpSocket> {
    let bind_addr = if is_local() { "127.0.0.1:0" } else { "0.0.0.0:0" };
    let socket = UdpSocket::bind(bind_addr)?;
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
    if let Err(e) = with_addr(|ra| socket.send_to(command.as_bytes(), ra)) {
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
        assert_eq!(
            generate_command(0x02024284, 4),
            "READ_CORE_MEMORY 0x02024284 4"
        );
    }

    #[test]
    fn generate_command_zero_address() {
        assert_eq!(generate_command(0, 1), "READ_CORE_MEMORY 0x00000000 1");
    }

    #[test]
    fn generate_command_max_address() {
        assert_eq!(
            generate_command(0xFFFFFFFF, 16),
            "READ_CORE_MEMORY 0xFFFFFFFF 16"
        );
    }

    #[test]
    fn generate_command_large_length() {
        assert_eq!(
            generate_command(0x02000000, 4096),
            "READ_CORE_MEMORY 0x02000000 4096"
        );
    }

    // ── make_socket ──────────────────────────────────────────────────────────

    #[test]
    fn make_socket_returns_ok() {
        assert!(make_socket().is_ok());
    }

    #[test]
    fn make_socket_binds_with_ephemeral_port() {
        let sock = make_socket().expect("make_socket should succeed");
        let addr = sock
            .local_addr()
            .expect("socket should have a local address");
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

    // ── generate_write_command ───────────────────────────────────────────────

    #[test]
    fn generate_write_command_single_byte() {
        assert_eq!(
            generate_write_command(0x02001234, &[0xFF]),
            "WRITE_CORE_MEMORY 0x02001234 FF"
        );
    }

    #[test]
    fn generate_write_command_multiple_bytes() {
        assert_eq!(
            generate_write_command(0x02001234, &[0x0D, 0x00, 0x01, 0x00]),
            "WRITE_CORE_MEMORY 0x02001234 0D 00 01 00"
        );
    }

    #[test]
    fn generate_write_command_zero_address() {
        assert_eq!(
            generate_write_command(0x00000000, &[0xAB]),
            "WRITE_CORE_MEMORY 0x00000000 AB"
        );
    }
}
