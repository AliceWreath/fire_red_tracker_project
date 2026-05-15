use std::alloc::{alloc, dealloc, Layout};
use std::ffi::{c_uchar, c_uint, c_ushort};
use std::sync::OnceLock;
use std::marker::PhantomData;

use fire_red_get_values::{read_u16, read_u8, read_u32};

//static WILD_POKEMON_HEADERS: LazyLock<Arc<Mutex<Vec<WildPokemonHeaderROM>>>> = LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));
static WILD_POKEMON_HEADERS: OnceLock<Vec<WildPokemonHeaderROM>> = OnceLock::new();

// This library will be used to pull pokemon information off the rom for easy access

/// this structure holds the pointers to the encounters for an area, which area is stored as map_group and map_num
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct WildPokemonHeaderROM {
    pub map_group: c_uchar,
    pub map_num: c_uchar,
    pub filler: [c_uchar; 2],
    pub land_mon_enounters_rom_ptr: c_uint,
    pub water_mon_encounters_rom_ptr: c_uint,
    pub rock_smash_encounters_rom_ptr: c_uint,
    pub fishing_encounters_rom_ptr: c_uint,
}

#[derive(Default, Debug, Clone)]
#[repr(C)]
pub struct WildPokemonHeaderFFI {
    pub map_group: c_uchar,
    pub map_num: c_uchar,
    pub land_mon_encounters: *mut WildPokemonInfoFFI,
    pub water_mon_encounters: *mut WildPokemonInfoFFI,
    pub rock_smash_encounters: *mut WildPokemonInfoFFI,
    pub fishing_encounters: *mut WildPokemonInfoFFI,
}

#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct WildPokemonHeader {
    pub map_group: c_uchar,
    pub map_num: c_uchar,
    pub land_mon_encounters: WildPokemonInfo,
    pub water_mon_encounters: WildPokemonInfo,
    pub rock_smash_encounters: WildPokemonInfo,
    pub fishing_encounters: WildPokemonInfo,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct WildPokemonInfoROM {
    pub encounter_rate: c_uchar,
    pub wild_pokemon_list_rom_ptr: c_uint,
}

#[derive(Default, Debug, Clone)]
#[repr(C)]
pub struct WildPokemonInfoFFI {
    pub encounter_rate: c_uchar,
    pub pokemon_count: usize,
    pub wild_pokemon_list: __IncompleteArrayField<WildPokemon>,
}

#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WildPokemonInfo {
    pub encounter_rate: u8,
    pub pokemon_count: usize,
    pub wild_pokemon_list: Vec<WildPokemon>,
}


#[derive(Default, Debug, Clone, Copy, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct WildPokemon {
    pub min_level: c_uchar,
    pub max_level: c_uchar,
    pub species: c_ushort,
}

impl WildPokemonHeaderROM {
    #[unsafe(no_mangle)]
    pub fn fill_header(buffer: &[u8], offset: usize) -> Self {
        let mut header = WildPokemonHeaderROM::default();
        let mut index: usize = offset;

        header.map_group = read_u8(buffer, index);
        index += 1;
        header.map_num = read_u8(buffer, index);
        index += 3; //account for the filler
        header.land_mon_enounters_rom_ptr = read_u32(buffer, index) & 0x7FFFFFF;
        index += 4;
        header.water_mon_encounters_rom_ptr = read_u32(buffer, index) & 0x7FFFFFF;
        index += 4;
        header.rock_smash_encounters_rom_ptr = read_u32(buffer, index) & 0x7FFFFFF;
        index += 4;
        header.fishing_encounters_rom_ptr =read_u32(buffer, index) & 0x7FFFFFF;
        
        header
    }
}

impl WildPokemonHeader {
    pub fn fill_head(header_rom: &WildPokemonHeaderROM, buffer: &[u8]) -> Self {
        let mut header = WildPokemonHeader::default();

        header.map_group = header_rom.map_group;
        header.map_num = header_rom.map_num;

        if header_rom.land_mon_enounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer, header_rom.land_mon_enounters_rom_ptr as usize);
            header.land_mon_encounters = WildPokemonInfo::fill_wild_pokemon_list(pokemon_info, buffer);
        }
        if header_rom.water_mon_encounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer,header_rom.water_mon_encounters_rom_ptr as usize);
            header.water_mon_encounters = WildPokemonInfo::fill_wild_pokemon_list(pokemon_info, buffer);
        }
        if header_rom.rock_smash_encounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer,header_rom.rock_smash_encounters_rom_ptr as usize);
            header.rock_smash_encounters = WildPokemonInfo::fill_wild_pokemon_list(pokemon_info, buffer);
        }
        if header_rom.fishing_encounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer,header_rom.fishing_encounters_rom_ptr as usize);
            header.fishing_encounters = WildPokemonInfo::fill_wild_pokemon_list(pokemon_info, buffer);
        }

        header
    }
}

