//! Utilities for reading, decrypting, and monitoring the player's pokemon
//! party data from Pokemon FireRed running in Retroarch with a core that
//! supports READ_CORE_MEMORY.
//! 
//! This module communicates with RetroArch over UDP in order to read live
//! memory from the emulator. The raw in-memory Pokemon data structures are
//! then decrypted and converted into Rust-friendly types.
//! 
//! The implementation mirrors the internal GBA pokemon data
//! layout used by Pokemon FireRed, including encrypted substructures
//! and personality-based substructure ordering.

use fire_red_get_values::*;
use fire_red_retroarch_interfacing::*;
use serde_big_array::BigArray;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use arc_swap::ArcSwap;

/// Memory Address containing the current number of pokemon in the player's party.
static POKEMON_PARTY_SIZE_ADDR: usize = 0x02024029;

/// Starting memory address of the player's party data.
static POKEMON_PARTY_ADDR: usize = 0x02024284;

/// Size of a single in-memory `Pokemon` structure in bytes
static POKEMON_SIZE: usize = 100;

/// ROM address containing pokemon ability name strings.
static ABILITY_NAMES_ADDR: u32 = 0x24FCB0;

/// Byte stride between ability name entries in ROM.
static ABILITY_NAMES_STRIDE: u32 = 13;

/// ROM address containing Pokemon base stat entries.
static BASE_STATS_ADDR: u32 = 0x2547F4;

/// Size of a single base stat table entry.
static BASE_STATS_ENTRY_SIZE: u32 = 28;

/// Offset of ability slot 1 inside a base stat entry.
static ABILITY_1_OFFSET: u32 = 0x16;

/// Offset of ability slot 2 inside a base stat entry.
static ABILITY_2_OFFSET: u32 = 0x17;

/// Delay between automatic party refreshes in milliseconds.
static SLEEP_TIMER_IN_MILLIS: u64 = 1000; 

/// Inidicates whether the background update loop is currently running.
static RUNNING:AtomicBool = AtomicBool::new(false);

/// Determines whether additional ROM-derived information should be populated.
/// 
/// When enabled, ability names are resolved into readable strings.
static IS_CLEAN:AtomicBool = AtomicBool::new(false);

/// Max length of a Pokemon nickname in bytes.
const POKEMON_NAME_LENGTH: usize = 10;

/// Max length of an original trainer name in bytes.
const PLAYER_NAME_LENGTH: usize = 7;

/// All possible encrypted pokemon substructure orders.
/// 
/// pokemon data is internally divided into four encrypted substructures.
/// - Growth (`G`)
/// - Attack (`A`)
/// - EV/Condition (`E`)
/// - Misc (`M`)
/// 
/// The order is determined by `personality % 24`
static ORDERS: [&str; 24] = [
    "GAEM", "GAME", "GEAM", "GEMA", "GMAE", "GMEA",
    "AGEM", "AGME", "AEGM", "AEMG", "AMGE", "AMEG",
    "EGAM", "EGMA", "EAGM", "EAMG", "EMGA", "EMAG",
    "MGAE", "MGEA", "MAGE", "MAEG", "MEGA", "MEAG",
];

/// Global shared party state.
/// 
/// Uses [`ArcSwap`] to allow lock-free reads while updates are occurring.
static PARTY_DATA: OnceLock<ArcSwap<Party>> = OnceLock::new();

/// Initializes the global shared party state.
/// 
/// This function also configures whether additional ROM-derived strings
/// should be populated.
/// 
/// # Parameters
/// 
/// * `is_clean` - Enables or disables clean string lookups.
/// 
/// # Returns
/// 
/// A reference to the global [`ArcSwap`] containing the current [`Party`].
pub fn initialize_static_party(is_clean: bool) -> &'static ArcSwap<Party> {  
    IS_CLEAN.swap(is_clean, Ordering::SeqCst);
    PARTY_DATA.get_or_init(|| {
        ArcSwap::from_pointee(Party::new(fire_red_rom_buffer::get_rom()))
    })
}

