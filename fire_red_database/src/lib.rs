use postgres::{Client, NoTls};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for Mutex<T> {
    #[track_caller]
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            let loc = std::panic::Location::caller();
            eprintln!("Warning: mutex poisoned at {}:{}: {e}", loc.file(), loc.line());
            e.into_inner()
        })
    }
}

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

const MOVES: [&str; 355] = [
    "—",              // 0
    "Pound",          "Karate Chop",    "Double Slap",    "Comet Punch",    "Mega Punch",     // 1–5
    "Pay Day",        "Fire Punch",     "Ice Punch",      "Thunder Punch",  "Scratch",        // 6–10
    "Vice Grip",      "Guillotine",     "Razor Wind",     "Swords Dance",   "Cut",            // 11–15
    "Gust",           "Wing Attack",    "Whirlwind",      "Fly",            "Bind",           // 16–20
    "Slam",           "Vine Whip",      "Stomp",          "Double Kick",    "Mega Kick",      // 21–25
    "Jump Kick",      "Rolling Kick",   "Sand Attack",    "Headbutt",       "Horn Attack",    // 26–30
    "Fury Attack",    "Horn Drill",     "Tackle",         "Body Slam",      "Wrap",           // 31–35
    "Take Down",      "Thrash",         "Double-Edge",    "Tail Whip",      "Poison Sting",   // 36–40
    "Twineedle",      "Pin Missile",    "Leer",           "Bite",           "Growl",          // 41–45
    "Roar",           "Sing",           "Supersonic",     "Sonic Boom",     "Disable",        // 46–50
    "Acid",           "Ember",          "Flamethrower",   "Mist",           "Water Gun",      // 51–55
    "Hydro Pump",     "Surf",           "Ice Beam",       "Blizzard",       "Psybeam",        // 56–60
    "Bubble Beam",    "Aurora Beam",    "Hyper Beam",     "Peck",           "Drill Peck",     // 61–65
    "Submission",     "Low Kick",       "Counter",        "Seismic Toss",   "Strength",       // 66–70
    "Absorb",         "Mega Drain",     "Leech Seed",     "Growth",         "Razor Leaf",     // 71–75
    "Solar Beam",     "Poison Powder",  "Stun Spore",     "Sleep Powder",   "Petal Dance",    // 76–80
    "String Shot",    "Dragon Rage",    "Fire Spin",      "Thunder Shock",  "Thunderbolt",    // 81–85
    "Thunder Wave",   "Thunder",        "Rock Throw",     "Earthquake",     "Fissure",        // 86–90
    "Dig",            "Toxic",          "Confusion",      "Psychic",        "Hypnosis",       // 91–95
    "Meditate",       "Agility",        "Quick Attack",   "Rage",           "Teleport",       // 96–100
    "Night Shade",    "Mimic",          "Screech",        "Double Team",    "Recover",        // 101–105
    "Harden",         "Minimize",       "Smokescreen",    "Confuse Ray",    "Withdraw",       // 106–110
    "Defense Curl",   "Barrier",        "Light Screen",   "Haze",           "Reflect",        // 111–115
    "Focus Energy",   "Bide",           "Metronome",      "Mirror Move",    "Self-Destruct",  // 116–120
    "Egg Bomb",       "Lick",           "Smog",           "Sludge",         "Bone Club",      // 121–125
    "Fire Blast",     "Waterfall",      "Clamp",          "Swift",          "Skull Bash",     // 126–130
    "Spike Cannon",   "Constrict",      "Amnesia",        "Kinesis",        "Soft-Boiled",    // 131–135
    "High Jump Kick", "Glare",          "Dream Eater",    "Poison Gas",     "Barrage",        // 136–140
    "Leech Life",     "Lovely Kiss",    "Sky Attack",     "Transform",      "Bubble",         // 141–145
    "Dizzy Punch",    "Spore",          "Flash",          "Psywave",        "Splash",         // 146–150
    "Acid Armor",     "Crabhammer",     "Explosion",      "Fury Swipes",    "Bonemerang",     // 151–155
    "Rest",           "Rock Slide",     "Hyper Fang",     "Sharpen",        "Conversion",     // 156–160
    "Tri Attack",     "Super Fang",     "Slash",          "Substitute",     "Struggle",       // 161–165
    "Sketch",         "Triple Kick",    "Thief",          "Spider Web",     "Mind Reader",    // 166–170
    "Nightmare",      "Flame Wheel",    "Snore",          "Curse",          "Flail",          // 171–175
    "Conversion 2",   "Aeroblast",      "Cotton Spore",   "Reversal",       "Spite",          // 176–180
    "Powder Snow",    "Protect",        "Mach Punch",     "Scary Face",     "Feint Attack",   // 181–185
    "Sweet Kiss",     "Belly Drum",     "Sludge Bomb",    "Mud-Slap",       "Octazooka",      // 186–190
    "Spikes",         "Zap Cannon",     "Foresight",      "Destiny Bond",   "Perish Song",    // 191–195
    "Icy Wind",       "Detect",         "Bone Rush",      "Lock-On",        "Outrage",        // 196–200
    "Sandstorm",      "Giga Drain",     "Endure",         "Charm",          "Rollout",        // 201–205
    "False Swipe",    "Swagger",        "Milk Drink",     "Spark",          "Fury Cutter",    // 206–210
    "Steel Wing",     "Mean Look",      "Attract",        "Sleep Talk",     "Heal Bell",      // 211–215
    "Return",         "Present",        "Frustration",    "Safeguard",      "Pain Split",     // 216–220
    "Sacred Fire",    "Magnitude",      "Dynamic Punch",  "Megahorn",       "Dragon Breath",  // 221–225
    "Baton Pass",     "Encore",         "Pursuit",        "Rapid Spin",     "Sweet Scent",    // 226–230
    "Iron Tail",      "Metal Claw",     "Vital Throw",    "Morning Sun",    "Synthesis",      // 231–235
    "Moonlight",      "Hidden Power",   "Cross Chop",     "Twister",        "Rain Dance",     // 236–240
    "Sunny Day",      "Crunch",         "Mirror Coat",    "Psych Up",       "Extreme Speed",  // 241–245
    "Ancient Power",  "Shadow Ball",    "Future Sight",   "Rock Smash",     "Whirlpool",      // 246–250
    "Beat Up",        "Fake Out",       "Uproar",         "Stockpile",      "Spit Up",        // 251–255
    "Swallow",        "Heat Wave",      "Hail",           "Torment",        "Flatter",        // 256–260
    "Will-O-Wisp",    "Memento",        "Facade",         "Focus Punch",    "Smelling Salts", // 261–265
    "Follow Me",      "Nature Power",   "Charge",         "Taunt",          "Helping Hand",   // 266–270
    "Trick",          "Role Play",      "Wish",           "Assist",         "Ingrain",        // 271–275
    "Superpower",     "Magic Coat",     "Recycle",        "Revenge",        "Brick Break",    // 276–280
    "Yawn",           "Knock Off",      "Endeavor",       "Eruption",       "Skill Swap",     // 281–285
    "Imprison",       "Refresh",        "Grudge",         "Snatch",         "Secret Power",   // 286–290
    "Dive",           "Arm Thrust",     "Camouflage",     "Tail Glow",      "Luster Purge",   // 291–295
    "Mist Ball",      "Feather Dance",  "Teeter Dance",   "Blaze Kick",     "Mud Sport",      // 296–300
    "Ice Ball",       "Needle Arm",     "Slack Off",      "Hyper Voice",    "Poison Fang",    // 301–305
    "Crush Claw",     "Blast Burn",     "Hydro Cannon",   "Meteor Mash",    "Astonish",       // 306–310
    "Weather Ball",   "Aromatherapy",   "Fake Tears",     "Air Cutter",     "Overheat",       // 311–315
    "Odor Sleuth",    "Rock Tomb",      "Silver Wind",    "Metal Sound",    "Grass Whistle",  // 316–320
    "Tickle",         "Cosmic Power",   "Water Spout",    "Signal Beam",    "Shadow Punch",   // 321–325
    "Extrasensory",   "Sky Uppercut",   "Sand Tomb",      "Sheer Cold",     "Muddy Water",    // 326–330
    "Bullet Seed",    "Aerial Ace",     "Icicle Spear",   "Iron Defense",   "Block",          // 331–335
    "Howl",           "Dragon Claw",    "Frenzy Plant",   "Bulk Up",        "Bounce",         // 336–340
    "Mud Shot",       "Poison Tail",    "Covet",          "Volt Tackle",    "Magical Leaf",   // 341–345
    "Water Sport",    "Calm Mind",      "Leaf Blade",     "Dragon Dance",   "Rock Blast",     // 346–350
    "Shock Wave",     "Water Pulse",    "Doom Desire",    "Psycho Boost",                     // 351–354
];

