use postgres::{Client, NoTls};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const NATURES: [&str; 25] = [
    "Hardy",   "Lonely", "Brave",   "Adamant", "Naughty",
    "Bold",    "Docile", "Relaxed", "Impish",  "Lax",
    "Timid",   "Hasty",  "Serious", "Jolly",   "Naive",
    "Modest",  "Mild",   "Quiet",   "Bashful", "Rash",
    "Calm",    "Gentle", "Sassy",   "Careful", "Quirky",
];

pub fn nature_name(personality: u32) -> &'static str {
    NATURES[(personality % 25) as usize]
}

pub fn format_timestamp(secs: u64) -> String {
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    let mut days = secs / 86400;
    let mut year = 1970u32;
    loop {
        let days_in_year: u64 = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year { break; }
        days -= days_in_year;
        year += 1;
    }
    let month_days: [u64; 12] = [
        31, if is_leap(year) { 29 } else { 28 }, 31, 30,
        31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md { break; }
        days -= md;
        month += 1;
    }
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", year, month, days + 1, h, m, s)
}

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct IVs {
    pub hp: u8,
    pub attack: u8,
    pub defense: u8,
    pub speed: u8,
    pub sp_attack: u8,
    pub sp_defense: u8,
}

#[derive(Clone, Debug, Default)]
pub struct EVs {
    pub hp: u8,
    pub attack: u8,
    pub defense: u8,
    pub speed: u8,
    pub sp_attack: u8,
    pub sp_defense: u8,
}

/// All meaningful data captured at the moment a Pokemon faints.
#[derive(Clone, Debug)]
pub struct DeadPokemon {
    /// Trainer name of the player who owned this Pokémon. Used to distinguish
    /// records from different players sharing the same run.
    pub player_name: String,
    pub personality: u32,
    pub ot_id: u32,
    pub ot_name: String,
    pub nickname: String,
    pub species: u16,
    pub species_name: String,
    pub is_shiny: bool,
    pub nature: String,

    pub level: u8,
    pub experience: u32,
    pub max_hp: u16,
    pub attack: u16,
    pub defense: u16,
    pub speed: u16,
    pub sp_attack: u16,
    pub sp_defense: u16,

    pub moves: [u16; 4],
    pub pp: [u8; 4],

    pub ivs: IVs,
    pub evs: EVs,
    pub held_item: u16,
    pub ability: u8,
    pub ability_name: String,
    pub friendship: u8,
    pub met_location: u8,
    pub died_at: u64,
    /// `0` = male, `1` = female, `2` = genderless.
    pub gender: u8,
}

/// A wild Pokémon encounter — the first one per area per player is stored for Nuzlocke tracking.
#[derive(Clone, Debug)]
pub struct Encounter {
    /// Trainer name of the player who had this encounter.
    pub player_name: String,
    pub map_group: u8,
    pub map_name: u8,
    pub species: u16,
    pub species_name: String,
    pub level: u8,
    /// `false` until updated to `true` if the Pokémon was successfully caught.
    pub caught: bool,
    pub encountered_at: u64,
}

/// Snapshot of a Pokemon at the moment it first joined the party.
#[derive(Clone, Debug)]
pub struct CaughtPokemon {
    /// Trainer name of the player who caught this Pokémon.
    pub player_name:  String,
    pub personality:  u32,
    pub ot_id:        u32,
    pub nickname:     String,
    pub species:      u16,
    pub species_name: String,
    pub is_shiny:     bool,
    pub nature:       String,
    pub level:        u8,
    pub met_location: u8,
    pub ivs:          IVs,
    pub caught_at:    u64,
    /// `0` = male, `1` = female, `2` = genderless.
    pub gender:       u8,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

struct DbState {
    client: Client,
    /// The shared run all connected trackers record into.
    run_id: Option<u32>,
    /// This tracker's player name, set when the game loads. Included on every
    /// write (dead_pokemon, caught_pokemon, encounters) so records from different
    /// players sharing the same run can be distinguished.
    current_player: String,
}

static DB: OnceLock<Mutex<DbState>> = OnceLock::new();

fn db() -> &'static Mutex<DbState> {
    DB.get().expect("fire_red_database::initialize must be called before any database operation")
}

