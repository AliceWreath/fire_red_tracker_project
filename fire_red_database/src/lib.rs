use fire_red_states::LockOrRecover;
use postgres::{Client, NoTls};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const NATURES: [&str; 25] = [
    "Hardy", "Lonely", "Brave", "Adamant", "Naughty", "Bold", "Docile", "Relaxed", "Impish", "Lax",
    "Timid", "Hasty", "Serious", "Jolly", "Naive", "Modest", "Mild", "Quiet", "Bashful", "Rash",
    "Calm", "Gentle", "Sassy", "Careful", "Quirky",
];

pub fn nature_name(personality: u32) -> &'static str {
    NATURES[(personality % 25) as usize]
}

const MOVES: [&str; 355] = [
    "—", // 0
    "Pound",
    "Karate Chop",
    "Double Slap",
    "Comet Punch",
    "Mega Punch", // 1–5
    "Pay Day",
    "Fire Punch",
    "Ice Punch",
    "Thunder Punch",
    "Scratch", // 6–10
    "Vice Grip",
    "Guillotine",
    "Razor Wind",
    "Swords Dance",
    "Cut", // 11–15
    "Gust",
    "Wing Attack",
    "Whirlwind",
    "Fly",
    "Bind", // 16–20
    "Slam",
    "Vine Whip",
    "Stomp",
    "Double Kick",
    "Mega Kick", // 21–25
    "Jump Kick",
    "Rolling Kick",
    "Sand Attack",
    "Headbutt",
    "Horn Attack", // 26–30
    "Fury Attack",
    "Horn Drill",
    "Tackle",
    "Body Slam",
    "Wrap", // 31–35
    "Take Down",
    "Thrash",
    "Double-Edge",
    "Tail Whip",
    "Poison Sting", // 36–40
    "Twineedle",
    "Pin Missile",
    "Leer",
    "Bite",
    "Growl", // 41–45
    "Roar",
    "Sing",
    "Supersonic",
    "Sonic Boom",
    "Disable", // 46–50
    "Acid",
    "Ember",
    "Flamethrower",
    "Mist",
    "Water Gun", // 51–55
    "Hydro Pump",
    "Surf",
    "Ice Beam",
    "Blizzard",
    "Psybeam", // 56–60
    "Bubble Beam",
    "Aurora Beam",
    "Hyper Beam",
    "Peck",
    "Drill Peck", // 61–65
    "Submission",
    "Low Kick",
    "Counter",
    "Seismic Toss",
    "Strength", // 66–70
    "Absorb",
    "Mega Drain",
    "Leech Seed",
    "Growth",
    "Razor Leaf", // 71–75
    "Solar Beam",
    "Poison Powder",
    "Stun Spore",
    "Sleep Powder",
    "Petal Dance", // 76–80
    "String Shot",
    "Dragon Rage",
    "Fire Spin",
    "Thunder Shock",
    "Thunderbolt", // 81–85
    "Thunder Wave",
    "Thunder",
    "Rock Throw",
    "Earthquake",
    "Fissure", // 86–90
    "Dig",
    "Toxic",
    "Confusion",
    "Psychic",
    "Hypnosis", // 91–95
    "Meditate",
    "Agility",
    "Quick Attack",
    "Rage",
    "Teleport", // 96–100
    "Night Shade",
    "Mimic",
    "Screech",
    "Double Team",
    "Recover", // 101–105
    "Harden",
    "Minimize",
    "Smokescreen",
    "Confuse Ray",
    "Withdraw", // 106–110
    "Defense Curl",
    "Barrier",
    "Light Screen",
    "Haze",
    "Reflect", // 111–115
    "Focus Energy",
    "Bide",
    "Metronome",
    "Mirror Move",
    "Self-Destruct", // 116–120
    "Egg Bomb",
    "Lick",
    "Smog",
    "Sludge",
    "Bone Club", // 121–125
    "Fire Blast",
    "Waterfall",
    "Clamp",
    "Swift",
    "Skull Bash", // 126–130
    "Spike Cannon",
    "Constrict",
    "Amnesia",
    "Kinesis",
    "Soft-Boiled", // 131–135
    "High Jump Kick",
    "Glare",
    "Dream Eater",
    "Poison Gas",
    "Barrage", // 136–140
    "Leech Life",
    "Lovely Kiss",
    "Sky Attack",
    "Transform",
    "Bubble", // 141–145
    "Dizzy Punch",
    "Spore",
    "Flash",
    "Psywave",
    "Splash", // 146–150
    "Acid Armor",
    "Crabhammer",
    "Explosion",
    "Fury Swipes",
    "Bonemerang", // 151–155
    "Rest",
    "Rock Slide",
    "Hyper Fang",
    "Sharpen",
    "Conversion", // 156–160
    "Tri Attack",
    "Super Fang",
    "Slash",
    "Substitute",
    "Struggle", // 161–165
    "Sketch",
    "Triple Kick",
    "Thief",
    "Spider Web",
    "Mind Reader", // 166–170
    "Nightmare",
    "Flame Wheel",
    "Snore",
    "Curse",
    "Flail", // 171–175
    "Conversion 2",
    "Aeroblast",
    "Cotton Spore",
    "Reversal",
    "Spite", // 176–180
    "Powder Snow",
    "Protect",
    "Mach Punch",
    "Scary Face",
    "Feint Attack", // 181–185
    "Sweet Kiss",
    "Belly Drum",
    "Sludge Bomb",
    "Mud-Slap",
    "Octazooka", // 186–190
    "Spikes",
    "Zap Cannon",
    "Foresight",
    "Destiny Bond",
    "Perish Song", // 191–195
    "Icy Wind",
    "Detect",
    "Bone Rush",
    "Lock-On",
    "Outrage", // 196–200
    "Sandstorm",
    "Giga Drain",
    "Endure",
    "Charm",
    "Rollout", // 201–205
    "False Swipe",
    "Swagger",
    "Milk Drink",
    "Spark",
    "Fury Cutter", // 206–210
    "Steel Wing",
    "Mean Look",
    "Attract",
    "Sleep Talk",
    "Heal Bell", // 211–215
    "Return",
    "Present",
    "Frustration",
    "Safeguard",
    "Pain Split", // 216–220
    "Sacred Fire",
    "Magnitude",
    "Dynamic Punch",
    "Megahorn",
    "Dragon Breath", // 221–225
    "Baton Pass",
    "Encore",
    "Pursuit",
    "Rapid Spin",
    "Sweet Scent", // 226–230
    "Iron Tail",
    "Metal Claw",
    "Vital Throw",
    "Morning Sun",
    "Synthesis", // 231–235
    "Moonlight",
    "Hidden Power",
    "Cross Chop",
    "Twister",
    "Rain Dance", // 236–240
    "Sunny Day",
    "Crunch",
    "Mirror Coat",
    "Psych Up",
    "Extreme Speed", // 241–245
    "Ancient Power",
    "Shadow Ball",
    "Future Sight",
    "Rock Smash",
    "Whirlpool", // 246–250
    "Beat Up",
    "Fake Out",
    "Uproar",
    "Stockpile",
    "Spit Up", // 251–255
    "Swallow",
    "Heat Wave",
    "Hail",
    "Torment",
    "Flatter", // 256–260
    "Will-O-Wisp",
    "Memento",
    "Facade",
    "Focus Punch",
    "Smelling Salts", // 261–265
    "Follow Me",
    "Nature Power",
    "Charge",
    "Taunt",
    "Helping Hand", // 266–270
    "Trick",
    "Role Play",
    "Wish",
    "Assist",
    "Ingrain", // 271–275
    "Superpower",
    "Magic Coat",
    "Recycle",
    "Revenge",
    "Brick Break", // 276–280
    "Yawn",
    "Knock Off",
    "Endeavor",
    "Eruption",
    "Skill Swap", // 281–285
    "Imprison",
    "Refresh",
    "Grudge",
    "Snatch",
    "Secret Power", // 286–290
    "Dive",
    "Arm Thrust",
    "Camouflage",
    "Tail Glow",
    "Luster Purge", // 291–295
    "Mist Ball",
    "Feather Dance",
    "Teeter Dance",
    "Blaze Kick",
    "Mud Sport", // 296–300
    "Ice Ball",
    "Needle Arm",
    "Slack Off",
    "Hyper Voice",
    "Poison Fang", // 301–305
    "Crush Claw",
    "Blast Burn",
    "Hydro Cannon",
    "Meteor Mash",
    "Astonish", // 306–310
    "Weather Ball",
    "Aromatherapy",
    "Fake Tears",
    "Air Cutter",
    "Overheat", // 311–315
    "Odor Sleuth",
    "Rock Tomb",
    "Silver Wind",
    "Metal Sound",
    "Grass Whistle", // 316–320
    "Tickle",
    "Cosmic Power",
    "Water Spout",
    "Signal Beam",
    "Shadow Punch", // 321–325
    "Extrasensory",
    "Sky Uppercut",
    "Sand Tomb",
    "Sheer Cold",
    "Muddy Water", // 326–330
    "Bullet Seed",
    "Aerial Ace",
    "Icicle Spear",
    "Iron Defense",
    "Block", // 331–335
    "Howl",
    "Dragon Claw",
    "Frenzy Plant",
    "Bulk Up",
    "Bounce", // 336–340
    "Mud Shot",
    "Poison Tail",
    "Covet",
    "Volt Tackle",
    "Magical Leaf", // 341–345
    "Water Sport",
    "Calm Mind",
    "Leaf Blade",
    "Dragon Dance",
    "Rock Blast", // 346–350
    "Shock Wave",
    "Water Pulse",
    "Doom Desire",
    "Psycho Boost", // 351–354
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
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_days: [u64; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year,
        month,
        days + 1,
        h,
        m,
        s
    )
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Returns the current time as Unix seconds (seconds since 1970-01-01 00:00:00 UTC).
/// Returns 0 if the system clock is before the epoch (should never happen in practice).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parses a timestamp string produced by [`format_timestamp`] back to Unix seconds.
///
/// Accepts `"YYYY-MM-DD HH:MM:SS UTC"`. Returns `None` if the string cannot be parsed.
/// Used by `import_run` to preserve original event times on round-trip.
pub fn parse_timestamp(s: &str) -> Option<u64> {
    let s = s.trim_end_matches(" UTC");
    let (date, time) = s.split_once(' ')?;
    let mut dp = date.splitn(3, '-');
    let year: u32 = dp.next()?.parse().ok()?;
    let month: u32 = dp.next()?.parse().ok()?;
    let day: u32 = dp.next()?.parse().ok()?;
    let mut tp = time.splitn(3, ':');
    let hour: u64 = tp.next()?.parse().ok()?;
    let min: u64 = tp.next()?.parse().ok()?;
    let sec: u64 = tp.next()?.parse().ok()?;
    // Reject pre-epoch years: the loop `1970..year` is empty for year ≤ 1969,
    // which would silently return Some(0) — the same value as 1970-01-01.
    if year < 1970 {
        return None;
    }
    if month == 0 || month > 12 {
        return None;
    }
    if day == 0 {
        return None;
    }
    if hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let month_days_arr: [u32; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day > month_days_arr[(month - 1) as usize] {
        return None;
    }
    for m in 1..month {
        days += month_days_arr[(m - 1) as usize] as u64;
    }
    days += (day - 1) as u64;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
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
    /// `true` when this death was caused by a Soul Link rule (the partner Pokémon
    /// died first) rather than the Pokémon reaching 0 HP itself.
    /// Stored explicitly to avoid using `max_hp == 0` as a sentinel.
    pub is_soul_link_death: bool,
    /// Species name of the enemy that dealt the killing blow, if known.
    pub killed_by_species: Option<String>,
    /// Move name used by the enemy for the killing blow, if known.
    pub killed_by_move: Option<String>,
    /// Map area name where the Pokémon died (e.g. "Route 1"). Empty for
    /// records from before v19 or when the location is not yet determined.
    pub area_name: String,
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
    pub player_name: String,
    pub personality: u32,
    pub ot_id: u32,
    pub nickname: String,
    pub species: u16,
    pub species_name: String,
    pub is_shiny: bool,
    pub nature: String,
    pub level: u8,
    pub met_location: u8,
    /// Human-readable location name resolved at catch time from the current map
    /// (group, map_name) coordinates. Empty for records created before this field
    /// was added.
    pub location_name: String,
    pub ivs: IVs,
    pub evs: EVs,
    pub caught_at: u64,
    /// `0` = male, `1` = female, `2` = genderless.
    pub gender: u8,
}

/// A defeated trainer — stored once per run/player/flag combination.
#[derive(Clone, Debug)]
pub struct TrainerDefeat {
    /// Player who beat this trainer.
    pub player_name: String,
    /// SaveBlock1 flag index for this trainer (range 0x100–0x3DF).
    pub flag_index: u32,
    /// Human-readable trainer label, e.g. "Trainer 0x1A3" or a named entry.
    pub trainer_name: String,
    /// Location string at the moment of defeat.
    pub location: String,
    /// Unix timestamp of the defeat.
    pub defeated_at: u64,
}

/// A registered user account.
#[derive(Clone, Debug)]
pub struct User {
    pub id: u32,
    pub username: String,
    pub created_at: u64,
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

impl DbState {
    /// Returns the run ID for the current call context.
    ///
    /// Direct-mode slots each run their game loop on a dedicated thread and
    /// set `THREAD_RUN_ID` to their own run ID via `set_thread_run_id`.  That
    /// per-thread override takes precedence over the global `run_id` so multiple
    /// slots can write to different runs simultaneously without interfering.
    fn effective_run_id(&self) -> Option<u32> {
        THREAD_RUN_ID.with(|c| c.get()).or(self.run_id)
    }
}

// Per-thread run ID override used by direct-mode game-loop threads.
// Set via set_thread_run_id at thread startup; cleared by clear_thread_run_id.
thread_local! {
    static THREAD_RUN_ID: Cell<Option<u32>> = const { Cell::new(None) };
}

/// Override the active run ID for the current thread.
///
/// Call this once at the start of a direct-mode game-loop thread so that all
/// DB writes from that thread go to the correct run, independent of the global
/// `DbState.run_id`.
///
/// For short-lived contexts (e.g. `spawn_blocking` closures) prefer
/// [`set_thread_run_id_scoped`], which returns a guard that auto-clears on
/// drop and prevents accidental leakage to subsequent tasks on the same thread.
pub fn set_thread_run_id(run_id: u32) {
    THREAD_RUN_ID.with(|c| c.set(Some(run_id)));
}

/// Clear the per-thread run ID override set by [`set_thread_run_id`].
pub fn clear_thread_run_id() {
    THREAD_RUN_ID.with(|c| c.set(None));
}

/// RAII guard returned by [`set_thread_run_id_scoped`].
/// Clears the thread-local run ID when dropped so it cannot leak to the next
/// task scheduled on the same OS thread.
pub struct ThreadRunIdGuard;

impl Drop for ThreadRunIdGuard {
    fn drop(&mut self) {
        THREAD_RUN_ID.with(|c| c.set(None));
    }
}

/// Override the active run ID for the current thread and return a guard that
/// clears it automatically when dropped.
///
/// Use this instead of [`set_thread_run_id`] in any context that may return
/// (e.g. a `spawn_blocking` closure or a test) to ensure the value does not
/// persist across unrelated tasks on the same tokio blocking-pool thread.
pub fn set_thread_run_id_scoped(run_id: u32) -> ThreadRunIdGuard {
    THREAD_RUN_ID.with(|c| c.set(Some(run_id)));
    ThreadRunIdGuard
}

static DB: OnceLock<Option<Mutex<DbState>>> = OnceLock::new();

fn db() -> Option<&'static Mutex<DbState>> {
    DB.get()?.as_ref()
}

/// Initialize in no-op mode: all database functions return immediately.
///
/// Call this when running without a PostgreSQL connection (e.g. aggregator
/// direct mode with no `--db` flag).  Must be called before any other
/// function in this crate, exactly like `initialize`.
pub fn initialize_noop() {
    let _ = DB.set(None);
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
const SCHEMA_VERSION: &str = "28";

pub fn initialize(connection_string: &str) -> Result<(), String> {
    // Accept bare `host/dbname` strings — prepend the scheme so callers don't have to.
    let normalized;
    let connection_string = if connection_string.starts_with("postgresql://")
        || connection_string.starts_with("postgres://")
        || connection_string.contains('=')
    {
        connection_string
    } else {
        normalized = format!("postgresql://{connection_string}");
        &normalized
    };

    let mut client = Client::connect(connection_string, NoTls).map_err(|e| {
        format!(
            "Failed to connect to PostgreSQL: {e}\n\
             Ensure the server is reachable and the database exists.\n\
             Create it with:  psql -c 'CREATE DATABASE nuzlocke;'"
        )
    })?;

    // If the meta table already records the current schema version, all
    // migrations have already been applied — skip the batch entirely.
    let already_current = client
        .query_opt("SELECT value FROM meta WHERE key = 'schema_version'", &[])
        .ok()
        .flatten()
        .map(|row| row.get::<_, String>(0))
        .as_deref()
        == Some(SCHEMA_VERSION);

    if already_current {
        return DB
            .set(Some(Mutex::new(DbState {
                client,
                run_id: None,
                current_player: String::new(),
            })))
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

        -- Migration: add explicit soul-link-death flag.
        -- Backfills TRUE for existing records where max_hp = 0 (the old sentinel).
        ALTER TABLE dead_pokemon ADD COLUMN IF NOT EXISTS is_soul_link_death BOOLEAN NOT NULL DEFAULT FALSE;
        UPDATE dead_pokemon SET is_soul_link_death = TRUE WHERE max_hp = 0 AND is_soul_link_death = FALSE;

        -- Data repair: fix species_name for NIDORAN♀ (29) and NIDORAN♂ (32) that were
        -- stored without the gender symbol due to a bug in the GBA text decoder.
        -- Only updates rows that don't already contain a gender symbol.
        UPDATE encounters     SET species_name = 'NIDORAN♀' WHERE species = 29 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE encounters     SET species_name = 'NIDORAN♂' WHERE species = 32 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE caught_pokemon SET species_name = 'NIDORAN♀' WHERE species = 29 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE caught_pokemon SET species_name = 'NIDORAN♂' WHERE species = 32 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE dead_pokemon   SET species_name = 'NIDORAN♀' WHERE species = 29 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';
        UPDATE dead_pokemon   SET species_name = 'NIDORAN♂' WHERE species = 32 AND species_name NOT LIKE '%♀%' AND species_name NOT LIKE '%♂%';

        -- Migration: persistent event log — one row per notable gameplay event.
        -- event_type values: 'catch', 'death', 'soul_link_death', 'shiny', 'wipe', 'badge', 'nickname_change'
        -- old_nickname is populated only for nickname_change events (the name that was overwritten).
        CREATE TABLE IF NOT EXISTS events (
            id           SERIAL  PRIMARY KEY,
            run_id       INTEGER NOT NULL REFERENCES runs(id),
            player_name  TEXT    NOT NULL,
            event_type   TEXT    NOT NULL,
            species_name TEXT    NOT NULL DEFAULT '',
            nickname     TEXT    NOT NULL DEFAULT '',
            old_nickname TEXT    NOT NULL DEFAULT '',
            level        INTEGER NOT NULL DEFAULT 0,
            occurred_at  BIGINT  NOT NULL
        );

        -- Idempotent guard for databases created before old_nickname was added
        -- to the CREATE TABLE above (schema versions prior to 6).
        ALTER TABLE events ADD COLUMN IF NOT EXISTS old_nickname TEXT NOT NULL DEFAULT '';

        -- Migration v7: webhook delivery receipt log.
        -- Records every attempted webhook delivery so the API can surface
        -- a history of what fired, when, and whether it succeeded.
        CREATE TABLE IF NOT EXISTS webhook_log (
            id         SERIAL  PRIMARY KEY,
            run_id     INTEGER REFERENCES runs(id),
            event_type TEXT    NOT NULL,
            url        TEXT    NOT NULL,
            success    BOOLEAN NOT NULL,
            attempts   INTEGER NOT NULL,
            payload    TEXT    NOT NULL DEFAULT '',
            fired_at   BIGINT  NOT NULL
        );

        -- Migration v8: index on webhook_log(run_id) for fast per-run queries.
        CREATE INDEX IF NOT EXISTS webhook_log_run_id_idx ON webhook_log(run_id);

        -- Migration v9: manual soul-link overrides — takes precedence over the
        -- automatic met_location / receipt-order pairing.  One row per
        -- (run, personality); the partner_personality column stores the target.
        CREATE TABLE IF NOT EXISTS soul_link_overrides (
            run_id              INTEGER NOT NULL REFERENCES runs(id),
            personality         BIGINT  NOT NULL,
            partner_personality BIGINT  NOT NULL,
            created_at          BIGINT  NOT NULL,
            PRIMARY KEY (run_id, personality)
        );

        -- Migration v10: death cause analysis — enemy species / move at time of death.
        ALTER TABLE dead_pokemon ADD COLUMN IF NOT EXISTS killed_by_species TEXT;
        ALTER TABLE dead_pokemon ADD COLUMN IF NOT EXISTS killed_by_move TEXT;

        -- Migration v11: trainer battle log — one row per defeated trainer flag per run/player.
        CREATE TABLE IF NOT EXISTS trainer_battles (
            id           SERIAL  PRIMARY KEY,
            run_id       INTEGER NOT NULL REFERENCES runs(id),
            player_name  TEXT    NOT NULL,
            flag_index   INTEGER NOT NULL,
            trainer_name TEXT    NOT NULL,
            location     TEXT    NOT NULL DEFAULT '',
            defeated_at  BIGINT  NOT NULL,
            UNIQUE (run_id, player_name, flag_index)
        );

        -- Migration v12: track the lowest HP ratio ever observed for each Pokémon.
        -- Both columns are NULL until the first sub-max-HP observation is recorded.
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS min_hp_seen_hp   SMALLINT;
        ALTER TABLE caught_pokemon ADD COLUMN IF NOT EXISTS min_hp_seen_max_hp SMALLINT;
        ALTER TABLE dead_pokemon   ADD COLUMN IF NOT EXISTS min_hp_seen_hp   SMALLINT;
        ALTER TABLE dead_pokemon   ADD COLUMN IF NOT EXISTS min_hp_seen_max_hp SMALLINT;

        -- Migration v13: per-party-mon HP change log and enemy HP observations.
        CREATE TABLE IF NOT EXISTS hp_history (
            id          BIGSERIAL PRIMARY KEY,
            run_id      INTEGER   NOT NULL REFERENCES runs(id),
            personality BIGINT    NOT NULL,
            observed_at BIGINT    NOT NULL,
            hp          INTEGER   NOT NULL,
            max_hp      INTEGER   NOT NULL
        );
        CREATE INDEX IF NOT EXISTS hp_history_lookup
            ON hp_history (run_id, personality, observed_at);

        CREATE TABLE IF NOT EXISTS enemy_hp_log (
            id          BIGSERIAL PRIMARY KEY,
            run_id      INTEGER   NOT NULL REFERENCES runs(id),
            personality BIGINT    NOT NULL,
            observed_at BIGINT    NOT NULL,
            hp          INTEGER   NOT NULL,
            max_hp      INTEGER   NOT NULL,
            phase       TEXT      NOT NULL
        );
        CREATE INDEX IF NOT EXISTS enemy_hp_log_lookup
            ON enemy_hp_log (run_id, personality);

        -- Migration v14: free-text annotation on individual event log entries.
        ALTER TABLE events ADD COLUMN IF NOT EXISTS note TEXT NOT NULL DEFAULT '';

        -- Migration v15: catch attempt log and per-area time tracking.
        CREATE TABLE IF NOT EXISTS catch_attempts (
            id             SERIAL  PRIMARY KEY,
            run_id         INTEGER NOT NULL REFERENCES runs(id),
            player_name    TEXT    NOT NULL DEFAULT '',
            species_name   TEXT    NOT NULL DEFAULT '',
            area           TEXT    NOT NULL DEFAULT '',
            balls_thrown   INTEGER NOT NULL DEFAULT 0,
            caught         BOOLEAN NOT NULL DEFAULT FALSE,
            encountered_at BIGINT  NOT NULL
        );
        CREATE INDEX IF NOT EXISTS catch_attempts_run
            ON catch_attempts (run_id);

        CREATE TABLE IF NOT EXISTS area_visits (
            id          SERIAL  PRIMARY KEY,
            run_id      INTEGER NOT NULL REFERENCES runs(id),
            player_name TEXT    NOT NULL DEFAULT '',
            map_group   INTEGER NOT NULL,
            map_name    INTEGER NOT NULL,
            area_name   TEXT    NOT NULL DEFAULT '',
            entered_at  BIGINT  NOT NULL,
            exited_at   BIGINT
        );
        CREATE INDEX IF NOT EXISTS area_visits_run
            ON area_visits (run_id, entered_at);

        -- Migration v16: user-defined run goals checklist.
        CREATE TABLE IF NOT EXISTS run_goals (
            id         SERIAL  PRIMARY KEY,
            run_id     INTEGER NOT NULL REFERENCES runs(id),
            text       TEXT    NOT NULL,
            completed  BOOLEAN NOT NULL DEFAULT FALSE,
            created_at BIGINT  NOT NULL
        );
        CREATE INDEX IF NOT EXISTS run_goals_run
            ON run_goals (run_id);

        -- Migration v17: named preset party configurations (global, not per-run).
        -- config stores a JSON array of ClientMessage-compatible command objects.
        CREATE TABLE IF NOT EXISTS presets (
            name       TEXT    PRIMARY KEY,
            config     TEXT    NOT NULL,
            created_at BIGINT  NOT NULL
        );

        -- Migration v18: per-run nuzlocke rule flags.
        CREATE TABLE IF NOT EXISTS run_rules (
            run_id           INTEGER PRIMARY KEY REFERENCES runs(id),
            duplicate_clause BOOLEAN NOT NULL DEFAULT FALSE,
            species_clause   BOOLEAN NOT NULL DEFAULT FALSE,
            gift_clause      BOOLEAN NOT NULL DEFAULT FALSE,
            shiny_clause     BOOLEAN NOT NULL DEFAULT FALSE,
            updated_at       BIGINT  NOT NULL
        );

        -- Migration v19: record the map area where each pokemon died.
        ALTER TABLE dead_pokemon ADD COLUMN IF NOT EXISTS area_name TEXT NOT NULL DEFAULT '';

        -- Migration v20: party level snapshots at each badge milestone (for level curve).
        CREATE TABLE IF NOT EXISTS party_snapshots (
            id          SERIAL   PRIMARY KEY,
            run_id      INTEGER  NOT NULL REFERENCES runs(id),
            player_name TEXT     NOT NULL DEFAULT '',
            badge_index SMALLINT NOT NULL,
            badge_name  TEXT     NOT NULL,
            occurred_at BIGINT   NOT NULL,
            avg_level   REAL     NOT NULL,
            levels      TEXT     NOT NULL
        );
        CREATE INDEX IF NOT EXISTS party_snapshots_run ON party_snapshots (run_id);

        -- Migration v21: per-pokemon move use counts derived from PP-delta detection.
        CREATE TABLE IF NOT EXISTS move_uses (
            id          SERIAL   PRIMARY KEY,
            run_id      INTEGER  NOT NULL REFERENCES runs(id),
            player_name TEXT     NOT NULL DEFAULT '',
            personality BIGINT   NOT NULL,
            move_slot   SMALLINT NOT NULL,
            move_id     SMALLINT NOT NULL,
            move_name   TEXT     NOT NULL DEFAULT '',
            use_count   INTEGER  NOT NULL DEFAULT 0,
            updated_at  BIGINT   NOT NULL,
            UNIQUE (run_id, player_name, personality, move_slot)
        );
        CREATE INDEX IF NOT EXISTS move_uses_run ON move_uses (run_id);

        -- Migration v22: friendship change log; threshold alerts at 220.
        CREATE TABLE IF NOT EXISTS friendship_log (
            id           BIGSERIAL PRIMARY KEY,
            run_id       INTEGER   NOT NULL REFERENCES runs(id),
            player_name  TEXT      NOT NULL DEFAULT '',
            personality  BIGINT    NOT NULL,
            nickname     TEXT      NOT NULL DEFAULT '',
            species_name TEXT      NOT NULL DEFAULT '',
            friendship   SMALLINT  NOT NULL,
            logged_at    BIGINT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS friendship_log_run ON friendship_log (run_id, personality);

        -- Migration v23: status condition change log (burn/paralysis/poison/freeze/sleep).
        -- status_value is the raw Gen III status bitmask (bits 0-2=sleep, 3=PSN, 4=BRN, 5=FRZ, 6=PAR, 7=TOX).
        -- event_type is 'onset' or 'clear'.
        CREATE TABLE IF NOT EXISTS status_events (
            id           BIGSERIAL PRIMARY KEY,
            run_id       INTEGER   NOT NULL REFERENCES runs(id),
            player_name  TEXT      NOT NULL DEFAULT '',
            personality  BIGINT    NOT NULL,
            nickname     TEXT      NOT NULL DEFAULT '',
            species_name TEXT      NOT NULL DEFAULT '',
            status_name  TEXT      NOT NULL DEFAULT '',
            status_value INTEGER   NOT NULL DEFAULT 0,
            event_type   TEXT      NOT NULL DEFAULT 'onset',
            occurred_at  BIGINT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS status_events_run ON status_events (run_id, personality);

        -- Migration v24: user accounts and sessions.
        -- Users provide a unique, password-protected identity so two players
        -- with the same in-game trainer name don't collide in the database.
        CREATE TABLE IF NOT EXISTS users (
            id            SERIAL  PRIMARY KEY,
            username      TEXT    UNIQUE NOT NULL,
            password_hash TEXT    NOT NULL,
            created_at    BIGINT  NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sessions (
            token      TEXT    PRIMARY KEY,
            user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            created_at BIGINT  NOT NULL,
            expires_at BIGINT  NOT NULL
        );
        CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id);
        CREATE INDEX IF NOT EXISTS sessions_expiry ON sessions (expires_at);

        ALTER TABLE runs ADD COLUMN IF NOT EXISTS user_id INTEGER REFERENCES users(id);

        CREATE TABLE IF NOT EXISTS run_invites (
            id           SERIAL  PRIMARY KEY,
            run_id       INTEGER NOT NULL REFERENCES runs(id)  ON DELETE CASCADE,
            invited_by   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            invited_user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            status       TEXT    NOT NULL DEFAULT 'pending',
            created_at   BIGINT  NOT NULL,
            responded_at BIGINT,
            UNIQUE (run_id, invited_user)
        );
        CREATE INDEX IF NOT EXISTS run_invites_user   ON run_invites (invited_user);
        CREATE INDEX IF NOT EXISTS run_invites_run    ON run_invites (run_id);

        ALTER TABLE run_invites ADD COLUMN IF NOT EXISTS is_request BOOLEAN NOT NULL DEFAULT FALSE;

        -- Migration v27: per-user integration configs (Twitch, Discord, YouTube, OBS).
        CREATE TABLE IF NOT EXISTS user_integrations (
            user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            kind       TEXT    NOT NULL,
            config     TEXT    NOT NULL DEFAULT '{}',
            updated_at BIGINT  NOT NULL DEFAULT 0,
            PRIMARY KEY (user_id, kind)
        );

        -- Migration v28: run owner can pin each player to a display column.
        ALTER TABLE runs ADD COLUMN IF NOT EXISTS slot_index INTEGER;
    ").map_err(|e| format!("Failed to create database schema: {e}"))?;

    // Record the schema version so future startups can skip all migrations.
    client
        .execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', $1)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            &[&SCHEMA_VERSION],
        )
        .map_err(|e| format!("Failed to write schema_version: {e}"))?;

    DB.set(Some(Mutex::new(DbState {
        client,
        run_id: None,
        current_player: String::new(),
    })))
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
        tracing::warn!("Failed to write meta key '{key}': {e}");
    }
}

fn delete_meta(client: &mut Client, key: &str) {
    if let Err(e) = client.execute("DELETE FROM meta WHERE key = $1", &[&key]) {
        tracing::warn!("Failed to delete meta key '{key}': {e}");
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
            player_name: row.get(0),
            personality: row.get::<_, i64>(1) as u32,
            ot_id: row.get::<_, i64>(2) as u32,
            nickname: row.get(3),
            species: row.get::<_, i32>(4) as u16,
            species_name: row.get(5),
            is_shiny: row.get(6),
            nature: row.get(7),
            level: row.get::<_, i32>(8) as u8,
            met_location: row.get::<_, i32>(9) as u8,
            location_name: row.get::<_, String>(18),
            ivs: IVs {
                hp: row.get::<_, i32>(10) as u8,
                attack: row.get::<_, i32>(11) as u8,
                defense: row.get::<_, i32>(12) as u8,
                speed: row.get::<_, i32>(13) as u8,
                sp_attack: row.get::<_, i32>(14) as u8,
                sp_defense: row.get::<_, i32>(15) as u8,
            },
            evs: EVs {
                hp: row.get::<_, i32>(19) as u8,
                attack: row.get::<_, i32>(20) as u8,
                defense: row.get::<_, i32>(21) as u8,
                speed: row.get::<_, i32>(22) as u8,
                sp_attack: row.get::<_, i32>(23) as u8,
                sp_defense: row.get::<_, i32>(24) as u8,
            },
            caught_at: row.get::<_, i64>(16) as u64,
            gender: row.get::<_, i32>(17) as u8,
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
///
/// Column 0 must be `player_name`; the remaining columns follow the standard
/// order used by all dead_pokemon queries in this file, with
/// `is_soul_link_death` appended last (column 44).
fn row_to_dead_pokemon(row: &postgres::Row) -> DeadPokemon {
    DeadPokemon {
        player_name: row.get(0),
        personality: row.get::<_, i64>(1) as u32,
        ot_id: row.get::<_, i64>(2) as u32,
        ot_name: row.get(3),
        nickname: row.get(4),
        species: row.get::<_, i32>(5) as u16,
        species_name: row.get(6),
        is_shiny: row.get(7),
        nature: row.get(8),
        level: row.get::<_, i32>(9) as u8,
        experience: row.get::<_, i64>(10) as u32,
        max_hp: row.get::<_, i32>(11) as u16,
        attack: row.get::<_, i32>(12) as u16,
        defense: row.get::<_, i32>(13) as u16,
        speed: row.get::<_, i32>(14) as u16,
        sp_attack: row.get::<_, i32>(15) as u16,
        sp_defense: row.get::<_, i32>(16) as u16,
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
            hp: row.get::<_, i32>(25) as u8,
            attack: row.get::<_, i32>(26) as u8,
            defense: row.get::<_, i32>(27) as u8,
            speed: row.get::<_, i32>(28) as u8,
            sp_attack: row.get::<_, i32>(29) as u8,
            sp_defense: row.get::<_, i32>(30) as u8,
        },
        evs: EVs {
            hp: row.get::<_, i32>(31) as u8,
            attack: row.get::<_, i32>(32) as u8,
            defense: row.get::<_, i32>(33) as u8,
            speed: row.get::<_, i32>(34) as u8,
            sp_attack: row.get::<_, i32>(35) as u8,
            sp_defense: row.get::<_, i32>(36) as u8,
        },
        held_item: row.get::<_, i32>(37) as u16,
        ability: row.get::<_, i32>(38) as u8,
        ability_name: row.get(39),
        friendship: row.get::<_, i32>(40) as u8,
        met_location: row.get::<_, i32>(41) as u8,
        died_at: row.get::<_, i64>(42) as u64,
        gender: row.get::<_, i32>(43) as u8,
        is_soul_link_death: row.get(44),
        killed_by_species: row.get(45),
        killed_by_move: row.get(46),
        area_name: row.try_get::<_, String>(47).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Public API — run management
// ---------------------------------------------------------------------------

/// Creates a fresh run, sets it as active in this process, and returns its ID.
pub fn new_run(player_name: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Ok(0) };
    let mut state = db.lock_or_recover();
    let row = state
        .client
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
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let exists = state
        .client
        .query_opt("SELECT 1 FROM runs WHERE id = $1", &[&(id as i32)])
        .map_err(|e| format!("Failed to query runs: {e}"))?
        .is_some();
    if exists {
        // Update only the persisted metadata (used by --list-runs and tracker
        // startup). Do NOT touch state.run_id — mutating the global would
        // silently redirect writes from all tracker-TCP game-loop threads to
        // this run ID until they restart.
        set_meta(&mut state.client, "active_run_id", &id.to_string());
    }
    Ok(exists)
}

/// Returns the active run ID for this process, falling back to the most
/// recently created run. Creates a new run if none exist.
pub fn get_or_create_run(player_name: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Ok(0) };
    let mut state = db.lock_or_recover();

    // Already selected in this session — keep it.
    if let Some(id) = state.run_id {
        return Ok(id);
    }

    // Fall back to the most recently created run — all trackers share one run.
    if let Some(row) = state
        .client
        .query_opt("SELECT id FROM runs ORDER BY id DESC LIMIT 1", &[])
        .map_err(|e| format!("Failed to query runs: {e}"))?
    {
        let id = row.get::<_, i32>(0) as u32;
        state.run_id = Some(id);
        set_meta(&mut state.client, "active_run_id", &id.to_string());
        return Ok(id);
    }

    // No runs at all — create one.
    let row = state
        .client
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
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    state.current_player = name.to_string();
    if let Some(id) = state.effective_run_id()
        && let Err(e) = state.client.execute(
            "UPDATE runs SET player_name = $1 WHERE id = $2 AND player_name = 'Unknown'",
            &[&name, &(id as i32)],
        )
    {
        tracing::warn!("Failed to update player name: {e}");
    }
}

/// Returns the run ID active in this process (or the last-written one from
/// the meta table, which is useful for the `--list-runs` display before
/// a run has been selected in the current session).
pub fn active_run_id() -> Option<u32> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    state
        .run_id
        .or_else(|| get_meta(&mut state.client, "active_run_id").and_then(|v| v.parse().ok()))
}

/// Returns `(player_name, started_at)` for the given run ID using the global DB connection.
pub fn get_run_info(run_id: u32) -> Option<(String, u64)> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT player_name, started_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).ok()??;
    Some((row.get(0), row.get::<_, i64>(1) as u64))
}

/// Ends the active run by recording its end timestamp and clearing the
/// in-process run ID. Subsequent writes (deaths, encounters, catches)
/// will be silently dropped until a new run is started.
///
/// Returns the ID of the run that was ended, or `None` if no run was active.
pub fn end_run() -> Option<u32> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let id = state.run_id.take()?;
    if let Err(e) = state.client.execute(
        "UPDATE runs SET ended_at = $1 WHERE id = $2",
        &[&(unix_now() as i64), &(id as i32)],
    ) {
        tracing::warn!("Failed to record run end time: {e}");
    }
    delete_meta(&mut state.client, "active_run_id");
    Some(id)
}

/// End a specific run by ID, verifying the caller owns it.
/// Returns `Err` if the DB is not initialised, the run doesn't exist, the
/// caller doesn't own it, or the run is already ended.
pub fn end_run_by_id(run_id: u32, user_id: u32) -> Result<(), String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT user_id, ended_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| "run not found".to_string())?;
    let owner_id: Option<i32> = row.get(0);
    // Allow only if the run's owner matches the caller.
    // Ownerless runs (created before the auth system) require server owner (user 1).
    let caller = user_id as i32;
    let allowed = match owner_id {
        Some(oid) => oid == caller,
        None => caller == 1,
    };
    if !allowed {
        return Err("you do not own this run".to_string());
    }
    let already_ended: Option<i64> = row.get(1);
    if already_ended.is_some() {
        return Err("run is already ended".to_string());
    }
    state.client.execute(
        "UPDATE runs SET ended_at = $1 WHERE id = $2",
        &[&(unix_now() as i64), &(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Returns a summary of every run: `(id, player_name, started_at, dead_count)`.
pub fn list_runs() -> Result<Vec<(u32, String, u64, usize)>, String> {
    let Some(db) = db() else { return Ok(vec![]) };
    let mut state = db.lock_or_recover();
    let rows = state
        .client
        .query(
            "SELECT r.id, r.player_name, r.started_at, COUNT(d.personality)
             FROM runs r
             LEFT JOIN dead_pokemon d ON d.run_id = r.id
             GROUP BY r.id
             ORDER BY r.id",
            &[],
        )
        .map_err(|e| format!("Failed to query runs: {e}"))?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get::<_, i32>(0) as u32,
                row.get(1),
                row.get::<_, i64>(2) as u64,
                row.get::<_, i64>(3) as usize,
            )
        })
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
/// Returns `Ok(true)` when the row was newly inserted, `Ok(false)` when there
/// is no active run (caller should skip the death event silently), and
/// `Err(e)` on a database error — the caller should log the error and skip.
pub fn mark_dead(pokemon: DeadPokemon) -> Result<bool, postgres::Error> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(false),
    };
    let player = pg_safe(&state.current_player);
    let ot_name = pg_safe(&pokemon.ot_name);
    let nickname = pg_safe(&pokemon.nickname);
    let spec_name = pg_safe(&pokemon.species_name);
    let ability_name = pg_safe(&pokemon.ability_name);
    let area_name = pg_safe(&pokemon.area_name);
    let n = state.client.execute(
        "INSERT INTO dead_pokemon (
            run_id, player_name, personality, ot_id, ot_name, nickname,
            species, species_name, is_shiny, nature,
            level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
            move1, move2, move3, move4,
            pp1, pp2, pp3, pp4,
            iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
            ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
            held_item, ability, ability_name, friendship, met_location, died_at, gender,
            is_soul_link_death, killed_by_species, killed_by_move, area_name
        ) VALUES (
            $1,  $2,  $3,  $4,  $5,  $6,  $7,  $8,  $9,  $10, $11,
            $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22,
            $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33,
            $34, $35, $36, $37, $38, $39, $40, $41, $42, $43, $44, $45,
            $46, $47, $48, $49
        ) ON CONFLICT (run_id, personality) DO NOTHING",
        &[
            &(active as i32),
            &player, // $2  = player_name
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
            &pokemon.is_soul_link_death,
            &pokemon.killed_by_species,
            &pokemon.killed_by_move,
            &area_name, // $49
        ],
    )?;
    // execute() returns the number of rows affected. ON CONFLICT DO NOTHING yields 0,
    // meaning the record already exists — return false so callers don't re-fire events.
    Ok(n > 0)
}