impl WildPokemonHeaderFFI {
    #[unsafe(no_mangle)]
    pub fn fill_head(header_rom: &WildPokemonHeaderROM, buffer: &[u8]) -> Self {
        let mut header = WildPokemonHeaderFFI::default();

        header.map_group = header_rom.map_group;
        header.map_num = header_rom.map_num;
        
        if header_rom.land_mon_enounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer, header_rom.land_mon_enounters_rom_ptr as usize);

            let list = pokemon_info.get_pokemon_list(buffer);
            header.land_mon_encounters = unsafe { new_filled_wild_pokemon_info_ffi(list) };
        }
        if header_rom.water_mon_encounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer, header_rom.water_mon_encounters_rom_ptr as usize);
            let list = pokemon_info.get_pokemon_list(buffer);
            header.water_mon_encounters = unsafe { new_filled_wild_pokemon_info_ffi(list) };
        }
        if header_rom.rock_smash_encounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer, header_rom.rock_smash_encounters_rom_ptr as usize);
            let list = pokemon_info.get_pokemon_list(buffer);
            header.rock_smash_encounters = unsafe { new_filled_wild_pokemon_info_ffi(list) };
        }
        if header_rom.fishing_encounters_rom_ptr != 0 {
            let pokemon_info = WildPokemonInfoROM::fill_pokemon_info(buffer, header_rom.fishing_encounters_rom_ptr as usize);
            let list: Vec<WildPokemon> = pokemon_info.get_pokemon_list(buffer);
            header.fishing_encounters = unsafe { new_filled_wild_pokemon_info_ffi(list) };
        }

        header
    }
}

impl WildPokemon {
    pub fn fill_wild_pokemon(buffer: &[u8], offset: usize) -> Self {
        let mut index: usize = offset;
        let mut mon = WildPokemon::default();
        
        mon.min_level = read_u8(&buffer, index);
        index += 1;
        mon.max_level = read_u8(&buffer, index);
        index += 1;
        mon.species = read_u16(&buffer, index);

        if mon.min_level == 0x15 && mon.max_level == 0 {
            return WildPokemon::default();
        }

        mon
    }
}

impl WildPokemonInfoROM {
    /// buffer is the entire rom data, do not truncate it or the offsets will need to be changed.
    pub fn get_pokemon_list(&self, buffer: &[u8]) -> Vec<WildPokemon> {
        let mut list:Vec<WildPokemon> = Vec::new();
        let mut index: usize = 0;
        let endpointer = self.wild_pokemon_list_rom_ptr | 0x08000000;

        let wild_mon_start_ptr = self.wild_pokemon_list_rom_ptr;
        
        while read_u32(&buffer, wild_mon_start_ptr as usize + index) != endpointer {
            let poke_result = WildPokemon::fill_wild_pokemon(&buffer, wild_mon_start_ptr as usize + index);
            index += std::mem::size_of::<WildPokemon>();

            if !list.iter().any(|&list| list.species == poke_result.species)
                && poke_result != WildPokemon::default() && poke_result.max_level != 0 {
                    list.push(poke_result);
            }
        }

        list
    }

    pub fn fill_pokemon_info(buffer: &[u8], offset: usize) -> Self {
        let mut info = WildPokemonInfoROM::default();
        let mut index = offset;

        info.encounter_rate = read_u8(buffer, index);
        index += 4;
        info.wild_pokemon_list_rom_ptr = read_u32(buffer, index) & 0x07FFFFFF;

        info
    }
}

impl WildPokemonInfoFFI {
    pub fn fill_wild_pokemon_list(pokemon_info_rom_data: WildPokemonInfoROM, buffer: &[u8]) -> *mut Self {
        let wild_pokemon_list = pokemon_info_rom_data.get_pokemon_list(buffer);
        let len = wild_pokemon_list.len();
        let info = unsafe  { new_filled_wild_pokemon_info_ffi(wild_pokemon_list) };

        unsafe {
            (*info).encounter_rate = pokemon_info_rom_data.encounter_rate;
            (*info).pokemon_count = len;
        }

        info
    }
}

impl WildPokemonInfo {
    pub fn fill_wild_pokemon_list(pokemon_info_rom_data: WildPokemonInfoROM, buffer: &[u8]) -> Self {
        let wild_pokemon_list = pokemon_info_rom_data.get_pokemon_list(buffer);
        let len = wild_pokemon_list.len();
        Self {
            encounter_rate: pokemon_info_rom_data.encounter_rate,
            pokemon_count: len,
            wild_pokemon_list
        }
    }
}

