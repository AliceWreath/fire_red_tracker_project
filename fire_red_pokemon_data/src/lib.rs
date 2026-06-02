//! # FireRed Pokemon Data
//!
//! ROM parsing for wild Pokémon encounter headers and tables.
//!
//! # Data layout
//!
//! Wild encounter data in the FireRed ROM is organized as a flat array of
//! [`WildPokemonHeaderROM`] structs terminated by a `0xFFFF` sentinel. Each
//! header stores ROM pointers to up to four encounter tables (land, water,
//! rock smash, fishing), each of which is a [`WildPokemonInfoROM`] containing
//! an encounter rate and a pointer to the actual [`WildPokemon`] list.
//!
//! # Safe vs. FFI types
//!
//! Three parallel type families exist:
//!
//! | Suffix  | Description                                              |
//! |---------|----------------------------------------------------------|
//! | `ROM`   | Raw ROM representation with integer pointer fields.      |
//! | (none)  | Safe Rust representation using owned `Vec` collections.  |
//! | `FFI`   | C-ABI representation using manually allocated raw ptrs.  |
//!
//! The `FFI` types exist for interop with C/C++ callers. Most Rust code should
//! use the plain (non-suffix) types.

use std::alloc::{alloc, dealloc, Layout};
use std::ffi::{c_uchar, c_uint, c_ushort};
use std::marker::PhantomData;
use std::sync::OnceLock;
use fire_red_get_values::{read_u16, read_u8, read_u32};

// ---------------------------------------------------------------------------
// Global cache
// ---------------------------------------------------------------------------

/// Global cache of all wild pokemon encounter headers loaded from the ROM.
///
/// Initialized once via [`fill_static_pokemon_header_list`] and shared
/// immutably for the lifetime of the program.
static WILD_POKEMON_HEADERS: OnceLock<Vec<WildPokemonHeaderROM>> = OnceLock::new();

// ---------------------------------------------------------------------------
// ROM types
// ---------------------------------------------------------------------------

/// Raw ROM representation of a wild encounter header.
///
/// Stores ROM pointers as integer offsets. Each header corresponds to a single
/// map and contains pointers to up to four encounter tables.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct WildPokemonHeaderROM {
    /// Map group index.
    pub map_group: c_uchar,
    /// Map name index within the group.
    pub map_num: c_uchar,
    /// Padding bytes (unused by the game).
    pub filler: [c_uchar; 2],
    /// ROM pointer to the land encounter table, or 0 if none.
    pub land_mon_enounters_rom_ptr: c_uint,
    /// ROM pointer to the water encounter table, or 0 if none.
    pub water_mon_encounters_rom_ptr: c_uint,
    /// ROM pointer to the rock smash encounter table, or 0 if none.
    pub rock_smash_encounters_rom_ptr: c_uint,
    /// ROM pointer to the fishing encounter table, or 0 if none.
    pub fishing_encounters_rom_ptr: c_uint,
}

/// Raw ROM representation of a wild encounter table.
///
/// Contains the encounter rate and a pointer to the list of [`WildPokemon`]
/// entries.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct WildPokemonInfoROM {
    /// Encounter rate (probability of a wild pokemon appearing per step).
    pub encounter_rate: c_uchar,
    /// ROM pointer to the encounter list.
    pub wild_pokemon_list_rom_ptr: c_uint,
}

// ---------------------------------------------------------------------------
// Safe Rust types
// ---------------------------------------------------------------------------

/// Safe Rust representation of a wild encounter header.
///
/// Uses owned Rust collections instead of raw pointers.
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

/// Safe Rust representation of a wild encounter table.
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WildPokemonInfo {
    /// Encounter rate for the area.
    pub encounter_rate: u8,
    /// Number of unique pokemon entries.
    pub pokemon_count: usize,
    /// List of encounterable pokemon, deduplicated by species.
    pub wild_pokemon_list: Vec<WildPokemon>,
}

/// A single wild pokemon encounter entry.
#[derive(Default, Debug, Clone, Copy, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct WildPokemon {
    /// Minimum encounter level.
    pub min_level: c_uchar,
    /// Maximum encounter level.
    pub max_level: c_uchar,
    /// National Pokédex species ID.
    pub species: c_ushort,
}