/// Returns `true` if the Pokemon with this personality is dead in the active run.
pub fn is_dead(personality: u32) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    query_is_dead(&mut state.client, active, personality)
}

/// Returns the stored `DeadPokemon` entry for this personality in the active run.
pub fn get_dead_pokemon(personality: u32) -> Option<DeadPokemon> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let active = state.effective_run_id()?;
    let row = state.client
        .query_opt(
            "SELECT
                player_name, personality, ot_id, ot_name, nickname, species, species_name, is_shiny, nature,
                level, experience, max_hp, attack, defense, speed, sp_attack, sp_defense,
                move1, move2, move3, move4, pp1, pp2, pp3, pp4,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                held_item, ability, ability_name, friendship, met_location, died_at, gender,
                is_soul_link_death, killed_by_species, killed_by_move,
                COALESCE(area_name, '') AS area_name
             FROM dead_pokemon
             WHERE run_id = $1 AND personality = $2",
            &[&(active as i32), &(personality as i64)],
        )
        .ok()??;

    Some(row_to_dead_pokemon(&row))
}

// ---------------------------------------------------------------------------
// Public API — HP closest-call tracking
// ---------------------------------------------------------------------------

/// Record the lowest HP ratio (current_hp / max_hp) ever seen for a party
/// Pokémon in the current run. Called every game-loop tick for each live mon.
///
/// Skips mons with `hp == 0` (already dead) or `max_hp == 0` (invalid read).
/// Uses integer cross-multiplication to avoid floating-point comparisons in SQL.
pub fn update_min_hp_seen(personality: u32, hp: u16, max_hp: u16) {
    if hp == 0 || max_hp == 0 {
        return;
    }
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let hp_i = hp as i32;
    let max_i = max_hp as i32;
    // Update only if no previous record (IS NULL) or new ratio is strictly lower.
    // Cross-multiply to compare fractions without floats:
    //   new_hp/new_max < old_hp/old_max  ⟺  new_hp*old_max < old_hp*new_max
    let _ = state.client.execute(
        "UPDATE caught_pokemon
         SET min_hp_seen_hp     = CASE
               WHEN min_hp_seen_hp IS NULL
                 OR ($3::bigint * min_hp_seen_max_hp) < (min_hp_seen_hp::bigint * $4)
               THEN $3 ELSE min_hp_seen_hp END,
             min_hp_seen_max_hp = CASE
               WHEN min_hp_seen_hp IS NULL
                 OR ($3::bigint * min_hp_seen_max_hp) < (min_hp_seen_hp::bigint * $4)
               THEN $4 ELSE min_hp_seen_max_hp END
         WHERE run_id = $1 AND personality = $2",
        &[&(run_id as i32), &(personality as i64), &hp_i, &max_i],
    );
}

// ---------------------------------------------------------------------------
// Public API — per-Pokémon HP history
// ---------------------------------------------------------------------------

/// Record a timestamped HP observation for a party Pokémon.
///
/// Call this whenever the Pokémon's HP differs from the last-recorded value.
/// Uses the shared DB connection so it is safe to call from the game loop.
pub fn record_hp_observation(personality: u32, hp: u16, max_hp: u16) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let _ = state.client.execute(
        "INSERT INTO hp_history (run_id, personality, observed_at, hp, max_hp)
         VALUES ($1, $2, $3, $4, $5)",
        &[
            &(run_id as i32),
            &(personality as i64),
            &(unix_now() as i64),
            &(hp as i32),
            &(max_hp as i32),
        ],
    );
}

/// Record an enemy Pokémon's HP at the start or end of an encounter.
///
/// `phase` should be `"initial"` (battle start) or `"final"` (battle end).
/// Uses the shared DB connection.
pub fn record_enemy_hp(personality: u32, hp: u16, max_hp: u16, phase: &str) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let _ = state.client.execute(
        "INSERT INTO enemy_hp_log (run_id, personality, observed_at, hp, max_hp, phase)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            &(run_id as i32),
            &(personality as i64),
            &(unix_now() as i64),
            &(hp as i32),
            &(max_hp as i32),
            &phase,
        ],
    );
}

/// Returns the full HP history for one Pokémon in a run, ordered oldest-first.
///
/// Each entry: `{ observed_at, hp, max_hp, timestamp }`.
pub fn get_hp_history(conn_str: &str, run_id: u32, personality: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT observed_at, hp, max_hp FROM hp_history
         WHERE run_id = $1 AND personality = $2
         ORDER BY observed_at ASC",
        &[&(run_id as i32), &(personality as i64)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let history: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let at: i64 = row.get(0);
            let hp: i32 = row.get(1);
            let max_hp: i32 = row.get(2);
            serde_json::json!({
                "observed_at": at,
                "timestamp": format_timestamp(at as u64),
                "hp": hp,
                "max_hp": max_hp,
            })
        })
        .collect();
    serde_json::json!({
        "run_id": run_id,
        "personality": personality,
        "history": history,
    })
}

/// Returns all enemy HP observations for a run, grouped by encounter.
///
/// Each entry: `{ personality, initial_hp, initial_max_hp, final_hp,
/// final_max_hp, damage_dealt, timestamp }`.
pub fn get_enemy_hp_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT e.personality, e.hp, e.max_hp, e.phase, e.observed_at,
                COALESCE(enc.species_name, '') AS species_name
         FROM enemy_hp_log e
         LEFT JOIN encounters enc
               ON enc.run_id = e.run_id
              AND enc.id = (
                  SELECT id FROM encounters
                  WHERE run_id = $1
                  ORDER BY ABS(encountered_at - e.observed_at)
                  LIMIT 1
              )
         WHERE e.run_id = $1
         ORDER BY e.personality, e.observed_at",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    // Group by personality → {initial, final}
    use std::collections::BTreeMap;
    struct Obs {
        hp: i32,
        max_hp: i32,
        at: i64,
        species_name: String,
    }
    let mut encounters_map: BTreeMap<i64, (Option<Obs>, Option<Obs>)> = BTreeMap::new();
    for row in &rows {
        let personality: i64 = row.get(0);
        let hp: i32 = row.get(1);
        let max_hp: i32 = row.get(2);
        let phase: String = row.get(3);
        let at: i64 = row.get(4);
        let species_name: String = row.get(5);
        let obs = Obs { hp, max_hp, at, species_name };
        let entry = encounters_map.entry(personality).or_insert((None, None));
        if phase == "initial" {
            entry.0 = Some(obs);
        } else {
            entry.1 = Some(obs);
        }
    }
    let entries: Vec<serde_json::Value> = encounters_map
        .into_iter()
        .map(|(personality, (init, fin))| {
            let species = init.as_ref().or(fin.as_ref()).map(|o| o.species_name.clone()).unwrap_or_default();
            let init_hp = init.as_ref().map(|o| o.hp).unwrap_or(0);
            let init_max = init.as_ref().map(|o| o.max_hp).unwrap_or(0);
            let fin_hp = fin.as_ref().map(|o| o.hp);
            let fin_max = fin.as_ref().map(|o| o.max_hp);
            let damage = fin_hp.map(|fh| (init_hp - fh).max(0));
            let at = init.as_ref().or(fin.as_ref()).map(|o| o.at).unwrap_or(0);
            serde_json::json!({
                "personality": personality as u32,
                "species_name": species,
                "timestamp": format_timestamp(at as u64),
                "initial_hp": init_hp,
                "initial_max_hp": init_max,
                "final_hp": fin_hp,
                "final_max_hp": fin_max,
                "damage_dealt": damage,
            })
        })
        .collect();
    serde_json::json!({ "run_id": run_id, "encounters": entries })
}

