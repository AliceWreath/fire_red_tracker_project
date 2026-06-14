//! LiveSplit Server integration.
//!
//! Sends "split\r\n" to a running LiveSplit Server instance over TCP whenever
//! a gym badge is earned or the Champion is defeated. Controlled by config
//! fields `livesplit_host`, `livesplit_port`, `livesplit_split_on_badges`, and
//! `livesplit_split_on_clear`.
//!
//! The module manages a single persistent TCP connection via a background
//! thread and a channel. On disconnect it retries automatically with
//! backoff so the speedrun timer reconnects if LiveSplit restarts mid-run.

use std::io::Write;
use std::net::TcpStream;
use std::sync::OnceLock;
use std::time::Duration;

#[allow(dead_code)]
enum Cmd {
    Split,
    Shutdown,
}

static SENDER: OnceLock<std::sync::Mutex<Option<std::sync::mpsc::Sender<Cmd>>>> =
    OnceLock::new();

/// Initialise the LiveSplit background thread.
///
/// Must be called once at startup. If `host` is `None` the module is a no-op.
pub fn init(host: Option<String>, port: u16) {
    let sender_slot = SENDER.get_or_init(|| std::sync::Mutex::new(None));
    let Some(h) = host else { return };

    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    *sender_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);

    std::thread::spawn(move || {
        let addr = format!("{}:{}", h, port);
        let mut stream: Option<TcpStream> = None;
        let mut backoff = Duration::from_secs(5);

        for cmd in rx {
            match cmd {
                Cmd::Shutdown => break,
                Cmd::Split    => {
                    // Lazily connect / reconnect.
                    if stream.is_none() {
                        match TcpStream::connect(&addr) {
                            Ok(s) => {
                                let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
                                stream = Some(s);
                                backoff = Duration::from_secs(5);
                                tracing::info!("LiveSplit connected: {addr}");
                            }
                            Err(e) => {
                                tracing::warn!("LiveSplit connect failed ({addr}): {e}; retry in {backoff:?}");
                                std::thread::sleep(backoff);
                                backoff = (backoff * 2).min(Duration::from_secs(60));
                                continue;
                            }
                        }
                    }

                    if let Some(ref mut s) = stream {
                        if s.write_all(b"split\r\n").is_err() {
                            tracing::warn!("LiveSplit write failed; will reconnect on next split");
                            stream = None;
                        } else {
                            tracing::info!("LiveSplit: sent split");
                        }
                    }
                }
            }
        }
        tracing::info!("LiveSplit thread shut down.");
    });
}

/// Send a split signal if the module is initialised.
pub fn split() {
    if let Some(slot) = SENDER.get() {
        if let Some(tx) = slot.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            let _ = tx.send(Cmd::Split);
        }
    }
}

/// Shut down the background thread cleanly.
#[allow(dead_code)]
pub fn shutdown() {
    if let Some(slot) = SENDER.get() {
        if let Some(tx) = slot.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            let _ = tx.send(Cmd::Shutdown);
        }
    }
}
