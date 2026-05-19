//! # FireRed Map Data Structures
//! 
//! C-compatible (in the works not completed) (`#[repr(C)]`) structs that mirror the in-memory layout of 
//! pokemon FireRed map data, along with the methods for deserializing them from raw emulate memory reads
//! and generating follow-up `READ_CORE_MEMORY` commands.
//! 
//! ## Parsing convention
//! 
//! Every `fill_*` method accepts a `buffer: &[&str]` slice of hex byte tokens as returned by the 
//! emulator's `READ_CORE_MEMORY` response. Parsing always starts at index 2 because index 0 is the
//! command echo and index 1 is the address; the actual data bytes begin at index 2.
//! 
//! Methods consume `self` and return the populated struct (builder pattern), except for 
//! `fill_allow_esc_run_map_name` which takes `&mut self` because it writes multiple fields from 
//! a single byte.

use fire_red_get_values::*;
use libc::size_t;
use std::os::raw::{c_int, c_short, c_uchar, c_uint, c_ushort};

// -------------------------------------------
// Map identification
// -------------------------------------------

/// Identifies a map by its group and name indices.
/// 
/// FireRed organizes maps into groups (roughly corresponding to towns/routes)
/// and gives each map within a group a sequential name index. Together,
/// `(group, name)` uniquely addresses any map in the game.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct CurrentMapGroupAndName {
    /// Map group index (e.g. 0 = Pallet Town area)
    pub group: c_uchar,

    /// Map name index within the group.
    pub name: c_uchar,
}

// ---------------------------------------------
// Map header
// ---------------------------------------------


/// Top-level descriptor for a single mpa, sotred at the map table entry.
/// 
/// Each map in FireRed begins with this 28-byte header. All `*_offset_ptr`
/// fields are GBA ROM/RAM pointers; dereference them with a follow-up
/// `READ_CORE_MEMORY` command to obtain the pointed-to struct.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapHeader {
    /// Pointer to the [`MapLayout`] struct (a.k.a. map footer)
    pub footer_offset_ptr: c_uint, // Byte 1-4  this is a pointer to the MapLayout struct
    /// Pointer to the [`MapEvents`] struct
    pub event_offset_ptr: c_uint,  // Byte 5-8
    /// Pointer to the map's script table
    pub script_offset_ptr: c_uint, // Byte 9-12
    /// Pointer to the [`MapConnections`] strucct, or 0 if there are none
    pub connections_offset_ptr: c_uint, // Byte 13-16
    /// BGM track ID played on this map
    pub music_id: c_ushort,      // Byte 17-18
    /// Lower byte of the map layout (footer) ID
    pub footer_id: c_uchar,      // Byte 19
    /// Upper byte of the map layout (footer) ID
    pub footer_id_cont: c_uchar, // Byte 20
    /// Index into the map-name string table shown on teh location banner.
    pub name_index: c_uchar,     // Byte 21
    /// Cave/dungeon type falg; controls lighting and wild encounter music.
    pub cave_type: c_uchar,      // Byte 22
    /// Weather effect index (rain, snow, sandstorm, etc.)
    pub weather_type: c_uchar,   // Byte 23
    /// Overrides the default trainer battle background for this map.
    pub trainer_battle_background_override: c_uchar, // Byte 24
    /// Non-zero if the player can use a bicycle here.
    pub allow_bicycle: c_uchar,  // Byte 25
    /// Bit 2 of byte 26: player can use Escape Rope / Dig.
    pub allow_escape: bool,      // Byte 26
    /// Bit 1 of byte 26: player can run (hold B).
    pub allow_running: bool,     // Byte 26
    /// Bit 0 of byte 26: show the location name banner on map entry.
    pub show_map_name: bool,     // Byte 26 + 5 unused bits
    /// Floor number displayed in multi-floor dungeons (e.g. "B1F")
    pub floor_number: c_uchar,   // Byte 27
    /// Overrides teh wild battle background; vallues 0x00-0x09 are standard
    /// 0x0A and above produce undefined behaviour.
    pub battle_background_override: c_uchar, // Byte 28
}

