//! Utilities for reading, decrypting, and monitoring the player's pokemon
//! party data from Pokemon FireRed running in Retroarch with a core that
//! supports READ_CORE_MEMORY.
//!
//! # Architecture
//!
//! Rather than issuing individual UDP reads per pokemon or field, this module
//! reads directly from the EWRAM snapshot maintained by the `fire_red_memory`
//! crate. The snapshot is a full 256 KiB copy of GBA EWRAM, updated roughly
//! every 500 ms on a background thread. Party data is parsed by computing byte
//! offsets relative to `EWRAM_BASE` and slicing directly into the buffer.
//!
//! This approach is substantially faster than per-field UDP reads and avoids
//! tearing (reading party size from one snapshot and pokemon data from another).
//!
//! # Pokemon data layout
//!
//! The raw in-memory pokemon structures are encrypted. Each pokemon contains
//! four 12-byte substructures (Growth, Attack, EV/Condition, Misc) whose
//! ordering is determined by `personality % 24`. This module handles
//! decryption and reordering transparently.

use fire_red_get_values::*;
use serde_big_array::BigArray;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use arc_swap::ArcSwap;

// ---------------------------------------------------------------------------
// Address constants
// ---------------------------------------------------------------------------

/// Base address of EWRAM in the GBA address space.
///
/// Used to convert absolute GBA addresses into byte offsets within the
/// EWRAM snapshot buffer.
const EWRAM_BASE: usize = 0x02000000;

/// GBA address of the party size byte.
///
/// A single byte in the range 0–6 indicating how many pokemon are currently
/// in the player's party.
const POKEMON_PARTY_SIZE_ADDR: usize = 0x02024029;

/// GBA address of the first party member's data block.
///
/// Each party slot is [`POKEMON_SIZE`] bytes wide, laid out contiguously.
const POKEMON_PARTY_ADDR: usize = 0x02024284;

/// Size in bytes of a single in-memory [`Pokemon`] structure.
const POKEMON_SIZE: usize = 100;

/// ROM address of the ability name string table.
const ABILITY_NAMES_ADDR: u32 = 0x24FCB0;

/// Byte stride between entries in the ability name table.
const ABILITY_NAMES_STRIDE: u32 = 13;

/// ROM address of the base stat table.
const BASE_STATS_ADDR: u32 = 0x2547F4;

/// Size in bytes of a single base stat table entry.
const BASE_STATS_ENTRY_SIZE: u32 = 28;

/// Byte offset of ability slot 1 within a base stat entry.
const ABILITY_1_OFFSET: u32 = 0x16;

/// Byte offset of ability slot 2 within a base stat entry.
const ABILITY_2_OFFSET: u32 = 0x17;

/// Maximum length of a pokemon nickname in bytes.
const POKEMON_NAME_LENGTH: usize = 10;

/// Maximum length of an original trainer name in bytes.
const PLAYER_NAME_LENGTH: usize = 7;

/// Delay between automatic party refreshes.
const SLEEP_TIMER: std::time::Duration = std::time::Duration::from_millis(250);

// ---------------------------------------------------------------------------
// Substructure ordering
// ---------------------------------------------------------------------------

/// All 24 possible encrypted pokemon substructure orderings.
///
/// Pokemon data is internally divided into four encrypted 12-byte
/// substructures:
/// - `G` — Growth (species, item, experience, friendship)
/// - `A` — Attack (moves and PP)
/// - `E` — EV/Condition (EVs and contest stats)
/// - `M` — Misc (IVs, origin, ribbons)
///
/// The ordering for a given pokemon is `personality % 24`.
static ORDERS: [&str; 24] = [
    "GAEM", "GAME", "GEAM", "GEMA", "GMAE", "GMEA",
    "AGEM", "AGME", "AEGM", "AEMG", "AMGE", "AMEG",
    "EGAM", "EGMA", "EAGM", "EAMG", "EMGA", "EMAG",
    "MGAE", "MGEA", "MAGE", "MAEG", "MEGA", "MEAG",
];

// ---------------------------------------------------------------------------
// Statics
// ---------------------------------------------------------------------------

