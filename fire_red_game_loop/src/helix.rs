//! Twitch Helix API integration — stream markers, polls, and predictions.
//!
//! Call [`init`] once at startup with a [`TwitchHelixConfig`].  Then call the
//! event helpers from game/encounter code; each enqueues a task for the
//! background thread and returns immediately so the game loop is never blocked.
//!
//! # Required OAuth scopes
//!
//! | Feature           | Scope                        |
//! |-------------------|------------------------------|
//! | Stream markers    | `channel:manage:broadcast`   |
//! | Polls             | `channel:manage:polls`       |
//! | Predictions       | `channel:manage:predictions` |
//!
//! # Config example (`config.toml`)
//!
//! ```toml
//! [twitch_helix]
//! client_id      = "xxxx"
//! token          = "oauth:xxxx"
//! broadcaster_id = "123456789"
//! marker_on_death        = true
//! marker_on_shiny        = true
//! marker_on_badge        = true
//! poll_on_legendary      = true
//! prediction_on_legendary = true
//! ```

use crate::config::TwitchHelixConfig;
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

const HELIX_MARKERS: &str     = "https://api.twitch.tv/helix/streams/markers";
const HELIX_POLLS: &str       = "https://api.twitch.tv/helix/polls";
const HELIX_PREDICTIONS: &str = "https://api.twitch.tv/helix/predictions";

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct HelixState {
    tx:     Sender<HelixTask>,
    config: TwitchHelixConfig,
    /// Tracks the active prediction so we can resolve it later.
    active_prediction: std::sync::Mutex<Option<ActivePrediction>>,
}

struct ActivePrediction {
    id:          String,
    outcome_yes: String,
    outcome_no:  String,
}

static STATE: OnceLock<HelixState> = OnceLock::new();

// ---------------------------------------------------------------------------
// Worker task
// ---------------------------------------------------------------------------

enum HelixTask {
    StreamMarker { description: String },
    CreatePoll { title: String, choices: Vec<String>, duration_secs: u32 },
    CreatePrediction { title: String, outcome_yes: String, outcome_no: String, window_secs: u32 },
    ResolvePrediction { outcome: PredictionResult },
}

