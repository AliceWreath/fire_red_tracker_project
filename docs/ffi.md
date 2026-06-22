# FFI Ownership and Lifetime Diagrams

Three crates expose C-callable functions or use manual heap allocation:
`fire_red_loop`, `fire_red_badge`, and `fire_red_pokemon_data`.

---

## fire_red_loop — C Entry Points

These functions are the primary integration surface.  Internally they drive
the entire tracker (ROM loading, scanner, subsystem init, polling thread).

### Function Signatures

```c
// Start the game-polling loop.
// file_path  — path to the FireRed .gba ROM (UTF-8, null-terminated)
// is_clean   — pass true for ability-name display (only reliable on unmodified ROMs)
// returns    — 0 = ok, -1 = empty path, -2 = ROM load failed,
//              -3 = wild headers not found, -4 = already running
int32_t c_start_loop(const char* file_path, bool is_clean);

// Stop the loop and join the polling thread.
void stop_loop(void);
```

### Ownership

```
Caller                          fire_red_loop
──────────────────────────────────────────────────────────────────────

c_start_loop(ptr, flag)
  │
  │ ptr is borrowed for the duration of the call only.
  │ Rust converts it via CStr::from_ptr(ptr).to_str() on entry.
  │ The C string does NOT need to outlive the call.
  │
  │ On success, ownership of all resources moves INSIDE the crate:
  │
  ▼
  ROM bytes        → fire_red_rom_buffer (global Vec<u8>, never freed)
  WildPokemonHeaders → fire_red_pokemon_data (OnceLock<Vec<...>>)
  FireRedState     → STATE: OnceLock<Mutex<FireRedState>>
  THREAD_HANDLE    → THREAD_HANDLE: Mutex<Option<JoinHandle<()>>>
  (memory loop, box monitor, etc. also stored in their own globals)
  │
  ◀── returns i32 (no heap allocation returned to caller)

stop_loop()
  │ Signals RUNNING = false, joins THREAD_HANDLE
  │ Does NOT free ROM or header globals — they live for the process.
  ▼
  (all owned resources remain inside the crate; nothing is returned)
```

**Rule:** the caller only passes a `*const c_char` borrow.  All heap
resources are owned and freed internally.  There is nothing for the caller
to free.

---

## fire_red_badge — C FFI

The badge crate exposes a full C-owned allocation model: the caller receives
a heap-allocated struct and is responsible for calling the matching free
function.

### Function Signatures

```c
// Allocate and return a snapshot of the current badge state.
// Returns NULL if IWRAM/EWRAM snapshots are unavailable or if SaveBlock1
// pointer is invalid.
BadgeStateFFI* badge_read_state(void);

// Count earned badges (0–8).
int32_t badge_state_count(const BadgeStateFFI* state);

// Return the next gym leader's name as a heap-allocated C string.
// The caller MUST free it with badge_free_string().
char* badge_gym_leader(const BadgeStateFFI* state);

// Free a C string returned by this crate.
void badge_free_string(char* ptr);

// Free a BadgeStateFFI returned by badge_read_state().
void badge_state_free(BadgeStateFFI* ptr);
```

### Struct Layout

```c
typedef struct {
    uint8_t  badges[8];        // 1 = earned, 0 = not
    int32_t  badge_count;
    int32_t  has_next_gym;     // 1 = yes, 0 = no
    char*    next_leader;      // heap-allocated C string (CString::into_raw)
    char*    next_city;        // heap-allocated C string
    char*    next_badge;       // heap-allocated C string
    uint8_t  next_max_level;
} BadgeStateFFI;
```

### Ownership Diagram

```
Rust side (badge_read_state)              C caller
──────────────────────────────────────────────────────────────────────

badge_read_state()
  │
  ├── Box::new(BadgeStateFFI { ... })     ← heap allocated
  │     .next_leader = CString::new(leader_str)
  │                      .unwrap()
  │                      .into_raw()      ← C string heap allocated
  │     .next_city   = CString::into_raw()
  │     .next_badge  = CString::into_raw()
  │
  └── Box::into_raw(boxed)
              │
              ▼ *mut BadgeStateFFI ──────────────▶  caller owns ptr

  ┌──────────── caller uses ptr ────────────────────────────────┐
  │                                                              │
  │  badge_state_count(ptr)   — borrows ptr, returns i32        │
  │  badge_gym_leader(ptr)    — returns *mut c_char (new alloc) │
  │    └─▶ badge_free_string(char_ptr)  MUST be called          │
  │                                                              │
  └──────────────────────────────────────────────────────────────┘
              │
              ▼
  badge_state_free(ptr)
    │
    ├── CString::from_raw(state.next_leader)  → dropped
    ├── CString::from_raw(state.next_city)    → dropped
    ├── CString::from_raw(state.next_badge)   → dropped
    └── Box::from_raw(ptr)                    → dropped
```

**Rules:**
1. Every non-NULL `BadgeStateFFI*` from `badge_read_state` MUST be freed with
   `badge_state_free`.  Double-free is undefined behaviour.
2. Every `char*` from `badge_gym_leader` MUST be freed with `badge_free_string`
   — not `free()`, not `delete[]`.  The allocator must match.
3. `NULL` is a valid return from `badge_read_state` (no snapshot available).
   The caller MUST check before use.
4. `BadgeStateFFI` contains `char*` fields (`next_leader`, `next_city`,
   `next_badge`).  These are owned by the struct — do NOT free them
   independently if you plan to call `badge_state_free`.

---

