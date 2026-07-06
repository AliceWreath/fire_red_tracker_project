//! Live per-slot JSON endpoints and command dispatch.

use super::*;

/// Returns the full current state as a JSON array of slot objects — same
/// payload the WebSocket would push on the next tick.
pub(crate) async fn api_state(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let json = state.tx.borrow().clone();
    let raw = if json.is_empty() { "[]".to_string() } else { json };
    let body = filter_slots_for_user(&raw, user.id).await;
    ([(header::CONTENT_TYPE, "application/json")], body)
}

/// Returns a single slot object by zero-based index, or 404 if out of range.
pub(crate) async fn api_slot(State(state): State<WebState>, Path(index): Path<usize>) -> impl IntoResponse {
    let json = state.tx.borrow().clone();
    let slots: serde_json::Value =
        serde_json::from_str(&json).unwrap_or(serde_json::Value::Array(vec![]));
    match slots.get(index) {
        Some(slot) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            slot.to_string(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "slot index out of range").into_response(),
    }
}

/// `GET /api/slot/:index/odds` — wildmon encounter table for the given slot's current map.
///
/// Returns the full [`WildPokemonHeader`] for whichever map the tracker in that
/// slot is currently on, broken down by encounter type (land, water, rock-smash,
/// fishing). Each encounter entry includes species id, min/max level, and the
/// party-wide encounter rate for the type.
///
/// Returns `{ "error": "..." }` if the slot is out of range, disconnected, or
/// the current map has no wild encounters.
pub(crate) async fn api_slot_odds(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return axum::Json(serde_json::json!({ "error": "slot index out of range" })),
    };
    let gs = match slot.state.lock_or_recover().clone() {
        Some(gs) => gs,
        None => return axum::Json(serde_json::json!({ "error": "slot not connected" })),
    };
    let h = &gs.encounters;
    let make_list = |info: &fire_red_pokemon_data::WildPokemonInfo| -> serde_json::Value {
        if info.encounter_rate == 0 {
            return serde_json::Value::Null;
        }
        serde_json::json!({
            "encounter_rate": info.encounter_rate,
            "slots": info.wild_pokemon_list.iter().map(|p| serde_json::json!({
                "species":    p.species,
                "min_level":  p.min_level,
                "max_level":  p.max_level,
            })).collect::<Vec<_>>()
        })
    };
    axum::Json(serde_json::json!({
        "map_group": h.map_group,
        "map_name":  h.map_num,
        "land":        make_list(&h.land_mon_encounters),
        "water":       make_list(&h.water_mon_encounters),
        "rock_smash":  make_list(&h.rock_smash_encounters),
        "fishing":     make_list(&h.fishing_encounters),
    }))
}

/// Returns a plain-text one-line summary of a tracker slot, suitable for chat
/// bots or stream commands. Format: `"<Player> — <HP>/<MaxHP> — <MapName>"`.
/// Returns `"Slot <n> not found"` or `"Slot <n> not connected"` on error.
pub(crate) async fn api_bot_summary(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(index): Path<usize>,
) -> String {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return format!("Slot {index} not found"),
    };
    if let Some(rid) = slot.db.as_ref().and_then(|db| db.get_run_id()) {
        let uid = user.id;
        let can = tokio::task::spawn_blocking(move || {
            fire_red_database::user_can_access_run(rid, uid)
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false);
        if !can {
            return format!("Slot {index} not found");
        }
    }
    let gs = match slot.state.lock_or_recover().clone() {
        Some(gs) => gs,
        None => return format!("Slot {index} not connected"),
    };
    let player = &gs.player_name;
    let map = if gs.zone_name.is_empty() {
        "Unknown location"
    } else {
        &gs.zone_name
    };
    let (hp, max_hp) = gs.party.first().map(|p| (p.hp, p.max_hp)).unwrap_or((0, 0));
    format!("{player} — {hp}/{max_hp} HP — {map}")
}