/// Returns a battle-by-battle damage summary for a run.
///
/// Damage events (HP decreases) are grouped into battles using a 120-second
/// gap threshold — if no damage occurs for 120 s the next damage event opens
/// a new battle entry.
///
/// Each battle entry: `{ battle_index, start_at, end_at, duration_secs, mons }`.
/// Each mon entry: `{ personality, nickname, species_name, damage_taken }`.
pub fn get_battle_damage_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    // Fetch HP observations ordered by time for all party mons.
    let rows = match client.query(
        "SELECT h.personality, h.observed_at, h.hp, h.max_hp,
                COALESCE(cp.nickname, '') AS nickname,
                COALESCE(cp.species_name, '') AS species_name
         FROM hp_history h
         LEFT JOIN caught_pokemon cp
               ON cp.run_id = h.run_id AND cp.personality = h.personality
         WHERE h.run_id = $1
         ORDER BY h.personality, h.observed_at",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    // Build per-personality HP sequences, find decreases.
    use std::collections::HashMap;
    struct DamageEvent {
        personality: i64,
        at: i64,
        damage: i32,
        nickname: String,
        species_name: String,
    }

    let mut prev: HashMap<i64, (i64, i32)> = HashMap::new(); // personality → (at, hp)
    let mut mon_labels: HashMap<i64, (String, String)> = HashMap::new(); // personality → (nick, species)
    let mut damage_events: Vec<DamageEvent> = Vec::new();

    for row in &rows {
        let personality: i64 = row.get(0);
        let at: i64 = row.get(1);
        let hp: i32 = row.get(2);
        let _max_hp: i32 = row.get(3);
        let nickname: String = row.get(4);
        let species_name: String = row.get(5);
        mon_labels.entry(personality).or_insert((nickname.clone(), species_name.clone()));
        if let Some(&(_prev_at, prev_hp)) = prev.get(&personality)
            && hp < prev_hp
        {
            let (nick, spec) = mon_labels.get(&personality).cloned().unwrap_or_default();
            damage_events.push(DamageEvent {
                personality,
                at,
                damage: prev_hp - hp,
                nickname: nick,
                species_name: spec,
            });
        }
        prev.insert(personality, (at, hp));
    }

    // Sort damage events by time.
    damage_events.sort_by_key(|e| e.at);

    // Group into battles using 120-second gap threshold.
    const BATTLE_GAP_SECS: i64 = 120;
    struct Battle {
        start_at: i64,
        end_at: i64,
        mons: HashMap<i64, (i32, String, String)>, // personality → (damage, nick, species)
    }
    let mut battles: Vec<Battle> = Vec::new();
    for ev in &damage_events {
        if let Some(last) = battles.last_mut()
            && ev.at - last.end_at <= BATTLE_GAP_SECS
        {
            last.end_at = ev.at;
            let entry = last.mons.entry(ev.personality).or_insert((0, ev.nickname.clone(), ev.species_name.clone()));
            entry.0 += ev.damage;
            continue;
        }
        let mut mons = HashMap::new();
        mons.insert(ev.personality, (ev.damage, ev.nickname.clone(), ev.species_name.clone()));
        battles.push(Battle { start_at: ev.at, end_at: ev.at, mons });
    }

    let result: Vec<serde_json::Value> = battles
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut mon_list: Vec<serde_json::Value> = b.mons.iter().map(|(p, (dmg, nick, spec))| {
                serde_json::json!({
                    "personality": *p as u32,
                    "nickname": nick,
                    "species_name": spec,
                    "damage_taken": dmg,
                })
            }).collect();
            mon_list.sort_by(|a, b| b["damage_taken"].as_i64().cmp(&a["damage_taken"].as_i64()));
            let total: i32 = b.mons.values().map(|(d, _, _)| d).sum();
            serde_json::json!({
                "battle_index": i + 1,
                "start_at": b.start_at,
                "end_at": b.end_at,
                "start_timestamp": format_timestamp(b.start_at as u64),
                "end_timestamp": format_timestamp(b.end_at as u64),
                "duration_secs": b.end_at - b.start_at,
                "total_damage": total,
                "mons": mon_list,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "battles": result })
}

// ---------------------------------------------------------------------------
// Public API — analytics
// ---------------------------------------------------------------------------

/// Compare stats for multiple runs side-by-side.
///
/// Returns a JSON array, one entry per requested run ID. Fields per entry:
/// `id`, `player_name`, `started_at`, `ended_at`, `duration_secs`,
/// `total_encounters`, `catch_count`, `death_count`, `avg_death_level`.
pub fn run_comparison(conn_str: &str, run_ids: &[u32]) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    if run_ids.is_empty() {
        return serde_json::json!([]);
    }

    // Build a parameterised ANY($1) using an i32 array.
    let ids_i32: Vec<i32> = run_ids.iter().map(|&id| id as i32).collect();
    let rows = match client.query(
        "SELECT
             r.id,
             r.player_name,
             r.started_at,
             r.ended_at,
             (SELECT COUNT(*) FROM encounters   WHERE run_id = r.id)::bigint AS total_encounters,
             (SELECT COUNT(*) FROM caught_pokemon WHERE run_id = r.id)::bigint AS catch_count,
             (SELECT COUNT(*) FROM dead_pokemon   WHERE run_id = r.id)::bigint AS death_count,
             (SELECT AVG(level)::float FROM dead_pokemon WHERE run_id = r.id) AS avg_death_level
         FROM runs r
         WHERE r.id = ANY($1)
         ORDER BY r.id",
        &[&ids_i32],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let now = unix_now() as i64;
    let results: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let id: i32 = row.get(0);
            let player_name: String = row.get(1);
            let started_at: i64 = row.get(2);
            let ended_at: Option<i64> = row.get(3);
            let total_enc: i64 = row.get(4);
            let catch_count: i64 = row.get(5);
            let death_count: i64 = row.get(6);
            let avg_death_level: Option<f64> = row.get(7);
            let duration_secs = ended_at.unwrap_or(now) - started_at;
            serde_json::json!({
                "id": id,
                "player_name": player_name,
                "started_at": format_timestamp(started_at as u64),
                "ended_at": ended_at.map(|t| format_timestamp(t as u64)),
                "duration_secs": duration_secs,
                "playtime": format!("{}h {}m", duration_secs / 3600, (duration_secs % 3600) / 60),
                "total_encounters": total_enc,
                "catch_count": catch_count,
                "catch_rate_pct": if total_enc > 0 {
                    (catch_count as f64 / total_enc as f64 * 100.0 * 10.0).round() / 10.0
                } else { 0.0 },
                "death_count": death_count,
                "avg_death_level": avg_death_level.map(|v| (v * 10.0).round() / 10.0),
            })
        })
        .collect();

    serde_json::json!(results)
}

/// Luck / RNG analysis for a single run.
///
/// Returns a JSON object with:
/// - `total_encounters` — number of first encounters
/// - `shiny_count` — how many were shiny
/// - `expected_shinies` — `total_encounters / 8192.0`
/// - `shiny_rate_observed` — `shiny_count / total_encounters` (or null)
/// - `encounters` — per-area list with `area`, `species_name`, `level`, `is_shiny`, `caught`
pub fn run_luck_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT map_group, map_name, species_name, level, caught, is_shiny
         FROM encounters
         WHERE run_id = $1
         ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let total = rows.len() as u64;
    let shiny_count = rows.iter().filter(|r| r.get::<_, bool>(5)).count() as u64;
    let expected = total as f64 / 8192.0;
    let observed_rate = if total > 0 {
        serde_json::json!(shiny_count as f64 / total as f64)
    } else {
        serde_json::Value::Null
    };

    let enc_list: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mg = row.get::<_, i32>(0) as u8;
            let mn = row.get::<_, i32>(1) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "area": area,
                "species_name": row.get::<_, String>(2),
                "level": row.get::<_, i32>(3),
                "caught": row.get::<_, bool>(4),
                "is_shiny": row.get::<_, bool>(5),
            })
        })
        .collect();

    serde_json::json!({
        "run_id": run_id,
        "total_encounters": total,
        "shiny_count": shiny_count,
        "expected_shinies": (expected * 1000.0).round() / 1000.0,
        "shiny_rate_observed": observed_rate,
        "encounters": enc_list,
    })
}

/// Returns the 50 closest-call Pokémon for a run — those that reached the
/// lowest HP/max_HP ratio while alive — ordered from closest to farthest from
/// fainting.
///
/// Only Pokémon that have at least one recorded sub-max-HP observation appear.
/// The `is_dead` field is `true` when the Pokémon also has a row in
/// `dead_pokemon` (i.e. it eventually fainted).
pub fn closest_calls(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT cp.personality, cp.nickname, cp.species_name,
                cp.min_hp_seen_hp, cp.min_hp_seen_max_hp,
                (dp.personality IS NOT NULL) AS is_dead
         FROM caught_pokemon cp
         LEFT JOIN dead_pokemon dp
               ON dp.run_id = cp.run_id AND dp.personality = cp.personality
         WHERE cp.run_id = $1
           AND cp.min_hp_seen_hp IS NOT NULL
           AND cp.min_hp_seen_max_hp > 0
         ORDER BY (cp.min_hp_seen_hp::float / cp.min_hp_seen_max_hp) ASC
         LIMIT 50",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let personality: i64 = row.get(0);
            let nickname: String = row.get(1);
            let species_name: String = row.get(2);
            let hp: i16 = row.get(3);
            let max_hp: i16 = row.get(4);
            let is_dead: bool = row.get(5);
            let ratio = if max_hp > 0 {
                (hp as f64 / max_hp as f64 * 1000.0).round() / 10.0
            } else {
                0.0
            };
            serde_json::json!({
                "personality": personality as u32,
                "nickname": nickname,
                "species_name": species_name,
                "min_hp": hp,
                "min_max_hp": max_hp,
                "hp_pct": ratio,
                "is_dead": is_dead,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "closest_calls": entries })
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

/// A notable gameplay event to persist in the `events` table.
///
/// Events are a supplementary audit trail alongside the `dead_pokemon`,
/// `caught_pokemon`, and `encounters` tables. They are append-only and ordered
/// by `occurred_at`, making them suitable for streaming or timeline displays.
pub enum EventKind<'a> {
    /// A Pokémon was caught and added to the party.
    Catch {
        species_name: &'a str,
        nickname: &'a str,
        level: u8,
    },
    /// A Pokémon fainted from direct in-game damage.
    Death {
        species_name: &'a str,
        nickname: &'a str,
        level: u8,
    },
    /// A Pokémon was killed by the Soul Link rule.
    SoulLinkDeath {
        species_name: &'a str,
        nickname: &'a str,
        level: u8,
    },
    /// A shiny Pokémon appeared in the wild.
    Shiny { species_name: &'a str, level: u8 },
    /// The party was wiped, ending the run.
    Wipe,
    /// A gym badge (or E4 win) was earned.
    Badge { badge_name: &'a str },
    /// A caught Pokémon's nickname was changed in-game.
    NicknameChange {
        species_name: &'a str,
        old_name: &'a str,
        new_name: &'a str,
    },
}

impl<'a> EventKind<'a> {
    /// Extracts `(event_type, species_name, nickname, old_nickname, level)` for a DB INSERT.
    fn row_parts(&self) -> (&'static str, &'a str, &'a str, &'a str, i32) {
        match self {
            EventKind::Catch {
                species_name,
                nickname,
                level,
            } => ("catch", species_name, nickname, "", *level as i32),
            EventKind::Death {
                species_name,
                nickname,
                level,
            } => ("death", species_name, nickname, "", *level as i32),
            EventKind::SoulLinkDeath {
                species_name,
                nickname,
                level,
            } => ("soul_link_death", species_name, nickname, "", *level as i32),
            EventKind::Shiny {
                species_name,
                level,
            } => ("shiny", species_name, "", "", *level as i32),
            EventKind::Wipe => ("wipe", "", "", "", 0),
            EventKind::Badge { badge_name } => ("badge", badge_name, "", "", 0),
            EventKind::NicknameChange {
                species_name,
                old_name,
                new_name,
            } => ("nickname_change", species_name, new_name, old_name, 0),
        }
    }
}

/// Appends a row to the `events` table for the active run.
///
/// No-op if no run is currently active. Returns `true` when the event was
/// successfully persisted, `false` on any failure or missing run.
/// Appends a row to the `events` table. Returns `Ok(())` on success,
/// `Ok(())` with a no-op when there is no active run, and `Err` on a DB error.
pub fn record_event(event: EventKind<'_>) -> Result<(), postgres::Error> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    let run_id = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(()),
    };
    let player = state.current_player.clone();
    let occurred_at = unix_now() as i64;
    let (event_type, species_name, nickname, old_nickname, level) = event.row_parts();
    state.client.execute(
        "INSERT INTO events (run_id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        &[
            &(run_id as i32),
            &player,
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

/// Records a defeated trainer in the `trainer_battles` table for the active run.
///
/// No-op if no run is currently active or if this flag has already been recorded
/// (`ON CONFLICT DO NOTHING` on `(run_id, player_name, flag_index)`).
/// Returns `Ok(true)` when a new row was inserted.
pub fn record_trainer_defeat(defeat: TrainerDefeat) -> Result<bool, postgres::Error> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let run_id = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(false),
    };
    let player = pg_safe(&state.current_player);
    let n = state.client.execute(
        "INSERT INTO trainer_battles (run_id, player_name, flag_index, trainer_name, location, defeated_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (run_id, player_name, flag_index) DO NOTHING",
        &[
            &(run_id as i32),
            &player,
            &(defeat.flag_index as i32),
            &defeat.trainer_name,
            &defeat.location,
            &(defeat.defeated_at as i64),
        ],
    )?;
    Ok(n > 0)
}

/// Returns all trainer defeats for a run as a JSON array, ordered by time.
///
/// Each entry has: `player_name`, `flag_index`, `trainer_name`, `location`,
/// `defeated_at` (unix seconds), `defeated_at_human` (formatted string).
pub fn get_trainer_defeats_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let Ok(mut client) = postgres::Client::connect(conn_str, NoTls) else {
        return serde_json::json!({ "error": "database connection failed" });
    };
    let rows = match client.query(
        "SELECT player_name, flag_index, trainer_name, location, defeated_at
         FROM trainer_battles WHERE run_id = $1 ORDER BY defeated_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": e.to_string() }),
    };
    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let defeated_at = row.get::<_, i64>(4) as u64;
            serde_json::json!({
                "player_name":     row.get::<_, String>(0),
                "flag_index":      row.get::<_, i32>(1),
                "trainer_name":    row.get::<_, String>(2),
                "location":        row.get::<_, String>(3),
                "defeated_at":     defeated_at,
                "defeated_at_human": format_timestamp(defeated_at),
            })
        })
        .collect();
    serde_json::json!(entries)
}

/// Records a Pokemon as caught in the active run.
///
/// No-op if this personality is already recorded (deduplicates on reconnect).
/// Returns `true` when a new row was inserted, `false` when the record already
/// existed (`ON CONFLICT DO NOTHING`) or no active run is set. Callers must
/// only fire downstream events (event log, webhooks) on `true`.
pub fn mark_caught(pokemon: CaughtPokemon) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = pg_safe(&state.current_player);
    let nickname = pg_safe(&pokemon.nickname);
    let spec_name = pg_safe(&pokemon.species_name);
    match state.client.execute(
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
        Ok(n) => n > 0,
        Err(e) => {
            tracing::warn!("Failed to record caught pokemon (personality={}): {e}", pokemon.personality);
            false
        }
    }
}

/// Updates the nickname of a caught Pokémon if it has changed.
///
/// No-op if the Pokémon is not registered, the run is not active, or the
/// nickname matches what is already stored.
/// Updates the in-game nickname of a caught Pokémon.
///
/// Returns `Some(old_name)` when the stored nickname differed and was updated,
/// or `None` if the name was already up to date, the Pokémon is not found, or
/// no active run is set.
pub fn update_caught_nickname(personality: u32, nickname: &str) -> Option<String> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let active = state.effective_run_id()?;
    // Read the current name first so we can return it as the "old" value.
    // On SELECT error, attempt the UPDATE anyway as a best-effort sync — but
    // note that if the client is in a broken state the UPDATE will also fail
    // silently (result is discarded). We still return None rather than panic.
    let old: Option<String> = match state.client.query_opt(
        "SELECT nickname FROM caught_pokemon
         WHERE run_id = $1 AND personality = $2 AND nickname != $3",
        &[&(active as i32), &(personality as i64), &nickname],
    ) {
        Ok(maybe_row) => maybe_row.map(|row| row.get(0)),
        Err(_) => {
            let _ = state.client.execute(
                "UPDATE caught_pokemon SET nickname = $1
                 WHERE run_id = $2 AND personality = $3 AND nickname != $1",
                &[&nickname, &(active as i32), &(personality as i64)],
            );
            return None;
        }
    };

    if let Some(old_name) = old {
        let _ = state.client.execute(
            "UPDATE caught_pokemon SET nickname = $1
             WHERE run_id = $2 AND personality = $3",
            &[&nickname, &(active as i32), &(personality as i64)],
        );
        Some(old_name)
    } else {
        None
    }
}

/// Updates the EVs of a caught Pokémon if any have changed.
///
/// No-op if the Pokémon is not registered, the run is not active, or all EVs
/// match what is already stored.
pub fn update_caught_evs(personality: u32, evs: &EVs) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
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
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon WHERE run_id = $1 AND personality = $2",
            &[&(active as i32), &(personality as i64)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns all caught Pokemon for the active run for the current player.
pub fn list_caught() -> Vec<CaughtPokemon> {
    let Some(db) = db() else { return vec![] };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
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
/// Records a wild encounter. Returns `Ok(true)` when the row was newly inserted
/// (first encounter for this area), `Ok(false)` when the encounter already
/// exists or there is no active run, and `Err` on a DB error.
pub fn record_encounter(encounter: Encounter) -> Result<bool, postgres::Error> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return Ok(false),
    };
    let player = pg_safe(&state.current_player);
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
    )?;
    Ok(rows == 1)
}

/// Marks the current player's encounter for this area as successfully caught.
pub fn set_encounter_caught(map_group: u8, map_name: u8) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return,
    };
    let player = state.current_player.clone();
    if let Err(e) = state.client.execute(
        "UPDATE encounters SET caught = TRUE
         WHERE run_id = $1 AND player_name = $2 AND map_group = $3 AND map_name = $4",
        &[
            &(active as i32),
            &player,
            &(map_group as i32),
            &(map_name as i32),
        ],
    ) {
        tracing::warn!("set_encounter_caught: DB error: {}", e);
    }
}

/// Records the outcome of a tracked wild encounter (first-per-area Nuzlocke slot).
///
/// Called by the encounter tracker when the encounter resolves — either a catch
/// or the next battle personality replacing the current one (fled/fainted).
/// Silently no-ops when there is no active run.
pub fn record_catch_attempt(
    species_name: &str,
    area: &str,
    balls_thrown: u32,
    caught: bool,
    encountered_at: u64,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return,
    };
    let player = pg_safe(&state.current_player);
    let spec = pg_safe(species_name);
    let area_s = pg_safe(area);
    if let Err(e) = state.client.execute(
        "INSERT INTO catch_attempts
             (run_id, player_name, species_name, area, balls_thrown, caught, encountered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &(active as i32),
            &player,
            &spec,
            &area_s,
            &(balls_thrown as i32),
            &caught,
            &(encountered_at as i64),
        ],
    ) {
        tracing::warn!("record_catch_attempt: DB error: {e}");
    }
}

/// Records the start of a new area visit.  Returns the row `id` so the caller
/// can later close it with [`close_area_visit`].  Returns `None` when there is
/// no active run or the insert fails.
pub fn open_area_visit(map_group: u8, map_name: u8, area_name: &str, entered_at: u64) -> Option<i64> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let active = state.effective_run_id()? as i32;
    let player = pg_safe(&state.current_player);
    let area_s = pg_safe(area_name);
    state
        .client
        .query_one(
            "INSERT INTO area_visits (run_id, player_name, map_group, map_name, area_name, entered_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id",
            &[
                &active,
                &player,
                &(map_group as i32),
                &(map_name as i32),
                &area_s,
                &(entered_at as i64),
            ],
        )
        .ok()
        .map(|row| row.get::<_, i32>(0) as i64)
}

/// Closes an open area visit by setting `exited_at`.  Silently ignores errors.
pub fn close_area_visit(visit_id: i64, exited_at: u64) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    if let Err(e) = state.client.execute(
        "UPDATE area_visits SET exited_at = $1 WHERE id = $2",
        &[&(exited_at as i64), &(visit_id as i32)],
    ) {
        tracing::warn!("close_area_visit: DB error: {e}");
    }
}

/// Returns `true` if a Pokémon with this species ID exists in the `caught_pokemon`
/// table for the active run under any player.
///
/// Used to enforce the dupes clause: when enabled, a new encounter is skipped if
/// the species was already caught at any point in the current run, regardless of
/// which area it was encountered in.
pub fn species_caught_any(species: u16) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon WHERE run_id = $1 AND species = $2",
            &[&(active as i32), &(species as i32)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns `true` if a Pokémon with this species ID exists in the `caught_pokemon`
/// table for the active run under the **current player only**.
///
/// Used to enforce the per-player dupes clause: a new encounter is skipped if
/// this player has already caught the species at any point in the current run.
pub fn species_caught_by_self(species: u16) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon \
             WHERE run_id = $1 AND player_name = $2 AND species = $3",
            &[&(active as i32), &player, &(species as i32)],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns `true` if this species has already been recorded as a first encounter
/// anywhere in the active run for the current player.
pub fn species_encountered(species: u16) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();
    state
        .client
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
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    state
        .client
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
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();

    // Build a single EXISTS query with one OR-clause per floor so we only
    // issue one round-trip instead of N while holding the DB mutex.
    // N is always small (≤5 for multi-floor dungeons), so dynamic query
    // construction is safe and the query planner handles it fine.
    use std::fmt::Write as _;
    let mut cond = String::new();
    let floor_pairs: Vec<(i32, i32)> = floors
        .iter()
        .map(|&(mg, mn)| (mg as i32, mn as i32))
        .collect();
    for i in 0..floor_pairs.len() {
        if i > 0 {
            cond.push_str(" OR ");
        }
        write!(
            &mut cond,
            "(map_group=${} AND map_name=${})",
            3 + i * 2,
            4 + i * 2
        )
        .unwrap();
    }
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM encounters \
         WHERE run_id = $1 AND player_name = $2 AND ({cond}))"
    );
    let active_i32 = active as i32;
    let mut params: Vec<&(dyn postgres::types::ToSql + Sync)> = vec![&active_i32, &player];
    for (mg, mn) in &floor_pairs {
        params.push(mg);
        params.push(mn);
    }
    state
        .client
        .query_one(&sql, &params)
        .map(|row| row.get::<_, bool>(0))
        .unwrap_or(false)
}

/// Returns `true` if an encounter has already been recorded for this area by the current player.
pub fn has_encounter(map_group: u8, map_name: u8) -> bool {
    let Some(db) = db() else { return false };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return false,
    };
    let player = state.current_player.clone();
    state
        .client
        .query_one(
            "SELECT COUNT(*) FROM encounters
             WHERE run_id = $1 AND player_name = $2 AND map_group = $3 AND map_name = $4",
            &[
                &(active as i32),
                &player,
                &(map_group as i32),
                &(map_name as i32),
            ],
        )
        .map(|row| row.get::<_, i64>(0) > 0)
        .unwrap_or(false)
}

/// Returns all encounters for the active run, ordered by time.
pub fn list_encounters() -> Vec<Encounter> {
    let Some(db) = db() else { return vec![] };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
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

/// Upserts a soul-link override for the active run: `personality` will be
/// linked to `partner_personality` regardless of met_location.
///
/// Replaces any existing override for the same personality in this run.
pub fn set_soul_link_override(personality: u32, partner_personality: u32) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => {
            tracing::warn!("set_soul_link_override: no active run");
            return;
        }
    };
    let p = personality as i64;
    let pp = partner_personality as i64;
    let now = unix_now() as i64;
    let run_i32 = active as i32;
    if let Err(e) = state.client.execute(
        "INSERT INTO soul_link_overrides (run_id, personality, partner_personality, created_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (run_id, personality)
         DO UPDATE SET partner_personality = EXCLUDED.partner_personality,
                       created_at          = EXCLUDED.created_at",
        &[&run_i32, &p, &pp, &now],
    ) {
        tracing::warn!("set_soul_link_override: DB error: {e}");
    }
}

/// Removes the soul-link override for `personality` in the active run.
///
/// After this call the automatic met_location / receipt-order pairing resumes.
pub fn clear_soul_link_override(personality: u32) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let active = match state.effective_run_id() {
        Some(id) => id,
        None => return,
    };
    let p = personality as i64;
    let run_i32 = active as i32;
    if let Err(e) = state.client.execute(
        "DELETE FROM soul_link_overrides WHERE run_id = $1 AND personality = $2",
        &[&run_i32, &p],
    ) {
        tracing::warn!("clear_soul_link_override: DB error: {e}");
    }
}

fn normalize_conn_str(s: &str) -> String {
    if s.starts_with("postgresql://") || s.starts_with("postgres://") || s.contains('=') {
        s.to_string()
    } else {
        format!("postgresql://{s}")
    }
}

// ---------------------------------------------------------------------------
// Run goals — user-defined checklist stored in `run_goals`
// ---------------------------------------------------------------------------