/// Outcome of a Twitch prediction: whether the poll result was "Yes" or "No".
#[derive(Clone, Copy)]
pub enum PredictionResult {
    Yes,
    No,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the Helix integration. No-op if called more than once.
pub fn init(config: TwitchHelixConfig) {
    if STATE.get().is_some() {
        return;
    }
    let (tx, rx) = channel::<HelixTask>();
    let cfg_clone = config.clone();
    let spawn_result = std::thread::Builder::new()
        .name("helix-worker".into())
        .spawn(move || run_worker(rx, &cfg_clone));
    match spawn_result {
        Err(e) => tracing::error!("Failed to spawn Helix worker thread: {e}"),
        Ok(_) => {
            let _ = STATE.set(HelixState {
                tx,
                config,
                active_prediction: std::sync::Mutex::new(None),
            });
        }
    }
}

/// Drop a VOD stream marker if `marker_on_death` is set.
pub fn on_death(species: &str, level: u8) {
    let Some(state) = STATE.get() else { return };
    if !state.config.marker_on_death { return }
    send(&state.tx, HelixTask::StreamMarker {
        description: format!("Death: {} Lv.{}", species, level),
    });
}

/// Drop a VOD stream marker (and optionally open a poll/prediction) on shiny
/// encounter. Call with `is_legendary = true` for legendary Pokémon.
pub fn on_shiny_encounter(species: &str, is_legendary: bool) {
    let Some(state) = STATE.get() else { return };
    if state.config.marker_on_shiny {
        send(&state.tx, HelixTask::StreamMarker {
            description: format!("Shiny {}!", species),
        });
    }
    maybe_poll_legendary(state, species, is_legendary);
}

/// Drop a VOD stream marker if `marker_on_badge` is set. Also resolves any
/// open prediction with a "Yes" outcome (caught/won).
pub fn on_badge(badge_name: &str) {
    let Some(state) = STATE.get() else { return };
    if state.config.marker_on_badge {
        send(&state.tx, HelixTask::StreamMarker {
            description: format!("{} Badge earned", badge_name),
        });
    }
}

/// Fire on a non-shiny legendary encounter. Opens a poll/prediction per config.
pub fn on_legendary_encounter(species: &str) {
    let Some(state) = STATE.get() else { return };
    maybe_poll_legendary(state, species, true);
}

/// Resolve the active prediction with the given outcome. No-op if no prediction
/// is open.
pub fn resolve_prediction(outcome: PredictionResult) {
    let Some(state) = STATE.get() else { return };
    send(&state.tx, HelixTask::ResolvePrediction { outcome });
}

fn maybe_poll_legendary(state: &HelixState, species: &str, is_legendary: bool) {
    if !is_legendary { return }
    if state.config.poll_on_legendary {
        send(&state.tx, HelixTask::CreatePoll {
            title: format!("Catch the {}?", species),
            choices: vec!["Yes".to_string(), "No".to_string()],
            duration_secs: state.config.poll_duration_secs,
        });
    }
    if state.config.prediction_on_legendary {
        send(&state.tx, HelixTask::CreatePrediction {
            title: format!("Will I catch the {}?", species),
            outcome_yes: "Caught it".to_string(),
            outcome_no:  "Got away".to_string(),
            window_secs: state.config.prediction_window_secs,
        });
    }
}

/// Returns true for the Gen-I/II legendaries and mythicals catchable in FireRed.
///
/// Species IDs: Articuno 144, Zapdos 145, Moltres 146, Mewtwo 150, Mew 151.
/// Also includes the Johto legends that appear via events: Entei 244, Raikou 243,
/// Suicune 245, Lugia 249, Ho-Oh 250, Celebi 251.
pub fn is_legendary(species: u16) -> bool {
    matches!(species, 144..=146 | 150 | 151 | 243..=245 | 249..=251)
}

fn send(tx: &Sender<HelixTask>, task: HelixTask) {
    if let Err(e) = tx.send(task) {
        tracing::debug!("Helix worker channel closed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

fn run_worker(rx: std::sync::mpsc::Receiver<HelixTask>, config: &TwitchHelixConfig) {
    let bearer = config.token.trim_start_matches("oauth:").to_string();
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    for task in rx {
        match task {
            HelixTask::StreamMarker { description } => {
                post_stream_marker(&client, &bearer, config, &description);
            }
            HelixTask::CreatePoll { title, choices, duration_secs } => {
                post_poll(&client, &bearer, config, &title, &choices, duration_secs);
            }
            HelixTask::CreatePrediction { title, outcome_yes, outcome_no, window_secs } => {
                post_prediction(&client, &bearer, config, &title, &outcome_yes, &outcome_no, window_secs);
            }
            HelixTask::ResolvePrediction { outcome } => {
                patch_prediction(&client, &bearer, config, outcome);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helix calls
// ---------------------------------------------------------------------------

fn helix_headers(
    rb: reqwest::blocking::RequestBuilder,
    bearer: &str,
    client_id: &str,
) -> reqwest::blocking::RequestBuilder {
    rb.header("Authorization", format!("Bearer {bearer}"))
      .header("Client-Id", client_id)
}

fn post_stream_marker(
    client: &reqwest::blocking::Client,
    bearer: &str,
    config: &TwitchHelixConfig,
    description: &str,
) {
    let body = serde_json::json!({
        "user_id":    config.broadcaster_id,
        "description": &description[..description.len().min(140)],
    });
    let result = helix_headers(client.post(HELIX_MARKERS), bearer, &config.client_id)
        .json(&body)
        .send();
    match result {
        Ok(r) if r.status().is_success() => {
            tracing::info!("Twitch stream marker posted: {description}");
        }
        Ok(r) => {
            let status = r.status();
            let text = r.text().unwrap_or_default();
            tracing::warn!("Twitch stream marker HTTP {status}: {text}");
        }
        Err(e) => tracing::warn!("Twitch stream marker request failed: {e}"),
    }
}

fn post_poll(
    client: &reqwest::blocking::Client,
    bearer: &str,
    config: &TwitchHelixConfig,
    title: &str,
    choices: &[String],
    duration_secs: u32,
) {
    let choice_objects: Vec<serde_json::Value> = choices
        .iter()
        .map(|c| serde_json::json!({ "title": c }))
        .collect();
    let body = serde_json::json!({
        "broadcaster_id": config.broadcaster_id,
        "title":          &title[..title.len().min(60)],
        "choices":        choice_objects,
        "duration":       duration_secs,
    });
    let result = helix_headers(client.post(HELIX_POLLS), bearer, &config.client_id)
        .json(&body)
        .send();
    match result {
        Ok(r) if r.status().is_success() => {
            tracing::info!("Twitch poll created: {title}");
        }
        Ok(r) => {
            let status = r.status();
            let text = r.text().unwrap_or_default();
            tracing::warn!("Twitch poll HTTP {status}: {text}");
        }
        Err(e) => tracing::warn!("Twitch poll request failed: {e}"),
    }
}

fn post_prediction(
    client: &reqwest::blocking::Client,
    bearer: &str,
    config: &TwitchHelixConfig,
    title: &str,
    outcome_yes: &str,
    outcome_no: &str,
    window_secs: u32,
) {
    let body = serde_json::json!({
        "broadcaster_id":     config.broadcaster_id,
        "title":              &title[..title.len().min(45)],
        "outcomes":           [
            { "title": outcome_yes },
            { "title": outcome_no  },
        ],
        "prediction_window": window_secs,
    });
    let result = helix_headers(client.post(HELIX_PREDICTIONS), bearer, &config.client_id)
        .json(&body)
        .send();
    match result {
        Ok(r) if r.status().is_success() => {
            // Parse and store the prediction ID + outcome IDs so we can resolve later.
            match r.json::<serde_json::Value>() {
                Ok(val) => {
                    if let Some(pred) = val["data"].get(0) {
                        let pred_id = pred["id"].as_str().unwrap_or("").to_string();
                        let outcomes = pred["outcomes"].as_array();
                        let yes_id = outcomes
                            .and_then(|o| o.first())
                            .and_then(|o| o["id"].as_str())
                            .unwrap_or("")
                            .to_string();
                        let no_id = outcomes
                            .and_then(|o| o.get(1))
                            .and_then(|o| o["id"].as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(state) = STATE.get() {
                            *state.active_prediction.lock().unwrap_or_else(|p| p.into_inner()) =
                                Some(ActivePrediction {
                                    id:          pred_id.clone(),
                                    outcome_yes: yes_id,
                                    outcome_no:  no_id,
                                });
                        }
                        tracing::info!("Twitch prediction created: {title} (id={pred_id})");
                    }
                }
                Err(e) => tracing::warn!("Twitch prediction response parse error: {e}"),
            }
        }
        Ok(r) => {
            let status = r.status();
            let text = r.text().unwrap_or_default();
            tracing::warn!("Twitch prediction HTTP {status}: {text}");
        }
        Err(e) => tracing::warn!("Twitch prediction request failed: {e}"),
    }
}

fn patch_prediction(
    client: &reqwest::blocking::Client,
    bearer: &str,
    config: &TwitchHelixConfig,
    outcome: PredictionResult,
) {
    let Some(state) = STATE.get() else { return };
    let pred = state
        .active_prediction
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    let Some(pred) = pred else {
        tracing::debug!("resolve_prediction called but no active prediction");
        return;
    };
    let winning_outcome_id = match outcome {
        PredictionResult::Yes => &pred.outcome_yes,
        PredictionResult::No  => &pred.outcome_no,
    };
    let body = serde_json::json!({
        "broadcaster_id":     config.broadcaster_id,
        "id":                 pred.id,
        "status":             "RESOLVED",
        "winning_outcome_id": winning_outcome_id,
    });
    let result = helix_headers(client.patch(HELIX_PREDICTIONS), bearer, &config.client_id)
        .json(&body)
        .send();
    match result {
        Ok(r) if r.status().is_success() => {
            tracing::info!("Twitch prediction resolved (id={})", pred.id);
        }
        Ok(r) => {
            let status = r.status();
            let text = r.text().unwrap_or_default();
            tracing::warn!("Twitch prediction resolve HTTP {status}: {text}");
        }
        Err(e) => tracing::warn!("Twitch prediction resolve request failed: {e}"),
    }
}
