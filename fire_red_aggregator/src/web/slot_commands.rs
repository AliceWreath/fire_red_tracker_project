//! Injection command endpoints (POST /api/slot/:index/...).

use super::*;

/// Returns the current time as a Unix timestamp (seconds).
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `POST /api/slot/:index/give_item` — inject an item into the player's bag.
///
/// Body: `{ "item_id": <u16>, "quantity": <u16 1–99> }`.
///
/// Queues a [`ClientMessage::GiveItem`] for the tracker in the given slot, which
/// writes the item directly into the in-memory items pocket via RetroArch's
/// `WRITE_CORE_MEMORY` command. The write happens asynchronously on the tracker
/// side; this endpoint returns 200 as soon as the command is enqueued.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but not connected to RetroArch.
pub(crate) async fn api_give_item(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let item_id = match body["item_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) if v > 0 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "item_id must be a positive u16".to_string(),
            );
        }
    };
    let quantity = match body["quantity"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
    {
        Some(v) if v > 0 && v <= 99 => v,
        _ => return (StatusCode::BAD_REQUEST, "quantity must be 1–99".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::GiveItem { item_id, quantity });
    let rom = fire_red_rom_buffer::get_rom();
    let item_name = fire_red_party_monitor::get_item_string_from_id(rom, item_id);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "give_item",
            "label": format!("Gave {quantity}× {item_name}"),
        }));
    (
        StatusCode::OK,
        format!("queued give_item item_id={item_id} quantity={quantity} for slot {index}"),
    )
}

/// `POST /api/slot/:index/make_shiny` — make a party Pokémon shiny in-memory.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Queues a [`ClientMessage::MakeShiny`] for the tracker, which patches the
/// Pokémon's stored OT Secret ID so the Gen III shiny formula holds.
/// Nature, ability, gender, and all other personality-derived properties are
/// preserved. Returns 200 as soon as the command is enqueued.
pub(crate) async fn api_make_shiny(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::MakeShiny { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "make_shiny",
            "label": format!("Made party[{party_position}] shiny"),
        }));
    (
        StatusCode::OK,
        format!("queued make_shiny party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/take_item` — remove an item from the player's bag.
///
/// Body: `{ "item_id": <u16>, "quantity": <u16 1–99> }`.
///
/// Queues a [`ClientMessage::TakeItem`] for the tracker. If the current stack
/// quantity is ≤ `quantity` the item is fully removed and the pocket is
/// compacted; otherwise only the quantity is decremented. Returns 200 as soon
/// as the command is enqueued.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but not connected to RetroArch.
pub(crate) async fn api_take_item(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let item_id = match body["item_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) if v > 0 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "item_id must be a positive u16".to_string(),
            );
        }
    };
    let quantity = match body["quantity"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
    {
        Some(v) if v > 0 && v <= 99 => v,
        _ => return (StatusCode::BAD_REQUEST, "quantity must be 1–99".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::TakeItem { item_id, quantity });
    let rom = fire_red_rom_buffer::get_rom();
    let item_name = fire_red_party_monitor::get_item_string_from_id(rom, item_id);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "take_item",
            "label": format!("Took {quantity}× {item_name}"),
        }));
    (
        StatusCode::OK,
        format!("queued take_item item_id={item_id} quantity={quantity} for slot {index}"),
    )
}