/// Returns the name of the move with the given Gen III index (1–354).
/// Returns `"—"` for index 0 (empty slot) or any out-of-range value.
pub fn move_name(id: u16) -> &'static str {
    MOVES.get(id as usize).copied().unwrap_or("—")
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
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
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
    pub is_shiny: bool,
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
    pub level:         u8,
    pub met_location:  u8,
    /// Human-readable location name resolved at catch time from the current map
    /// (group, map_name) coordinates. Empty for records created before this field
    /// was added.
    pub location_name: String,
    pub ivs:           IVs,
    pub evs:           EVs,
    pub caught_at:     u64,
    /// `0` = male, `1` = female, `2` = genderless.
    pub gender:        u8,
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
///
/// # Errors
///
/// Returns an error string if the connection fails, schema setup fails, or
/// `initialize` has already been called.
/// Increment this whenever a new migration or data-repair statement is added.
/// `initialize` skips all SQL when the DB already records this version.
const SCHEMA_VERSION: &str = "2";

pub fn initialize(connection_string: &str) -> Result<(), String> {
    let mut client = Client::connect(connection_string, NoTls)
        .map_err(|e| format!(
            "Failed to connect to PostgreSQL: {e}\n\
             Ensure the server is reachable and the database exists.\n\
             Create it with:  psql -c 'CREATE DATABASE nuzlocke;'"
        ))?;

    // If the meta table already records the current schema version, all
    // migrations have already been applied — skip the batch entirely.
    let already_current = client
        .query_opt(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            &[],
        )
        .ok()
        .flatten()
        .map(|row| row.get::<_, String>(0))
        .as_deref() == Some(SCHEMA_VERSION);

    if already_current {
        return DB
            .set(Mutex::new(DbState { client, run_id: None, current_player: String::new() }))
            .map_err(|_| "fire_red_database::initialize called more than once".to_string());
    }

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

        -- Migration: add human-readable location name resolved at catch time.
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS location_name TEXT NOT NULL DEFAULT '';

        -- Migration: record whether a first encounter was shiny.
        ALTER TABLE encounters ADD COLUMN IF NOT EXISTS is_shiny BOOLEAN NOT NULL DEFAULT FALSE;

        -- Data repair: assign caught_pokemon records with a blank player_name to the
        -- correct player, using the encounters table as the source of truth.
        -- Only runs for single-player runs (one distinct player in encounters) where
        -- the assignment is unambiguous.
        UPDATE caught_pokemon cp
        SET player_name = sub.player_name
        FROM (
            SELECT run_id, MIN(player_name) AS player_name
            FROM encounters
            WHERE player_name != ''
            GROUP BY run_id
            HAVING COUNT(DISTINCT player_name) = 1
        ) sub
        WHERE cp.run_id = sub.run_id AND cp.player_name = '';

        -- Migration: add EV columns to caught_pokemon.
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS ev_hp        INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS ev_attack    INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS ev_defense   INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS ev_speed     INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS ev_sp_attack INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS ev_sp_defense INTEGER NOT NULL DEFAULT 0;

        -- Data repair: fix species_name for NIDORAN♀ (29) and NIDORAN♂ (32) that were
        -- stored without the gender symbol due to a bug in the GBA text decoder.
        -- Only updates rows that don't already contain a gender symbol.
        UPDATE encounters     SET species_name = 'NIDORAN♀' WHERE species = 29 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE encounters     SET species_name = 'NIDORAN♂' WHERE species = 32 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE caught_pokemon SET species_name = 'NIDORAN♀' WHERE species = 29 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE caught_pokemon SET species_name = 'NIDORAN♂' WHERE species = 32 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE dead_pokemon   SET species_name = 'NIDORAN♀' WHERE species = 29 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE dead_pokemon   SET species_name = 'NIDORAN♂' WHERE species = 32 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
    ").map_err(|e| format!("Failed to create database schema: {e}"))?;

    // Record the schema version so future startups can skip all migrations.
    client.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', $1)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        &[&SCHEMA_VERSION],
    ).map_err(|e| format!("Failed to write schema_version: {e}"))?;

    DB.set(Mutex::new(DbState { client, run_id: None, current_player: String::new() }))
        .map_err(|_| "fire_red_database::initialize called more than once".to_string())?;

    Ok(())
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
                    caught_at, gender, location_name,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense
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
            met_location:  row.get::<_, i32>(9) as u8,
            location_name: row.get::<_, String>(18),
            ivs: IVs {
                hp:         row.get::<_, i32>(10) as u8,
                attack:     row.get::<_, i32>(11) as u8,
                defense:    row.get::<_, i32>(12) as u8,
                speed:      row.get::<_, i32>(13) as u8,
                sp_attack:  row.get::<_, i32>(14) as u8,
                sp_defense: row.get::<_, i32>(15) as u8,
            },
            evs: EVs {
                hp:         row.get::<_, i32>(19) as u8,
                attack:     row.get::<_, i32>(20) as u8,
                defense:    row.get::<_, i32>(21) as u8,
                speed:      row.get::<_, i32>(22) as u8,
                sp_attack:  row.get::<_, i32>(23) as u8,
                sp_defense: row.get::<_, i32>(24) as u8,
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
pub fn new_run(player_name: &str) -> Result<u32, String> {
    let mut state = db().lock_or_recover();
    let row = state.client
        .query_one(
            "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
            &[&player_name, &(unix_now() as i64)],
        )
        .map_err(|e| format!("Failed to insert run: {e}"))?;
    let id = row.get::<_, i32>(0) as u32;
    state.run_id = Some(id);
    set_meta(&mut state.client, "active_run_id", &id.to_string());
    Ok(id)
}

/// Switches the active run for this process to an existing run by ID.
///
/// Returns `Ok(false)` if no run with that ID exists.
pub fn resume_run(id: u32) -> Result<bool, String> {
    let mut state = db().lock_or_recover();
    let exists = state.client
        .query_opt("SELECT 1 FROM runs WHERE id = $1", &[&(id as i32)])
        .map_err(|e| format!("Failed to query runs: {e}"))?
        .is_some();
    if exists {
        state.run_id = Some(id);
        set_meta(&mut state.client, "active_run_id", &id.to_string());
    }
    Ok(exists)
}

/// Returns the active run ID for this process, falling back to the most
/// recently created run. Creates a new run if none exist.
pub fn get_or_create_run(player_name: &str) -> Result<u32, String> {
    let mut state = db().lock_or_recover();

    // Already selected in this session — keep it.
    if let Some(id) = state.run_id {
        return Ok(id);
    }

    // Fall back to the most recently created run — all trackers share one run.
    if let Some(row) = state.client
        .query_opt(
            "SELECT id FROM runs ORDER BY id DESC LIMIT 1",
            &[],
        )
        .map_err(|e| format!("Failed to query runs: {e}"))?
    {
        let id = row.get::<_, i32>(0) as u32;
        state.run_id = Some(id);
        set_meta(&mut state.client, "active_run_id", &id.to_string());
        return Ok(id);
    }

    // No runs at all — create one.
    let row = state.client
        .query_one(
            "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
            &[&player_name, &(unix_now() as i64)],
        )
        .map_err(|e| format!("Failed to insert run: {e}"))?;
    let id = row.get::<_, i32>(0) as u32;
    state.run_id = Some(id);
    set_meta(&mut state.client, "active_run_id", &id.to_string());
    Ok(id)
}

/// Updates the player name once it is known from the game.
///
/// Stores the name in-process for tagging all subsequent DB writes, and updates
/// the run row if it still holds the placeholder 'Unknown'.
pub fn set_player_name(name: &str) {
    let mut state = db().lock_or_recover();
    state.current_player = name.to_string();
    if let Some(id) = state.run_id
        && let Err(e) = state.client.execute(
            "UPDATE runs SET player_name = $1 WHERE id = $2 AND player_name = 'Unknown'",
            &[&name, &(id as i32)],
        )
    {
        eprintln!("Warning: failed to update player name: {e}");
    }
}

/// Returns the run ID active in this process (or the last-written one from
/// the meta table, which is useful for the `--list-runs` display before
/// a run has been selected in the current session).
pub fn active_run_id() -> Option<u32> {
    let mut state = db().lock_or_recover();
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
    let mut state = db().lock_or_recover();
    let id = state.run_id.take()?;
    if let Err(e) = state.client.execute(
        "UPDATE runs SET ended_at = $1 WHERE id = $2",
        &[&(unix_now() as i64), &(id as i32)],
    ) {
        eprintln!("Warning: failed to record run end time: {e}");
    }
    delete_meta(&mut state.client, "active_run_id");
    Some(id)
}

/// Returns a summary of every run: `(id, player_name, started_at, dead_count)`.
pub fn list_runs() -> Result<Vec<(u32, String, u64, usize)>, String> {
    let mut state = db().lock_or_recover();
    let rows = state.client
        .query(
            "SELECT r.id, r.player_name, r.started_at, COUNT(d.personality)
             FROM runs r
             LEFT JOIN dead_pokemon d ON d.run_id = r.id
             GROUP BY r.id
             ORDER BY r.id",
            &[],
        )
        .map_err(|e| format!("Failed to query runs: {e}"))?;
    Ok(rows.iter()
        .map(|row| (
            row.get::<_, i32>(0) as u32,
            row.get(1),
            row.get::<_, i64>(2) as u64,
            row.get::<_, i64>(3) as usize,
        ))
        .collect())
}

// ---------------------------------------------------------------------------
// Public API — death tracking
// ---------------------------------------------------------------------------

/// Records a Pokemon as permanently dead in the active run.
///
/// Returns `true` if the row was newly inserted; `false` if there is no active
/// run, the DB write failed, or the record already existed (ON CONFLICT).
/// Callers should only fire downstream events (webhooks, etc.) on `true`.
pub fn mark_dead(pokemon: DeadPokemon) -> bool {
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    let player       = pg_safe(&state.current_player);
    let ot_name      = pg_safe(&pokemon.ot_name);
    let nickname     = pg_safe(&pokemon.nickname);
    let spec_name    = pg_safe(&pokemon.species_name);
    let ability_name = pg_safe(&pokemon.ability_name);
    if let Err(e) = state.client.execute(
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
            &player,       // $2 = player_name
            &(pokemon.personality as i64),
            &(pokemon.ot_id as i64),
            &ot_name,
            &nickname,
            &(pokemon.species as i32),
            &spec_name,
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
            &ability_name,
            &(pokemon.friendship as i32),
            &(pokemon.met_location as i32),
            &(pokemon.died_at as i64),
            &(pokemon.gender as i32),
        ],
    ) {
        eprintln!("Warning: failed to record dead pokemon (personality={}): {e}", pokemon.personality);
        return false;
    }
    true
}

