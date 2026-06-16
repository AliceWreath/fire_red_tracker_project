//! Discord Rich Presence via the local Discord IPC socket.
//!
//! Connects to `/tmp/discord-ipc-N` (N = 0..9), performs the OAuth handshake,
//! and exposes [`update`] to push a new presence payload.  The background
//! thread retries the connection on disconnect so the presence resumes if
//! Discord restarts mid-session.
//!
//! The module is a no-op when `client_id` is `None`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::OnceLock;
use std::time::Duration;

// OP codes for the Discord IPC protocol.
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

pub struct Presence {
    pub details: String,
    pub state: String,
    pub large_image: &'static str,
    pub large_text: String,
}

enum Cmd {
    Update(Presence),
    Shutdown,
}

static SENDER: OnceLock<std::sync::Mutex<Option<std::sync::mpsc::Sender<Cmd>>>> = OnceLock::new();

/// Initialise the Discord RPC background thread.
///
/// Must be called once at startup. If `client_id` is `None` the module is a no-op.
pub fn init(client_id: Option<u64>) {
    let slot = SENDER.get_or_init(|| std::sync::Mutex::new(None));
    let Some(id) = client_id else { return };

    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);

    std::thread::spawn(move || {
        let mut backoff = Duration::from_secs(5);

        'outer: loop {
            // Try each IPC socket slot (Discord can use 0–9).
            let mut stream: Option<UnixStream> = None;
            for n in 0..10u8 {
                let path = format!("/tmp/discord-ipc-{n}");
                if let Ok(s) = UnixStream::connect(&path) {
                    stream = Some(s);
                    break;
                }
            }

            let Some(mut sock) = stream else {
                tracing::debug!("Discord IPC socket not found; retry in {backoff:?}");
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(60));
                continue;
            };
            backoff = Duration::from_secs(5);

            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = sock.set_write_timeout(Some(Duration::from_secs(3)));

            // Handshake.
            let hs = serde_json::json!({ "v": 1, "client_id": id.to_string() });
            if send_packet(&mut sock, OP_HANDSHAKE, &hs).is_err() {
                tracing::warn!("Discord RPC handshake write failed");
                continue;
            }
            // Read (and discard) the READY response.
            if read_packet(&mut sock).is_err() {
                tracing::warn!("Discord RPC handshake read failed");
                continue;
            }
            tracing::info!("Discord RPC connected (client_id={id})");

            for cmd in &rx {
                match cmd {
                    Cmd::Shutdown => break 'outer,
                    Cmd::Update(p) => {
                        let now_secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let payload = serde_json::json!({
                            "cmd": "SET_ACTIVITY",
                            "args": {
                                "pid": std::process::id(),
                                "activity": {
                                    "details":     p.details,
                                    "state":       p.state,
                                    "assets": {
                                        "large_image": p.large_image,
                                        "large_text":  p.large_text,
                                    },
                                    "timestamps": { "start": now_secs },
                                }
                            },
                            "nonce": now_secs.to_string(),
                        });
                        if send_packet(&mut sock, OP_FRAME, &payload).is_err() {
                            tracing::warn!("Discord RPC write failed; reconnecting");
                            break;
                        }
                        // Drain the ACK response.
                        let _ = read_packet(&mut sock);
                    }
                }
            }
        }
        tracing::info!("Discord RPC thread shut down.");
    });
}

/// Push updated Rich Presence to Discord.
///
/// Returns immediately; the background thread handles the socket write.
pub fn update(presence: Presence) {
    if let Some(slot) = SENDER.get()
        && let Some(tx) = slot.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
    {
        let _ = tx.send(Cmd::Update(presence));
    }
}

/// Shut down the background thread cleanly.
#[allow(dead_code)]
pub fn shutdown() {
    if let Some(slot) = SENDER.get()
        && let Some(tx) = slot.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
    {
        let _ = tx.send(Cmd::Shutdown);
    }
}

// ---------------------------------------------------------------------------
// IPC helpers
// ---------------------------------------------------------------------------

fn send_packet(sock: &mut UnixStream, op: u32, body: &serde_json::Value) -> std::io::Result<()> {
    let json = body.to_string();
    let json_bytes = json.as_bytes();
    let len = json_bytes.len() as u32;

    let mut buf = Vec::with_capacity(8 + json_bytes.len());
    buf.extend_from_slice(&op.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(json_bytes);
    sock.write_all(&buf)
}

fn read_packet(sock: &mut UnixStream) -> std::io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 8];
    sock.read_exact(&mut header)?;
    let op = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut body = vec![0u8; len];
    sock.read_exact(&mut body)?;
    // Detect graceful close from Discord side.
    if op == OP_CLOSE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "Discord closed",
        ));
    }
    Ok((op, body))
}