/// Returns the global shared [`Party`] state.
/// 
/// If the party has not yet been initalized, it is created automatically.
pub fn get_static_party() -> &'static ArcSwap<Party> {
    PARTY_DATA.get_or_init(|| {
        ArcSwap::from_pointee(Party::new(fire_red_rom_buffer::get_rom()))
    })
}

/// Returns the current party snapshot.
/// 
/// # Returns
/// 
/// An [`Arc`] containing the latest available [`Party`] state,
/// or `None` if initialization has not yet occurred.
pub fn get_party() -> Option<Arc<Party>> {
    let data = PARTY_DATA.get();
    data.map(|arc| arc.load_full())
}

/// Rebuilds and replaces the global party snapshot.
/// 
/// This performs a fresh memory read from RetroArch
pub fn update_party() {
    get_static_party().store(Arc::new(Party::new(fire_red_rom_buffer::get_rom())));
}

/// Returns whether clean ROM-derived metadata is enabled.
pub fn get_is_clean() -> bool {
    IS_CLEAN.load(Ordering::SeqCst)
}

/// Starts the background polling loop.
/// 
/// The loop periodically refreshes the shared [`Party`] state
/// from emulator memory until [`end_loop`] is called.
pub fn start_loop() {
    RUNNING.swap(true, Ordering::SeqCst);

    let _handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            update_party();
            std::thread::sleep(std::time::Duration::from_millis(SLEEP_TIMER_IN_MILLIS));
        }
    });
}

/// Stops the background polling loop.
pub fn end_loop() {
    RUNNING.store(false, Ordering::SeqCst);
}

/// Represents the player's current pokemon party.
#[derive(Debug)]
#[repr(C)]
pub struct Party {
    /// Number of valid pokemon currently in the party.
    pub number_pokemon: u8,

    /// Party member data.
    pub members: Vec<Pokemon>,
}

impl Default for Party {
    /// Enables the Default trait for party by creating an empty party with no members
    fn default() -> Self {
        Self::empty()
    }
}

impl Party {
    /// Creates an empty party with zero members.
    pub fn empty() -> Self {
        Self {
            number_pokemon: 0,
            members: Vec::new()
        }
    }