// ---------------------------------------------------------------------------
// FFI types
// ---------------------------------------------------------------------------

/// FFI-safe wild encounter header.
///
/// Stores pointers to dynamically allocated encounter lists suitable for use
/// from C-compatible languages. Memory is managed by [`WildPokemonHeaderFFI`]'s
/// [`Drop`] implementation.
#[derive(Default, Debug, Clone)]
#[repr(C)]
pub struct WildPokemonHeaderFFI {
    pub map_group: c_uchar,
    pub map_num: c_uchar,
    /// Heap-allocated land encounter list, or null if none.
    pub land_mon_encounters: *mut WildPokemonInfoFFI,
    /// Heap-allocated water encounter list, or null if none.
    pub water_mon_encounters: *mut WildPokemonInfoFFI,
    /// Heap-allocated rock smash encounter list, or null if none.
    pub rock_smash_encounters: *mut WildPokemonInfoFFI,
    /// Heap-allocated fishing encounter list, or null if none.
    pub fishing_encounters: *mut WildPokemonInfoFFI,
}

/// FFI-safe encounter table.
///
/// Uses a flexible array member pattern: the `wild_pokemon_list` field is
/// a zero-sized marker; the actual `WildPokemon` entries are laid out
/// immediately after the struct in the same allocation.
#[derive(Default, Debug, Clone)]
#[repr(C)]
pub struct WildPokemonInfoFFI {
    /// Encounter rate for the area.
    pub encounter_rate: c_uchar,
    /// Number of `WildPokemon` entries in the trailing array.
    pub pokemon_count: usize,
    /// Zero-sized marker for the trailing flexible array.
    pub wild_pokemon_list: __IncompleteArrayField<WildPokemon>,
}

/// Flexible array member helper for FFI.
///
/// Mimics the incomplete array fields found in C structs. The actual data
/// lives in memory immediately following the struct. This type is zero-sized
/// and acts only as a typed anchor for pointer arithmetic.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct __IncompleteArrayField<T>(PhantomData<T>);

impl<T> __IncompleteArrayField<T> {
    /// Creates a new zero-sized incomplete array field marker.
    pub fn new() -> Self {
        __IncompleteArrayField(PhantomData)
    }

    /// Returns a raw immutable pointer to the start of the trailing array.
    ///
    /// # Safety
    ///
    /// The caller must ensure the backing allocation is valid and contains
    /// at least as many `T` elements as will be accessed.
    pub unsafe fn as_ptr(&self) -> *const T {
        self as *const __IncompleteArrayField<T> as *const T
    }

    /// Returns a raw mutable pointer to the start of the trailing array.
    ///
    /// # Safety
    ///
    /// Same requirements as [`as_ptr`](Self::as_ptr).
    pub unsafe fn as_mut_ptr(&mut self) -> *mut T {
        self as *mut __IncompleteArrayField<T> as *mut T
    }

    /// Returns a slice over `len` elements of the trailing array.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The backing allocation is valid.
    /// - At least `len` elements have been initialized.
    pub unsafe fn as_slice(&self, len: usize) -> &[T] {
        unsafe { std::slice::from_raw_parts(self.as_ptr(), len) }
    }
}

// ---------------------------------------------------------------------------
// ROM parsing implementations
// ---------------------------------------------------------------------------

impl WildPokemonHeaderROM {
    /// Reads a wild encounter header from the ROM buffer at `offset`.
    pub fn fill_header(buffer: &[u8], offset: usize) -> Self {
        // layout: u8 map_group, u8 map_num, u16 filler, then four u32 pointers
        WildPokemonHeaderROM {
            map_group:                    read_u8(buffer, offset),
            map_num:                      read_u8(buffer, offset + 1),
            filler:                       Default::default(),
            land_mon_enounters_rom_ptr:   read_u32(buffer, offset + 4)  & 0x07FFFFFF,
            water_mon_encounters_rom_ptr:  read_u32(buffer, offset + 8)  & 0x07FFFFFF,
            rock_smash_encounters_rom_ptr: read_u32(buffer, offset + 12) & 0x07FFFFFF,
            fishing_encounters_rom_ptr:    read_u32(buffer, offset + 16) & 0x07FFFFFF,
        }
    }
}