/// Connects to the PostgreSQL server and creates the schema if absent.
///
/// Must be called exactly once before any other function in this crate.
/// The database itself must already exist:
///
/// ```sql
/// CREATE DATABASE nuzlocke;
/// ```
///
/// Example connection strings:
/// - `postgresql://localhost/nuzlocke`
/// - `postgresql://user:password@192.168.1.10/nuzlocke`
/// - `host=192.168.1.10 user=alice dbname=nuzlocke`
pub fn initialize(connection_string: &str) {
    let mut client = Client::connect(connection_string, NoTls)
        .unwrap_or_else(|e| panic!(
            "Failed to connect to PostgreSQL: {e}\n\
             Ensure the server is reachable and the database exists.\n\
             Create it with:  psql -c 'CREATE DATABASE nuzlocke;'"
        ));

    client.batch_execute("
        CREATE TABLE IF NOT EXISTS runs (
            id          SERIAL  PRIMARY KEY,
            player_name TEXT    NOT NULL DEFAULT 'Unknown',
            started_at  BIGINT  NOT NULL
        );

        CREATE TABLE IF NOT EXISTS dead_pokemon (
            run_id       INTEGER NOT NULL REFERENCES runs(id),
            personality  BIGINT  NOT NULL,
            ot_id        BIGINT  NOT NULL,
            ot_name      TEXT    NOT NULL,
            nickname     TEXT    NOT NULL,
            species      INTEGER NOT NULL,
            species_name TEXT    NOT NULL,
            is_shiny     BOOLEAN NOT NULL,
            nature       TEXT    NOT NULL,
            level        INTEGER NOT NULL,
            experience   BIGINT  NOT NULL,
            max_hp       INTEGER NOT NULL,
            attack       INTEGER NOT NULL,
            defense      INTEGER NOT NULL,
            speed        INTEGER NOT NULL,
            sp_attack    INTEGER NOT NULL,
            sp_defense   INTEGER NOT NULL,
            move1        INTEGER NOT NULL,
            move2        INTEGER NOT NULL,
            move3        INTEGER NOT NULL,
            move4        INTEGER NOT NULL,
            pp1          INTEGER NOT NULL,
            pp2          INTEGER NOT NULL,
            pp3          INTEGER NOT NULL,
            pp4          INTEGER NOT NULL,
            iv_hp        INTEGER NOT NULL,
            iv_attack    INTEGER NOT NULL,
            iv_defense   INTEGER NOT NULL,
            iv_speed     INTEGER NOT NULL,
            iv_sp_attack  INTEGER NOT NULL,
            iv_sp_defense INTEGER NOT NULL,
            ev_hp        INTEGER NOT NULL,
            ev_attack    INTEGER NOT NULL,
            ev_defense   INTEGER NOT NULL,
            ev_speed     INTEGER NOT NULL,
            ev_sp_attack  INTEGER NOT NULL,
            ev_sp_defense INTEGER NOT NULL,
            held_item    INTEGER NOT NULL,
            ability      INTEGER NOT NULL,
            ability_name TEXT    NOT NULL,
            friendship   INTEGER NOT NULL,
            met_location INTEGER NOT NULL,
            died_at      BIGINT  NOT NULL,
            PRIMARY KEY (run_id, personality)
        );

        CREATE TABLE IF NOT EXISTS caught_pokemon (
            run_id        INTEGER NOT NULL REFERENCES runs(id),
            personality   BIGINT  NOT NULL,
            ot_id         BIGINT  NOT NULL,
            nickname      TEXT    NOT NULL,
            species       INTEGER NOT NULL,
            species_name  TEXT    NOT NULL,
            is_shiny      BOOLEAN NOT NULL,
            nature        TEXT    NOT NULL,
            level         INTEGER NOT NULL,
            met_location  INTEGER NOT NULL,
            iv_hp         INTEGER NOT NULL,
            iv_attack     INTEGER NOT NULL,
            iv_defense    INTEGER NOT NULL,
            iv_speed      INTEGER NOT NULL,
            iv_sp_attack  INTEGER NOT NULL,
            iv_sp_defense INTEGER NOT NULL,
            caught_at     BIGINT  NOT NULL,
            PRIMARY KEY (run_id, personality)
        );

        -- First wild encounter per area per run (Nuzlocke rule).
        CREATE TABLE IF NOT EXISTS encounters (
            id             SERIAL  PRIMARY KEY,
            run_id         INTEGER NOT NULL REFERENCES runs(id),
            map_group      INTEGER NOT NULL,
            map_name       INTEGER NOT NULL,
            species        INTEGER NOT NULL,
            species_name   TEXT    NOT NULL,
            level          INTEGER NOT NULL,
            caught         BOOLEAN NOT NULL DEFAULT FALSE,
            encountered_at BIGINT  NOT NULL,
            UNIQUE (run_id, map_group, map_name)
        );

        -- Stores the last-active run_id per process for the --list-runs display.
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- Migration: add ended_at for runs that were explicitly ended.
        ALTER TABLE runs ADD COLUMN IF NOT EXISTS ended_at BIGINT;

        -- Migration: add player_name to record tables so that entries from
        -- different players sharing the same run can be distinguished.
        ALTER TABLE dead_pokemon   ADD COLUMN IF NOT EXISTS player_name TEXT NOT NULL DEFAULT 'Unknown';
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS player_name TEXT NOT NULL DEFAULT 'Unknown';
        ALTER TABLE encounters     ADD COLUMN IF NOT EXISTS player_name TEXT NOT NULL DEFAULT 'Unknown';

        -- Migration: drop the old per-run encounters unique constraint and replace
        -- it with a per-player one so two players can each have a first encounter
        -- in the same map area within the same shared run.
        ALTER TABLE encounters DROP CONSTRAINT IF EXISTS encounters_run_id_map_group_map_name_key;
        DO $$ BEGIN
            ALTER TABLE encounters ADD CONSTRAINT encounters_run_id_player_name_map_key
                UNIQUE (run_id, player_name, map_group, map_name);
        EXCEPTION WHEN duplicate_object OR duplicate_table THEN NULL;
        END $$;

        -- Migration: add gender column (0=male 1=female 2=genderless, default genderless).
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS gender INTEGER NOT NULL DEFAULT 2;
        ALTER TABLE dead_pokemon   ADD COLUMN IF NOT EXISTS gender INTEGER NOT NULL DEFAULT 2;
    ").expect("Failed to create database schema");

    DB.set(Mutex::new(DbState { client, run_id: None, current_player: String::new() }))
        .unwrap_or_else(|_| panic!("fire_red_database::initialize called more than once"));
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn get_meta(client: &mut Client, key: &str) -> Option<String> {
    client
        .query_opt("SELECT value FROM meta WHERE key = $1", &[&key])
        .ok()?
        .map(|row| row.get(0))
}

fn set_meta(client: &mut Client, key: &str, value: &str) {
    if let Err(e) = client.execute(
        "INSERT INTO meta (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        &[&key, &value],
    ) {
        eprintln!("Warning: failed to write meta key '{key}': {e}");
    }
}

fn delete_meta(client: &mut Client, key: &str) {
    if let Err(e) = client.execute("DELETE FROM meta WHERE key = $1", &[&key]) {
        eprintln!("Warning: failed to delete meta key '{key}': {e}");
    }
}

fn query_caught(client: &mut Client, run_id: u32, player_name: &str) -> Vec<CaughtPokemon> {
    client
        .query(
            "SELECT player_name, personality, ot_id, nickname, species, species_name,
                    is_shiny, nature, level, met_location,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    caught_at, gender
             FROM caught_pokemon
             WHERE run_id = $1 AND player_name = $2
             ORDER BY caught_at ASC",
            &[&(run_id as i32), &player_name],
        )
        .unwrap_or_default()
        .iter()
        .map(|row| CaughtPokemon {
            player_name:  row.get(0),
            personality:  row.get::<_, i64>(1) as u32,
            ot_id:        row.get::<_, i64>(2) as u32,
            nickname:     row.get(3),
            species:      row.get::<_, i32>(4) as u16,
            species_name: row.get(5),
            is_shiny:     row.get(6),
            nature:       row.get(7),
            level:        row.get::<_, i32>(8) as u8,
            met_location: row.get::<_, i32>(9) as u8,
            ivs: IVs {
                hp:         row.get::<_, i32>(10) as u8,
                attack:     row.get::<_, i32>(11) as u8,
                defense:    row.get::<_, i32>(12) as u8,
                speed:      row.get::<_, i32>(13) as u8,
                sp_attack:  row.get::<_, i32>(14) as u8,
                sp_defense: row.get::<_, i32>(15) as u8,
            },
            caught_at: row.get::<_, i64>(16) as u64,
            gender:    row.get::<_, i32>(17) as u8,
        })
        .collect()
}

fn query_is_dead(client: &mut Client, run_id: u32, personality: u32) -> bool {
    client
        .query_one(
            "SELECT COUNT(*) FROM dead_pokemon WHERE run_id = $1 AND personality = $2",
            &[&(run_id as i32), &(personality as i64)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Converts a dead_pokemon SELECT row into a [`DeadPokemon`].
/// Column 0 must be `player_name`; the remaining columns follow the standard
/// order used by all dead_pokemon queries in this file.
fn row_to_dead_pokemon(row: &postgres::Row) -> DeadPokemon {
    DeadPokemon {
        player_name:  row.get(0),
        personality:  row.get::<_, i64>(1) as u32,
        ot_id:        row.get::<_, i64>(2) as u32,
        ot_name:      row.get(3),
        nickname:     row.get(4),
        species:      row.get::<_, i32>(5) as u16,
        species_name: row.get(6),
        is_shiny:     row.get(7),
        nature:       row.get(8),
        level:        row.get::<_, i32>(9) as u8,
        experience:   row.get::<_, i64>(10) as u32,
        max_hp:       row.get::<_, i32>(11) as u16,
        attack:       row.get::<_, i32>(12) as u16,
        defense:      row.get::<_, i32>(13) as u16,
        speed:        row.get::<_, i32>(14) as u16,
        sp_attack:    row.get::<_, i32>(15) as u16,
        sp_defense:   row.get::<_, i32>(16) as u16,
        moves: [
            row.get::<_, i32>(17) as u16,
            row.get::<_, i32>(18) as u16,
            row.get::<_, i32>(19) as u16,
            row.get::<_, i32>(20) as u16,
        ],
        pp: [
            row.get::<_, i32>(21) as u8,
            row.get::<_, i32>(22) as u8,
            row.get::<_, i32>(23) as u8,
            row.get::<_, i32>(24) as u8,
        ],
        ivs: IVs {
            hp:         row.get::<_, i32>(25) as u8,
            attack:     row.get::<_, i32>(26) as u8,
            defense:    row.get::<_, i32>(27) as u8,
            speed:      row.get::<_, i32>(28) as u8,
            sp_attack:  row.get::<_, i32>(29) as u8,
            sp_defense: row.get::<_, i32>(30) as u8,
        },
        evs: EVs {
            hp:         row.get::<_, i32>(31) as u8,
            attack:     row.get::<_, i32>(32) as u8,
            defense:    row.get::<_, i32>(33) as u8,
            speed:      row.get::<_, i32>(34) as u8,
            sp_attack:  row.get::<_, i32>(35) as u8,
            sp_defense: row.get::<_, i32>(36) as u8,
        },
        held_item:    row.get::<_, i32>(37) as u16,
        ability:      row.get::<_, i32>(38) as u8,
        ability_name: row.get(39),
        friendship:   row.get::<_, i32>(40) as u8,
        met_location: row.get::<_, i32>(41) as u8,
        died_at:      row.get::<_, i64>(42) as u64,
        gender:       row.get::<_, i32>(43) as u8,
    }
}

// ---------------------------------------------------------------------------
// Public API — run management
// ---------------------------------------------------------------------------

/// Creates a fresh run, sets it as active in this process, and returns its ID.
pub fn new_run(player_name: &str) -> u32 {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let row = state.client
        .query_one(
            "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
            &[&player_name, &(unix_now() as i64)],
        )
        .expect("Failed to insert run");
    let id = row.get::<_, i32>(0) as u32;
    state.run_id = Some(id);
    set_meta(&mut state.client, "active_run_id", &id.to_string());
    id
}

/// Switches the active run for this process to an existing run by ID.
///
/// Returns `false` if no run with that ID exists.
pub fn resume_run(id: u32) -> bool {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let exists = state.client
        .query_opt("SELECT 1 FROM runs WHERE id = $1", &[&(id as i32)])
        .expect("Failed to query runs")
        .is_some();
    if exists {
        state.run_id = Some(id);
        set_meta(&mut state.client, "active_run_id", &id.to_string());
    }
    exists
}

/// Returns the active run ID for this process, falling back to the most
/// recently created run. Creates a new run if none exist.
pub fn get_or_create_run(player_name: &str) -> u32 {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());

    // Already selected in this session — keep it.
    if let Some(id) = state.run_id {
        return id;
    }

    // Fall back to the most recently created run — all trackers share one run.
    if let Some(row) = state.client
        .query_opt(
            "SELECT id FROM runs ORDER BY id DESC LIMIT 1",
            &[],
        )
        .expect("Failed to query runs")
    {
        let id = row.get::<_, i32>(0) as u32;
        state.run_id = Some(id);
        set_meta(&mut state.client, "active_run_id", &id.to_string());
        return id;
    }

    // No runs at all — create one.
    let row = state.client
        .query_one(
            "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
            &[&player_name, &(unix_now() as i64)],
        )
        .expect("Failed to insert run");
    let id = row.get::<_, i32>(0) as u32;
    state.run_id = Some(id);
    set_meta(&mut state.client, "active_run_id", &id.to_string());
    id
}

/// Updates the player name once it is known from the game.
///
/// Stores the name in-process for tagging all subsequent DB writes, and updates
/// the run row if it still holds the placeholder 'Unknown'.
pub fn set_player_name(name: &str) {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    state.current_player = name.to_string();
    if let Some(id) = state.run_id {
        state.client
            .execute(
                "UPDATE runs SET player_name = $1 WHERE id = $2 AND player_name = 'Unknown'",
                &[&name, &(id as i32)],
            )
            .expect("Failed to update player name");
    }
}

/// Returns the run ID active in this process (or the last-written one from
/// the meta table, which is useful for the `--list-runs` display before
/// a run has been selected in the current session).
pub fn active_run_id() -> Option<u32> {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    state.run_id.or_else(|| {
        get_meta(&mut state.client, "active_run_id")
            .and_then(|v| v.parse().ok())
    })
}

/// Ends the active run by recording its end timestamp and clearing the
/// in-process run ID. Subsequent writes (deaths, encounters, catches)
/// will be silently dropped until a new run is started.
///
/// Returns the ID of the run that was ended, or `None` if no run was active.
pub fn end_run() -> Option<u32> {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let id = state.run_id.take()?;
    state.client.execute(
        "UPDATE runs SET ended_at = $1 WHERE id = $2",
        &[&(unix_now() as i64), &(id as i32)],
    ).expect("Failed to end run");
    delete_meta(&mut state.client, "active_run_id");
    Some(id)
}

/// Returns a summary of every run: `(id, player_name, started_at, dead_count)`.
pub fn list_runs() -> Vec<(u32, String, u64, usize)> {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    state.client
        .query(
            "SELECT r.id, r.player_name, r.started_at, COUNT(d.personality)
             FROM runs r
             LEFT JOIN dead_pokemon d ON d.run_id = r.id
             GROUP BY r.id
             ORDER BY r.id",
            &[],
        )
        .expect("Failed to query runs")
        .iter()
        .map(|row| (
            row.get::<_, i32>(0) as u32,
            row.get(1),
            row.get::<_, i64>(2) as u64,
            row.get::<_, i64>(3) as usize,
        ))
        .collect()
}

// ---------------------------------------------------------------------------
// Public API — death tracking
// ---------------------------------------------------------------------------

/// Records a Pokemon as permanently dead in the active run.
///
/// No-op if the Pokemon (identified by personality) is already recorded.
pub fn mark_dead(pokemon: DeadPokemon) {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return,
    };
    let player = state.current_player.clone();
    state.client.execute(
        "INSERT INTO dead_pokemon (
            run_id, player_name, personality, ot_id, ot_name, nickname,
            species, species_name, is_shiny, nature,
            level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
            move1, move2, move3, move4,
            pp1, pp2, pp3, pp4,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
            held_item, ability, ability_name, friendship, met_location, died_at, gender
        ) VALUES (
            $1,  $2,  $3,  $4,  $5,  $6,  $7,  $8,  $9,  $10, $11,
            $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22,
            $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33,
            $34, $35, $36, $37, $38, $39, $40, $41, $42, $43, $44, $45
        ) ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
            &player,  // $2 = player_name
            &(pokemon.personality as i64),
            &(pokemon.ot_id as i64),
            &pokemon.ot_name,
            &pokemon.nickname,
            &(pokemon.species as i32),
            &pokemon.species_name,
            &pokemon.is_shiny,
            &pokemon.nature,
            &(pokemon.level as i32),
            &(pokemon.experience as i64),
            &(pokemon.max_hp as i32),
            &(pokemon.attack as i32),
            &(pokemon.defense as i32),
            &(pokemon.speed as i32),
            &(pokemon.sp_attack as i32),
            &(pokemon.sp_defense as i32),
            &(pokemon.moves[0] as i32),
            &(pokemon.moves[1] as i32),
            &(pokemon.moves[2] as i32),
            &(pokemon.moves[3] as i32),
            &(pokemon.pp[0] as i32),
            &(pokemon.pp[1] as i32),
            &(pokemon.pp[2] as i32),
            &(pokemon.pp[3] as i32),
            &(pokemon.ivs.hp as i32),
            &(pokemon.ivs.attack as i32),
            &(pokemon.ivs.defense as i32),
            &(pokemon.ivs.speed as i32),
            &(pokemon.ivs.sp_attack as i32),
            &(pokemon.ivs.sp_defense as i32),
            &(pokemon.evs.hp as i32),
            &(pokemon.evs.attack as i32),
            &(pokemon.evs.defense as i32),
            &(pokemon.evs.speed as i32),
            &(pokemon.evs.sp_attack as i32),
            &(pokemon.evs.sp_defense as i32),
            &(pokemon.held_item as i32),
            &(pokemon.ability as i32),
            &pokemon.ability_name,
            &(pokemon.friendship as i32),
            &(pokemon.met_location as i32),
            &(pokemon.died_at as i64),
            &(pokemon.gender as i32),
        ],
    ).expect("Failed to insert dead pokemon");
}

/// Returns `true` if the Pokemon with this personality is dead in the active run.
pub fn is_dead(personality: u32) -> bool {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    query_is_dead(&mut state.client, active, personality)
}

/// Returns the stored `DeadPokemon` entry for this personality in the active run.
pub fn get_dead_pokemon(personality: u32) -> Option<DeadPokemon> {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = state.run_id?;
    let row = state.client
        .query_opt(
            "SELECT
                player_name, personality, ot_id, ot_name, nickname, species, species_name, is_shiny, nature,
                level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                move1, move2, move3, move4, pp1, pp2, pp3, pp4,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                held_item, ability, ability_name, friendship, met_location, died_at, gender
             FROM dead_pokemon
             WHERE run_id = $1 AND personality = $2",
            &[&(active as i32), &(personality as i64)],
        )
        .ok()??;

    Some(row_to_dead_pokemon(&row))
}

// ---------------------------------------------------------------------------
// Public API — catch tracking
// ---------------------------------------------------------------------------

/// Records a Pokemon as caught in the active run.
///
/// No-op if this personality is already recorded (deduplicates on reconnect).
pub fn mark_caught(pokemon: CaughtPokemon) {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return,
    };
    let player = state.current_player.clone();
    state.client.execute(
        "INSERT INTO caught_pokemon (
            run_id, player_name, personality, ot_id, nickname, species, species_name,
            is_shiny, nature, level, met_location,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            caught_at, gender
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
        ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
            &player,
            &(pokemon.personality as i64),
            &(pokemon.ot_id as i64),
            &pokemon.nickname,
            &(pokemon.species as i32),
            &pokemon.species_name,
            &pokemon.is_shiny,
            &pokemon.nature,
            &(pokemon.level as i32),
            &(pokemon.met_location as i32),
            &(pokemon.ivs.hp as i32),
            &(pokemon.ivs.attack as i32),
            &(pokemon.ivs.defense as i32),
            &(pokemon.ivs.speed as i32),
            &(pokemon.ivs.sp_attack as i32),
            &(pokemon.ivs.sp_defense as i32),
            &(pokemon.caught_at as i64),
            &(pokemon.gender as i32),
        ],
    ).expect("Failed to insert caught pokemon");
}