/// A single user-defined goal row.
#[derive(Debug, Clone)]
pub struct GoalRow {
    pub id: i32,
    pub text: String,
    pub completed: bool,
}

/// Creates a new goal for `run_id` and returns its assigned `id`.
///
/// Returns `None` on DB error or when the database is not initialized.
pub fn create_goal(conn_str: &str, run_id: u32, text: &str) -> Option<i32> {
    let conn_str = normalize_conn_str(conn_str);
    let mut client = Client::connect(&conn_str, NoTls).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let row = client
        .query_one(
            "INSERT INTO run_goals (run_id, text, completed, created_at)
             VALUES ($1, $2, FALSE, $3) RETURNING id",
            &[&(run_id as i32), &text, &now],
        )
        .ok()?;
    Some(row.get(0))
}

/// Marks the goal with `goal_id` as completed.  Returns `true` if a row was updated.
pub fn complete_goal(conn_str: &str, goal_id: i32) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute(
            "UPDATE run_goals SET completed = TRUE WHERE id = $1",
            &[&goal_id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Sets the `completed` flag on `goal_id` to `completed`.  Returns `true` if a row was updated.
pub fn set_goal_completed(conn_str: &str, goal_id: i32, completed: bool) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute(
            "UPDATE run_goals SET completed = $2 WHERE id = $1",
            &[&goal_id, &completed],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Deletes the goal with `goal_id`.  Returns `true` if a row was deleted.
pub fn delete_goal(conn_str: &str, goal_id: i32) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute("DELETE FROM run_goals WHERE id = $1", &[&goal_id])
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Returns all goals for `run_id`, ordered by creation time.
pub fn list_goals_for_run(conn_str: &str, run_id: u32) -> Vec<GoalRow> {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return vec![] };
    client
        .query(
            "SELECT id, text, completed FROM run_goals WHERE run_id = $1 ORDER BY created_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default()
        .into_iter()
        .map(|row| GoalRow {
            id:        row.get(0),
            text:      row.get(1),
            completed: row.get(2),
        })
        .collect()
}

/// Returns the `run_id` for the goal with `goal_id`, or `None` if not found.
pub fn get_run_id_for_goal(conn_str: &str, goal_id: i32) -> Option<u32> {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return None };
    let row = client
        .query_opt("SELECT run_id FROM run_goals WHERE id = $1", &[&goal_id])
        .ok()??;
    Some(row.get::<_, i32>(0) as u32)
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

    /// Returns the pinned display-column index for the current run, or `None`.
    pub fn query_slot_index(&self) -> Option<u8> {
        let run_id = self.get_run_id()? as i32;
        let row = self
            .client
            .lock_or_recover()
            .query_opt("SELECT slot_index FROM runs WHERE id = $1", &[&run_id])
            .ok()??;
        let v: Option<i32> = row.get(0);
        v.and_then(|n| u8::try_from(n).ok())
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
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let messages = match client.simple_query(sql) {
        Ok(m) => m,
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
                let obj: serde_json::Map<String, serde_json::Value> = row
                    .columns()
                    .iter()
                    .enumerate()
                    .map(|(i, col)| {
                        let val = row
                            .get(i)
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
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let run_row = match client.query_opt(
        "SELECT player_name, started_at, ended_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return serde_json::json!({ "error": "Run not found" }),
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let player_name: String = run_row.get(0);
    let started_at: i64 = run_row.get(1);
    let ended_at: Option<i64> = run_row.get(2);

    let now = unix_now() as i64;
    let duration_secs = ended_at.unwrap_or(now) - started_at;
    let playtime = format!("{}h {}m", duration_secs / 3600, (duration_secs % 3600) / 60);

    let enc_rows = client
        .query(
            "SELECT map_group, map_name, species_name, level, caught, is_shiny, encountered_at
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default();

    let total_encounters = enc_rows.len();
    let total_caught: usize = enc_rows.iter().filter(|r| r.get::<_, bool>(4)).count();
    let catch_rate = if total_encounters > 0 {
        (total_caught as f64 / total_encounters as f64 * 100.0).round()
    } else {
        0.0
    };

    let zone_stats: Vec<serde_json::Value> = enc_rows
        .iter()
        .map(|row| {
            let mg = row.get::<_, i32>(0) as u8;
            let mn = row.get::<_, i32>(1) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "area":          area,
                "species_name":  row.get::<_, String>(2),
                "level":         row.get::<_, i32>(3),
                "caught":        row.get::<_, bool>(4),
                "is_shiny":      row.get::<_, bool>(5),
                "encountered_at": format_timestamp(row.get::<_, i64>(6) as u64),
            })
        })
        .collect();

    let dead_rows = client
        .query(
            "SELECT level, species_name, met_location, died_at, is_soul_link_death
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default();

    let total_deaths = dead_rows.len();
    let avg_death_level = if total_deaths > 0 {
        let total: i64 = dead_rows.iter().map(|r| r.get::<_, i32>(0) as i64).sum();
        (total as f64 / total_deaths as f64).round()
    } else {
        0.0
    };

    let deaths: Vec<serde_json::Value> = dead_rows
        .iter()
        .map(|row| {
            let met_loc = row.get::<_, i32>(2) as u8;
            let raw = fire_red_location_names::location_name(met_loc);
            let location = if raw.is_empty() {
                format!("loc {}", met_loc)
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "level":        row.get::<_, i32>(0),
                "species_name": row.get::<_, String>(1),
                "location":     location,
                "died_at":      format_timestamp(row.get::<_, i64>(3) as u64),
                "soul_link":    row.get::<_, bool>(4),
            })
        })
        .collect();

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

/// Returns per-route catch statistics for the given run ID as JSON.
///
/// Each entry in `zones` covers one (map_group, map_name) pair and includes
/// the encounter count, catch count, and catch-rate percentage. Opens its own
/// connection so live tracker connections are not blocked.
pub fn route_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT map_group, map_name,
                COUNT(*) AS total,
                SUM(CASE WHEN caught THEN 1 ELSE 0 END) AS caught_count
         FROM encounters
         WHERE run_id = $1
         GROUP BY map_group, map_name
         ORDER BY map_group, map_name",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let zones: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mg: u8 = row.get::<_, i32>(0) as u8;
            let mn: u8 = row.get::<_, i32>(1) as u8;
            let total: i64 = row.get(2);
            let caught: i64 = row.get(3);
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            let catch_rate = if total > 0 {
                (caught as f64 / total as f64 * 100.0).round()
            } else {
                0.0
            };
            serde_json::json!({
                "map_group":      mg,
                "map_name":       mn,
                "area":           area,
                "total":          total,
                "caught":         caught,
                "catch_rate_pct": catch_rate,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "zones": zones })
}

/// Returns badge split times for the given run as JSON.
///
/// Each entry in `splits` has `badge_name`, `earned_at` (formatted timestamp),
/// `elapsed_secs` (seconds since run started), and `split_secs` (seconds since
/// the previous badge, or since run start for the first badge).
pub fn badge_splits(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let started_at: i64 = match client.query_opt(
        "SELECT started_at FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ) {
        Ok(Some(r)) => r.get(0),
        Ok(None) => return serde_json::json!({ "error": "Run not found" }),
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let rows = match client.query(
        "SELECT species_name, occurred_at
         FROM events
         WHERE run_id = $1 AND event_type = 'badge'
         ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let mut prev_ts = started_at;
    let splits: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let badge_name: String = row.get(0);
            let occurred_at: i64 = row.get(1);
            let elapsed = (occurred_at - started_at).max(0);
            let split = (occurred_at - prev_ts).max(0);
            prev_ts = occurred_at;
            serde_json::json!({
                "badge_name":   badge_name,
                "earned_at":    format_timestamp(occurred_at as u64),
                "elapsed_secs": elapsed,
                "split_secs":   split,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "started_at": format_timestamp(started_at as u64), "splits": splits })
}

/// Returns catch-attempt log for the given run as JSON.
///
/// Each entry covers one wild encounter (Nuzlocke first-per-area only) and
/// includes `species_name`, `area`, `balls_thrown`, `caught`, and
/// `encountered_at`.  Summary totals are included at the top level.
pub fn catch_attempt_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT player_name, species_name, area, balls_thrown, caught, encountered_at
         FROM catch_attempts
         WHERE run_id = $1
         ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let mut total_balls: i64 = 0;
    let mut max_balls: i32 = 0;
    let mut worst_encounter = String::new();

    let attempts: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let player_name: String = row.get(0);
            let species_name: String = row.get(1);
            let area: String = row.get(2);
            let balls_thrown: i32 = row.get(3);
            let caught: bool = row.get(4);
            let encountered_at: i64 = row.get(5);
            total_balls += balls_thrown as i64;
            if balls_thrown > max_balls {
                max_balls = balls_thrown;
                worst_encounter = format!("{} ({})", species_name, area);
            }
            serde_json::json!({
                "player_name":    player_name,
                "species_name":   species_name,
                "area":           area,
                "balls_thrown":   balls_thrown,
                "caught":         caught,
                "encountered_at": format_timestamp(encountered_at as u64),
            })
        })
        .collect();

    serde_json::json!({
        "run_id":          run_id,
        "total_balls_thrown": total_balls,
        "most_balls_in_one_encounter": max_balls,
        "hardest_encounter": worst_encounter,
        "attempts":        attempts,
    })
}

/// Returns a composite difficulty score (0–100) for the given run, plus the
/// component breakdown used to compute it.
///
/// Components:
/// - `death_ratio`  (40 %) — deaths / (deaths + survivors) × 100
/// - `hp_danger`    (30 %) — avg "danger fraction" (1 − min_hp/max_hp) × 100
/// - `catch_miss`   (20 %) — (total_encounters − caught) / total_encounters × 100
/// - `trainer_load` (10 %) — min(trainer_count / 80, 1.0) × 100
pub fn difficulty_score(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rid = run_id as i32;

    let death_count: i64 = client
        .query_one("SELECT COUNT(*) FROM dead_pokemon WHERE run_id = $1", &[&rid])
        .map(|r| r.get(0))
        .unwrap_or(0);

    let survivor_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM caught_pokemon cp
             WHERE cp.run_id = $1
               AND NOT EXISTS (
                   SELECT 1 FROM dead_pokemon dp
                   WHERE dp.run_id = $1 AND dp.personality = cp.personality
               )",
            &[&rid],
        )
        .map(|r| r.get(0))
        .unwrap_or(0);

    let total_pokemon = death_count + survivor_count;
    let death_ratio = if total_pokemon > 0 {
        death_count as f64 / total_pokemon as f64 * 100.0
    } else {
        0.0
    };

    // HP danger: average of (1 - min_hp/max_hp) for all mons with recorded HP
    let hp_rows = client
        .query(
            "SELECT min_hp_seen_hp, min_hp_seen_max_hp
             FROM caught_pokemon
             WHERE run_id = $1
               AND min_hp_seen_hp IS NOT NULL
               AND min_hp_seen_max_hp > 0",
            &[&rid],
        )
        .unwrap_or_default();

    let hp_danger = if hp_rows.is_empty() {
        0.0
    } else {
        let sum: f64 = hp_rows
            .iter()
            .map(|r| {
                let hp: i16 = r.get(0);
                let max_hp: i16 = r.get(1);
                1.0 - (hp as f64 / max_hp as f64)
            })
            .sum();
        sum / hp_rows.len() as f64 * 100.0
    };

    let enc_row = client
        .query_one(
            "SELECT COUNT(*), SUM(CASE WHEN caught THEN 1 ELSE 0 END)
             FROM encounters WHERE run_id = $1",
            &[&rid],
        )
        .ok();
    let (total_enc, total_caught): (i64, i64) = enc_row
        .as_ref()
        .map(|r| (r.get(0), r.get::<_, Option<i64>>(1).unwrap_or(0)))
        .unwrap_or((0, 0));

    let catch_miss = if total_enc > 0 {
        (total_enc - total_caught) as f64 / total_enc as f64 * 100.0
    } else {
        0.0
    };

    let trainer_count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM trainer_battles WHERE run_id = $1",
            &[&rid],
        )
        .map(|r| r.get(0))
        .unwrap_or(0);

    let trainer_load = (trainer_count as f64 / 80.0).min(1.0) * 100.0;

    let score = (0.40 * death_ratio + 0.30 * hp_danger + 0.20 * catch_miss + 0.10 * trainer_load)
        .clamp(0.0, 100.0);

    serde_json::json!({
        "run_id":        run_id,
        "difficulty":    (score * 10.0).round() / 10.0,
        "components": {
            "death_ratio_pct":   (death_ratio  * 10.0).round() / 10.0,
            "hp_danger_pct":     (hp_danger    * 10.0).round() / 10.0,
            "catch_miss_pct":    (catch_miss   * 10.0).round() / 10.0,
            "trainer_load_pct":  (trainer_load * 10.0).round() / 10.0,
        },
        "raw": {
            "deaths":         death_count,
            "survivors":      survivor_count,
            "total_encounters": total_enc,
            "total_caught":   total_caught,
            "trainer_battles": trainer_count,
        }
    })
}

/// Returns time spent per map area for the given run, sorted by total seconds
/// descending.  Open visits (player still there) use the current time as the
/// exit.
pub fn area_time_breakdown(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let now = unix_now() as i64;
    let rows = match client.query(
        "SELECT area_name, map_group, map_name,
                SUM(COALESCE(exited_at, $2) - entered_at) AS total_secs,
                COUNT(*) AS visits
         FROM area_visits
         WHERE run_id = $1
         GROUP BY area_name, map_group, map_name
         ORDER BY total_secs DESC",
        &[&(run_id as i32), &now],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let areas: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let area_name: String = row.get(0);
            let map_group: i32 = row.get(1);
            let map_name: i32 = row.get(2);
            let total_secs: i64 = row.get(3);
            let visits: i64 = row.get(4);
            let hours = total_secs / 3600;
            let mins = (total_secs % 3600) / 60;
            let secs = total_secs % 60;
            serde_json::json!({
                "area_name":   area_name,
                "map_group":   map_group,
                "map_name":    map_name,
                "total_secs":  total_secs,
                "formatted":   format!("{}h {:02}m {:02}s", hours, mins, secs),
                "visits":      visits,
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "areas": areas })
}

/// Returns shiny encounter statistics for the given run ID as JSON.
///
/// Counts total encounters, total shinies, and encounters since the last shiny.
/// Opens its own connection so live tracker connections are not blocked.
pub fn shiny_stats(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = client
        .query(
            "SELECT species_name, encountered_at, is_shiny, map_group, map_name, level, caught
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default();

    let total_encounters = rows.len();
    let total_shinies: usize = rows.iter().filter(|r| r.get::<_, bool>(2)).count();

    let last_shiny_idx = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.get::<_, bool>(2))
        .map(|(i, _)| i)
        .last();

    let (encounters_since_shiny, last_shiny) = match last_shiny_idx {
        Some(idx) => {
            let sr = &rows[idx];
            let mg = sr.get::<_, i32>(3) as u8;
            let mn = sr.get::<_, i32>(4) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
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
    let since_last_shiny: Vec<serde_json::Value> = rows[recent_start..]
        .iter()
        .map(|row| {
            let mg = row.get::<_, i32>(3) as u8;
            let mn = row.get::<_, i32>(4) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{}:{}", mg, mn)
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "species_name":  row.get::<_, String>(0),
                "encountered_at": format_timestamp(row.get::<_, i64>(1) as u64),
                "area":          area,
                "level":         row.get::<_, i32>(5),
                "caught":        row.get::<_, bool>(6),
            })
        })
        .collect();

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
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    client
        .batch_execute(
            "
        DELETE FROM encounters;
        DELETE FROM caught_pokemon;
        DELETE FROM dead_pokemon;
        DELETE FROM runs;
        DELETE FROM meta WHERE key = 'active_run_id';
    ",
        )
        .map_err(|e| format!("Clear failed: {e}"))
}

/// Returns a JSON export of a single run: metadata, caught, dead, and encounter lists.
///
/// Opens its own connection so the live tracker connection is not blocked.
pub fn export_run(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rid = run_id as i32;

    let run_row = match client.query_opt(
        "SELECT id, player_name, started_at, ended_at FROM runs WHERE id = $1",
        &[&rid],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return serde_json::json!({ "error": format!("Run {run_id} not found") }),
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };

    let caught_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, location_name, caught_at, player_name, personality, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM caught_pokemon WHERE run_id = $1 ORDER BY caught_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run caught query failed for run {run_id}: {e}");
            vec![]
        });

    let dead_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, died_at, player_name, is_soul_link_death, personality, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run dead query failed for run {run_id}: {e}");
            vec![]
        });

    let enc_rows = client
        .query(
            "SELECT species_name, level, map_group, map_name, caught, is_shiny, \
                encountered_at, player_name \
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run encounters query failed for run {run_id}: {e}");
            vec![]
        });

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
            "personality":   r.get::<_, i64>(10),
            "iv_hp":         r.get::<_, i32>(11),
            "iv_atk":        r.get::<_, i32>(12),
            "iv_def":        r.get::<_, i32>(13),
            "iv_spe":        r.get::<_, i32>(14),
            "iv_spa":        r.get::<_, i32>(15),
            "iv_spd":        r.get::<_, i32>(16),
            "ev_hp":         r.get::<_, i32>(17),
            "ev_atk":        r.get::<_, i32>(18),
            "ev_def":        r.get::<_, i32>(19),
            "ev_spe":        r.get::<_, i32>(20),
            "ev_spa":        r.get::<_, i32>(21),
            "ev_spd":        r.get::<_, i32>(22),
        })).collect::<Vec<_>>(),
        "dead": dead_rows.iter().map(|r| serde_json::json!({
            "nickname":          r.get::<_, String>(0),
            "species_name":      r.get::<_, String>(1),
            "level":             r.get::<_, i32>(2),
            "nature":            r.get::<_, String>(3),
            "is_shiny":          r.get::<_, bool>(4),
            "gender":            r.get::<_, i32>(5),
            "met_location":      r.get::<_, i32>(6),
            "died_at":           format_timestamp(r.get::<_, i64>(7) as u64),
            "player_name":       r.get::<_, String>(8),
            "is_soul_link_death": r.get::<_, bool>(9),
            "personality":       r.get::<_, i64>(10),
            "iv_hp":             r.get::<_, i32>(11),
            "iv_atk":            r.get::<_, i32>(12),
            "iv_def":            r.get::<_, i32>(13),
            "iv_spe":            r.get::<_, i32>(14),
            "iv_spa":            r.get::<_, i32>(15),
            "iv_spd":            r.get::<_, i32>(16),
            "ev_hp":             r.get::<_, i32>(17),
            "ev_atk":            r.get::<_, i32>(18),
            "ev_def":            r.get::<_, i32>(19),
            "ev_spe":            r.get::<_, i32>(20),
            "ev_spa":            r.get::<_, i32>(21),
            "ev_spd":            r.get::<_, i32>(22),
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

/// Returns a CSV export of a single run: three sections separated by blank lines.
///
/// Sections: `caught`, `dead`, `encounters`. Each section has a header row.
/// Opens its own connection so the live tracker is not blocked.
pub fn export_run_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let mut out = String::new();

    // Caught Pokémon
    out.push_str(
        "section,player_name,nickname,species_name,level,nature,is_shiny,gender,\
met_location,location_name,iv_hp,iv_atk,iv_def,iv_spe,iv_spa,iv_spd,\
ev_hp,ev_atk,ev_def,ev_spe,ev_spa,ev_spd,caught_at\n",
    );
    let caught_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, location_name, caught_at, player_name, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM caught_pokemon WHERE run_id = $1 ORDER BY caught_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run_csv caught query failed for run {run_id}: {e}");
            vec![]
        });
    for r in &caught_rows {
        out.push_str(&format!(
            "caught,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(9)), // player_name
            csv_field(r.get::<_, String>(0)), // nickname
            csv_field(r.get::<_, String>(1)), // species_name
            r.get::<_, i32>(2),               // level
            csv_field(r.get::<_, String>(3)), // nature
            r.get::<_, bool>(4),              // is_shiny
            r.get::<_, i32>(5),               // gender
            r.get::<_, i32>(6),               // met_location
            csv_field(r.get::<_, String>(7)), // location_name
            r.get::<_, i32>(10),              // iv_hp
            r.get::<_, i32>(11),              // iv_atk
            r.get::<_, i32>(12),              // iv_def
            r.get::<_, i32>(13),              // iv_spe
            r.get::<_, i32>(14),              // iv_spa
            r.get::<_, i32>(15),              // iv_spd
            r.get::<_, i32>(16),              // ev_hp
            r.get::<_, i32>(17),              // ev_atk
            r.get::<_, i32>(18),              // ev_def
            r.get::<_, i32>(19),              // ev_spe
            r.get::<_, i32>(20),              // ev_spa
            r.get::<_, i32>(21),              // ev_spd
            csv_field(format_timestamp(r.get::<_, i64>(8) as u64)), // caught_at
        ));
    }

    out.push('\n');

    // Dead Pokémon
    out.push_str(
        "section,player_name,nickname,species_name,level,nature,is_shiny,gender,\
met_location,soul_link_death,iv_hp,iv_atk,iv_def,iv_spe,iv_spa,iv_spd,\
ev_hp,ev_atk,ev_def,ev_spe,ev_spa,ev_spd,died_at\n",
    );
    let dead_rows = client
        .query(
            "SELECT nickname, species_name, level, nature, is_shiny, gender, \
                met_location, died_at, player_name, is_soul_link_death, \
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense \
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run_csv dead query failed for run {run_id}: {e}");
            vec![]
        });
    for r in &dead_rows {
        out.push_str(&format!(
            "dead,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(8)), // player_name
            csv_field(r.get::<_, String>(0)), // nickname
            csv_field(r.get::<_, String>(1)), // species_name
            r.get::<_, i32>(2),               // level
            csv_field(r.get::<_, String>(3)), // nature
            r.get::<_, bool>(4),              // is_shiny
            r.get::<_, i32>(5),               // gender
            r.get::<_, i32>(6),               // met_location
            r.get::<_, bool>(9),              // soul_link_death
            r.get::<_, i32>(10),              // iv_hp
            r.get::<_, i32>(11),              // iv_atk
            r.get::<_, i32>(12),              // iv_def
            r.get::<_, i32>(13),              // iv_spe
            r.get::<_, i32>(14),              // iv_spa
            r.get::<_, i32>(15),              // iv_spd
            r.get::<_, i32>(16),              // ev_hp
            r.get::<_, i32>(17),              // ev_atk
            r.get::<_, i32>(18),              // ev_def
            r.get::<_, i32>(19),              // ev_spe
            r.get::<_, i32>(20),              // ev_spa
            r.get::<_, i32>(21),              // ev_spd
            csv_field(format_timestamp(r.get::<_, i64>(7) as u64)), // died_at
        ));
    }

    out.push('\n');

    // Encounters
    out.push_str("section,player_name,species_name,level,map_group,map_name,caught,is_shiny,encountered_at\n");
    let enc_rows = client
        .query(
            "SELECT species_name, level, map_group, map_name, caught, is_shiny, \
                encountered_at, player_name \
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at",
            &[&rid],
        )
        .unwrap_or_else(|e| {
            tracing::warn!("export_run_csv encounters query failed for run {run_id}: {e}");
            vec![]
        });
    for r in &enc_rows {
        out.push_str(&format!(
            "encounter,{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(7)),
            csv_field(r.get::<_, String>(0)),
            r.get::<_, i32>(1),
            r.get::<_, i32>(2),
            r.get::<_, i32>(3),
            r.get::<_, bool>(4),
            r.get::<_, bool>(5),
            csv_field(format_timestamp(r.get::<_, i64>(6) as u64)),
        ));
    }

    Ok(out)
}

/// Escapes a field value for CSV: wraps in quotes if it contains a comma, quote,
/// or newline. Interior double-quotes are doubled per RFC 4180.
fn csv_field(s: impl AsRef<str>) -> String {
    let s = s.as_ref();
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Returns a summary JSON array of every run: id, player_name, started_at,
/// ended_at, deaths, catches, and encounter count.
///
/// Opens its own connection so the live tracker is not blocked.
pub fn list_all_runs_json(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT r.id, r.player_name, r.started_at, r.ended_at,
                COUNT(DISTINCT d.personality)  AS deaths,
                COUNT(DISTINCT c.personality)  AS catches,
                COUNT(DISTINCT e.id)           AS encounters
         FROM runs r
         LEFT JOIN dead_pokemon  d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         LEFT JOIN encounters     e ON e.run_id = r.id
         GROUP BY r.id
         ORDER BY r.id DESC",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let runs: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let started: i64 = row.get(2);
            let ended: Option<i64> = row.get(3);
            serde_json::json!({
                "id":          row.get::<_, i32>(0),
                "player_name": row.get::<_, String>(1),
                "started_at":  format_timestamp(started as u64),
                "ended_at":    ended.map(|t| format_timestamp(t as u64)),
                "deaths":      row.get::<_, i64>(4),
                "catches":     row.get::<_, i64>(5),
                "encounters":  row.get::<_, i64>(6),
            })
        })
        .collect();
    serde_json::json!({ "runs": runs })
}