impl WildPokemonInfoROM {
    /// Reads encounter table metadata from the ROM buffer at `offset`.
    pub fn fill_pokemon_info(buffer: &[u8], offset: usize) -> Self {
        // layout: u8 encounter_rate, u8[3] pad, u32 ptr
        WildPokemonInfoROM {
            encounter_rate:            read_u8(buffer, offset),
            wild_pokemon_list_rom_ptr: read_u32(buffer, offset + 4) & 0x07FFFFFF,
        }
    }

    /// Reads the encounter list for this table from the ROM buffer.
    ///
    /// Walks the list of [`WildPokemon`] entries until it encounters the
    /// sentinel (a 4-byte value equal to the list's own ROM address ORed with
    /// `0x08000000`), deduplicating entries by species. Empty or zero-level
    /// entries are discarded.
    pub fn get_pokemon_list(&self, buffer: &[u8]) -> Vec<WildPokemon> {
        let mut list: Vec<WildPokemon> = Vec::new();
        let start = self.wild_pokemon_list_rom_ptr as usize;

        // The sentinel value is the pointer itself with the ROM bank byte set.
        let sentinel = self.wild_pokemon_list_rom_ptr | 0x08000000;
        let entry_size = std::mem::size_of::<WildPokemon>();
        let mut index = 0;
        // FireRed has at most 13 slots per encounter table. 200 is a safe ceiling
        // that prevents an infinite loop when the ROM is corrupt or the pointer
        // is wrong and the sentinel is never reached.
        const MAX_ENTRIES: usize = 200;

        loop {
            if index / entry_size >= MAX_ENTRIES {
                break;
            }
            // Stop when we read back the sentinel value.
            if read_u32(buffer, start + index) == sentinel {
                break;
            }

            let mon = WildPokemon::fill_wild_pokemon(buffer, start + index);
            index += entry_size;

            // Discard empty or malformed entries.
            if mon == WildPokemon::default() || mon.max_level == 0 {
                continue;
            }

            // Deduplicate by species.
            if !list.iter().any(|m| m.species == mon.species) {
                list.push(mon);
            }
        }

        list
    }
}

impl WildPokemon {
    /// Reads a single wild pokemon entry from the ROM buffer at `offset`.
    ///
    /// Returns a default (zeroed) entry if the data looks malformed
    /// (min_level `0x15` with max_level `0` is a known sentinel pattern).
    pub fn fill_wild_pokemon(buffer: &[u8], offset: usize) -> Self {
        let mon = WildPokemon {
            min_level: read_u8(buffer, offset),
            max_level: read_u8(buffer, offset + 1),
            species:   read_u16(buffer, offset + 2),
        };
        // Known malformed-entry sentinel.
        if mon.min_level == 0x15 && mon.max_level == 0 {
            return WildPokemon::default();
        }
        mon
    }
}

impl WildPokemonInfo {
    /// Builds a [`WildPokemonInfo`] from parsed ROM data.
    pub fn fill_wild_pokemon_list(pokemon_info: WildPokemonInfoROM, buffer: &[u8]) -> Self {
        let wild_pokemon_list = pokemon_info.get_pokemon_list(buffer);
        let pokemon_count = wild_pokemon_list.len();
        Self {
            encounter_rate: pokemon_info.encounter_rate,
            pokemon_count,
            wild_pokemon_list,
        }
    }
}

// ---------------------------------------------------------------------------
// Safe Rust header builder
// ---------------------------------------------------------------------------