/// Returns `true` if a Pokemon with this personality has been caught in the active run.
pub fn is_caught(personality: u32) -> bool {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    state.client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon WHERE run_id = $1 AND personality = $2",
            &[&(active as i32), &(personality as i64)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns all caught Pokemon for the active run for the current player.
pub fn list_caught() -> Vec<CaughtPokemon> {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return vec![],
    };
    let player = state.current_player.clone();
    query_caught(&mut state.client, active, &player)
}

// ---------------------------------------------------------------------------
// Public API — encounter tracking
// ---------------------------------------------------------------------------

/// Records the first wild encounter in an area for the current player.
///
/// Subsequent encounters in the same area by the same player are silently
/// ignored (Nuzlocke rule). Returns `true` if this was a new encounter.
pub fn record_encounter(encounter: Encounter) -> bool {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();
    let rows = state.client.execute(
        "INSERT INTO encounters (
            run_id, player_name, map_group, map_name, species, species_name, level, caught, encountered_at
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, $8)
         ON CONFLICT (run_id, player_name, map_group, map_name) DO NOTHING",
        &[
            &(active as i32),
            &player,
            &(encounter.map_group as i32),
            &(encounter.map_name as i32),
            &(encounter.species as i32),
            &encounter.species_name,
            &(encounter.level as i32),
            &(encounter.encountered_at as i64),
        ],
    ).unwrap_or(0);
    rows == 1
}

