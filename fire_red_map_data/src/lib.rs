use fire_red_get_values::*;
use libc::size_t;
use std::os::raw::{c_int, c_short, c_uchar, c_uint, c_ushort};

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct CurrentMapGroupAndName {
    pub group: c_uchar,
    pub name: c_uchar,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapHeader {
    pub footer_offset_ptr: c_uint, // Byte 1-4  this is a pointer to the MapLayout struct
    pub event_offset_ptr: c_uint,  // Byte 5-8
    pub script_offset_ptr: c_uint, // Byte 9-12
    pub connections_offset_ptr: c_uint, // Byte 13-16

    pub music_id: c_ushort,      // Byte 17-18 in reverse hex form
    pub footer_id: c_uchar,      // Byte 19
    pub footer_id_cont: c_uchar, // Byte 20
    pub name_index: c_uchar,     // Byte 21
    pub cave_type: c_uchar,      // Byte 22
    pub weather_type: c_uchar,   // Byte 23
    pub trainer_battle_background_override: c_uchar, // Byte 24
    pub allow_bicycle: c_uchar,  // Byte 25
    pub allow_escape: bool,      // Byte 26
    pub allow_running: bool,     // Byte 26
    pub show_map_name: bool,     // Byte 26 + 5 unused bits
    pub floor_number: c_uchar,   // Byte 27
    pub battle_background_override: c_uchar, // Byte 28: 00 - 09 are standard, 0A and higher do weirdness
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct BgEvent {
    pub x: c_ushort,
    pub y: c_ushort,
    pub elevation: c_uchar,
    pub kind: c_uchar,

    // union
    pub script_ptr: c_uint, //points to u8
    pub hidden_items: c_uint,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct CoordEvent {
    pub x: c_ushort,
    pub y: c_ushort,
    pub elevation: c_uchar,
    pub trigger: c_ushort,
    pub index: c_ushort,
    pub script_ptr: c_uint, // points to u8
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct WarpEvent {
    pub x: c_short,
    pub y: c_short,
    pub elevation: c_uchar,
    pub warp_id: c_uchar,
    pub map_num: c_uchar,
    pub map_group: c_uchar,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapConnections {
    count: c_int,
    map_connection_ptr: c_uint,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapConnection {
    pub direction: c_uchar,
    pub offset: c_uint,
    pub map_group: c_uchar,
    pub map_number: c_uchar,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapScripts {
    scripts: c_uchar,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapEvents {
    pub object_event_count: c_uchar,
    pub warp_count: c_uchar,
    pub coord_event_count: c_uchar,
    pub bg_event_count: c_uchar,
    pub object_event_template_ptr: c_uint,
    pub warp_event_pointer: c_uint,
    pub coord_event_pointer: c_uint,
    pub bg_event_pointer: c_uint,
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct MapLayout {
    // aka footer
    pub width: c_int,                  // map width
    pub height: c_int,                 // map height
    pub border_ptr: c_uint,            // ptr to the borders
    pub map_ptr: c_uint,               // ptr to the map?
    pub tileset_ptr: c_uint,           // ptr to primary tileset struct
    pub secondary_tileset_ptr: c_uint, // ptr to secondary tileset struct
    pub border_width: c_uchar,         // border width
    pub border_height: c_uchar,        // border height
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct ObjectEventTemplate {
    pub local_id: c_uchar,
    pub graphics_id: c_uchar,
    pub in_connection: c_uchar,
    pub x: c_short,
    pub y: c_short,
    pub elevation: c_uchar,
    pub movement_type: c_uchar,
    pub movement_range_x: c_ushort,
    pub movement_range_y: c_ushort,
    pub trainer_type: c_ushort,
    pub trainer_range_berry_tree_id: c_ushort,
    pub script_ptr: c_uint,
    pub flag_id: c_ushort,
}

impl BgEvent {
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

    pub fn generate_get_script_command(self) -> String {
        generate_follow_ptr_command(self.script_ptr, 1)
    }
}

impl CoordEvent {
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

    pub fn generate_get_script_command(self) -> String {
        format!("READ_CORE_MEMORY {:08X} {}\n", self.script_ptr, 1)
    }
}

impl WarpEvent {
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
    pub fn generate_get_script_command(self) -> String {
        format!("READ_CORE_MEMORY {:08X} {}\n", self.script_ptr, 1)
    }
}

impl MapConnections {
    pub fn fill_connections(mut self, buffer: &[&str]) -> Self {
        let mut index = 2;
        self.count = get_i32(&buffer[index..index + 4]);
        index += 4;
        self.map_connection_ptr = get_u32(&buffer[index..index + 4]);
        self
    }
}

impl MapConnection {
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
    pub fn fill_script(mut self, buffer: &[&str]) -> Self {
        let index = 2;
        self.scripts = get_u8(&[buffer[index]]);
        self
    }
}

impl MapEvents {
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
    pub fn generate_get_border_command(self) -> String {
        format!(
            "READ_CORE_MEMORY {:08X} {}\n",
            self.border_ptr,
            std::mem::size_of::<c_ushort>()
        )
    }
    pub fn generate_get_map_command(self) -> String {
        format!(
            "READ_CORE_MEMORY {:08X} {}\n",
            self.map_ptr,
            std::mem::size_of::<c_ushort>()
        )
    }
}

impl MapHeader {
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

    pub fn fill_allow_esc_run_map_name(&mut self, buffer: &[&str]) {
        let byte = get_u8(buffer);
        self.allow_escape = (byte & 4) == 4;
        self.allow_running = (byte & 2) == 2;
        self.show_map_name = (byte & 1) == 1;
    }
}

pub fn generate_follow_ptr_command(ptr: c_uint, len: size_t) -> String {
    format!("READ_CORE_MEMORY {:08X} {}\n", ptr, len)
}