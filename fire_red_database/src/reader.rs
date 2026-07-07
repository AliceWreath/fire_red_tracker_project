//! `DbReader`: read access to the shared database for the aggregator.

use super::*;

// ---------------------------------------------------------------------------
// DbReader — read access to the shared database for the aggregator
//
// Each DbReader holds its own connection so the aggregator can read multiple
// players' data from the same PostgreSQL instance. The active run is resolved
// by player name: call sync_player() each frame with the name received from
// the live game state, and the DbReader will update its cached run_id whenever
// the name changes.
// ---------------------------------------------------------------------------

pub struct DbReader {
    client: Mutex<Client>,
    run_id: Mutex<Option<u32>>,
    last_player: Mutex<String>,
    dirty: std::sync::atomic::AtomicBool,
    /// `true` when the tracked run has `ended_at IS NULL` (currently active).
    is_active: std::sync::atomic::AtomicBool,
    /// When `Some`, `sync_player` uses this run ID instead of querying for the
    /// most-recent run.  Set by direct-mode resume so the user's chosen run is
    /// always used regardless of how many newer runs exist in the database.
    forced_run_id: Mutex<Option<u32>>,
}

impl DbReader {
    /// Opens a connection to the PostgreSQL server.
    ///
    /// Returns `None` if the server is unreachable.
    pub fn open(connection_string: &str) -> Option<Self> {
        let client = match Client::connect(connection_string, NoTls) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("DB connection failed ({}): {}", connection_string, e);
                return None;
            }
        };
        tracing::info!("DB connected: {}", connection_string);
        Some(Self {
            client: Mutex::new(client),
            run_id: Mutex::new(None),
            last_player: Mutex::new(String::new()),
            dirty: std::sync::atomic::AtomicBool::new(false),
            is_active: std::sync::atomic::AtomicBool::new(false),
            forced_run_id: Mutex::new(None),
        })
    }

    /// Forces the next `sync_player` call to re-query the database even if the
    /// player name has not changed. Call this after a run is ended or started
    /// remotely so the cached run ID is immediately refreshed.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Pin this reader to a specific run ID, bypassing the "most recent run"
    /// query in `sync_player`.  Used by direct-mode slots when the user picks
    /// an existing run to resume from the join page.
    pub fn set_forced_run_id(&self, id: u32) {
        *self.forced_run_id.lock_or_recover() = Some(id);
        self.mark_dirty();
    }

    /// Returns the run ID this reader is currently tracking — forced_run_id takes
    /// precedence, then the most recently synced run_id.
    pub fn get_run_id(&self) -> Option<u32> {
        if let Some(id) = *self.forced_run_id.lock_or_recover() {
            return Some(id);
        }
        *self.run_id.lock_or_recover()
    }

    /// Returns the active run ID if the tracked run has not been ended, else `None`.
    ///
    /// `forced_run_id` is always considered active (it was explicitly resumed,
    /// so `is_active` may not yet be set if `sync_player` hasn't been called).
    pub fn active_run_id(&self) -> Option<u32> {
        if let Some(id) = *self.forced_run_id.lock_or_recover() {
            return Some(id);
        }
        if self.is_active.load(std::sync::atomic::Ordering::SeqCst) {
            *self.run_id.lock_or_recover()
        } else {
            None
        }
    }

    /// Updates the cached run ID to the most recent shared run (active or ended).
    ///
    /// All connected trackers share a single run. The most recently created run
    /// is always used so that historical data is still visible after a run ends.
    ///
    /// Re-queries whenever the player name changes OR `mark_dirty()` was called.
    /// Returns `true` if the run ID changed (triggers a caught-list refresh).
    pub fn sync_player(&self, player_name: &str) -> bool {
        // If the caller pinned a specific run (e.g. direct-mode resume), use
        // it unconditionally rather than querying for the most-recent run.
        if let Some(id) = *self.forced_run_id.lock_or_recover() {
            let mut rid = self.run_id.lock_or_recover();
            let changed = *rid != Some(id);
            if changed {
                *rid = Some(id);
                drop(rid);
                self.is_active.store(true, std::sync::atomic::Ordering::SeqCst);
                *self.last_player.lock_or_recover() = player_name.to_string();
            }
            self.dirty.store(false, std::sync::atomic::Ordering::SeqCst);
            return changed;
        }

        let forced = self.dirty.swap(false, std::sync::atomic::Ordering::SeqCst);
        if !forced {
            let last = self.last_player.lock_or_recover();
            if *last == player_name {
                return false;
            }
        }

        let row = self
            .client
            .lock_or_recover()
            .query_opt(
                "SELECT id, ended_at FROM runs ORDER BY id DESC LIMIT 1",
                &[],
            )
            .ok()
            .flatten();

        let (new_id, active) = match row {
            Some(r) => {
                let id: i32 = r.get(0);
                let ended: Option<i64> = r.get(1);
                (Some(id as u32), ended.is_none())
            }
            None => (None, false),
        };

        self.is_active
            .store(active, std::sync::atomic::Ordering::SeqCst);
        let mut rid = self.run_id.lock_or_recover();
        let old_id = *rid;
        *rid = new_id;
        drop(rid);
        if new_id.is_some() || forced {
            *self.last_player.lock_or_recover() = player_name.to_string();
        }
        new_id != old_id
    }

    /// Returns caught Pokemon for the active run belonging to `player_name`.
    pub fn list_caught(&self, player_name: &str) -> Vec<CaughtPokemon> {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return vec![],
        };
        query_caught(&mut self.client.lock_or_recover(), run_id, player_name)
    }

    /// Returns `true` if the Pokemon with this personality is dead in the active run.
    pub fn is_dead(&self, personality: u32) -> bool {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return false,
        };
        query_is_dead(&mut self.client.lock_or_recover(), run_id, personality)
    }

    /// Returns dead Pokemon for the active run belonging to `player_name`, keyed by personality.
    pub fn list_dead_with_records(&self, player_name: &str) -> HashMap<u32, DeadPokemon> {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return HashMap::new(),
        };
        self.client
            .lock_or_recover()
            .query(
                "SELECT
                    player_name, personality, ot_id, ot_name, nickname, species, species_name, is_shiny, nature,
                    level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                    move1, move2, move3, move4, pp1, pp2, pp3, pp4,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                    held_item, ability, ability_name, friendship, met_location, died_at, gender,
                    is_soul_link_death, killed_by_species, killed_by_move,
                    COALESCE(area_name, '') AS area_name
                 FROM dead_pokemon WHERE run_id = $1 AND player_name = $2",
                &[&(run_id as i32), &player_name],
            )
            .unwrap_or_else(|e| { tracing::warn!("list_dead_with_records DB query failed: {e}"); vec![] })
            .iter()
            .map(|row| {
                let dp = row_to_dead_pokemon(row);
                (dp.personality, dp)
            })
            .collect()
    }

    /// Returns recorded first encounters for the active run belonging to `player_name`.
    pub fn list_encounters(&self, player_name: &str) -> Vec<Encounter> {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return vec![],
        };
        self.client
            .lock_or_recover()
            .query(
                "SELECT player_name, map_group, map_name, species, species_name, level, caught, encountered_at, is_shiny
                 FROM encounters WHERE run_id = $1 AND player_name = $2 ORDER BY encountered_at ASC",
                &[&(run_id as i32), &player_name],
            )
            .unwrap_or_else(|e| { tracing::warn!("list_encounters DB query failed: {e}"); vec![] })
            .iter()
            .map(|row| Encounter {
                player_name:    row.get(0),
                map_group:      row.get::<_, i32>(1) as u8,
                map_name:       row.get::<_, i32>(2) as u8,
                species:        row.get::<_, i32>(3) as u16,
                species_name:   row.get(4),
                level:          row.get::<_, i32>(5) as u8,
                caught:         row.get(6),
                encountered_at: row.get::<_, i64>(7) as u64,
                is_shiny:       row.get(8),
            })
            .collect()
    }

    /// Returns encounters from the most recently completed run, for cross-run comparison.
    pub fn list_prev_run_encounters(&self, player_name: &str) -> Vec<Encounter> {
        let current_run_id = *self.run_id.lock_or_recover();
        let mut client = self.client.lock_or_recover();

        let prev_run_id: Option<u32> = if let Some(cid) = current_run_id {
            client.query_opt(
                "SELECT id FROM runs WHERE ended_at IS NOT NULL AND id != $1 ORDER BY id DESC LIMIT 1",
                &[&(cid as i32)],
            )
        } else {
            client.query_opt(
                "SELECT id FROM runs WHERE ended_at IS NOT NULL ORDER BY id DESC LIMIT 1",
                &[],
            )
        }
        .ok()
        .flatten()
        .map(|row| row.get::<_, i32>(0) as u32);

        let Some(prev_id) = prev_run_id else {
            return vec![];
        };

        client
            .query(
                "SELECT player_name, map_group, map_name, species, species_name, level, caught, encountered_at, is_shiny
                 FROM encounters WHERE run_id = $1 AND player_name = $2 ORDER BY encountered_at ASC",
                &[&(prev_id as i32), &player_name],
            )
            .unwrap_or_else(|e| { tracing::warn!("list_prev_run_encounters DB query failed: {e}"); vec![] })
            .iter()
            .map(|row| Encounter {
                player_name:    row.get(0),
                map_group:      row.get::<_, i32>(1) as u8,
                map_name:       row.get::<_, i32>(2) as u8,
                species:        row.get::<_, i32>(3) as u16,
                species_name:   row.get(4),
                level:          row.get::<_, i32>(5) as u8,
                caught:         row.get(6),
                encountered_at: row.get::<_, i64>(7) as u64,
                is_shiny:       row.get(8),
            })
            .collect()
    }

    /// Returns a summary of the tracked run: player name, start/end times,
    /// death count, and catch count. Returns `None` if no run is tracked yet.
    pub fn run_summary(&self) -> Option<(u32, String, u64, Option<u64>, usize, usize)> {
        let run_id = (*self.run_id.lock_or_recover())?;
        self.client
            .lock_or_recover()
            .query_opt(
                "SELECT r.player_name, r.started_at, r.ended_at,
                        COUNT(DISTINCT d.personality), COUNT(DISTINCT c.personality)
                 FROM runs r
                 LEFT JOIN dead_pokemon    d ON d.run_id = r.id
                 LEFT JOIN caught_pokemon  c ON c.run_id = r.id
                 WHERE r.id = $1
                 GROUP BY r.id, r.player_name, r.started_at, r.ended_at",
                &[&(run_id as i32)],
            )
            .ok()
            .flatten()
            .map(|row| {
                (
                    run_id,
                    row.get::<_, String>(0),
                    row.get::<_, i64>(1) as u64,
                    row.get::<_, Option<i64>>(2).map(|v| v as u64),
                    row.get::<_, i64>(3) as usize,
                    row.get::<_, i64>(4) as usize,
                )
            })
    }

    /// Appends a row to the `events` table using this reader's connection and run.
    ///
    /// Used by the aggregator process, which does not share the tracker's global
    /// `DB` singleton. Mirrors the standalone `record_event` function.
    pub fn record_event(
        &self,
        player_name: &str,
        event: EventKind<'_>,
    ) -> Result<(), postgres::Error> {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return Ok(()),
        };
        let occurred_at = unix_now() as i64;
        let (event_type, species_name, nickname, old_nickname, level) = event.row_parts();
        self.client.lock_or_recover().execute(
            "INSERT INTO events (run_id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &(run_id as i32),
                &player_name,
                &event_type,
                &species_name,
                &nickname,
                &old_nickname,
                &level,
                &occurred_at,
            ],
        )?;
        Ok(())
    }

    /// Returns all soul-link overrides for the current run as a `HashMap`
    /// (personality → partner_personality).
    ///
    /// Called by `BroadcastLoop` once per cache-refresh cycle so
    /// `propagate_soul_links` and `build_party_dto` can consult overrides
    /// without a per-frame DB round-trip.
    pub fn load_soul_link_overrides(&self) -> HashMap<u32, u32> {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return HashMap::new(),
        };
        let run_i32 = run_id as i32;
        match self.client.lock_or_recover().query(
            "SELECT personality, partner_personality FROM soul_link_overrides WHERE run_id = $1",
            &[&run_i32],
        ) {
            Ok(rows) => rows
                .iter()
                .map(|r| (r.get::<_, i64>(0) as u32, r.get::<_, i64>(1) as u32))
                .collect(),
            Err(e) => {
                tracing::warn!("load_soul_link_overrides: {e}");
                HashMap::new()
            }
        }
    }

    /// Returns all goals for the current run, ordered by creation time.
    /// Returns an empty `Vec` when no run is active or the table is empty.
    pub fn list_goals(&self) -> Vec<GoalRow> {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return vec![],
        };
        let run_i32 = run_id as i32;
        match self.client.lock_or_recover().query(
            "SELECT id, text, completed FROM run_goals WHERE run_id = $1 ORDER BY created_at ASC",
            &[&run_i32],
        ) {
            Ok(rows) => rows
                .iter()
                .map(|r| GoalRow {
                    id:        r.get(0),
                    text:      r.get(1),
                    completed: r.get(2),
                })
                .collect(),
            Err(e) => {
                tracing::warn!("list_goals: {e}");
                vec![]
            }
        }
    }

    /// Returns the pinned display-column index for `player_name` within the
    /// current run, or `None` if that player has no pin recorded.
    pub fn query_player_slot_index(&self, player_name: &str) -> Option<u8> {
        let run_id = self.get_run_id()? as i32;
        let row = self
            .client
            .lock_or_recover()
            .query_opt(
                "SELECT slot_index FROM run_player_slots WHERE run_id = $1 AND player_name = $2",
                &[&run_id, &player_name],
            )
            .ok()??;
        let v: i32 = row.get(0);
        u8::try_from(v).ok()
    }

    /// Returns all soul-link overrides for the current run as a JSON array.
    ///
    /// Each element: `{"personality":<u32>,"partner_personality":<u32>,"created_at":<i64>}`.
    /// Returns `"[]"` when the run ID is unknown or the table is empty.
    pub fn list_soul_link_overrides_json(&self) -> String {
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return "[]".to_string(),
        };
        let run_i32 = run_id as i32;
        let rows = match self.client.lock_or_recover().query(
            "SELECT personality, partner_personality, created_at
             FROM soul_link_overrides WHERE run_id = $1 ORDER BY created_at",
            &[&run_i32],
        ) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("list_soul_link_overrides_json: {e}");
                return "[]".to_string();
            }
        };
        let mut out = String::from("[");
        for (i, row) in rows.iter().enumerate() {
            let p: i64 = row.get(0);
            let pp: i64 = row.get(1);
            let ca: i64 = row.get(2);
            if i > 0 {
                out.push(',');
            }
            use std::fmt::Write as _;
            let _ = write!(
                &mut out,
                r#"{{"personality":{p},"partner_personality":{pp},"created_at":{ca}}}"#
            );
        }
        out.push(']');
        out
    }

    /// Inserts a soul-link death record for `caught` in this player's active run.
    ///
    /// Battle stats (HP, Attack, etc.) are stored as 0 to signal a soul-link
    /// kill rather than a direct in-game death. Safe to call if the record
    /// already exists — the insert is a no-op in that case.
    ///
    /// Returns:
    /// - `Some(true)` — row was newly inserted; caller should fire the event and mark as propagated.
    /// - `Some(false)` — row already existed (`ON CONFLICT DO NOTHING`); caller should mark as
    ///   propagated but **not** re-fire the event.
    /// - `None` — run ID not yet known or DB write failed; caller should retry next frame.
    pub fn mark_soul_link_dead(&self, caught: &CaughtPokemon) -> Option<bool> {
        let run_id = (*self.run_id.lock_or_recover())?;
        let now = unix_now();
        match self.client.lock_or_recover().execute(
            "INSERT INTO dead_pokemon (
                    run_id, player_name, personality, ot_id, ot_name, nickname,
                    species, species_name, is_shiny, nature,
                    level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                    move1, move2, move3, move4,
                    pp1, pp2, pp3, pp4,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                    held_item, ability, ability_name, friendship, met_location, died_at, gender,
                    is_soul_link_death
                ) VALUES (
                    $1, $2, $3, $4, '', $5, $6, $7, $8, $9, $10,
                    0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0,
                    0, 0, 0, 0,
                    $11, $12, $13, $14, $15, $16,
                    0, 0, 0, 0, 0, 0,
                    0, 0, '', 0, $17, $18, $19,
                    TRUE
                ) ON CONFLICT (run_id, personality) DO NOTHING",
            &[
                &(run_id as i32),
                &caught.player_name,
                &(caught.personality as i64),
                &(caught.ot_id as i64),
                &caught.nickname,
                &(caught.species as i32),
                &caught.species_name,
                &caught.is_shiny,
                &caught.nature,
                &(caught.level as i32),
                &(caught.ivs.hp as i32),
                &(caught.ivs.attack as i32),
                &(caught.ivs.defense as i32),
                &(caught.ivs.speed as i32),
                &(caught.ivs.sp_attack as i32),
                &(caught.ivs.sp_defense as i32),
                &(caught.met_location as i32),
                &(now as i64),
                &(caught.gender as i32),
            ],
        ) {
            Ok(1) => Some(true),
            Ok(_) => Some(false), // ON CONFLICT DO NOTHING — row already existed
            Err(e) => {
                tracing::warn!("mark_soul_link_dead: DB error: {e}");
                None
            }
        }
    }
}