/// Returns `true` if the Pokemon with this personality is dead in the active run.
pub fn is_dead(personality: u32) -> bool {
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    query_is_dead(&mut state.client, active, personality)
}

/// Returns the stored `DeadPokemon` entry for this personality in the active run.
pub fn get_dead_pokemon(personality: u32) -> Option<DeadPokemon> {
    let mut state = db().lock_or_recover();
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

/// Strips null bytes from a string so it is safe to insert into PostgreSQL.
///
/// GBA text encoding uses 0x00 as a padding byte. PostgreSQL rejects strings
/// containing null bytes with `invalid byte sequence for encoding "UTF8": 0x00`.
fn pg_safe(s: &str) -> String {
    s.replace('\0', "")
}

/// Records a Pokemon as caught in the active run.
///
/// No-op if this personality is already recorded (deduplicates on reconnect).
pub fn mark_caught(pokemon: CaughtPokemon) {
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return,
    };
    let player    = pg_safe(&state.current_player);
    let nickname  = pg_safe(&pokemon.nickname);
    let spec_name = pg_safe(&pokemon.species_name);
    if let Err(e) = state.client.execute(
        "INSERT INTO caught_pokemon (
            run_id, player_name, personality, ot_id, nickname, species, species_name,
            is_shiny, nature, level, met_location,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
            caught_at, gender, location_name
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
        ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
            &player,
            &(pokemon.personality as i64),
            &(pokemon.ot_id as i64),
            &nickname,
            &(pokemon.species as i32),
            &spec_name,
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
            &(pokemon.evs.hp as i32),
            &(pokemon.evs.attack as i32),
            &(pokemon.evs.defense as i32),
            &(pokemon.evs.speed as i32),
            &(pokemon.evs.sp_attack as i32),
            &(pokemon.evs.sp_defense as i32),
            &(pokemon.caught_at as i64),
            &(pokemon.gender as i32),
            &pokemon.location_name,
        ],
    ) {
        eprintln!("Warning: failed to record caught pokemon (personality={}): {e}", pokemon.personality);
    }
}

