//! # Fire Red Badge Monitor
//!
//! Reads the player's badge flags from SaveBlock1 and exposes which badges
//! have been obtained, as well as information about the next gym leader.
//!
//! # Memory layout
//!
//! SaveBlock1 is dynamically allocated — its base address is not fixed. At
//! runtime the GBA stores a pointer to SaveBlock1 at `SAVE_BLOCK_1_PTR`
//! (`0x03005008`) in IWRAM. Reading badge state therefore requires two steps:
//!
//! 1. Read 4 bytes from the IWRAM snapshot at the IWRAM offset of
//!    `SAVE_BLOCK_1_PTR` to obtain the SaveBlock1 base address.
//! 2. Read the flags array from the EWRAM snapshot at
//!    `(base - EWRAM_BASE) + FLAGS_OFFSET`.
//!
//! The flags array starts at offset `0x0EE0` within SaveBlock1. Badge flags
//! occupy flag indices `0x820`–`0x827`, one bit per badge:
//!
//! | Index  | Badge         | Leader    |
//! |--------|---------------|-----------|
//! | 0x820  | Boulder Badge | Brock     |
//! | 0x821  | Cascade Badge | Misty     |
//! | 0x822  | Thunder Badge | Lt. Surge |
//! | 0x823  | Rainbow Badge | Erika     |
//! | 0x824  | Soul Badge    | Koga      |
//! | 0x825  | Marsh Badge   | Sabrina   |
//! | 0x826  | Volcano Badge | Blaine    |
//! | 0x827  | Earth Badge   | Giovanni  |
//!
//! Each flag index maps to: `byte = index / 8`, `bit = index % 8` within the
//! flags array.
//!
//! # FFI
//!
//! C-compatible wrappers are provided for all public types and functions.
//! The FFI surface uses `#[repr(C)]` structs and raw pointers. Strings are
//! returned as null-terminated `*mut c_char` allocated on the Rust heap;
//! callers must free them with [`badge_free_string`]. The [`BadgeStateFFI`]
//! struct must be freed with [`badge_state_free`] after use.
//!
//! ## Example (C)
//!
//! ```c
//! BadgeStateFFI *state = badge_read_state();
//! if (state) {
//!     printf("Badges: %d/8\n", badge_state_count(state));
//!     if (state->has_next_gym) {
//!         char *leader = badge_gym_leader(state);
//!         printf("Next: %s\n", leader);
//!         badge_free_string(leader);
//!     }
//!     badge_state_free(state);
//! }
//! ```

use std::ffi::{CString, c_char};
use std::os::raw::c_uchar;

// ---------------------------------------------------------------------------
// Address constants
// ---------------------------------------------------------------------------

/// Base address of IWRAM in the GBA address space.
const IWRAM_BASE: usize = 0x03000000;

/// Base address of EWRAM in the GBA address space.
const EWRAM_BASE: usize = 0x02000000;

/// IWRAM address of the pointer to SaveBlock1.
///
/// Dereferencing this 4-byte little-endian pointer yields the runtime base
/// address of SaveBlock1, which lies somewhere in EWRAM.
const SAVE_BLOCK_1_PTR: usize = 0x03005008;

/// Byte offset of the flags array within SaveBlock1.
const FLAGS_OFFSET: usize = 0x0EE0;

/// Flag index of the first badge (Boulder Badge / Brock).
const BADGE_FLAG_START: usize = 0x820;

/// Total number of badges.
const NUM_BADGES: usize = 8;

// ---------------------------------------------------------------------------
// Offset helpers
// ---------------------------------------------------------------------------

/// Converts an absolute GBA IWRAM address to a byte offset within the IWRAM
/// snapshot buffer.
#[inline]
fn iwram_offset(addr: usize) -> usize {
    debug_assert!(addr >= IWRAM_BASE, "address 0x{:08X} is below IWRAM_BASE", addr);
    addr - IWRAM_BASE
}

/// Converts an absolute GBA EWRAM address to a byte offset within the EWRAM
/// snapshot buffer.
#[inline]
fn ewram_offset(addr: usize) -> usize {
    debug_assert!(addr >= EWRAM_BASE, "address 0x{:08X} is below EWRAM_BASE", addr);
    addr - EWRAM_BASE
}

// ---------------------------------------------------------------------------
// Rust types
// ---------------------------------------------------------------------------

