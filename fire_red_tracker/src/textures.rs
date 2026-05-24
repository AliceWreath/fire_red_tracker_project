//! # Texture helpers
//!
//! Sprite loading, compression, decompression, and placeholder generation for
//! the tracker GUI. Handles both direct ROM loading (standalone/server mode)
//! and compressed sprite packets received over TCP (client mode).

use crate::game::is_shiny;
use std::io::{Read, Write};

/// Size of party pokemon sprites, in logical pixels.
pub const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// Size of encounter pokemon sprites, in logical pixels.
pub const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// A sprite received from the server, waiting to be uploaded to the GPU.
///
/// Decompression happens on the network thread; the GUI thread only calls
/// [`egui::Context::load_texture`] and stores the handle.
pub struct PendingTexture {
    pub species: u16,
    pub shiny:   bool,
    /// Decompressed RGBA bytes (width × height × 4).
    pub pixels:  Vec<u8>,
    pub width:   u32,
    pub height:  u32,
}

/// Compresses raw RGBA pixel data using zlib (fast preset).
///
/// Used server-side before sending sprites over TCP to reduce bandwidth.
pub fn compress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::{Compression, write::ZlibEncoder};
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// Decompresses zlib-compressed pixel data back to raw RGBA bytes.
///
/// Called client-side after receiving a sprite packet. Returns an empty `Vec`
/// on failure so the texture pipeline can continue without panicking.
pub fn decompress_pixels(data: &[u8]) -> Vec<u8> {
    use flate2::read::ZlibDecoder;
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).unwrap_or(0);
    out
}

/// Extracts a pokemon sprite from the ROM, compresses it, and returns a
/// [`SpriteData`] packet ready to send to a client.
///
/// Returns `None` if the species index is invalid or the sprite cannot be
/// decoded.
pub fn build_sprite_data(
    rom: &[u8],
    species: u16,
    shiny: bool,
) -> Option<fire_red_states::SpriteData> {
    let img    = fire_red_image_data::get_pokemon_sprite(rom, species, shiny).ok()?;
    let width  = img.width();
    let height = img.height();
    let pixels = compress_pixels(&img.into_raw());
    Some(fire_red_states::SpriteData { species, shiny, pixels, width, height })
}

/// Loads a pokemon sprite from the ROM and uploads it as an egui texture.
///
/// The shiny variant is selected automatically via [`is_shiny`].
pub fn load_texture(
    ctx: &egui::Context,
    rom: &[u8],
    species: u16,
    personality: u32,
    ot_id: u32,
) -> Result<egui::TextureHandle, Box<dyn std::error::Error>> {
    let shiny = is_shiny(personality, ot_id);
    let img   = fire_red_image_data::get_pokemon_sprite(rom, species, shiny)?;
    let size  = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Ok(ctx.load_texture(
        format!("pokemon_{}_{}", species, if shiny { "shiny" } else { "normal" }),
        image,
        egui::TextureOptions::NEAREST,
    ))
}

/// Loads the non-shiny sprite for a species and uploads it as an egui texture.
///
/// Used for wild encounter sprites, which are always shown in normal palette.
pub fn load_texture_normal(
    ctx: &egui::Context,
    rom: &[u8],
    species: u16,
) -> Result<egui::TextureHandle, Box<dyn std::error::Error>> {
    let img   = fire_red_image_data::get_pokemon_sprite(rom, species, false)?;
    let size  = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Ok(ctx.load_texture(
        format!("pokemon_{}_normal", species),
        image,
        egui::TextureOptions::NEAREST,
    ))
}

/// Creates a solid-red placeholder texture for species whose sprites could not
/// be loaded. Makes missing sprites visually obvious without panicking.
pub fn make_placeholder(ctx: &egui::Context, species: u16) -> egui::TextureHandle {
    let w      = PARTY_IMAGE_SIZE.0 as usize;
    let h      = PARTY_IMAGE_SIZE.1 as usize;
    let pixels = vec![255u8, 0, 0, 255].repeat(w * h);
    let image  = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
    ctx.load_texture(
        format!("pokemon_{}_placeholder", species),
        image,
        egui::TextureOptions::NEAREST,
    )
}