/// Returns all runs owned by `user_id` in the same shape as [`list_all_runs_json`].
pub fn list_runs_for_user_json(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT r.id, r.player_name, r.started_at, r.ended_at,
                COUNT(DISTINCT d.personality)  AS deaths,
                COUNT(DISTINCT c.personality)  AS catches,
                COUNT(DISTINCT e.id)           AS encounters,
                (r.user_id = $1)               AS is_owner
         FROM runs r
         LEFT JOIN dead_pokemon   d  ON d.run_id = r.id
         LEFT JOIN caught_pokemon c  ON c.run_id = r.id
         LEFT JOIN encounters     e  ON e.run_id = r.id
         WHERE r.user_id = $1
            OR EXISTS (
                SELECT 1 FROM run_invites ri
                WHERE ri.run_id = r.id
                  AND ri.invited_user = $1
                  AND ri.status = 'accepted'
            )
         GROUP BY r.id, r.user_id
         ORDER BY r.id DESC",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let runs: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let started: i64 = row.get(2);
            let ended: Option<i64> = row.get(3);
            let is_owner: bool = row.get(7);
            serde_json::json!({
                "id":          row.get::<_, i32>(0),
                "player_name": row.get::<_, String>(1),
                "started_at":  format_timestamp(started as u64),
                "ended_at":    ended.map(|t| format_timestamp(t as u64)),
                "deaths":      row.get::<_, i64>(4),
                "catches":     row.get::<_, i64>(5),
                "encounters":  row.get::<_, i64>(6),
                "is_owner":    is_owner,
            })
        })
        .collect();
    serde_json::json!({ "runs": runs })
}

/// Imports a run from the JSON format produced by [`export_run`].
///
/// Creates a new `runs` row and re-inserts every caught, dead, and encounter
/// record from the export. The original run id is **not** preserved — a new
/// id is assigned so there are no conflicts with existing data.
///
/// Original `personality` values, timestamps (`caught_at`, `died_at`,
/// `encountered_at`), `is_soul_link_death`, and the run's `started_at`/`ended_at`
/// are all preserved from the export JSON. Exports produced before these fields
/// were added fall back to safe defaults (synthetic personalities, import time).
///
/// Returns `{ "run_id": <new_id> }` on success or `{ "error": "..." }` on failure.
pub fn import_run(conn_str: &str, body: &serde_json::Value) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let run_obj = match body.get("run") {
        Some(v) => v,
        None => return serde_json::json!({ "error": "missing 'run' field" }),
    };

    let player_name = run_obj
        .get("player_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Imported");

    // Preserve original run timestamps; fall back to now if absent (old exports).
    let now = unix_now() as i64;
    let started_at: i64 = run_obj
        .get("started_at")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp)
        .map(|t| t as i64)
        .unwrap_or(now);
    let ended_at: Option<i64> = run_obj
        .get("ended_at")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp)
        .map(|t| t as i64);

    let new_id: i32 = match client.query_one(
        "INSERT INTO runs (player_name, started_at, ended_at) VALUES ($1, $2, $3) RETURNING id",
        &[&player_name, &started_at, &ended_at],
    ) {
        Ok(row) => row.get(0),
        Err(e) => return serde_json::json!({ "error": format!("Failed to create run: {e}") }),
    };

    // Re-insert encounters.
    if let Some(encounters) = body.get("encounters").and_then(|v| v.as_array()) {
        for enc in encounters {
            let species_name = enc
                .get("species_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let level = enc.get("level").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let map_group = enc.get("map_group").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let map_name = enc.get("map_name").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let caught = enc.get("caught").and_then(|v| v.as_bool()).unwrap_or(false);
            let is_shiny = enc
                .get("is_shiny")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let enc_player = enc
                .get("player_name")
                .and_then(|v| v.as_str())
                .unwrap_or(player_name);
            let encountered_at: i64 = enc
                .get("encountered_at")
                .and_then(|v| v.as_str())
                .and_then(parse_timestamp)
                .map(|t| t as i64)
                .unwrap_or(now);
            match client.execute(
                "INSERT INTO encounters (run_id, player_name, species_name, level, \
                                        map_group, map_name, caught, is_shiny, encountered_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT DO NOTHING",
                &[
                    &new_id,
                    &enc_player,
                    &species_name,
                    &level,
                    &map_group,
                    &map_name,
                    &caught,
                    &is_shiny,
                    &encountered_at,
                ],
            ) {
                Ok(0) => tracing::warn!(
                    "import_run: encounter ({species_name}, map {map_group}/{map_name}, \
                     player {enc_player}) already exists in run {new_id}; skipped"
                ),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("import_run: failed to insert encounter ({species_name}): {e}")
                }
            }
        }
    }

    // Re-insert caught.
    if let Some(caught_list) = body.get("caught").and_then(|v| v.as_array()) {
        for (idx, c) in caught_list.iter().enumerate() {
            let nickname = c.get("nickname").and_then(|v| v.as_str()).unwrap_or("");
            let species_name = c.get("species_name").and_then(|v| v.as_str()).unwrap_or("");
            let level = c.get("level").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let nature = c.get("nature").and_then(|v| v.as_str()).unwrap_or("");
            let is_shiny = c.get("is_shiny").and_then(|v| v.as_bool()).unwrap_or(false);
            let gender = c.get("gender").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let met_location = c.get("met_location").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let location_name = c
                .get("location_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let c_player = c
                .get("player_name")
                .and_then(|v| v.as_str())
                .unwrap_or(player_name);
            // Use original personality; fall back to a synthetic unique value for
            // exports produced before this field was added.
            let personality: i64 = c
                .get("personality")
                .and_then(|v| v.as_i64())
                .unwrap_or(new_id as i64 * 10_000 + idx as i64 + 1);
            let caught_at: i64 = c
                .get("caught_at")
                .and_then(|v| v.as_str())
                .and_then(parse_timestamp)
                .map(|t| t as i64)
                .unwrap_or(now);
            let iv_hp: i32 = c.get("iv_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_atk: i32 = c.get("iv_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_def: i32 = c.get("iv_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spe: i32 = c.get("iv_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spa: i32 = c.get("iv_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spd: i32 = c.get("iv_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_hp: i32 = c.get("ev_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_atk: i32 = c.get("ev_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_def: i32 = c.get("ev_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spe: i32 = c.get("ev_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spa: i32 = c.get("ev_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spd: i32 = c.get("ev_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            match client.execute(
                "INSERT INTO caught_pokemon (run_id, player_name, personality, ot_id, \
                                            nickname, species, species_name, is_shiny, \
                                            nature, level, met_location, location_name, \
                                            iv_hp, iv_attack, iv_defense, iv_speed, \
                                            iv_sp_attack, iv_sp_defense, \
                                            ev_hp, ev_attack, ev_defense, ev_speed, \
                                            ev_sp_attack, ev_sp_defense, \
                                            caught_at, gender) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12, \
                         $13,$14,$15,$16,$17,$18, $19,$20,$21,$22,$23,$24, $25,$26) \
                 ON CONFLICT (run_id, personality) DO NOTHING",
                &[
                    &new_id,
                    &c_player,
                    &personality,
                    &0i64,
                    &nickname,
                    &0i32,
                    &species_name,
                    &is_shiny,
                    &nature,
                    &level,
                    &met_location,
                    &location_name,
                    &iv_hp,
                    &iv_atk,
                    &iv_def,
                    &iv_spe,
                    &iv_spa,
                    &iv_spd,
                    &ev_hp,
                    &ev_atk,
                    &ev_def,
                    &ev_spe,
                    &ev_spa,
                    &ev_spd,
                    &caught_at,
                    &gender,
                ],
            ) {
                Ok(0) => tracing::warn!(
                    "import_run: caught personality 0x{personality:08X} ({species_name}) already \
                     exists in run {new_id}; skipped — possible duplicate import"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    "import_run: failed to insert caught personality 0x{personality:08X}: {e}"
                ),
            }
        }
    }

    // Re-insert dead.
    if let Some(dead_list) = body.get("dead").and_then(|v| v.as_array()) {
        for (idx, d) in dead_list.iter().enumerate() {
            let nickname = d.get("nickname").and_then(|v| v.as_str()).unwrap_or("");
            let species_name = d.get("species_name").and_then(|v| v.as_str()).unwrap_or("");
            let level = d.get("level").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let nature = d.get("nature").and_then(|v| v.as_str()).unwrap_or("");
            let is_shiny = d.get("is_shiny").and_then(|v| v.as_bool()).unwrap_or(false);
            let gender = d.get("gender").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let met_location = d.get("met_location").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let d_player = d
                .get("player_name")
                .and_then(|v| v.as_str())
                .unwrap_or(player_name);
            // Accept "is_soul_link_death" (current) or "soul_link" (old exports).
            let is_soul_link_death = d
                .get("is_soul_link_death")
                .or_else(|| d.get("soul_link"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let personality: i64 = d
                .get("personality")
                .and_then(|v| v.as_i64())
                .unwrap_or(new_id as i64 * 10_000 + 5_000 + idx as i64 + 1);
            let died_at: i64 = d
                .get("died_at")
                .and_then(|v| v.as_str())
                .and_then(parse_timestamp)
                .map(|t| t as i64)
                .unwrap_or(now);
            let iv_hp: i32 = d.get("iv_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_atk: i32 = d.get("iv_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_def: i32 = d.get("iv_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spe: i32 = d.get("iv_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spa: i32 = d.get("iv_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let iv_spd: i32 = d.get("iv_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_hp: i32 = d.get("ev_hp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_atk: i32 = d.get("ev_atk").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_def: i32 = d.get("ev_def").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spe: i32 = d.get("ev_spe").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spa: i32 = d.get("ev_spa").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let ev_spd: i32 = d.get("ev_spd").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            match client.execute(
                "INSERT INTO dead_pokemon (run_id, player_name, personality, ot_id, \
                                          nickname, species, species_name, is_shiny, \
                                          nature, level, met_location, died_at, \
                                          gender, max_hp, is_soul_link_death, \
                                          experience, attack, defense, speed, sp_attack, sp_defense, \
                                          move1, move2, move3, move4, pp1, pp2, pp3, pp4, \
                                          iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense, \
                                          ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense, \
                                          held_item, ability, ability_name, friendship, ot_name) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,0,$14, \
                         0,0,0,0,0,0, 0,0,0,0,0,0,0,0, \
                         $15,$16,$17,$18,$19,$20, $21,$22,$23,$24,$25,$26, 0,0,'',0,'') \
                 ON CONFLICT (run_id, personality) DO NOTHING",
                &[&new_id, &d_player, &personality, &0i64,
                  &nickname, &0i32, &species_name, &is_shiny,
                  &nature, &level, &met_location, &died_at, &gender,
                  &is_soul_link_death,
                  &iv_hp, &iv_atk, &iv_def, &iv_spe, &iv_spa, &iv_spd,
                  &ev_hp, &ev_atk, &ev_def, &ev_spe, &ev_spa, &ev_spd],
            ) {
                Ok(0) => tracing::warn!(
                    "import_run: dead personality 0x{personality:08X} ({species_name}) already \
                     exists in run {new_id}; skipped — possible duplicate import"),
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    "import_run: failed to insert dead personality 0x{personality:08X}: {e}"),
            }
        }
    }

    serde_json::json!({ "run_id": new_id })
}

/// Typed error returned by [`list_events_json`] and [`active_run_timeline_json`].
///
/// HTTP handlers match on variants to assign status codes without string-matching
/// on error text embedded in a JSON body.
#[derive(Debug)]
pub enum EventsError {
    /// No run is currently marked active in the `meta` table.
    NoActiveRun,
    /// The PostgreSQL connection could not be opened.
    ConnectionFailed(String),
    /// A database query failed.
    QueryFailed(String),
}

impl std::fmt::Display for EventsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventsError::NoActiveRun => f.write_str("no active run"),
            EventsError::ConnectionFailed(e) => write!(f, "DB connection failed: {e}"),
            EventsError::QueryFailed(e) => write!(f, "Query failed: {e}"),
        }
    }
}

/// Returns a JSON array of events for the given run ID, ordered by time.
///
/// Opens its own connection so the live tracker is not blocked.
pub fn list_events_json(conn_str: &str, run_id: u32) -> Result<serde_json::Value, EventsError> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| EventsError::ConnectionFailed(e.to_string()))?;
    let rows = client.query(
        "SELECT id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at, note
         FROM events WHERE run_id = $1 ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ).map_err(|e| EventsError::QueryFailed(e.to_string()))?;
    let events: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "id":            row.get::<_, i32>(0),
                "player_name":   row.get::<_, String>(1),
                "event_type":    row.get::<_, String>(2),
                "species_name":  row.get::<_, String>(3),
                "nickname":      row.get::<_, String>(4),
                "old_nickname":  row.get::<_, String>(5),
                "level":         row.get::<_, i32>(6),
                "occurred_at":   format_timestamp(row.get::<_, i64>(7) as u64),
                "note":          row.get::<_, String>(8),
            })
        })
        .collect();
    Ok(serde_json::json!({ "run_id": run_id, "events": events }))
}

/// Returns the chronological event timeline for the **currently active** run.
///
/// Opens its own connection and reads `active_run_id` from the `meta` table
/// directly — this avoids the global [`DB`] singleton, which is only
/// initialised in the tracker process. Calling the previous `active_run_id()`
/// helper from the aggregator process would panic immediately.
///
/// Includes both `occurred_at` as a Unix integer and a human-readable
/// `occurred_at_human` string. Returns [`EventsError`] for the typed result.
pub fn active_run_timeline_json(conn_str: &str) -> Result<serde_json::Value, EventsError> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| EventsError::ConnectionFailed(e.to_string()))?;
    let run_id: u32 = get_meta(&mut client, "active_run_id")
        .and_then(|v| v.parse().ok())
        .ok_or(EventsError::NoActiveRun)?;
    let rows = client.query(
        "SELECT id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at, note
         FROM events WHERE run_id = $1 ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ).map_err(|e| EventsError::QueryFailed(e.to_string()))?;
    let events: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let ts = row.get::<_, i64>(7) as u64;
            serde_json::json!({
                "id":                row.get::<_, i32>(0),
                "player_name":       row.get::<_, String>(1),
                "event_type":        row.get::<_, String>(2),
                "species_name":      row.get::<_, String>(3),
                "nickname":          row.get::<_, String>(4),
                "old_nickname":      row.get::<_, String>(5),
                "level":             row.get::<_, i32>(6),
                "occurred_at":       ts,
                "occurred_at_human": format_timestamp(ts),
                "note":              row.get::<_, String>(8),
            })
        })
        .collect();
    Ok(serde_json::json!({ "run_id": run_id, "events": events }))
}

/// Like [`active_run_timeline_json`] but returns `Err(NoActiveRun)` if the
/// active run is not accessible to `user_id`.
pub fn active_run_timeline_for_user_json(
    conn_str: &str,
    user_id: u32,
) -> Result<serde_json::Value, EventsError> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| EventsError::ConnectionFailed(e.to_string()))?;
    let run_id: u32 = get_meta(&mut client, "active_run_id")
        .and_then(|v| v.parse().ok())
        .ok_or(EventsError::NoActiveRun)?;
    // Check access via global DB handle.
    let accessible = user_can_access_run(run_id, user_id)
        .unwrap_or(false);
    if !accessible {
        return Err(EventsError::NoActiveRun);
    }
    let rows = client.query(
        "SELECT id, player_name, event_type, species_name, nickname, old_nickname, level, occurred_at, note
         FROM events WHERE run_id = $1 ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ).map_err(|e| EventsError::QueryFailed(e.to_string()))?;
    let events: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let ts = row.get::<_, i64>(7) as u64;
            serde_json::json!({
                "id":                row.get::<_, i32>(0),
                "player_name":       row.get::<_, String>(1),
                "event_type":        row.get::<_, String>(2),
                "species_name":      row.get::<_, String>(3),
                "nickname":          row.get::<_, String>(4),
                "old_nickname":      row.get::<_, String>(5),
                "level":             row.get::<_, i32>(6),
                "occurred_at":       ts,
                "occurred_at_human": format_timestamp(ts),
                "note":              row.get::<_, String>(8),
            })
        })
        .collect();
    Ok(serde_json::json!({ "run_id": run_id, "events": events }))
}

/// Sets (or clears) the free-text note on an event log entry identified by its
/// `event_id`. Passing an empty string effectively removes the annotation.
///
/// Returns `Ok(())` on success, `Err(message)` if the connection or query fails.
pub fn set_event_note(conn_str: &str, event_id: i32, note: &str) -> Result<(), String> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    client
        .execute(
            "UPDATE events SET note = $1 WHERE id = $2",
            &[&note, &event_id],
        )
        .map_err(|e| format!("Query failed: {e}"))?;
    Ok(())
}

/// Exports the living and fallen Pokémon for `run_id` in
/// [Pokémon Showdown Pokepaste](https://pokepast.es/) format.
///
/// Living party members (caught but not dead) appear first in a `# Living Party`
/// block. Because only the snapshot-at-catch is stored for survivors, move lines
/// are omitted. Fallen members appear in a `# Fallen` block with full moveset,
/// ability, and held-item data.
pub fn pokepaste_export(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let living = client.query(
        "SELECT nickname, species_name, is_shiny, nature, level,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                gender
         FROM caught_pokemon
         WHERE run_id = $1
           AND personality NOT IN (SELECT personality FROM dead_pokemon WHERE run_id = $1)
         ORDER BY caught_at",
        &[&rid],
    ).map_err(|e| format!("Query failed: {e}"))?;

    let dead = client.query(
        "SELECT nickname, species_name, is_shiny, nature, level,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                move1, move2, move3, move4, ability_name, held_item, gender
         FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
        &[&rid],
    ).map_err(|e| format!("Query failed: {e}"))?;

    let mut out = String::new();

    if !living.is_empty() {
        out.push_str("# Living Party\n\n");
        for row in &living {
            pokepaste_entry_no_moves(&mut out, row);
        }
    }

    if !dead.is_empty() {
        if !living.is_empty() {
            out.push('\n');
        }
        out.push_str("# Fallen\n\n");
        for row in &dead {
            pokepaste_entry_with_moves(&mut out, row);
        }
    }

    Ok(out)
}

fn pokepaste_entry_no_moves(out: &mut String, row: &postgres::Row) {
    let nickname: String     = row.get(0);
    let species: String      = row.get(1);
    let shiny: bool          = row.get(2);
    let nature: String       = row.get(3);
    let level: i32           = row.get(4);
    let iv_hp: i32           = row.get(5);
    let iv_atk: i32          = row.get(6);
    let iv_def: i32          = row.get(7);
    let iv_spe: i32          = row.get(8);
    let iv_spa: i32          = row.get(9);
    let iv_spd: i32          = row.get(10);
    let ev_hp: i32           = row.get(11);
    let ev_atk: i32          = row.get(12);
    let ev_def: i32          = row.get(13);
    let ev_spe: i32          = row.get(14);
    let ev_spa: i32          = row.get(15);
    let ev_spd: i32          = row.get(16);

    let header = if nickname == species {
        species.clone()
    } else {
        format!("{nickname} ({species})")
    };
    out.push_str(&header);
    out.push('\n');
    out.push_str(&format!("Level: {level}\n"));
    if shiny {
        out.push_str("Shiny: Yes\n");
    }
    out.push_str(&format!("{nature} Nature\n"));

    let evs = pokepaste_stat_line(ev_hp, ev_atk, ev_def, ev_spe, ev_spa, ev_spd);
    if !evs.is_empty() {
        out.push_str(&format!("EVs: {evs}\n"));
    }
    let ivs = pokepaste_iv_line(iv_hp, iv_atk, iv_def, iv_spe, iv_spa, iv_spd);
    if !ivs.is_empty() {
        out.push_str(&format!("IVs: {ivs}\n"));
    }
    out.push('\n');
}

fn pokepaste_entry_with_moves(out: &mut String, row: &postgres::Row) {
    let nickname: String     = row.get(0);
    let species: String      = row.get(1);
    let shiny: bool          = row.get(2);
    let nature: String       = row.get(3);
    let level: i32           = row.get(4);
    let iv_hp: i32           = row.get(5);
    let iv_atk: i32          = row.get(6);
    let iv_def: i32          = row.get(7);
    let iv_spe: i32          = row.get(8);
    let iv_spa: i32          = row.get(9);
    let iv_spd: i32          = row.get(10);
    let ev_hp: i32           = row.get(11);
    let ev_atk: i32          = row.get(12);
    let ev_def: i32          = row.get(13);
    let ev_spe: i32          = row.get(14);
    let ev_spa: i32          = row.get(15);
    let ev_spd: i32          = row.get(16);
    let move1: i32           = row.get(17);
    let move2: i32           = row.get(18);
    let move3: i32           = row.get(19);
    let move4: i32           = row.get(20);
    let ability: String      = row.get(21);
    let held_item: i32       = row.get(22);

    let header = if nickname == species {
        species.clone()
    } else {
        format!("{nickname} ({species})")
    };
    // Item ID 0 means "no item held".
    if held_item > 0 {
        out.push_str(&format!("{header} @ Item #{held_item}\n"));
    } else {
        out.push_str(&header);
        out.push('\n');
    }
    if !ability.is_empty() {
        out.push_str(&format!("Ability: {ability}\n"));
    }
    out.push_str(&format!("Level: {level}\n"));
    if shiny {
        out.push_str("Shiny: Yes\n");
    }
    out.push_str(&format!("{nature} Nature\n"));

    let evs = pokepaste_stat_line(ev_hp, ev_atk, ev_def, ev_spe, ev_spa, ev_spd);
    if !evs.is_empty() {
        out.push_str(&format!("EVs: {evs}\n"));
    }
    let ivs = pokepaste_iv_line(iv_hp, iv_atk, iv_def, iv_spe, iv_spa, iv_spd);
    if !ivs.is_empty() {
        out.push_str(&format!("IVs: {ivs}\n"));
    }
    for mv in [move1, move2, move3, move4] {
        if mv > 0 {
            out.push_str(&format!("- {}\n", move_name(mv as u16)));
        }
    }
    out.push('\n');
}

/// Formats non-zero EVs as a Pokepaste EV line (e.g. `"252 HP / 4 Def"`).
fn pokepaste_stat_line(hp: i32, atk: i32, def: i32, spe: i32, spa: i32, spd: i32) -> String {
    let parts: Vec<String> = [
        (hp,  "HP"),
        (atk, "Atk"),
        (def, "Def"),
        (spe, "Spe"),
        (spa, "SpA"),
        (spd, "SpD"),
    ]
    .into_iter()
    .filter(|(v, _)| *v != 0)
    .map(|(v, name)| format!("{v} {name}"))
    .collect();
    parts.join(" / ")
}

/// Formats non-31 IVs as a Pokepaste IV line.
fn pokepaste_iv_line(hp: i32, atk: i32, def: i32, spe: i32, spa: i32, spd: i32) -> String {
    let parts: Vec<String> = [
        (hp,  "HP"),
        (atk, "Atk"),
        (def, "Def"),
        (spe, "Spe"),
        (spa, "SpA"),
        (spd, "SpD"),
    ]
    .into_iter()
    .filter(|(v, _)| *v != 31)
    .map(|(v, name)| format!("{v} {name}"))
    .collect();
    parts.join(" / ")
}

// ---------------------------------------------------------------------------
// Webhook delivery log
// ---------------------------------------------------------------------------

/// Returns the `run_id` of the currently active run, or `None` if there is no
/// active run or the database has not been initialized (tracker process only).
pub fn get_active_run_id() -> Option<u32> {
    db()?.lock_or_recover().run_id
}

/// Records the final outcome of a webhook delivery attempt.
///
/// Silently no-ops when the database is not initialized (e.g. in tests or the
/// aggregator process — this function should only be called from the tracker).
pub fn record_webhook_delivery(
    run_id: Option<u32>,
    event_type: &str,
    url: &str,
    success: bool,
    attempts: u32,
    payload: &str,
) {
    let Some(db) = db() else {
        return;
    };
    let mut state = db.lock_or_recover();
    let fired_at = unix_now() as i64;
    if let Err(e) = state.client.execute(
        "INSERT INTO webhook_log (run_id, event_type, url, success, attempts, payload, fired_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &run_id.map(|id| id as i32),
            &event_type,
            &url,
            &success,
            &(attempts as i32),
            &payload,
            &fired_at,
        ],
    ) {
        tracing::warn!("Failed to record webhook delivery: {e}");
    }
}

/// Returns a JSON array of webhook delivery log entries for the given run.
///
/// Opens its own connection; intended for the aggregator's API endpoint.
pub fn get_webhook_log_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT event_type, url, success, attempts, payload, fired_at
         FROM webhook_log WHERE run_id = $1 ORDER BY fired_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "event_type": row.get::<_, String>(0),
                "url":        row.get::<_, String>(1),
                "success":    row.get::<_, bool>(2),
                "attempts":   row.get::<_, i32>(3),
                "payload":    row.get::<_, String>(4),
                "fired_at":   row.get::<_, i64>(5),
                "fired_at_human": format_timestamp(row.get::<_, i64>(5) as u64),
            })
        })
        .collect();
    serde_json::json!({ "run_id": run_id, "webhook_log": entries })
}