/// The full badge state for the current player.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BadgeState {
    /// One entry per badge, in gym order. `true` = obtained.
    pub badges: [bool; NUM_BADGES],

    /// The next gym leader the player hasn't beaten yet, or `None` if all
    /// badges have been obtained.
    pub next_gym: Option<GymInfo>,
}

impl BadgeState {
    /// Returns how many badges the player currently holds.
    pub fn count(&self) -> usize {
        self.badges.iter().filter(|&&b| b).count()
    }

    /// Returns `true` if all 8 badges have been obtained.
    pub fn all_obtained(&self) -> bool {
        self.badges.iter().all(|&b| b)
    }
}

/// Information about a single gym leader.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GymInfo {
    /// Gym leader name.
    pub leader: String,
    /// City the gym is located in.
    pub city: String,
    /// Name of the badge awarded on victory.
    pub badge: String,
    /// Highest level pokemon on the leader's team in FireRed.
    pub max_level: u8,
}

// ---------------------------------------------------------------------------
// FFI types
// ---------------------------------------------------------------------------

/// C-ABI-compatible badge state.
///
/// All string fields are heap-allocated null-terminated C strings. After use,
/// the entire struct must be freed with [`badge_state_free`] — do not free
/// individual fields separately.
///
/// # Layout
///
/// ```c
/// typedef struct {
///     uint8_t badges[8];   // 1 = obtained, 0 = not obtained, gym order
///     int     badge_count; // number of obtained badges (0–8)
///     int     has_next_gym;// 1 if next_gym fields are valid, 0 otherwise
///     char   *next_leader; // gym leader name, or NULL
///     char   *next_city;   // city name, or NULL
///     char   *next_badge;  // badge name, or NULL
///     uint8_t next_max_level; // highest level on next leader's team
/// } BadgeStateFFI;
/// ```
#[repr(C)]
pub struct BadgeStateFFI {
    /// Badge obtained flags in gym order (1 = obtained, 0 = not yet).
    pub badges: [c_uchar; NUM_BADGES],
    /// Number of badges obtained (0–8).
    pub badge_count: i32,
    /// `1` if the `next_*` fields are valid, `0` if all badges are obtained.
    pub has_next_gym: i32,
    /// Null-terminated gym leader name, or null if `has_next_gym` is 0.
    pub next_leader: *mut c_char,
    /// Null-terminated city name, or null if `has_next_gym` is 0.
    pub next_city: *mut c_char,
    /// Null-terminated badge name, or null if `has_next_gym` is 0.
    pub next_badge: *mut c_char,
    /// Highest level pokemon on the next gym leader's team, or 0.
    pub next_max_level: c_uchar,
}

// ---------------------------------------------------------------------------
// Gym leader table
// ---------------------------------------------------------------------------

/// Static table of all 8 Kanto gym leaders in order.
fn gym_leaders() -> [GymInfo; NUM_BADGES] {
    [
        GymInfo { leader: "Brock".into(),     city: "Pewter City".into(),     badge: "Boulder Badge".into(), max_level: 14 },
        GymInfo { leader: "Misty".into(),     city: "Cerulean City".into(),   badge: "Cascade Badge".into(), max_level: 21 },
        GymInfo { leader: "Lt. Surge".into(), city: "Vermilion City".into(),  badge: "Thunder Badge".into(), max_level: 24 },
        GymInfo { leader: "Erika".into(),     city: "Celadon City".into(),    badge: "Rainbow Badge".into(), max_level: 29 },
        GymInfo { leader: "Koga".into(),      city: "Fuchsia City".into(),    badge: "Soul Badge".into(),    max_level: 43 },
        GymInfo { leader: "Sabrina".into(),   city: "Saffron City".into(),    badge: "Marsh Badge".into(),   max_level: 50 },
        GymInfo { leader: "Blaine".into(),    city: "Cinnabar Island".into(), badge: "Volcano Badge".into(), max_level: 54 },
        GymInfo { leader: "Giovanni".into(),  city: "Viridian City".into(),   badge: "Earth Badge".into(),   max_level: 55 },
    ]
}

// ---------------------------------------------------------------------------
// Internal helper
// ---------------------------------------------------------------------------

/// Converts a Rust `String` to a heap-allocated null-terminated C string.
///
/// The caller is responsible for freeing the returned pointer via
/// [`badge_free_string`]. Returns a null pointer if the string contains
/// interior null bytes (which should never happen for our data).
fn to_c_string(s: &str) -> *mut c_char {
    CString::new(s).map(|cs| cs.into_raw()).unwrap_or(std::ptr::null_mut())
}