/// `GET /api/slot/:index/bag` — bag pockets JSON for a specific tracker slot.
pub(crate) async fn api_bag(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return axum::Json(serde_json::json!({ "error": "slot index out of range" })),
    };
    match slot.bag_data.lock_or_recover().clone() {
        Some(pockets) => axum::Json(serde_json::json!({
            "items":     pockets.items.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
            "key_items": pockets.key_items.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
            "balls":     pockets.balls.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
            "tms":       pockets.tms.iter().map(|s| serde_json::json!({ "item_id": s.item_id, "quantity": s.quantity })).collect::<Vec<_>>(),
        })),
        None => axum::Json(serde_json::json!({ "error": "bag data not yet available" })),
    }
}

pub(crate) async fn api_slot_ev_progress(
    State(state): State<WebState>,
    Path(slot_index): Path<usize>,
) -> axum::Json<serde_json::Value> {
    let slots = state.live_slots.lock_or_recover();
    let Some(slot) = slots.get(slot_index) else {
        return axum::Json(serde_json::json!({ "error": "Slot index out of range" }));
    };
    let game_state = slot.state.lock_or_recover();
    let Some(ref gs) = *game_state else {
        return axum::Json(serde_json::json!({ "error": "Slot not connected" }));
    };
    let ev_list: Vec<serde_json::Value> = gs.party.iter()
        .filter(|p| p.box_mon.secure.growth.species != 0)
        .map(|p| {
            let ev = &p.box_mon.secure.ev_condition;
            let total = ev.hp_ev as u32
                + ev.attack_ev as u32
                + ev.defense_ev as u32
                + ev.speed_ev as u32
                + ev.sp_attack_ev as u32
                + ev.sp_defense_ev as u32;
            let remaining_total = 510u32.saturating_sub(total);
            serde_json::json!({
                "personality": p.box_mon.personality,
                "nickname":    p.box_mon.nickname_string,
                "species":     p.box_mon.secure.growth.species_string,
                "hp":         ev.hp_ev,
                "attack":     ev.attack_ev,
                "defense":    ev.defense_ev,
                "speed":      ev.speed_ev,
                "sp_attack":  ev.sp_attack_ev,
                "sp_defense": ev.sp_defense_ev,
                "total":      total,
                "remaining":  remaining_total,
                "hp_capped":         ev.hp_ev >= 255,
                "attack_capped":     ev.attack_ev >= 255,
                "defense_capped":    ev.defense_ev >= 255,
                "speed_capped":      ev.speed_ev >= 255,
                "sp_attack_capped":  ev.sp_attack_ev >= 255,
                "sp_defense_capped": ev.sp_defense_ev >= 255,
                "fully_trained": total >= 510,
            })
        })
        .collect();
    axum::Json(serde_json::json!(ev_list))
}

/// Broadcasts a command to all active game slots.
///
/// Supported commands (no request body needed — suitable for Stream Deck buttons):
///
/// | `cmd`       | Effect                                                   |
/// |-------------|----------------------------------------------------------|
/// | `end_run`   | End the active run for every connected player.           |
/// | `new_run`   | Start a new run for every connected player.              |
/// | `heal_all`  | Heal HP/PP/status of every party Pokémon for all slots.  |
pub(crate) async fn api_command(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    Path(cmd): Path<String>,
) -> impl IntoResponse {
    let msg = match cmd.as_str() {
        "end_run"  => ClientMessage::EndRun,
        "new_run"  => ClientMessage::NewRun,
        "heal_all" => ClientMessage::HealParty,
        other => return (StatusCode::BAD_REQUEST, format!("Unknown command: {other}")),
    };
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    let slots = state.live_slots.lock_or_recover().clone();
    let mut count = 0usize;
    for slot in &slots {
        let run_id = slot.db.as_ref().and_then(|db| db.get_run_id());
        if run_id.is_some_and(|rid| accessible.contains(&rid)) {
            slot.command_queue.lock_or_recover().push_back(msg.clone());
            count += 1;
        }
    }
    (
        StatusCode::OK,
        format!("Command '{cmd}' sent to {count} slot(s)"),
    )
}

