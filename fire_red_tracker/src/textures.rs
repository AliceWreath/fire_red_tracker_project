//! Sprite loading and texture helpers for the tracker GUI.

use crate::game::is_shiny;

/// Size of party pokemon sprites, in logical pixels.
pub const PARTY_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// Size of encounter pokemon sprites, in logical pixels.
pub const ENCOUNTER_IMAGE_SIZE: (f32, f32) = (64.0, 64.0);

/// Placeholder type kept for GUI compatibility; always empty in standalone mode.
pub struct PendingTexture {
    pub species: u16,
    pub shiny: bool,
    pub variant: fire_red_states::SpriteVariant,
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
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
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, shiny)?;
    let size = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Ok(ctx.load_texture(
        format!(
            "pokemon_{}_{}",
            species,
            if shiny { "shiny" } else { "normal" }
        ),
        image,
        egui::TextureOptions::NEAREST,
    ))
}

/// Loads a pokemon back sprite from the ROM and uploads it as an egui texture.
///
/// The shiny variant is selected automatically via [`is_shiny`].
pub fn load_texture_back(
    ctx: &egui::Context,
    rom: &[u8],
    species: u16,
    personality: u32,
    ot_id: u32,
) -> Result<egui::TextureHandle, Box<dyn std::error::Error>> {
    let shiny = is_shiny(personality, ot_id);
    let img = fire_red_image_data::get_pokemon_back_sprite(rom, species, shiny)?;
    let size = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &img.into_raw());
    Ok(ctx.load_texture(
        format!(
            "pokemon_{}_{}_back",
            species,
            if shiny { "shiny" } else { "normal" }
        ),
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
    let img = fire_red_image_data::get_pokemon_sprite(rom, species, false)?;
    let size = [img.width() as usize, img.height() as usize];
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
    let w = PARTY_IMAGE_SIZE.0 as usize;
    let h = PARTY_IMAGE_SIZE.1 as usize;
    let pixels = [255u8, 0, 0, 255].repeat(w * h);
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
    ctx.load_texture(
        format!("pokemon_{}_placeholder", species),
        image,
        egui::TextureOptions::NEAREST,
    )
}