// ---------------------------------------------------------------------------
// Public Rust API
// ---------------------------------------------------------------------------

/// Reads the current badge state from the IWRAM and EWRAM snapshots.
///
/// # How it works
///
/// 1. Reads 4 bytes from the IWRAM snapshot at the offset of
///    [`SAVE_BLOCK_1_PTR`] to obtain the runtime SaveBlock1 base address.
/// 2. Validates that the resolved address falls within EWRAM.
/// 3. Reads 2 bytes covering all 8 badge flag bits in a single slice.
///
/// # Returns
///
/// `None` if either snapshot is unpopulated, the pointer is out of range,
/// or the resolved address falls outside EWRAM.
pub fn read_badge_state() -> Option<BadgeState> {
    let iwram = fire_red_memory::get_iwram();
    let ewram = fire_red_memory::get_ewram();

    // Step 1: read the SaveBlock1 pointer from IWRAM.
    let ptr_offset = iwram_offset(SAVE_BLOCK_1_PTR);
    if iwram.len() < ptr_offset + 4 {
        return None;
    }
    let save_block_base = u32::from_le_bytes([
        iwram[ptr_offset],
        iwram[ptr_offset + 1],
        iwram[ptr_offset + 2],
        iwram[ptr_offset + 3],
    ]) as usize;

    // Step 2: validate the pointer falls within EWRAM.
    if save_block_base < EWRAM_BASE || save_block_base >= EWRAM_BASE + ewram.len() {
        eprintln!(
            "SaveBlock1 pointer 0x{:08X} is outside EWRAM — snapshot may not be ready.",
            save_block_base
        );
        return None;
    }

    // Step 3: locate the two badge flag bytes.
    // Badge flags 0x820–0x827 occupy bits 0–7 of the two bytes at
    // flags_array[0x820 / 8] = flags_array[0x104].
    let badge_byte_index       = BADGE_FLAG_START / 8; // 0x104
    let flags_offset_in_ewram  = ewram_offset(save_block_base) + FLAGS_OFFSET + badge_byte_index;

    if ewram.len() < flags_offset_in_ewram + 2 {
        return None;
    }

    let b0   = ewram[flags_offset_in_ewram];
    let b1   = ewram[flags_offset_in_ewram + 1];
    let both = (b0 as u16) | ((b1 as u16) << 8);

    // Step 4: extract one bit per badge.
    // BADGE_FLAG_START (0x820) is 8-aligned so bit position for badge i is i.
    let bit_start = BADGE_FLAG_START % 8; // 0
    let mut badges = [false; NUM_BADGES];
    for i in 0..NUM_BADGES {
        badges[i] = (both >> (bit_start + i)) & 1 == 1;
    }

    // Step 5: find the first unearned badge to identify the next gym.
    let next_gym = badges
        .iter()
        .position(|&obtained| !obtained)
        .map(|i| gym_leaders()[i].clone());

    Some(BadgeState { badges, next_gym })
}