/// Sends a no-body command to a single tracker slot. Designed for Stream Deck
/// buttons where a separate body editor is inconvenient.
///
/// Supported per-slot commands:
///
/// | `cmd`         | Effect                                                  |
/// |---------------|---------------------------------------------------------|
/// | `heal_party`  | Heal HP/PP/status of all party Pokémon for this slot.   |
pub(crate) async fn api_slot_command(
    State(state): State<WebState>,
    Path((index, cmd)): Path<(usize, String)>,
) -> impl IntoResponse {
    let msg = match cmd.as_str() {
        "heal_party" => ClientMessage::HealParty,
        other => return (StatusCode::BAD_REQUEST, format!("Unknown slot command: {other}")),
    };
    let slots = state.live_slots.lock_or_recover();
    let Some(slot) = slots.get(index) else {
        return (StatusCode::NOT_FOUND, format!("No slot at index {index}"));
    };
    slot.command_queue.lock_or_recover().push_back(msg);
    (StatusCode::OK, format!("Command '{cmd}' sent to slot {index}"))
}

/// Runs arbitrary SQL against the database and returns results as JSON.
///
/// Restricted to loopback connections — returns 403 for any remote caller.
/// `POST /api/slot/:index/refresh_rom` — force-re-download the cached ROM for a
/// direct-mode slot from its RetroArch instance.
///
/// Deletes the cached `.gba` file, re-fetches the full 16 MiB ROM from RetroArch
/// over UDP (takes 5–15 s on a typical LAN), and replaces the in-memory ROM
/// buffer used by the sprite loader so new sprites are decoded from the fresh ROM.
///
/// Returns 400 if the slot is not in direct mode, 404 if the slot index is out of
/// range, or 503 if RetroArch is unreachable.
pub(crate) async fn api_refresh_rom(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => {
            return (StatusCode::NOT_FOUND, "slot index out of range".to_string())
                .into_response()
        }
    };
    let host_port = match &slot.direct_host {
        Some(hp) => hp.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "slot is not in direct mode — ROM refresh is only available for \
                 direct-mode connections"
                    .to_string(),
            )
                .into_response()
        }
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(55355)),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("malformed direct_host: {}", host_port),
            )
                .into_response()
        }
    };

    let rom_bytes_arc    = slot.rom_bytes.clone();
    let rom_identity_arc = slot.rom_identity.clone();
    let known_species    = slot.known_species.clone();
    let pending_textures = slot.pending_textures.clone();
    let sprite_cache     = slot.sprite_cache.clone();
    let game_encounters  = slot.game_encounters.clone();

    let run_id = slot.run_id;
    let result = tokio::task::spawn_blocking(move || {
        crate::rom_fetch::force_fetch_rom(&host, port, run_id)
            .and_then(|path| std::fs::read(&path).map_err(|e| e.to_string()))
    })
    .await;

    match result {
        Ok(Ok(bytes)) => {
            let new_id  = crate::direct::rom_identity_from_bytes(&bytes);
            let old_id  = rom_identity_arc.lock_or_recover().clone();
            let changed = old_id != new_id && !old_id.is_empty();

            if changed {
                tracing::info!(
                    "ROM force-refresh: slot {} — ROM changed from \"{}\" to \"{}\"",
                    index, old_id, new_id
                );
            } else {
                tracing::info!(
                    "ROM force-refresh: slot {} — same ROM identity \"{}\" (re-fetched bytes)",
                    index, new_id
                );
            }

            // Update ROM bytes and identity.
            *rom_identity_arc.lock_or_recover() = new_id.clone();
            *rom_bytes_arc.lock_or_recover()    = bytes;

            // Clear sprite pipeline so sprites are re-decoded from the new ROM.
            known_species.lock_or_recover().clear();
            pending_textures.lock_or_recover().clear();
            if let Some(cache_arc) = sprite_cache.lock_or_recover().as_ref() {
                cache_arc.lock_or_recover().clear();
            }

            // Reset the game loop's encounter-table cache so stale area data
            // from the old ROM is evicted immediately.
            if let Some(enc_arc) = game_encounters.lock_or_recover().as_ref() {
                *enc_arc.lock_or_recover() =
                    fire_red_pokemon_data::WildPokemonHeader::default();
            }

            let body = if changed {
                format!("ROM changed: {} → {}", old_id, new_id)
            } else {
                format!("ROM refreshed ({})", new_id)
            };
            (StatusCode::OK, body).into_response()
        }
        Ok(Err(e)) => {
            tracing::error!("ROM force-refresh failed for slot {}: {}", index, e);
            (StatusCode::SERVICE_UNAVAILABLE, e).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
