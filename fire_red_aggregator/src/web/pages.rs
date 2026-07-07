//! HTML page/overlay serving handlers (embedded overlay HTML).

use super::*;

pub(crate) const OVERLAY_HTML: &str = include_str!("../overlay.html");
pub(crate) const FOCUSED_HTML: &str = include_str!("../focused.html");
pub(crate) const DBVIEWER_HTML: &str = include_str!("../db.html");
pub(crate) const HISTORY_HTML: &str = include_str!("../history.html");
pub(crate) const ALERTS_HTML: &str = include_str!("../alerts.html");
pub(crate) const ROUTES_HTML: &str = include_str!("../routes.html");
pub(crate) const PARTY_PLAIN_HTML: &str = include_str!("../party_plain.html");
pub(crate) const CMD_HTML: &str = include_str!("../cmd.html");
pub(crate) const DBQUERY_HTML: &str = include_str!("../dbquery.html");
pub(crate) const RUNSTATS_HTML: &str = include_str!("../run_stats.html");
pub(crate) const SHINY_HTML: &str = include_str!("../shiny.html");
pub(crate) const MEMORIAL_HTML: &str = include_str!("../memorial.html");
pub(crate) const SOULLINK_HTML: &str = include_str!("../soullink.html");
pub(crate) const SOULLINK_MANAGE_HTML: &str = include_str!("../soullink_manage.html");
pub(crate) const TYPES_HTML: &str = include_str!("../types.html");
pub(crate) const ABOUT_HTML: &str = include_str!("../about.html");
pub(crate) const COMPARE_HTML: &str = include_str!("../compare.html");
pub(crate) const ITEMS_HTML: &str = include_str!("../items.html");
pub(crate) const MOVES_HTML: &str = include_str!("../moves.html");
pub(crate) const MOBILE_HTML: &str = include_str!("../mobile.html");
pub(crate) const TRAINERS_HTML: &str = include_str!("../trainers.html");
pub(crate) const TIMELINE_HTML: &str = include_str!("../timeline.html");
pub(crate) const SPECIES_HTML: &str = include_str!("../species.html");
pub(crate) const DEATHS_HTML: &str = include_str!("../deaths.html");
pub(crate) const ENCOUNTER_COUNT_HTML: &str = include_str!("../encounter_count.html");
pub(crate) const HP_HTML: &str = include_str!("../hp.html");
pub(crate) const DAMAGE_CALC_HTML: &str = include_str!("../damage_calc.html");
pub(crate) const BADGES_HTML: &str = include_str!("../badges.html");
pub(crate) const NEXT_GYM_HTML: &str = include_str!("../next_gym.html");
pub(crate) const ENCOUNTER_TABLE_HTML: &str = include_str!("../encounter_table.html");
pub(crate) const MONEY_HTML: &str = include_str!("../money.html");
pub(crate) const PLAYTIME_HTML: &str = include_str!("../playtime.html");
pub(crate) const GOALS_HTML: &str = include_str!("../goals.html");
pub(crate) const GOALS_MANAGE_HTML: &str = include_str!("../goals_manage.html");
pub(crate) const VS_LEADER_HTML: &str = include_str!("../vs_leader.html");

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");


pub(crate) const TESTING_BANNER: &str = r#"<div id="testing-banner" style="position:fixed;top:0;left:0;right:0;z-index:9999;background:#b00;color:#fff;font-weight:bold;text-align:center;padding:4px 0;font-family:sans-serif;font-size:14px;">[TESTING]</div>"#;

pub(crate) fn apply_page(html: &str, testing: bool) -> String {
    apply_page_with_theme(html, testing, None)
}

