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

/// Byte offset within the ROM where the back-sprite pointer table pointer is stored.
///
/// Verified against the `pokefirered` decompilation for USA Rev 1.  Each entry
/// is 8 bytes wide; the back-sprite pointer occupies the first 4 bytes.
const BACK_SPRITE_TABLE_PTR: u32 = 0x12C;

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
        tracing::warn!(
            "invalid table pointer at {:#X}: {:#010X}",
            header_offset,
            raw
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

/// Reads the ROM byte offset of the compressed back sprite for `species`.
///
/// Mirror of [`read_sprite_pointer`] but uses [`BACK_SPRITE_TABLE_PTR`].
/// Each table entry is 8 bytes; the back-sprite pointer occupies bytes 0–3.
fn read_back_sprite_pointer(rom: &[u8], species: u16) -> Result<u32, Box<dyn std::error::Error>> {
    let table = read_table_pointer(rom, BACK_SPRITE_TABLE_PTR)
        .ok_or("failed to read back sprite table pointer")?;
    let ptr_addr = (table + (species as u32 * 8)) as usize;
    let raw = u32::from_le_bytes(
        rom.get(ptr_addr..ptr_addr + 4)
            .ok_or("back sprite pointer address out of ROM bounds")?
            .try_into()?,
    );
    if !(ROM_BASE..0x09000000).contains(&raw) {
        return Err("invalid back sprite pointer".into());
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

    // next 3 bytes are decompressed size (little endian 24-bit)
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
/// * `shiny`                   - `true` to use the shiny palette.
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

    let palette = decode_palette(&palette_data, 0)?;
    let mut img = ImageBuffer::new(64, 64);
    for (i, palette_index) in pixels.iter().copied().enumerate() {
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

/// Returns the back sprite of a Pokemon as a 64x64 RGBA [`ImageBuffer`].
///
/// Identical pipeline to [`get_pokemon_sprite`] but reads compressed tile data
/// from [`BACK_SPRITE_TABLE_PTR`].  The same normal/shiny palette tables are
/// used — back sprites share the palette with the front sprite.
///
/// # Arguments
/// * `rom`     - Full ROM byte slice.
/// * `species` - National Pokédex number (1–386 for FireRed).
/// * `shiny`   - `true` to use the shiny palette.
///
/// # Errors
/// Propagates any error from pointer resolution, decompression, or tile decoding.
pub fn get_pokemon_back_sprite(
    rom: &[u8],
    species: u16,
    shiny: bool,
) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>, Box<dyn std::error::Error>> {
    let sprite_offset = read_back_sprite_pointer(rom, species)? as usize;

    let palette_offset = if shiny {
        read_entity_pointer(rom, SHINY_PALETTE_TABLE_PTR, species, "shiny palette")? as usize
    } else {
        read_entity_pointer(rom, PALETTE_TABLE_PTR, species, "palette")? as usize
    };

    let sprite_data = decompress_lz77(rom, sprite_offset)?;
    let palette_data = decompress_lz77(rom, palette_offset)?;
    let pixels = decode_4bpp_tiles(&sprite_data, 8, 8)?;

    let palette = decode_palette(&palette_data, 0)?;
    let mut img = ImageBuffer::new(64, 64);
    for (i, palette_index) in pixels.iter().copied().enumerate() {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── decode_4bpp_tiles ─────────────────────────────────────────────────────

    #[test]
    fn tiles_data_too_short_returns_error() {
        // 1x1 tile requires 32 bytes; 31 is not enough.
        assert!(decode_4bpp_tiles(&vec![0u8; 31], 1, 1).is_err());
    }

    #[test]
    fn tiles_all_zeros_produce_all_zero_indices() {
        let pixels = decode_4bpp_tiles(&vec![0u8; 32], 1, 1).unwrap();
        assert_eq!(pixels.len(), 64);
        assert!(pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn tiles_low_nibble_is_left_pixel_high_nibble_is_right() {
        // Byte 0xAB: lo=0xB goes to the left pixel, hi=0xA to the right.
        let mut data = vec![0u8; 32];
        data[0] = 0xAB; // row=0, col=0 → px=(0,1) py=0
        let pixels = decode_4bpp_tiles(&data, 1, 1).unwrap();
        assert_eq!(pixels[0], 0xB);
        assert_eq!(pixels[1], 0xA);
    }

    #[test]
    fn tiles_pixel_row_col_mapping() {
        // row=2, col=3 in a single tile lives at data[2*4+3]=data[11].
        // px = col*2 = 6, py = row = 2 → pixel offsets 22 (lo) and 23 (hi).
        let mut data = vec![0u8; 32];
        data[11] = 0x59; // lo=0x9, hi=0x5
        let pixels = decode_4bpp_tiles(&data, 1, 1).unwrap();
        assert_eq!(pixels[22], 0x9);
        assert_eq!(pixels[23], 0x5);
    }

    #[test]
    fn tiles_second_horizontal_tile_starts_at_x8() {
        // 2×1 tiles → 16×8 canvas. Tile 1 data starts at byte 32.
        // row=0, col=0 of tile 1 → px=8, py=0 → pixel offsets 8 (lo) and 9 (hi).
        let mut data = vec![0u8; 64];
        data[32] = 0x73; // lo=0x3, hi=0x7
        let pixels = decode_4bpp_tiles(&data, 2, 1).unwrap();
        assert_eq!(pixels[8], 0x3);
        assert_eq!(pixels[9], 0x7);
    }

    #[test]
    fn tiles_second_vertical_tile_starts_at_y8() {
        // 1×2 tiles → 8×16 canvas. Tile 1 (tile_y=1) data starts at byte 32.
        // row=0, col=0 → px=0, py=8 → pixel offset 64 (lo) and 65 (hi).
        let mut data = vec![0u8; 64];
        data[32] = 0xC1; // lo=0x1, hi=0xC
        let pixels = decode_4bpp_tiles(&data, 1, 2).unwrap();
        assert_eq!(pixels[64], 0x1);
        assert_eq!(pixels[65], 0xC);
    }

    #[test]
    fn tiles_2x2_all_four_corners_placed_correctly() {
        // 2×2 tiles → 16×16 canvas (width=16).
        // Tile (tx,ty) data starts at [(ty*2 + tx) * 32].
        // row=0, col=0 of each tile maps to the tile's top-left pixel.
        let mut data = vec![0u8; 128];
        data[0] = 0xAB; // tile(0,0): pixel[0]=0xB,  pixel[1]=0xA
        data[32] = 0xCD; // tile(1,0): pixel[8]=0xD,  pixel[9]=0xC
        data[64] = 0xEF; // tile(0,1): pixel[128]=0xF, pixel[129]=0xE  (y=8 → offset=8*16)
        data[96] = 0x12; // tile(1,1): pixel[136]=0x2, pixel[137]=0x1  (y=8,x=8)
        let pixels = decode_4bpp_tiles(&data, 2, 2).unwrap();
        assert_eq!(pixels[0], 0xB);
        assert_eq!(pixels[1], 0xA);
        assert_eq!(pixels[8], 0xD);
        assert_eq!(pixels[9], 0xC);
        assert_eq!(pixels[128], 0xF);
        assert_eq!(pixels[129], 0xE);
        assert_eq!(pixels[136], 0x2);
        assert_eq!(pixels[137], 0x1);
    }

    // ── decode_palette ────────────────────────────────────────────────────────

    #[test]
    fn palette_too_short_returns_error() {
        assert!(decode_palette(&vec![0u8; 31], 0).is_err());
    }

    #[test]
    fn palette_too_short_with_offset_returns_error() {
        // 33 bytes but offset=2 → only 31 bytes available for the palette.
        assert!(decode_palette(&vec![0u8; 33], 2).is_err());
    }

    #[test]
    fn palette_index_0_is_always_transparent() {
        // White (0x7FFF) at index 0 must still get alpha=0.
        let mut data = vec![0u8; 32];
        data[0] = 0xFF;
        data[1] = 0x7F; // little-endian 0x7FFF
        let pal = decode_palette(&data, 0).unwrap();
        assert_eq!(pal[0][3], 0);
    }

    #[test]
    fn palette_non_zero_indices_get_full_alpha() {
        let pal = decode_palette(&vec![0u8; 32], 0).unwrap();
        for entry in &pal[1..] {
            assert_eq!(entry[3], 255);
        }
    }

    #[test]
    fn palette_black_entry_decodes_to_zeros() {
        // 0x0000 → R=0, G=0, B=0. Index 0 also gets A=0 (transparent).
        let pal = decode_palette(&vec![0u8; 32], 0).unwrap();
        assert_eq!(pal[0], [0, 0, 0, 0]);
    }

    #[test]
    fn palette_white_decodes_to_full_rgb() {
        // 0x7FFF: R=G=B=0x1F. Expansion: (0x1F << 3) | (0x1F >> 2) = 0xF8 | 0x07 = 0xFF.
        let mut data = vec![0u8; 32];
        data[2] = 0xFF; // index 1, little-endian 0x7FFF
        data[3] = 0x7F;
        let pal = decode_palette(&data, 0).unwrap();
        assert_eq!(pal[1], [0xFF, 0xFF, 0xFF, 255]);
    }

    #[test]
    fn palette_pure_red_channel() {
        // BGR555 bits 4-0 = R. 0x001F → R=0x1F, G=0, B=0.
        let mut data = vec![0u8; 32];
        data[2] = 0x1F; // index 1: raw=0x001F
        data[3] = 0x00;
        let pal = decode_palette(&data, 0).unwrap();
        assert_eq!(pal[1], [0xFF, 0, 0, 255]);
    }

    #[test]
    fn palette_pure_green_channel() {
        // BGR555 bits 9-5 = G. 0x03E0 → G=0x1F, R=0, B=0.
        let mut data = vec![0u8; 32];
        data[2] = 0xE0; // index 1: raw=0x03E0 little-endian
        data[3] = 0x03;
        let pal = decode_palette(&data, 0).unwrap();
        assert_eq!(pal[1], [0, 0xFF, 0, 255]);
    }

    #[test]
    fn palette_pure_blue_channel() {
        // BGR555 bits 14-10 = B. 0x7C00 → B=0x1F, R=0, G=0.
        let mut data = vec![0u8; 32];
        data[2] = 0x00; // index 1: raw=0x7C00 little-endian
        data[3] = 0x7C;
        let pal = decode_palette(&data, 0).unwrap();
        assert_eq!(pal[1], [0, 0, 0xFF, 255]);
    }

    #[test]
    fn palette_5bit_mid_value_expansion() {
        // 0x10 (16): (16 << 3) | (16 >> 2) = 128 | 4 = 132. Use R channel at index 1.
        let mut data = vec![0u8; 32];
        data[2] = 0x10; // raw=0x0010 → R=0x10
        data[3] = 0x00;
        let pal = decode_palette(&data, 0).unwrap();
        assert_eq!(pal[1][0], 132);
        assert_eq!(pal[1][1], 0);
        assert_eq!(pal[1][2], 0);
    }

    #[test]
    fn palette_offset_skips_leading_bytes() {
        // Palette at offset 32; index 1 bytes at 32 + 2 = 34.
        let mut data = vec![0u8; 64];
        data[34] = 0x1F; // pure red at index 1
        data[35] = 0x00;
        let pal = decode_palette(&data, 32).unwrap();
        assert_eq!(pal[1], [0xFF, 0, 0, 255]);
    }

    // ── decompress_lz77 ───────────────────────────────────────────────────────

    #[test]
    fn lz77_wrong_type_byte_returns_error() {
        let data = vec![0x00, 0x03, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC];
        assert!(decompress_lz77(&data, 0).is_err());
    }

    #[test]
    fn lz77_all_literals() {
        // flags=0x00 → all 8 tokens are literals; read 3 of them then stop.
        let data = vec![0x10, 0x03, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC];
        assert_eq!(decompress_lz77(&data, 0).unwrap(), vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn lz77_back_reference_repeats_prior_byte() {
        // flags=0x40: bit7=0 (literal 0xAA), bit6=1 (back-ref).
        // back-ref b0=0x00, b1=0x00 → length=3, disp=0 → copies output[0] three times.
        let data = vec![0x10, 0x04, 0x00, 0x00, 0x40, 0xAA, 0x00, 0x00];
        assert_eq!(
            decompress_lz77(&data, 0).unwrap(),
            vec![0xAA, 0xAA, 0xAA, 0xAA]
        );
    }

    #[test]
    fn lz77_back_reference_with_nonzero_displacement() {
        // flags=0x10: bits 7-5=0 (literals A,B,C), bit4=1 (back-ref), bits 3-0=0.
        // back-ref b0=0x00, b1=0x02 → length=3, disp=2 → start=3-2-1=0 → copies "ABC".
        let data = vec![0x10, 0x06, 0x00, 0x00, 0x10, b'A', b'B', b'C', 0x00, 0x02];
        assert_eq!(
            decompress_lz77(&data, 0).unwrap(),
            vec![b'A', b'B', b'C', b'A', b'B', b'C'],
        );
    }
}
