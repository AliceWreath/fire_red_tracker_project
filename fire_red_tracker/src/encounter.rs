use crate::config::DupesClauseMode;
use fire_red_loop::FireRedState;
use fire_red_party_monitor::Pokemon;
use fire_red_states::LockOrRecover;
use std::sync::{Arc, Mutex};

/// Tracks wild Pokémon encounters for the active Nuzlocke run.
///
/// `gEnemyParty[0]` is never cleared between battles — FireRed only overwrites
/// it at the START of each new battle. Battle detection therefore uses
/// personality CHANGE rather than presence/absence.
///
/// Catch detection watches the player party for the wild Pokémon's exact
/// personality value. No timer is needed: the next battle (personality change)
/// implicitly closes any unresolved encounter as failed/fled.
#[derive(Default)]
pub struct EncounterTracker {
    last_enemy_personality: u32,
    tracked_personality:    Option<u32>,
    enc_map:                (u8, u8),
    /// Latches to `true` once the run is considered officially underway (5+ balls
    /// detected or encounters already exist in DB) and stays true until reset.
    run_tracking_active:    bool,
    /// Set when a party wipe ends the run. Prevents `tick` from re-enabling
    /// tracking until the game unloads or a new run is started.
    wipe_detected:          bool,
    /// One-shot clause-enforcement warnings accumulated by `tick`. Drained by
    /// `drain_warnings()` so each warning appears in exactly one `GameState`.
    pending_warnings:       Vec<String>,
}

impl EncounterTracker {
    pub fn new() -> Self { Self::default() }

    pub fn reset(&mut self) {
        self.last_enemy_personality = 0;
        self.tracked_personality    = None;
        self.run_tracking_active     = false;
        self.wipe_detected          = false;
    }

    /// Called when a party wipe ends the run. Clears the ball latch and locks
    /// `tick` so it won't re-enable tracking until `reset` is called.
    pub fn mark_wipe(&mut self) {
        self.run_tracking_active = false;
        self.wipe_detected      = true;
    }

    /// Seeds the latch from the database. If any encounters have been recorded
    /// for the active run the player must have had balls at some point, so we
    /// treat the pre-ball phase as over. This is more reliable than reading
    /// the bag directly, which can return false positives from stale EWRAM
    /// data at startup.
    pub fn seed_from_db(&mut self) {
        if fire_red_database::has_any_encounters() {
            self.run_tracking_active = true;
        }
    }

    pub fn run_tracking_active(&self) -> bool {
        self.run_tracking_active
    }