/// Returns the name of badge N (0-indexed), or `"Unknown"` if out of range.
pub fn badge_name(index: usize) -> &'static str {
    match index {
        0 => "Boulder Badge",
        1 => "Cascade Badge",
        2 => "Thunder Badge",
        3 => "Rainbow Badge",
        4 => "Soul Badge",
        5 => "Marsh Badge",
        6 => "Volcano Badge",
        7 => "Earth Badge",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// FFI API
// ---------------------------------------------------------------------------

/// Reads the current badge state and returns it as a heap-allocated
/// [`BadgeStateFFI`] pointer.
///
/// # Returns
///
/// A non-null pointer to a [`BadgeStateFFI`] on success, or a null pointer
/// if the IWRAM/EWRAM snapshots are not yet ready or the SaveBlock1 pointer
/// is invalid.
///
/// # Safety
///
/// The returned pointer must be freed with [`badge_state_free`] exactly once.
/// Do not free individual string fields — [`badge_state_free`] handles all
/// allocations owned by the struct.
#[unsafe(no_mangle)]
pub extern "C" fn badge_read_state() -> *mut BadgeStateFFI {
    let Some(state) = read_badge_state() else {
        return std::ptr::null_mut();
    };

    let (has_next_gym, next_leader, next_city, next_badge, next_max_level) =
        match &state.next_gym {
            Some(gym) => (
                1,
                to_c_string(&gym.leader),
                to_c_string(&gym.city),
                to_c_string(&gym.badge),
                gym.max_level,
            ),
            None => (0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0),
        };

    let mut badges = [0u8; NUM_BADGES];
    for (i, &obtained) in state.badges.iter().enumerate() {
        badges[i] = obtained as u8;
    }

    let ffi = Box::new(BadgeStateFFI {
        badges,
        badge_count:    state.count() as i32,
        has_next_gym,
        next_leader,
        next_city,
        next_badge,
        next_max_level,
    });

    Box::into_raw(ffi)
}

/// Frees a [`BadgeStateFFI`] previously returned by [`badge_read_state`].
///
/// Frees all heap-allocated string fields before freeing the struct itself.
/// Passing a null pointer is a no-op.
///
/// # Safety
///
/// `ptr` must have been returned by [`badge_read_state`] and must not have
/// been freed already. After this call, `ptr` is invalid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_state_free(ptr: *mut BadgeStateFFI) {
    if ptr.is_null() {
        return;
    }
    let state = unsafe { Box::from_raw(ptr) };
    // Retake ownership of the C strings so they are properly dropped.
    if !state.next_leader.is_null() {
        drop(unsafe { CString::from_raw(state.next_leader) });
    }
    if !state.next_city.is_null() {
        drop(unsafe { CString::from_raw(state.next_city) });
    }
    if !state.next_badge.is_null() {
        drop(unsafe { CString::from_raw(state.next_badge) });
    }
    // state (the Box) is dropped here, freeing the struct itself.
}

/// Returns the number of badges obtained from a [`BadgeStateFFI`] pointer.
///
/// Returns `0` if `ptr` is null.
///
/// # Safety
///
/// `ptr` must be a valid pointer returned by [`badge_read_state`] that has
/// not yet been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_state_count(ptr: *const BadgeStateFFI) -> i32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).badge_count }
}

/// Returns `1` if the badge at `index` (0–7, gym order) has been obtained,
/// `0` otherwise. Returns `0` for out-of-range indices or a null pointer.
///
/// # Safety
///
/// `ptr` must be a valid pointer returned by [`badge_read_state`] that has
/// not yet been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_state_get(ptr: *const BadgeStateFFI, index: usize) -> i32 {
    if ptr.is_null() || index >= NUM_BADGES {
        return 0;
    }
    unsafe { (*ptr).badges[index] as i32 }
}

/// Returns `1` if all 8 badges have been obtained, `0` otherwise.
/// Returns `0` for a null pointer.
///
/// # Safety
///
/// `ptr` must be a valid pointer returned by [`badge_read_state`] that has
/// not yet been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_state_all_obtained(ptr: *const BadgeStateFFI) -> i32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { ((*ptr).badge_count == NUM_BADGES as i32) as i32 }
}

/// Returns the name of badge `index` (0-indexed, gym order) as a static
/// null-terminated string. Returns `"Unknown\0"` for out-of-range indices.
///
/// The returned pointer is valid for the lifetime of the process and must
/// **not** be freed by the caller.
#[unsafe(no_mangle)]
pub extern "C" fn badge_get_name(index: usize) -> *const c_char {
    // SAFETY: these string literals are 'static and null-terminated.
    let s: &'static [u8] = match index {
        0 => b"Boulder Badge\0",
        1 => b"Cascade Badge\0",
        2 => b"Thunder Badge\0",
        3 => b"Rainbow Badge\0",
        4 => b"Soul Badge\0",
        5 => b"Marsh Badge\0",
        6 => b"Volcano Badge\0",
        7 => b"Earth Badge\0",
        _ => b"Unknown\0",
    };
    s.as_ptr() as *const c_char
}

/// Returns a heap-allocated null-terminated string containing the next gym
/// leader's name, or a null pointer if all badges are obtained or `ptr` is
/// null.
///
/// The caller must free the returned string with [`badge_free_string`].
///
/// # Safety
///
/// `ptr` must be a valid pointer returned by [`badge_read_state`] that has
/// not yet been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_gym_leader(ptr: *const BadgeStateFFI) -> *mut c_char {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    let state = unsafe { &*ptr };
    if state.has_next_gym == 0 || state.next_leader.is_null() {
        return std::ptr::null_mut();
    }
    // Duplicate the string so the caller owns an independent copy.
    let s = unsafe { std::ffi::CStr::from_ptr(state.next_leader) }
        .to_string_lossy()
        .into_owned();
    to_c_string(&s)
}

