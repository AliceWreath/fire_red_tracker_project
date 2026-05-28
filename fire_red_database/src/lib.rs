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
}

/// Snapshot of a Pokemon at the moment it first joined the party.
#[derive(Clone, Debug)]
pub struct CaughtPokemon {
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
}

// ---------------------------------------------------------------------------
// Storage
//
// The active run ID is stored in process memory alongside the connection.
// This means two tracker processes can safely share the same PostgreSQL
// database — each manages its own run independently without overwriting a
// global "active" pointer in the database.
// ---------------------------------------------------------------------------

struct DbState {
    client: Client,
    /// Which run this process is currently recording into.
    run_id: Option<u32>,
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

        -- Stores the last-active run_id per process for the --list-runs display.
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
    ").expect("Failed to create database schema");

    DB.set(Mutex::new(DbState { client, run_id: None })).ok();
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
    client
        .execute(
            "INSERT INTO meta (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            &[&key, &value],
        )
        .expect("Failed to write meta");
}

fn query_caught(client: &mut Client, run_id: u32) -> Vec<CaughtPokemon> {
    client
        .query(
            "SELECT personality, ot_id, nickname, species, species_name,
                    is_shiny, nature, level, met_location,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    caught_at
             FROM caught_pokemon
             WHERE run_id = $1
             ORDER BY caught_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default()
        .iter()
        .map(|row| CaughtPokemon {
            personality:  row.get::<_, i64>(0) as u32,
            ot_id:        row.get::<_, i64>(1) as u32,
            nickname:     row.get(2),
            species:      row.get::<_, i32>(3) as u16,
            species_name: row.get(4),
            is_shiny:     row.get(5),
            nature:       row.get(6),
            level:        row.get::<_, i32>(7) as u8,
            met_location: row.get::<_, i32>(8) as u8,
            ivs: IVs {
                hp:         row.get::<_, i32>(9)  as u8,
                attack:     row.get::<_, i32>(10) as u8,
                defense:    row.get::<_, i32>(11) as u8,
                speed:      row.get::<_, i32>(12) as u8,
                sp_attack:  row.get::<_, i32>(13) as u8,
                sp_defense: row.get::<_, i32>(14) as u8,
            },
            caught_at: row.get::<_, i64>(15) as u64,
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

/// Converts a dead_pokemon SELECT row (columns in the order used by all queries
/// in this file) into a [`DeadPokemon`].
fn row_to_dead_pokemon(row: &postgres::Row) -> DeadPokemon {
    DeadPokemon {
        personality:  row.get::<_, i64>(0) as u32,
        ot_id:        row.get::<_, i64>(1) as u32,
        ot_name:      row.get(2),
        nickname:     row.get(3),
        species:      row.get::<_, i32>(4) as u16,
        species_name: row.get(5),
        is_shiny:     row.get(6),
        nature:       row.get(7),
        level:        row.get::<_, i32>(8) as u8,
        experience:   row.get::<_, i64>(9) as u32,
        max_hp:       row.get::<_, i32>(10) as u16,
        attack:       row.get::<_, i32>(11) as u16,
        defense:      row.get::<_, i32>(12) as u16,
        speed:        row.get::<_, i32>(13) as u16,
        sp_attack:    row.get::<_, i32>(14) as u16,
        sp_defense:   row.get::<_, i32>(15) as u16,
        moves: [
            row.get::<_, i32>(16) as u16,
            row.get::<_, i32>(17) as u16,
            row.get::<_, i32>(18) as u16,
            row.get::<_, i32>(19) as u16,
        ],
        pp: [
            row.get::<_, i32>(20) as u8,
            row.get::<_, i32>(21) as u8,
            row.get::<_, i32>(22) as u8,
            row.get::<_, i32>(23) as u8,
        ],
        ivs: IVs {
            hp:         row.get::<_, i32>(24) as u8,
            attack:     row.get::<_, i32>(25) as u8,
            defense:    row.get::<_, i32>(26) as u8,
            speed:      row.get::<_, i32>(27) as u8,
            sp_attack:  row.get::<_, i32>(28) as u8,
            sp_defense: row.get::<_, i32>(29) as u8,
        },
        evs: EVs {
            hp:         row.get::<_, i32>(30) as u8,
            attack:     row.get::<_, i32>(31) as u8,
            defense:    row.get::<_, i32>(32) as u8,
            speed:      row.get::<_, i32>(33) as u8,
            sp_attack:  row.get::<_, i32>(34) as u8,
            sp_defense: row.get::<_, i32>(35) as u8,
        },
        held_item:    row.get::<_, i32>(36) as u16,
        ability:      row.get::<_, i32>(37) as u8,
        ability_name: row.get(38),
        friendship:   row.get::<_, i32>(39) as u8,
        met_location: row.get::<_, i32>(40) as u8,
        died_at:      row.get::<_, i64>(41) as u64,
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

    // Fall back to the most recently created run.
    if let Some(row) = state.client
        .query_opt("SELECT id FROM runs ORDER BY id DESC LIMIT 1", &[])
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

/// Updates the player name on the active run once it is known from the game.
///
/// Only writes if the run name is still 'Unknown' — this prevents a second
/// tracker process (soul-link partner) from overwriting the first player's name
/// and breaking the aggregator's run lookup.
pub fn set_player_name(name: &str) {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
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
    state.client.execute(
        "INSERT INTO dead_pokemon (
            run_id, personality, ot_id, ot_name, nickname,
            species, species_name, is_shiny, nature,
            level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
            move1, move2, move3, move4,
            pp1, pp2, pp3, pp4,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
            held_item, ability, ability_name, friendship, met_location, died_at
        ) VALUES (
            $1,  $2,  $3,  $4,  $5,  $6,  $7,  $8,  $9,  $10, $11,
            $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22,
            $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33,
            $34, $35, $36, $37, $38, $39, $40, $41, $42, $43
        ) ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
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
                personality, ot_id, ot_name, nickname, species, species_name, is_shiny, nature,
                level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                move1, move2, move3, move4, pp1, pp2, pp3, pp4,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                held_item, ability, ability_name, friendship, met_location, died_at
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
    state.client.execute(
        "INSERT INTO caught_pokemon (
            run_id, personality, ot_id, nickname, species, species_name,
            is_shiny, nature, level, met_location,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            caught_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
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

/// Returns all caught Pokemon for the active run, ordered by catch time.
pub fn list_caught() -> Vec<CaughtPokemon> {
    let mut state = db().lock().unwrap_or_else(|e| e.into_inner());
    let active = match state.run_id {
        Some(id) => id,
        None => return vec![],
    };
    query_caught(&mut state.client, active)
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
        })
    }

    /// Updates the cached run ID to the most recent run in the database.
    ///
    /// `player_name` is used only to avoid re-querying every frame — the lookup
    /// itself picks the most recent run regardless of name. This allows both
    /// players in a soul-link run to resolve to the same shared run even though
    /// only one player's name is stored on the run row.
    ///
    /// Returns `true` if the run ID actually changed (including the first time
    /// a run is successfully resolved). Safe to call every frame.
    pub fn sync_player(&self, player_name: &str) -> bool {
        {
            let last = self.last_player.lock().unwrap_or_else(|e| e.into_inner());
            if *last == player_name { return false; }
        }
        let new_id = self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query_opt(
                "SELECT id FROM runs ORDER BY id DESC LIMIT 1",
                &[],
            )
            .ok()
            .flatten()
            .map(|row| row.get::<_, i32>(0) as u32);

        let old_id = *self.run_id.lock().unwrap_or_else(|e| e.into_inner());
        *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) = new_id;
        // Only cache the name on success so that a failed lookup retries every
        // frame (the tracker may not have written the player name yet).
        if new_id.is_some() {
            *self.last_player.lock().unwrap_or_else(|e| e.into_inner()) = player_name.to_string();
        }
        new_id != old_id
    }

    /// Returns all caught Pokemon for the active run, ordered by catch time.
    pub fn list_caught(&self) -> Vec<CaughtPokemon> {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return vec![],
        };
        query_caught(&mut *self.client.lock().unwrap_or_else(|e| e.into_inner()), run_id)
    }

    /// Returns `true` if the Pokemon with this personality is dead in the active run.
    pub fn is_dead(&self, personality: u32) -> bool {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return false,
        };
        query_is_dead(&mut *self.client.lock().unwrap_or_else(|e| e.into_inner()), run_id, personality)
    }

    /// Returns all dead Pokemon for the active run, keyed by personality.
    pub fn list_dead_with_records(&self) -> HashMap<u32, DeadPokemon> {
        let run_id = match *self.run_id.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(id) => id,
            None => return HashMap::new(),
        };
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .query(
                "SELECT
                    personality, ot_id, ot_name, nickname, species, species_name, is_shiny, nature,
                    level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                    move1, move2, move3, move4, pp1, pp2, pp3, pp4,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                    held_item, ability, ability_name, friendship, met_location, died_at
                 FROM dead_pokemon WHERE run_id = $1",
                &[&(run_id as i32)],
            )
            .unwrap_or_default()
            .iter()
            .map(|row| {
                let dp = row_to_dead_pokemon(row);
                (dp.personality, dp)
            })
            .collect()
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
        self.client
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
                    held_item, ability, ability_name, friendship, met_location, died_at
                ) VALUES (
                    $1, $2, $3, '', $4, $5, $6, $7, $8, $9,
                    0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0,
                    0, 0, 0, 0,
                    $10, $11, $12, $13, $14, $15,
                    0, 0, 0, 0, 0, 0,
                    0, 0, '', 0, $16, $17
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
                ],
            )
            .ok();
        true
    }
}