/// Renders an HTML page, injecting the version, optional testing banner,
/// and an optional theme by setting `data-theme` on `<html>` and replacing
/// the `<!-- THEME_SLOT -->` placeholder with a `<script>` that applies it.
///
/// Supported theme values: `dark` (default, no-op), `light`, and any custom
/// string that maps to a CSS `data-theme` attribute value.
pub(crate) fn apply_page_with_theme(html: &str, testing: bool, theme: Option<&str>) -> String {
    let html = html.replace("__VERSION__", VERSION);

    // Inject theme attribute and a tiny script that sets it before first paint,
    // preventing a flash of the default (dark) theme.
    //
    // Only themes whose names consist entirely of `[a-zA-Z0-9_-]` are accepted.
    // Any theme containing other characters is rejected and treated as the default
    // rather than silently concatenating the sanitized fragments (which would
    // produce confusing output and mask typos).
    let html = match theme {
        None | Some("dark") | Some("") => html.replace("<!-- THEME_SLOT -->", ""),
        Some(t) => {
            let all_safe = t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            let within_len = t.len() <= 32;
            if all_safe && within_len {
                let injection =
                    format!(r#"<script>document.documentElement.dataset.theme="{t}"</script>"#);
                html.replace("<!-- THEME_SLOT -->", &injection)
            } else {
                // Invalid theme — fall back to default (dark) silently.
                html.replace("<!-- THEME_SLOT -->", "")
            }
        }
    };

    if testing {
        html.replacen("<body>", &format!("<body>{}", TESTING_BANNER), 1)
    } else {
        html
    }
}

pub(crate) async fn serve_html(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(OVERLAY_HTML, state.testing, theme))
}

pub(crate) async fn serve_focused(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(FOCUSED_HTML, state.testing, theme))
}

pub(crate) async fn serve_party(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    if params.contains_key("plain-view") {
        Html(apply_page_with_theme(
            PARTY_PLAIN_HTML,
            state.testing,
            theme,
        ))
    } else {
        Html(apply_page_with_theme(FOCUSED_HTML, state.testing, theme))
    }
}

pub(crate) async fn serve_db_viewer(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(DBVIEWER_HTML, state.testing))
}

pub(crate) async fn serve_history(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(HISTORY_HTML, state.testing))
}

pub(crate) async fn serve_alerts(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(ALERTS_HTML, state.testing))
}

pub(crate) async fn serve_routes(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(ROUTES_HTML, state.testing))
}

pub(crate) async fn serve_compare(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(COMPARE_HTML, state.testing))
}

pub(crate) async fn serve_cmd(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(CMD_HTML, state.testing))
}

pub(crate) async fn serve_db_query(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(DBQUERY_HTML, state.testing))
}

pub(crate) async fn serve_run_stats(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(RUNSTATS_HTML, state.testing))
}

pub(crate) async fn serve_shiny(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SHINY_HTML, state.testing))
}

pub(crate) async fn serve_memorial(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(MEMORIAL_HTML, state.testing))
}

pub(crate) async fn serve_soullink(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SOULLINK_HTML, state.testing))
}

pub(crate) async fn serve_types_page(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(TYPES_HTML, state.testing, theme))
}

pub(crate) async fn serve_about(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(ABOUT_HTML, state.testing))
}

/// `GET /soullink/manage` — Soul Link partner override management page.
pub(crate) async fn serve_soullink_manage(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SOULLINK_MANAGE_HTML, state.testing))
}

/// `GET /:index/items` — bag item viewer page for a specific tracker slot.
pub(crate) async fn serve_items(
    State(state): State<WebState>,
    Path(_index): Path<usize>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(ITEMS_HTML, state.testing, theme))
}

/// `GET /:index/moves` — move / PP overlay for a specific tracker slot.
pub(crate) async fn serve_moves_page(
    State(state): State<WebState>,
    Path(_index): Path<usize>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(MOVES_HTML, state.testing, theme))
}

/// `GET /party/mobile` — mobile-friendly party viewer.
pub(crate) async fn serve_mobile_party(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(MOBILE_HTML, state.testing))
}

/// `GET /timeline` and `GET /run/:id/timeline` — visual run timeline page.
pub(crate) async fn serve_timeline(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(TIMELINE_HTML, state.testing))
}

/// `GET /species` — cross-run per-species survival statistics page.
pub(crate) async fn serve_species(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(SPECIES_HTML, state.testing))
}

