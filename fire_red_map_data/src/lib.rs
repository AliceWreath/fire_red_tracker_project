//! # FireRed Map Data Structures
//!
//! `#[repr(C)]` structs that mirror the in-memory layout of Pokémon FireRed map
//! data, along with methods for deserializing them from raw ROM bytes.
//!
//! ## Parsing convention
//!
//! [`MapHeader::fill_from_bytes`] takes a raw `&[u8]` ROM slice and a map offset
//! (GBA bus address minus `0x08000000`).  All pointer fields are stored as raw
//! GBA bus addresses; subtract `ROM_BASE` (`0x08000000`) before using them as
//! ROM slice indices.

use fire_red_get_values::*;
#[cfg(feature = "retroarch-parser")]
use libc::size_t;
#[cfg(feature = "retroarch-parser")]
use std::os::raw::{c_int, c_short};
use std::os::raw::{c_uchar, c_uint, c_ushort};

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

/// Top-level descriptor for a single map, stored at the map table entry.
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
    pub event_offset_ptr: c_uint, // Byte 5-8
    /// Pointer to the map's script table
    pub script_offset_ptr: c_uint, // Byte 9-12
    /// Pointer to the [`MapConnections`] struct, or 0 if there are none
    pub connections_offset_ptr: c_uint, // Byte 13-16
    /// BGM track ID played on this map
    pub music_id: c_ushort, // Byte 17-18
    /// Lower byte of the map layout (footer) ID
    pub footer_id: c_uchar, // Byte 19
    /// Upper byte of the map layout (footer) ID
    pub footer_id_cont: c_uchar, // Byte 20
    /// Index into the map-name string table shown on the location banner.
    pub name_index: c_uchar, // Byte 21
    /// Cave/dungeon type flag; controls lighting and wild encounter music.
    pub cave_type: c_uchar, // Byte 22
    /// Weather effect index (rain, snow, sandstorm, etc.)
    pub weather_type: c_uchar, // Byte 23
    /// Overrides the default trainer battle background for this map.
    pub trainer_battle_background_override: c_uchar, // Byte 24
    /// Non-zero if the player can use a bicycle here.
    pub allow_bicycle: c_uchar, // Byte 25
    /// Bit 2 of byte 26: player can use Escape Rope / Dig.
    pub allow_escape: bool, // Byte 26
    /// Bit 1 of byte 26: player can run (hold B).
    pub allow_running: bool, // Byte 26
    /// Bit 0 of byte 26: show the location name banner on map entry.
    pub show_map_name: bool, // Byte 26 + 5 unused bits
    /// Floor number displayed in multi-floor dungeons (e.g. "B1F")
    pub floor_number: c_uchar, // Byte 27
    /// Overrides the wild battle background; values 0x00-0x09 are standard
    /// 0x0A and above produce undefined behaviour.
    pub battle_background_override: c_uchar, // Byte 28
}

// ------------------------------------------------
// Map events
// ------------------------------------------------

/// A background event (hidden item, secret base entrance, sign, etc.)
///
/// Only available with the `retroarch-parser` feature.
#[cfg(feature = "retroarch-parser")]
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
#[cfg(feature = "retroarch-parser")]
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
#[cfg(feature = "retroarch-parser")]
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
    /// Map number within the destination group
    pub map_num: c_uchar,
    /// destination map group
    pub map_group: c_uchar,
}

// --------------------------------------------
// Map connections
// --------------------------------------------

/// Header for the list of adjacent-map connections attached to a map.
///
/// Only available with the `retroarch-parser` feature.
#[cfg(feature = "retroarch-parser")]
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapConnections {
    /// Number of [`MapConnection`] entries in the list
    count: c_int,
    /// Pointer to the fires [`MapConnection`] entry
    map_connection_ptr: c_uint,
}

/// A single directional connection to the adjacent map.
#[cfg(feature = "retroarch-parser")]
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
/// Only available with the `retroarch-parser` feature.
#[cfg(feature = "retroarch-parser")]
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapScripts {
    scripts: c_uchar,
}