/// Updates the nickname of a caught Pokémon if it has changed.
///
/// No-op if the Pokémon is not registered, the run is not active, or the
/// nickname matches what is already stored.
pub fn update_caught_nickname(personality: u32, nickname: &str) {
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return,
    };
    let _ = state.client.execute(
        "UPDATE caught_pokemon SET nickname = $1
         WHERE run_id = $2 AND personality = $3 AND nickname != $1",
        &[&nickname, &(active as i32), &(personality as i64)],
    );
}

/// Updates the EVs of a caught Pokémon if any have changed.
///
/// No-op if the Pokémon is not registered, the run is not active, or all EVs
/// match what is already stored.
pub fn update_caught_evs(personality: u32, evs: &EVs) {
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return,
    };
    let _ = state.client.execute(
        "UPDATE caught_pokemon
         SET ev_hp = $1, ev_attack = $2, ev_defense = $3,
             ev_speed = $4, ev_sp_attack = $5, ev_sp_defense = $6
         WHERE run_id = $7 AND personality = $8
           AND (ev_hp != $1 OR ev_attack != $2 OR ev_defense != $3
             OR ev_speed != $4 OR ev_sp_attack != $5 OR ev_sp_defense != $6)",
        &[
            &(evs.hp as i32),
            &(evs.attack as i32),
            &(evs.defense as i32),
            &(evs.speed as i32),
            &(evs.sp_attack as i32),
            &(evs.sp_defense as i32),
            &(active as i32),
            &(personality as i64),
        ],
    );
}