// ------------------------------------------------
// Map events
// ------------------------------------------------

/// A background event (hidden item, secret base entrance, sign, etc.)
/// 
/// The `script_ptr` and `hidden_items` fields are a union in teh original C
/// source; which one is meaningful depends on `kind`
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct BgEvent {
    /// Tile X coordinate of the event within the map.
    pub x: c_ushort,
    /// Tile Y coordinate of the event within the map.
    pub y: c_ushort,
    /// Elevation layer the event sits on.
    pub elevation: c_uchar,
    /// Event sub-type (determines how the union fields are interpreted)
    pub kind: c_uchar,

    // union - only one field is valid depending on the `kind`
    /// Pointer to the event script (valid when `kind` indicates a script event).
    pub script_ptr: c_uint, //points to u8
    /// Hidden-item data (valid when `kind` indicates a hidden item).
    pub hidden_items: c_uint,
}

/// A coordinate trigger event: fires a script when the player steps on a tile.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct CoordEvent {
    /// Tile X coordinate of the trigger
    pub x: c_ushort,
    /// Tile Y coordinate of the trigger
    pub y: c_ushort,
    /// Elevation layer the trigger is on
    pub elevation: c_uchar,
    /// Trigger condition index (determines when the script fires)
    pub trigger: c_ushort,
    /// Secondary index used with `trigger` for multi-state triggers.
    pub index: c_ushort,
    /// Pointer to the script executed when the trigger fires.
    pub script_ptr: c_uint, // points to u8
}

/// A warp event: teleports the player to another map when stepped on.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct WarpEvent {
    /// Tile X coordinate of the warp tile (signed to allow connection offsets)
    pub x: c_short,
    /// Tile Y coordinate of the warp tile
    pub y: c_short,
    /// Elevation layer of the warp tile.
    pub elevation: c_uchar,
    /// Index of the destination warp on the target map.
    pub warp_id: c_uchar,
    /// Map number within the destinatino group
    pub map_num: c_uchar,
    /// destination map group
    pub map_group: c_uchar,
}

// --------------------------------------------
// Map connections
// --------------------------------------------

/// Header for the list of adjacent-map connections attached to a map.
/// 
/// Connections define the maps that border this one in each cardinal direction,
/// enabling seamless scrolling between routes, towns, and so on.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapConnections {
    /// Number of [`MapConnection`] entries in the list
    count: c_int,
    /// Pointer to the fires [`MapConnection`] entry
    map_connection_ptr: c_uint,
}

/// A single directional connection to the adjacent map.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapConnection {
    /// Direction of the connection (0 = south, 1 = north, 2 = west, 3 = east)
    pub direction: c_uchar,
    /// Pixel offset applied when entering the connected map.
    pub offset: c_uint,
    /// Map group of the connected map
    pub map_group: c_uchar,
    /// Map number within the `map_group`
    pub map_number: c_uchar,
}

// ------------------------------------------------------
// Map scripts / event table
// ------------------------------------------------------

/// Placeholder wrapper for the map script table entry byte.
/// 
/// The full script table is variable-length and parsed separately; this stuct
/// holds only the first byte as a handle for the `fill_*` pattern.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapScripts {
    scripts: c_uchar,
}

/// Counts and pointers for all event lists on a map.
/// 
/// The four `*_pointer` fields point to arrays of their respective event types;
/// use the corresponding count fields to know how many entries to read.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapEvents {
    /// Number of [`ObjectEventTemplate`] entries (NPCs, items on the ground, etc.)
    pub object_event_count: c_uchar,
    /// Number of [`WarpEvent`] entries.
    pub warp_count: c_uchar,
    /// Number of [`CoordEvent`] entries
    pub coord_event_count: c_uchar,
    /// Number of [`BgEvent`] entries
    pub bg_event_count: c_uchar,
    /// Pointer to the array of `[ObjectEventTemplate`] structs
    pub object_event_template_ptr: c_uint,
    /// Pointer to the array of [`WarpEvent`] structs
    pub warp_event_pointer: c_uint,
    /// Pointer to the array of [`CoordEvent`] structs
    pub coord_event_pointer: c_uint,
    /// Point to the array of [`BgEvent`] structs.
    pub bg_event_pointer: c_uint,
}