/// Marks the current player's encounter for this area as successfully caught.
pub fn set_encounter_caught(map_group: u8, map_name: u8) {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return,
    };
    let player = state.current_player.clone();
    if let Err(e) = state.client.execute(
        "UPDATE encounters SET caught = TRUE
         WHERE run_id = $1 AND player_name = $2 AND map_group = $3 AND map_name = $4",
        &[&(active as i32), &player, &(map_group as i32), &(map_name as i32)],
    ) {
        eprintln!("set_encounter_caught: DB error: {}", e);
    }
}

/// Returns `true` if an encounter has already been recorded for this area by the current player.
pub fn has_encounter(map_group: u8, map_name: u8) -> bool {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();
    state.client
        .query_one(
            "SELECT COUNT(*) FROM encounters
             WHERE run_id = $1 AND player_name = $2 AND map_group = $3 AND map_name = $4",
            &[&(active as i32), &player, &(map_group as i32), &(map_name as i32)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns all encounters for the active run, ordered by time.
pub fn list_encounters() -> Vec<Encounter> {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return vec![],
    };
    let player = state.current_player.clone();
    state.client
        .query(
            "SELECT player_name, map_group, map_name, species, species_name, level, caught, encountered_at
             FROM encounters
             WHERE run_id = $1 AND player_name = $2
             ORDER BY encountered_at ASC",
            &[&(active as i32), &player],
        )
        .unwrap_or_default()
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
        })
        .collect()
}

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
    client:      Mutex<Client>,
    run_id:      Mutex<Option<u32>>,
    last_player: Mutex<String>,
    dirty:       std::sync::atomic::AtomicBool,
    /// `true` when the tracked run has `ended_at IS NULL` (currently active).
    is_active:   std::sync::atomic::AtomicBool,
}