impl Drop for WildPokemonHeaderFFI {
    fn drop(&mut self) {
        unsafe {
            if !self.land_mon_encounters.is_null() {
                drop_filled_wild_pokemon_info(self.land_mon_encounters);
            }
            if !self.fishing_encounters.is_null() {
                drop_filled_wild_pokemon_info(self.fishing_encounters);
            }
            if !self.rock_smash_encounters.is_null() {
                drop_filled_wild_pokemon_info(self.rock_smash_encounters);
            }
            if !self.water_mon_encounters.is_null() {
                drop_filled_wild_pokemon_info(self.water_mon_encounters);
            }
        }
    }
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct __IncompleteArrayField<T>(PhantomData<T>);

impl<T> __IncompleteArrayField<T> {
    pub fn new() -> Self {
        __IncompleteArrayField(PhantomData)
    }

    // convert to a raw pointer to access data
    pub unsafe fn as_ptr(&self) -> *const T {
        unsafe { std::mem::transmute(self) }
    }

    // convert to a mutable raw pointer 
    pub unsafe fn as_mut_ptr(&mut self) -> *mut T {
        unsafe { std::mem::transmute(self) }
    }

    // convert to a slice given a known length
    pub unsafe fn as_slice(&self, len: usize) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.as_ptr(), len) }
    }
}

// function to create a container with 'n' items
unsafe fn new_filled_wild_pokemon_info_ffi(list: Vec<WildPokemon>) -> *mut WildPokemonInfoFFI {
    let n = list.len();

    let layout = Layout::from_size_align(
        std::mem::size_of::<WildPokemonInfo>() + n * std::mem::size_of::<WildPokemon>(),
        std::mem::align_of::<WildPokemon>(),
    ).unwrap();

    let ptr = unsafe { alloc(layout) as *mut WildPokemonInfoFFI };
    if ptr.is_null() { std::alloc::handle_alloc_error(layout); }

    unsafe { (*ptr).pokemon_count = n; }

    // initialize items
    let pokemon_ptr = unsafe { &(*ptr).wild_pokemon_list as *const _ as *mut WildPokemon };
    for (i, wild_pokemon) in list.iter().enumerate().take(n) {
        unsafe { pokemon_ptr.add(i).write(*wild_pokemon); }
    }

    ptr
}

pub unsafe fn get_wild_pokemon_vector_from_ptr_ffi(ptr: *mut WildPokemonInfoFFI) -> Vec<WildPokemon> {
    let mut pokemon: Vec<WildPokemon> = Vec::new();
    if ptr.is_null() {
        eprintln!("invalid pointer");
        return pokemon;
    }

    let count = unsafe { (*ptr).pokemon_count };

    let pokemon_ptr = unsafe { &(*ptr).wild_pokemon_list as *const _ as *const WildPokemon };

    for i in 0..count {
        let pokemon_entry = unsafe { pokemon_ptr.add(i).read() };
        pokemon.push(pokemon_entry);
    }

    pokemon
}

unsafe fn drop_filled_wild_pokemon_info(ptr: *mut WildPokemonInfoFFI) {
    if ptr.is_null() {
        return;
    }

    let layout = unsafe { Layout::from_size_align(
        std::mem::size_of::<WildPokemonInfo>() + (*ptr).pokemon_count * std::mem::size_of::<WildPokemon>(),
        std::mem::align_of::<WildPokemon>(),
    ).unwrap() };
    unsafe { dealloc(ptr as *mut c_uchar, layout); }
}

// #[unsafe(no_mangle)] ---todo! update for FFI
pub fn get_all_pokemon_headers_from_rom(buffer: &[u8], offset: usize) -> Vec<WildPokemonHeaderROM> {
    let mut wild_header: Vec<WildPokemonHeaderROM> = Vec::new();
    let mut index: usize = 0;
    let header_size = std::mem::size_of::<WildPokemonHeaderROM>();

    while read_u16(buffer, offset + index) != 0xFFFF {
        if offset + index > buffer.len() {
            eprintln!("Error could not fill the pokemon headers");
        }

        wild_header.push(WildPokemonHeaderROM::fill_header(buffer, offset + index));
        index += header_size;
    }

    wild_header
}

pub fn fill_static_pokemon_header_list(buffer: &[u8], offset: usize) {
    let list = get_all_pokemon_headers_from_rom(buffer, offset);
    WILD_POKEMON_HEADERS.get_or_init(|| list);
}

pub fn get_pokemon_header_list() -> &'static Vec<WildPokemonHeaderROM> {
    WILD_POKEMON_HEADERS.get().expect("headers not initialized")
}