/// `GET /trainers` and `GET /run/:id/trainers` — trainer battle log page.
pub(crate) async fn serve_trainers(State(state): State<WebState>) -> Html<String> {
    Html(apply_page(TRAINERS_HTML, state.testing))
}

/// `GET /:index/deaths` — compact death-counter overlay for a small OBS Browser Source.
///
/// Shows a large red death count. Subscribes to `?show=deaths` WS filter so
/// the browser only receives the `dead` list and `run_summary`; no party,
/// encounter, or box data is transferred.
pub(crate) async fn serve_deaths_overlay(
    State(state): State<WebState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(|s| s.as_str());
    Html(apply_page_with_theme(DEATHS_HTML, state.testing, theme))
}

/// `GET /:index/encounter_count` — encounter counter overlay for OBS.
///
/// Shows the total encounter count for the run with a caught/missed breakdown.
/// Subscribes to `?show=counter` WS filter (only `db_encounters` transferred).
pub(crate) async fn serve_encounter_count(
    State(state): State<WebState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(|s| s.as_str());
    Html(apply_page_with_theme(ENCOUNTER_COUNT_HTML, state.testing, theme))
}

pub(crate) async fn serve_hp_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(HP_HTML, state.testing, theme))
}

pub(crate) async fn serve_damage_calc_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(DAMAGE_CALC_HTML, state.testing, theme))
}

pub(crate) async fn serve_badges_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(BADGES_HTML, state.testing, theme))
}

pub(crate) async fn serve_next_gym_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(NEXT_GYM_HTML, state.testing, theme))
}

pub(crate) async fn serve_encounter_table_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(ENCOUNTER_TABLE_HTML, state.testing, theme))
}

pub(crate) async fn serve_money_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(MONEY_HTML, state.testing, theme))
}

pub(crate) async fn serve_playtime_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(PLAYTIME_HTML, state.testing, theme))
}

pub(crate) async fn serve_goals_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(GOALS_HTML, state.testing, theme))
}

pub(crate) async fn serve_vs_leader_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(VS_LEADER_HTML, state.testing, theme))
}

/// `GET /:index/goals/manage` — interactive goal management page for a specific slot.
pub(crate) async fn serve_goals_manage(
    State(state): State<WebState>,
    Path(_index): Path<usize>,
) -> Html<String> {
    Html(apply_page(GOALS_MANAGE_HTML, state.testing))
}

// ---------------------------------------------------------------------------
// New overlay handlers
// ---------------------------------------------------------------------------