/// Returns `true` if a Pokemon with this personality has been caught in the active run.
pub fn is_caught(personality: u32) -> bool {
    let mut state = db().lock_or_recover();
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
    let mut state = db().lock_or_recover();
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
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    let player    = pg_safe(&state.current_player);
    let spec_name = pg_safe(&encounter.species_name);
    let rows = state.client.execute(
        "INSERT INTO encounters (
            run_id, player_name, map_group, map_name, species, species_name, level, caught, encountered_at, is_shiny
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, $8, $9)
         ON CONFLICT (run_id, player_name, map_group, map_name) DO NOTHING",
        &[
            &(active as i32),
            &player,
            &(encounter.map_group as i32),
            &(encounter.map_name as i32),
            &(encounter.species as i32),
            &spec_name,
            &(encounter.level as i32),
            &(encounter.encountered_at as i64),
            &encounter.is_shiny,
        ],
    ).unwrap_or(0);
    rows == 1
}

/// Marks the current player's encounter for this area as successfully caught.
pub fn set_encounter_caught(map_group: u8, map_name: u8) {
    let mut state = db().lock_or_recover();
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

/// Returns `true` if this species has already been recorded as a first encounter
/// anywhere in the active run for the current player.
pub fn species_encountered(species: u16) -> bool {
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();
    state.client
        .query_one(
            "SELECT COUNT(*) FROM encounters
             WHERE run_id = $1 AND player_name = $2 AND species = $3",
            &[&(active as i32), &player, &(species as i32)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns `true` if any encounters have been recorded for the active run.
/// Used at startup to seed the pre-ball latch: if the run already has
/// encounters the player must have had balls at some point this run.
pub fn has_any_encounters() -> bool {
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    state.client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM encounters WHERE run_id = $1)",
            &[&(active as i32)],
        )
        .map(|row| row.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Returns `true` if an encounter has already been recorded for **any** of the
/// given `(map_group, map_name)` pairs by the current player in the active run.
///
/// Pass the slice returned by `fire_red_location_names::dungeon_floors` to
/// check whether any floor of a multi-floor dungeon is already claimed.
/// Returns `false` immediately for an empty slice.
pub fn has_encounter_for_any_floor(floors: &[(u8, u8)]) -> bool {
    if floors.is_empty() {
        return false;
    }
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();
    for &(mg, mn) in floors {
        let found = state.client
            .query_one(
                "SELECT COUNT(*) FROM encounters
                 WHERE run_id = $1 AND player_name = $2 AND map_group = $3 AND map_name = $4",
                &[&(active as i32), &player, &(mg as i32), &(mn as i32)],
            )
            .map(|row| row.get::<_, i64>(0) > 0)
            .unwrap_or(false);
        if found {
            return true;
        }
    }
    false
}

/// Returns `true` if an encounter has already been recorded for this area by the current player.
pub fn has_encounter(map_group: u8, map_name: u8) -> bool {
    let mut state = db().lock_or_recover();
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
    let mut state = db().lock_or_recover();
    let active = match state.run_id {
        Some(id) => id,
        None => return vec![],
    };
    let player = state.current_player.clone();
    state.client
        .query(
            "SELECT player_name, map_group, map_name, species, species_name, level, caught, encountered_at, is_shiny
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
            is_shiny:       row.get(8),
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
        let forced = self.dirty.swap(false, std::sync::atomic::Ordering::SeqCst);
        if !forced {
            let last = self.last_player.lock_or_recover();
            if *last == player_name { return false; }
        }

        let row = self.client
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

        self.is_active.store(active, std::sync::atomic::Ordering::SeqCst);
        let old_id = *self.run_id.lock_or_recover();
        *self.run_id.lock_or_recover() = new_id;
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

        let Some(prev_id) = prev_run_id else { return vec![] };

        client
            .query(
                "SELECT player_name, map_group, map_name, species, species_name, level, caught, encountered_at, is_shiny
                 FROM encounters WHERE run_id = $1 AND player_name = $2 ORDER BY encountered_at ASC",
                &[&(prev_id as i32), &player_name],
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
        let run_id = match *self.run_id.lock_or_recover() {
            Some(id) => id,
            None => return false,
        };
        let now = unix_now();
        if let Err(e) = self.client
            .lock_or_recover()
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

/// Deletes every record from all tables in the database.
///
/// Opens its own connection (like `dump_all`) so the live tracker connections
/// are not blocked. Deletes child tables before parent to satisfy foreign keys,
/// then resets the active_run_id meta key.
///
/// Returns `Ok(())` on success or an error string on failure.
/// Executes arbitrary SQL via a fresh connection and returns the results as JSON.
///
/// SELECT queries return `{ "columns": [...], "rows": [{col: val, ...}, ...] }`.
/// Non-SELECT statements return `{ "columns": [], "rows": [], "rows_affected": N }`.
/// Errors return `{ "error": "..." }`.
pub fn run_sql(conn_str: &str, sql: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c)  => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let messages = match client.simple_query(sql) {
        Ok(m)  => m,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let mut rows_affected: Option<u64> = None;
    for msg in messages {
        match msg {
            postgres::SimpleQueryMessage::Row(row) => {
                if columns.is_empty() {
                    columns = row.columns().iter().map(|c| c.name().to_string()).collect();
                }
                let obj: serde_json::Map<String, serde_json::Value> = row.columns()
                    .iter()
                    .enumerate()
                    .map(|(i, col)| {
                        let val = row.get(i)
                            .map(|s| serde_json::Value::String(s.to_string()))
                            .unwrap_or(serde_json::Value::Null);
                        (col.name().to_string(), val)
                    })
                    .collect();
                rows.push(serde_json::Value::Object(obj));
            }
            postgres::SimpleQueryMessage::CommandComplete(n) => {
                rows_affected = Some(n);
            }
            _ => {}
        }
    }
    serde_json::json!({ "columns": columns, "rows": rows, "rows_affected": rows_affected })
}

/// Returns per-run statistics for the given run ID as JSON.
///
/// Opens its own connection (like `dump_all`) so live tracker connections are not blocked.
pub fn run_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c)  => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let run_row = match client.query_opt(
        "SELECT player_name, started_at, ended_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ) {
        Ok(Some(r)) => r,
        Ok(None)    => return serde_json::json!({ "error": "Run not found" }),
        Err(e)      => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let player_name: String  = run_row.get(0);
    let started_at:  i64     = run_row.get(1);
    let ended_at:    Option<i64> = run_row.get(2);

    let now = unix_now() as i64;
    let duration_secs = ended_at.unwrap_or(now) - started_at;
    let playtime = format!("{}h {}m", duration_secs / 3600, (duration_secs % 3600) / 60);

    let enc_rows = client.query(
        "SELECT map_group, map_name, species_name, level, caught, is_shiny, encountered_at
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ).unwrap_or_default();

    let total_encounters = enc_rows.len();
    let total_caught: usize = enc_rows.iter().filter(|r| r.get::<_, bool>(4)).count();
    let catch_rate = if total_encounters > 0 {
        (total_caught as f64 / total_encounters as f64 * 100.0).round()
    } else {
        0.0
    };

    let zone_stats: Vec<serde_json::Value> = enc_rows.iter().map(|row| {
        let mg  = row.get::<_, i32>(0) as u8;
        let mn  = row.get::<_, i32>(1) as u8;
        let raw = fire_red_location_names::map_area_name(mg, mn);
        let area = if raw.is_empty() { format!("{}:{}", mg, mn) } else { raw.to_string() };
        serde_json::json!({
            "area":          area,
            "species_name":  row.get::<_, String>(2),
            "level":         row.get::<_, i32>(3),
            "caught":        row.get::<_, bool>(4),
            "is_shiny":      row.get::<_, bool>(5),
            "encountered_at": format_timestamp(row.get::<_, i64>(6) as u64),
        })
    }).collect();

    let dead_rows = client.query(
        "SELECT level, species_name, met_location, died_at, (max_hp = 0) AS soul_link
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at ASC",
        &[&(run_id as i32)],
    ).unwrap_or_default();

    let total_deaths = dead_rows.len();
    let avg_death_level = if total_deaths > 0 {
        let total: i64 = dead_rows.iter().map(|r| r.get::<_, i32>(0) as i64).sum();
        (total as f64 / total_deaths as f64).round()
    } else {
        0.0
    };

    let deaths: Vec<serde_json::Value> = dead_rows.iter().map(|row| {
        let met_loc  = row.get::<_, i32>(2) as u8;
        let raw      = fire_red_location_names::location_name(met_loc);
        let location = if raw.is_empty() { format!("loc {}", met_loc) } else { raw.to_string() };
        serde_json::json!({
            "level":        row.get::<_, i32>(0),
            "species_name": row.get::<_, String>(1),
            "location":     location,
            "died_at":      format_timestamp(row.get::<_, i64>(3) as u64),
            "soul_link":    row.get::<_, bool>(4),
        })
    }).collect();

    serde_json::json!({
        "run_id":          run_id,
        "player_name":     player_name,
        "started_at":      format_timestamp(started_at as u64),
        "ended_at":        ended_at.map(|t| format_timestamp(t as u64)),
        "playtime":        playtime,
        "total_encounters": total_encounters,
        "total_caught":    total_caught,
        "catch_rate_pct":  catch_rate,
        "total_deaths":    total_deaths,
        "avg_death_level": avg_death_level,
        "zone_stats":      zone_stats,
        "deaths":          deaths,
    })
}

/// Returns shiny encounter statistics for the given run ID as JSON.
///
/// Counts total encounters, total shinies, and encounters since the last shiny.
/// Opens its own connection so live tracker connections are not blocked.
pub fn shiny_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c)  => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = client.query(
        "SELECT species_name, encountered_at, is_shiny, map_group, map_name, level, caught
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ).unwrap_or_default();

    let total_encounters = rows.len();
    let total_shinies: usize = rows.iter().filter(|r| r.get::<_, bool>(2)).count();

    let last_shiny_idx = rows.iter().enumerate()
        .filter(|(_, r)| r.get::<_, bool>(2))
        .map(|(i, _)| i)
        .last();

    let (encounters_since_shiny, last_shiny) = match last_shiny_idx {
        Some(idx) => {
            let sr  = &rows[idx];
            let mg  = sr.get::<_, i32>(3) as u8;
            let mn  = sr.get::<_, i32>(4) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() { format!("{}:{}", mg, mn) } else { raw.to_string() };
            let shiny = serde_json::json!({
                "species_name":  sr.get::<_, String>(0),
                "encountered_at": format_timestamp(sr.get::<_, i64>(1) as u64),
                "area":          area,
                "level":         sr.get::<_, i32>(5),
                "caught":        sr.get::<_, bool>(6),
            });
            (total_encounters - idx - 1, Some(shiny))
        }
        None => (total_encounters, None),
    };

    let recent_start = last_shiny_idx.map(|i| i + 1).unwrap_or(0);
    let since_last_shiny: Vec<serde_json::Value> = rows[recent_start..].iter().map(|row| {
        let mg  = row.get::<_, i32>(3) as u8;
        let mn  = row.get::<_, i32>(4) as u8;
        let raw = fire_red_location_names::map_area_name(mg, mn);
        let area = if raw.is_empty() { format!("{}:{}", mg, mn) } else { raw.to_string() };
        serde_json::json!({
            "species_name":  row.get::<_, String>(0),
            "encountered_at": format_timestamp(row.get::<_, i64>(1) as u64),
            "area":          area,
            "level":         row.get::<_, i32>(5),
            "caught":        row.get::<_, bool>(6),
        })
    }).collect();

    serde_json::json!({
        "run_id":                   run_id,
        "total_encounters":         total_encounters,
        "total_shinies":            total_shinies,
        "encounters_since_last_shiny": encounters_since_shiny,
        "last_shiny":               last_shiny,
        "since_last_shiny":         since_last_shiny,
    })
}

pub fn clear_all_records(conn_str: &str) -> Result<(), String> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    client.batch_execute("
        DELETE FROM encounters;
        DELETE FROM caught_pokemon;
        DELETE FROM dead_pokemon;
        DELETE FROM runs;
        DELETE FROM meta WHERE key = 'active_run_id';
    ").map_err(|e| format!("Clear failed: {e}"))
}

/// Returns a JSON export of a single run: metadata, caught, dead, and encounter lists.
///
/// Opens its own connection so the live tracker connection is not blocked.
pub fn export_run(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c)  => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rid = run_id as i32;

    let run_row = match client.query_opt(
        "SELECT id, player_name, started_at, ended_at FROM runs WHERE id = $1",
        &[&rid],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return serde_json::json!({ "error": format!("Run {run_id} not found") }),
        Err(e)   => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };

    let caught_rows = client.query(
        "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, location_name, caught_at, player_name \
         FROM caught_pokemon WHERE run_id = $1 ORDER BY caught_at",
        &[&rid],
    ).unwrap_or_default();

    let dead_rows = client.query(
        "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, died_at, player_name \
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
        &[&rid],
    ).unwrap_or_default();

    let enc_rows = client.query(
        "SELECT species_name, level, map_group, map_name, caught, is_shiny, \
                encountered_at, player_name \
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at",
        &[&rid],
    ).unwrap_or_default();

    serde_json::json!({
        "run": {
            "id":          run_row.get::<_, i32>(0),
            "player_name": run_row.get::<_, String>(1),
            "started_at":  format_timestamp(run_row.get::<_, i64>(2) as u64),
            "ended_at":    run_row.get::<_, Option<i64>>(3).map(|t| format_timestamp(t as u64)),
        },
        "caught": caught_rows.iter().map(|r| serde_json::json!({
            "nickname":      r.get::<_, String>(0),
            "species_name":  r.get::<_, String>(1),
            "level":         r.get::<_, i32>(2),
            "nature":        r.get::<_, String>(3),
            "is_shiny":      r.get::<_, bool>(4),
            "gender":        r.get::<_, i32>(5),
            "met_location":  r.get::<_, i32>(6),
            "location_name": r.get::<_, String>(7),
            "caught_at":     format_timestamp(r.get::<_, i64>(8) as u64),
            "player_name":   r.get::<_, String>(9),
        })).collect::<Vec<_>>(),
        "dead": dead_rows.iter().map(|r| serde_json::json!({
            "nickname":     r.get::<_, String>(0),
            "species_name": r.get::<_, String>(1),
            "level":        r.get::<_, i32>(2),
            "nature":       r.get::<_, String>(3),
            "is_shiny":     r.get::<_, bool>(4),
            "gender":       r.get::<_, i32>(5),
            "met_location": r.get::<_, i32>(6),
            "died_at":      format_timestamp(r.get::<_, i64>(7) as u64),
            "player_name":  r.get::<_, String>(8),
        })).collect::<Vec<_>>(),
        "encounters": enc_rows.iter().map(|r| serde_json::json!({
            "species_name":   r.get::<_, String>(0),
            "level":          r.get::<_, i32>(1),
            "map_group":      r.get::<_, i32>(2),
            "map_name":       r.get::<_, i32>(3),
            "caught":         r.get::<_, bool>(4),
            "is_shiny":       r.get::<_, bool>(5),
            "encountered_at": format_timestamp(r.get::<_, i64>(6) as u64),
            "player_name":    r.get::<_, String>(7),
        })).collect::<Vec<_>>(),
    })
}

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
        "SELECT run_id, player_name, nickname,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, nature, is_shiny,
                location_name,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
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
            "location":  row.get::<_, String>(7),
            "ivs":       format!("{}/{}/{}/{}/{}/{}",
                row.get::<_, i32>(8),  row.get::<_, i32>(9),  row.get::<_, i32>(10),
                row.get::<_, i32>(11), row.get::<_, i32>(12), row.get::<_, i32>(13)),
            "evs":       format!("{}/{}/{}/{}/{}/{}",
                row.get::<_, i32>(14), row.get::<_, i32>(15), row.get::<_, i32>(16),
                row.get::<_, i32>(17), row.get::<_, i32>(18), row.get::<_, i32>(19)),
            "caught_at": format_timestamp(row.get::<_, i64>(20) as u64),
            "gender":    row.get::<_, i32>(21),
        })
    }).collect())
}