/// Global shared party state.
///
/// Uses [`ArcSwap`] for lock-free reads while the background thread updates.
static PARTY_DATA: OnceLock<ArcSwap<Party>> = OnceLock::new();

/// Controls the background polling loop.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether to populate ROM-derived metadata (ability name strings etc.).
static IS_CLEAN: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initializes the global party state and configures ROM-derived metadata.
///
/// # Parameters
///
/// * `is_clean` — When `true`, ability names and other ROM strings are
///   resolved and stored on each pokemon. When `false`, only raw numeric
///   IDs are populated (faster).
///
/// # Returns
///
/// A reference to the global [`ArcSwap`] containing the current [`Party`].
pub fn initialize_static_party(is_clean: bool) -> &'static ArcSwap<Party> {
    IS_CLEAN.store(is_clean, Ordering::SeqCst);
    PARTY_DATA.get_or_init(|| {
        ArcSwap::from_pointee(Party::from_ewram(
            &fire_red_memory::get_ewram(),
            fire_red_rom_buffer::get_rom(),
        ))
    })
}

/// Returns the global shared party state, initializing it if necessary.
pub fn get_static_party() -> &'static ArcSwap<Party> {
    PARTY_DATA.get_or_init(|| {
        ArcSwap::from_pointee(Party::from_ewram(
            &fire_red_memory::get_ewram(),
            fire_red_rom_buffer::get_rom(),
        ))
    })
}

/// Returns the current party snapshot.
///
/// Returns `None` if the party has not yet been initialized.
pub fn get_party() -> Option<Arc<Party>> {
    PARTY_DATA.get().map(|arc| arc.load_full())
}

/// Rebuilds the global party snapshot from the current EWRAM buffer.
///
/// Reads the latest EWRAM snapshot atomically and parses party data from it.
/// No UDP communication occurs.
pub fn update_party() {
    let ewram = fire_red_memory::get_ewram();
    let rom = fire_red_rom_buffer::get_rom();
    get_static_party().store(Arc::new(Party::from_ewram(&ewram, rom)));
}

/// Returns whether ROM-derived metadata population is enabled.
pub fn get_is_clean() -> bool {
    IS_CLEAN.load(Ordering::SeqCst)
}

/// Starts the background party polling loop.
///
/// Spawns a thread that calls [`update_party`] every [`SLEEP_TIMER`].
/// The party is read from the EWRAM snapshot maintained by `fire_red_memory`,
/// so no UDP calls occur here. Ensure `fire_red_memory::start_loop()` is
/// running before calling this.
pub fn start_loop() {
    RUNNING.store(true, Ordering::SeqCst);
    std::thread::spawn(|| {
        while RUNNING.load(Ordering::SeqCst) {
            update_party();
            std::thread::sleep(SLEEP_TIMER);
        }
    });
}

