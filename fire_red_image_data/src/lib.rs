//! # FireRed Image Data
//! 
//! Extracts and decodes pokemon front sprites from a FireRed ROM into RGBA
//! [`ImageBuffer`]s ready for upload to the GPU.
//! 
//! ## GBA graphics pipeline
//! 
//! FireRed stores sprites and palettes as LZ77-compressed blobs. The decoding
//! pipeline is:
//! 
//! 1. **Locate** the sprite / palette pointer via the ROM's indirection tables
//!    ([`read_sprite_pointer`] / [`read_entity_pointer`]).
//! 2. **Decompress** the blob with [`decompress_lz77`].
//! 3. **Decode tiles** from 4bpp GBA tile format into a flat palette-index
//!    array with [`decode_4bpp_tiles`]
//! 4. **Resolved palette** entires from 15-bit BGR555 to 8-bit-per-channel RGBA
//!    with [`decode_palette`]
//! 5. **Assemble** the final [`ImageBuffer`] in [`get_pokemon_sprite`]
//! 
//! All pokemon front sprites in FireRed are 64x64 pixles (8x8 tiles of 8x8 pixels each),
//! stored in 4bpp tiled format with a 16-color palette.
use image::{ImageBuffer, Rgba};

// ---------------------------------------------------------------------------
// ROM layout constants
// ---------------------------------------------------------------------------

/// Base address of the GBA ROM in teh cartridge address space.
/// 
/// All ROM pointers stored in the file are absolute GBA addresses; subtracting
/// this constant converts them to byte offsets usable for direct slice indexing.
const ROM_BASE: u32 = 0x08000000;

/// Byte offset within the ROM where the front-sprite pointer table pointer is stored.
/// 
/// Dereferencing this yields the address of the table that maps species indices 
/// to compressed sprite data pointers (8 bytes per entry).
const FRONT_SPRITE_TABLE_PTR: u32 = 0x128;

/// Byte offset within the ROM where the normal palette pointer table pointer is stored
/// 
/// Each entry in the resolved table points to a compressed 16-color BGR555 palette.
const PALETTE_TABLE_PTR: u32 = 0x130; 

/// Byte offset within the ROM where the shiny palette pointer table pointer is stored.
/// 
/// Parallel structure to [`PALLET_TABLE_PTR`] but for alternate shiny palettes.
const SHINY_PALETTE_TABLE_PTR: u32 = 0x134;

// ---------------------------------------------------------------------------
// Pointer resolution helpers
// ---------------------------------------------------------------------------

/// Reads a species-specific data pointer from one of the ROM's indirect tables.
/// 
/// The ROM uses a two-level indirection scheme: a fixed header offset holds a
/// pointer to a table, and each table entry (8 bytes wide) holds a pointer to
/// the actual data. This function handles both dereferences.
/// 
/// # Arguments
/// * `rom`                  - Full ROM byte slice.
/// * `table_header`         - Byte offset of the table-pointer slot (e.g. [`PALETTE_TABLE_PTR`])
/// * `species`              - National pokedex number (0-based index into the table).
/// * `label`                - Human-readable name used in error messages (e.g. "palette")
/// 
/// # Returns
/// ROM byte offset (i.e. GBA pointer minus [`ROM_BASE`]) of the data for `species`
/// 
/// # Errors
/// Returns an error if any pointer is missing, out of ROM bounds, or outside
/// the valid cartridge address range `[ROM_BASE, 0x09000000)`
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
    if !(ROM_BASE..0x09000000).contains(&raw) {
        return Err(format!("invalid {label} pointer: {raw:#010X}").into());
    }
    Ok(raw - ROM_BASE)
}

/// Reads a single table pointer from `header_offset` in the ROM and converts it
/// to a ROM byte offset.
/// 
/// Returns `None` (and prints a diagnostic) if the stored value is outside the
/// valid cartridge range `[ROM_BASE, 0x09000000)`.
/// 
/// # Arguments
/// * `rom`                     - Full ROM byte slice.
/// * `header_offset`           - Byte offset of the 4-byte little-endian pointer slot.
fn read_table_pointer(rom: &[u8], header_offset: u32) -> Option<u32> {
    let o = header_offset as usize;
    let raw = u32::from_le_bytes(rom.get(o..o + 4)?.try_into().ok()?);
    if !(ROM_BASE..0x09000000).contains(&raw) {
        eprintln!(
            "invalid table pointer at {:#X}: {:#010X}",
            header_offset, raw
        );
        return None;
    }
    Some(raw - ROM_BASE)
}

/// Reads the ROM byte offset of the compressed front sprite for `species`
/// 
/// Specialized wrapper around [`read_entity_pointer`] for teh front-sprite
/// table. Each entry in the table is 8 bytes wide; the sprite pointer occupies
/// the first 4 bytes.
/// 
/// # Arguments
/// * `rom`                     - Full ROM byte slice
/// * `species`                 - National pokedex number.
/// 
/// # Errors
/// Propogates errors from [`read_table_pointer`] and pointer validation.
fn read_sprite_pointer(rom: &[u8], species: u16) -> Result<u32, Box<dyn std::error::Error>> {
    let table = read_table_pointer(rom, FRONT_SPRITE_TABLE_PTR)
        .ok_or("failed to read sprite table pointer")?;
    let ptr_addr = (table + (species as u32 * 8)) as usize;
    let raw = u32::from_le_bytes(
        rom.get(ptr_addr..ptr_addr + 4)
            .ok_or("sprite pointer address out of ROM bounds")?
            .try_into()?,
    );
    if !(ROM_BASE..0x09000000).contains(&raw) {
        return Err("invalid sprite pointer".into());
    }
    Ok(raw - ROM_BASE)
}