// --------------------------------------------------------------------
// Map layout (footer)
// --------------------------------------------------------------------

/// Describes the visual and spatial layout of a map.
/// 
/// Also called the "footer" in the original FireRed source. Contains the map
/// dimensions, pointers to the tile data and tilesets, and border tile info.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapLayout {
    // aka footer
    /// Map width in tiles
    pub width: c_int,                  // map width
    /// Map height in tiles
    pub height: c_int,                 // map height
    /// pointer to the border tile block (shown outside the playable area)
    pub border_ptr: c_uint,            // ptr to the borders
    /// pointer to the map tile data (array of `c_ushort` metatile indices)
    pub map_ptr: c_uint,               // ptr to the map?
    /// pointer to the primary tileset struct
    pub tileset_ptr: c_uint,           // ptr to primary tileset struct
    /// pointer to the secondary tileset struct
    pub secondary_tileset_ptr: c_uint, // ptr to secondary tileset struct
    /// width of the border tile region in tiles.
    pub border_width: c_uchar,         // border width
    /// height of the border tile region in tiles.
    pub border_height: c_uchar,        // border height
}

// ------------------------------------------------------------------------
// Object event template (NPC / overworld object)
// ------------------------------------------------------------------------

/// Template used to spawn an overword object (NPC, item ball, etc.)
/// 
/// One of these exists per object on the map; the engine instantiates live
/// `ObjectEvent` structs from them at runtime.
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct ObjectEventTemplate {
    /// Per-map unique ID for this object
    pub local_id: c_uchar,
    /// Index into the object graphics table (sprite sheet)
    pub graphics_id: c_uchar,
    /// Non-zero if the object was spawned via a map connection
    pub in_connection: c_uchar,
    /// Tile X spawn position 
    pub x: c_short,
    /// Tile Y spawn position
    pub y: c_short,
    /// Elevation layer the object starts on.
    pub elevation: c_uchar,
    /// Movement behaviour (wander, face down, walk path, etc)
    pub movement_type: c_uchar,
    /// Horizontal wander radius in tiles
    pub movement_range_x: c_ushort,
    /// Vertical wander radius in tiles
    pub movement_range_y: c_ushort,
    /// Trainer type (0 = not a trainer, 1 = normal, 2 = see-all-directions, etc)
    pub trainer_type: c_ushort,
    /// Trainer sight range, or Berry Tree ID when `movement_type` is berry-tree
    pub trainer_range_berry_tree_id: c_ushort,
    /// pointer to the interactino / trainer script
    pub script_ptr: c_uint,
    /// game flag that hides or disables this object when set.
    pub flag_id: c_ushort,
}

// ---------------------------------------------------------------------
// impl blocks - deserialization and command generation
// ---------------------------------------------------------------------

impl BgEvent {
    /// Populates this [`BgEvent`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_bg_event(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;
        self.x = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.y = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.elevation = get_u8(&[buffer[index]]);
        index += 1;
        self.kind = get_u8(&[buffer[index]]);
        index += 1;
        self.script_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.hidden_items = get_u32(&buffer[index..index + 4]);

        self
    }

    /// Returns a `READ_CORE_MEMORY` command string that follows `script_ptr`
    /// to read the first byte of the event's script.
    pub fn generate_get_script_command(self) -> String {
        generate_follow_ptr_command(self.script_ptr, 1)
    }
}

impl CoordEvent {
    /// Populates this [`CoordEvent`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_coord_event(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;
        self.x = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.y = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.elevation = get_u8(&[buffer[index]]);
        index += 1;
        self.trigger = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.index = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.script_ptr = get_u32(&buffer[index..index + 4]);