// ---------------------------------------------------------------------------
// Soul-link override management — connection-string variants for the aggregator
// ---------------------------------------------------------------------------

/// Returns all soul-link overrides for `run_id` as JSON.
///
/// Used by `GET /api/run/:id/soul_link/overrides`.
pub fn soul_link_overrides_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, partner_personality, created_at
         FROM soul_link_overrides WHERE run_id = $1 ORDER BY created_at",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let overrides: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "personality":         row.get::<_, i64>(0) as u64,
                "partner_personality": row.get::<_, i64>(1) as u64,
                "created_at":          row.get::<_, i64>(2),
            })
        })
        .collect();
    serde_json::json!({ "run_id": run_id, "overrides": overrides })
}

/// Upserts a soul-link override for `run_id`: `personality` ↔ `partner_personality`.
///
/// Used by `POST /api/run/:id/soul_link/override`.
pub fn set_soul_link_override_by_run(
    conn_str: &str,
    run_id: u32,
    personality: u32,
    partner_personality: u32,
) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let p = personality as i64;
    let pp = partner_personality as i64;
    let now = unix_now() as i64;
    let rid = run_id as i32;
    match client.execute(
        "INSERT INTO soul_link_overrides (run_id, personality, partner_personality, created_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (run_id, personality)
         DO UPDATE SET partner_personality = EXCLUDED.partner_personality,
                       created_at          = EXCLUDED.created_at",
        &[&rid, &p, &pp, &now],
    ) {
        Ok(_) => serde_json::json!({ "ok": true }),
        Err(e) => serde_json::json!({ "error": format!("DB error: {e}") }),
    }
}

/// Deletes the soul-link override for `personality` in `run_id`.
///
/// Used by `DELETE /api/run/:id/soul_link/override/:personality`.
pub fn clear_soul_link_override_by_run(
    conn_str: &str,
    run_id: u32,
    personality: u32,
) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let p = personality as i64;
    let rid = run_id as i32;
    match client.execute(
        "DELETE FROM soul_link_overrides WHERE run_id = $1 AND personality = $2",
        &[&rid, &p],
    ) {
        Ok(n) => serde_json::json!({ "ok": true, "deleted": n }),
        Err(e) => serde_json::json!({ "error": format!("DB error: {e}") }),
    }
}

// ---------------------------------------------------------------------------
// Route odds — unencountered areas
// ---------------------------------------------------------------------------

/// Returns encountered and unencountered wild areas for the given run as JSON.
///
/// `encountered` — routes already visited (species, level, caught flag).
/// `unencountered` — all known FireRed wild areas not yet recorded for the run.
///
/// Opens its own connection; intended for the aggregator's API endpoint.
pub fn route_odds_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    // Load all encounter rows for this run.
    let rows = match client.query(
        "SELECT player_name, map_group, map_name, species, species_name, level, caught, is_shiny, encountered_at
         FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r)  => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    // Build set of (map_group, map_name) pairs that have been encountered,
    // respecting the dungeon-floor grouping (multi-floor dungeons share a
    // Nuzlocke slot via the dungeon_floors() canonical floor list).
    use std::collections::HashSet;
    let mut seen_canonical: HashSet<(u8, u8)> = HashSet::new();
    for row in &rows {
        let mg = row.get::<_, i32>(1) as u8; // col 1 = map_group
        let mn = row.get::<_, i32>(2) as u8; // col 2 = map_name
        let floors = fire_red_location_names::dungeon_floors(mg, mn);
        if floors.is_empty() {
            seen_canonical.insert((mg, mn));
        } else {
            for &(fg, fn_) in floors {
                seen_canonical.insert((fg, fn_));
            }
        }
    }

    let encountered: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let mg = row.get::<_, i32>(1) as u8;
            let mn = row.get::<_, i32>(2) as u8;
            let raw = fire_red_location_names::map_area_name(mg, mn);
            let area = if raw.is_empty() {
                format!("{mg}:{mn}")
            } else {
                raw.to_string()
            };
            serde_json::json!({
                "player_name":    row.get::<_, String>(0),
                "map_group":      mg,
                "map_name":       mn,
                "area":           area,
                "species":        row.get::<_, i32>(3),
                "species_name":   row.get::<_, String>(4),
                "level":          row.get::<_, i32>(5),
                "caught":         row.get::<_, bool>(6),
                "is_shiny":       row.get::<_, bool>(7),
                "encountered_at": format_timestamp(row.get::<_, i64>(8) as u64),
            })
        })
        .collect();

    // Unencountered: all known wild areas minus those in seen_canonical.
    let unencountered: Vec<serde_json::Value> = fire_red_location_names::all_wild_areas()
        .iter()
        .filter(|&&(mg, mn, _)| !seen_canonical.contains(&(mg, mn)))
        .map(|&(mg, mn, area)| {
            serde_json::json!({
                "map_group": mg,
                "map_name":  mn,
                "area":      area,
            })
        })
        .collect();

    serde_json::json!({
        "run_id":        run_id,
        "encountered":   encountered,
        "unencountered": unencountered,
    })
}

// ---------------------------------------------------------------------------
// Full DB dump
// ---------------------------------------------------------------------------

/// Opens a fresh connection and returns a JSON snapshot of every table.
///
/// Intended for the `/db.json` endpoint; opens its own connection so the live
/// tracker connections are not blocked. Returns a JSON error object on failure.
pub fn dump_all(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let runs = dump_runs(&mut client);
    let caught = dump_caught(&mut client);
    let dead = dump_dead(&mut client);
    let encounters = dump_encounters(&mut client);

    serde_json::json!({ "runs": runs, "caught": caught, "dead": dead, "encounters": encounters })
}

/// Like `dump_all` but restricted to runs accessible to `user_id` (owned or accepted invite).
pub fn dump_for_user(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let ids: Vec<i32> = match client.query(
        "SELECT r.id FROM runs r
         WHERE r.user_id = $1
            OR EXISTS (
                SELECT 1 FROM run_invites ri
                WHERE ri.run_id = r.id AND ri.invited_user = $1 AND ri.status = 'accepted'
            )",
        &[&(user_id as i32)],
    ) {
        Ok(rows) => rows.iter().map(|r| r.get::<_, i32>(0)).collect(),
        Err(e) => return serde_json::json!({ "error": format!("Access query failed: {e}") }),
    };

    let runs = dump_runs_for(&mut client, &ids);
    let caught = dump_caught_for(&mut client, &ids);
    let dead = dump_dead_for(&mut client, &ids);
    let encounters = dump_encounters_for(&mut client, &ids);

    serde_json::json!({ "runs": runs, "caught": caught, "dead": dead, "encounters": encounters })
}

fn dump_runs(client: &mut Client) -> serde_json::Value {
    dump_runs_for(client, &[])
}

fn dump_runs_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("r.id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT r.id, r.player_name, r.started_at, r.ended_at,
                COUNT(DISTINCT d.personality) AS deaths,
                COUNT(DISTINCT c.personality) AS catches,
                COUNT(DISTINCT e.id) AS encounters
         FROM runs r
         LEFT JOIN dead_pokemon d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         LEFT JOIN encounters e ON e.run_id = r.id
         WHERE {filter}
         GROUP BY r.id ORDER BY r.id");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
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
            })
            .collect(),
    )
}

fn dump_caught(client: &mut Client) -> serde_json::Value {
    dump_caught_for(client, &[])
}

fn dump_caught_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("run_id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT run_id, player_name, nickname,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, nature, is_shiny,
                location_name,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                caught_at, gender
         FROM caught_pokemon WHERE {filter} ORDER BY caught_at ASC");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
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
            })
            .collect(),
    )
}

fn dump_dead(client: &mut Client) -> serde_json::Value {
    dump_dead_for(client, &[])
}

fn dump_dead_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("run_id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT run_id, player_name, nickname,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, nature, is_shiny,
                max_hp, attack, defense, speed, sp_attack, sp_defense,
                iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                is_soul_link_death,
                died_at, gender
         FROM dead_pokemon WHERE {filter} ORDER BY died_at ASC");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
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
            })
            .collect(),
    )
}

fn dump_encounters(client: &mut Client) -> serde_json::Value {
    dump_encounters_for(client, &[])
}

fn dump_encounters_for(client: &mut Client, ids: &[i32]) -> serde_json::Value {
    let filter = if ids.is_empty() { "1=1".to_string() } else { format!("run_id = ANY(ARRAY[{}]::int[])", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")) };
    let sql = format!("SELECT run_id, player_name, map_group, map_name,
                CASE WHEN species = 29 THEN 'NIDORAN♀' WHEN species = 32 THEN 'NIDORAN♂' ELSE species_name END,
                level, caught, encountered_at
         FROM encounters WHERE {filter} ORDER BY encountered_at ASC");
    let rows = client.query(&sql, &[]).unwrap_or_default();

    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
                let group = row.get::<_, i32>(2) as u8;
                let map = row.get::<_, i32>(3) as u8;
                let name = fire_red_location_names::map_area_name(group, map);
                let area = if name.is_empty() {
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
            })
            .collect(),
    )
}

/// Returns cross-run per-species statistics as JSON.
///
/// For every species that has been caught or killed across all runs, returns
/// the total caught count, total death count, and a naive survival rate.
/// Results are ordered by total deaths descending so the most dangerous species
/// appear first.
pub fn species_stats(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT species_name,
                SUM(caught_count)   AS total_caught,
                SUM(dead_count)     AS total_dead
         FROM (
             SELECT species_name, 1 AS caught_count, 0 AS dead_count FROM caught_pokemon
             UNION ALL
             SELECT species_name, 0 AS caught_count, 1 AS dead_count FROM dead_pokemon
         ) t
         GROUP BY species_name
         ORDER BY total_dead DESC, total_caught DESC",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let species: String = row.get(0);
            let total_caught: i64 = row.get(1);
            let total_dead: i64 = row.get(2);
            let survival_pct = if total_caught > 0 {
                let survived = total_caught - total_dead;
                (survived.max(0) as f64 / total_caught as f64 * 100.0).round()
            } else {
                0.0
            };
            serde_json::json!({
                "species_name":   species,
                "total_caught":   total_caught,
                "total_dead":     total_dead,
                "survival_pct":   survival_pct,
            })
        })
        .collect();

    serde_json::json!({ "species": entries })
}

/// Generate a Markdown text recap for `run_id`.
///
/// Returns `Err(message)` when the run is not found or the DB is unreachable.
/// The caller can present the error as plain text or JSON as needed.
pub fn run_summary_markdown(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client = Client::connect(conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let run_row = client
        .query_opt(
            "SELECT player_name, started_at, ended_at FROM runs WHERE id = $1",
            &[&rid],
        )
        .map_err(|e| format!("Query error: {e}"))?
        .ok_or_else(|| format!("Run {run_id} not found"))?;

    let player_name: String = run_row.get(0);
    let started_at: i64 = run_row.get(1);
    let ended_at: Option<i64> = run_row.get(2);

    let now = unix_now() as i64;
    let duration_secs = ended_at.unwrap_or(now) - started_at;
    let started_str = format_timestamp(started_at as u64);
    let ended_str = ended_at
        .map(|t| format_timestamp(t as u64))
        .unwrap_or_else(|| "in progress".to_string());
    let playtime = format!("{}h {}m", duration_secs / 3600, (duration_secs % 3600) / 60);

    // Encounters
    let enc_rows = client
        .query(
            "SELECT map_group, map_name, species_name, level, caught, is_shiny, encountered_at \
             FROM encounters WHERE run_id = $1 ORDER BY encountered_at ASC",
            &[&rid],
        )
        .unwrap_or_default();

    let total_zones = enc_rows.len();
    let total_caught_enc: usize = enc_rows.iter().filter(|r| r.get::<_, bool>(4)).count();
    let total_shinies: usize = enc_rows.iter().filter(|r| r.get::<_, bool>(5)).count();
    let catch_pct = (total_caught_enc * 100).checked_div(total_zones).unwrap_or(0) as u32;

    // Deaths
    let dead_rows = client
        .query(
            "SELECT nickname, species_name, level, died_at, is_soul_link_death \
             FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at ASC",
            &[&rid],
        )
        .unwrap_or_default();
    let total_deaths = dead_rows.len();

    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "# FireRed Nuzlocke Run #{run_id} \u{2014} {player_name}\n\n"
    ));
    out.push_str(&format!(
        "**Started:** {started_str}  **Ended:** {ended_str}  **Playtime:** {playtime}\n\n"
    ));

    // Summary table
    out.push_str("## Run Summary\n\n");
    out.push_str("| Metric | Value |\n|--------|-------|\n");
    out.push_str(&format!("| Zones visited | {total_zones} |\n"));
    out.push_str(&format!(
        "| Caught | {total_caught_enc} / {total_zones} ({catch_pct}%) |\n"
    ));
    out.push_str(&format!("| Deaths | {total_deaths} |\n"));
    out.push_str(&format!("| Shiny encounters | {total_shinies} |\n\n"));

    // Deaths section
    if total_deaths > 0 {
        out.push_str("## \u{2620} Deaths\n\n");
        out.push_str("| # | Nickname | Species | Lv. | Date | Soul Link |\n");
        out.push_str("|---|----------|---------|-----|------|-----------|\n");
        for (i, row) in dead_rows.iter().enumerate() {
            let nickname: String = row.get(0);
            let species: String = row.get(1);
            let level: i32 = row.get(2);
            let died_at: i64 = row.get(3);
            let soul_link: bool = row.get(4);
            let date = format_timestamp(died_at as u64);
            let sl_mark = if soul_link { "yes" } else { "\u{2013}" };
            out.push_str(&format!(
                "| {} | {nickname} | {species} | {level} | {date} | {sl_mark} |\n",
                i + 1
            ));
        }
        out.push('\n');
    } else {
        out.push_str("## \u{2620} Deaths\n\nNo deaths this run!\n\n");
    }

    // Encounters section
    out.push_str("## \u{1f3af} Encounters\n\n");
    if enc_rows.is_empty() {
        out.push_str("No encounters recorded.\n\n");
    } else {
        out.push_str("| # | Zone | Species | Lv. | Caught | Shiny |\n");
        out.push_str("|---|------|---------|-----|--------|-------|\n");
        for (i, row) in enc_rows.iter().enumerate() {
            let mg = row.get::<_, i32>(0) as u8;
            let mn = row.get::<_, i32>(1) as u8;
            let raw_zone = fire_red_location_names::map_area_name(mg, mn);
            let zone = if raw_zone.is_empty() {
                format!("{mg}:{mn}")
            } else {
                raw_zone.to_string()
            };
            let species: String = row.get(2);
            let level: i32 = row.get(3);
            let caught: bool = row.get(4);
            let shiny: bool = row.get(5);
            let caught_str = if caught { "\u{2713}" } else { "\u{2717}" };
            let shiny_str = if shiny { "\u{2728}" } else { "\u{2013}" };
            out.push_str(&format!(
                "| {} | {zone} | {species} | {level} | {caught_str} | {shiny_str} |\n",
                i + 1
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "*Generated by fire_red_tracker v{}*\n",
        env!("CARGO_PKG_VERSION")
    ));

    Ok(out)
}

// ---------------------------------------------------------------------------
// Presets (v17)
// ---------------------------------------------------------------------------

/// Save or replace a named party preset. `config_json` should be a JSON array
/// of `ClientMessage`-compatible command objects (the caller serialises it).
pub fn save_preset(conn_str: &str, name: &str, config_json: &str) -> Result<(), String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let now = unix_now() as i64;
    client
        .execute(
            "INSERT INTO presets (name, config, created_at) VALUES ($1, $2, $3)
             ON CONFLICT (name) DO UPDATE SET config = EXCLUDED.config, created_at = EXCLUDED.created_at",
            &[&name, &config_json, &now],
        )
        .map_err(|e| format!("Failed to save preset: {e}"))?;
    Ok(())
}

/// Return all presets as `{ "presets": [ { "name": ..., "commands": [...], "created_at": ... } ] }`.
pub fn list_presets(conn_str: &str) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query("SELECT name, config, created_at FROM presets ORDER BY name", &[]) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let presets: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let config: String = r.get(1);
            let created_at: i64 = r.get(2);
            let commands: serde_json::Value =
                serde_json::from_str(&config).unwrap_or(serde_json::Value::Array(vec![]));
            serde_json::json!({
                "name": name,
                "commands": commands,
                "created_at": format_timestamp(created_at as u64),
            })
        })
        .collect();
    serde_json::json!({ "presets": presets })
}

/// Fetch the command list for a single preset by name.
/// Returns `None` if the preset does not exist.
pub fn get_preset(conn_str: &str, name: &str) -> Option<serde_json::Value> {
    let mut client = Client::connect(conn_str, NoTls).ok()?;
    let row = client
        .query_opt("SELECT config FROM presets WHERE name = $1", &[&name])
        .ok()??;
    let config: String = row.get(0);
    serde_json::from_str(&config).ok()
}

/// Delete a preset by name. Returns `true` if a row was removed.
pub fn delete_preset(conn_str: &str, name: &str) -> bool {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .execute("DELETE FROM presets WHERE name = $1", &[&name])
        .map(|n| n > 0)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Run rules (v18)
// ---------------------------------------------------------------------------

/// Return the challenge-rule flags for a run.
/// Inserts a default all-false row on first access.
pub fn get_run_rules(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rid = run_id as i32;
    let row = client
        .query_opt(
            "SELECT duplicate_clause, species_clause, gift_clause, shiny_clause, updated_at
             FROM run_rules WHERE run_id = $1",
            &[&rid],
        )
        .unwrap_or(None);
    match row {
        Some(r) => serde_json::json!({
            "run_id": run_id,
            "duplicate_clause": r.get::<_, bool>(0),
            "species_clause":   r.get::<_, bool>(1),
            "gift_clause":      r.get::<_, bool>(2),
            "shiny_clause":     r.get::<_, bool>(3),
            "updated_at":       format_timestamp(r.get::<_, i64>(4) as u64),
        }),
        None => serde_json::json!({
            "run_id": run_id,
            "duplicate_clause": false,
            "species_clause":   false,
            "gift_clause":      false,
            "shiny_clause":     false,
            "updated_at":       null,
        }),
    }
}

/// Upsert the challenge-rule flags for a run. Only fields present in `patch`
/// are changed; others keep their current value.
pub fn set_run_rules(conn_str: &str, run_id: u32, patch: &serde_json::Value) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rid = run_id as i32;
    let now = unix_now() as i64;

    // Read existing or default.
    let existing = client
        .query_opt(
            "SELECT duplicate_clause, species_clause, gift_clause, shiny_clause
             FROM run_rules WHERE run_id = $1",
            &[&rid],
        )
        .unwrap_or(None);
    let (mut dup, mut spc, mut gift, mut shiny) = match existing {
        Some(r) => (
            r.get::<_, bool>(0),
            r.get::<_, bool>(1),
            r.get::<_, bool>(2),
            r.get::<_, bool>(3),
        ),
        None => (false, false, false, false),
    };

    if let Some(v) = patch.get("duplicate_clause").and_then(|v| v.as_bool()) { dup = v; }
    if let Some(v) = patch.get("species_clause").and_then(|v| v.as_bool())   { spc = v; }
    if let Some(v) = patch.get("gift_clause").and_then(|v| v.as_bool())      { gift = v; }
    if let Some(v) = patch.get("shiny_clause").and_then(|v| v.as_bool())     { shiny = v; }

    if let Err(e) = client.execute(
        "INSERT INTO run_rules (run_id, duplicate_clause, species_clause, gift_clause, shiny_clause, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (run_id) DO UPDATE
         SET duplicate_clause = EXCLUDED.duplicate_clause,
             species_clause   = EXCLUDED.species_clause,
             gift_clause      = EXCLUDED.gift_clause,
             shiny_clause     = EXCLUDED.shiny_clause,
             updated_at       = EXCLUDED.updated_at",
        &[&rid, &dup, &spc, &gift, &shiny, &now],
    ) {
        return serde_json::json!({ "error": format!("Failed to upsert run_rules: {e}") });
    }

    serde_json::json!({
        "run_id": run_id,
        "duplicate_clause": dup,
        "species_clause":   spc,
        "gift_clause":      gift,
        "shiny_clause":     shiny,
        "updated_at":       format_timestamp(now as u64),
    })
}

// ---------------------------------------------------------------------------
// Slot-index (display column order) for a run
// ---------------------------------------------------------------------------

/// Returns the pinned display-column index (1 = leftmost) for the given run,
/// or `None` if no preference has been set.
pub fn get_run_slot_index(conn_str: &str, run_id: u32) -> Option<u8> {
    let mut client = Client::connect(conn_str, NoTls).ok()?;
    let row = client
        .query_opt("SELECT slot_index FROM runs WHERE id = $1", &[&(run_id as i32)])
        .ok()??;
    let v: Option<i32> = row.get(0);
    v.and_then(|n| u8::try_from(n).ok())
}

/// Set (or clear) the display-column index for a run.
///
/// `owner_id` must match the run's `user_id`; returns an error string otherwise.
/// Pass `slot_index = None` to remove the preference.
pub fn set_run_slot_index(
    conn_str: &str,
    run_id: u32,
    owner_id: u32,
    slot_index: Option<u8>,
) -> Result<(), String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;

    let row = client
        .query_opt("SELECT user_id FROM runs WHERE id = $1", &[&rid])
        .map_err(|e| format!("DB query failed: {e}"))?
        .ok_or_else(|| format!("run {run_id} not found"))?;
    let stored_owner: Option<i32> = row.get(0);
    if stored_owner != Some(owner_id as i32) {
        return Err("only the run owner can set the slot index".to_string());
    }

    let val: Option<i32> = slot_index.map(|v| v as i32);
    client
        .execute("UPDATE runs SET slot_index = $1 WHERE id = $2", &[&val, &rid])
        .map_err(|e| format!("DB update failed: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-section CSV exports (v0.9.51)
// ---------------------------------------------------------------------------

/// `GET /api/run/:id/encounters.csv` — first encounters per area.
pub fn export_encounters_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;
    let mut out = String::from(
        "player_name,species_name,level,map_group,map_name,caught,is_shiny,encountered_at\n",
    );
    let rows = client
        .query(
            "SELECT player_name, species_name, level, map_group, map_name, caught, is_shiny, encountered_at
             FROM encounters WHERE run_id = $1 ORDER BY encountered_at",
            &[&rid],
        )
        .unwrap_or_default();
    for r in &rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(0)),
            csv_field(r.get::<_, String>(1)),
            r.get::<_, i32>(2),
            r.get::<_, i32>(3),
            r.get::<_, i32>(4),
            r.get::<_, bool>(5),
            r.get::<_, bool>(6),
            csv_field(format_timestamp(r.get::<_, i64>(7) as u64)),
        ));
    }
    Ok(out)
}

/// `GET /api/run/:id/deaths.csv` — deaths log.
pub fn export_deaths_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;
    let mut out = String::from(
        "player_name,nickname,species_name,level,nature,is_shiny,gender,\
met_location,soul_link_death,iv_hp,iv_atk,iv_def,iv_spe,iv_spa,iv_spd,\
ev_hp,ev_atk,ev_def,ev_spe,ev_spa,ev_spd,died_at\n",
    );
    let rows = client
        .query(
            "SELECT player_name, nickname, species_name, level, nature, is_shiny, gender,
                    met_location, is_soul_link_death,
                    iv_hp, iv_attack, iv_defense, iv_speed, iv_sp_attack, iv_sp_defense,
                    ev_hp, ev_attack, ev_defense, ev_speed, ev_sp_attack, ev_sp_defense,
                    died_at
             FROM dead_pokemon WHERE run_id = $1 ORDER BY died_at",
            &[&rid],
        )
        .unwrap_or_default();
    for r in &rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(0)),
            csv_field(r.get::<_, String>(1)),
            csv_field(r.get::<_, String>(2)),
            r.get::<_, i32>(3),
            csv_field(r.get::<_, String>(4)),
            r.get::<_, bool>(5),
            r.get::<_, i32>(6),
            r.get::<_, i32>(7),
            r.get::<_, bool>(8),
            r.get::<_, i32>(9),
            r.get::<_, i32>(10),
            r.get::<_, i32>(11),
            r.get::<_, i32>(12),
            r.get::<_, i32>(13),
            r.get::<_, i32>(14),
            r.get::<_, i32>(15),
            r.get::<_, i32>(16),
            r.get::<_, i32>(17),
            r.get::<_, i32>(18),
            r.get::<_, i32>(19),
            r.get::<_, i32>(20),
            csv_field(format_timestamp(r.get::<_, i64>(21) as u64)),
        ));
    }
    Ok(out)
}