/// Counts and pointers for all event lists on a map.
///
/// Only available with the `retroarch-parser` feature.
#[cfg(feature = "retroarch-parser")]
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
/// Only available with the `retroarch-parser` feature.
#[cfg(feature = "retroarch-parser")]
#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapLayout {
    // aka footer
    /// Map width in tiles
    pub width: c_int, // map width
    /// Map height in tiles
    pub height: c_int, // map height
    /// pointer to the border tile block (shown outside the playable area)
    pub border_ptr: c_uint, // ptr to the borders
    /// pointer to the map tile data (array of `c_ushort` metatile indices)
    pub map_ptr: c_uint, // ptr to the map?
    /// pointer to the primary tileset struct
    pub tileset_ptr: c_uint, // ptr to primary tileset struct
    /// pointer to the secondary tileset struct
    pub secondary_tileset_ptr: c_uint, // ptr to secondary tileset struct
    /// width of the border tile region in tiles.
    pub border_width: c_uchar, // border width
    /// height of the border tile region in tiles.
    pub border_height: c_uchar, // border height
}

// ------------------------------------------------------------------------
// Object event template (NPC / overworld object)
// ------------------------------------------------------------------------

/// Template used to spawn an overworld object (NPC, item ball, etc.)
///
/// Only available with the `retroarch-parser` feature.
#[cfg(feature = "retroarch-parser")]
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

