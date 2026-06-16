//! Shared configuration types used by both the tracker and aggregator.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

// ---------------------------------------------------------------------------
// DupesClauseMode
// ---------------------------------------------------------------------------

/// Controls how the duplicate-species clause is enforced during a Nuzlocke run.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum DupesClauseMode {
    #[default]
    Off,
    PerPlayer,
    Shared,
}

impl Serialize for DupesClauseMode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            DupesClauseMode::Off => "off",
            DupesClauseMode::PerPlayer => "per_player",
            DupesClauseMode::Shared => "shared",
        })
    }
}

impl<'de> Deserialize<'de> for DupesClauseMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = DupesClauseMode;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, r#"bool or one of "off", "per_player", "shared""#)
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<DupesClauseMode, E> {
                Ok(if v {
                    DupesClauseMode::Shared
                } else {
                    DupesClauseMode::Off
                })
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<DupesClauseMode, E> {
                match v {
                    "off" => Ok(DupesClauseMode::Off),
                    "per_player" => Ok(DupesClauseMode::PerPlayer),
                    "shared" => Ok(DupesClauseMode::Shared),
                    _ => Err(E::unknown_variant(v, &["off", "per_player", "shared"])),
                }
            }
        }
        d.deserialize_any(V)
    }
}

impl fmt::Display for DupesClauseMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DupesClauseMode::Off => "Off",
            DupesClauseMode::PerPlayer => "Per Player",
            DupesClauseMode::Shared => "Shared",
        })
    }
}

// ---------------------------------------------------------------------------
// NuzlockePreset
// ---------------------------------------------------------------------------

/// Pre-configured Nuzlocke rule sets selectable in config.
///
/// Each variant maps to a `(DupesClauseMode, allow_species_repeats)` tuple via
/// [`NuzlockePreset::settings`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NuzlockePreset {
    Standard,
    Hardcore,
    Randomizer,
    SoulLink,
}

impl NuzlockePreset {
    pub fn settings(self) -> (DupesClauseMode, bool) {
        match self {
            NuzlockePreset::Standard => (DupesClauseMode::Off, false),
            NuzlockePreset::Hardcore => (DupesClauseMode::PerPlayer, false),
            NuzlockePreset::Randomizer => (DupesClauseMode::Off, true),
            NuzlockePreset::SoulLink => (DupesClauseMode::Shared, false),
        }
    }
}

// ---------------------------------------------------------------------------
// WebhookConfig
// ---------------------------------------------------------------------------

/// Per-event HTTP webhook configuration.
///
/// Each event type has an optional `*_url` (destination) and `*_template`
/// (body template).  When a template is omitted the default JSON payload is
/// sent.  See `fire_red_game_loop::webhook` for placeholder documentation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shiny_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shiny_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wipe_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wipe_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nuzlocke_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nuzlocke_template: Option<String>,
    #[serde(default)]
    pub notify_on_death: bool,
    #[serde(default)]
    pub notify_on_shiny: bool,
    #[serde(default)]
    pub notify_on_wipe: bool,
}

impl WebhookConfig {
    pub fn is_empty(&self) -> bool {
        self.death_url.is_none()
            && self.death_template.is_none()
            && self.catch_url.is_none()
            && self.catch_template.is_none()
            && self.shiny_url.is_none()
            && self.shiny_template.is_none()
            && self.wipe_url.is_none()
            && self.wipe_template.is_none()
            && self.badge_url.is_none()
            && self.badge_template.is_none()
            && self.nickname_url.is_none()
            && self.nickname_template.is_none()
            && self.nuzlocke_url.is_none()
            && self.nuzlocke_template.is_none()
            && !self.notify_on_death
            && !self.notify_on_shiny
            && !self.notify_on_wipe
    }
}

// ---------------------------------------------------------------------------
// ObsConfig
// ---------------------------------------------------------------------------

/// OBS WebSocket v5 integration config for clip triggers and scene switching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsConfig {
    #[serde(default = "default_obs_host")]
    pub host: String,
    #[serde(default = "default_obs_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub clip_on_death: bool,
    #[serde(default)]
    pub clip_on_shiny: bool,
    #[serde(default)]
    pub clip_on_wipe: bool,
    #[serde(default)]
    pub clip_on_badge: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_on_death: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_on_wipe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_on_shiny: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_on_badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_on_catch: Option<String>,
}

fn default_obs_host() -> String { "localhost".to_string() }
fn default_obs_port() -> u16 { 4455 }

impl Default for ObsConfig {
    fn default() -> Self {
        Self {
            host: default_obs_host(),
            port: default_obs_port(),
            password: None,
            clip_on_death: false,
            clip_on_shiny: false,
            clip_on_wipe: false,
            clip_on_badge: false,
            scene_on_death: None,
            scene_on_wipe: None,
            scene_on_shiny: None,
            scene_on_badge: None,
            scene_on_catch: None,
        }
    }
}

impl ObsConfig {
    pub fn is_default(&self) -> bool {
        !self.clip_on_death
            && !self.clip_on_shiny
            && !self.clip_on_wipe
            && !self.clip_on_badge
            && self.scene_on_death.is_none()
            && self.scene_on_wipe.is_none()
            && self.scene_on_shiny.is_none()
            && self.scene_on_badge.is_none()
            && self.scene_on_catch.is_none()
    }
}

// ---------------------------------------------------------------------------
// TwitchHelixConfig
// ---------------------------------------------------------------------------

/// Twitch Helix API config for stream markers, polls, and predictions.
///
/// Requires a user-access token with scopes matching the enabled features;
/// see `fire_red_game_loop::helix` for the scope table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchHelixConfig {
    pub client_id: String,
    pub token: String,
    pub broadcaster_id: String,
    #[serde(default)]
    pub marker_on_death: bool,
    #[serde(default)]
    pub marker_on_shiny: bool,
    #[serde(default)]
    pub marker_on_badge: bool,
    #[serde(default)]
    pub poll_on_legendary: bool,
    #[serde(default = "default_poll_duration_secs")]
    pub poll_duration_secs: u32,
    #[serde(default)]
    pub prediction_on_legendary: bool,
    #[serde(default = "default_prediction_window_secs")]
    pub prediction_window_secs: u32,
}

fn default_poll_duration_secs() -> u32 { 60 }
fn default_prediction_window_secs() -> u32 { 120 }