impl DbReader {
    /// Opens a connection to the PostgreSQL server.
    ///
    /// Returns `None` if the server is unreachable.
    pub fn open(connection_string: &str) -> Option<Self> {
        let client = match Client::connect(connection_string, NoTls) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("DB connection failed ({}): {}", connection_string, e);
                return None;
            }
        };
        eprintln!("DB connected: {}", connection_string);
        Some(Self {
            client:      Mutex::new(client),
            run_id:      Mutex::new(None),
            last_player: Mutex::new(String::new()),
            dirty:       std::sync::atomic::AtomicBool::new(false),
            is_active:   std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Forces the next `sync_player` call to re-query the database even if the
    /// player name has not changed. Call this after a run is ended or started
    /// remotely so the cached run ID is immediately refreshed.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Returns the active run ID if the tracked run has not been ended, else `None`.
    pub fn active_run_id(&self) -> Option<u32> {
        if self.is_active.load(std::sync::atomic::Ordering::SeqCst) {
            *self.run_id.lock().unwrap_or_else(|e| e.into_inner())
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
        let forced = self.dirty.swap(false, std::sync::atomic::Ordering::SeqCst);
        if !forced {
            let last = self.last_player.lock().unwrap_or_else(|e| e.into_inner());
            if *last == player_name { return false; }
        }

        let row = self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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

        self.is_active.store(active, std::sync::atomic::Ordering::SeqCst);
        let old_id = *self.run_id.lock().unwrap_or_else(|e| e.into_inner());
        *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) = new_id;
        if new_id.is_some() || forced {
            *self.last_player.lock().unwrap_or_else(|e| e.into_inner()) = player_name.to_string();
        }
        new_id != old_id
    }

    /// Returns caught Pokemon for the active run belonging to `player_name`.
    pub fn list_caught(&self, player_name: &str) -> Vec<CaughtPokemon> {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return vec![],
        };
        query_caught(&mut *self.client.lock().unwrap_or_else(|e| e.into_inner()), run_id, player_name)
    }

    /// Returns `true` if the Pokemon with this personality is dead in the active run.
    pub fn is_dead(&self, personality: u32) -> bool {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return false,
        };
        query_is_dead(&mut *self.client.lock().unwrap_or_else(|e| e.into_inner()), run_id, personality)
    }

    /// Returns dead Pokemon for the active run belonging to `player_name`, keyed by personality.
    pub fn list_dead_with_records(&self, player_name: &str) -> HashMap<u32, DeadPokemon> {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return HashMap::new(),
        };
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query(
                "SELECT
                    player_name, personality, ot_id, ot_name, nickname, species, species_name, is_shiny, nature,
                    level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                    move1, move2, move3, move4, pp1, pp2, pp3, pp4,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                    held_item, ability, ability_name, friendship, met_location, died_at, gender
                 FROM dead_pokemon WHERE run_id = $1 AND player_name = $2",
                &[&(run_id as i32), &player_name],
            )
            .unwrap_or_default()
            .iter()
            .map(|row| {
                let dp = row_to_dead_pokemon(row);
                (dp.personality, dp)
            })
            .collect()
    }

    /// Returns recorded first encounters for the active run belonging to `player_name`.
    pub fn list_encounters(&self, player_name: &str) -> Vec<Encounter> {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return vec![],
        };
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query(
                "SELECT player_name, map_group, map_name, species, species_name, level, caught, encountered_at
                 FROM encounters WHERE run_id = $1 AND player_name = $2 ORDER BY encountered_at ASC",
                &[&(run_id as i32), &player_name],
            )
            .unwrap_or_default()
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
            })
            .collect()
    }

    /// Returns a summary of the tracked run: player name, start/end times,
    /// death count, and catch count. Returns `None` if no run is tracked yet.
    pub fn run_summary(&self) -> Option<(u32, String, u64, Option<u64>, usize, usize)> {
        let run_id = (*self.run_id.lock().unwrap_or_else(|e| e.into_inner()))?;
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
            .map(|row| (
                run_id,
                row.get::<_, String>(0),
                row.get::<_, i64>(1) as u64,
                row.get::<_, Option<i64>>(2).map(|v| v as u64),
                row.get::<_, i64>(3) as usize,
                row.get::<_, i64>(4) as usize,
            ))
    }

    /// Inserts a soul-link death record for `caught` in this player's active run.
    ///
    /// Battle stats (HP, Attack, etc.) are stored as 0 to signal a soul-link
    /// kill rather than a direct in-game death. Safe to call if the record
    /// already exists — the insert is a no-op in that case.
    /// Returns `true` if the insert was attempted (run_id was known), `false` if
    /// the run has not been identified yet (caller should retry next frame).
    pub fn mark_soul_link_dead(&self, caught: &CaughtPokemon) -> bool {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return false,
        };
        let now = unix_now();
        if let Err(e) = self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .execute(
                "INSERT INTO dead_pokemon (
                    run_id, personality, ot_id, ot_name, nickname,
                    species, species_name, is_shiny, nature,
                    level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                    move1, move2, move3, move4,
                    pp1, pp2, pp3, pp4,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                    held_item, ability, ability_name, friendship, met_location, died_at, gender
                ) VALUES (
                    $1, $2, $3, '', $4, $5, $6, $7, $8, $9,
                    0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0,
                    0, 0, 0, 0,
                    $10, $11, $12, $13, $14, $15,
                    0, 0, 0, 0, 0, 0,
                    0, 0, '', 0, $16, $17, $18
                ) ON CONFLICT (run_id, personality) DO NOTHING",
                &[
                    &(run_id as i32),
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
            )
        {
            eprintln!("mark_caught: DB error: {}", e);
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Full database dump — used by the DB viewer web page
// ---------------------------------------------------------------------------

/// Opens a fresh connection and returns a JSON snapshot of every table.
///
/// Intended for the `/db.json` endpoint; opens its own connection so the live
/// tracker connections are not blocked. Returns a JSON error object on failure.
pub fn dump_all(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c)  => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let runs       = dump_runs(&mut client);
    let caught     = dump_caught(&mut client);
    let dead       = dump_dead(&mut client);
    let encounters = dump_encounters(&mut client);

    serde_json::json!({ "runs": runs, "caught": caught, "dead": dead, "encounters": encounters })
}