/// Stops the background party polling loop.
pub fn end_loop() {
    RUNNING.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Offset helpers
// ---------------------------------------------------------------------------

/// Converts an absolute GBA EWRAM address to a byte offset within the
/// EWRAM snapshot buffer.
///
/// # Panics
///
/// Panics in debug builds if `addr` is below [`EWRAM_BASE`].
#[inline]
fn ewram_offset(addr: usize) -> usize {
    debug_assert!(addr >= EWRAM_BASE, "address 0x{:08X} is below EWRAM_BASE", addr);
    addr - EWRAM_BASE
}

// ---------------------------------------------------------------------------
// Party
// ---------------------------------------------------------------------------

/// Represents the player's current pokemon party.
#[derive(Debug)]
#[repr(C)]
pub struct Party {
    /// Number of valid pokemon currently in the party (0–6).
    pub number_pokemon: u8,

    /// Party member data, in slot order.
    pub members: Vec<Pokemon>,
}

impl Default for Party {
    fn default() -> Self {
        Self::empty()
    }
}

impl Party {
    /// Creates an empty party with no members.
    pub fn empty() -> Self {
        Self {
            number_pokemon: 0,
            members: Vec::new(),
        }
    }

    /// Parses the current party from a full EWRAM snapshot.
    ///
    /// Reads the party size byte and each party member's 100-byte block
    /// directly from the buffer. Returns an empty party if the buffer is
    /// too small or the party size is out of range.
    ///
    /// # Parameters
    ///
    /// * `ewram`      — Full 256 KiB EWRAM snapshot from `fire_red_memory`.
    /// * `rom_buffer` — Full FireRed ROM data, used for metadata lookups.
    pub fn from_ewram(ewram: &[u8], rom_buffer: &[u8]) -> Self {
        let size_offset = ewram_offset(POKEMON_PARTY_SIZE_ADDR);

        // Guard against a snapshot that hasn't been populated yet.
        if ewram.len() <= size_offset {
            return Self::empty();
        }

        let number_pokemon = ewram[size_offset];
        if number_pokemon == 0 || number_pokemon > 6 {
            return Self::empty();
        }

        let party_offset = ewram_offset(POKEMON_PARTY_ADDR);
        let required_len = party_offset + (number_pokemon as usize * POKEMON_SIZE);
        if ewram.len() < required_len {
            return Self::empty();
        }

        let members: Vec<Pokemon> = (0..number_pokemon as usize)
            .filter_map(|i| {
                let slot_offset = party_offset + (i * POKEMON_SIZE);
                Pokemon::from_bytes(&ewram[slot_offset..slot_offset + POKEMON_SIZE], rom_buffer)
            })
            .collect();

        Self {
            number_pokemon,
            members,
        }
    }

    /// Returns the species name of the pokemon at the given party position.
    ///
    /// Returns an empty string if the position is out of range.
    pub fn get_species_string(&self, position: usize) -> String {
        self.members
            .get(position)
            .map(|m| m.box_mon.secure.growth.species_string.clone())
            .unwrap_or_default()
    }

    /// Returns the nickname of the pokemon at the given party position.
    ///
    /// Returns an empty string if the position is out of range.
    pub fn get_nickname(&self, position: usize) -> String {
        self.members
            .get(position)
            .map(|m| m.box_mon.nickname_string.clone())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// ROM helpers
// ---------------------------------------------------------------------------

/// Resolves an ability name string from the FireRed ROM.
///
/// Returns `"None"` for ability ID 0 and `"???"` if the offset is out of
/// range.
pub fn get_ability_string_from_id(rom_buffer: &[u8], id: u8) -> String {
    if id == 0 {
        return String::from("None");
    }
    let offset = (ABILITY_NAMES_ADDR + (id as u32 * ABILITY_NAMES_STRIDE)) as usize;
    if offset >= rom_buffer.len() {
        return String::from("???");
    }
    let name_bytes: Vec<u8> = rom_buffer[offset..]
        .iter()
        .copied()
        .take_while(|&b| b != 0xFF)
        .collect();
    fire_red_text::gba_string_to_ascii(&name_bytes, name_bytes.len(), 0)
}

/// Reads the ability ID for a given species from the FireRed ROM base stat
/// table.
///
/// Returns `0` for species ID 0 or if the offset is out of range.
pub fn get_species_ability_id(rom_buffer: &[u8], species: u16, ability_number: u8) -> u8 {
    if species == 0 {
        return 0;
    }
    let entry_addr = (BASE_STATS_ADDR + (species as u32 * BASE_STATS_ENTRY_SIZE)) as usize;
    let offset = if ability_number == 0 {
        ABILITY_1_OFFSET
    } else {
        ABILITY_2_OFFSET
    } as usize;
    if entry_addr + offset >= rom_buffer.len() {
        return 0;
    }
    rom_buffer[entry_addr + offset]
}

/// Verifies the pokemon data checksum.
///
/// The checksum is the 16-bit sum of all 24 decrypted `u16` words in the
/// 48-byte secure block. Returns `true` if it matches `stored_checksum`.
pub fn verify_checksum(decrypted: &[u8; 48], stored_checksum: u16) -> bool {
    let sum: u32 = decrypted
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
        .sum();
    (sum & 0xFFFF) as u16 == stored_checksum
}

// ---------------------------------------------------------------------------
// BoxPokemon
// ---------------------------------------------------------------------------

/// Raw boxed pokemon structure as stored in GBA memory.
///
/// Matches the game's in-memory `BoxPokemon` layout, including the four
/// encrypted 12-byte substructures in the `secure` union. The `secure_raw`
/// field preserves the original encrypted bytes for reference.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct BoxPokemon {
    pub personality: u32,
    pub ot_id: u32,
    pub nickname: [u8; POKEMON_NAME_LENGTH],
    pub language: u8,
    pub is_bad_egg: u8,
    pub has_species: u8,
    pub is_egg: u8,
    /// Unused flag; Pokémon Box Ruby & Sapphire refuse to deposit pokemon
    /// with this bit set.
    pub black_box_rs: u8,
    pub unused: [u8; 4],
    pub ot_name: [u8; PLAYER_NAME_LENGTH],
    pub markings: u8,
    pub checksum: u16,
    pub unknown: u16,
    #[serde(with = "BigArray")]
    pub secure_raw: [u8; 48],
    pub secure: SecureSubstruct,
    pub nickname_string: String,
    pub ability: u8,
    pub ability_string: String,
}

impl Default for BoxPokemon {
    fn default() -> Self {
        Self {
            personality: 0,
            ot_id: 0,
            nickname: [0u8; POKEMON_NAME_LENGTH],
            language: 0,
            is_bad_egg: 0,
            has_species: 0,
            is_egg: 0,
            black_box_rs: 0,
            unused: [0u8; 4],
            ot_name: [0u8; PLAYER_NAME_LENGTH],
            markings: 0,
            checksum: 0,
            unknown: 0,
            secure_raw: [0u8; 48],
            secure: SecureSubstruct::default(),
            nickname_string: String::new(),
            ability: 0,
            ability_string: String::new(),
        }
    }
}

impl BoxPokemon {
    /// Resolves and fills ability information from the ROM base stat table.
    pub fn fill_ability(&mut self, rom_buffer: &[u8]) {
        if self.secure.growth.species == 0 {
            self.ability = 0;
            self.ability_string = String::from("None");
            return;
        }
        self.ability = get_species_ability_id(
            rom_buffer,
            self.secure.growth.species,
            self.secure.misc.iv_egg_ability.ability_number,
        );
        if get_is_clean() {
            self.ability_string = get_ability_string_from_id(rom_buffer, self.ability);
        }
    }

    /// Parses a [`BoxPokemon`] from a raw byte slice.
    ///
    /// Returns `None` if:
    /// - `personality` or `ot_id` are both zero (empty slot)
    /// - The decrypted secure block fails checksum verification
    /// - The byte slice is too short
    pub fn from_bytes(buffer: &[u8], rom_buffer: &[u8]) -> Option<Self> {
        if buffer.len() < 80 {
            return None;
        }

        let mut offset = 0;

        let personality = read_u32(buffer, offset); offset += 4;
        let ot_id       = read_u32(buffer, offset); offset += 4;

        // An all-zero personality and OT ID indicates an empty party slot.
        if personality == 0 && ot_id == 0 {
            return None;
        }

        let mut nickname = [0u8; POKEMON_NAME_LENGTH];
        nickname.copy_from_slice(&buffer[offset..offset + POKEMON_NAME_LENGTH]);
        offset += POKEMON_NAME_LENGTH;

        let language = read_u8(buffer, offset); offset += 1;

        let egg_data    = read_u8(buffer, offset); offset += 1;
        let is_bad_egg  = egg_data & 0x80;
        let has_species = egg_data & 0x40;
        let is_egg      = egg_data & 0x20;
        let black_box_rs = egg_data & 0x10;
        let unused = [
            egg_data & 0x08,
            egg_data & 0x04,
            egg_data & 0x02,
            egg_data & 0x01,
        ];

        let mut ot_name = [0u8; PLAYER_NAME_LENGTH];
        ot_name.copy_from_slice(&buffer[offset..offset + PLAYER_NAME_LENGTH]);
        offset += PLAYER_NAME_LENGTH;

        let markings = read_u8(buffer, offset);  offset += 1;
        let checksum = read_u16(buffer, offset); offset += 2;
        let unknown  = read_u16(buffer, offset); offset += 2;

        // The secure block is always 48 bytes starting at offset 32.
        let secure_raw: [u8; 48] = buffer[offset..offset + 48]
            .try_into()
            .ok()?;

        let secure = SecureSubstruct::from_bytes(personality, ot_id, &secure_raw);

        // Reject pokemon whose decrypted data doesn't match the stored checksum.
        if !verify_checksum(&secure.decrypted_value, checksum) {
            return None;
        }

        let nickname_string = fire_red_text::gba_string_to_ascii(&nickname, nickname.len(), 0)
            .trim_matches('\0')
            .to_string();

        let mut ret = BoxPokemon {
            personality,
            ot_id,
            nickname,
            language,
            is_bad_egg,
            has_species,
            is_egg,
            black_box_rs,
            unused,
            ot_name,
            markings,
            checksum,
            unknown,
            secure_raw,
            secure,
            nickname_string,
            ability: 0,
            ability_string: String::new(),
        };
        ret.fill_ability(rom_buffer);
        Some(ret)
    }
}

// ---------------------------------------------------------------------------
// Pokemon
// ---------------------------------------------------------------------------

/// Full in-party pokemon structure.
///
/// Extends [`BoxPokemon`] with live battle stats that are only present for
/// pokemon currently in the party (not in the box).
#[repr(C)]
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pokemon {
    /// Boxed data (encrypted substructures, nickname, ability, etc.).
    pub box_mon: BoxPokemon,

    /// Current status condition bitmask.
    pub status: u32,     // offset 0x50

    /// Current level.
    pub level: u8,       // offset 0x54

    /// Mail item index.
    pub mail: u8,        // offset 0x55

    /// Current HP.
    pub hp: u16,         // offset 0x56

    /// Maximum HP.
    pub max_hp: u16,     // offset 0x58

    /// Attack stat.
    pub attack: u16,     // offset 0x5A

    /// Defense stat.
    pub defense: u16,    // offset 0x5C

    /// Speed stat.
    pub speed: u16,      // offset 0x5E

    /// Special Attack stat.
    pub sp_attack: u16,  // offset 0x60

    /// Special Defense stat.
    pub sp_defense: u16, // offset 0x62
}

impl Pokemon {
    /// Parses a full in-party pokemon from a 100-byte raw slice.
    ///
    /// Returns `None` if the `BoxPokemon` portion is invalid (empty slot or
    /// bad checksum).
    pub fn from_bytes(buffer: &[u8], rom_buffer: &[u8]) -> Option<Self> {
        if buffer.len() < POKEMON_SIZE {
            return None;
        }

        // BoxPokemon occupies the first 80 bytes.
        let box_mon = BoxPokemon::from_bytes(&buffer[..80], rom_buffer)?;

        // Battle stats follow immediately after the 80-byte BoxPokemon block.
        let mut offset = 80;
        let status    = read_u32(buffer, offset); offset += 4;
        let level     = read_u8(buffer, offset);  offset += 1;
        let mail      = read_u8(buffer, offset);  offset += 1;
        let hp        = read_u16(buffer, offset); offset += 2;
        let max_hp    = read_u16(buffer, offset); offset += 2;
        let attack    = read_u16(buffer, offset); offset += 2;
        let defense   = read_u16(buffer, offset); offset += 2;
        let speed     = read_u16(buffer, offset); offset += 2;
        let sp_attack = read_u16(buffer, offset); offset += 2;
        let sp_defense = read_u16(buffer, offset);

        Some(Pokemon {
            box_mon,
            status,
            level,
            mail,
            hp,
            max_hp,
            attack,
            defense,
            speed,
            sp_attack,
            sp_defense,
        })
    }

    /// Returns the pokemon's species name.
    pub fn get_species_string(&self) -> String {
        self.box_mon.secure.growth.species_string.clone()
    }

    /// Returns the pokemon's nickname.
    pub fn get_nickname_string(&self) -> String {
        self.box_mon.nickname_string.clone()
    }
}

// ---------------------------------------------------------------------------
// SecureSubstruct
// ---------------------------------------------------------------------------

/// The encrypted four-substructure block of a [`BoxPokemon`].
///
/// Contains both the raw encrypted bytes and the fully decrypted and parsed
/// substructures. Decryption uses the XOR key `personality ^ ot_id`, applied
/// to each 4-byte word in the 48-byte block.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct SecureSubstruct {
    /// XOR encryption key (`personality ^ ot_id`).
    pub key: u32,
    /// Original encrypted bytes.
    #[serde(with = "BigArray")]
    pub encrypted_value: [u8; 48],
    /// Fully decrypted bytes.
    #[serde(with = "BigArray")]
    pub decrypted_value: [u8; 48],
    pub growth:       GrowthSubstruct,
    pub attack:       AttackSubstruct,
    pub ev_condition: EvConditionSubstruct,
    pub misc:         MiscSubstruct,
}

impl Default for SecureSubstruct {
    fn default() -> Self {
        Self {
            key: 0,
            encrypted_value: [0u8; 48],
            decrypted_value: [0u8; 48],
            growth:       GrowthSubstruct::default(),
            attack:       AttackSubstruct::default(),
            ev_condition: EvConditionSubstruct::default(),
            misc:         MiscSubstruct::default(),
        }
    }
}

impl SecureSubstruct {
    /// Decrypts a single 4-byte word using the XOR key.
    fn decrypt_word(encrypted: &[u8], key: u32) -> [u8; 4] {
        let word = u32::from_le_bytes([encrypted[0], encrypted[1], encrypted[2], encrypted[3]]);
        (word ^ key).to_le_bytes()
    }

    /// Parses and decrypts a secure block from a 48-byte raw slice.
    pub fn from_bytes(personality: u32, ot_id: u32, encrypted_value: &[u8; 48]) -> Self {
        let key = personality ^ ot_id;

        // Decrypt each 4-byte word in the 48-byte block.
        let mut decrypted_value = [0u8; 48];
        for i in 0..12 {
            let word = Self::decrypt_word(&encrypted_value[i * 4..], key);
            decrypted_value[i * 4..i * 4 + 4].copy_from_slice(&word);
        }

        let order = ORDERS[(personality % 24) as usize];

        let mut secure = SecureSubstruct {
            key,
            encrypted_value: *encrypted_value,
            decrypted_value,
            growth:       GrowthSubstruct::default(),
            attack:       AttackSubstruct::default(),
            ev_condition: EvConditionSubstruct::default(),
            misc:         MiscSubstruct::default(),
        };

        // Each substructure is 12 bytes; their positions depend on `order`.
        for (i, ch) in order.chars().enumerate() {
            let index = i * 12;
            match ch {
                'G' => secure.growth       = GrowthSubstruct::fill_struct(&decrypted_value, index),
                'A' => secure.attack       = AttackSubstruct::fill_struct(&decrypted_value, index),
                'E' => secure.ev_condition = EvConditionSubstruct::fill_struct(&decrypted_value, index),
                'M' => secure.misc         = MiscSubstruct::fill_struct(&decrypted_value, index),
                _   => eprintln!("Unexpected substructure order character: {}", ch),
            }
        }

        secure
    }
}

// ---------------------------------------------------------------------------
// Substructs
// ---------------------------------------------------------------------------

/// Growth-related pokemon data (species, held item, experience, friendship).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct GrowthSubstruct {
    pub species: u16,
    pub held_item: u16,
    pub experience: u32,
    pub pp_bonuses: u8,
    pub friendship: u8,
    pub unknown: [u8; 2],
    /// Human-readable species name resolved from the species ID.
    pub species_string: String,
}

