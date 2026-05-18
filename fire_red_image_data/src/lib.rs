use image::{ImageBuffer, Rgba};

const ROM_BASE: u32 = 0x08000000;

const FRONT_SPRITE_TABLE_PTR: u32 = 0x128; // offset in ROM that holds the pointer
const PALETTE_TABLE_PTR: u32 = 0x130; // offset in ROM that holds the pointer
const SHINY_PALETTE_TABLE_PTR: u32 = 0x134; // offset for shiny pointer

fn read_entity_pointer(
    rom: &[u8],
    table_header: u32,
    species: u16,
    label: &str,
) -> Result<u32, Box<dyn std::error::Error>> {
    let table = read_table_pointer(rom, table_header)
        .ok_or_else(|| format!("failed to read {label} table pointer"))?;
    let ptr_addr = table
        .checked_add(species as u32 * 8)
        .ok_or("pointer address overflow")? as usize;
    let raw = u32::from_le_bytes(
        rom.get(ptr_addr..ptr_addr + 4)
            .ok_or("pointer address out of ROM bounds")?
            .try_into()?,
    );
    if raw < ROM_BASE || raw >= 0x09000000 {
        return Err(format!("invalid {label} pointer: {raw:#010X}").into());
    }
    Ok(raw - ROM_BASE)
}

fn read_table_pointer(rom: &[u8], header_offset: u32) -> Option<u32> {
    let o = header_offset as usize;
    let raw = u32::from_le_bytes(rom.get(o..o + 4)?.try_into().ok()?);
    if raw < ROM_BASE || raw >= 0x09000000 {
        eprintln!(
            "invalid table pointer at {:#X}: {:#010X}",
            header_offset, raw
        );
        return None;
    }
    Some(raw - ROM_BASE)
}

/// Read a pointer from the ROM pointer table for a given species
fn read_sprite_pointer(rom: &[u8], species: u16) -> Result<u32, Box<dyn std::error::Error>> {
    let table = read_table_pointer(rom, FRONT_SPRITE_TABLE_PTR)
        .ok_or("failed to read sprite table pointer")?;
    let ptr_addr = (table + (species as u32 * 8)) as usize;
    let raw = u32::from_le_bytes(
        rom.get(ptr_addr..ptr_addr + 4)
            .ok_or("sprite pointer address out of ROM bounds")?
            .try_into()?,
    );
    if raw < ROM_BASE || raw >= 0x09000000 {
        return Err("invalid sprite pointer".into());
    }
    Ok(raw - ROM_BASE)
}

/// Decompress GBA LZ77 data
pub fn decompress_lz77(rom: &[u8], offset: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut i = offset;

    // header: first byte should be 0x10 (LZ77 type)
    if rom.get(i) != Some(&0x10) {
        return Err("not LZ77 compressed data".into());
    }
    i += 1;

    // next 3 bytes are decrompressed size (little endian 24-bit)
    let size_bytes = rom.get(i..i + 3).ok_or("truncated LZ77 header")?;
    let decompressed_size =
        size_bytes[0] as usize | ((size_bytes[1] as usize) << 8) | ((size_bytes[2] as usize) << 16);
    i += 3;

    let mut output: Vec<u8> = Vec::with_capacity(decompressed_size);

    while output.len() < decompressed_size {
        let flags = *rom.get(i).ok_or("unexpected end of compressed data")?;
        i += 1;

        for bit in (0..8).rev() {
            if output.len() >= decompressed_size {
                break;
            }

            if (flags >> bit) & 1 == 0 {
                // literal byte
                output.push(*rom.get(i).ok_or("unexpected end of input in literal")?);
                i += 1;
            } else {
                // back reference
                let b0 = *rom.get(i).ok_or("unexpected end of input reading b0")? as usize;
                let b1 = *rom.get(i + 1).ok_or("unexpected end of input reading b1")? as usize;
                i += 2;

                let length = (b0 >> 4) + 3;
                let disp = ((b0 & 0xF) << 8) | b1;
                let start = output
                    .len()
                    .checked_sub(disp + 1)
                    .ok_or("LZ77 back-reference underflow")?;
                for j in 0..length {
                    let byte = *output
                        .get(start + j)
                        .ok_or("LZ77 back-reference read out of bounds")?;
                    output.push(byte);
                }
            }
        }
    }

    Ok(output)
}

/// Decode GBA 4bpp tiled graphcis into a flat pixel array (palette indices)
/// Tiles are 8x8 pixels, 4 bits per pixel
pub fn decode_4bpp_tiles(
    data: &[u8],
    width_tiles: usize,
    height_tiles: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let width = width_tiles * 8;
    let height = height_tiles * 8;
    let mut pixels = vec![0u8; width * height];

    let required = width_tiles * height_tiles * 32;
    if data.len() < required {
        return Err(format!("sprite data too short: need {required}, got {}", data.len()).into());
    }

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

    Ok(pixels)
}

pub fn decode_palette(
    rom: &[u8],
    offset: usize,
) -> Result<Vec<[u8; 4]>, Box<dyn std::error::Error>> {
    let mut palette = Vec::with_capacity(16);
    if rom.len() < offset + 32 {
        return Err("palette data too short".into());
    }
    for i in 0..16 {
        let raw = u16::from_le_bytes([rom[offset + i * 2], rom[offset + i * 2 + 1]]);
        let r5 = (raw & 0x1F) as u8;
        let g5 = ((raw >> 5) & 0x1F) as u8;
        let b5 = ((raw >> 10) & 0x1F) as u8;
        let r = (r5 << 3) | (r5 >> 2);
        let g = (g5 << 3) | (g5 >> 2);
        let b = (b5 << 3) | (b5 >> 2);
        let a = if i == 0 { 0 } else { 255 };
        palette.push([r, g, b, a]);
    }
    Ok(palette)
}

/// Get a pokemon front sprite as an RGBA image buffer
pub fn get_pokemon_sprite(
    rom: &[u8],
    species: u16,
    shiny: bool,
) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>, Box<dyn std::error::Error>> {
    let sprite_offset = read_sprite_pointer(rom, species)? as usize;

    let palette_offset = if shiny {
        read_entity_pointer(rom, SHINY_PALETTE_TABLE_PTR, species, "shiny palette")? as usize
    } else {
        read_entity_pointer(rom, PALETTE_TABLE_PTR, species, "palette")? as usize
    };

    let sprite_data = decompress_lz77(rom, sprite_offset)?;
    let palette_data = decompress_lz77(rom, palette_offset)?;
    let pixels = decode_4bpp_tiles(&sprite_data, 8, 8)?;

    let mut img = ImageBuffer::new(64, 64);
    for (i, palette_index) in pixels.iter().copied().enumerate() {
        let palette = decode_palette(&palette_data, 0)?;
        let x = (i % 64) as u32;
        let y = (i / 64) as u32;
        let color = palette
            .get(palette_index as usize)
            .copied()
            .unwrap_or([0, 0, 0, 0]);
        img.put_pixel(x, y, Rgba(color));
    }
    Ok(img)
}