fn dump_runs(client: &mut Client) -> serde_json::Value {
    let rows = client.query(
        "SELECT r.id, r.player_name, r.started_at, r.ended_at,
                COUNT(DISTINCT d.personality) AS deaths,
                COUNT(DISTINCT c.personality) AS catches,
                COUNT(DISTINCT e.id) AS encounters
         FROM runs r
         LEFT JOIN dead_pokemon d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         LEFT JOIN encounters e ON e.run_id = r.id
         GROUP BY r.id ORDER BY r.id",
        &[],
    ).unwrap_or_default();

    serde_json::Value::Array(rows.iter().map(|row| {
        let ended: Option<i64> = row.get(3);
        serde_json::json!({
            "id":         row.get::<_, i32>(0),
            "player":     row.get::<_, String>(1),
            "started":    format_timestamp(row.get::<_, i64>(2) as u64),
            "ended":      ended.map(|t| format_timestamp(t as u64)),
            "deaths":     row.get::<_, i64>(4),
            "catches":    row.get::<_, i64>(5),
            "encounters": row.get::<_, i64>(6),
        })
    }).collect())
}

fn dump_caught(client: &mut Client) -> serde_json::Value {
    let rows = client.query(
        "SELECT run_id, player_name, nickname, species_name, level, nature, is_shiny,
                met_location,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                caught_at, gender
         FROM caught_pokemon ORDER BY caught_at ASC",
        &[],
    ).unwrap_or_default();

    serde_json::Value::Array(rows.iter().map(|row| {
        serde_json::json!({
            "run_id":    row.get::<_, i32>(0),
            "player":    row.get::<_, String>(1),
            "nickname":  row.get::<_, String>(2),
            "species":   row.get::<_, String>(3),
            "level":     row.get::<_, i32>(4),
            "nature":    row.get::<_, String>(5),
            "shiny":     row.get::<_, bool>(6),
            "location":  row.get::<_, i32>(7),
            "ivs":       format!("{}/{}/{}/{}/{}/{}",
                row.get::<_, i32>(8),  row.get::<_, i32>(9),  row.get::<_, i32>(10),
                row.get::<_, i32>(11), row.get::<_, i32>(12), row.get::<_, i32>(13)),
            "caught_at": format_timestamp(row.get::<_, i64>(14) as u64),
            "gender":    row.get::<_, i32>(15),
        })
    }).collect())
}