impl WildPokemonHeader {
    /// Builds a [`WildPokemonHeader`] from a [`WildPokemonHeaderROM`] and the
    /// full ROM buffer.
    ///
    /// Only populates encounter tables whose ROM pointer is non-zero.
    pub fn fill_head(header_rom: &WildPokemonHeaderROM, buffer: &[u8]) -> Self {
        let mut header = WildPokemonHeader {
            map_group: header_rom.map_group,
            map_num:   header_rom.map_num,
            ..Default::default()
        };

        let fill = |ptr: c_uint, dest: &mut WildPokemonInfo| {
            if ptr != 0 {
                let info = WildPokemonInfoROM::fill_pokemon_info(buffer, ptr as usize);
                *dest = WildPokemonInfo::fill_wild_pokemon_list(info, buffer);
            }
        };

        fill(header_rom.land_mon_enounters_rom_ptr,    &mut header.land_mon_encounters);
        fill(header_rom.water_mon_encounters_rom_ptr,  &mut header.water_mon_encounters);
        fill(header_rom.rock_smash_encounters_rom_ptr, &mut header.rock_smash_encounters);
        fill(header_rom.fishing_encounters_rom_ptr,    &mut header.fishing_encounters);

        header
    }
}

// ---------------------------------------------------------------------------
// FFI header builder
// ---------------------------------------------------------------------------

impl WildPokemonHeaderFFI {
    /// Builds a [`WildPokemonHeaderFFI`] from a [`WildPokemonHeaderROM`] and
    /// the full ROM buffer.
    ///
    /// Allocates heap memory for each encounter table. The caller is
    /// responsible for ensuring the returned value is eventually dropped
    /// (the [`Drop`] impl handles deallocation).
    pub fn fill_head(header_rom: &WildPokemonHeaderROM, buffer: &[u8]) -> Self {
        let mut header = WildPokemonHeaderFFI {
            map_group: header_rom.map_group,
            map_num:   header_rom.map_num,
            ..Default::default()
        };

        let fill = |ptr: c_uint, dest: &mut *mut WildPokemonInfoFFI| {
            if ptr != 0 {
                let info = WildPokemonInfoROM::fill_pokemon_info(buffer, ptr as usize);
                let list = info.get_pokemon_list(buffer);
                *dest = unsafe { alloc_wild_pokemon_info_ffi(info.encounter_rate, list) };
            }
        };

        fill(header_rom.land_mon_enounters_rom_ptr,    &mut header.land_mon_encounters);
        fill(header_rom.water_mon_encounters_rom_ptr,  &mut header.water_mon_encounters);
        fill(header_rom.rock_smash_encounters_rom_ptr, &mut header.rock_smash_encounters);
        fill(header_rom.fishing_encounters_rom_ptr,    &mut header.fishing_encounters);

        header
    }
}

impl WildPokemonInfoFFI {
    /// Allocates and returns a pointer to a [`WildPokemonInfoFFI`] populated
    /// from parsed ROM data.
    ///
    /// # Safety
    ///
    /// The returned pointer must eventually be freed via
    /// [`dealloc_wild_pokemon_info_ffi`].
    pub fn fill_wild_pokemon_list(pokemon_info: WildPokemonInfoROM, buffer: &[u8]) -> *mut Self {
        let list = pokemon_info.get_pokemon_list(buffer);
        unsafe { alloc_wild_pokemon_info_ffi(pokemon_info.encounter_rate, list) }
    }
}