/// Returns a heap-allocated null-terminated string containing the next gym's
/// city name, or a null pointer if all badges are obtained or `ptr` is null.
///
/// The caller must free the returned string with [`badge_free_string`].
///
/// # Safety
///
/// `ptr` must be a valid pointer returned by [`badge_read_state`] that has
/// not yet been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_gym_city(ptr: *const BadgeStateFFI) -> *mut c_char {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    let state = unsafe { &*ptr };
    if state.has_next_gym == 0 || state.next_city.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { std::ffi::CStr::from_ptr(state.next_city) }
        .to_string_lossy()
        .into_owned();
    to_c_string(&s)
}

/// Returns the highest level pokemon on the next gym leader's team, or `0`
/// if all badges are obtained or `ptr` is null.
///
/// # Safety
///
/// `ptr` must be a valid pointer returned by [`badge_read_state`] that has
/// not yet been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_gym_max_level(ptr: *const BadgeStateFFI) -> c_uchar {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (*ptr).next_max_level }
}

/// Frees a string previously returned by [`badge_gym_leader`],
/// [`badge_gym_city`], or any other FFI function that documents its return
/// value as needing to be freed with this function.
///
/// Passing a null pointer is a no-op.
///
/// # Safety
///
/// `ptr` must have been returned by one of the FFI string-returning functions
/// in this crate and must not have been freed already.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badge_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(ptr) });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
 
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
 
    // ── Helpers ──────────────────────────────────────────────────────────────
 
    /// Constructs a heap-allocated [`BadgeStateFFI`] with the given badge
    /// flags and optional next-gym info, bypassing `fire_red_memory` so the
    /// FFI layer can be tested without a live RetroArch connection.
    fn make_ffi_state(
        badge_flags: [bool; NUM_BADGES],
        next_gym: Option<(&str, &str, &str, u8)>,
    ) -> *mut BadgeStateFFI {
        let count = badge_flags.iter().filter(|&&b| b).count();
 
        let mut badges = [0u8; NUM_BADGES];
        for (i, &b) in badge_flags.iter().enumerate() {
            badges[i] = b as u8;
        }
 
        let (has_next_gym, next_leader, next_city, next_badge, next_max_level) =
            match next_gym {
                Some((leader, city, badge, level)) => (
                    1,
                    to_c_string(leader),
                    to_c_string(city),
                    to_c_string(badge),
                    level,
                ),
                None => (0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0),
            };
 
        Box::into_raw(Box::new(BadgeStateFFI {
            badges,
            badge_count: count as i32,
            has_next_gym,
            next_leader,
            next_city,
            next_badge,
            next_max_level,
        }))
    }
 
    /// Reads a `*const c_char` as a `&str`. Panics if null or invalid UTF-8.
    unsafe fn cstr(ptr: *const c_char) -> &'static str {
        unsafe { CStr::from_ptr(ptr) }.to_str().expect("invalid UTF-8")
    }
 
    // ── badge_state_count ─────────────────────────────────────────────────────
 
    #[test]
    fn test_count_zero_badges() {
        let ptr = make_ffi_state([false; 8], Some(("Brock", "Pewter City", "Boulder Badge", 14)));
        assert_eq!(unsafe { badge_state_count(ptr) }, 0);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_count_three_badges() {
        let flags = [true, true, true, false, false, false, false, false];
        let ptr = make_ffi_state(flags, Some(("Erika", "Celadon City", "Rainbow Badge", 29)));
        assert_eq!(unsafe { badge_state_count(ptr) }, 3);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_count_all_badges() {
        let ptr = make_ffi_state([true; 8], None);
        assert_eq!(unsafe { badge_state_count(ptr) }, 8);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_count_null_returns_zero() {
        assert_eq!(unsafe { badge_state_count(std::ptr::null()) }, 0);
    }
 
    // ── badge_state_get ───────────────────────────────────────────────────────
 
    #[test]
    fn test_get_individual_badges() {
        // Only badges 0, 2, 4 obtained.
        let flags = [true, false, true, false, true, false, false, false];
        let ptr = make_ffi_state(flags, Some(("Misty", "Cerulean City", "Cascade Badge", 21)));
 
        assert_eq!(unsafe { badge_state_get(ptr, 0) }, 1, "badge 0 should be obtained");
        assert_eq!(unsafe { badge_state_get(ptr, 1) }, 0, "badge 1 should not be obtained");
        assert_eq!(unsafe { badge_state_get(ptr, 2) }, 1, "badge 2 should be obtained");
        assert_eq!(unsafe { badge_state_get(ptr, 3) }, 0, "badge 3 should not be obtained");
        assert_eq!(unsafe { badge_state_get(ptr, 4) }, 1, "badge 4 should be obtained");
        assert_eq!(unsafe { badge_state_get(ptr, 5) }, 0, "badge 5 should not be obtained");
 
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_get_out_of_range_returns_zero() {
        let ptr = make_ffi_state([true; 8], None);
        assert_eq!(unsafe { badge_state_get(ptr, 8) }, 0);
        assert_eq!(unsafe { badge_state_get(ptr, 999) }, 0);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_get_null_returns_zero() {
        assert_eq!(unsafe { badge_state_get(std::ptr::null(), 0) }, 0);
    }
 
    // ── badge_state_all_obtained ──────────────────────────────────────────────
 
    #[test]
    fn test_all_obtained_true_when_full() {
        let ptr = make_ffi_state([true; 8], None);
        assert_eq!(unsafe { badge_state_all_obtained(ptr) }, 1);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_all_obtained_false_when_partial() {
        let flags = [true, true, true, true, true, true, true, false];
        let ptr = make_ffi_state(flags, Some(("Giovanni", "Viridian City", "Earth Badge", 55)));
        assert_eq!(unsafe { badge_state_all_obtained(ptr) }, 0);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_all_obtained_null_returns_zero() {
        assert_eq!(unsafe { badge_state_all_obtained(std::ptr::null()) }, 0);
    }
 
    // ── badge_get_name ────────────────────────────────────────────────────────
 
    #[test]
    fn test_badge_names_all_indices() {
        let expected = [
            "Boulder Badge",
            "Cascade Badge",
            "Thunder Badge",
            "Rainbow Badge",
            "Soul Badge",
            "Marsh Badge",
            "Volcano Badge",
            "Earth Badge",
        ];
        for (i, &name) in expected.iter().enumerate() {
            let ptr = badge_get_name(i);
            assert!(!ptr.is_null(), "badge_get_name({}) returned null", i);
            assert_eq!(unsafe { cstr(ptr) }, name, "badge name mismatch at index {}", i);
            // Static strings must NOT be freed — we just verify the pointer is valid.
        }
    }
 
    #[test]
    fn test_badge_name_out_of_range() {
        let ptr = badge_get_name(8);
        assert!(!ptr.is_null());
        assert_eq!(unsafe { cstr(ptr) }, "Unknown");
 
        let ptr = badge_get_name(usize::MAX);
        assert!(!ptr.is_null());
        assert_eq!(unsafe { cstr(ptr) }, "Unknown");
    }
 
    // ── badge_gym_leader / badge_gym_city ─────────────────────────────────────
 
    #[test]
    fn test_gym_leader_and_city_with_next_gym() {
        let ptr = make_ffi_state(
            [false; 8],
            Some(("Brock", "Pewter City", "Boulder Badge", 14)),
        );
 
        let leader = unsafe { badge_gym_leader(ptr) };
        assert!(!leader.is_null());
        assert_eq!(unsafe { CStr::from_ptr(leader).to_str().unwrap() }, "Brock");
        unsafe { badge_free_string(leader) };
 
        let city = unsafe { badge_gym_city(ptr) };
        assert!(!city.is_null());
        assert_eq!(unsafe { CStr::from_ptr(city).to_str().unwrap() }, "Pewter City");
        unsafe { badge_free_string(city) };
 
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_gym_leader_null_when_all_badges_obtained() {
        let ptr = make_ffi_state([true; 8], None);
        assert!(unsafe { badge_gym_leader(ptr) }.is_null());
        assert!(unsafe { badge_gym_city(ptr) }.is_null());
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_gym_leader_null_on_null_ptr() {
        assert!(unsafe { badge_gym_leader(std::ptr::null()) }.is_null());
        assert!(unsafe { badge_gym_city(std::ptr::null()) }.is_null());
    }
 
    /// Verify that `badge_gym_leader` returns an independent copy — mutating
    /// the returned string does not affect the struct's internal pointer.
    #[test]
    fn test_gym_leader_returns_independent_copy() {
        let ptr = make_ffi_state(
            [false; 8],
            Some(("Brock", "Pewter City", "Boulder Badge", 14)),
        );
 
        let copy1 = unsafe { badge_gym_leader(ptr) };
        let copy2 = unsafe { badge_gym_leader(ptr) };
 
        assert!(!copy1.is_null());
        assert!(!copy2.is_null());
        // Two separate heap allocations — different pointers.
        assert_ne!(copy1, copy2, "expected independent copies, got same pointer");
 
        unsafe { badge_free_string(copy1) };
        unsafe { badge_free_string(copy2) };
        unsafe { badge_state_free(ptr) };
    }
 
    // ── badge_gym_max_level ───────────────────────────────────────────────────
 
    #[test]
    fn test_max_level_with_next_gym() {
        let ptr = make_ffi_state(
            [false; 8],
            Some(("Brock", "Pewter City", "Boulder Badge", 14)),
        );
        assert_eq!(unsafe { badge_gym_max_level(ptr) }, 14);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_max_level_zero_when_all_obtained() {
        let ptr = make_ffi_state([true; 8], None);
        assert_eq!(unsafe { badge_gym_max_level(ptr) }, 0);
        unsafe { badge_state_free(ptr) };
    }
 
    #[test]
    fn test_max_level_null_returns_zero() {
        assert_eq!(unsafe { badge_gym_max_level(std::ptr::null()) }, 0);
    }
 
    // ── badge_free_string ─────────────────────────────────────────────────────
 
    #[test]
    fn test_free_string_null_is_noop() {
        // Should not panic or crash.
        unsafe { badge_free_string(std::ptr::null_mut()) };
    }
 
    #[test]
    fn test_free_string_valid_pointer() {
        let ptr = to_c_string("test string");
        assert!(!ptr.is_null());
        unsafe { badge_free_string(ptr) };
        // If we reach here without SIGABRT/SIGSEGV, the free was clean.
    }
 
    // ── badge_state_free ──────────────────────────────────────────────────────
 
    #[test]
    fn test_state_free_null_is_noop() {
        // Should not panic or crash.
        unsafe { badge_state_free(std::ptr::null_mut()) };
    }
 
    #[test]
    fn test_state_free_with_gym_info() {
        // Ensures all string fields are freed without leaking or double-freeing.
        let ptr = make_ffi_state(
            [true, true, false, false, false, false, false, false],
            Some(("Lt. Surge", "Vermilion City", "Thunder Badge", 24)),
        );
        unsafe { badge_state_free(ptr) };
        // Reaching here without AddressSanitizer complaints means the free was clean.
    }
 
    #[test]
    fn test_state_free_no_gym_info() {
        let ptr = make_ffi_state([true; 8], None);
        unsafe { badge_state_free(ptr) };
    }
 
    // ── BadgeState Rust API ───────────────────────────────────────────────────
 
    #[test]
    fn test_badge_state_count_method() {
        let state = BadgeState {
            badges: [true, true, false, false, false, false, false, false],
            next_gym: None,
        };
        assert_eq!(state.count(), 2);
    }
 
    #[test]
    fn test_badge_state_all_obtained_method() {
        let full = BadgeState { badges: [true; 8], next_gym: None };
        assert!(full.all_obtained());
 
        let partial = BadgeState {
            badges: [true, true, true, true, true, true, true, false],
            next_gym: None,
        };
        assert!(!partial.all_obtained());
    }
 
    #[test]
    fn test_badge_state_default_is_empty() {
        let state = BadgeState::default();
        assert_eq!(state.count(), 0);
        assert!(!state.all_obtained());
        assert!(state.next_gym.is_none());
    }
 
    // ── Gym leader table correctness ──────────────────────────────────────────
 
    #[test]
    fn test_gym_leader_table_order_and_levels() {
        let leaders = gym_leaders();
        let expected = [
            ("Brock",     14u8),
            ("Misty",     21),
            ("Lt. Surge", 24),
            ("Erika",     29),
            ("Koga",      43),
            ("Sabrina",   50),
            ("Blaine",    54),
            ("Giovanni",  55),
        ];
        for (i, (name, level)) in expected.iter().enumerate() {
            assert_eq!(leaders[i].leader, *name,  "leader mismatch at index {}", i);
            assert_eq!(leaders[i].max_level, *level, "level mismatch at index {}", i);
        }
    }
 
    #[test]
    fn test_gym_table_has_eight_entries() {
        assert_eq!(gym_leaders().len(), NUM_BADGES);
    }
}