fn dump_dead(client: &mut Client) -> serde_json::Value {
    let rows = client.query(
        "SELECT run_id, player_name, nickname,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, nature, is_shiny,
                max_hp, attack, defense, speed, sp_attack, sp_defense,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
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
            "evs":       format!("{}/{}/{}/{}/{}/{}",
                row.get::<_, i32>(19), row.get::<_, i32>(20), row.get::<_, i32>(21),
                row.get::<_, i32>(22), row.get::<_, i32>(23), row.get::<_, i32>(24)),
            "soul_link": row.get::<_, bool>(25),
            "died_at":   format_timestamp(row.get::<_, i64>(26) as u64),
            "gender":    row.get::<_, i32>(27),
        })
    }).collect())
}

fn dump_encounters(client: &mut Client) -> serde_json::Value {
    let rows = client.query(
        "SELECT run_id, player_name, map_group, map_name,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, caught, encountered_at
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── nature_name ───────────────────────────────────────────────────────────

    #[test]
    fn nature_name_first_and_last() {
        assert_eq!(nature_name(0),  "Hardy");
        assert_eq!(nature_name(24), "Quirky");
    }

    #[test]
    fn nature_name_wraps_at_25() {
        assert_eq!(nature_name(25), "Hardy");
        assert_eq!(nature_name(26), "Lonely");
    }

    #[test]
    fn nature_name_known_values() {
        assert_eq!(nature_name(1),  "Lonely");
        assert_eq!(nature_name(10), "Timid");
        assert_eq!(nature_name(20), "Calm");
    }

    #[test]
    fn nature_name_large_personality() {
        let expected = NATURES[(u32::MAX % 25) as usize];
        assert_eq!(nature_name(u32::MAX), expected);
    }

    // ── is_leap ───────────────────────────────────────────────────────────────

    #[test]
    fn is_leap_divisible_by_400() {
        assert!(is_leap(2000));
        assert!(is_leap(1600));
    }

    #[test]
    fn is_leap_divisible_by_4_not_100() {
        assert!(is_leap(2024));
        assert!(is_leap(2020));
    }

    #[test]
    fn is_leap_divisible_by_100_not_400() {
        assert!(!is_leap(1900));
        assert!(!is_leap(2100));
    }

    #[test]
    fn is_leap_not_divisible_by_4() {
        assert!(!is_leap(2023));
        assert!(!is_leap(2019));
    }

    // ── format_timestamp ─────────────────────────────────────────────────────

    #[test]
    fn format_timestamp_unix_epoch() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn format_timestamp_time_of_day() {
        // 1 day + 1h 1m 1s = 86400 + 3661
        assert_eq!(format_timestamp(90061), "1970-01-02 01:01:01 UTC");
    }

    #[test]
    fn format_timestamp_known_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        assert_eq!(format_timestamp(1704067200), "2024-01-01 00:00:00 UTC");
    }

    #[test]
    fn format_timestamp_leap_day() {
        // 2024-02-29 00:00:00 UTC = 1709164800
        assert_eq!(format_timestamp(1709164800), "2024-02-29 00:00:00 UTC");
    }
}
