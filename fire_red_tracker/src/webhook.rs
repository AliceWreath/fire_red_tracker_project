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
// Internal types
// ---------------------------------------------------------------------------

enum PostBody {
    /// Serialize the event to JSON using serde (existing behaviour).
    Json(WebhookEvent),
    /// Already-rendered string; POST verbatim as application/json.
    Raw(String),
}

struct WebhookState {
    tx:     Sender<(String, PostBody)>,
    config: WebhookConfig,
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
pub fn init(config: WebhookConfig) {
    let (tx, rx) = channel::<(String, PostBody)>();
    if STATE.set(WebhookState { tx, config }).is_err() {
        return; // already initialized
    }
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        for (url, body) in rx {
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
    });
}

/// Enqueue a webhook event for delivery. Returns immediately; the HTTP POST
/// is performed by the background thread started in [`init`].
///
/// Does nothing if no URL is configured for the event type, or if [`init`]
/// was never called.
pub fn fire_event(event: WebhookEvent) {
    let Some(state) = STATE.get() else { return; };
    let (url, template) = match &event {
        WebhookEvent::Death { .. } => (state.config.death_url.as_deref(), state.config.death_template.as_deref()),
        WebhookEvent::Catch { .. } => (state.config.catch_url.as_deref(), state.config.catch_template.as_deref()),
        WebhookEvent::Shiny { .. } => (state.config.shiny_url.as_deref(), state.config.shiny_template.as_deref()),
        WebhookEvent::Wipe  { .. } => (state.config.wipe_url.as_deref(),  state.config.wipe_template.as_deref()),
    };
    if let Some(url) = url {
        let body = match template {
            Some(t) => PostBody::Raw(render_template(t, &event)),
            None    => PostBody::Json(event),
        };
        let _ = state.tx.send((url.to_string(), body));
    }
}
