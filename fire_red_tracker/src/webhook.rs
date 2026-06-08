//! Webhook support — fire-and-forget HTTP POST on game events.
//!
//! Call [`init`] once at startup with the loaded [`WebhookConfig`].
//! Then call [`fire_event`] from anywhere in the tracker; it enqueues
//! the payload and returns immediately. A background thread performs the
//! actual HTTP POST so the game-polling loop is never blocked.
//!
//! # Default payload format
//!
//! When no template is configured, every POST is `application/json`:
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
//!
//! # Template format
//!
//! When a `*_template` is configured for an event, that string is rendered
//! and POSTed verbatim (`application/json`). Supported placeholders:
//!
//! | Placeholder           | Value                                  |
//! |-----------------------|----------------------------------------|
//! | `{event}`             | `death`, `catch`, `shiny`, or `wipe`  |
//! | `{player}`            | Player name from config                |
//! | `{timestamp}`         | Unix seconds                           |
//! | `{pokemon.nickname}`  | Pokémon nickname (empty on wipe)       |
//! | `{pokemon.species}`   | Pokémon species name (empty on wipe)   |
//! | `{pokemon.level}`     | Level as integer string (empty on wipe)|
//! | `{pokemon.shiny}`     | `true` or `false` (empty on wipe)     |
//! | `{pokemon.nature}`    | Nature name (empty on wipe)            |
//!
//! Use `{{` and `}}` to emit a literal `{` or `}` in the output.
//!
//! Discord example:
//! ```text
//! {"content": "🎮 **{player}** just lost **{pokemon.nickname}** (Lv.{pokemon.level})!"}
//! ```

use crate::config::{ObsConfig, WebhookConfig};
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
// Internal types
// ---------------------------------------------------------------------------

enum PostBody {
    /// Serialize the event to JSON using serde (existing behaviour).
    Json(WebhookEvent),
    /// Already-rendered string; POST verbatim as application/json.
    Raw(String),
}

enum WorkerTask {
    Webhook { url: String, body: PostBody },
    ObsClip,
}

struct WebhookState {
    tx:         Sender<WorkerTask>,
    config:     WebhookConfig,
    obs_config: ObsConfig,
}

static STATE: OnceLock<WebhookState> = OnceLock::new();

// ---------------------------------------------------------------------------
// Template rendering
// ---------------------------------------------------------------------------