#[cfg(feature = "retroarch-parser")]
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

    /// Reads a [`BgEvent`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: x(2) y(2) elevation(1) kind(1) script_ptr(4) hidden_items(4).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        Self {
            x: read_u16(buffer, o),
            y: read_u16(buffer, o + 2),
            elevation: read_u8(buffer, o + 4),
            kind: read_u8(buffer, o + 5),
            script_ptr: read_u32(buffer, o + 6),
            hidden_items: read_u32(buffer, o + 10),
        }
    }

    /// Returns a `READ_CORE_MEMORY` command string that follows `script_ptr`
    /// to read the first byte of the event's script.
    pub fn generate_get_script_command(self) -> String {
        generate_follow_ptr_command(self.script_ptr, 1)
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads a [`CoordEvent`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: x(2) y(2) elevation(1) trigger(2) index(2) script_ptr(4).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        Self {
            x: read_u16(buffer, o),
            y: read_u16(buffer, o + 2),
            elevation: read_u8(buffer, o + 4),
            trigger: read_u16(buffer, o + 5),
            index: read_u16(buffer, o + 7),
            script_ptr: read_u32(buffer, o + 9),
        }
    }

    /// Returns a `READ_CORE_MEMORY` command string that reads the first byte
    /// of this coordinate event's script
    pub fn generate_get_script_command(self) -> String {
        format!("READ_CORE_MEMORY {:08X} {}\n", self.script_ptr, 1)
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads a [`WarpEvent`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: x(2) y(2) elevation(1) warp_id(1) map_num(1) map_group(1).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        Self {
            x: read_i16(buffer, o),
            y: read_i16(buffer, o + 2),
            elevation: read_u8(buffer, o + 4),
            warp_id: read_u8(buffer, o + 5),
            map_num: read_u8(buffer, o + 6),
            map_group: read_u8(buffer, o + 7),
        }
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads an [`ObjectEventTemplate`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: local_id(1) graphics_id(1) in_connection(1)
    /// x(2) y(2) elevation(1) movement_type(1) movement_range_x(2) movement_range_y(2)
    /// trainer_type(2) trainer_range_berry_tree_id(2) script_ptr(4) flag_id(2).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        Self {
            local_id: read_u8(buffer, o),
            graphics_id: read_u8(buffer, o + 1),
            in_connection: read_u8(buffer, o + 2),
            x: read_i16(buffer, o + 3),
            y: read_i16(buffer, o + 5),
            elevation: read_u8(buffer, o + 7),
            movement_type: read_u8(buffer, o + 8),
            movement_range_x: read_u16(buffer, o + 9),
            movement_range_y: read_u16(buffer, o + 11),
            trainer_type: read_u16(buffer, o + 13),
            trainer_range_berry_tree_id: read_u16(buffer, o + 15),
            script_ptr: read_u32(buffer, o + 17),
            flag_id: read_u16(buffer, o + 21),
        }
    }

    /// Returns a `READ_CORE_MEMORY` command string that reads the first byte
    /// of this object's interaction script.
    pub fn generate_get_script_command(self) -> String {
        format!("READ_CORE_MEMORY {:08X} {}\n", self.script_ptr, 1)
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads a [`MapConnections`] header from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: count(4) map_connection_ptr(4).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        Self {
            count: read_i32(buffer, offset),
            map_connection_ptr: read_u32(buffer, offset + 4),
        }
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads a [`MapConnection`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: direction(1) offset(4) map_group(1) map_number(1).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        Self {
            direction: read_u8(buffer, o),
            offset: read_u32(buffer, o + 1),
            map_group: read_u8(buffer, o + 5),
            map_number: read_u8(buffer, o + 6),
        }
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads the first script-table byte from a raw byte buffer at `offset`.
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        Self {
            scripts: read_u8(buffer, offset),
        }
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads a [`MapEvents`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: object_event_count(1) warp_count(1) coord_event_count(1)
    /// bg_event_count(1) object_event_template_ptr(4) warp_event_pointer(4)
    /// coord_event_pointer(4) bg_event_pointer(4).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        Self {
            object_event_count: read_u8(buffer, o),
            warp_count: read_u8(buffer, o + 1),
            coord_event_count: read_u8(buffer, o + 2),
            bg_event_count: read_u8(buffer, o + 3),
            object_event_template_ptr: read_u32(buffer, o + 4),
            warp_event_pointer: read_u32(buffer, o + 8),
            coord_event_pointer: read_u32(buffer, o + 12),
            bg_event_pointer: read_u32(buffer, o + 16),
        }
    }
}

#[cfg(feature = "retroarch-parser")]
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

    /// Reads a [`MapLayout`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially: width(4) height(4) border_ptr(4) map_ptr(4)
    /// tileset_ptr(4) secondary_tileset_ptr(4) border_width(1) border_height(1).
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        Self {
            width: read_i32(buffer, o),
            height: read_i32(buffer, o + 4),
            border_ptr: read_u32(buffer, o + 8),
            map_ptr: read_u32(buffer, o + 12),
            tileset_ptr: read_u32(buffer, o + 16),
            secondary_tileset_ptr: read_u32(buffer, o + 20),
            border_width: read_u8(buffer, o + 24),
            border_height: read_u8(buffer, o + 25),
        }
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
    /// Populates this [`MapHeader`] from a Retroarch string-buffer response.
    #[cfg(feature = "retroarch-parser")]
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

    /// Unpacks the three permission flags packed into byte 26 of the header.
    #[cfg(feature = "retroarch-parser")]
    pub fn fill_allow_esc_run_map_name(&mut self, buffer: &[&str]) {
        let byte = get_u8(buffer);
        self.allow_escape = (byte & 4) == 4;
        self.allow_running = (byte & 2) == 2;
        self.show_map_name = (byte & 1) == 1;
    }

    /// Reads a [`MapHeader`] from a raw byte buffer at `offset`.
    ///
    /// Bytes are read sequentially matching the 28-byte GBA map header layout.
    /// Byte 25 (relative to `offset`) is a bitfield:
    /// bit 2 = allow_escape, bit 1 = allow_running, bit 0 = show_map_name.
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        let o = offset;
        let flags = read_u8(buffer, o + 25);
        Self {
            footer_offset_ptr: read_u32(buffer, o),
            event_offset_ptr: read_u32(buffer, o + 4),
            script_offset_ptr: read_u32(buffer, o + 8),
            connections_offset_ptr: read_u32(buffer, o + 12),
            music_id: read_u16(buffer, o + 16),
            footer_id: read_u8(buffer, o + 18),
            footer_id_cont: read_u8(buffer, o + 19),
            name_index: read_u8(buffer, o + 20),
            cave_type: read_u8(buffer, o + 21),
            weather_type: read_u8(buffer, o + 22),
            trainer_battle_background_override: read_u8(buffer, o + 23),
            allow_bicycle: read_u8(buffer, o + 24),
            allow_escape: (flags & 4) == 4,
            allow_running: (flags & 2) == 2,
            show_map_name: (flags & 1) == 1,
            floor_number: read_u8(buffer, o + 26),
            battle_background_override: read_u8(buffer, o + 27),
        }
    }
}