/// `GET /api/run/:id/events.csv` — full event log.
pub fn export_events_csv(conn_str: &str, run_id: u32) -> Result<String, String> {
    let mut client =
        Client::connect(conn_str, NoTls).map_err(|e| format!("DB connection failed: {e}"))?;
    let rid = run_id as i32;
    let mut out = String::from(
        "player_name,event_type,species_name,nickname,old_nickname,level,note,occurred_at\n",
    );
    let rows = client
        .query(
            "SELECT player_name, event_type, species_name, nickname, old_nickname, level, note, occurred_at
             FROM events WHERE run_id = $1 ORDER BY occurred_at",
            &[&rid],
        )
        .unwrap_or_default();
    for r in &rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_field(r.get::<_, String>(0)),
            csv_field(r.get::<_, String>(1)),
            csv_field(r.get::<_, String>(2)),
            csv_field(r.get::<_, String>(3)),
            csv_field(r.get::<_, String>(4)),
            r.get::<_, i32>(5),
            csv_field(r.get::<_, String>(6)),
            csv_field(format_timestamp(r.get::<_, i64>(7) as u64)),
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Analytics functions (v19-v22 features)
// ---------------------------------------------------------------------------

/// `GET /api/run/:id/death_map` — deaths grouped by the area they occurred in.
///
/// Returns `[{ "area": "Route 1", "count": 3 }, ...]` sorted descending by count.
pub fn death_map(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT COALESCE(NULLIF(area_name, ''), 'Unknown') AS area, COUNT(*) AS count
         FROM dead_pokemon
         WHERE run_id = $1
         GROUP BY area
         ORDER BY count DESC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let areas: Vec<serde_json::Value> = rows.iter().map(|r| {
        serde_json::json!({ "area": r.get::<_, String>(0), "count": r.get::<_, i64>(1) })
    }).collect();
    serde_json::json!(areas)
}

/// `GET /api/run/:id/level_curve` — average party level at each badge milestone.
///
/// Returns `[{ "badge_index": 0, "badge_name": "Boulder Badge", "avg_level": 14.2,
///             "levels": [12,14,15,...], "occurred_at": 1748000000 }, ...]`.
pub fn level_curve(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT badge_index, badge_name, occurred_at, avg_level, levels
         FROM party_snapshots
         WHERE run_id = $1
         ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let snapshots: Vec<serde_json::Value> = rows.iter().map(|r| {
        let levels_str: String = r.get(4);
        let levels: serde_json::Value = serde_json::from_str(&levels_str).unwrap_or(serde_json::json!([]));
        serde_json::json!({
            "badge_index": r.get::<_, i16>(0),
            "badge_name":  r.get::<_, String>(1),
            "occurred_at": r.get::<_, i64>(2),
            "avg_level":   r.get::<_, f32>(3),
            "levels":      levels,
        })
    }).collect();
    serde_json::json!(snapshots)
}

/// `GET /api/run/:id/move_usage` — move use counts per mon per slot.
///
/// Returns `[{ "personality": 123, "move_slot": 0, "move_name": "Tackle",
///             "use_count": 14 }, ...]` ordered by use_count descending.
pub fn move_usage(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, move_slot, move_id, move_name, use_count, player_name
         FROM move_uses
         WHERE run_id = $1
         ORDER BY use_count DESC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let uses: Vec<serde_json::Value> = rows.iter().map(|r| {
        serde_json::json!({
            "personality": r.get::<_, i64>(0) as u32,
            "move_slot":   r.get::<_, i16>(1),
            "move_id":     r.get::<_, i16>(2),
            "move_name":   r.get::<_, String>(3),
            "use_count":   r.get::<_, i32>(4),
            "player_name": r.get::<_, String>(5),
        })
    }).collect();
    serde_json::json!(uses)
}

/// `GET /api/run/:id/friendship` — friendship change history per mon.
///
/// Returns grouped by personality: `[{ "personality": 123, "nickname": "Squirtle",
///   "species_name": "Squirtle", "history": [{ "friendship": 70, "logged_at": ... }] }]`.
pub fn friendship_history(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, nickname, species_name, friendship, logged_at, player_name
         FROM friendship_log
         WHERE run_id = $1
         ORDER BY logged_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    // Group by personality.
    let mut by_mon: std::collections::HashMap<u32, serde_json::Value> = std::collections::HashMap::new();
    for r in &rows {
        let personality = r.get::<_, i64>(0) as u32;
        let entry = by_mon.entry(personality).or_insert_with(|| serde_json::json!({
            "personality": personality,
            "nickname":     r.get::<_, String>(1),
            "species_name": r.get::<_, String>(2),
            "player_name":  r.get::<_, String>(5),
            "history":      serde_json::json!([]),
        }));
        entry["history"].as_array_mut().unwrap().push(serde_json::json!({
            "friendship": r.get::<_, i16>(3),
            "logged_at":  r.get::<_, i64>(4),
        }));
    }
    serde_json::json!(by_mon.into_values().collect::<Vec<_>>())
}

/// Log a party-level snapshot at a badge milestone. Uses the global DB connection.
pub fn log_party_snapshot(
    player_name: &str,
    badge_index: u8,
    badge_name: &str,
    occurred_at: u64,
    levels: &[u8],
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    if levels.is_empty() { return; }
    let avg = levels.iter().map(|&l| l as f32).sum::<f32>() / levels.len() as f32;
    let levels_json = serde_json::to_string(levels).unwrap_or_else(|_| "[]".to_string());
    if let Err(e) = state.client.execute(
        "INSERT INTO party_snapshots (run_id, player_name, badge_index, badge_name, occurred_at, avg_level, levels)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &(run_id as i32),
            &player_name,
            &(badge_index as i16),
            &badge_name,
            &(occurred_at as i64),
            &avg,
            &levels_json,
        ],
    ) {
        tracing::warn!("log_party_snapshot: {e}");
    }
}

/// Increment a move use counter for a party Pokémon. Uses the global DB connection.
pub fn log_move_use(
    player_name: &str,
    personality: u32,
    move_slot: u8,
    move_id: u16,
    move_name: &str,
    uses: i32,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let now = unix_now();
    if let Err(e) = state.client.execute(
        "INSERT INTO move_uses (run_id, player_name, personality, move_slot, move_id, move_name, use_count, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (run_id, player_name, personality, move_slot)
         DO UPDATE SET use_count = move_uses.use_count + EXCLUDED.use_count,
                       move_name = EXCLUDED.move_name,
                       updated_at = EXCLUDED.updated_at",
        &[
            &(run_id as i32),
            &player_name,
            &(personality as i64),
            &(move_slot as i16),
            &(move_id as i16),
            &move_name,
            &uses,
            &(now as i64),
        ],
    ) {
        tracing::warn!("log_move_use: {e}");
    }
}

/// Append a friendship observation for a party Pokémon. Uses the global DB connection.
pub fn log_friendship(
    player_name: &str,
    personality: u32,
    nickname: &str,
    species_name: &str,
    friendship: u8,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let now = unix_now();
    if let Err(e) = state.client.execute(
        "INSERT INTO friendship_log (run_id, player_name, personality, nickname, species_name, friendship, logged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &(run_id as i32),
            &player_name,
            &(personality as i64),
            &nickname,
            &species_name,
            &(friendship as i16),
            &(now as i64),
        ],
    ) {
        tracing::warn!("log_friendship: {e}");
    }
}

/// Log a status condition onset or clear. Uses the global DB connection.
///
/// `status_name` is a human-readable string such as `"BRN"`, `"PAR"`, `"PSN"`, etc.
/// `event_type` is either `"onset"` or `"clear"`.
pub fn log_status_event(
    player_name: &str,
    personality: u32,
    nickname: &str,
    species_name: &str,
    status_name: &str,
    status_value: u32,
    event_type: &str,
) {
    let Some(db) = db() else { return };
    let mut state = db.lock_or_recover();
    let Some(run_id) = state.effective_run_id() else { return };
    let now = unix_now();
    if let Err(e) = state.client.execute(
        "INSERT INTO status_events
             (run_id, player_name, personality, nickname, species_name, status_name, status_value, event_type, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[
            &(run_id as i32),
            &player_name,
            &(personality as i64),
            &nickname,
            &species_name,
            &status_name,
            &(status_value as i32),
            &event_type,
            &(now as i64),
        ],
    ) {
        tracing::warn!("log_status_event: {e}");
    }
}

/// Returns the full status condition log for a run.
///
/// Each entry: `{ personality, nickname, species_name, status_name, event_type, occurred_at }`.
pub fn get_status_log(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT personality, nickname, species_name, status_name, status_value, event_type, occurred_at
         FROM status_events
         WHERE run_id = $1
         ORDER BY occurred_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let at: i64 = row.get(6);
            serde_json::json!({
                "personality": row.get::<_, i64>(0) as u32,
                "nickname":    row.get::<_, String>(1),
                "species_name": row.get::<_, String>(2),
                "status_name": row.get::<_, String>(3),
                "status_value": row.get::<_, i32>(4),
                "event_type":  row.get::<_, String>(5),
                "occurred_at": at,
                "timestamp":   format_timestamp(at as u64),
            })
        })
        .collect();

    serde_json::json!({ "run_id": run_id, "status_log": entries })
}

/// Gen III FireRed move type lookup table.
///
/// Index = move ID (1–354), value = type ID using the same 0–16 encoding as
/// `fire_red_party_monitor::type_name` (0=Normal … 8=Steel, 9=Fire … 16=Dark).
/// Index 0 is unused (no move has ID 0).
#[rustfmt::skip]
static MOVE_TYPES: [u8; 355] = [
    //  0:unused
    0,
    //  1–10
    0,  1,  0,  0,  0,  0,  9, 14, 12,  0,
    //  11–20
    0,  0,  0,  0,  0,  2,  2,  0,  2,  0,
    //  21–30
    0, 11,  0,  1,  0,  1,  1,  4,  0,  0,
    //  31–40
    0,  0,  0,  0,  0,  0,  0,  0,  0,  3,
    //  41–50
    6,  6,  0, 16,  0,  0,  0,  0,  0,  0,
    //  51–60
    3,  9,  9, 14, 10, 10, 10, 14, 14, 13,
    //  61–70
    10, 14,  0,  2,  2,  1,  1,  1,  1,  0,
    //  71–80
    11, 11, 11,  0, 11, 11,  3, 11, 11, 11,
    //  81–90
    6, 15,  9, 12, 12, 12, 12,  5,  4,  4,
    //  91–100
    4,  3, 13, 13, 13, 13, 13,  0,  0, 13,
    // 101–110
    7,  0,  0,  0,  0,  0,  0,  0,  7, 10,
    // 111–120
    0, 13, 13, 14, 13,  0,  0,  0,  2,  0,
    // 121–130
    0,  7,  3,  3,  4,  9, 10, 10,  0,  0,
    // 131–140
    0,  0, 13, 13,  0,  1,  0, 13,  3,  0,
    // 141–150
    6,  0,  2,  0, 10,  0, 11,  0, 13,  0,
    // 151–160
    3, 10,  0,  0,  4, 13,  5,  0,  0,  0,
    // 161–170
    0,  0,  0,  0,  0,  0,  1, 16,  6,  0,
    // 171–180
    7,  9,  0,  7,  0,  0,  2, 11,  1,  7,
    // 181–190
    14,  0,  1,  0, 16,  0,  0,  3,  4, 10,
    // 191–200
    4, 12,  0,  7,  0, 14,  1,  4,  0, 15,
    // 201–210
    5, 11,  0,  0,  5,  0,  0,  0, 12,  6,
    // 211–220
    8,  0,  0,  0,  0,  0,  0,  0,  0,  0,
    // 221–230
    9,  4,  1,  6, 15,  0,  0, 16,  0,  0,
    // 231–240
    8,  8,  1,  0, 11,  0,  0,  1, 15, 10,
    // 241–250
    9, 16, 13,  0,  0,  5,  7, 13,  1, 10,
    // 251–260
    16,  0,  0,  0,  0,  0,  9, 14, 16, 16,
    // 261–270
    9, 16,  0,  1,  0,  0,  0, 12, 16,  0,
    // 271–280
    13, 13,  0,  0, 11,  1, 13,  0,  1,  1,
    // 281–290
    0, 16,  0,  9, 13, 13,  0,  7, 16,  0,
    // 291–300
    10,  1,  0,  6, 13, 13,  2,  0,  9,  4,
    // 301–310
    14, 11,  0,  0,  3,  0,  9, 10,  8,  7,
    // 311–320
    0, 11, 16,  2,  9,  0,  5,  6,  8, 11,
    // 321–330
    0, 13, 10,  6,  7, 13,  1,  4, 14, 10,
    // 331–340
    11,  2, 14,  8,  0,  0, 15, 11,  1,  2,
    // 341–350
    4,  3,  0, 12, 11, 10, 13, 11, 15,  5,
    // 351–354
    12, 10,  8, 13,
];

fn move_type_for_id(move_id: u16) -> u8 {
    MOVE_TYPES.get(move_id as usize).copied().unwrap_or(0)
}

/// Returns a type-usage breakdown derived from recorded move uses.
///
/// Aggregates `move_uses` by move ID → attacking type using a static Gen III
/// move-type table, returning sorted totals per type.
/// Type IDs follow Gen III encoding (Normal=0 … Dark=16).
pub fn type_matchup_heatmap(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let rows = match client.query(
        "SELECT move_id, SUM(use_count)::bigint AS total_uses
         FROM move_uses
         WHERE run_id = $1 AND move_id > 0
         GROUP BY move_id",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };

    let mut type_uses: std::collections::HashMap<u8, i64> = std::collections::HashMap::new();
    for row in &rows {
        let move_id: i16 = row.get(0);
        let uses: i64 = row.get(1);
        let atk_type = move_type_for_id(move_id as u16);
        *type_uses.entry(atk_type).or_insert(0) += uses;
    }

    const TYPE_NAMES: [&str; 17] = [
        "Normal","Fighting","Flying","Poison","Ground","Rock","Bug","Ghost",
        "Steel","Fire","Water","Grass","Electric","Psychic","Ice","Dragon","Dark",
    ];

    let entries: Vec<serde_json::Value> = type_uses
        .into_iter()
        .map(|(type_id, uses)| {
            let name = TYPE_NAMES.get(type_id as usize).copied().unwrap_or("???");
            serde_json::json!({
                "type_id":    type_id,
                "type_name":  name,
                "total_uses": uses,
            })
        })
        .collect();

    let mut sorted = entries;
    sorted.sort_by(|a, b| b["total_uses"].as_i64().cmp(&a["total_uses"].as_i64()));

    serde_json::json!({ "run_id": run_id, "type_usage": sorted })
}

/// Ghost-run milestone comparison.
///
/// Returns a side-by-side diff of the current run vs a ghost run, aligned on
/// badge milestones. For each badge milestone (0–7) present in either run,
/// returns the elapsed time, deaths, and average party level at that point.
pub fn ghost_run_comparison(conn_str: &str, run_id: u32, ghost_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    let snapshots_for = |c: &mut Client, rid: u32| -> Vec<(i16, String, i64, f32)> {
        c.query(
            "SELECT ps.badge_index, ps.badge_name, ps.occurred_at, ps.avg_level
             FROM party_snapshots ps
             WHERE ps.run_id = $1
             ORDER BY ps.badge_index ASC",
            &[&(rid as i32)],
        )
        .unwrap_or_default()
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect()
    };

    let deaths_before = |c: &mut Client, rid: u32, ts: i64| -> i64 {
        c.query_one(
            "SELECT COUNT(*) FROM dead_pokemon WHERE run_id = $1 AND died_at <= $2",
            &[&(rid as i32), &ts],
        )
        .map(|r| r.get::<_, i64>(0))
        .unwrap_or(0)
    };

    let started_at = |c: &mut Client, rid: u32| -> i64 {
        c.query_one("SELECT started_at FROM runs WHERE id = $1", &[&(rid as i32)])
            .map(|r| r.get::<_, i64>(0))
            .unwrap_or(0)
    };

    let run_start   = started_at(&mut client, run_id);
    let ghost_start = started_at(&mut client, ghost_id);
    let run_snaps   = snapshots_for(&mut client, run_id);
    let ghost_snaps = snapshots_for(&mut client, ghost_id);

    let all_badges: std::collections::BTreeSet<i16> = run_snaps
        .iter()
        .chain(ghost_snaps.iter())
        .map(|(i, _, _, _)| *i)
        .collect();

    let milestones: Vec<serde_json::Value> = all_badges
        .iter()
        .map(|&badge_idx| {
            let run_entry   = run_snaps.iter().find(|(i, _, _, _)| *i == badge_idx);
            let ghost_entry = ghost_snaps.iter().find(|(i, _, _, _)| *i == badge_idx);

            let badge_name = run_entry
                .or(ghost_entry)
                .map(|(_, n, _, _)| n.as_str())
                .unwrap_or("Badge");

            let make_side = |entry: Option<&(i16, String, i64, f32)>, run_start: i64, c: &mut Client, rid: u32| {
                entry.map(|(_, _, at, avg_lv)| {
                    let elapsed = (at - run_start).max(0) as u64;
                    let deaths  = deaths_before(c, rid, *at);
                    serde_json::json!({
                        "elapsed_secs": elapsed,
                        "elapsed_human": format!("{}h {:02}m {:02}s", elapsed / 3600, (elapsed % 3600) / 60, elapsed % 60),
                        "deaths":        deaths,
                        "avg_level":     avg_lv,
                    })
                })
            };

            let current = make_side(run_entry,   run_start,   &mut client, run_id);
            let ghost   = make_side(ghost_entry, ghost_start, &mut client, ghost_id);

            serde_json::json!({
                "badge_index": badge_idx,
                "badge_name":  badge_name,
                "current":     current,
                "ghost":       ghost,
            })
        })
        .collect();

    serde_json::json!({
        "run_id":   run_id,
        "ghost_id": ghost_id,
        "milestones": milestones,
    })
}

/// Cumulative shiny encounter probability for a run.
///
/// Returns the number of encounters logged and the cumulative probability of
/// having seen at least one shiny (P = 1 − (1 − 1/8192)^n).
pub fn shiny_pressure(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let row = match client.query_one(
        "SELECT COUNT(*), SUM(CASE WHEN is_shiny THEN 1 ELSE 0 END)::bigint
         FROM encounters WHERE run_id = $1",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let total: i64 = row.get(0);
    let shinies: i64 = row.get(1);
    // P(≥1 shiny in n encounters) = 1 - (1 - 1/8192)^n
    let prob_at_least_one = if total > 0 {
        1.0 - (1.0f64 - 1.0 / 8192.0).powi(total as i32)
    } else {
        0.0
    };
    let expected_at = if shinies == 0 { 8192i64 } else { total / shinies };
    serde_json::json!({
        "run_id":           run_id,
        "total_encounters": total,
        "shiny_count":      shinies,
        "probability_pct":  (prob_at_least_one * 10000.0).round() / 100.0,
        "expected_at":      expected_at,
        "unlucky":          shinies == 0 && total >= 8192,
    })
}

/// Pokédex completion count for a run.
///
/// Returns the number of unique species caught (`caught = true`) across all
/// encounters for the run, plus a list of species IDs / names.
pub fn dex_count(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT DISTINCT species, species_name
         FROM encounters
         WHERE run_id = $1 AND caught = TRUE
         ORDER BY species ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query error: {e}") }),
    };
    let caught: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| serde_json::json!({
            "species": row.get::<_, i32>(0),
            "species_name": row.get::<_, String>(1),
        }))
        .collect();
    serde_json::json!({
        "run_id":  run_id,
        "count":   caught.len(),
        "species": caught,
    })
}

/// Create a time-limited read-only share token for a run.
///
/// Stores the token in the `meta` table under key `share:<token>` with value
/// `<run_id>:<expires_at_unix>`. Returns the token string.
pub fn create_share_token(run_id: u32, ttl_secs: u64) -> Option<String> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    // Generate a 32-byte random token encoded as hex.
    // Use the system time + run_id + a counter as entropy (no rand crate needed).
    let now = unix_now();
    let expires = now + ttl_secs;
    let raw = format!("{run_id}-{now}-{expires}");
    let hash = Sha256::digest(raw.as_bytes());
    let token: String = hash.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    });
    let key = format!("share:{token}");
    let value = format!("{run_id}:{expires}");
    if let Err(e) = state.client.execute(
        "INSERT INTO meta (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        &[&key, &value],
    ) {
        tracing::warn!("create_share_token: {e}");
        return None;
    }
    Some(token)
}