// ---------------------------------------------------------------------------
// Decompression
// ---------------------------------------------------------------------------

/// Decompress a GBA LZ77 (BIOS type 0x10) blob from the ROM.
/// 
/// The GBA BIOS LZ77 format uses an 8-byte header followed by flag bytes that
/// control groups of 8 tokens. Each token is either a literal byte or a 
/// back-reference into already-decoded output.
/// 
/// ## Header layout
/// | Bytes | Meaning
/// |-------|-------------------------------------------|
/// | 0     | `0x10` - compression type identifier      |
/// | 1-3   | Decompressed size (little-endian 24-bit)  |
/// 
/// ## Token encoding
/// For each flag byte, bits are tested from MSB to LSB:
/// - **0** - Literal: copy the next byte to output
/// - **1** - back-reference: read 2 bytes `(b0, b1)`:
///     - `length   = (b0 >> 4) + 3`
///     - `disp     =` ((b0 & 0xF) << 8) | b1`
///     - Copy `length` bytes from `output[len - disp - 1..]`
/// 
/// # Arguments
/// * `rom`                     - Full ROM byte slice.
/// * `offset`                  - Byte offset of the compressed data (must start with `0x10`)
/// 
/// # Errors
/// Returns an error if the data does not start with `0x10`, if the input is
/// truncated, or if a back-reference points before the start of the output.
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

// ---------------------------------------------------------------------------
// Tile decoding
// ---------------------------------------------------------------------------

/// Decodes GBA 4bpp tiled graphics into a flat array of palette indices.
/// 
/// GBA sprites are stored in 8x8-pixel tiles, each tile taking 32 bytes
/// (64 pixels x 4 bits). within each tile byte, the **low nibble** is the
/// left pixel and the **high nibble** is the right pixel. Tiles are laid out
/// left-to-right, top-to-bottom
/// 
/// The output is a row-major flat array of palette indices with dimensions
/// `(width_tiles * 8) x (height_tiles * 8)`. Palette index 0 is treated as
/// transparent by [`decode_palette`].
/// 
/// # Arguments
/// * `data`                - Decompressed 4bpp tile data (as returned by [`decompress_lz77`])
/// * `width_tiles`         - Sprite width in 8-pixel tiles (e.g. `8` for 64 px wide).
/// * `height_tiles`        - Sprite height in 8-pixel tiles (e.g. `8` for 64 px tall).
/// 
/// # Errors
/// Returns an error if `data` is shorter than the `width_tiles * height_tiles * 32`
/// byte requred to fill the sprite.
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
                    // Low nibble = left pixel, high nibble = right pixel
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

// ---------------------------------------------------------------------------
// Palette decoding
// ---------------------------------------------------------------------------

/// Decodes a 16-color GBA BGR555 palette into a `Vec` of `[R, G, B, A]` bytes.
/// 
/// GBA palettes store each color as a little-endian 16-bit value:
/// - Bits 4-0  : red             (0-31)
/// - Bits 9-5  : green           (0-31)
/// - Bits 14-10: blue            (0-31)
/// - Bit 15: unused
/// 
/// Each 5-bit channel is expanded to 8 bits using the formula
/// `value8 = (value5 << 3) | (value5 >> 2)`, which preserves full-scale
/// white (`0x1F` -> `0xFF`) and black (`0x00` -> `0x00`).
/// 
/// Palette index 0 is always the **transparent** color and gets alpha `0`;
/// all other entries get alpha `255`.
/// 
/// # Arguments
/// * `rom`                 - Byte slice containing the palette data (may be the
///   full ROM or a decompressed palette blob; `offset`
///   selects the start).
/// * `offset`              - Byte offset of the first palette entry within `rom`
/// 
/// # Errors 
/// Returns an error if fewer than 32 bytes are available at `offset`
/// (16 colors x 2 bytes each)
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
        // Expand 5-bit channels to 8-bit: replicate the top bits into the freed low bits.
        let r = (r5 << 3) | (r5 >> 2);
        let g = (g5 << 3) | (g5 >> 2);
        let b = (b5 << 3) | (b5 >> 2);
        // Index 0 is the transparency color.
        let a = if i == 0 { 0 } else { 255 };
        palette.push([r, g, b, a]);
    }
    Ok(palette)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns the front sprite of a pokemon as a 64x64 RGBA [`ImageBuffer`].
/// 
/// Orchestrates teh full decode pipeline for one species:
/// 1. Resolves the sprite and palette pointers from the ROM's indirection tables.
/// 2. Decompresses both blobs with LZ77
/// 3. Decodes the 4bpp tile data into a flat palette-index array.
/// 4. Resolves each palette index to an RGBA color and write it into the image.
/// 
/// The `shiny` flag selects between the normal and alternate (shiny) palette
/// table; the sprite geometry is identical for both variants.
/// 
/// # Arguments
/// * `rom`                     - Full ROM byte slice.
/// * `species`                 - National pokedex number (1-386 for FireRed)
/// * `shinty`                  - `true` to use the shiny palette.
/// 
/// # Errors
/// Propagates any error from pointer resolution, decompression, or tile decoding.
/// 
/// # Performance notes
/// [`decode_palette`] is called once per pixel (64x64 = 4096 times). For
/// bulk sprite generation, consider decoding the palette once outside the pixel
/// loop.
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