/// `POST /api/slot/:index/change_species` — change a party Pokémon's species.
///
/// Body: `{ "party_position": <u8 0–5>, "new_species": <u16 1–386> }`.
///
/// Queues a [`ClientMessage::ChangeSpecies`] for the tracker, which decrypts
/// the party Pokémon's data block, updates the species field in the Growth
/// substructure, recalculates the checksum, and re-encrypts. Personality,
/// nickname, moves, EVs, IVs, nature, ability, and gender are all preserved.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but not connected to RetroArch.
pub(crate) async fn api_change_species(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let new_species = match body["new_species"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
    {
        Some(v) if v > 0 && v <= 386 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "new_species must be 1–386".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeSpecies {
            party_position,
            new_species,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_species",
            "label": format!("Changed party[{party_position}] to species #{new_species}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_species party_position={party_position} new_species={new_species} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_ability` — switch a party Pokémon's ability slot.
///
/// Body: `{ "party_position": <u8 0–5>, "ability_slot": <u8 0 or 1> }`.
///
/// Queues a [`ClientMessage::ChangeAbility`] for the tracker. Sets or clears
/// bit 31 of the IV/egg/ability word in the Misc substructure; all other fields
/// (species, EVs, IVs, moves, nature, personality) are preserved. The checksum
/// is recalculated and the data block re-encrypted.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but not connected to RetroArch.
pub(crate) async fn api_change_ability(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let ability_slot = match body["ability_slot"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v <= 1 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "ability_slot must be 0 or 1".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeAbility {
            party_position,
            ability_slot,
        });
    let ability_label = if ability_slot == 0 {
        "primary"
    } else {
        "secondary"
    };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_ability",
            "label": format!("Party[{party_position}] → {ability_label} ability"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_ability party_position={party_position} ability_slot={ability_slot} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_gender` — change the gender of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "target_gender": <u8 0 or 1> }` where
/// 0 = male and 1 = female.
///
/// Adjusts only the low byte of the personality, preserving nature
/// (personality % 25). If the Pokémon is currently shiny only bytes that keep
/// the shiny formula satisfied are considered; the command is rejected (logged as
/// a warning) if none exist for the requested gender.  Genderless and
/// fixed-gender species are also rejected.
///
/// Returns:
/// - `200 OK` — command enqueued.
/// - `400 Bad Request` — missing or invalid body fields.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot exists but not connected to RetroArch.
pub(crate) async fn api_change_gender(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let target_gender = match body["target_gender"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v <= 1 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "target_gender must be 0 (male) or 1 (female)".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeGender {
            party_position,
            target_gender,
        });
    let gender_label = if target_gender == 0 { "male" } else { "female" };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_gender",
            "label": format!("Party[{party_position}] → {gender_label}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_gender party_position={party_position} target_gender={target_gender} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_nickname` — rename a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "nickname": <string, max 10 chars> }`.
///
/// The nickname is sent as UTF-8; the tracker converts it to GBA encoding and
/// silently drops unmapped characters. Only the 10-byte nickname field is written;
/// the encrypted data block (nature, IVs, etc.) is untouched.
pub(crate) async fn api_change_nickname(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let nickname = match body["nickname"].as_str() {
        Some(s) if !s.is_empty() && s.chars().count() <= 10 => s.to_string(),
        Some(s) if s.chars().count() > 10 => {
            return (
                StatusCode::BAD_REQUEST,
                "nickname must be 10 characters or fewer (Gen III buffer limit)".to_string(),
            );
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "nickname must be a non-empty string".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeNickname {
            party_position,
            nickname: nickname.clone(),
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_nickname",
            "label": format!("Renamed party[{party_position}] to \"{nickname}\""),
        }));
    (
        StatusCode::OK,
        format!("queued change_nickname party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/change_held_item` — set the held item of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "item_id": <u16> }`.
/// Use `item_id = 0` to remove the held item.
///
/// Decrypts the Growth substructure, writes the held-item field, recalculates
/// the checksum, and re-encrypts. All other data is preserved.
pub(crate) async fn api_change_held_item(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let item_id = match body["item_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "item_id must be a u16 (0 = remove)".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeHeldItem {
            party_position,
            item_id,
        });
    let label = if item_id == 0 {
        format!("Removed party[{party_position}] held item")
    } else {
        let rom = fire_red_rom_buffer::get_rom();
        let item_name = fire_red_party_monitor::get_item_string_from_id(rom, item_id);
        format!("party[{party_position}] now holds {item_name}")
    };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_held_item", "label": label,
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_held_item party_position={party_position} item_id={item_id} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/cure_status` — clear the status condition of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Writes 4 zero bytes to the status word (bytes 80–83 of the PartyPokemon
/// struct), clearing burn, sleep turn counter, paralysis, poison, freeze,
/// and Toxic stage in one write.
pub(crate) async fn api_cure_status(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::CureStatus { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "cure_status",
            "label": format!("Cured party[{party_position}] status"),
        }));
    (
        StatusCode::OK,
        format!("queued cure_status party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/change_nature` — change the nature of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "nature": <u8 0–24> }`.
///
/// Adjusts the low byte of the personality to satisfy `personality % 25 ==
/// nature` while preserving the current gender (for species with
/// personality-derived gender) and shiny status. Substructures are rearranged
/// when `personality % 24` changes. Returns `200` with an explanatory message
/// if no single low byte satisfies all constraints simultaneously.
///
/// Nature indices: 0=Hardy 1=Lonely 2=Brave 3=Adamant 4=Naughty 5=Bold
/// 6=Docile 7=Relaxed 8=Impish 9=Lax 10=Timid 11=Hasty 12=Serious 13=Jolly
/// 14=Naive 15=Modest 16=Mild 17=Quiet 18=Bashful 19=Rash 20=Calm 21=Gentle
/// 22=Sassy 23=Careful 24=Quirky
pub(crate) async fn api_change_nature(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let nature = match body["nature"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if v <= 24 => v,
        _ => return (StatusCode::BAD_REQUEST, "nature must be 0–24".to_string()),
    };
    const NATURE_NAMES: [&str; 25] = [
        "Hardy", "Lonely", "Brave", "Adamant", "Naughty", "Bold", "Docile", "Relaxed", "Impish",
        "Lax", "Timid", "Hasty", "Serious", "Jolly", "Naive", "Modest", "Mild", "Quiet", "Bashful",
        "Rash", "Calm", "Gentle", "Sassy", "Careful", "Quirky",
    ];
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeNature {
            party_position,
            target_nature: nature,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_nature",
            "label": format!("Party[{party_position}] → {} nature", NATURE_NAMES[nature as usize]),
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_nature party_position={party_position} nature={nature} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/restore_pp` — restore all move PP to current maximums.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Decrypts the Attacks and Growth substructures, computes maximum PP for each
/// equipped move slot (base PP + PP-Up bonus), and writes the result back.
/// Personality, nature, shiny status, and all other data are untouched.
pub(crate) async fn api_restore_pp(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::RestorePp { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "restore_pp",
            "label": format!("Restored party[{party_position}] PP"),
        }));
    (
        StatusCode::OK,
        format!("queued restore_pp party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_friendship` — set the friendship (happiness) byte.
///
/// Body: `{ "party_position": <u8 0–5>, "friendship": <u8 0–255> }`.
///
/// Common values: 0 = min (max Frustration damage), 255 = max (Happiness
/// evolutions trigger, max Return damage).
pub(crate) async fn api_set_friendship(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let friendship = match body["friendship"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "friendship must be 0–255".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetFriendship {
            party_position,
            friendship,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_friendship",
            "label": format!("party[{party_position}] friendship → {friendship}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued set_friendship party_position={party_position} friendship={friendship} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/change_move` — replace a move slot.
///
/// Body: `{ "party_position": <u8 0–5>, "slot": <u8 0–3>, "move_id": <u16> }`.
///
/// PP is set to the new move's maximum (base PP + existing PP-Up bonus).
/// Use `move_id = 0` to clear the slot.
pub(crate) async fn api_change_move(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let move_slot = match body["slot"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if v <= 3 => v,
        _ => return (StatusCode::BAD_REQUEST, "slot must be 0–3".to_string()),
    };
    let move_id = match body["move_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) if v == 0 || v <= 354 => v,
        Some(_) => return (StatusCode::BAD_REQUEST, "move_id must be 0 (clear) or 1–354".to_string()),
        None => return (StatusCode::BAD_REQUEST, "move_id must be a u16".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ChangeMove {
            party_position,
            slot: move_slot,
            move_id,
        });
    let label = if move_id == 0 {
        format!("Cleared party[{party_position}] move slot {move_slot}")
    } else {
        format!("party[{party_position}] move {move_slot} → move_id {move_id}")
    };
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "change_move", "label": label,
        }));
    (
        StatusCode::OK,
        format!(
            "queued change_move party_position={party_position} slot={move_slot} move_id={move_id} for slot {index}"
        ),
    )
}

/// Shared stat-field parser for IV/EV handlers — extracts `hp/atk/def/spd/spa/spdef`
/// from a JSON body, returning an error string on the first missing or invalid field.
pub(crate) fn parse_six_stats(body: &serde_json::Value) -> Result<(u8, u8, u8, u8, u8, u8), String> {
    let mut vals = [0u8; 6];
    for (i, key) in ["hp", "atk", "def", "spd", "spa", "spdef"]
        .iter()
        .enumerate()
    {
        vals[i] = body[key]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| format!("{key} must be 0–255"))?;
    }
    Ok((vals[0], vals[1], vals[2], vals[3], vals[4], vals[5]))
}

/// `POST /api/slot/:index/set_ivs` — set all six IVs of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
/// Values are clamped to 31 by the tracker. Egg and ability bits are preserved.
pub(crate) async fn api_set_ivs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetIvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_ivs",
            "label": format!("party[{party_position}] IVs → {hp}/{atk}/{def}/{spd}/{spa}/{spdef}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_ivs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/increase_ivs` — add to each IV, clamping at 31.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
pub(crate) async fn api_increase_ivs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::IncreaseIvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events.lock_or_recover().push_back(serde_json::json!({
        "at": now_secs(), "kind": "increase_ivs",
        "label": format!("party[{party_position}] IVs +{hp}/+{atk}/+{def}/+{spd}/+{spa}/+{spdef}"),
    }));
    (
        StatusCode::OK,
        format!("queued increase_ivs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_evs` — set all six EVs of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8 0–255>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
/// The 510-total game cap is not enforced. Contest-condition bytes are preserved.
pub(crate) async fn api_set_evs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetEvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_evs",
            "label": format!("party[{party_position}] EVs → {hp}/{atk}/{def}/{spd}/{spa}/{spdef}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_evs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/increase_evs` — add to each EV, clamping at 255.
///
/// Body: `{ "party_position": <u8 0–5>, "hp": <u8>, "atk": <u8>, "def": <u8>,
/// "spd": <u8>, "spa": <u8>, "spdef": <u8> }`.
pub(crate) async fn api_increase_evs(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let (hp, atk, def, spd, spa, spdef) = match parse_six_stats(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::IncreaseEvs {
            party_position,
            hp,
            atk,
            def,
            spd,
            spa,
            spdef,
        });
    slot.injection_events.lock_or_recover().push_back(serde_json::json!({
        "at": now_secs(), "kind": "increase_evs",
        "label": format!("party[{party_position}] EVs +{hp}/+{atk}/+{def}/+{spd}/+{spa}/+{spdef}"),
    }));
    (
        StatusCode::OK,
        format!("queued increase_evs party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/restore_hp` — restore a party Pokémon's current HP to maximum.
///
/// Body: `{ "party_position": <u8 0–5> }`.
///
/// Reads the calculated max-HP word from PartyPokemon offset 88–89 and writes
/// it to offset 86–87. No encrypted data block is touched.
pub(crate) async fn api_restore_hp(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::RestoreHp { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "restore_hp",
            "label": format!("Restored party[{party_position}] HP to full"),
        }));
    (
        StatusCode::OK,
        format!("queued restore_hp party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/heal_party` — restore HP and cure status for the whole party.
///
/// No request body required.
///
/// The tracker reuses a single UDP socket and processes all six party slots in
/// one pass: zeroes the status word and writes max HP to current HP for every
/// occupied slot.
pub(crate) async fn api_heal_party(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::HealParty);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "heal_party",
            "label": "Full party heal (HP + status)",
        }));
    (
        StatusCode::OK,
        format!("queued heal_party for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_exp` — set the experience points of a party Pokémon.
///
/// Body: `{ "party_position": <u8 0–5>, "exp": <u32> }`.
/// Updates the Growth substructure; the level byte is not changed.
pub(crate) async fn api_set_exp(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let exp = match body["exp"].as_u64().and_then(|v| u32::try_from(v).ok()) {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "exp must be a u32".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetExp {
            party_position,
            exp,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_exp",
            "label": format!("party[{party_position}] exp → {exp}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_exp party_position={party_position} exp={exp} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_level` — set the level of a party Pokémon (1–100).
///
/// Body: `{ "party_position": <u8 0–5>, "level": <u8 1–100> }`.
/// Writes both the level byte and updates the experience in the Growth
/// substructure to the Gen III minimum for the target level.
pub(crate) async fn api_set_level(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let level = match body["level"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if (1..=100).contains(&v) => v,
        _ => return (StatusCode::BAD_REQUEST, "level must be 1–100".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetLevel {
            party_position,
            level,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_level",
            "label": format!("party[{party_position}] → level {level}"),
        }));
    (
        StatusCode::OK,
        format!("queued set_level party_position={party_position} level={level} for slot {index}"),
    )
}

/// `POST /api/slot/:index/learn_move` — add a move to the first empty move slot.
///
/// Body: `{ "party_position": <u8 0–5>, "move_id": <u16> }`.
/// No-op if the Pokémon already knows the move or all four slots are occupied.
pub(crate) async fn api_learn_move(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let move_id = match body["move_id"].as_u64().and_then(|v| u16::try_from(v).ok()) {
        Some(v) if v > 0 && v <= 354 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "move_id must be 1–354".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::LearnMove {
            party_position,
            move_id,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "learn_move",
            "label": format!("party[{party_position}] learn move_id={move_id}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued learn_move party_position={party_position} move_id={move_id} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/forget_move` — clear a move slot and compact.
///
/// Body: `{ "party_position": <u8 0–5>, "slot": <u8 0–3> }`.
/// Clears the move at `slot` and shifts subsequent moves left to fill the gap.
pub(crate) async fn api_forget_move(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let move_slot = match body["slot"].as_u64().and_then(|v| u8::try_from(v).ok()) {
        Some(v) if v < 4 => v,
        _ => return (StatusCode::BAD_REQUEST, "slot must be 0–3".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::ForgetMove {
            party_position,
            slot: move_slot,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "forget_move",
            "label": format!("party[{party_position}] forget slot {move_slot}"),
        }));
    (
        StatusCode::OK,
        format!(
            "queued forget_move party_position={party_position} slot={move_slot} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/set_pokerus` — infect a party Pokémon with Pokérus.
///
/// Body: `{ "party_position": <u8 0–5> }`.
/// Sets Pokérus to strain 1, 4 days remaining. No-op if already actively infected.
pub(crate) async fn api_set_pokerus(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetPokerus { party_position });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_pokerus",
            "label": format!("party[{party_position}] infected with Pokérus"),
        }));
    (
        StatusCode::OK,
        format!("queued set_pokerus party_position={party_position} for slot {index}"),
    )
}

/// `POST /api/slot/:index/set_pp_ups` — set PP-Up counts for all four move slots.
///
/// Body: `{ "party_position": <u8 0–5>, "pp0": <u8 0–3>, "pp1": <u8 0–3>,
///          "pp2": <u8 0–3>, "pp3": <u8 0–3> }`.
/// Sets the PP-Up bonus for each slot and refills current PP to the new maximum.
pub(crate) async fn api_set_pp_ups(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let parse_pp = |key: &str| -> Option<u8> {
        body[key].as_u64().and_then(|v| u8::try_from(v).ok()).filter(|&v| v <= 3)
    };
    let pp0 = match parse_pp("pp0") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp0 must be 0–3".to_string()),
    };
    let pp1 = match parse_pp("pp1") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp1 must be 0–3".to_string()),
    };
    let pp2 = match parse_pp("pp2") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp2 must be 0–3".to_string()),
    };
    let pp3 = match parse_pp("pp3") {
        Some(v) => v,
        None => return (StatusCode::BAD_REQUEST, "pp3 must be 0–3".to_string()),
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::SetPpUps {
            party_position,
            pp0,
            pp1,
            pp2,
            pp3,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "set_pp_ups",
            "label": format!(
                "party[{party_position}] PP-Ups → ({pp0},{pp1},{pp2},{pp3})"
            ),
        }));
    (
        StatusCode::OK,
        format!(
            "queued set_pp_ups party_position={party_position} \
             pp0={pp0},pp1={pp1},pp2={pp2},pp3={pp3} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/revive_pokemon` — revive a dead Pokémon into a party slot.
///
/// Body: `{ "party_position": <u8 0–5>, "personality": <u32> }`.
/// Looks up the Pokémon by `personality` in the current run's `dead_pokemon`
/// table and writes it at `party_position` with 1 HP.
pub(crate) async fn api_revive_pokemon(
    State(state): State<WebState>,
    Path(index): Path<usize>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    let party_position = match body["party_position"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
    {
        Some(v) if v < 6 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "party_position must be 0–5".to_string(),
            );
        }
    };
    let personality = match body["personality"].as_u64().and_then(|v| u32::try_from(v).ok()) {
        Some(v) if v != 0 => v,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "personality must be a non-zero u32".to_string(),
            );
        }
    };
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::RevivePokemon {
            party_position,
            personality,
        });
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "revive_pokemon",
            "label": format!(
                "party[{party_position}] ← revive personality={personality:#010x}"
            ),
        }));
    (
        StatusCode::OK,
        format!(
            "queued revive_pokemon party_position={party_position} \
             personality={personality:#010x} for slot {index}"
        ),
    )
}

/// `POST /api/slot/:index/undo` — revert the last injection command for the given slot.
///
/// Sends [`ClientMessage::UndoLastCommand`] to the slot's game loop, which writes the
/// bytes that were captured before the last `write_to_retroarch` call back to
/// RetroArch memory.  No-op if no injection command has been executed since the
/// slot was started.
///
/// - `200 OK` — command enqueued.
/// - `403 Forbidden` — injection commands are disabled.
/// - `404 Not Found` — slot index out of range.
/// - `503 Service Unavailable` — slot not connected.
pub(crate) async fn api_undo(
    State(state): State<WebState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    if !state.allow_injections {
        return (
            StatusCode::FORBIDDEN,
            "injection commands are disabled".to_string(),
        );
    }
    let slots = state.live_slots.lock_or_recover().clone();
    let slot = match slots.get(index) {
        Some(s) => s.clone(),
        None => return (StatusCode::NOT_FOUND, "slot index out of range".to_string()),
    };
    if slot.state.lock_or_recover().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "slot not connected".to_string(),
        );
    }
    slot.command_queue
        .lock_or_recover()
        .push_back(ClientMessage::UndoLastCommand);
    slot.injection_events
        .lock_or_recover()
        .push_back(serde_json::json!({
            "at": now_secs(), "kind": "undo",
            "label": "undo last command",
        }));
    (
        StatusCode::OK,
        format!("queued undo for slot {index}"),
    )
}