## fire_red_pokemon_data — Internal FFI Allocations

This crate does NOT expose `#[no_mangle] extern "C"` functions.  The FFI
types (`WildPokemonHeaderFFI`, `WildPokemonInfoFFI`) and their allocation
helpers are `pub unsafe` Rust functions used internally across crate
boundaries.

### Structs

```rust
// C-layout header (safe to pass across FFI)
#[repr(C)]
pub struct WildPokemonHeaderFFI {
    pub map_group:             c_uchar,
    pub map_num:               c_uchar,
    pub land_mon_encounters:   *mut WildPokemonInfoFFI,  // heap, may be null
    pub water_mon_encounters:  *mut WildPokemonInfoFFI,
    pub rock_smash_encounters: *mut WildPokemonInfoFFI,
    pub fishing_encounters:    *mut WildPokemonInfoFFI,
}

// Flexible array member — trailing WildPokemon[] immediately follows
// the struct in memory (C99 §6.7.2.1 flexible array member pattern)
#[repr(C)]
pub struct WildPokemonInfoFFI {
    pub encounter_rate: c_uchar,
    pub pokemon_count:  usize,
    pub wild_pokemon_list: __IncompleteArrayField<WildPokemon>,
    //  ↑ zero-size marker; actual data at &self + size_of::<WildPokemonInfoFFI>()
}

#[repr(C)]
pub struct WildPokemon {
    pub min_level: c_uchar,
    pub max_level: c_uchar,
    pub species:   c_ushort,
}
```

### Allocation Layout in Memory

```
alloc_wild_pokemon_info_ffi(encounter_rate, list: Vec<WildPokemon>)

  Layout computed as:
    size  = size_of::<WildPokemonInfoFFI>()          (header)
          + size_of::<WildPokemon>() * list.len()     (trailing array)
    align = align_of::<WildPokemonInfoFFI>()

  Heap:
  ┌──────────────────────────────────────────────────────────────┐
  │  WildPokemonInfoFFI                                          │
  │   encounter_rate : u8           ← +0                        │
  │   pokemon_count  : usize        ← +8 (alignment padded)     │
  │   wild_pokemon_list: [zero-size marker]  ← +16              │
  ├──────────────────────────────────────────────────────────────┤
  │  WildPokemon[0]   min_level, max_level, species  ← +16      │
  │  WildPokemon[1]                                  ← +22      │
  │  ...                                                         │
  │  WildPokemon[N-1]                                ← +16+6N   │
  └──────────────────────────────────────────────────────────────┘
       ▲
       └── *mut WildPokemonInfoFFI (returned)
```

### Ownership Diagram

```
Rust caller                              alloc/dealloc helpers
──────────────────────────────────────────────────────────────────────

let ptr = unsafe {
    alloc_wild_pokemon_info_ffi(rate, vec)
};
// ptr: *mut WildPokemonInfoFFI
// caller owns the allocation

// Read:
let list: Vec<WildPokemon> = unsafe {
    get_wild_pokemon_vector_from_ptr_ffi(ptr)
    // copies data into a new Vec; does NOT free ptr
};

// Free:
unsafe { dealloc_wild_pokemon_info_ffi(ptr) };
// ptr must not be used again
```

### Invariants That Must Be Upheld

| Invariant | Why |
|-----------|-----|
| `ptr` must come from `alloc_wild_pokemon_info_ffi` | `dealloc` uses the same layout computation; a pointer from anywhere else causes UB |
| `ptr` must be freed exactly once | Standard double-free UB |
| `pokemon_count` must equal `list.len()` at allocation time | `get_wild_pokemon_vector_from_ptr_ffi` slices `pokemon_count` elements; a mismatch reads out of bounds |
| The allocation must outlive any pointer into the trailing array | Any `*const WildPokemon` derived from `wild_pokemon_list.as_ptr()` is invalidated after `dealloc` |
| No other thread may read the struct while it is being written | The struct is not `Send` by default; callers that share across threads must add external synchronisation |

### __IncompleteArrayField

```rust
pub struct __IncompleteArrayField<T>(PhantomData<T>);

impl<T> __IncompleteArrayField<T> {
    // Returns a raw pointer to the first trailing array element.
    // SAFETY: the allocation must have been made with enough space
    //         for at least `count` T values following the parent struct.
    pub unsafe fn as_ptr(&self) -> *const T {
        unsafe { std::mem::transmute(self) }
    }
}
```

The transmute is sound because `__IncompleteArrayField<T>` is a zero-size
type at the _end_ of a `#[repr(C)]` struct, so its address is the address of
the first trailing element — identical to how C flexible array members work.
The safety requirement is entirely on the caller: the pointee memory must
actually contain valid `T` values.

---

## Summary: Who Owns What

| Resource | Owner | Freed By |
|----------|-------|----------|
| ROM bytes | `fire_red_rom_buffer` global | Never (process lifetime) |
| EWRAM/IWRAM snapshots | `fire_red_memory` globals | Never (process lifetime) |
| `BadgeStateFFI*` | C caller, after `badge_read_state()` | `badge_state_free()` |
| `char*` from `badge_gym_leader()` | C caller | `badge_free_string()` |
| `*mut WildPokemonInfoFFI` | Rust caller of `alloc_*` | `dealloc_wild_pokemon_info_ffi()` |
| `Arc<MonitorSlot>` | `SharedSlots` vec + per-thread clones | Last `Arc` drop |
| `DbState` (global) | `DB: OnceLock<Mutex<DbState>>` | Never (process lifetime) |