        self
    }

    /// Returns a `READ_CORE_MEMORY` command string that reads the first byte
    /// of this coordinate event's script
    pub fn generate_get_script_command(self) -> String {
        format!("READ_CORE_MEMORY {:08X} {}\n", self.script_ptr, 1)
    }
}

impl WarpEvent {
    /// Populates this [`WarpEvent`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_warp_event(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;
        self.x = get_i16(&buffer[index..index + 2]);
        index += 2;
        self.y = get_i16(&buffer[index..index + 2]);
        index += 2;
        self.elevation = get_u8(&[buffer[index]]);
        index += 1;
        self.warp_id = get_u8(&[buffer[index]]);
        index += 1;
        self.map_num = get_u8(&[buffer[index]]);
        index += 1;
        self.map_group = get_u8(&[buffer[index]]);

        self
    }
}

impl ObjectEventTemplate {
    /// Populates this [`ObjectEventTemplate`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_obj_event_template(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;
        self.local_id = get_u8(&[buffer[index]]);
        index += 1;
        self.graphics_id = get_u8(&[buffer[index]]);
        index += 1;
        self.in_connection = get_u8(&[buffer[index]]);
        index += 1;
        self.x = get_i16(&buffer[index..index + 2]);
        index += 2;
        self.y = get_i16(&buffer[index..index + 2]);
        index += 2;
        self.elevation = get_u8(&[buffer[index]]);
        index += 1;
        self.movement_type = get_u8(&[buffer[index]]);
        index += 1;
        self.movement_range_x = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.movement_range_y = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.trainer_type = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.trainer_range_berry_tree_id = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.script_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.flag_id = get_u16(&buffer[index..index + 2]);

        self
    }

    /// Returns a `READ_CORE_MEMORY` command string that reads the first byte
    /// of this object's interaction script.
    pub fn generate_get_script_command(self) -> String {
        format!("READ_CORE_MEMORY {:08X} {}\n", self.script_ptr, 1)
    }
}

impl MapConnections {
    /// Populates this [`MapConnections`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_connections(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;
        self.count = get_i32(&buffer[index..index + 4]);
        index += 4;
        self.map_connection_ptr = get_u32(&buffer[index..index + 4]);
        self
    }
}

impl MapConnection {
    /// Populates this [`MapConnection`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_connection(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;
        self.direction = get_u8(&[buffer[index]]);
        index += 1;
        self.offset = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.map_group = get_u8(&[buffer[index]]);
        index += 1;
        self.map_number = get_u8(&[buffer[index]]);
        self
    }
}

impl MapScripts {
    /// Populates this [`MapScripts`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_script(mut self, buffer: &[&str]) -> Self {
        let index = 2;
        self.scripts = get_u8(&[buffer[index]]);
        self
    }
}

impl MapEvents {
    /// Populates this [`MapEvents`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_event(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;

        self.object_event_count = get_u8(&[buffer[index]]);
        index += 1;
        self.warp_count = get_u8(&[buffer[index]]);
        index += 1;
        self.coord_event_count = get_u8(&[buffer[index]]);
        index += 1;
        self.bg_event_count = get_u8(&[buffer[index]]);
        index += 1;
        self.object_event_template_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.warp_event_pointer = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.coord_event_pointer = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.bg_event_pointer = get_u32(&buffer[index..index + 4]);
        self
    }
}

impl MapLayout {
    /// Populates this [`MapLayout`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_layout(mut self, buffer: &[&str]) -> Self {
        let mut index: size_t = 2;

