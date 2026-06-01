//! Human-readable location name lookup for FireRed USA Rev 1.
//!
//! The `met_location` byte stored in every `BoxPokemon`'s `MiscSubstruct` is
//! the same value as `MapHeader::name_index` (MAPSEC) from the map the Pokémon
//! was caught on.  The constants below come from
//! `include/constants/map_groups.h` in the pret/firered-leafgreen decomp and
//! are valid for the unmodified FireRed USA Rev 1 ROM.

/// Returns a human-readable name for a `(map_group, map_name)` pair.
///
/// Used for encounter-table display where the raw map ID is stored rather than
/// the MAPSEC value.  Returns `""` for unknown pairs — callers should fall back
/// to a formatted `"G·N"` string in that case.
///
/// # Verified data points (FireRed USA Rev 1, from `READ_CORE_MEMORY 0x2031DBC 2`)
///
/// | Location        | group | map  |
/// |-----------------|-------|------|
/// | Route 1         | 3     | 0x13 |
/// | Route 2         | 3     | 0x14 |
/// | Route 22        | 3     | 0x29 |
/// | Viridian Forest | 1     | 0x00 |
/// | Mt. Moon        | 1     | 0x01 |
///
/// Route 10 is split into North (0x1C) and South (0x1D), which accounts for
/// the +1 offset seen between routes 2 and 22.  All entries marked with `?`
/// are inferred from that pattern and should be verified in-game.
pub fn map_area_name(group: u8, map: u8) -> &'static str {
    match (group, map) {
        // ── Group 1: forest / cave overworld sections ─────────────────────
        (1, 0x00) => "Viridian Forest",   // verified
        (1, 0x01) => "Mt. Moon",          // verified
        (1, 0x02) => "Rock Tunnel",       // ?
        (1, 0x03) => "Safari Zone",       // ?
        (1, 0x04) => "Seafoam Islands",   // ?
        (1, 0x05) => "Victory Road",      // ?
        (1, 0x06) => "Cerulean Cave",     // ?
        (1, 0x07) => "Power Plant",       // ?
        (1, 0x08) => "Pokémon Tower",     // ?

        // ── Group 3: Kanto outdoor routes ─────────────────────────────────
        // Routes 1–9 (verified: Route 1 = 0x13, Route 2 = 0x14)
        (3, 0x13) => "Route 1",           // verified
        (3, 0x14) => "Route 2",           // verified
        (3, 0x15) => "Route 3",           // ?
        (3, 0x16) => "Route 4",           // ?
        (3, 0x17) => "Route 5",           // ?
        (3, 0x18) => "Route 6",           // ?
        (3, 0x19) => "Route 7",           // ?
        (3, 0x1A) => "Route 8",           // ?
        (3, 0x1B) => "Route 9",           // ?
        // Route 10 is split — accounts for the +1 offset at Route 22
        (3, 0x1C) => "Route 10 (N)",      // ?
        (3, 0x1D) => "Route 10 (S)",      // ?
        // Routes 11–25 (inferred; Route 22 = 0x29 verified)
        (3, 0x1E) => "Route 11",          // ?
        (3, 0x1F) => "Route 12",          // ?
        (3, 0x20) => "Route 13",          // ?
        (3, 0x21) => "Route 14",          // ?
        (3, 0x22) => "Route 15",          // ?
        (3, 0x23) => "Route 16",          // ?
        (3, 0x24) => "Route 17",          // ?
        (3, 0x25) => "Route 18",          // ?
        (3, 0x26) => "Route 19",          // ?
        (3, 0x27) => "Route 20",          // ?
        (3, 0x28) => "Route 21",          // ?
        (3, 0x29) => "Route 22",          // verified
        (3, 0x2A) => "Route 23",          // ?
        (3, 0x2B) => "Route 24",          // ?
        (3, 0x2C) => "Route 25",          // ?

        _ => "",
    }
}