impl CurrentMapGroupAndName {
    /// Reads the two-byte `(group, name)` field from a raw byte buffer at `offset`.
    #[cfg(feature = "retroarch-parser")]
    pub fn fill_from_bytes(buffer: &[u8], offset: usize) -> Self {
        Self {
            group: read_u8(buffer, offset),
            name: read_u8(buffer, offset + 1),
        }
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
#[cfg(feature = "retroarch-parser")]
pub fn generate_follow_ptr_command(ptr: c_uint, len: size_t) -> String {
    format!("READ_CORE_MEMORY {:08X} {}\n", ptr, len)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a 28-byte map header at `offset` inside a padded buffer so the
    /// offset-based reads are exercised, not just offset 0.
    fn sample_header_bytes(offset: usize) -> Vec<u8> {
        let mut buf = vec![0xEEu8; offset];
        buf.extend_from_slice(&0x0834_5678u32.to_le_bytes()); // footer ptr
        buf.extend_from_slice(&0x0812_0000u32.to_le_bytes()); // events ptr
        buf.extend_from_slice(&0x0800_0010u32.to_le_bytes()); // scripts ptr
        buf.extend_from_slice(&0u32.to_le_bytes()); // connections ptr (none)
        buf.extend_from_slice(&0x012Cu16.to_le_bytes()); // music_id = 300
        buf.extend_from_slice(&[
            0x2A, // footer_id
            0x01, // footer_id_cont
            88,   // name_index
            1,    // cave_type
            2,    // weather_type
            3,    // trainer_battle_background_override
            1,    // allow_bicycle
            0b0000_0110, // flags: escape + running, no map name banner
            4,    // floor_number
            5,    // battle_background_override
        ]);
        buf
    }

    #[test]
    fn map_header_fill_from_bytes_reads_all_fields() {
        let buf = sample_header_bytes(0x40);
        let h = MapHeader::fill_from_bytes(&buf, 0x40);
        assert_eq!(h.footer_offset_ptr, 0x0834_5678);
        assert_eq!(h.event_offset_ptr, 0x0812_0000);
        assert_eq!(h.script_offset_ptr, 0x0800_0010);
        assert_eq!(h.connections_offset_ptr, 0);
        assert_eq!(h.music_id, 300);
        assert_eq!(h.footer_id, 0x2A);
        assert_eq!(h.footer_id_cont, 0x01);
        assert_eq!(h.name_index, 88);
        assert_eq!(h.cave_type, 1);
        assert_eq!(h.weather_type, 2);
        assert_eq!(h.trainer_battle_background_override, 3);
        assert_eq!(h.allow_bicycle, 1);
        assert_eq!(h.floor_number, 4);
        assert_eq!(h.battle_background_override, 5);
    }

    #[test]
    fn map_header_flag_bits_unpack_independently() {
        let mut buf = sample_header_bytes(0);
        for flags in 0..8u8 {
            buf[25] = flags;
            let h = MapHeader::fill_from_bytes(&buf, 0);
            assert_eq!(h.allow_escape, flags & 4 != 0, "escape bit, flags={flags:03b}");
            assert_eq!(h.allow_running, flags & 2 != 0, "running bit, flags={flags:03b}");
            assert_eq!(h.show_map_name, flags & 1 != 0, "map-name bit, flags={flags:03b}");
        }
    }

    #[test]
    fn map_header_short_buffer_reads_zeroes_without_panicking() {
        // read_* helpers return 0 out of bounds, so a truncated buffer must
        // produce a zeroed header rather than a panic.
        let h = MapHeader::fill_from_bytes(&[0xAB; 4], 0);
        assert_eq!(h.footer_offset_ptr, 0xABAB_ABAB);
        assert_eq!(h.event_offset_ptr, 0);
        assert_eq!(h.music_id, 0);
        assert!(!h.allow_escape && !h.allow_running && !h.show_map_name);
    }
}

/// Byte-layout tests for the `retroarch-parser` structs. Run with
/// `cargo test -p fire_red_map_data --features retroarch-parser`
/// to verify the legacy parsers haven't bitrotted.
#[cfg(all(test, feature = "retroarch-parser"))]
mod retroarch_parser_tests {
    use super::*;

    #[test]
    fn map_events_fill_from_bytes_reads_counts_and_pointers() {
        let mut buf = vec![2u8, 4, 6, 8];
        buf.extend_from_slice(&0x0811_1111u32.to_le_bytes());
        buf.extend_from_slice(&0x0822_2222u32.to_le_bytes());
        buf.extend_from_slice(&0x0833_3333u32.to_le_bytes());
        buf.extend_from_slice(&0x0844_4444u32.to_le_bytes());
        let e = MapEvents::fill_from_bytes(&buf, 0);
        assert_eq!(e.object_event_count, 2);
        assert_eq!(e.warp_count, 4);
        assert_eq!(e.coord_event_count, 6);
        assert_eq!(e.bg_event_count, 8);
        assert_eq!(e.object_event_template_ptr, 0x0811_1111);
        assert_eq!(e.warp_event_pointer, 0x0822_2222);
        assert_eq!(e.coord_event_pointer, 0x0833_3333);
        assert_eq!(e.bg_event_pointer, 0x0844_4444);
    }

    #[test]
    fn warp_event_fill_from_bytes_reads_signed_coordinates() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(-3i16).to_le_bytes());
        buf.extend_from_slice(&25i16.to_le_bytes());
        buf.extend_from_slice(&[3, 1, 12, 4]);
        let w = WarpEvent::fill_from_bytes(&buf, 0);
        assert_eq!(w.x, -3);
        assert_eq!(w.y, 25);
        assert_eq!(w.elevation, 3);
        assert_eq!(w.warp_id, 1);
        assert_eq!(w.map_num, 12);
        assert_eq!(w.map_group, 4);
    }