    /// Reads and constructs the current party directly from emulator memory.
    /// 
    /// # Parameters
    /// 
    /// * `rom_buffer` - Full FireRed ROM data used for metadata lookups.
    /// 
    /// # Notes
    /// 
    /// This function continuously retries failed RetroArch reads until valid
    /// data is received.
    pub fn new(rom_buffer: &[u8]) -> Self {
        let mut got_return = false;
        let mut ret: Option<Vec<String>>;
        let mut number_pokemon: u8 = 0;

        while got_return == false {
            let command = generate_command(POKEMON_PARTY_SIZE_ADDR as u32, 1);
            ret = fire_red_retroarch_interfacing::get_from_retroarch(command.as_str(), 3);
            if ret.is_none(){
                println!("Failed to read party size, retrying...");
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            if ret.as_ref().unwrap().len() < 3 {
                println!("Received malformed response for party size, retrying...");
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            let result = match ret.as_ref().unwrap()[2].parse::<i8>() {
                Ok(v) => v,
                Err(_) => {
                    println!("Failed to parse party size, retrying...");
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            };
            if result < 0 {
                println!("Received invalid party size {}, retrying...", result);
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            number_pokemon = result as u8;
            if number_pokemon > 6 {
                println!("Received invalid party size {}, retrying...", number_pokemon);
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            got_return = true;
        }        
        let mut members: Vec<Pokemon> = Vec::new();
        let mut ret: Option<Vec<String>>;

        for i in 0..number_pokemon {
            ret = None;
            let mut got_return = false;
            while got_return == false {
                let command = generate_command((POKEMON_PARTY_ADDR as u32) + (i as usize * POKEMON_SIZE) as u32, POKEMON_SIZE);
                ret = fire_red_retroarch_interfacing::get_from_retroarch(command.as_str(), POKEMON_SIZE + 2);
                if ret.is_none() {
                    println!("Failed to read data for party member {}, retrying...", i);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                got_return = true;
            }
            let ret = ret.unwrap();
            let buffer: Vec<&str> = ret.iter().map(|s| s.as_str()).collect();
            
            members.push(Pokemon::fill_struct(&buffer, 2, &rom_buffer));
        }

        Self {
            number_pokemon,
            members,
        }
    }

    /// Rebuilds teh current party using the latest emulator memory state
    pub fn update(self) -> Self {
        Self::new(fire_red_rom_buffer::get_rom())
    }

    /// Returns the species name of the pokemon at the given party position.
    /// 
    /// Returns an empty string if the position is invalid.
    pub fn get_species_string(&self, position: usize) -> String {
        if position >= self.members.len() {
            return String::from("");
        }
        self.members[position].box_mon.secure.growth.species_string.clone()
    }

    /// Returns the nickname of the pokemon at the given party position.
    /// 
    /// Returns an empty string if the position is invalid.
    pub fn get_nickname(&self, position: usize) -> String {
        if position >= self.members.len() {
            return String::from("");
        }
        self.members[position].box_mon.nickname_string.clone()
    }
}

/// Raw boxed pokemon structure used internally by FireRed
/// 
/// This structure matches the game's in-memory boxed pokemon format,
/// including encrypted substructures.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct BoxPokemon {
    pub personality: u32, 
    pub ot_id: u32,       

    pub nickname: [u8; POKEMON_NAME_LENGTH], 
    pub language: u8,       

    // the following is in one byte:
    pub is_bad_egg: u8,     
    pub has_species: u8,   
    pub is_egg: u8,
    pub black_box_rs: u8, //unused, but ruby and sapphire refuse to deposite pokemon w/ this flag
    pub unused: [u8; 4],

    // back to normal
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

/*

struct BoxPokemon
{
    u32 personality;
    u32 otId;
    u8 nickname[POKEMON_NAME_LENGTH];
    u8 language;
    u8 isBadEgg:1;
    u8 hasSpecies:1;
    u8 isEgg:1;
    u8 blockBoxRS:1; // Unused, but Pokémon Box Ruby & Sapphire will refuse to deposit a Pokémon with this flag set
    u8 unused:4;
    u8 otName[PLAYER_NAME_LENGTH];
    u8 markings;
    u16 checksum;
    u16 unknown;

    union
    {
        u32 raw[(NUM_SUBSTRUCT_BYTES * 4) / 4]; // *4 because there are 4 substructs, /4 because it's u32, not u8
        union PokemonSubstruct substructs[4];
    } secure;
};
*/

/// Resolves an ability name from an ability ID.
/// 
/// # Parameters
/// 
/// * `rom_buffer` - Full ROM buffer.
/// * `id` - Ability ID.
/// 
/// # Returns
/// 
/// A decoded ability name string.
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

/// Reads the ability ID assigned to a species.
/// 
/// # Parameters
/// 
/// * `rom_buffer` - Full ROM buffer.
/// * `species` - pokemon species ID.
/// * `abililty_number` - Ability slot index
/// 
/// # Return
/// 
/// The resolved ability ID.
pub fn get_species_ability_id(rom_buffer: &[u8], species: u16, ability_number: u8) -> u8 {
    if species == 0 {
        return 0;
    }
    let entry_addr = (BASE_STATS_ADDR + (species as u32 * BASE_STATS_ENTRY_SIZE)) as usize;
    let offset = if ability_number == 0 { ABILITY_1_OFFSET } else { ABILITY_2_OFFSET } as usize;
    if entry_addr + offset >= rom_buffer.len() {
        return 0;
    }
    rom_buffer[entry_addr + offset]
}

impl BoxPokemon {
    /// Resolves and fills the pokemon's ability information.
    pub fn fill_ability(&mut self, rom_buffer: &[u8]) {
        if self.secure.growth.species == 0 {
            self.ability = 0;
            self.ability_string = String::from("None");
            return;
        }
        self.ability = get_species_ability_id(rom_buffer, self.secure.growth.species, self.secure.misc.iv_egg_ability.ability_number);
        if get_is_clean() { self.ability_string = get_ability_string_from_id(rom_buffer, self.ability); }
    }

    /// Parses a boxed pokemon directly from raw bytes
    /// 
    /// # Returns
    /// 
    /// `None` if checksum verification fails.
    pub fn fill_struct_from_bytes(buffer: &[u8], mut offset: usize, rom_buffer: &[u8]) -> Option<Self> {
        let personality = read_u32(&buffer, offset);
        offset += 4;
        let ot_id = read_u32(&buffer, offset);
        offset += 4;

        let mut nickname = [0u8; POKEMON_NAME_LENGTH];
        for i in 0..POKEMON_NAME_LENGTH {
            nickname[i] = read_u8(&buffer, offset);
            offset += 1;
        }
        let language = read_u8(&buffer, offset);
        offset += 1;

        let egg_data = read_u8(&buffer, offset);
        let is_bad_egg = egg_data & 0x80;
        let has_species = egg_data & 0x40;
        let is_egg = egg_data & 0x20;
        let black_box_rs = egg_data & 0x10;
        let unused: [u8; 4] = [egg_data & 0x08, egg_data & 0x04, egg_data & 0x02, egg_data & 0x01];
        offset += 1;

        let mut ot_name = [0u8; PLAYER_NAME_LENGTH];
        for i in 0..PLAYER_NAME_LENGTH {
            ot_name[i] = read_u8(&buffer, offset);
            offset += 1;
        }
        let markings = read_u8(&buffer, offset);
        offset += 1;
        let checksum = read_u16(&buffer, offset);
        offset += 2;
        let unknown = read_u16(&buffer, offset);
        offset += 2;
        let secure_raw = (&buffer[offset..offset + 48]).as_array().unwrap().clone();
        
        let secure = SecureSubstruct::fill_struct_from_bytes(personality, ot_id, buffer, offset);
        let nickname_string = fire_red_text::gba_string_to_ascii(&nickname, nickname.len(), 0).trim_matches('\0').to_string();

        if verify_checksum(&secure.decrypted_value, checksum) == false {
            return None;
        }

        let mut ret_ = BoxPokemon {
            personality,
            ot_id,
            nickname,
            language,
            is_bad_egg,
            has_species,
            is_egg,
            black_box_rs,
            unused: unused,
            ot_name: ot_name,
            markings,
            checksum,
            unknown,
            secure_raw,
            secure,
            nickname_string,
            ability: 0,
            ability_string: String::new(),
        };
        ret_.fill_ability(rom_buffer);
        Some(ret_)
        
    }

    /// Parses a boxed pokemon from RetroArch string-based memory output.
    /// 
    /// # Returns
    /// 
    /// Returns the parsed pokemon and the next unread offset.
    pub fn fill_struct(buffer: &[&str], mut offset: usize, rom_buffer: &[u8]) -> Option<(Self, usize)> {
        let personality = get_u32(&buffer[offset..offset + 4]);
        offset += 4;
        let ot_id = get_u32(&buffer[offset..offset + 4]);
        offset += 4;

        if personality == 0 || ot_id == 0 {
            return None;
        }

        let mut nickname = [0u8; POKEMON_NAME_LENGTH];
        for i in 0..POKEMON_NAME_LENGTH {
            nickname[i] = get_u8(&[buffer[offset]]);
            offset += 1;
        }
        let language = get_u8(&[buffer[offset]]);
        offset += 1;
        let egg_data = get_u8(&[buffer[offset]]);
        let is_bad_egg = egg_data & 0x80;
        let has_species = egg_data & 0x40;
        let is_egg = egg_data & 0x20;
        let black_box_rs = egg_data & 0x10;
        let unused: [u8; 4] = [egg_data & 0x08, egg_data & 0x04, egg_data & 0x02, egg_data & 0x01];
        offset += 1;

        let mut ot_name = [0u8; PLAYER_NAME_LENGTH];
        for i in 0..PLAYER_NAME_LENGTH {
            ot_name[i] = get_u8(&[buffer[offset]]);
            offset += 1;
        }
        let markings = get_u8(&[buffer[offset]]);
        offset += 1;
        let checksum = get_u16(&buffer[offset..offset + 2]);
        offset += 2;
        let unknown = get_u16(&buffer[offset..offset + 2]);
        offset += 2;


        let secure_raw: Vec<u8> = buffer[offset..offset + 48]
            .iter()
            .filter_map(|n| u8::from_str_radix(n, 16).ok())
            .collect();
        let secure_raw: [u8; 48] = match secure_raw.try_into() {
                Ok(arr) => arr,
                Err(_) => return None,
            };

        let secure = SecureSubstruct::fill_struct(personality, ot_id, buffer, offset).unwrap_or_default();
        let nickname_string = fire_red_text::gba_string_to_ascii(&nickname, nickname.len(), 0).trim_matches('\0').to_string();

        let mut ret = BoxPokemon {
            personality,
            ot_id,
            nickname,
            language,
            is_bad_egg,
            has_species,
            is_egg,
            black_box_rs,
            unused: unused,
            ot_name: ot_name,
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
        Some((ret, offset + 0x30))
    }
}

/// Full in-party pokemon structure
/// 
/// This extends [`BoxPokemon`] with battle-related stats.
#[repr(C)]
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pokemon {
    /// Boxed pokemon data.
    pub box_mon: BoxPokemon,

    /// Current status condition
    pub status: u32,     //0x50
    
    /// Current Level
    pub level: u8,       //0x54

    /// Mail item index
    pub mail: u8,        //0x55

    /// Current HP
    pub hp: u16,         //0x56

    /// Maxium HP
    pub max_hp: u16,     //0x58

    /// Attack stat
    pub attack: u16,     //0x5A

    /// Defense stat
    pub defense: u16,    //0x5C

    /// Speed stat
    pub speed: u16,      //0x5E

    /// Special attack stat
    pub sp_attack: u16,  //0x60

    /// Special defense stat
    pub sp_defense: u16, //0x62
}

impl Pokemon {
    /// Parses a full in-party pokemon structure from RetroArch memory output.
    pub fn fill_struct(buffer: &[&str], offset: usize, rom_buffer: &[u8]) -> Self {
        //let Some((box_mon, new_offset)) = BoxPokemon::fill_struct(buffer, offset);
        let mut offset = offset;
        let box_mon: BoxPokemon;
        match BoxPokemon::fill_struct(buffer, offset, &rom_buffer) {
            Some((mon, new_offset)) => {
                offset = new_offset;
                box_mon = mon;
            },
            None => box_mon = BoxPokemon::default(),
        }
        let status = get_u32(&buffer[offset..offset + 4]);
        offset += 4;
        let level = get_u8(&[buffer[offset]]);
        offset += 1;
        let mail = get_u8(&[buffer[offset]]);
        offset += 1;
        let hp = get_u16(&buffer[offset..offset + 2]);
        offset += 2;
        let max_hp = get_u16(&buffer[offset..offset + 2]);
        offset += 2;
        let attack = get_u16(&buffer[offset..offset + 2]);
        offset += 2;
        let defense = get_u16(&buffer[offset..offset + 2]);
        offset += 2;
        let speed = get_u16(&buffer[offset..offset + 2]);
        offset += 2;
        let sp_attack = get_u16(&buffer[offset..offset + 2]);
        offset += 2;
        let sp_defense = get_u16(&buffer[offset..offset + 2]);

        Pokemon {
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
        }
    }

    /// Returns the pokemon species name.
    pub fn get_species_string(&self) -> String {
        self.box_mon.secure.growth.species_string.clone()
    }

    /// Returns the pokemon nickname.
    pub fn get_nickname_string(&self) -> String { 
        self.box_mon.nickname_string.clone()
    }
}

/// Represents the encrypted pokemon substructure block.
/// 
/// Teh secure data block contains four encrypted 12-byte substructures.
/// The ordering depends on `personality % 24`
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct SecureSubstruct {    
    /// XOR encryption key (`personality ^ ot_id`).
    pub key: u32,

    /// Raw encrypted bytes.
    #[serde(with = "BigArray")]
    pub encrypted_value: [u8; 48],

    /// Fully decrypted bytes.
    #[serde(with = "BigArray")]
    pub decrypted_value: [u8; 48],

    /// Growth-related data.
    pub growth: GrowthSubstruct,

    /// Move and PP data.
    pub attack: AttackSubstruct,

    /// EV and contest condition data.
    pub ev_condition: EvConditionSubstruct,

    /// Misc. metadata
    pub misc: MiscSubstruct,
}

impl Default for SecureSubstruct {
    fn default() -> Self {
        Self {
            key: 0,
            encrypted_value: [0u8; 48],
            decrypted_value: [0u8; 48],
            growth: GrowthSubstruct::default(),
            attack: AttackSubstruct::default(),
            ev_condition: EvConditionSubstruct::default(),
            misc: MiscSubstruct::default(),
        }
    }
}

impl SecureSubstruct {
    /// Decrypts a 4-byte encrypted pokemon data chunk.
    fn decrypt_chunk(encrypted_value: &[u8], key: u32) -> Vec<u8> {
        let encrypted_word = u32::from_le_bytes([encrypted_value[0], encrypted_value[1], encrypted_value[2], encrypted_value[3]]);
        
        let decrypted_word = encrypted_word ^ key;
        decrypted_word.to_le_bytes().to_vec()
    }

    /// Parses and decrypts a secure substructure block from raw bytes.
    pub fn fill_struct_from_bytes(personality: u32, ot_id: u32, buffer: &[u8], offset: usize) -> Self {
        let key = personality ^ ot_id;
        let order_number = personality % 24;

        let mut encrypted_value = [0u8; 48];
        encrypted_value.copy_from_slice(&buffer[offset..offset + 48]);

        if encrypted_value.len() != 48 {
            panic!("didn't copy the correct number of bytes!");
        }        
        let mut decrypted_value: Vec<u8> = Vec::new();

        for i in 0..12 {
            let mut result = SecureSubstruct::decrypt_chunk(&encrypted_value[(i * 4)..(i * 4 + 4)], key);
            decrypted_value.append(&mut result);
        }

        let encrypted_value = encrypted_value.clone();

        let mut decrypted_value_copy = [0u8; 48];
        decrypted_value_copy.copy_from_slice(&decrypted_value);

        let order = ORDERS[order_number as usize];

        let mut secure = SecureSubstruct {
            key,
            encrypted_value,
            decrypted_value: decrypted_value_copy,
            growth: GrowthSubstruct::default(),
            attack: AttackSubstruct::default(),
            ev_condition: EvConditionSubstruct::default(),
            misc: MiscSubstruct::default(),
        };

        let mut index = 0;
        for i in 0..4 {
            secure.fill_substruct_by_char(order.chars().nth(i).unwrap(), &decrypted_value, index);
            index += 12;
        }

        secure
    }

    /// Parses and decrypts a secure substructure block from RetroArch output.
    pub fn fill_struct(personality: u32, ot_id: u32, buffer: &[&str], offset: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let key = personality ^ ot_id;
        let order_number = personality % 24;

        let encrypted_value = get_n_bytes(48, &buffer[offset..offset + 48]);
        if encrypted_value.is_none() {
            return Err("failed to parse encrypted value bytes!".into());
        }
        let encrypted_value = encrypted_value.unwrap();
        if encrypted_value.len() != 48 {
            return Err("didn't copy the correct number of bytes!".into());
        }        
        let mut decrypted_value: Vec<u8> = Vec::new();

        for i in 0..12 {
            let mut result = SecureSubstruct::decrypt_chunk(&encrypted_value[(i * 4)..(i * 4 + 4)], key);
            decrypted_value.append(&mut result);
        }
        
        let encrypted_value: &[u8] = &encrypted_value;
        let encrypted_value: Result<[u8; 48], _> = encrypted_value.try_into();
        let encrypted_value = encrypted_value.ok().unwrap();

        let decrypted_value: &[u8] = &decrypted_value;
        let decrypted_value: Result<[u8; 48], _> = decrypted_value.try_into();
        let decrypted_value = decrypted_value.ok().unwrap();

        let order = ORDERS[order_number as usize];

        let mut secure = SecureSubstruct {
            key,
            encrypted_value,
            decrypted_value,
            growth: GrowthSubstruct::default(),
            attack: AttackSubstruct::default(),
            ev_condition: EvConditionSubstruct::default(),
            misc: MiscSubstruct::default(),
        };

        let mut index = 0;
        for i in 0..4 {
            secure.fill_substruct_by_char(order.chars().nth(i).unwrap(), &decrypted_value, index);
            index += 12;
        }

        Ok(secure)
    }

    /// Dispatches a decrypted substructure into its correct destination field.
    fn fill_substruct_by_char(&mut self, letter: char, buffer: &[u8], index: usize) {
        match letter {
            'A' => self.attack = AttackSubstruct::fill_struct(buffer, index),
            'E' => self.ev_condition = EvConditionSubstruct::fill_struct(buffer, index),
            'G' => self.growth = GrowthSubstruct::fill_struct(buffer, index),
            'M' => self.misc = MiscSubstruct::fill_struct(buffer,index),
            _ => println!("how did you get a letter outside the range?!"),
        }
    }
}

/// Growth-related pokemon data.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct GrowthSubstruct {
    /// Pokemon species id
    pub species: u16,

    /// held item id
    pub held_item: u16,

    /// current exp.
    pub experience: u32,

    /// pp up bonuses
    pub pp_bonuses: u8,

    /// Friendship value.
    pub friendship: u8,

    /// Unknown bytes
    pub unknown: [u8; 2],

    /// Human-readable species name
    pub species_string: String,
}

impl GrowthSubstruct {
    /// Parses a growth substructure from decrypted bytes.
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let mut index = 0;
        let species = read_u16(&buffer, offset + index);
        index += 2;
        let held_item = read_u16(&buffer, offset + index);
        index += 2;
        let experience = read_u32(&buffer, offset + index);
        index += 4;
        let pp_bonuses = read_u8(&buffer, offset + index);
        index += 1;
        let friendship = read_u8(&buffer, offset + index);
        index += 1;
        let unknown: [u8; 2] = [
            read_u8(&buffer, offset + index),
            read_u8(&buffer, offset + index + 1),
        ];
        let species_string = match fire_red_text::get_pokemon_name_by_number(species as usize) {
            Ok(str) => str,
            Err(err) => err,
        };

        Self {
            species,
            held_item,
            experience,
            pp_bonuses,
            friendship,
            unknown,
            species_string,
        }
    }
}

/// Move and PP data.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct AttackSubstruct {
    /// Learned move IDs.
    pub moves: [u16; 4],

    /// Currrent PP values for each move.
    pub pp: [u8; 4],
}

impl AttackSubstruct {
    /// Parses an attack substructure from decrypted bytes.
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let buffer = &buffer[offset..offset + 12];
        let mut index = 0;
        let mut moves = [0u16; 4];
        let mut pp = [0u8; 4];

        for i in 0..4 {
            moves[i] = read_u16(&buffer, index);
            index += 2;
        }

        for i in 0..4 {
            pp[i] = read_u8(&buffer, index);
            index += 1;
        }

        Self { moves, pp }
    }
}

/// EV and contest condition data.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct EvConditionSubstruct {
    /// Stat EVs
    pub hp_ev: u8,
    pub attack_ev: u8,
    pub defense_ev: u8,
    pub speed_ev: u8,
    pub sp_attack_ev: u8,
    pub sp_defense_ev: u8,

    /// Contest stats
    pub cool: u8,
    pub beauty: u8,
    pub cute: u8,
    pub smart: u8,
    pub tough: u8,
    pub sheen: u8,
}

impl EvConditionSubstruct {
    /// Parse an EV/condition substructure from decrypted bytes.
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let mut index = 0;
        let hp_ev = read_u8(&buffer, offset);
        index += 1;
        let attack_ev = read_u8(&buffer, offset + index);
        index += 1;
        let defense_ev = read_u8(&buffer, offset + index);
        index += 1;
        let speed_ev = read_u8(&buffer, offset + index);
        index += 1;
        let sp_attack_ev = read_u8(&buffer, offset + index);
        index += 1;
        let sp_defense_ev = read_u8(&buffer, offset + index);
        index += 1;
        let cool = read_u8(&buffer, offset + index);
        index += 1;
        let beauty = read_u8(&buffer, offset + index);
        index += 1;
        let cute = read_u8(&buffer, offset + index);
        index += 1;
        let smart = read_u8(&buffer, offset + index);
        index += 1;
        let tough = read_u8(&buffer, offset + index);
        index += 1;
        let sheen = read_u8(&buffer, offset + index);

        Self {
            hp_ev,
            attack_ev,
            defense_ev,
            speed_ev,
            sp_attack_ev,
            sp_defense_ev,
            cool,
            beauty,
            cute,
            smart,
            tough,
            sheen,
        }
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
    /// Parses Misc. substructure from decrypted bytes.
    pub fn fill_struct(buffer: &[u8], offset: usize) -> Self {
        let mut index = 0;
        let pokerus = read_u8(&buffer, offset);
        index += 1;
        let met_location = read_u8(&buffer, offset + index);
        index += 1;
        let origins = read_u16(&buffer, offset + index);
        index += 2;
        let egg_ability = IvEggAbility::fill_struct(read_u32(&buffer, offset + index));
        index += 4;
        let ribbons_obedience = read_u32(&buffer, offset + index);

        Self {
            pokerus,
            met_location,
            origins,
            iv_egg_ability: egg_ability,
            ribbons_obedience,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct IvEggAbility {
    pub raw_data: u32,
    pub hp_iv: u8,          //bits 0-4
    pub attack_iv: u8,      //bits 5-9
    pub defense_iv: u8,     //bits 10-14
    pub speed_iv: u8,       //bits 15-19
    pub sp_attack_iv: u8,   //bits 20-24
    pub sp_def_iv: u8,      //bits 25-29
    pub egg: u8,            //bit 30
    pub ability_number: u8, //bit 31
}

// 00000 00000 00000 00000 00000 00000 0 0
impl IvEggAbility {
    pub fn new(buffer: u32) -> Self {
        IvEggAbility::fill_struct(buffer)
    }
    fn fill_struct(value: u32) -> Self {
        if value == 0 {
            return IvEggAbility::default();
        }

        let mut result = IvEggAbility::default();

        result.raw_data = value;
        result.hp_iv = IvEggAbility::get_bits(value, 0, 5);
        result.attack_iv = IvEggAbility::get_bits(value, 5, 5);
        result.defense_iv = IvEggAbility::get_bits(value, 10, 5);
        result.speed_iv = IvEggAbility::get_bits(value, 15, 5);
        result.sp_attack_iv = IvEggAbility::get_bits(value, 20, 5);
        result.sp_def_iv = IvEggAbility::get_bits(value, 25, 5);
        result.egg = IvEggAbility::get_bits(value, 30, 1);
        result.ability_number = IvEggAbility::get_bits(value, 31, 1);

        result
    }
    fn get_bits(value: u32, position: usize, number_bits: usize) -> u8 {
        let mask = (1u32 << number_bits) - 1;
        ((value >> position) & mask) as u8
    }
}

pub fn verify_checksum(decrypted: &[u8; 48], stored_checksum: u16) -> bool {
    let sum: u32 = decrypted  // only 44 bytes, not 48
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
        .sum();
    (sum & 0xFFFF) as u16 == stored_checksum
}