/// Returns a human-readable name for a `met_location` byte (FireRed USA Rev 1).
///
/// Values 0x00–0x5C correspond to named locations (MAPSEC constants from the
/// pret decomp).  0xFF is the "no section" sentinel used for interior maps that
/// don't show a location banner; all other out-of-range values return
/// `"Unknown Location"`.
pub fn location_name(loc: u8) -> &'static str {
    match loc {
        0x00 => "Pallet Town",
        0x01 => "Viridian City",
        0x02 => "Pewter City",
        0x03 => "Cerulean City",
        0x04 => "Lavender Town",
        0x05 => "Vermilion City",
        0x06 => "Celadon City",
        0x07 => "Fuchsia City",
        0x08 => "Saffron City",
        0x09 => "Cinnabar Island",
        0x0A => "Indigo Plateau",
        0x0B => "Viridian Forest",
        0x0C => "Mt. Moon",
        0x0D => "S.S. Anne",
        0x0E => "Underground Path",
        0x0F => "Underground Path",
        0x10 => "Diglett's Cave",
        0x11 => "Victory Road",
        0x12 => "Rocket Hideout",
        0x13 => "Silph Co.",
        0x14 => "Pokémon Mansion",
        0x15 => "Safari Zone",
        0x16 => "Pokémon League",
        0x17 => "Rock Tunnel",
        0x18 => "Power Plant",
        0x19 => "Seafoam Islands",
        0x1A => "Pokémon Tower",
        0x1B => "Cerulean Cave",
        0x1C => "Mt. Ember",
        0x1D => "Berry Forest",
        0x1E => "Icefall Cave",
        0x1F => "Lost Cave",
        0x20 => "Pattern Bush",
        0x21 => "Altering Cave",
        0x22 => "Tanoby Ruins",
        0x23 => "Monean Chamber",
        0x24 => "Liptoo Chamber",
        0x25 => "Weepth Chamber",
        0x26 => "Dilford Chamber",
        0x27 => "Scufib Chamber",
        0x28 => "Rixy Chamber",
        0x29 => "Viapois Chamber",
        0x2A => "Three Isle Path",
        0x2B => "Navel Rock",
        0x2C => "Birth Island",
        0x2D => "Route 1",
        0x2E => "Route 2",
        0x2F => "Route 3",
        0x30 => "Route 4",
        0x31 => "Route 5",
        0x32 => "Route 6",
        0x33 => "Route 7",
        0x34 => "Route 8",
        0x35 => "Route 9",
        0x36 => "Route 10",
        0x37 => "Route 11",
        0x38 => "Route 12",
        0x39 => "Route 13",
        0x3A => "Route 14",
        0x3B => "Route 15",
        0x3C => "Route 16",
        0x3D => "Route 17",
        0x3E => "Route 18",
        0x3F => "Route 19",
        0x40 => "Route 20",
        0x41 => "Route 21",
        0x42 => "Route 22",
        0x43 => "Route 23",
        0x44 => "Route 24",
        0x45 => "Route 25",
        0x46 => "One Island",
        0x47 => "Two Island",
        0x48 => "Three Island",
        0x49 => "Four Island",
        0x4A => "Five Island",
        0x4B => "Six Island",
        0x4C => "Seven Island",
        0x4D => "Treasure Beach",
        0x4E => "Kindle Road",
        0x4F => "Cape Brink",
        0x50 => "Bond Bridge",
        0x51 => "Three Isle Port",
        0x52 => "Sevii Isle 6",
        0x53 => "Sevii Isle 7",
        0x54 => "Sevii Isle 8",
        0x55 => "Sevii Isle 9",
        0x56 => "Resort Gorgeous",
        0x57 => "Water Path",
        0x58 => "Ruin Valley",
        0x59 => "Trainer Tower",
        0x5A => "Canyon Entrance",
        0x5B => "Sevault Canyon",
        0x5C => "Tanoby Chambers",
        0xFF => "—",           // MAPSEC_NONE: interior map, no banner shown
        _    => "Unknown Location",
    }
}