impl GrowthSubstruct {
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let mut i = offset;
        let species    = read_u16(buffer, i); i += 2;
        let held_item  = read_u16(buffer, i); i += 2;
        let experience = read_u32(buffer, i); i += 4;
        let pp_bonuses = read_u8(buffer, i);  i += 1;
        let friendship = read_u8(buffer, i);  i += 1;
        let unknown = [read_u8(buffer, i), read_u8(buffer, i + 1)];
        let species_string = fire_red_text::get_pokemon_name_by_number(species as usize)
            .unwrap_or_else(|e| e);
        Self { species, held_item, experience, pp_bonuses, friendship, unknown, species_string }
    }
}

/// Move and PP data.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct AttackSubstruct {
    pub moves: [u16; 4],
    pub pp: [u8; 4],
}

impl AttackSubstruct {
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let buf = &buffer[offset..offset + 12];
        let mut i = 0;
        let mut moves = [0u16; 4];
        let mut pp    = [0u8; 4];
        for m in moves.iter_mut() { *m = read_u16(buf, i); i += 2; }
        for p in pp.iter_mut()    { *p = read_u8(buf, i);  i += 1; }
        Self { moves, pp }
    }
}

/// EV and contest condition data.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct EvConditionSubstruct {
    pub hp_ev: u8, pub attack_ev: u8, pub defense_ev: u8,
    pub speed_ev: u8, pub sp_attack_ev: u8, pub sp_defense_ev: u8,
    pub cool: u8, pub beauty: u8, pub cute: u8,
    pub smart: u8, pub tough: u8, pub sheen: u8,
}

