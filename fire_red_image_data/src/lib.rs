use image::{ImageBuffer, Rgba};

const ROM_BASE: u32 = 0x08000000;

const FRONT_SPRITE_TABLE_PTR: u32 = 0x128; // offset in ROM that holds the pointer
const PALETTE_TABLE_PTR: u32 = 0x130;      // offset in ROM that holds the pointer
const SHINY_PALETTE_TABLE_PTR: u32 = 0x134; // offset for shiny pointer

fn read_table_pointer(rom: &[u8], header_offset: u32) -> u32 {
    let raw = u32::from_le_bytes([
        rom[header_offset as usize],
        rom[header_offset as usize + 1],
        rom[header_offset as usize + 2],
        rom[header_offset as usize + 3],
    ]);
    if raw < 0x08000000 || raw >= 0x09000000 {
        panic!("invalid table pointer at header offset {:#X}: {:#010X}", header_offset, raw);
    }
    raw - ROM_BASE
}

/// Read a pointer from the ROM pointer table for a given species
fn read_sprite_pointer(rom: &[u8], species: u16) -> u32 {
    let table = read_table_pointer(rom, FRONT_SPRITE_TABLE_PTR);
    let ptr_addr = (table + (species as u32 * 8)) as usize;
    let raw = u32::from_le_bytes([rom[ptr_addr], rom[ptr_addr+1], rom[ptr_addr+2], rom[ptr_addr+3]]);
    if raw < ROM_BASE || raw >= 0x09000000 {
        panic!("invalid sprite pointer: {:#010X}", raw);
    }
    raw - ROM_BASE
}

fn read_palette_pointer(rom: &[u8], species: u16) -> u32 {
    let table = read_table_pointer(rom, PALETTE_TABLE_PTR);
    let ptr_addr = (table + (species as u32 * 8)) as usize;
    let raw = u32::from_le_bytes([rom[ptr_addr], rom[ptr_addr+1], rom[ptr_addr+2], rom[ptr_addr+3]]);
    if raw < ROM_BASE || raw >= 0x09000000 {
        panic!("invalid palette pointer: {:#010X}", raw);
    }
    raw - ROM_BASE
}

fn read_shiny_palette_pointer(rom: &[u8], species: u16) -> u32 {
    let table = read_table_pointer(rom, SHINY_PALETTE_TABLE_PTR);
    let ptr_addr = (table + (species as u32 * 8)) as usize;
    let raw = u32::from_le_bytes([rom[ptr_addr], rom[ptr_addr+1], rom[ptr_addr+2], rom[ptr_addr+3]]);
    if raw < 0x08000000 || raw >= 0x09000000 {
        panic!("invalid shiny palette pointer: {:#010X}", raw);
    }
    raw - 0x08000000
}

/// Decompress GBA LZ77 data
pub fn decompress_lz77(rom: &[u8], offset: usize) -> Vec<u8> {
    let mut i = offset;

    // header: first byte should be 0x10 (LZ77 type)
    assert_eq!(rom[i], 0x10, "not LZ77 compressed data"); // change from assert at some point
    i += 1;

    // next 3 bytes are decrompressed size (little endian 24-bit)
    let decompressed_size = rom[i] as usize
        | ((rom[i + 1] as usize) << 8)
        | ((rom[i + 2] as usize) << 16);
    i += 3;

    let mut output: Vec<u8> = Vec::with_capacity(decompressed_size);

    while output.len() < decompressed_size {
        let flags = rom[i];
        i += 1;

        for bit in (0..8).rev() {
            if output.len() >= decompressed_size {
                break;
            }

            if (flags >> bit) & 1 == 0 {
                // literal byte
                output.push(rom[i]);
                i += 1;
            } else {
                // back reference
                let b0 = rom[i] as usize;
                let b1 = rom[i + 1] as usize;
                i += 2;

                let length = (b0 >> 4) + 3;
                let disp = ((b0 & 0xF) << 8) | b1;
                let start = output.len() - disp - 1;

                for j in 0..length {
                    let byte = output[start + j];
                    output.push(byte);
                }
            }
        }
    }

    output
}