impl Drop for WildPokemonHeaderFFI {
    fn drop(&mut self) {
        unsafe {
            for ptr in [
                self.land_mon_encounters,
                self.water_mon_encounters,
                self.rock_smash_encounters,
                self.fishing_encounters,
            ] {
                if !ptr.is_null() {
                    dealloc_wild_pokemon_info_ffi(ptr);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FFI heap helpers
// ---------------------------------------------------------------------------

/// Allocates a [`WildPokemonInfoFFI`] with `encounter_rate` and a trailing
/// array of `list.len()` [`WildPokemon`] entries.
///
/// # Safety
///
/// The returned pointer must be freed with [`dealloc_wild_pokemon_info_ffi`].
unsafe fn alloc_wild_pokemon_info_ffi(
    encounter_rate: c_uchar,
    list: Vec<WildPokemon>,
) -> *mut WildPokemonInfoFFI {
    let n = list.len();
    let layout = Layout::from_size_align(
        std::mem::size_of::<WildPokemonInfoFFI>() + n * std::mem::size_of::<WildPokemon>(),
        std::mem::align_of::<WildPokemonInfoFFI>(),
    )
    .unwrap();

    let ptr = unsafe { alloc(layout) as *mut WildPokemonInfoFFI };
    if ptr.is_null() {
        std::alloc::handle_alloc_error(layout);
    }

    unsafe {
        (*ptr).encounter_rate = encounter_rate;
        (*ptr).pokemon_count  = n;
    }

    let pokemon_ptr = unsafe {
        &(*ptr).wild_pokemon_list as *const _ as *mut WildPokemon
    };
    for (i, mon) in list.iter().enumerate() {
        unsafe { pokemon_ptr.add(i).write(*mon) };
    }

    ptr
}

/// Frees memory allocated by [`alloc_wild_pokemon_info_ffi`].
///
/// # Safety
///
/// `ptr` must have been allocated by [`alloc_wild_pokemon_info_ffi`] and must
/// not have already been freed.
unsafe fn dealloc_wild_pokemon_info_ffi(ptr: *mut WildPokemonInfoFFI) {
    if ptr.is_null() {
        return;
    }
    let layout = Layout::from_size_align(
        std::mem::size_of::<WildPokemonInfoFFI>()
            + unsafe { (*ptr).pokemon_count } * std::mem::size_of::<WildPokemon>(),
        std::mem::align_of::<WildPokemonInfoFFI>(),
    )
    .unwrap();
    unsafe { dealloc(ptr as *mut c_uchar, layout) };
}

/// Converts an FFI encounter list pointer into a Rust `Vec`.
///
/// Returns an empty `Vec` if `ptr` is null.
///
/// # Safety
///
/// `ptr` must be a valid pointer allocated by [`alloc_wild_pokemon_info_ffi`].
pub unsafe fn get_wild_pokemon_vector_from_ptr_ffi(ptr: *mut WildPokemonInfoFFI) -> Vec<WildPokemon> {
    if ptr.is_null() {
        eprintln!("get_wild_pokemon_vector_from_ptr_ffi: null pointer");
        return Vec::new();
    }

    let count       = unsafe { (*ptr).pokemon_count };
    let pokemon_ptr = unsafe { &(*ptr).wild_pokemon_list as *const _ as *const WildPokemon };

    (0..count).map(|i| unsafe { pokemon_ptr.add(i).read() }).collect()
}

// ---------------------------------------------------------------------------
// Static header cache
// ---------------------------------------------------------------------------

/// Reads all wild encounter headers from the ROM starting at `offset`.
///
/// Scans until a `0xFFFF` sentinel entry is reached.
pub fn get_all_pokemon_headers_from_rom(buffer: &[u8], offset: usize) -> Vec<WildPokemonHeaderROM> {
    let header_size = std::mem::size_of::<WildPokemonHeaderROM>();
    let mut headers = Vec::new();
    let mut index = 0;

    while offset + index + 2 <= buffer.len()
        && read_u16(buffer, offset + index) != 0xFFFF
    {
        if offset + index >= buffer.len() {
            eprintln!("Overran ROM buffer while reading pokemon headers.");
            break;
        }
        headers.push(WildPokemonHeaderROM::fill_header(buffer, offset + index));
        index += header_size;
    }

    headers
}

/// Initializes the global wild encounter header cache from ROM data.
///
/// Has no effect if the cache has already been initialized.
///
/// # Arguments
///
/// * `buffer` — Full ROM byte slice.
/// * `offset` — Byte offset of the encounter header table within the ROM.
pub fn fill_static_pokemon_header_list(buffer: &[u8], offset: usize) {
    WILD_POKEMON_HEADERS.get_or_init(|| get_all_pokemon_headers_from_rom(buffer, offset));
}

/// Returns a reference to the cached wild encounter header list.
///
/// # Panics
///
/// Panics if [`fill_static_pokemon_header_list`] has not been called first.
pub fn get_pokemon_header_list() -> &'static Vec<WildPokemonHeaderROM> {
    WILD_POKEMON_HEADERS.get().expect("wild pokemon header list not initialized — call fill_static_pokemon_header_list first")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Builds a minimal 20-byte WildPokemonHeaderROM buffer at offset 0.
    fn header_buf(group: u8, num: u8, land: u32, water: u32, rock: u32, fish: u32) -> Vec<u8> {
        let mut b = vec![group, num, 0, 0];
        b.extend_from_slice(&land.to_le_bytes());
        b.extend_from_slice(&water.to_le_bytes());
        b.extend_from_slice(&rock.to_le_bytes());
        b.extend_from_slice(&fish.to_le_bytes());
        b
    }

    // ── WildPokemonHeaderROM::fill_header ─────────────────────────────────────

    #[test]
    fn fill_header_map_ids() {
        let buf = header_buf(3, 0x13, 0, 0, 0, 0);
        let h = WildPokemonHeaderROM::fill_header(&buf, 0);
        assert_eq!(h.map_group, 3);
        assert_eq!(h.map_num, 0x13);
    }

    #[test]
    fn fill_header_pointer_mask() {
        // ROM pointers have the 0x08000000 bank byte stripped (mask 0x07FFFFFF).
        let buf = header_buf(1, 0, 0x0812_3456, 0x0800_0001, 0, 0);
        let h = WildPokemonHeaderROM::fill_header(&buf, 0);
        assert_eq!(h.land_mon_enounters_rom_ptr,  0x0012_3456);
        assert_eq!(h.water_mon_encounters_rom_ptr, 0x0000_0001);
        assert_eq!(h.rock_smash_encounters_rom_ptr, 0);
        assert_eq!(h.fishing_encounters_rom_ptr,    0);
    }

    #[test]
    fn fill_header_with_nonzero_offset() {
        let mut buf = vec![0u8; 8]; // 8 bytes of padding
        buf.extend(header_buf(2, 5, 0, 0, 0, 0));
        let h = WildPokemonHeaderROM::fill_header(&buf, 8);
        assert_eq!(h.map_group, 2);
        assert_eq!(h.map_num, 5);
    }

    // ── WildPokemonInfoROM::fill_pokemon_info ─────────────────────────────────

    #[test]
    fn fill_pokemon_info_rate_and_ptr() {
        // layout: u8 rate, u8[3] pad, u32 ptr
        let mut buf = vec![25u8, 0, 0, 0];
        buf.extend_from_slice(&(0x0812_0000u32).to_le_bytes());
        let info = WildPokemonInfoROM::fill_pokemon_info(&buf, 0);
        assert_eq!(info.encounter_rate, 25);
        assert_eq!(info.wild_pokemon_list_rom_ptr, 0x0012_0000);
    }

    #[test]
    fn fill_pokemon_info_zero_rate() {
        let buf = vec![0u8; 8];
        let info = WildPokemonInfoROM::fill_pokemon_info(&buf, 0);
        assert_eq!(info.encounter_rate, 0);
        assert_eq!(info.wild_pokemon_list_rom_ptr, 0);
    }

    // ── WildPokemon::fill_wild_pokemon ────────────────────────────────────────

    #[test]
    fn fill_wild_pokemon_normal_entry() {
        // min=5, max=10, species=25 (Pikachu), little-endian
        let buf = [5u8, 10, 25, 0];
        let mon = WildPokemon::fill_wild_pokemon(&buf, 0);
        assert_eq!(mon.min_level, 5);
        assert_eq!(mon.max_level, 10);
        assert_eq!(mon.species, 25);
    }

    #[test]
    fn fill_wild_pokemon_sentinel_returns_default() {
        // Known malformed-entry sentinel: min_level=0x15, max_level=0.
        let buf = [0x15u8, 0x00, 0x01, 0x00];
        assert_eq!(WildPokemon::fill_wild_pokemon(&buf, 0), WildPokemon::default());
    }

    #[test]
    fn fill_wild_pokemon_with_offset() {
        let buf = [0u8, 0, 3, 7, 0x84, 0x00]; // 2 padding, then min=3, max=7, species=132 (Ditto)
        let mon = WildPokemon::fill_wild_pokemon(&buf, 2);
        assert_eq!(mon.min_level, 3);
        assert_eq!(mon.max_level, 7);
        assert_eq!(mon.species, 132);
    }

    #[test]
    fn fill_wild_pokemon_species_endianness() {
        // species=0x0101 stored little-endian as [0x01, 0x01]
        let buf = [1u8, 5, 0x01, 0x01];
        let mon = WildPokemon::fill_wild_pokemon(&buf, 0);
        assert_eq!(mon.species, 0x0101);
    }
}