    /// Drains and returns all pending clause-enforcement warnings.
    pub fn drain_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_warnings)
    }

    /// Called once per poll cycle while the game is loaded and state is
    /// initialized. Records first encounters and detects catches.
    ///
    /// `dupes_clause` controls whether previously-caught species are skipped:
    /// - `Off` — no extra check; standard Nuzlocke first-encounter-per-area applies.
    /// - `PerPlayer` — skip if *this* player has previously caught this species.
    /// - `Shared` — skip if *any* player in the shared run has caught this species
    ///   (Soul Link / co-op: one catch covers the whole group).
    ///
    /// `run_start_balls` is the minimum Pokéball count that triggers the
    /// run-start latch (configurable via `TrackerConfig::run_start_balls`).
    pub fn tick(&mut self, current_state: FireRedState, thread_party: &Arc<Mutex<Vec<Pokemon>>>, dupes_clause: DupesClauseMode, allow_species_repeats: bool, run_start_balls: u32) {
        if self.wipe_detected { return; }
        if let Some(enemy) = crate::game::get_wild_enemy_pokemon()
            && enemy.box_mon.personality != self.last_enemy_personality
        {
            self.last_enemy_personality = enemy.box_mon.personality;

            if !self.run_tracking_active {
                if crate::game::has_pokeballs_threshold(run_start_balls) {
                    self.run_tracking_active = true;
                } else {
                    return;
                }
            }

            let map_group = current_state.map_group_id;
            let map_name  = current_state.map_name_id;

            let dungeon = fire_red_location_names::dungeon_floors(map_group, map_name);
            if fire_red_database::has_encounter_for_any_floor(dungeon) {
                let area = fire_red_location_names::map_area_name(map_group, map_name);
                let area_label = if area.is_empty() {
                    format!("{}:{}", map_group, map_name)
                } else {
                    area.to_string()
                };
                self.pending_warnings.push(format!("Area already encountered: {}", area_label));
                return;
            }

            let species = enemy.box_mon.secure.growth.species;
            if !allow_species_repeats && fire_red_database::species_encountered(species) {
                self.pending_warnings.push(format!(
                    "Species already encountered: {}",
                    enemy.box_mon.secure.growth.species_string
                ));
                return;
            }
            let skip = match dupes_clause {
                DupesClauseMode::Off       => false,
                DupesClauseMode::PerPlayer => fire_red_database::species_caught_by_self(species),
                DupesClauseMode::Shared    => fire_red_database::species_caught_any(species),
            };
            if skip {
                self.pending_warnings.push(format!(
                    "Dupes clause: {} already caught",
                    enemy.box_mon.secure.growth.species_string
                ));
                return;
            }
            let now = fire_red_database::unix_now();

            let personality = enemy.box_mon.personality;
            let ot_id       = enemy.box_mon.ot_id;
            let is_shiny    = crate::game::is_shiny(personality, ot_id);

            if is_shiny {
                if let Err(e) = fire_red_database::record_event(fire_red_database::EventKind::Shiny {
                    species_name: &enemy.box_mon.secure.growth.species_string,
                    level:        enemy.level,
                }) {
                    tracing::warn!("Failed to record Shiny event: {e}");
                }
                crate::webhook::fire_event(crate::webhook::WebhookEvent::Shiny {
                    player:    fire_red_loop::get_trainer_name(),
                    timestamp: now,
                    pokemon:   crate::webhook::PokemonInfo {
                        nickname: String::new(),
                        species:  enemy.box_mon.secure.growth.species_string.clone(),
                        level:    enemy.level,
                        shiny:    true,
                        nature:   fire_red_database::nature_name(personality).to_string(),
                    },
                });
            }

            let is_first = match fire_red_database::record_encounter(
                fire_red_database::Encounter {
                    player_name:    fire_red_loop::get_trainer_name(),
                    map_group,
                    map_name,
                    species:        enemy.box_mon.secure.growth.species,
                    species_name:   enemy.box_mon.secure.growth.species_string.clone(),
                    level:          enemy.level,
                    caught:         false,
                    encountered_at: now,
                    is_shiny,
                },
            ) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to record encounter: {e}");
                    false
                }
            };

            if is_first {
                self.enc_map             = (map_group, map_name);
                self.tracked_personality = Some(self.last_enemy_personality);
            } else {
                self.tracked_personality = None;
            }
        }

        if let Some(tp) = self.tracked_personality {
            let party  = thread_party.lock_or_recover();
            let caught = party.iter().any(|p| p.box_mon.personality == tp);
            drop(party);
            if caught {
                fire_red_database::set_encounter_caught(self.enc_map.0, self.enc_map.1);
                self.tracked_personality = None;
                self.enc_map             = (0, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_not_active_and_not_wiped() {
        let t = EncounterTracker::new();
        assert!(!t.run_tracking_active());
        assert!(!t.wipe_detected);
    }

    #[test]
    fn mark_wipe_clears_tracking_and_latches_wipe_flag() {
        let mut t = EncounterTracker::new();
        t.run_tracking_active = true;
        t.mark_wipe();
        assert!(!t.run_tracking_active());
        assert!(t.wipe_detected);
    }

    #[test]
    fn reset_clears_wipe_flag_and_tracking() {
        let mut t = EncounterTracker::new();
        t.run_tracking_active = true;
        t.mark_wipe();
        t.reset();
        assert!(!t.run_tracking_active());
        assert!(!t.wipe_detected);
    }

    #[test]
    fn reset_clears_tracked_personality() {
        let mut t = EncounterTracker::new();
        t.tracked_personality = Some(0xDEAD_BEEF);
        t.reset();
        assert!(t.tracked_personality.is_none());
    }

    #[test]
    fn wipe_can_be_set_again_after_reset() {
        let mut t = EncounterTracker::new();
        t.mark_wipe();
        t.reset();
        assert!(!t.wipe_detected);
        t.mark_wipe();
        assert!(t.wipe_detected);
    }

    #[test]
    fn run_tracking_active_accessor_mirrors_field() {
        let mut t = EncounterTracker::new();
        assert!(!t.run_tracking_active());
        t.run_tracking_active = true;
        assert!(t.run_tracking_active());
        t.run_tracking_active = false;
        assert!(!t.run_tracking_active());
    }
}
