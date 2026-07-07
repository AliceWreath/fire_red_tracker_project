//! WebSocket overlay server mode.
//!
//! Activated via `--ws-port PORT`. Runs a headless HTTP + WebSocket server
//! instead of the egui window. OBS connects to it as a Browser Source.
//!
//! - `GET /`   — serves the HTML overlay page
//! - `GET /ws` — WebSocket endpoint; pushes JSON state ~10/s
//!
//! Sprites are embedded directly in the JSON as base64 PNG data URIs, so no
//! separate HTTP sprite endpoint or browser caching issues exist.

use crate::client::{MonitorSlot, PngSpriteCache, SharedSlots, encode_png};
use axum::{
    Extension, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse},
    routing::{delete, get, patch, post},
};
use fire_red_database::{CaughtPokemon, DeadPokemon, User};
use fire_red_states::{
    ClientMessage, GameState, LockOrRecover, MAX_NATIONAL_DEX_FIRERED, is_shiny,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

// Base64 encoding delegated to the shared implementation in fire_red_states.
use fire_red_states::base64_encode;

/// Extracts the DB connection string from `WebState::db_conn`, returning an
/// error JSON response if none is configured. Used in every DB-backed handler.
///
/// `macro_rules!` scoping is textual, so this must stay above the `mod`
/// declarations for the submodules to use it.
macro_rules! require_db {
    ($state:expr) => {
        match $state.db_conn {
            Some(s) => s,
            None => return axum::Json(serde_json::json!({ "error": "No database configured" })),
        }
    };
}

mod api_runs;
mod api_slots;
mod auth;
mod commands;
mod dashboard;
mod discord_interactions;
mod dto;
mod integrations;
mod pages;
mod rate_limit;
mod router;
mod run_admin;
mod site_pages;
mod slot_commands;
mod state;
mod ws;

pub(crate) use api_runs::*;
pub(crate) use api_slots::*;
pub(crate) use auth::*;
pub(crate) use commands::*;
pub(crate) use dashboard::*;
pub(crate) use discord_interactions::*;
pub(crate) use dto::*;
pub(crate) use integrations::*;
pub(crate) use pages::*;
pub(crate) use router::*;
pub(crate) use run_admin::*;
pub(crate) use site_pages::*;
pub(crate) use slot_commands::*;
pub(crate) use state::*;
pub(crate) use ws::*;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_HTML: &str =
        r#"<!DOCTYPE html><html><head><!-- THEME_SLOT --></head><body>__VERSION__</body></html>"#;

    #[test]
    fn apply_page_replaces_version() {
        let out = apply_page(SAMPLE_HTML, false);
        assert!(out.contains(VERSION), "VERSION not injected");
        assert!(!out.contains("__VERSION__"), "__VERSION__ not replaced");
    }

    #[test]
    fn apply_page_no_theme_removes_slot() {
        let out = apply_page(SAMPLE_HTML, false);
        assert!(
            !out.contains("<!-- THEME_SLOT -->"),
            "theme slot should be removed"
        );
        assert!(!out.contains("data-theme"), "no theme attr expected");
    }

    #[test]
    fn apply_page_with_theme_dark_removes_slot() {
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("dark"));
        assert!(!out.contains("<!-- THEME_SLOT -->"));
        assert!(!out.contains("data-theme"));
    }

    #[test]
    fn apply_page_with_theme_light_injects_attr() {
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("light"));
        assert!(!out.contains("<!-- THEME_SLOT -->"));
        assert!(
            out.contains(r#"dataset.theme="light""#),
            "light theme not injected: {out}"
        );
    }

    #[test]
    fn apply_page_with_theme_rejects_invalid_input() {
        // Themes containing characters outside [a-zA-Z0-9_-] are rejected entirely
        // rather than being stripped and concatenated, which would produce confusing output.
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("light<script>alert(1)</script>"));
        assert!(!out.contains("<script>alert"), "XSS not sanitized");
        assert!(
            !out.contains("lightscript"),
            "stripped-and-concatenated theme should not appear"
        );
        assert!(
            !out.contains("data-theme"),
            "rejected theme should not inject any attribute"
        );
    }

    #[test]
    fn apply_page_with_theme_rejects_oversized_input() {
        let long = "a".repeat(33);
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some(&long));
        assert!(
            !out.contains("data-theme"),
            "theme longer than 32 chars should be rejected"
        );
    }

    #[test]
    fn apply_page_with_theme_accepts_hyphen_and_underscore() {
        let out = apply_page_with_theme(SAMPLE_HTML, false, Some("my_custom-theme"));
        assert!(
            out.contains(r#"dataset.theme="my_custom-theme""#),
            "valid theme with - and _ rejected"
        );
    }

    #[test]
    fn apply_page_testing_injects_banner() {
        let out = apply_page(SAMPLE_HTML, true);
        assert!(out.contains("[TESTING]"), "testing banner missing");
    }

    #[test]
    fn apply_page_theme_and_testing_both_applied() {
        let out = apply_page_with_theme(SAMPLE_HTML, true, Some("light"));
        assert!(out.contains("[TESTING]"));
        assert!(out.contains(r#"dataset.theme="light""#));
    }

    // ── API integration tests ────────────────────────────────────────────────

    fn empty_web_state() -> WebState {
        let (tx, _rx) = tokio::sync::watch::channel(String::new());
        let live_slots: SharedSlots = Arc::new(Mutex::new(vec![]));
        WebState {
            tx,
            live_slots,
            db_conn: None,
            testing: true,
            allow_injections: false,
            connector: None,
            backup_dir: None,
            backup_keep: 10,
            discord_slash: None,
            config_path: None,
            user_active_run: Arc::new(Mutex::new(HashMap::new())),
            integration_manager: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // The auth wall (v0.9.70) protects every non-public route: API and WS
    // requests without a session get 401 JSON, page requests get a 303
    // redirect to the landing page.

    #[tokio::test]
    async fn api_state_unauthenticated_returns_401_json() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).expect("body must be JSON");
        assert!(v.get("error").is_some(), "expected error field");
    }

    #[tokio::test]
    async fn api_slot_unauthenticated_returns_401() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/slot/0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ws_unauthenticated_returns_401() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ws")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn serve_html_root_returns_ok() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn page_unauthenticated_redirects_to_landing() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode, header};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/about")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/",
            "unauthenticated page request should redirect to the landing page"
        );
    }

    #[tokio::test]
    async fn api_runs_unauthenticated_returns_401() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/runs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Auth helper unit tests ───────────────────────────────────────────────

    #[test]
    fn is_public_route_matrix() {
        use axum::http::Method;
        for path in ["/", "/register", "/api/login", "/api/catch_rate", "/share/abc123", "/static/overlay.js"] {
            assert!(is_public_route(path, &Method::GET), "{path} should be public");
        }
        assert!(is_public_route("/api/users", &Method::POST), "register endpoint is public");
        for path in ["/api/state", "/api/runs", "/api/users", "/dashboard", "/ws", "/about"] {
            assert!(!is_public_route(path, &Method::GET), "{path} should require auth");
        }
    }

    #[test]
    fn extract_bearer_all_sources() {
        use axum::http::{HeaderMap, HeaderValue};

        let mut h = HeaderMap::new();
        h.insert("Authorization", HeaderValue::from_static("Bearer tok-a"));
        assert_eq!(extract_bearer(&h).as_deref(), Some("tok-a"));

        let mut h = HeaderMap::new();
        h.insert("X-Session-Token", HeaderValue::from_static("tok-b"));
        assert_eq!(extract_bearer(&h).as_deref(), Some("tok-b"));

        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; frt_token=tok-c; more=2"),
        );
        assert_eq!(extract_bearer(&h).as_deref(), Some("tok-c"));

        assert_eq!(extract_bearer(&HeaderMap::new()), None);
    }

    #[test]
    fn extract_query_token_parses_uri() {
        let uri: axum::http::Uri = "/1/hp?theme=dark&token=tok-q".parse().unwrap();
        assert_eq!(extract_query_token(&uri).as_deref(), Some("tok-q"));

        let uri: axum::http::Uri = "/1/hp?token=".parse().unwrap();
        assert_eq!(extract_query_token(&uri), None, "empty token is ignored");

        let uri: axum::http::Uri = "/1/hp".parse().unwrap();
        assert_eq!(extract_query_token(&uri), None);
    }

    #[tokio::test]
    async fn api_catch_rate_missing_params_returns_error_json() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_router(empty_web_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/catch_rate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).expect("body must be JSON");
        assert!(v.get("error").is_some(), "expected error field for missing params");
    }
}
