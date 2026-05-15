// Each PC lot is 80 bytes (0x50)
// there are 14 boxes x 30 slots
// starting at gPokemonStorage in WRAM
// slot address = POKEMON_STORAGE_ADDR + ((box * 30 + slot) * 0x50);

use fire_red_party_monitor::BoxPokemon;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static SAVE_BLOCK_3_PTR: usize = 0x03005010;
static BOX_DATA_OFFSET: usize = 0x4;
//static POKEMON_STORAGE_ADDR: u32 = 0x02029338; //0x02029814; <- this is based on memory that can shift and therefore not useful
static SLOT_SIZE: usize = 0x50;
static NUMBER_BOXES: usize = 14;
static NUMBER_SLOTS: usize = 30;
static POKEMON_STORAGE_LIST: OnceLock<Mutex<PokemonStorage>> = OnceLock::new();
static SLEEP_TIMER_IN_SECS: u64 = 5;
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn start_loop() {
    RUNNING.swap(true, Ordering::SeqCst);

    let _handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            update_box_list();
            std::thread::sleep(std::time::Duration::from_millis(SLEEP_TIMER_IN_SECS));
        }
    });
}

pub fn end_loop() {
    RUNNING.store(false, Ordering::SeqCst);
}

#[derive(Debug)]
pub struct PokemonStorage {
    entries: Vec<BoxPokemon>,
    species_set: HashSet<u16>,
}

impl PokemonStorage {
    pub fn get_storage_list() -> &'static Mutex<PokemonStorage> {
        POKEMON_STORAGE_LIST.get_or_init(|| {
            Mutex::new(PokemonStorage {
                entries: Vec::new(),
                species_set: HashSet::new(),
            })
        })
    }
}

fn get_box_0_ram_location() -> u32 {
    let command = fire_red_retroarch_interfacing::generate_command((SAVE_BLOCK_3_PTR) as u32, 4);
    let res = fire_red_retroarch_interfacing::get_from_retroarch(command, 6);
    let bytes: Vec<u8> = res
        .iter()
        .skip(2)
        .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
        .collect();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) + BOX_DATA_OFFSET as u32
}

pub fn get_storage_entries() -> Vec<BoxPokemon> {
    PokemonStorage::get_storage_list()
        .lock()
        .unwrap()
        .entries
        .clone()
}
pub fn get_storage_species_set() -> HashSet<u16> {
    PokemonStorage::get_storage_list()
        .lock()
        .unwrap()
        .species_set
        .clone()
}

pub fn get_storage_list() -> &'static Mutex<PokemonStorage> {
    POKEMON_STORAGE_LIST.get_or_init(|| {
        Mutex::new(PokemonStorage {
            entries: Vec::new(),
            species_set: HashSet::new(),
        })
    })
}

pub fn check_for_new_entry(entry: &BoxPokemon) -> Option<&BoxPokemon> {
    if entry.secure.growth.species == 0 {
        return None;
    }

    let mut storage = POKEMON_STORAGE_LIST
        .get_or_init(|| {
            Mutex::new(PokemonStorage {
                entries: Vec::new(),
                species_set: HashSet::new(),
            })
        })
        .lock()
        .unwrap();

    if storage.species_set.contains(&entry.secure.growth.species) {
        return None;
    }

    storage.species_set.insert(entry.secure.growth.species);
    storage.entries.push(entry.clone());
    Some(entry)
}

pub fn sync_storage() -> usize {
    let mut storage = POKEMON_STORAGE_LIST
        .get_or_init(|| {
            Mutex::new(PokemonStorage {
                entries: Vec::new(),
                species_set: HashSet::new(),
            })
        })
        .lock()
        .unwrap();
    let initial_size = storage.species_set.len();
    let list = get_box_entries_from_ram();

    if list.is_empty() {
        storage.entries = Vec::new();
        storage.species_set = HashSet::new();
    } else {
        let current_species: HashSet<u16> = list.iter().map(|i| i.secure.growth.species).collect();

        storage
            .entries
            .retain(|p| current_species.contains(&p.secure.growth.species));
        storage.species_set = storage
            .entries
            .iter()
            .map(|p| p.secure.growth.species)
            .collect();
    }
    storage.species_set.len() - initial_size
}

pub fn get_box_entries_from_ram() -> Vec<BoxPokemon> {
    use fire_red_retroarch_interfacing::*;

    let mut list: Vec<BoxPokemon> = Vec::new();

    let chunk_size = 5 * NUMBER_SLOTS * SLOT_SIZE;
    let full_size = NUMBER_BOXES * NUMBER_SLOTS * SLOT_SIZE;

    for chunk_start in (0..full_size).step_by(chunk_size) {
        let this_chunk_bytes = (full_size - chunk_start).min(chunk_size);

        let command = generate_command(
            get_box_0_ram_location() + chunk_start as u32,
            this_chunk_bytes,
        );
        let ret = get_from_retroarch(command, this_chunk_bytes + 2);
        let data: Vec<&str> = ret.iter().map(|s| s.as_str()).collect();

        // guard against malformed responses
        if data.len() < 3 {
            continue;
        }

        let bytes: Vec<u8> = data
            .iter()
            .skip(2)
            .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
            .collect();

        // guard against incomplete chunks
        if bytes.len() < SLOT_SIZE {
            continue;
        }

        for current_offset in (0..bytes.len()).step_by(SLOT_SIZE) {
            if current_offset + SLOT_SIZE > bytes.len() {
                break; // use break instead of panic
            }

            let res = BoxPokemon::fill_struct_from_bytes(
                &bytes,
                current_offset,
                fire_red_rom_buffer::get_rom(),
            );
            match res {
                Some(mon) => {
                    if mon.checksum != 0 {
                        list.push(mon);
                    }
                }
                None => continue,
            };
        }
    }

    list
}

pub fn update_box_list() -> bool {
    let mut change_occured = false;
    let list = get_box_entries_from_ram();
    for entry in list {
        let result = check_for_new_entry(&entry);
        if let Some(_) = result { change_occured = true };
    }
    sync_storage();
    change_occured
}

pub fn scan_for_pokemon(known_personality: u32) {
    use fire_red_retroarch_interfacing::*;

    // Scan all of EWRAM: 0x02000000 to 0x02040000
    // Do it in 16KB chunks to avoid huge requests
    let ewram_start = 0x02000000u32;
    let ewram_size = 0x00040000usize;
    let chunk = 0x4000usize;

    println!(
        "Scanning EWRAM for personality {:08X}...",
        known_personality
    );
    let target = known_personality.to_le_bytes();

    for offset in (0..ewram_size).step_by(chunk) {
        let addr = ewram_start + offset as u32;
        let command = generate_command(addr, chunk);
        let ret = get_from_retroarch(command, chunk + 2);
        let data: Vec<&str> = ret.iter().map(|s| s.as_str()).collect();
        let bytes: Vec<u8> = data
            .iter()
            .skip(2)
            .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
            .collect();

        for i in 0..bytes.len().saturating_sub(4) {
            if bytes[i..i + 4] == target {
                println!("HIT at absolute 0x{:08X}", addr + i as u32);
            }
        }
    }
    println!("Scan complete.");
}