    #[test]
    fn map_connection_fill_from_bytes_reads_unaligned_offset_field() {
        let mut buf = vec![1u8]; // direction = north
        buf.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf.extend_from_slice(&[3, 7]);
        let c = MapConnection::fill_from_bytes(&buf, 0);
        assert_eq!(c.direction, 1);
        assert_eq!(c.offset, 0xDEAD_BEEF);
        assert_eq!(c.map_group, 3);
        assert_eq!(c.map_number, 7);
    }

    #[test]
    fn object_event_template_fill_from_bytes_reads_all_fields() {
        let mut buf = vec![5u8, 60, 0];
        buf.extend_from_slice(&(-2i16).to_le_bytes()); // x
        buf.extend_from_slice(&9i16.to_le_bytes()); // y
        buf.extend_from_slice(&[3, 8]); // elevation, movement_type
        buf.extend_from_slice(&2u16.to_le_bytes()); // movement_range_x
        buf.extend_from_slice(&1u16.to_le_bytes()); // movement_range_y
        buf.extend_from_slice(&3u16.to_le_bytes()); // trainer_type
        buf.extend_from_slice(&4u16.to_le_bytes()); // trainer_range_berry_tree_id
        buf.extend_from_slice(&0x0855_AA00u32.to_le_bytes()); // script_ptr
        buf.extend_from_slice(&0x0123u16.to_le_bytes()); // flag_id
        let t = ObjectEventTemplate::fill_from_bytes(&buf, 0);
        assert_eq!(t.local_id, 5);
        assert_eq!(t.graphics_id, 60);
        assert_eq!(t.in_connection, 0);
        assert_eq!(t.x, -2);
        assert_eq!(t.y, 9);
        assert_eq!(t.elevation, 3);
        assert_eq!(t.movement_type, 8);
        assert_eq!(t.movement_range_x, 2);
        assert_eq!(t.movement_range_y, 1);
        assert_eq!(t.trainer_type, 3);
        assert_eq!(t.trainer_range_berry_tree_id, 4);
        assert_eq!(t.script_ptr, 0x0855_AA00);
        assert_eq!(t.flag_id, 0x0123);
    }

    #[test]
    fn generate_follow_ptr_command_formats_address() {
        assert_eq!(
            generate_follow_ptr_command(0x0812_3456, 16),
            "READ_CORE_MEMORY 08123456 16\n"
        );
    }
}