fn dump_dead(client: &mut Client) -> serde_json::Value {
    let rows = client.query(
        "SELECT run_id, player_name, nickname, species_name, level, nature, is_shiny,
                max_hp, attack, defense, speed, sp_attack, sp_defense,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                (max_hp = 0) AS soul_link,
                died_at, gender
         FROM dead_pokemon ORDER BY died_at ASC",
        &[],
    ).unwrap_or_default();

    serde_json::Value::Array(rows.iter().map(|row| {
        serde_json::json!({
            "run_id":    row.get::<_, i32>(0),
            "player":    row.get::<_, String>(1),
            "nickname":  row.get::<_, String>(2),
            "species":   row.get::<_, String>(3),
            "level":     row.get::<_, i32>(4),
            "nature":    row.get::<_, String>(5),
            "shiny":     row.get::<_, bool>(6),
            "stats":     format!("{}/{}/{}/{}/{}/{}",
                row.get::<_, i32>(7),  row.get::<_, i32>(8),  row.get::<_, i32>(9),
                row.get::<_, i32>(10), row.get::<_, i32>(11), row.get::<_, i32>(12)),
            "ivs":       format!("{}/{}/{}/{}/{}/{}",
                row.get::<_, i32>(13), row.get::<_, i32>(14), row.get::<_, i32>(15),
                row.get::<_, i32>(16), row.get::<_, i32>(17), row.get::<_, i32>(18)),
            "soul_link": row.get::<_, bool>(19),
            "died_at":   format_timestamp(row.get::<_, i64>(20) as u64),
            "gender":    row.get::<_, i32>(21),
        })
    }).collect())
}

fn dump_encounters(client: &mut Client) -> serde_json::Value {
    let rows = client.query(
        "SELECT run_id, player_name, map_group, map_name, species_name, level, caught, encountered_at
         FROM encounters ORDER BY encountered_at ASC",
        &[],
    ).unwrap_or_default();

    serde_json::Value::Array(rows.iter().map(|row| {
        let group = row.get::<_, i32>(2) as u8;
        let map   = row.get::<_, i32>(3) as u8;
        let name  = fire_red_location_names::map_area_name(group, map);
        let area  = if name.is_empty() {
            format!("{}:{}", group, map)
        } else {
            name.to_string()
        };
        serde_json::json!({
            "run_id":  row.get::<_, i32>(0),
            "player":  row.get::<_, String>(1),
            "area":    area,
            "species": row.get::<_, String>(4),
            "level":   row.get::<_, i32>(5),
            "caught":  row.get::<_, bool>(6),
            "seen_at": format_timestamp(row.get::<_, i64>(7) as u64),
        })
    }).collect())
}