/// Decode GBA 4bpp tiled graphcis into a flat pixel array (palette indices)
/// Tiles are 8x8 pixels, 4 bits per pixel
pub fn decode_4bpp_tiles(data: &[u8], width_tiles: usize, height_tiles: usize) -> Vec<u8> {
    let width = width_tiles * 8;
    let height = height_tiles * 8;
    let mut pixels = vec![0u8; width * height];

    for tile_y in 0..height_tiles {
        for tile_x in 0..width_tiles {
            let tile_index = tile_y * width_tiles + tile_x;
            let tile_offset = tile_index * 32; // 32 bytes per 8x8 4bpp tile

            for row in 0..8 {
                for col in 0..4 {
                    let byte = data[tile_offset + row * 4 + col];
                    let lo = byte & 0xF;
                    let hi = (byte >> 4) & 0xF;

                    let px = tile_x * 8 + col * 2;
                    let py = tile_y * 8 + row;

                    pixels[py * width + px] = lo;
                    pixels[py * width + px + 1] = hi;
                }
            }
        }
    }

    pixels
}

/// Decode GBA palette (16 colors, 2 bytes each BGR555 format)
/*pub fn decode_palette(rom: &[u8], offset: usize) -> Vec<[u8; 4]> {
    let mut palette = Vec::with_capacity(16);
    for i in 0..16 {
        let raw = u16::from_le_bytes([rom[offset + i*2], rom[offset + i*2 + 1]]);
        let r5 = (raw & 0x1F) as u8;
        let g5 = ((raw >> 5) & 0x1F) as u8;
        let b5 = ((raw >> 10) & 0x1F) as u8;
        // fill low bits to scale 0-31 → 0-255 properly
        let r = (r5 << 3) | (r5 >> 2);
        let g = (g5 << 3) | (g5 >> 2);
        let b = (b5 << 3) | (b5 >> 2);
        let a = if i == 0 { 0 } else { 255 };
        palette.push([r, g, b, a]);
    }
    palette
}*/

pub fn decode_palette(rom: &[u8], offset: usize) -> Vec<[u8; 4]> {
    let mut palette = Vec::with_capacity(16);
    for i in 0..16 {
        let raw = u16::from_le_bytes([rom[offset + i*2], rom[offset + i*2 + 1]]);
        let r5 = (raw & 0x1F) as u8;
        let g5 = ((raw >> 5) & 0x1F) as u8;
        let b5 = ((raw >> 10) & 0x1F) as u8;
        let r = (r5 << 3) | (r5 >> 2);
        let g = (g5 << 3) | (g5 >> 2);
        let b = (b5 << 3) | (b5 >> 2);
        let a = if i == 0 { 0 } else { 255 };
        palette.push([r, g, b, a]);
    }
    palette
}

/// Get a pokemon front sprite as an RGBA image buffer
pub fn get_pokemon_sprite(rom: &[u8], species: u16, shiny: bool) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let sprite_offset = read_sprite_pointer(rom, species) as usize;
    
    let palette_offset = if shiny {
        read_shiny_palette_pointer(rom, species) as usize
    } else {
        read_palette_pointer(rom, species) as usize
    };

    let sprite_data = decompress_lz77(rom, sprite_offset);
    let palette_data = decompress_lz77(rom, palette_offset);
    let palette = decode_palette(&palette_data, 0);
    let pixels = decode_4bpp_tiles(&sprite_data, 8, 8);

    let mut img = ImageBuffer::new(64, 64);
    for (i, &palette_index) in pixels.iter().enumerate() {
        let x = (i % 64) as u32;
        let y = (i / 64) as u32;
        let color = palette[palette_index as usize];
        img.put_pixel(x, y, Rgba(color));
    }
    img
}