impl EvConditionSubstruct {
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let mut i = offset;
        let hp_ev        = read_u8(buffer, i); i += 1;
        let attack_ev    = read_u8(buffer, i); i += 1;
        let defense_ev   = read_u8(buffer, i); i += 1;
        let speed_ev     = read_u8(buffer, i); i += 1;
        let sp_attack_ev = read_u8(buffer, i); i += 1;
        let sp_defense_ev = read_u8(buffer, i); i += 1;
        let cool   = read_u8(buffer, i); i += 1;
        let beauty = read_u8(buffer, i); i += 1;
        let cute   = read_u8(buffer, i); i += 1;
        let smart  = read_u8(buffer, i); i += 1;
        let tough  = read_u8(buffer, i); i += 1;
        let sheen  = read_u8(buffer, i);
        Self { hp_ev, attack_ev, defense_ev, speed_ev, sp_attack_ev, sp_defense_ev,
               cool, beauty, cute, smart, tough, sheen }
    }
}

/// Miscellaneous encrypted pokemon metadata.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct MiscSubstruct {
    pub pokerus: u8,
    pub met_location: u8,
    pub origins: u16,
    pub iv_egg_ability: IvEggAbility,
    pub ribbons_obedience: u32,
}

impl MiscSubstruct {
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let mut i = offset;
        let pokerus      = read_u8(buffer, i);  i += 1;
        let met_location = read_u8(buffer, i);  i += 1;
        let origins      = read_u16(buffer, i); i += 2;
        let iv_egg_ability = IvEggAbility::fill_struct(read_u32(buffer, i)); i += 4;
        let ribbons_obedience = read_u32(buffer, i);
        Self { pokerus, met_location, origins, iv_egg_ability, ribbons_obedience }
    }
}

