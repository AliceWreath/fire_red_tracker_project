//! JSON DTOs sent to overlay pages and API clients.

#[derive(serde::Serialize, Clone)]
pub(crate) struct RunSummaryDto {
    pub(crate) run_id: u32,
    pub(crate) player_name: String,
    pub(crate) started_at: String,
    pub(crate) ended_at: Option<String>,
    pub(crate) deaths: usize,
    pub(crate) caught: usize,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct DbEncounterDto {
    pub(crate) species_name: String,
    pub(crate) level: u8,
    pub(crate) caught: bool,
    pub(crate) is_shiny: bool,
    pub(crate) encountered_at: String,
    pub(crate) area: String,
    pub(crate) sprite: Option<String>,
    pub(crate) map_group: u8,
    pub(crate) map_name: u8,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct SlotDto {
    pub(crate) label: String,
    pub(crate) connected: bool,
    pub(crate) db_connected: bool,
    pub(crate) active_run_id: Option<u32>,
    pub(crate) run_summary: Option<RunSummaryDto>,
    pub(crate) db_encounters: Vec<DbEncounterDto>,
    pub(crate) badges: Vec<bool>,
    pub(crate) next_gym: Option<GymDto>,
    pub(crate) party: Vec<MemberDto>,
    pub(crate) encounters: Vec<EncounterGroupDto>,
    pub(crate) dead: Vec<DeadMonDto>,
    pub(crate) caught: Vec<CaughtMonDto>,
    pub(crate) box_pokemon: Vec<BoxMonDto>,
    /// map_group of the current wild-encounter zone (0 if no encounter area).
    pub(crate) current_map_group: u8,
    /// map_name of the current wild-encounter zone (0 if no encounter area).
    pub(crate) current_map_name: u8,
    /// Human-readable name for the current zone, empty when not in a wild area.
    pub(crate) current_zone_name: String,
    /// Encounters from the most recently completed run, for cross-run hints.
    pub(crate) prev_run_encounters: Vec<DbEncounterDto>,
    /// Elite 4 + Champion defeat flags: indices 0–4 = Lorelei, Bruno, Agatha, Lance, Blue.
    pub(crate) e4_progress: Vec<bool>,
    /// True when all 8 badges and all 5 Elite 4 members (incl. Champion) are defeated.
    pub(crate) game_cleared: bool,
    /// Injection events (give/take item, make shiny, etc.) queued since the last
    /// tick. Drained on every broadcast; alerts.html shows toasts for each entry.
    pub(crate) injection_events: Vec<serde_json::Value>,
    /// Current Pokédollar balance (decrypted from SaveBlock1).
    pub(crate) money: u32,
    /// Live battle damage panel (every party member's moves vs the current
    /// enemy); `None` outside battle. Rendered by /:index/damage_calc.
    pub(crate) damage_panel: Option<fire_red_states::DamagePanel>,
    /// In-game save-file play time: hours component.
    pub(crate) play_time_hours: u16,
    /// In-game save-file play time: minutes component (0–59).
    pub(crate) play_time_minutes: u8,
    /// In-game save-file play time: seconds component (0–59).
    pub(crate) play_time_seconds: u8,
    /// User-defined run goals from the `run_goals` DB table.
    pub(crate) goals: Vec<GoalDto>,
    /// Upcoming gym leader's full party read from ROM (randomizer-aware).
    pub(crate) leader_party: Vec<LeaderPartyMonDto>,
    /// Owner-pinned display column for this slot's active run (1 = leftmost),
    /// or `None` if unpinned (falls back to in-game player position). Lets the
    /// overview page show/edit the override via `PATCH /api/run/:id/slot_index`.
    pub(crate) pinned_slot_index: Option<u8>,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct DeadMonDto {
    pub(crate) nickname: String,
    pub(crate) species_name: String,
    pub(crate) level: u8,
    pub(crate) nature: String,
    pub(crate) shiny: bool,
    pub(crate) soul_link: bool,
    pub(crate) died_at: String,
    pub(crate) gender: u8,
    pub(crate) max_hp: u16,
    pub(crate) attack: u16,
    pub(crate) defense: u16,
    pub(crate) speed: u16,
    pub(crate) sp_attack: u16,
    pub(crate) sp_defense: u16,
    pub(crate) iv_hp: u8,
    pub(crate) iv_atk: u8,
    pub(crate) iv_def: u8,
    pub(crate) iv_spe: u8,
    pub(crate) iv_spa: u8,
    pub(crate) iv_spd: u8,
    pub(crate) ev_hp: u8,
    pub(crate) ev_atk: u8,
    pub(crate) ev_def: u8,
    pub(crate) ev_spe: u8,
    pub(crate) ev_spa: u8,
    pub(crate) ev_spd: u8,
    pub(crate) sprite: Option<String>,
    pub(crate) killed_by: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct CaughtMonDto {
    pub(crate) nickname: String,
    pub(crate) species_name: String,
    pub(crate) level: u8,
    pub(crate) nature: String,
    pub(crate) shiny: bool,
    pub(crate) caught_at: String,
    pub(crate) met_location_name: String,
    pub(crate) gender: u8,
    pub(crate) iv_hp: u8,
    pub(crate) iv_atk: u8,
    pub(crate) iv_def: u8,
    pub(crate) iv_spe: u8,
    pub(crate) iv_spa: u8,
    pub(crate) iv_spd: u8,
    pub(crate) sprite: Option<String>,
    /// GBA personality value — exposed so the override manager can identify mons.
    pub(crate) personality: u32,
    /// True when this Pokémon has a death record or is a soul-link casualty.
    pub(crate) dead: bool,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct BoxMonDto {
    pub(crate) box_index: u8,
    pub(crate) slot_index: u8,
    pub(crate) species_name: String,
    pub(crate) nickname: String,
    pub(crate) is_shiny: bool,
    pub(crate) nature: String,
    pub(crate) is_egg: bool,
    pub(crate) iv_hp: u8,
    pub(crate) iv_atk: u8,
    pub(crate) iv_def: u8,
    pub(crate) iv_spe: u8,
    pub(crate) iv_spa: u8,
    pub(crate) iv_spd: u8,
    /// `0` = male, `1` = female, `2` = genderless.
    pub(crate) gender: u8,
    pub(crate) sprite: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct EncounterGroupDto {
    pub(crate) label: String,
    /// Party-wide encounter rate (0–255) for this encounter type.
    pub(crate) encounter_rate: u8,
    pub(crate) mons: Vec<EncounterMonDto>,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct EncounterMonDto {
    pub(crate) species_name: String,
    pub(crate) min_level: u8,
    pub(crate) max_level: u8,
    pub(crate) sprite: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct GymDto {
    pub(crate) leader: String,
    pub(crate) city: String,
    pub(crate) max_level: u8,
    /// Primary type ID of the gym leader / Elite 4 member (Gen III ID, 0–16).
    /// Used by overlay pages to pre-highlight relevant matchups.
    pub(crate) type_id: u8,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct GoalDto {
    pub(crate) id: i32,
    pub(crate) text: String,
    pub(crate) completed: bool,
}

/// One Pokémon on the upcoming gym leader's team, read directly from ROM
/// so randomizer runs show the actual (post-randomization) team.
#[derive(serde::Serialize, Clone)]
pub(crate) struct LeaderPartyMonDto {
    pub(crate) species_name: String,
    pub(crate) level: u8,
    pub(crate) moves: [String; 4],
    pub(crate) type1: u8,
    pub(crate) type2: u8,
    pub(crate) sprite: Option<String>,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct SoulLinkPartnerDto {
    pub(crate) nickname: String,
    pub(crate) player: String,
}

#[derive(serde::Serialize, Clone)]
pub(crate) struct MemberDto {
    pub(crate) nickname: String,
    pub(crate) species_name: String,
    pub(crate) level: u8,
    pub(crate) hp: u16,
    pub(crate) max_hp: u16,
    pub(crate) exp: u32,
    pub(crate) nature: String,
    pub(crate) shiny: bool,
    pub(crate) dead: bool,
    pub(crate) soul_link_kill: bool,
    pub(crate) soul_link_partner: Option<SoulLinkPartnerDto>,
    pub(crate) died_at: Option<String>,
    pub(crate) attack: u16,
    pub(crate) defense: u16,
    pub(crate) speed: u16,
    pub(crate) sp_attack: u16,
    pub(crate) sp_defense: u16,
    /// `0` = male, `1` = female, `2` = genderless.
    pub(crate) gender: u8,
    pub(crate) ability: String,
    pub(crate) held_item: String,
    pub(crate) held_item_id: u16,
    pub(crate) growth_rate: String,
    pub(crate) ev_hp: u8,
    pub(crate) ev_atk: u8,
    pub(crate) ev_def: u8,
    pub(crate) ev_spe: u8,
    pub(crate) ev_spa: u8,
    pub(crate) ev_spd: u8,
    pub(crate) iv_hp: u8,
    pub(crate) iv_atk: u8,
    pub(crate) iv_def: u8,
    pub(crate) iv_spe: u8,
    pub(crate) iv_spa: u8,
    pub(crate) iv_spd: u8,
    /// Base64 PNG data URI for the sprite, e.g. `data:image/png;base64,...`.
    /// `None` while the sprite is still in transit from the tracker server.
    pub(crate) sprite: Option<String>,
    /// Unique personality value — used by the overlay to detect death transitions.
    pub(crate) personality: u32,
    /// Status condition bitmask (Gen III encoding):
    /// bits 0-2 = sleep turns, bit 3 = PSN, bit 4 = BRN, bit 5 = FRZ, bit 6 = PAR, bit 7 = TOX.
    pub(crate) status: u32,
    /// Current move names (empty string for empty slots).
    pub(crate) moves: [String; 4],
    /// Current PP for each move slot.
    pub(crate) pp: [u8; 4],
    /// Gen III type ID for the species' first type (0=Normal … 16=Dark).
    pub(crate) type1: u8,
    /// Gen III type ID for the species' second type; equals `type1` for mono-type species.
    pub(crate) type2: u8,
}

/// Display name for where a caught Pokémon was obtained.
///
/// Prefers the human-readable string recorded at catch time, but falls back
/// to translating the raw `met_location` byte when the stored string is
/// empty or is one of the old numeric `"G·N"` designations written before
/// towns and interiors could be named (fixed in v0.9.111).
pub(crate) fn display_met_location(stored: &str, met_location: u8) -> String {
    let is_numeric_designation = {
        let mut parts = stored.split('\u{B7}');
        matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(a), Some(b), None)
                if !a.is_empty() && !b.is_empty()
                    && a.bytes().all(|c| c.is_ascii_digit())
                    && b.bytes().all(|c| c.is_ascii_digit())
        )
    };
    if stored.is_empty() || is_numeric_designation {
        let met = fire_red_location_names::location_name(met_location);
        if met != "Unknown Location" && met != "—" || stored.is_empty() {
            return met.to_string();
        }
    }
    stored.to_string()
}

#[cfg(test)]
mod met_location_tests {
    use super::display_met_location;

    #[test]
    fn stored_name_wins_when_meaningful() {
        assert_eq!(display_met_location("Route 1", 0x58), "Route 1");
        assert_eq!(display_met_location("Mt. Moon B2F", 0x7F), "Mt. Moon B2F");
    }

    #[test]
    fn numeric_designation_is_replaced_by_met_byte() {
        // Old rows recorded in unnamed maps stored "group·map" numbers.
        assert_eq!(display_met_location("4\u{B7}3", 0x58), "Pallet Town");
        assert_eq!(display_met_location("3\u{B7}1", 0x59), "Viridian City");
    }

    #[test]
    fn empty_stored_falls_back_to_met_byte() {
        assert_eq!(display_met_location("", 0x65), "Route 1");
        assert_eq!(display_met_location("", 0xFE), "Unknown Location");
    }

    #[test]
    fn numeric_designation_kept_when_met_byte_is_unknown() {
        assert_eq!(display_met_location("9\u{B7}77", 0xFE), "9\u{B7}77");
        assert_eq!(display_met_location("9\u{B7}77", 0xFF), "9\u{B7}77");
    }

    #[test]
    fn non_numeric_oddballs_are_never_touched() {
        assert_eq!(display_met_location("Route\u{B7}X", 0x58), "Route\u{B7}X");
    }
}