        self.width = get_i32(&buffer[index..index + 4]);
        index += 4;
        self.height = get_i32(&buffer[index..index + 4]);
        index += 4;
        self.border_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.map_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.tileset_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.secondary_tileset_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.border_width = get_u8(&[buffer[index]]);
        index += 1;
        self.border_height = get_u8(&[buffer[index]]);
        self
    }

    /// Returns a `READ_CORE_MEMORY` command string that reads one `u16` metatile
    /// entry from the border tile data at `border_ptr`
    pub fn generate_get_border_command(self) -> String {
        format!(
            "READ_CORE_MEMORY {:08X} {}\n",
            self.border_ptr,
            std::mem::size_of::<c_ushort>()
        )
    }

    /// Returns a `READ_CORE_MEMORY` command string that reads one `u16` metatile
    /// entry from the border tile data at `map_ptr`
    pub fn generate_get_map_command(self) -> String {
        format!(
            "READ_CORE_MEMORY {:08X} {}\n",
            self.map_ptr,
            std::mem::size_of::<c_ushort>()
        )
    }
}

impl MapHeader {
    /// Populates this [`MapHeader`] by parsing hex byte tokens from `buffer`
    /// 
    /// Parsing begins at index 2
    /// 
    /// # Arguments
    /// * `buffer` - Slice of hex byte strings as returned by `READ_CORE_MEMORY`
    pub fn fill_header(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;

        self.footer_offset_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.event_offset_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.script_offset_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.connections_offset_ptr = get_u32(&buffer[index..index + 4]);
        index += 4;
        self.music_id = get_u16(&buffer[index..index + 2]);
        index += 2;
        self.footer_id = get_u8(&[buffer[index]]);
        index += 1;
        self.footer_id_cont = get_u8(&[buffer[index]]);
        index += 1;
        self.name_index = get_u8(&[buffer[index]]);
        index += 1;
        self.cave_type = get_u8(&[buffer[index]]);
        index += 1;
        self.weather_type = get_u8(&[buffer[index]]);
        index += 1;
        self.trainer_battle_background_override = get_u8(&[buffer[index]]);
        index += 1;
        self.allow_bicycle = get_u8(&[buffer[index]]);
        index += 1;
        self.fill_allow_esc_run_map_name(&[buffer[index]]);
        index += 1;
        self.floor_number = get_u8(&[buffer[index]]);
        index += 1;
        self.battle_background_override = get_u8(&[buffer[index]]);

        self
    }

    /// Unpacks the three permission flags stored in a single byte (byte 26 of the header)
    /// 
    /// The byte is a bitfield.
    /// - Bit 2 (`0x04`) -> [`allow_escape`](MapHeader::allow_escape)
    /// - Bit 1 (`0x02`) -> [`allow_running`](MapHeader::allow_running)
    /// - Bit 0 (`0x01`) -> [`show_map_name`](MapHeader::show_map_name)
    /// 
    /// The remaining 5 bits are unused padding.
    /// 
    /// Takes `&mut self` rather than consuming `self` because it is called
    /// mid-parse from [`fill_header`](MapHeader::fill_header)
    /// 
    /// # Arguments
    /// * `buffer` - Single-element slice containing the packed byte token
    pub fn fill_allow_esc_run_map_name(&mut self, buffer: &[&str]) {
        let byte = get_u8(buffer);
        self.allow_escape = (byte & 4) == 4;
        self.allow_running = (byte & 2) == 2;
        self.show_map_name = (byte & 1) == 1;
    }
}

// ---------------------------------------------------
// Utility
// ---------------------------------------------------

/// Generates a `READ_CORE_MEMORY` command string to read `len` bytes starting
/// at the given GBA poitner.
/// 
/// Used throughout the codebase to produce follow-up emulator commands after
/// dereferencing a pointer field from a previously parsed struct.
/// 
/// # Arguments
/// * `ptr` - GBA memory address (typically 0x08xxxxxx for ROM or 0x02xxxxxx for EWRAM)
/// * `len` - Number of bytes to read.
/// 
/// # Returns
/// A newline-terminated command string ready to send to the emulator.
pub fn generate_follow_ptr_command(ptr: c_uint, len: size_t) -> String {
    format!("READ_CORE_MEMORY {:08X} {}\n", ptr, len)
}