// ---------------------------------------------------------------------------
// IvEggAbility
// ---------------------------------------------------------------------------

/// Packed 32-bit field encoding IVs, egg flag, and ability slot.
///
/// Bit layout:
/// - bits  0–4:  HP IV
/// - bits  5–9:  Attack IV
/// - bits 10–14: Defense IV
/// - bits 15–19: Speed IV
/// - bits 20–24: Sp. Attack IV
/// - bits 25–29: Sp. Def IV
/// - bit  30:    Is Egg
/// - bit  31:    Ability slot (0 = ability 1, 1 = ability 2)
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct IvEggAbility {
    pub raw_data: u32,
    pub hp_iv: u8,
    pub attack_iv: u8,
    pub defense_iv: u8,
    pub speed_iv: u8,
    pub sp_attack_iv: u8,
    pub sp_def_iv: u8,
    pub egg: u8,
    pub ability_number: u8,
}

impl IvEggAbility {
    pub fn new(value: u32) -> Self {
        Self::fill_struct(value)
    }

    fn fill_struct(value: u32) -> Self {
        if value == 0 {
            return Self::default();
        }
        Self {
            raw_data:      value,
            hp_iv:         Self::bits(value,  0, 5),
            attack_iv:     Self::bits(value,  5, 5),
            defense_iv:    Self::bits(value, 10, 5),
            speed_iv:      Self::bits(value, 15, 5),
            sp_attack_iv:  Self::bits(value, 20, 5),
            sp_def_iv:     Self::bits(value, 25, 5),
            egg:           Self::bits(value, 30, 1),
            ability_number: Self::bits(value, 31, 1),
        }
    }

    #[inline]
    fn bits(value: u32, position: usize, count: usize) -> u8 {
        let mask = (1u32 << count) - 1;
        ((value >> position) & mask) as u8
    }
}