use fire_red_loop::FireRedState;
use fire_red_party_monitor::Pokemon;
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
pub struct EncounterTracker {
    last_enemy_personality: u32,
    tracked_personality:    Option<u32>,
    enc_map:                (u8, u8),
}

impl EncounterTracker {
    pub fn new() -> Self {
        Self {
            last_enemy_personality: 0,
            tracked_personality:    None,
            enc_map:                (0, 0),
        }
    }

    pub fn reset(&mut self) {
        self.last_enemy_personality = 0;
        self.tracked_personality    = None;
    }

    /// Called once per poll cycle while the game is loaded and state is
    /// initialized. Records first encounters and detects catches.
    pub fn tick(&mut self, current_state: FireRedState, thread_party: &Arc<Mutex<Vec<Pokemon>>>) {
        if let Some(enemy) = crate::game::get_wild_enemy_pokemon() {
            if enemy.box_mon.personality != self.last_enemy_personality {
                self.last_enemy_personality = enemy.box_mon.personality;

                let map_group = current_state.map_group_id;
                let map_name  = current_state.map_name_id;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let is_first = fire_red_database::record_encounter(
                    fire_red_database::Encounter {
                        player_name:    String::new(), // populated from DbState::current_player
                        map_group,
                        map_name,
                        species:        enemy.box_mon.secure.growth.species,
                        species_name:   enemy.box_mon.secure.growth.species_string.clone(),
                        level:          enemy.level,
                        caught:         false,
                        encountered_at: now,
                    },
                );

                if is_first {
                    self.enc_map             = (map_group, map_name);
                    self.tracked_personality = Some(self.last_enemy_personality);
                } else {
                    self.tracked_personality = None;
                }
            }
        }

        if let Some(tp) = self.tracked_personality {
            let party  = thread_party.lock().unwrap_or_else(|e| e.into_inner());
            let caught = party.iter().any(|p| p.box_mon.personality == tp);
            drop(party);
            if caught {
                fire_red_database::set_encounter_caught(self.enc_map.0, self.enc_map.1);
                self.tracked_personality = None;
            }
        }
    }
}