fn render_template(template: &str, event: &WebhookEvent) -> String {
    let (event_name, player, timestamp, pokemon) = match event {
        WebhookEvent::Death { player, timestamp, pokemon } => ("death", player.as_str(), *timestamp, Some(pokemon)),
        WebhookEvent::Catch { player, timestamp, pokemon } => ("catch", player.as_str(), *timestamp, Some(pokemon)),
        WebhookEvent::Shiny { player, timestamp, pokemon } => ("shiny", player.as_str(), *timestamp, Some(pokemon)),
        WebhookEvent::Wipe  { player, timestamp }          => ("wipe",  player.as_str(), *timestamp, None),
    };
    let ts = timestamp.to_string();
    // Allocate these only when there is a pokemon, so wipe events pay nothing.
    let level_buf;
    let shiny_buf;
    let (nickname, species, level, shiny, nature): (&str, &str, &str, &str, &str) = if let Some(p) = pokemon {
        level_buf = p.level.to_string();
        shiny_buf = p.shiny.to_string();
        (&p.nickname, &p.species, &level_buf, &shiny_buf, &p.nature)
    } else {
        ("", "", "", "", "")
    };

    let placeholders: &[(&str, &str)] = &[
        ("{event}",            event_name),
        ("{player}",           player),
        ("{timestamp}",        &ts),
        ("{pokemon.nickname}", nickname),
        ("{pokemon.species}",  species),
        ("{pokemon.level}",    level),
        ("{pokemon.shiny}",    shiny),
        ("{pokemon.nature}",   nature),
    ];

    // Single-pass scan: {{ → {, }} → }, known placeholders → value, else copy.
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("{{") {
            out.push('{');
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("}}") {
            out.push('}');
            rest = r;
            continue;
        }
        if rest.starts_with('{') {
            let mut matched = false;
            for &(ph, val) in placeholders {
                if let Some(r) = rest.strip_prefix(ph) {
                    out.push_str(val);
                    rest = r;
                    matched = true;
                    break;
                }
            }
            if !matched {
                let c = rest.chars().next().unwrap();
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        } else {
            let end = rest.find('{').unwrap_or(rest.len());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the webhook sender. Must be called once before any [`fire_event`]
/// calls; subsequent calls are a no-op (the first config wins).
pub fn init(config: WebhookConfig, obs_config: ObsConfig) {
    let (tx, rx) = channel::<WorkerTask>();
    if STATE.set(WebhookState { tx, config, obs_config }).is_err() {
        return; // already initialized
    }
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        for task in rx {
            match task {
                WorkerTask::Webhook { url, body } => {
                    let result = match body {
                        PostBody::Json(event) => client.post(&url).json(&event).send(),
                        PostBody::Raw(text)   => client.post(&url)
                            .header("content-type", "application/json")
                            .body(text)
                            .send(),
                    };
                    if let Err(e) = result {
                        eprintln!("Webhook POST to {url} failed: {e}");
                    }
                }
                WorkerTask::ObsClip => obs_clip_inner(),
            }
        }
    });
}

/// Enqueue a webhook event for delivery. Returns immediately; the HTTP POST
/// is performed by the background thread started in [`init`].
///
/// Also triggers an OBS replay-buffer clip if configured for the event type.
///
/// Does nothing if [`init`] was never called.
pub fn fire_event(event: WebhookEvent) {
    let Some(state) = STATE.get() else { return; };

    let (url, template, obs_clip) = match &event {
        WebhookEvent::Death { .. } => (
            state.config.death_url.as_deref(),
            state.config.death_template.as_deref(),
            state.obs_config.clip_on_death,
        ),
        WebhookEvent::Catch { .. } => (
            state.config.catch_url.as_deref(),
            state.config.catch_template.as_deref(),
            false,
        ),
        WebhookEvent::Shiny { .. } => (
            state.config.shiny_url.as_deref(),
            state.config.shiny_template.as_deref(),
            state.obs_config.clip_on_shiny,
        ),
        WebhookEvent::Wipe { .. } => (
            state.config.wipe_url.as_deref(),
            state.config.wipe_template.as_deref(),
            state.obs_config.clip_on_wipe,
        ),
    };

    if let Some(url) = url {
        let body = match template {
            Some(t) => PostBody::Raw(render_template(t, &event)),
            None    => PostBody::Json(event),
        };
        let _ = state.tx.send(WorkerTask::Webhook { url: url.to_string(), body });
    }

    if obs_clip {
        let _ = state.tx.send(WorkerTask::ObsClip);
    }
}

// ---------------------------------------------------------------------------
// OBS WebSocket clip trigger (v5 protocol, local TCP, no TLS)
// ---------------------------------------------------------------------------

fn obs_clip_inner() {
    if let Err(e) = try_obs_clip() {
        eprintln!("OBS clip failed: {e}");
    }
}

fn try_obs_clip() -> Result<(), String> {
    let state = STATE.get().ok_or("webhook not initialized")?;
    let obs   = &state.obs_config;

    // Raw TCP connection — OBS WebSocket is always local, no TLS needed.
    let stream = std::net::TcpStream::connect(format!("{}:{}", obs.host, obs.port))
        .map_err(|e| format!("TCP connect: {e}"))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|e| format!("set_read_timeout: {e}"))?;

    let (mut ws, _) = tungstenite::client(
        format!("ws://{}:{}/", obs.host, obs.port),
        stream,
    ).map_err(|e| format!("WS handshake: {e}"))?;

    // Read Hello (op 0)
    let hello_text = match ws.read().map_err(|e| format!("read hello: {e}"))? {
        tungstenite::Message::Text(t) => t,
        other => return Err(format!("unexpected hello frame: {:?}", other)),
    };
    let hello: serde_json::Value = serde_json::from_str(&hello_text)
        .map_err(|e| format!("parse hello: {e}"))?;

    // Build Identify (op 1) — with authentication if OBS requires it.
    let auth_str: Option<String> = if let (Some(auth_info), Some(password)) = (
        hello["d"]["authentication"].as_object(),
        obs.password.as_deref().filter(|p| !p.is_empty()),
    ) {
        let salt      = auth_info["salt"].as_str().unwrap_or("");
        let challenge = auth_info["challenge"].as_str().unwrap_or("");
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(password.as_bytes());
        h.update(salt.as_bytes());
        let secret = obs_b64(&h.finalize());
        let mut h2 = Sha256::new();
        h2.update(secret.as_bytes());
        h2.update(challenge.as_bytes());
        Some(obs_b64(&h2.finalize()))
    } else {
        None
    };

    let identify = match auth_str {
        Some(auth) => format!(r#"{{"op":1,"d":{{"rpcVersion":1,"authentication":"{}"}}}}"#, auth),
        None       => r#"{"op":1,"d":{"rpcVersion":1}}"#.to_string(),
    };
    ws.send(tungstenite::Message::Text(identify))
        .map_err(|e| format!("send identify: {e}"))?;

    // Read Identified (op 2)
    ws.read().map_err(|e| format!("read identified: {e}"))?;

    // Send SaveReplayBuffer request (op 6)
    ws.send(tungstenite::Message::Text(
        r#"{"op":6,"d":{"requestType":"SaveReplayBuffer","requestId":"clip"}}"#.to_string(),
    )).map_err(|e| format!("send request: {e}"))?;

    let _ = ws.close(None);
    Ok(())
}

fn obs_b64(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n  = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >>  6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[( n        & 63) as usize] as char } else { '=' });
    }
    out
}