/// POST /api/webhook/donation — ingest a StreamElements/Streamlabs donation alert.
///
/// Accepts generic JSON with a `type` field (`"donation"`, `"subscription"`, etc.)
/// and an optional `amount` (number). Fires a WebSocket overlay event to all
/// connected clients. If `heal_on_donation` is true in the query params, also
/// queues a `HealParty` command to all slots.
pub(crate) async fn api_donation_webhook(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let event_type = body.get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("donation")
        .to_string();
    let amount = body.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let donor  = body.get("name").or_else(|| body.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or("Anonymous")
        .to_string();

    let ws_event = serde_json::json!({
        "event":  "donation",
        "type":   event_type,
        "amount": amount,
        "donor":  donor,
    });
    let _ = state.tx.send(serde_json::to_string(&ws_event).unwrap_or_default());

    if params.get("heal_on_donation").is_some_and(|v| v == "true" || v == "1") {
        let slots = state.live_slots.lock_or_recover();
        for slot in slots.iter() {
            slot.command_queue.lock_or_recover().push_back(ClientMessage::HealParty);
        }
    }

    axum::Json(serde_json::json!({ "ok": true }))
}

/// POST /api/savefile — import a Gen III `.sav` snapshot.
///
/// The body must be the raw binary savefile bytes (Content-Type: application/octet-stream).
/// Extracts the player name from the save game section and seeds a new run.
/// Returns the detected player name and a success/error status.
pub(crate) async fn api_import_savefile(
    State(_state): State<WebState>,
    body: axum::body::Bytes,
) -> axum::Json<serde_json::Value> {
    if body.len() < 0x20000 {
        return axum::Json(serde_json::json!({ "error": "Savefile too small (expected ≥ 128 KiB)" }));
    }
    // Gen III save has two save slots of 57 KiB each (0xE000 bytes).
    // Each slot is 14 sections of 4096 bytes. Section 0 contains the
    // trainer info at offset 0: player_name (7 bytes, FF-terminated), gender, etc.
    let player_name = parse_gen3_player_name(&body).unwrap_or_else(|| "Unknown".to_string());

    axum::Json(serde_json::json!({
        "ok": true,
        "player_name": player_name,
        "size_bytes": body.len(),
        "note": "Savefile accepted. Start a new run to associate it.",
    }))
}

/// Parse the player name from a Gen III savefile.
///
/// Looks at slot 1 section 0 (offset 0x0000) for the trainer info block.
/// Player name is 7 bytes at offset 0, encoded in Gen III character encoding.
pub(crate) fn parse_gen3_player_name(sav: &[u8]) -> Option<String> {
    // Try section 0 of save slot 1 (offset 0) first, then slot 2 (offset 0xE000).
    for base in [0usize, 0xE000] {
        if base + 8 > sav.len() { continue; }
        let name_bytes = &sav[base..base + 7];
        let name: String = name_bytes.iter()
            .take_while(|&&b| b != 0xFF)
            .map(|&b| gen3_char(b))
            .collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

pub(crate) fn gen3_char(b: u8) -> char {
    // Partial Gen III character table for Latin letters/digits.
    match b {
        0xBB => 'A', 0xBC => 'B', 0xBD => 'C', 0xBE => 'D', 0xBF => 'E',
        0xC0 => 'F', 0xC1 => 'G', 0xC2 => 'H', 0xC3 => 'I', 0xC4 => 'J',
        0xC5 => 'K', 0xC6 => 'L', 0xC7 => 'M', 0xC8 => 'N', 0xC9 => 'O',
        0xCA => 'P', 0xCB => 'Q', 0xCC => 'R', 0xCD => 'S', 0xCE => 'T',
        0xCF => 'U', 0xD0 => 'V', 0xD1 => 'W', 0xD2 => 'X', 0xD3 => 'Y',
        0xD4 => 'Z',
        0xD5 => 'a', 0xD6 => 'b', 0xD7 => 'c', 0xD8 => 'd', 0xD9 => 'e',
        0xDA => 'f', 0xDB => 'g', 0xDC => 'h', 0xDD => 'i', 0xDE => 'j',
        0xDF => 'k', 0xE0 => 'l', 0xE1 => 'm', 0xE2 => 'n', 0xE3 => 'o',
        0xE4 => 'p', 0xE5 => 'q', 0xE6 => 'r', 0xE7 => 's', 0xE8 => 't',
        0xE9 => 'u', 0xEA => 'v', 0xEB => 'w', 0xEC => 'x', 0xED => 'y',
        0xEE => 'z',
        0xA1 => '0', 0xA2 => '1', 0xA3 => '2', 0xA4 => '3', 0xA5 => '4',
        0xA6 => '5', 0xA7 => '6', 0xA8 => '7', 0xA9 => '8', 0xAA => '9',
        _ => '?',
    }
}

pub(crate) const DEX_HTML: &str = include_str!("../dex.html");
pub(crate) const TYPECHART_HTML: &str = include_str!("../typechart.html");
pub(crate) const OVERLAY_JS: &str = include_str!("../overlay.js");

/// Shared client-side runtime loaded by every overlay/stat page.
/// `no-cache` so OBS browser sources pick up changes on the next load.
pub(crate) async fn serve_overlay_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        OVERLAY_JS,
    )
}

pub(crate) async fn serve_dex_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(DEX_HTML, state.testing, theme))
}

pub(crate) async fn serve_typechart_overlay(
    State(state): State<WebState>,
    Query(params): Query<HashMap<String, String>>,
) -> Html<String> {
    let theme = params.get("theme").map(String::as_str);
    Html(apply_page_with_theme(TYPECHART_HTML, state.testing, theme))
}