/// Resolve a share token to a run ID, returning None if expired or not found.
pub fn resolve_share_token(conn_str: &str, token: &str) -> Option<u32> {
    let mut client = Client::connect(conn_str, NoTls).ok()?;
    let key = format!("share:{token}");
    let row = client
        .query_opt("SELECT value FROM meta WHERE key = $1", &[&key])
        .ok()??;
    let value: String = row.get(0);
    let mut parts = value.splitn(2, ':');
    let run_id: u32 = parts.next()?.parse().ok()?;
    let expires: u64 = parts.next()?.parse().ok()?;
    if unix_now() > expires {
        // Token expired — clean up silently.
        let _ = client.execute("DELETE FROM meta WHERE key = $1", &[&key]);
        return None;
    }
    Some(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── nature_name ───────────────────────────────────────────────────────────

    #[test]
    fn nature_name_first_and_last() {
        assert_eq!(nature_name(0), "Hardy");
        assert_eq!(nature_name(24), "Quirky");
    }

    #[test]
    fn nature_name_wraps_at_25() {
        assert_eq!(nature_name(25), "Hardy");
        assert_eq!(nature_name(26), "Lonely");
    }

    #[test]
    fn nature_name_known_values() {
        assert_eq!(nature_name(1), "Lonely");
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

    // ── csv_field ─────────────────────────────────────────────────────────────

    #[test]
    fn csv_field_plain_string_unchanged() {
        assert_eq!(csv_field("Bulbasaur"), "Bulbasaur");
    }

    #[test]
    fn csv_field_string_with_comma_is_quoted() {
        assert_eq!(csv_field("Route 1, East"), "\"Route 1, East\"");
    }

    #[test]
    fn csv_field_string_with_quote_is_escaped() {
        assert_eq!(csv_field("say \"hello\""), "\"say \"\"hello\"\"\"");
    }

    #[test]
    fn csv_field_empty_string_unchanged() {
        assert_eq!(csv_field(""), "");
    }

    // ── EventKind dispatch ────────────────────────────────────────────────────

    fn event_type_str(event: &EventKind<'_>) -> &'static str {
        event.row_parts().0
    }

    #[test]
    fn event_kind_catch_maps_to_correct_type() {
        let e = EventKind::Catch {
            species_name: "BULBASAUR",
            nickname: "Bulby",
            level: 5,
        };
        assert_eq!(event_type_str(&e), "catch");
    }

    #[test]
    fn event_kind_death_maps_to_correct_type() {
        let e = EventKind::Death {
            species_name: "CHARMANDER",
            nickname: "Ember",
            level: 10,
        };
        assert_eq!(event_type_str(&e), "death");
    }

    #[test]
    fn event_kind_soul_link_death_maps_to_correct_type() {
        let e = EventKind::SoulLinkDeath {
            species_name: "SQUIRTLE",
            nickname: "Shell",
            level: 8,
        };
        assert_eq!(event_type_str(&e), "soul_link_death");
    }

    #[test]
    fn event_kind_shiny_maps_to_correct_type() {
        let e = EventKind::Shiny {
            species_name: "MEWTWO",
            level: 70,
        };
        assert_eq!(event_type_str(&e), "shiny");
    }

    #[test]
    fn event_kind_wipe_maps_to_correct_type() {
        assert_eq!(event_type_str(&EventKind::Wipe), "wipe");
    }

    #[test]
    fn event_kind_badge_maps_to_correct_type() {
        let e = EventKind::Badge {
            badge_name: "Boulder Badge",
        };
        assert_eq!(event_type_str(&e), "badge");
    }

    #[test]
    fn event_kind_badge_carries_name_in_species_field() {
        let e = EventKind::Badge {
            badge_name: "Boulder Badge",
        };
        let (event_type, species_name, nickname, old_nickname, level) = e.row_parts();
        assert_eq!(event_type, "badge");
        assert_eq!(species_name, "Boulder Badge");
        assert_eq!(nickname, "");
        assert_eq!(old_nickname, "");
        assert_eq!(level, 0);
    }

    #[test]
    fn event_kind_nickname_change_maps_to_correct_type() {
        let e = EventKind::NicknameChange {
            species_name: "EEVEE",
            old_name: "Eevee",
            new_name: "Sylvi",
        };
        assert_eq!(event_type_str(&e), "nickname_change");
    }

    #[test]
    fn event_kind_nickname_change_carries_names() {
        let e = EventKind::NicknameChange {
            species_name: "EEVEE",
            old_name: "Eevee",
            new_name: "Sylvi",
        };
        let (event_type, species_name, nickname, old_nickname, level) = e.row_parts();
        assert_eq!(event_type, "nickname_change");
        assert_eq!(species_name, "EEVEE");
        assert_eq!(nickname, "Sylvi"); // new_name → nickname column
        assert_eq!(old_nickname, "Eevee"); // old_name → old_nickname column
        assert_eq!(level, 0);
    }

    // ── format_timestamp / parse_timestamp round-trip ────────────────────────

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

    #[test]
    fn parse_timestamp_roundtrips_epoch() {
        assert_eq!(parse_timestamp("1970-01-01 00:00:00 UTC"), Some(0));
    }

    #[test]
    fn parse_timestamp_roundtrips_arbitrary() {
        for secs in [1, 86400, 90061, 1704067200u64, 1709164800] {
            let formatted = format_timestamp(secs);
            assert_eq!(
                parse_timestamp(&formatted),
                Some(secs),
                "round-trip failed for secs={secs} (formatted={formatted})"
            );
        }
    }

    #[test]
    fn parse_timestamp_returns_none_on_garbage() {
        assert_eq!(parse_timestamp("not a date"), None);
        assert_eq!(parse_timestamp(""), None);
    }

    #[test]
    fn parse_timestamp_rejects_out_of_range_fields() {
        assert_eq!(parse_timestamp("2025-00-01 00:00:00 UTC"), None); // month 0
        assert_eq!(parse_timestamp("2025-13-01 00:00:00 UTC"), None); // month 13
        assert_eq!(parse_timestamp("2025-06-00 00:00:00 UTC"), None); // day 0
        assert_eq!(parse_timestamp("2025-06-01 24:00:00 UTC"), None); // hour 24
        assert_eq!(parse_timestamp("2025-06-01 00:60:00 UTC"), None); // min 60
        assert_eq!(parse_timestamp("2025-06-01 00:00:60 UTC"), None); // sec 60
    }

    #[test]
    fn parse_timestamp_rejects_day_exceeds_month_length() {
        assert_eq!(parse_timestamp("2025-02-29 00:00:00 UTC"), None); // not a leap year
        assert!(parse_timestamp("2000-02-29 00:00:00 UTC").is_some()); // IS a leap year
        assert_eq!(parse_timestamp("2025-04-31 00:00:00 UTC"), None); // April has 30 days
        assert_eq!(parse_timestamp("2025-01-32 00:00:00 UTC"), None); // 32nd of any month
    }
}

// ---------------------------------------------------------------------------
// Public API — user accounts and sessions
// ---------------------------------------------------------------------------

const SESSION_TTL_SECS: u64 = 86_400 * 30; // 30 days

/// Register a new user. Returns `Err` if the username is already taken or the
/// password cannot be hashed.
///
/// The password is hashed with bcrypt (cost 12) before storage; the plaintext
/// is never written to the database.
pub fn create_user(username: &str, password: &str) -> Result<User, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("username must not be empty".to_string());
    }
    if password.len() < 8 {
        return Err("password must be at least 8 characters".to_string());
    }

    let hash = bcrypt::hash(password, 12)
        .map_err(|e| format!("Failed to hash password: {e}"))?;

    let Some(db) = db() else {
        return Err("database not initialised".to_string());
    };
    let mut state = db.lock_or_recover();

    let row = state.client.query_opt(
        "INSERT INTO users (username, password_hash, created_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (username) DO NOTHING
         RETURNING id, created_at",
        &[&username, &hash, &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;

    let row = row.ok_or_else(|| format!("username '{}' is already taken", username))?;
    Ok(User {
        id: row.get::<_, i32>(0) as u32,
        username: username.to_string(),
        created_at: row.get::<_, i64>(1) as u64,
    })
}

/// Look up a user by username and verify their password.
///
/// Returns `Ok(Some(user))` on success, `Ok(None)` when credentials are wrong
/// or the user doesn't exist.
pub fn authenticate_user(username: &str, password: &str) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();

    let row = match state.client.query_opt(
        "SELECT id, username, password_hash, created_at FROM users WHERE username = $1",
        &[&username],
    ) {
        Ok(r) => r,
        Err(e) => return Err(format!("DB error: {e}")),
    };

    let Some(row) = row else {
        tracing::warn!(username = %username, reason = "no such user", "login failed");
        return Ok(None);
    };
    let hash: String = row.get(2);

    let ok = bcrypt::verify(password, &hash)
        .map_err(|e| format!("bcrypt error: {e}"))?;

    if ok {
        tracing::info!(username = %username, "login succeeded");
        Ok(Some(User {
            id: row.get::<_, i32>(0) as u32,
            username: row.get(1),
            created_at: row.get::<_, i64>(3) as u64,
        }))
    } else {
        tracing::warn!(username = %username, reason = "wrong password", "login failed");
        Ok(None)
    }
}

/// Fetch a user by ID. Returns `Ok(None)` if the ID doesn't exist.
pub fn get_user_by_id(id: u32) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT id, username, created_at FROM users WHERE id = $1",
        &[&(id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(row.map(|r| User {
        id: r.get::<_, i32>(0) as u32,
        username: r.get(1),
        created_at: r.get::<_, i64>(2) as u64,
    }))
}

/// Return all registered users, ordered by ID.
pub fn list_users() -> Result<Vec<User>, String> {
    let Some(db) = db() else { return Ok(vec![]) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT id, username, created_at FROM users ORDER BY id",
        &[],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(rows.iter().map(|r| User {
        id: r.get::<_, i32>(0) as u32,
        username: r.get(1),
        created_at: r.get::<_, i64>(2) as u64,
    }).collect())
}

/// Create a session token for a user. Returns the opaque bearer token.
///
/// Tokens expire after 30 days. Old expired sessions are pruned on each call.
pub fn create_session(user_id: u32) -> Result<String, String> {
    let Some(db) = db() else {
        return Err("database not initialised".to_string());
    };
    let mut state = db.lock_or_recover();

    // Prune expired sessions to keep the table tidy.
    let _ = state.client.execute(
        "DELETE FROM sessions WHERE expires_at < $1",
        &[&(unix_now() as i64)],
    );

    // Generate a token: SHA-256 of (user_id || 32 CSPRNG bytes).
    let mut rng_bytes = [0u8; 32];
    {
        use rand::RngCore;
        rand::rng().fill_bytes(&mut rng_bytes);
    }
    let mut hasher = Sha256::new();
    hasher.update(user_id.to_le_bytes());
    hasher.update(rng_bytes);
    let digest = hasher.finalize();
    let token: String = digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
        s
    });

    let now = unix_now() as i64;
    let expires = now + SESSION_TTL_SECS as i64;
    state.client.execute(
        "INSERT INTO sessions (token, user_id, created_at, expires_at) VALUES ($1, $2, $3, $4)",
        &[&token, &(user_id as i32), &now, &expires],
    ).map_err(|e| format!("DB error: {e}"))?;

    Ok(token)
}

/// Validate a session token. Returns the associated `User` if the token is
/// valid and not expired; `Ok(None)` otherwise.
pub fn validate_session(token: &str) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT u.id, u.username, u.created_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token = $1 AND s.expires_at > $2",
        &[&token, &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(row.map(|r| User {
        id: r.get::<_, i32>(0) as u32,
        username: r.get(1),
        created_at: r.get::<_, i64>(2) as u64,
    }))
}

/// Revoke a session token (logout).
pub fn delete_session(token: &str) -> Result<(), String> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    state.client.execute("DELETE FROM sessions WHERE token = $1", &[&token])
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Return all run IDs accessible to `user_id` — runs they own plus runs they
/// have an accepted invite for.  Used to filter live slot data per-user.
pub fn get_accessible_run_ids(user_id: u32) -> Result<std::collections::HashSet<u32>, String> {
    let Some(db) = db() else { return Ok(std::collections::HashSet::new()) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT id FROM runs WHERE user_id = $1
         UNION
         SELECT run_id FROM run_invites WHERE invited_user = $1 AND status = 'accepted'",
        &[&(user_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(rows.iter().map(|r| r.get::<_, i32>(0) as u32).collect())
}

// ---------------------------------------------------------------------------
// Per-user integration configs
// ---------------------------------------------------------------------------

/// Return the stored JSON config for `(user_id, kind)`, or `None` if not set.
pub fn get_user_integration(conn_str: &str, user_id: u32, kind: &str) -> Option<String> {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return None };
    let row = client
        .query_opt(
            "SELECT config FROM user_integrations WHERE user_id = $1 AND kind = $2",
            &[&(user_id as i32), &kind],
        )
        .ok()??;
    Some(row.get(0))
}

/// Upsert a JSON config for `(user_id, kind)`. `config` is a JSON string.
pub fn set_user_integration(conn_str: &str, user_id: u32, kind: &str, config: &str) -> Result<(), String> {
    let conn_str = normalize_conn_str(conn_str);
    let mut client = Client::connect(&conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    let now = unix_now() as i64;
    client
        .execute(
            "INSERT INTO user_integrations (user_id, kind, config, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, kind) DO UPDATE
             SET config = EXCLUDED.config, updated_at = EXCLUDED.updated_at",
            &[&(user_id as i32), &kind, &config, &now],
        )
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Delete the integration config for `(user_id, kind)`. Returns `true` if a row was deleted.
pub fn delete_user_integration(conn_str: &str, user_id: u32, kind: &str) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute(
            "DELETE FROM user_integrations WHERE user_id = $1 AND kind = $2",
            &[&(user_id as i32), &kind],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Return all integration configs for a user as `{ kind: config_json_string }`.
pub fn list_user_integrations(conn_str: &str, user_id: u32) -> serde_json::Value {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else {
        return serde_json::json!({});
    };
    let rows = match client.query(
        "SELECT kind, config FROM user_integrations WHERE user_id = $1 ORDER BY kind",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(_) => return serde_json::json!({}),
    };
    let mut map = serde_json::Map::new();
    for row in &rows {
        let kind: String = row.get(0);
        let config_str: String = row.get(1);
        let val: serde_json::Value =
            serde_json::from_str(&config_str).unwrap_or(serde_json::Value::Null);
        map.insert(kind, val);
    }
    serde_json::Value::Object(map)
}

/// Associate an existing run with a user account.
pub fn link_run_to_user(run_id: u32, user_id: u32) -> Result<(), String> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    state.client.execute(
        "UPDATE runs SET user_id = $1 WHERE id = $2 AND user_id IS NULL",
        &[&(user_id as i32), &(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Create a new run for a direct-mode slot **without** touching the global
/// `DbState.run_id`.  The caller must then call [`set_thread_run_id`] on the
/// game-loop thread so DB writes from that thread go to this run.
pub fn create_run_for_slot(player_name: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Err("No database configured".into()) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_one(
        "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
        &[&player_name, &(unix_now() as i64)],
    ).map_err(|e| format!("Failed to create run: {e}"))?;
    Ok(row.get::<_, i32>(0) as u32)
}

/// Delete a run row created by this process (used to clean up orphan rows when
/// direct-mode setup fails after the run was already inserted).
pub fn delete_run(run_id: u32) -> Result<(), String> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    state.client.execute(
        "DELETE FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Verify that a run with the given ID exists (for resuming in direct mode).
///
/// Returns `Ok(true)` if found, `Ok(false)` if not, `Err` on DB error.
/// Does **not** set the global active run — the caller should call
/// [`set_thread_run_id`] on the game-loop thread afterward.
pub fn run_exists(run_id: u32) -> Result<bool, String> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT 1 FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(!rows.is_empty())
}

/// Look up a user by username. Returns `None` if no account with that name exists.
pub fn get_user_by_username(username: &str) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT id, username, created_at FROM users WHERE username = $1",
        &[&username],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(rows.first().map(|row| User {
        id: row.get::<_, i32>(0) as u32,
        username: row.get(1),
        created_at: row.get::<_, i64>(2) as u64,
    }))
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

/// Returns a dashboard JSON blob for the given user:
/// - `user`: `{ id, username }`
/// - `open_runs`: runs owned or accepted-invite runs that have no `ended_at`
/// - `stats`: totals across all accessible runs `{ deaths, catches, encounters, runs }`
/// - `recent_party`: alive caught_pokemon from the most recent open run (up to 6)
/// - `pending_invites`: invites waiting for this user's response
pub fn user_dashboard_json(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    // User info
    let user_row = match client.query_opt(
        "SELECT id, username FROM users WHERE id = $1",
        &[&(user_id as i32)],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return serde_json::json!({ "error": "user not found" }),
        Err(e) => return serde_json::json!({ "error": format!("DB error: {e}") }),
    };
    let username: String = user_row.get(1);

    // Open runs (owned or accepted invite)
    let open_rows = match client.query(
        "SELECT r.id, r.player_name, r.started_at,
                COUNT(DISTINCT d.personality) AS deaths,
                COUNT(DISTINCT c.personality) AS catches,
                (r.user_id = $1)              AS is_owner
         FROM runs r
         LEFT JOIN dead_pokemon   d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         WHERE r.ended_at IS NULL
           AND (
               r.user_id = $1
               OR EXISTS (
                   SELECT 1 FROM run_invites ri
                   WHERE ri.run_id = r.id AND ri.invited_user = $1 AND ri.status = 'accepted'
               )
           )
         GROUP BY r.id, r.user_id
         ORDER BY (r.user_id = $1) DESC, r.id DESC",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let open_runs: Vec<serde_json::Value> = open_rows.iter().map(|row| {
        let started: i64 = row.get(2);
        serde_json::json!({
            "id":          row.get::<_, i32>(0),
            "player_name": row.get::<_, String>(1),
            "started_at":  format_timestamp(started as u64),
            "deaths":      row.get::<_, i64>(3),
            "catches":     row.get::<_, i64>(4),
            "is_owner":    row.get::<_, bool>(5),
        })
    }).collect();

    // Aggregate stats across all accessible runs (open and closed)
    let stats_row = match client.query_opt(
        "SELECT COUNT(DISTINCT r.id)           AS run_count,
                COUNT(DISTINCT d.personality)  AS deaths,
                COUNT(DISTINCT c.personality)  AS catches,
                COUNT(DISTINCT e.id)           AS encounters
         FROM runs r
         LEFT JOIN dead_pokemon   d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         LEFT JOIN encounters     e ON e.run_id = r.id
         WHERE r.user_id = $1
            OR EXISTS (
                SELECT 1 FROM run_invites ri
                WHERE ri.run_id = r.id AND ri.invited_user = $1 AND ri.status = 'accepted'
            )",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Stats query failed: {e}") }),
    };
    let stats = stats_row.map(|row| serde_json::json!({
        "runs":       row.get::<_, i64>(0),
        "deaths":     row.get::<_, i64>(1),
        "catches":    row.get::<_, i64>(2),
        "encounters": row.get::<_, i64>(3),
    })).unwrap_or_else(|| serde_json::json!({ "runs": 0, "deaths": 0, "catches": 0, "encounters": 0 }));

    // Most recent party: alive caught pokemon from most recent open run
    let recent_party: Vec<serde_json::Value> = if let Some(first) = open_rows.first() {
        let run_id: i32 = first.get(0);
        match client.query(
            "SELECT cp.nickname, cp.species_name, cp.level, cp.is_shiny
             FROM caught_pokemon cp
             WHERE cp.run_id = $1
               AND NOT EXISTS (
                   SELECT 1 FROM dead_pokemon dp
                   WHERE dp.run_id = cp.run_id AND dp.personality = cp.personality
               )
             ORDER BY cp.caught_at ASC
             LIMIT 6",
            &[&run_id],
        ) {
            Ok(rows) => rows.iter().map(|row| serde_json::json!({
                "nickname":     row.get::<_, String>(0),
                "species_name": row.get::<_, String>(1),
                "level":        row.get::<_, i32>(2),
                "is_shiny":     row.get::<_, bool>(3),
            })).collect(),
            Err(_) => vec![],
        }
    } else {
        vec![]
    };

    // Pending invites for this user
    let invite_rows = match client.query(
        "SELECT ri.id, r.id AS run_id, r.player_name, u.username AS invited_by, ri.created_at
         FROM run_invites ri
         JOIN runs  r ON r.id  = ri.run_id
         JOIN users u ON u.id  = ri.invited_by
         WHERE ri.invited_user = $1 AND ri.status = 'pending' AND ri.is_request = FALSE
         ORDER BY ri.created_at DESC",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Invite query failed: {e}") }),
    };
    let pending_invites: Vec<serde_json::Value> = invite_rows.iter().map(|row| {
        let created: i64 = row.get(4);
        serde_json::json!({
            "invite_id":   row.get::<_, i32>(0),
            "run_id":      row.get::<_, i32>(1),
            "player_name": row.get::<_, String>(2),
            "invited_by":  row.get::<_, String>(3),
            "created_at":  format_timestamp(created as u64),
        })
    }).collect();

    serde_json::json!({
        "user": { "id": user_id, "username": username },
        "open_runs": open_runs,
        "stats": stats,
        "recent_party": recent_party,
        "pending_invites": pending_invites,
    })
}

// ---------------------------------------------------------------------------
// Run invites
// ---------------------------------------------------------------------------

/// Invite another user (by username) to collaborate on a run.
///
/// Only the run owner (`runs.user_id`) may invite others.
/// Returns `Ok(invite_id)` on success or `Err(message)` on failure.
pub fn invite_user_to_run(run_id: u32, inviter_user_id: u32, invitee_username: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();

    // Verify caller owns the run.
    let row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    let owner_id: Option<i32> = row.as_ref().and_then(|r| r.get(0));
    if owner_id != Some(inviter_user_id as i32) {
        return Err("you do not own this run".to_string());
    }

    // Resolve invitee.
    let inv_row = state.client.query_opt(
        "SELECT id FROM users WHERE username = $1",
        &[&invitee_username],
    ).map_err(|e| format!("DB error: {e}"))?;
    let invitee_id: i32 = inv_row
        .ok_or_else(|| format!("user '{}' not found", invitee_username))?
        .get(0);

    if invitee_id == inviter_user_id as i32 {
        return Err("cannot invite yourself".to_string());
    }

    let result = state.client.query_one(
        "INSERT INTO run_invites (run_id, invited_by, invited_user, is_request, created_at)
         VALUES ($1, $2, $3, FALSE, $4)
         ON CONFLICT (run_id, invited_user) DO UPDATE
             SET status = 'pending', is_request = FALSE, responded_at = NULL, created_at = EXCLUDED.created_at
         RETURNING id",
        &[&(run_id as i32), &(inviter_user_id as i32), &invitee_id, &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;

    Ok(result.get::<_, i32>(0) as u32)
}

/// Accept or decline a run invite.
///
/// The responding user must be `invited_user` for the invite.
/// Returns `Ok(())` or `Err(reason)`.
pub fn respond_to_invite(run_id: u32, user_id: u32, accept: bool) -> Result<(), String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();
    let status = if accept { "accepted" } else { "declined" };
    let rows_affected = state.client.execute(
        "UPDATE run_invites SET status = $1, responded_at = $2
         WHERE run_id = $3 AND invited_user = $4 AND is_request = FALSE AND status = 'pending'",
        &[&status, &(unix_now() as i64), &(run_id as i32), &(user_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    if rows_affected == 0 {
        Err("no pending invite found for this run".to_string())
    } else {
        Ok(())
    }
}

/// List all invites for a run (owner view).
///
/// Returns `{ "invites": [...] }` or `{ "error": "..." }`.
pub fn get_run_invites_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT ri.id, u.username AS invitee, ub.username AS invited_by, ri.status, ri.created_at, ri.responded_at
         FROM run_invites ri
         JOIN users u  ON u.id  = ri.invited_user
         JOIN users ub ON ub.id = ri.invited_by
         WHERE ri.run_id = $1 AND ri.is_request = FALSE
         ORDER BY ri.created_at DESC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let invites: Vec<serde_json::Value> = rows.iter().map(|row| {
        let created: i64 = row.get(4);
        let responded: Option<i64> = row.get(5);
        serde_json::json!({
            "invite_id":    row.get::<_, i32>(0),
            "invitee":      row.get::<_, String>(1),
            "invited_by":   row.get::<_, String>(2),
            "status":       row.get::<_, String>(3),
            "created_at":   format_timestamp(created as u64),
            "responded_at": responded.map(|t| format_timestamp(t as u64)),
        })
    }).collect();
    serde_json::json!({ "run_id": run_id, "invites": invites })
}

/// Return `true` if `user_id` owns or has an accepted invite for `run_id`.
pub fn user_can_access_run(run_id: u32, user_id: u32) -> Result<bool, String> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT 1 FROM runs WHERE id = $1 AND user_id = $2
         UNION ALL
         SELECT 1 FROM run_invites
         WHERE run_id = $1 AND invited_user = $2 AND status = 'accepted'
         LIMIT 1",
        &[&(run_id as i32), &(user_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(row.is_some())
}

/// Return the user_id that owns `run_id`, or `None` if not found or unowned.
pub fn get_run_owner_id(run_id: u32) -> Option<u32> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).ok()??;
    let owner: Option<i32> = row.get(0);
    owner.map(|v| v as u32)
}

/// Submit an access request for a run the caller does not own.
///
/// Reuses the `run_invites` table with `is_request = TRUE` and
/// `invited_by = invited_user = requester_id`.  If a prior request (or
/// invite) exists for this `(run_id, user)` pair it is reset to pending.
/// Returns the invite-row id on success.
pub fn request_run_invite(run_id: u32, requester_id: u32) -> Result<u32, String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();

    // Run must exist.
    let run_row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    let owner: Option<i32> = run_row
        .as_ref()
        .ok_or_else(|| format!("run #{run_id} not found"))?
        .get(0);

    if owner.is_none() {
        return Err("this run has no owner and cannot accept access requests".to_string());
    }

    if owner == Some(requester_id as i32) {
        return Err("you already own this run".to_string());
    }

    let row = state.client.query_one(
        "INSERT INTO run_invites (run_id, invited_by, invited_user, is_request, created_at)
         VALUES ($1, $2, $2, TRUE, $3)
         ON CONFLICT (run_id, invited_user) DO UPDATE
             SET status = 'pending', is_request = TRUE,
                 responded_at = NULL, created_at = EXCLUDED.created_at
         RETURNING id",
        &[&(run_id as i32), &(requester_id as i32), &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;

    Ok(row.get::<_, i32>(0) as u32)
}

/// Approve or deny an access request.  `approver_id` must own the run.
pub fn respond_to_invite_request(run_id: u32, requester_id: u32, approver_id: u32, approve: bool) -> Result<(), String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();

    let row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    let owner: Option<i32> = row.as_ref().and_then(|r| r.get(0));
    if owner != Some(approver_id as i32) {
        return Err("you do not own this run".to_string());
    }

    let status = if approve { "accepted" } else { "declined" };
    let n = state.client.execute(
        "UPDATE run_invites SET status = $1, responded_at = $2
         WHERE run_id = $3 AND invited_user = $4 AND is_request = TRUE AND status = 'pending'",
        &[&status, &(unix_now() as i64), &(run_id as i32), &(requester_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;

    if n == 0 { Err("no pending request found".to_string()) } else { Ok(()) }
}

/// List pending access requests for a run (owner view).
pub fn get_run_invite_requests_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT ri.id, u.id AS uid, u.username, ri.created_at
         FROM run_invites ri
         JOIN users u ON u.id = ri.invited_user
         WHERE ri.run_id = $1 AND ri.is_request = TRUE AND ri.status = 'pending'
         ORDER BY ri.created_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let requests: Vec<serde_json::Value> = rows.iter().map(|row| {
        let created: i64 = row.get(3);
        serde_json::json!({
            "invite_id":  row.get::<_, i32>(0),
            "user_id":    row.get::<_, i32>(1),
            "username":   row.get::<_, String>(2),
            "created_at": format_timestamp(created as u64),
        })
    }).collect();
    serde_json::json!({ "run_id": run_id, "requests": requests })
}

/// Return each run's access status for the given user.
///
/// `"owner"` | `"accepted"` | `"pending_invite"` | `"pending_request"`.
/// Declined / non-existent entries are omitted.
pub fn get_my_run_statuses_json(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let owned = match client.query(
        "SELECT id FROM runs WHERE user_id = $1",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let invite_rows = match client.query(
        "SELECT run_id, status, is_request FROM run_invites WHERE invited_user = $1",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Invite query failed: {e}") }),
    };

    let mut statuses = serde_json::Map::new();
    for row in &owned {
        let id: i32 = row.get(0);
        statuses.insert(id.to_string(), serde_json::json!("owner"));
    }
    for row in &invite_rows {
        let run_id: i32 = row.get(0);
        let status: String = row.get(1);
        let is_request: bool = row.get(2);
        let label = match (status.as_str(), is_request) {
            ("accepted", _) => "accepted",
            ("pending", false) => "pending_invite",
            ("pending", true) => "pending_request",
            _ => continue,
        };
        statuses.entry(run_id.to_string())
            .or_insert_with(|| serde_json::json!(label));
    }
    serde_json::json!({ "statuses": statuses })
}

/// Return all pending access requests on runs owned by the given user.
///
/// Used by the join-page owner view to approve/deny requests in bulk.
pub fn get_my_run_requests_json(conn_str: &str, owner_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT ri.id, ri.run_id, r.player_name, u.id AS uid, u.username, ri.created_at
         FROM run_invites ri
         JOIN runs  r ON r.id  = ri.run_id
         JOIN users u ON u.id  = ri.invited_user
         WHERE ri.is_request = TRUE AND ri.status = 'pending'
           AND r.user_id = $1
         ORDER BY ri.created_at ASC",
        &[&(owner_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let requests: Vec<serde_json::Value> = rows.iter().map(|row| {
        let created: i64 = row.get(5);
        serde_json::json!({
            "invite_id":   row.get::<_, i32>(0),
            "run_id":      row.get::<_, i32>(1),
            "player_name": row.get::<_, String>(2),
            "user_id":     row.get::<_, i32>(3),
            "username":    row.get::<_, String>(4),
            "created_at":  format_timestamp(created as u64),
        })
    }).collect();
    serde_json::json!({ "requests": requests })
}
