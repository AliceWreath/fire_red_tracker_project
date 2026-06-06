//! Webhook support — fire-and-forget HTTP POST on game events.
//!
//! Call [`init`] once at startup with the loaded [`WebhookConfig`].
//! Then call [`fire_event`] from anywhere in the tracker; it enqueues
//! the payload and returns immediately. A background thread performs the
//! actual HTTP POST so the game-polling loop is never blocked.
//!
//! # Payload format
//!
//! Every POST is `application/json`:
//! ```json
//! {
//!   "event":     "death",
//!   "player":    "Alice",
//!   "timestamp": 1748989234,
//!   "pokemon": {
//!     "nickname": "Bulbasaur",
//!     "species":  "Bulbasaur",
//!     "level":    14,
//!     "shiny":    false,
//!     "nature":   "Jolly"
//!   }
//! }
//! ```
//! The `pokemon` field is absent for `wipe` events.

use crate::config::WebhookConfig;
use serde::Serialize;
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
pub struct PokemonInfo {
    pub nickname: String,
    pub species:  String,
    pub level:    u8,
    pub shiny:    bool,
    pub nature:   String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WebhookEvent {
    Death { player: String, timestamp: u64, pokemon: PokemonInfo },
    Catch { player: String, timestamp: u64, pokemon: PokemonInfo },
    Shiny { player: String, timestamp: u64, pokemon: PokemonInfo },
    Wipe  { player: String, timestamp: u64 },
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct WebhookState {
    tx:     Sender<(String, WebhookEvent)>,
    config: WebhookConfig,
}

static STATE: OnceLock<WebhookState> = OnceLock::new();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the webhook sender. Must be called once before any [`fire_event`]
/// calls; subsequent calls are a no-op (the first config wins).
pub fn init(config: WebhookConfig) {
    let (tx, rx) = channel::<(String, WebhookEvent)>();
    if STATE.set(WebhookState { tx, config }).is_err() {
        return; // already initialized
    }
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        for (url, event) in rx {
            if let Err(e) = client.post(&url).json(&event).send() {
                eprintln!("Webhook POST to {url} failed: {e}");
            }
        }
    });
}

/// Enqueue a webhook event for delivery. Returns immediately; the HTTP POST
/// is performed by the background thread started in [`init`].
///
/// Does nothing if no URL is configured for the event type, or if [`init`]
/// was never called.
pub fn fire_event(event: WebhookEvent) {
    let Some(state) = STATE.get() else { return; };
    let url = match &event {
        WebhookEvent::Death { .. } => state.config.death_url.as_deref(),
        WebhookEvent::Catch { .. } => state.config.catch_url.as_deref(),
        WebhookEvent::Shiny { .. } => state.config.shiny_url.as_deref(),
        WebhookEvent::Wipe  { .. } => state.config.wipe_url.as_deref(),
    };
    if let Some(url) = url {
        let _ = state.tx.send((url.to_string(), event));
    }
}
