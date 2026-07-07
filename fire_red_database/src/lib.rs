use fire_red_states::LockOrRecover;
use postgres::{Client, NoTls};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
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

    /// Returns the player name for the current call context.
    ///
    /// Mirrors [`Self::effective_run_id`]: direct-mode slots each run their game
    /// loop on a dedicated thread and set `THREAD_CURRENT_PLAYER` to their own
    /// name via `set_thread_player_name`. That per-thread override takes
    /// precedence over the global `current_player` so multiple slots sharing one
    /// process don't clobber each other's player-name tag on every DB write.
    fn effective_player_name(&self) -> String {
        THREAD_CURRENT_PLAYER
            .with(|c| c.borrow().clone())
            .unwrap_or_else(|| self.current_player.clone())
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

// Per-thread player-name override used by direct-mode game-loop threads.
// Set via set_thread_player_name at thread startup; cleared by clear_thread_player_name.
thread_local! {
    static THREAD_CURRENT_PLAYER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Override the active player name for the current thread, and (like
/// [`set_player_name`]) persist it as the global fallback and update the run
/// row if it still holds the placeholder 'Unknown'.
///
/// Call this once a direct-mode game-loop thread discovers its trainer name,
/// so that all DB writes from that thread are tagged with the right player,
/// independent of whatever other threads are doing to the global
/// `DbState.current_player`. Without this, two concurrent per-host threads
/// sharing one process would race to overwrite each other's player-name tag,
/// causing every write from both threads to be tagged with whichever thread
/// won last.
pub fn set_thread_player_name(name: &str) {
    THREAD_CURRENT_PLAYER.with(|c| *c.borrow_mut() = Some(name.to_string()));
    set_player_name(name);
}

/// Clear the per-thread player name override set by [`set_thread_player_name`].
pub fn clear_thread_player_name() {
    THREAD_CURRENT_PLAYER.with(|c| *c.borrow_mut() = None);
}

/// RAII guard returned by [`set_thread_player_name_scoped`].
/// Clears the thread-local player name when dropped so it cannot leak to the
/// next task scheduled on the same OS thread.
pub struct ThreadPlayerNameGuard;

impl Drop for ThreadPlayerNameGuard {
    fn drop(&mut self) {
        THREAD_CURRENT_PLAYER.with(|c| *c.borrow_mut() = None);
    }
}

/// Override the active player name for the current thread and return a guard
/// that clears it automatically when dropped.
pub fn set_thread_player_name_scoped(name: &str) -> ThreadPlayerNameGuard {
    THREAD_CURRENT_PLAYER.with(|c| *c.borrow_mut() = Some(name.to_string()));
    set_player_name(name);
    ThreadPlayerNameGuard
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
const SCHEMA_VERSION: &str = "30";

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

        -- Migration v29: per-player display column, replacing v28's run-wide
        -- column above. A shared soul-link run has multiple physical
        -- connections tagged by player_name, so the pin must be keyed by
        -- (run_id, player_name), not just run_id.
        CREATE TABLE IF NOT EXISTS run_player_slots (
            run_id      INTEGER NOT NULL REFERENCES runs(id),
            player_name TEXT    NOT NULL,
            slot_index  INTEGER NOT NULL,
            PRIMARY KEY (run_id, player_name)
        );

        -- Migration v30: record where each session was created from, so the
        -- dashboard session manager can show recognizable entries.
        ALTER TABLE sessions ADD COLUMN IF NOT EXISTS ip TEXT;
        ALTER TABLE sessions ADD COLUMN IF NOT EXISTS user_agent TEXT;
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

/// Strips null bytes from a string so it is safe to insert into PostgreSQL.
///
/// GBA text encoding uses 0x00 as a padding byte. PostgreSQL rejects strings
/// containing null bytes with `invalid byte sequence for encoding "UTF8": 0x00`.
fn pg_safe(s: &str) -> String {
    s.replace('\0', "")
}

fn normalize_conn_str(s: &str) -> String {
    if s.starts_with("postgresql://") || s.starts_with("postgres://") || s.contains('=') {
        s.to_string()
    } else {
        format!("postgresql://{s}")
    }
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

// ---------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------
// One module per table group / feature area, split from the former single-file
// crate. Everything is re-exported so the public API is unchanged
// (`fire_red_database::foo` for every previously public item).

mod analytics;
mod catches;
mod deaths;
mod dump;
mod encounters;
mod export;
mod goals;
mod reader;
mod run_settings;
mod runs;
mod stats;
mod users;

pub use analytics::*;
pub use catches::*;
pub use deaths::*;
pub use dump::*;
pub use encounters::*;
pub use export::*;
pub use goals::*;
pub use reader::*;
pub use run_settings::*;
pub use runs::*;
pub use stats::*;
pub use users::*;

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
// Thread-local override tests
// ---------------------------------------------------------------------------

// These run without a live Postgres connection: the DB OnceLock is never
// initialized in the test process, so set_player_name / set_thread_player_name
// return early after updating the thread-local, which is exactly the state
// under test. Each test's assertions are confined to threads it owns, so the
// per-thread state cannot race with other tests.
#[cfg(test)]
mod thread_local_tests {
    use super::*;

    fn thread_run_id() -> Option<u32> {
        THREAD_RUN_ID.with(|c| c.get())
    }

    fn thread_player() -> Option<String> {
        THREAD_CURRENT_PLAYER.with(|c| c.borrow().clone())
    }

    #[test]
    fn set_and_clear_thread_run_id() {
        std::thread::spawn(|| {
            assert_eq!(thread_run_id(), None);
            set_thread_run_id(42);
            assert_eq!(thread_run_id(), Some(42));
            clear_thread_run_id();
            assert_eq!(thread_run_id(), None);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn thread_run_id_guard_clears_on_drop() {
        std::thread::spawn(|| {
            {
                let _guard = set_thread_run_id_scoped(7);
                assert_eq!(thread_run_id(), Some(7));
            }
            assert_eq!(thread_run_id(), None);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn set_and_clear_thread_player_name() {
        std::thread::spawn(|| {
            assert_eq!(thread_player(), None);
            set_thread_player_name("RED");
            assert_eq!(thread_player(), Some("RED".to_string()));
            clear_thread_player_name();
            assert_eq!(thread_player(), None);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn thread_player_name_guard_clears_on_drop() {
        std::thread::spawn(|| {
            {
                let _guard = set_thread_player_name_scoped("MISTY");
                assert_eq!(thread_player(), Some("MISTY".to_string()));
            }
            assert_eq!(thread_player(), None);
        })
        .join()
        .unwrap();
    }

    /// The bug fixed in v0.9.102: two direct-mode game-loop threads sharing
    /// one process must each keep their own player name — the second thread's
    /// assignment must not leak into the first thread's writes.
    #[test]
    fn thread_player_names_are_isolated_between_threads() {
        let t1 = std::thread::spawn(|| {
            set_thread_player_name("RED");
            // Give the other thread time to set its own name.
            std::thread::sleep(std::time::Duration::from_millis(50));
            thread_player()
        });
        let t2 = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            set_thread_player_name("LEAF");
            thread_player()
        });
        assert_eq!(t1.join().unwrap(), Some("RED".to_string()));
        assert_eq!(t2.join().unwrap(), Some("LEAF".to_string()));
    }

    #[test]
    fn thread_run_ids_are_isolated_between_threads() {
        let t1 = std::thread::spawn(|| {
            set_thread_run_id(1);
            std::thread::sleep(std::time::Duration::from_millis(50));
            thread_run_id()
        });
        let t2 = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            set_thread_run_id(2);
            thread_run_id()
        });
        assert_eq!(t1.join().unwrap(), Some(1));
        assert_eq!(t2.join().unwrap(), Some(2));
    }

    #[test]
    fn overrides_start_unset_on_new_threads() {
        std::thread::spawn(|| {
            assert_eq!(thread_run_id(), None);
            assert_eq!(thread_player(), None);
        })
        .join()
        .unwrap();
    }
}
