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
/// All values sourced from the FireRed/LeafGreen map groups document and
/// cross-checked against `READ_CORE_MEMORY 0x2031DBC 2` for in-game entries
/// marked "verified".  Note: Route 21 is split North/South (not Route 10),
/// which is why Route 22 lands at 0x29.
///
/// | Location             | group | map  | notes    |
/// |----------------------|-------|------|----------|
/// | Viridian Forest      | 1     | 0x00 | verified |
/// | Mt. Moon 1F          | 1     | 0x01 | verified |
/// | S.S. Anne (Exterior) | 1     | 0x04 | verified |
/// | S.S. Anne 1F         | 1     | 0x05 | verified |
/// | Diglett's Cave       | 1     | 0x25 | verified |
/// | Route 1              | 3     | 0x13 | verified |
/// | Route 2              | 3     | 0x14 | verified |
/// | Route 22             | 3     | 0x29 | verified |
/// | Route 21 (N)         | 3     | 0x27 | from doc |
/// | Route 21 (S)         | 3     | 0x28 | from doc |
pub fn map_area_name(group: u8, map: u8) -> &'static str {
    match (group, map) {
        // ── Group 1: caves and multi-floor dungeons ───────────────────────
        (1, 0x00) => "Viridian Forest",
        (1, 0x01) => "Mt. Moon 1F",
        (1, 0x02) => "Mt. Moon B1F",
        (1, 0x03) => "Mt. Moon B2F",
        (1, 0x04) => "S.S. Anne (Exterior)", // no wild encounters
        (1, 0x05) => "S.S. Anne 1F",         // no wild encounters
        (1, 0x1F) => "Underground Path (N-S)", // no wild encounters
        (1, 0x22) => "Underground Path (E-W)", // no wild encounters
        (1, 0x24) => "Diglett's Cave (N Entrance)",
        (1, 0x25) => "Diglett's Cave",
        (1, 0x26) => "Diglett's Cave (S Entrance)",
        (1, 0x27) => "Victory Road 1F",
        (1, 0x28) => "Victory Road 2F",
        (1, 0x29) => "Victory Road 3F",
        (1, 0x3B) => "Pokemon Mansion 1F",
        (1, 0x3C) => "Pokemon Mansion 2F",
        (1, 0x3D) => "Pokemon Mansion 3F",
        (1, 0x3E) => "Pokemon Mansion B1F",
        (1, 0x3F) => "Safari Zone (Center)",
        (1, 0x40) => "Safari Zone (East)",
        (1, 0x41) => "Safari Zone (North)",
        (1, 0x42) => "Safari Zone (West)",
        (1, 0x48) => "Cerulean Cave 1F",
        (1, 0x49) => "Cerulean Cave 2F",
        (1, 0x4A) => "Cerulean Cave B1F",
        (1, 0x51) => "Rock Tunnel 1F",
        (1, 0x52) => "Rock Tunnel B1F",
        (1, 0x53) => "Seafoam Islands 1F",
        (1, 0x54) => "Seafoam Islands B1F",
        (1, 0x55) => "Seafoam Islands B2F",
        (1, 0x56) => "Seafoam Islands B3F",
        (1, 0x57) => "Seafoam Islands B4F",
        (1, 0x58) => "Pokemon Tower 1F",
        (1, 0x59) => "Pokemon Tower 2F",
        (1, 0x5A) => "Pokemon Tower 3F",
        (1, 0x5B) => "Pokemon Tower 4F",
        (1, 0x5C) => "Pokemon Tower 5F",
        (1, 0x5D) => "Pokemon Tower 6F",
        (1, 0x5E) => "Pokemon Tower 7F",
        (1, 0x5F) => "Power Plant",
        // Sevii – Mt. Ember
        (1, 0x60) => "Mt. Ember Ruby Path B4F",
        (1, 0x61) => "Mt. Ember (Exterior)",
        (1, 0x62) => "Mt. Ember Summit Path 1F",
        (1, 0x63) => "Mt. Ember Summit Path 2F",
        (1, 0x64) => "Mt. Ember Summit Path 3F",
        (1, 0x65) => "Mt. Ember Summit",
        (1, 0x66) => "Mt. Ember Ruby Path B5F",
        (1, 0x67) => "Mt. Ember Ruby Path 1F",
        (1, 0x68) => "Mt. Ember Ruby Path B1F",
        (1, 0x69) => "Mt. Ember Ruby Path B2F",
        (1, 0x6A) => "Mt. Ember Ruby Path B3F",
        // Sevii – Berry Forest / Icefall Cave
        (1, 0x6D) => "Berry Forest",
        (1, 0x6E) => "Icefall Cave (Entrance)",
        (1, 0x6F) => "Icefall Cave 1F",
        (1, 0x70) => "Icefall Cave B1F",
        (1, 0x71) => "Icefall Cave (Back)",
        // Sevii – Dotted Hole / Pattern Bush / Altering Cave
        (1, 0x73) => "Dotted Hole 1F",
        (1, 0x74) => "Dotted Hole B1F",
        (1, 0x75) => "Dotted Hole B2F",
        (1, 0x76) => "Dotted Hole B3F",
        (1, 0x77) => "Dotted Hole B4F",
        (1, 0x79) => "Pattern Bush",
        (1, 0x7A) => "Altering Cave",

        // ── Group 2: Sevii special areas ──────────────────────────────────
        (2, 0x0C) => "Lost Cave (Entrance)",
        (2, 0x0D) => "Lost Cave Room 1",
        (2, 0x0E) => "Lost Cave Room 2",
        (2, 0x0F) => "Lost Cave Room 3",
        (2, 0x10) => "Lost Cave Room 4",
        (2, 0x11) => "Lost Cave Room 5",
        (2, 0x12) => "Lost Cave Room 6",
        (2, 0x13) => "Lost Cave Room 7",
        (2, 0x14) => "Lost Cave Room 8",
        (2, 0x15) => "Lost Cave Room 9",
        (2, 0x16) => "Lost Cave Room 10",
        (2, 0x17) => "Lost Cave Room 11",
        (2, 0x18) => "Lost Cave Room 12",
        (2, 0x19) => "Lost Cave Room 13",
        (2, 0x1A) => "Lost Cave Room 14",
        (2, 0x1B) => "Monean Chamber",
        (2, 0x1C) => "Liptoo Chamber",
        (2, 0x1D) => "Weepth Chamber",
        (2, 0x1E) => "Dilford Chamber",
        (2, 0x1F) => "Scufib Chamber",
        (2, 0x20) => "Rixy Chamber",
        (2, 0x21) => "Viapois Chamber",
        (2, 0x22) => "Dunsparce Tunnel",

        // ── Group 3: towns and cities ──────────────────────────────────────
        // Vanilla FireRed has no wild encounters in towns, but ROM hacks
        // frequently add grass there, and catches/events can be recorded
        // while standing in one — so these need names, not "3·N" fallbacks.
        (3, 0x00) => "Pallet Town",
        (3, 0x01) => "Viridian City",
        (3, 0x02) => "Pewter City",
        (3, 0x03) => "Cerulean City",
        (3, 0x04) => "Lavender Town",
        (3, 0x05) => "Vermilion City",
        (3, 0x06) => "Celadon City",
        (3, 0x07) => "Fuchsia City",
        (3, 0x08) => "Cinnabar Island",
        (3, 0x09) => "Indigo Plateau",
        (3, 0x0A) => "Saffron City",
        (3, 0x0B) => "Saffron City", // connection strip west of Silph Co.
        (3, 0x0C) => "One Island",
        (3, 0x0D) => "Two Island",
        (3, 0x0E) => "Three Island",
        (3, 0x0F) => "Four Island",
        (3, 0x10) => "Five Island",
        (3, 0x11) => "Seven Island", // FireRed orders Seven before Six
        (3, 0x12) => "Six Island",

        // ── Group 3: Kanto outdoor routes ─────────────────────────────────
        (3, 0x13) => "Route 1",
        (3, 0x14) => "Route 2",
        (3, 0x15) => "Route 3",
        (3, 0x16) => "Route 4",
        (3, 0x17) => "Route 5",
        (3, 0x18) => "Route 6",
        (3, 0x19) => "Route 7",
        (3, 0x1A) => "Route 8",
        (3, 0x1B) => "Route 9",
        (3, 0x1C) => "Route 10",
        (3, 0x1D) => "Route 11",
        (3, 0x1E) => "Route 12",
        (3, 0x1F) => "Route 13",
        (3, 0x20) => "Route 14",
        (3, 0x21) => "Route 15",
        (3, 0x22) => "Route 16",
        (3, 0x23) => "Route 17",
        (3, 0x24) => "Route 18",
        (3, 0x25) => "Route 19",
        (3, 0x26) => "Route 20",
        (3, 0x27) => "Route 21 (N)",
        (3, 0x28) => "Route 21 (S)",
        (3, 0x29) => "Route 22",
        (3, 0x2A) => "Route 23",
        (3, 0x2B) => "Route 24",
        (3, 0x2C) => "Route 25",
        // Sevii outdoor wild areas
        (3, 0x2D) => "Kindle Road",
        (3, 0x2E) => "Treasure Beach",
        (3, 0x2F) => "Cape Brink",
        (3, 0x30) => "Bond Bridge",
        (3, 0x37) => "Water Labyrinth",
        (3, 0x38) => "Five Island Meadow",
        (3, 0x3B) => "Green Path",
        (3, 0x3C) => "Water Path",
        (3, 0x3D) => "Ruin Valley",
        (3, 0x40) => "Sevault Canyon",
        (3, 0x41) => "Tanoby Ruins",

        _ => "",
    }
}

/// Returns all known (map_group, map_name, area_name) tuples for FireRed.
///
/// Only includes areas that have wild encounters in at least one encounter type
/// (land, water, rock smash, or fishing).  Locations with no wild encounters
/// (S.S. Anne interior, Underground Paths, etc.) are omitted.
pub fn all_wild_areas() -> &'static [(u8, u8, &'static str)] {
    &[
        // Group 1: caves / multi-floor dungeons
        (1, 0x00, "Viridian Forest"),
        (1, 0x01, "Mt. Moon 1F"),
        (1, 0x02, "Mt. Moon B1F"),
        (1, 0x03, "Mt. Moon B2F"),
        (1, 0x25, "Diglett's Cave"),
        (1, 0x27, "Victory Road 1F"),
        (1, 0x28, "Victory Road 2F"),
        (1, 0x29, "Victory Road 3F"),
        (1, 0x3B, "Pokemon Mansion 1F"),
        (1, 0x3C, "Pokemon Mansion 2F"),
        (1, 0x3D, "Pokemon Mansion 3F"),
        (1, 0x3E, "Pokemon Mansion B1F"),
        (1, 0x3F, "Safari Zone (Center)"),
        (1, 0x40, "Safari Zone (East)"),
        (1, 0x41, "Safari Zone (North)"),
        (1, 0x42, "Safari Zone (West)"),
        (1, 0x48, "Cerulean Cave 1F"),
        (1, 0x49, "Cerulean Cave 2F"),
        (1, 0x4A, "Cerulean Cave B1F"),
        (1, 0x51, "Rock Tunnel 1F"),
        (1, 0x52, "Rock Tunnel B1F"),
        (1, 0x53, "Seafoam Islands 1F"),
        (1, 0x54, "Seafoam Islands B1F"),
        (1, 0x55, "Seafoam Islands B2F"),
        (1, 0x56, "Seafoam Islands B3F"),
        (1, 0x57, "Seafoam Islands B4F"),
        (1, 0x58, "Pokemon Tower 1F"),
        (1, 0x59, "Pokemon Tower 2F"),
        (1, 0x5A, "Pokemon Tower 3F"),
        (1, 0x5B, "Pokemon Tower 4F"),
        (1, 0x5C, "Pokemon Tower 5F"),
        (1, 0x5D, "Pokemon Tower 6F"),
        (1, 0x5E, "Pokemon Tower 7F"),
        (1, 0x5F, "Power Plant"),
        // Sevii Islands – Mt. Ember
        (1, 0x61, "Mt. Ember (Exterior)"),
        (1, 0x62, "Mt. Ember Summit Path 1F"),
        (1, 0x63, "Mt. Ember Summit Path 2F"),
        (1, 0x64, "Mt. Ember Summit Path 3F"),
        (1, 0x65, "Mt. Ember Summit"),
        (1, 0x67, "Mt. Ember Ruby Path 1F"),
        (1, 0x68, "Mt. Ember Ruby Path B1F"),
        (1, 0x69, "Mt. Ember Ruby Path B2F"),
        (1, 0x6A, "Mt. Ember Ruby Path B3F"),
        (1, 0x60, "Mt. Ember Ruby Path B4F"),
        (1, 0x66, "Mt. Ember Ruby Path B5F"),
        // Sevii Islands – other
        (1, 0x6D, "Berry Forest"),
        (1, 0x6E, "Icefall Cave (Entrance)"),
        (1, 0x6F, "Icefall Cave 1F"),
        (1, 0x70, "Icefall Cave B1F"),
        (1, 0x71, "Icefall Cave (Back)"),
        (1, 0x73, "Dotted Hole 1F"),
        (1, 0x74, "Dotted Hole B1F"),
        (1, 0x75, "Dotted Hole B2F"),
        (1, 0x76, "Dotted Hole B3F"),
        (1, 0x77, "Dotted Hole B4F"),
        (1, 0x79, "Pattern Bush"),
        (1, 0x7A, "Altering Cave"),
        // Group 2: Sevii special areas
        (2, 0x1B, "Monean Chamber"),
        (2, 0x1C, "Liptoo Chamber"),
        (2, 0x1D, "Weepth Chamber"),
        (2, 0x1E, "Dilford Chamber"),
        (2, 0x1F, "Scufib Chamber"),
        (2, 0x20, "Rixy Chamber"),
        (2, 0x21, "Viapois Chamber"),
        (2, 0x22, "Dunsparce Tunnel"),
        // Group 3: Kanto outdoor routes
        (3, 0x13, "Route 1"),
        (3, 0x14, "Route 2"),
        (3, 0x15, "Route 3"),
        (3, 0x16, "Route 4"),
        (3, 0x17, "Route 5"),
        (3, 0x18, "Route 6"),
        (3, 0x19, "Route 7"),
        (3, 0x1A, "Route 8"),
        (3, 0x1B, "Route 9"),
        (3, 0x1C, "Route 10"),
        (3, 0x1D, "Route 11"),
        (3, 0x1E, "Route 12"),
        (3, 0x1F, "Route 13"),
        (3, 0x20, "Route 14"),
        (3, 0x21, "Route 15"),
        (3, 0x22, "Route 16"),
        (3, 0x23, "Route 17"),
        (3, 0x24, "Route 18"),
        (3, 0x25, "Route 19"),
        (3, 0x26, "Route 20"),
        (3, 0x27, "Route 21 (N)"),
        (3, 0x28, "Route 21 (S)"),
        (3, 0x29, "Route 22"),
        (3, 0x2A, "Route 23"),
        (3, 0x2B, "Route 24"),
        (3, 0x2C, "Route 25"),
        // Sevii outdoor wild areas
        (3, 0x2D, "Kindle Road"),
        (3, 0x2E, "Treasure Beach"),
        (3, 0x2F, "Cape Brink"),
        (3, 0x30, "Bond Bridge"),
        (3, 0x37, "Water Labyrinth"),
        (3, 0x38, "Five Island Meadow"),
        (3, 0x3B, "Green Path"),
        (3, 0x3C, "Water Path"),
        (3, 0x3D, "Ruin Valley"),
        (3, 0x40, "Sevault Canyon"),
        (3, 0x41, "Tanoby Ruins"),
    ]
}

/// Returns a human-readable name for a raw Gen III map-section (MAPSEC) byte.
///
/// This is the value FireRed stores both in each map header's
/// `regionMapSectionId` field and in a Pokémon's `met_location` byte. The
/// Gen III numbering is shared across RSE and FRLG: 0x00–0x57 are Hoenn
/// sections (a Pokémon traded from Ruby/Sapphire/Emerald), and the Kanto
/// sections start at 0x58 = Pallet Town. Verified against the map headers of
/// a vanilla FireRed USA Rev 1 ROM.
///
/// 0xFF is the "no section" sentinel used for interior maps that don't show
/// a location banner; unassigned values return `"Unknown Location"`.
pub fn location_name(loc: u8) -> &'static str {
    match loc {
        // Hoenn sections (RSE) — seen on Pokémon traded from those games.
        0x00..=0x57 => "Hoenn",
        0x58 => "Pallet Town",
        0x59 => "Viridian City",
        0x5A => "Pewter City",
        0x5B => "Cerulean City",
        0x5C => "Lavender Town",
        0x5D => "Vermilion City",
        0x5E => "Celadon City",
        0x5F => "Fuchsia City",
        0x60 => "Cinnabar Island",
        0x61 => "Indigo Plateau",
        0x62 => "Saffron City",
        0x63 => "Route 4",  // Route 4 Pokémon Center fly-dup section
        0x64 => "Route 10", // Route 10 Pokémon Center fly-dup section
        0x65 => "Route 1",
        0x66 => "Route 2",
        0x67 => "Route 3",
        0x68 => "Route 4",
        0x69 => "Route 5",
        0x6A => "Route 6",
        0x6B => "Route 7",
        0x6C => "Route 8",
        0x6D => "Route 9",
        0x6E => "Route 10",
        0x6F => "Route 11",
        0x70 => "Route 12",
        0x71 => "Route 13",
        0x72 => "Route 14",
        0x73 => "Route 15",
        0x74 => "Route 16",
        0x75 => "Route 17",
        0x76 => "Route 18",
        0x77 => "Route 19",
        0x78 => "Route 20",
        0x79 => "Route 21",
        0x7A => "Route 22",
        0x7B => "Route 23",
        0x7C => "Route 24",
        0x7D => "Route 25",
        0x7E => "Viridian Forest",
        0x7F => "Mt. Moon",
        0x80 => "S.S. Anne",
        0x81 => "Underground Path",
        0x82 => "Underground Path",
        0x83 => "Diglett's Cave",
        0x84 => "Victory Road",
        0x85 => "Rocket Hideout",
        0x86 => "Silph Co.",
        0x87 => "Pokémon Mansion",
        0x88 => "Safari Zone",
        0x89 => "Pokémon League",
        0x8A => "Rock Tunnel",
        0x8B => "Seafoam Islands",
        0x8C => "Pokémon Tower",
        0x8D => "Cerulean Cave",
        0x8E => "Power Plant",
        0x8F => "One Island",
        0x90 => "Two Island",
        0x91 => "Three Island",
        0x92 => "Four Island",
        0x93 => "Five Island",
        0x94 => "Seven Island", // FireRed orders Seven before Six
        0x95 => "Six Island",
        0x96 => "Kindle Road",
        0x97 => "Treasure Beach",
        0x98 => "Cape Brink",
        0x99 => "Bond Bridge",
        0x9A => "Three Isle Port",
        0x9B => "Sevii Isle 6",
        0x9C => "Sevii Isle 7",
        0x9D => "Sevii Isle 8",
        0x9E => "Sevii Isle 9",
        0x9F => "Resort Gorgeous",
        0xA0 => "Water Labyrinth",
        0xA1 => "Five Isle Meadow",
        0xA2 => "Memorial Pillar",
        0xA3 => "Outcast Island",
        0xA4 => "Green Path",
        0xA5 => "Water Path",
        0xA6 => "Ruin Valley",
        0xA7 => "Trainer Tower",
        0xA8 => "Canyon Entrance",
        0xA9 => "Sevault Canyon",
        0xAA => "Tanoby Ruins",
        0xAB => "Sevii Isle 22",
        0xAC => "Sevii Isle 23",
        0xAD => "Sevii Isle 24",
        0xAE => "Navel Rock",
        0xAF => "Mt. Ember",
        0xB0 => "Berry Forest",
        0xB1 => "Icefall Cave",
        0xB2 => "Rocket Warehouse",
        0xB3 => "Trainer Tower",
        0xB4 => "Dotted Hole",
        0xB5 => "Lost Cave",
        0xB6 => "Pattern Bush",
        0xB7 => "Altering Cave",
        0xB8 => "Tanoby Chambers",
        0xB9 => "Three Isle Path",
        0xBA => "Tanoby Key",
        0xBB => "Birth Island",
        0xBC => "Monean Chamber",
        0xBD => "Liptoo Chamber",
        0xBE => "Weepth Chamber",
        0xBF => "Dilford Chamber",
        0xC0 => "Scufib Chamber",
        0xC1 => "Rixy Chamber",
        0xC2 => "Viapois Chamber",
        0xC3 => "Ember Spa",
        0xC4 => "Special Area",
        0xFF => "—", // MAPSEC_NONE: interior map, no banner shown
        _ => "Unknown Location",
    }
}

/// Returns every `(map_group, map_name)` pair that belongs to the same
/// multi-floor dungeon as the given pair.
///
/// The returned slice always includes the given pair itself when it is part
/// of a dungeon.  Returns an empty slice for single-floor or outdoor areas —
/// callers should treat an empty result as "no dungeon grouping applies."
///
/// Use this with `fire_red_database::has_encounter_for_any_floor` to check
/// whether any floor of the dungeon has already been claimed this run before
/// recording a new encounter.
pub fn dungeon_floors(group: u8, map: u8) -> &'static [(u8, u8)] {
    match (group, map) {
        // Mt. Moon
        (1, 0x01) | (1, 0x02) | (1, 0x03) => &[(1, 0x01), (1, 0x02), (1, 0x03)],
        // Diglett's Cave (entrances + main tunnel)
        (1, 0x24) | (1, 0x25) | (1, 0x26) => &[(1, 0x24), (1, 0x25), (1, 0x26)],
        // Victory Road
        (1, 0x27) | (1, 0x28) | (1, 0x29) => &[(1, 0x27), (1, 0x28), (1, 0x29)],
        // Pokémon Mansion
        (1, 0x3B) | (1, 0x3C) | (1, 0x3D) | (1, 0x3E) => {
            &[(1, 0x3B), (1, 0x3C), (1, 0x3D), (1, 0x3E)]
        }
        // Safari Zone
        (1, 0x3F) | (1, 0x40) | (1, 0x41) | (1, 0x42) => {
            &[(1, 0x3F), (1, 0x40), (1, 0x41), (1, 0x42)]
        }
        // Cerulean Cave
        (1, 0x48) | (1, 0x49) | (1, 0x4A) => &[(1, 0x48), (1, 0x49), (1, 0x4A)],
        // Rock Tunnel
        (1, 0x51) | (1, 0x52) => &[(1, 0x51), (1, 0x52)],
        // Seafoam Islands
        (1, 0x53) | (1, 0x54) | (1, 0x55) | (1, 0x56) | (1, 0x57) => {
            &[(1, 0x53), (1, 0x54), (1, 0x55), (1, 0x56), (1, 0x57)]
        }
        // Pokémon Tower
        (1, 0x58) | (1, 0x59) | (1, 0x5A) | (1, 0x5B) | (1, 0x5C) | (1, 0x5D) | (1, 0x5E) => &[
            (1, 0x58),
            (1, 0x59),
            (1, 0x5A),
            (1, 0x5B),
            (1, 0x5C),
            (1, 0x5D),
            (1, 0x5E),
        ],
        // Mt. Ember (exterior + summit path + ruby path)
        (1, 0x60)
        | (1, 0x61)
        | (1, 0x62)
        | (1, 0x63)
        | (1, 0x64)
        | (1, 0x65)
        | (1, 0x66)
        | (1, 0x67)
        | (1, 0x68)
        | (1, 0x69)
        | (1, 0x6A) => &[
            (1, 0x60),
            (1, 0x61),
            (1, 0x62),
            (1, 0x63),
            (1, 0x64),
            (1, 0x65),
            (1, 0x66),
            (1, 0x67),
            (1, 0x68),
            (1, 0x69),
            (1, 0x6A),
        ],
        // Icefall Cave
        (1, 0x6E) | (1, 0x6F) | (1, 0x70) | (1, 0x71) => {
            &[(1, 0x6E), (1, 0x6F), (1, 0x70), (1, 0x71)]
        }
        // Dotted Hole
        (1, 0x73) | (1, 0x74) | (1, 0x75) | (1, 0x76) | (1, 0x77) => {
            &[(1, 0x73), (1, 0x74), (1, 0x75), (1, 0x76), (1, 0x77)]
        }
        // Lost Cave
        (2, 0x0C)
        | (2, 0x0D)
        | (2, 0x0E)
        | (2, 0x0F)
        | (2, 0x10)
        | (2, 0x11)
        | (2, 0x12)
        | (2, 0x13)
        | (2, 0x14)
        | (2, 0x15)
        | (2, 0x16)
        | (2, 0x17)
        | (2, 0x18)
        | (2, 0x19)
        | (2, 0x1A) => &[
            (2, 0x0C),
            (2, 0x0D),
            (2, 0x0E),
            (2, 0x0F),
            (2, 0x10),
            (2, 0x11),
            (2, 0x12),
            (2, 0x13),
            (2, 0x14),
            (2, 0x15),
            (2, 0x16),
            (2, 0x17),
            (2, 0x18),
            (2, 0x19),
            (2, 0x1A),
        ],
        // Tanoby Chambers (all seven share one encounter slot)
        (2, 0x1B) | (2, 0x1C) | (2, 0x1D) | (2, 0x1E) | (2, 0x1F) | (2, 0x20) | (2, 0x21) => &[
            (2, 0x1B),
            (2, 0x1C),
            (2, 0x1D),
            (2, 0x1E),
            (2, 0x1F),
            (2, 0x20),
            (2, 0x21),
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── map_area_name: in-game verified ───────────────────────────────────────

    #[test]
    fn viridian_forest_verified() {
        assert_eq!(map_area_name(1, 0x00), "Viridian Forest");
    }
    #[test]
    fn mt_moon_verified() {
        assert_eq!(map_area_name(1, 0x01), "Mt. Moon 1F");
    }
    #[test]
    fn ss_anne_ext_verified() {
        assert_eq!(map_area_name(1, 0x04), "S.S. Anne (Exterior)");
    }
    #[test]
    fn ss_anne_1f_verified() {
        assert_eq!(map_area_name(1, 0x05), "S.S. Anne 1F");
    }
    #[test]
    fn digletts_cave_verified() {
        assert_eq!(map_area_name(1, 0x25), "Diglett's Cave");
    }
    #[test]
    fn route_1_verified() {
        assert_eq!(map_area_name(3, 0x13), "Route 1");
    }
    #[test]
    fn route_2_verified() {
        assert_eq!(map_area_name(3, 0x14), "Route 2");
    }
    #[test]
    fn route_22_verified() {
        assert_eq!(map_area_name(3, 0x29), "Route 22");
    }

    // ── map_area_name: route 10/21 split correction ───────────────────────────

    #[test]
    fn route_10_is_single() {
        assert_eq!(map_area_name(3, 0x1C), "Route 10");
    }
    #[test]
    fn route_11_correct() {
        assert_eq!(map_area_name(3, 0x1D), "Route 11");
    }
    #[test]
    fn route_21_north() {
        assert_eq!(map_area_name(3, 0x27), "Route 21 (N)");
    }
    #[test]
    fn route_21_south() {
        assert_eq!(map_area_name(3, 0x28), "Route 21 (S)");
    }

    // ── map_area_name: key wild areas from doc ────────────────────────────────

    #[test]
    fn rock_tunnel_1f() {
        assert_eq!(map_area_name(1, 0x51), "Rock Tunnel 1F");
    }
    #[test]
    fn cerulean_cave_b1f() {
        assert_eq!(map_area_name(1, 0x4A), "Cerulean Cave B1F");
    }
    #[test]
    fn power_plant() {
        assert_eq!(map_area_name(1, 0x5F), "Power Plant");
    }
    #[test]
    fn safari_zone_center() {
        assert_eq!(map_area_name(1, 0x3F), "Safari Zone (Center)");
    }
    #[test]
    fn berry_forest() {
        assert_eq!(map_area_name(1, 0x6D), "Berry Forest");
    }
    #[test]
    fn dunsparce_tunnel() {
        assert_eq!(map_area_name(2, 0x22), "Dunsparce Tunnel");
    }
    #[test]
    fn unknown_pair_returns_empty() {
        assert_eq!(map_area_name(0xFF, 0xFF), "");
    }

    // ── location_name (MAPSEC) ────────────────────────────────────────────────

    #[test]
    fn pallet_town() {
        // Kanto MAPSECs start at 0x58 in the shared Gen III numbering —
        // this is the raw byte in both map headers and met_location.
        assert_eq!(location_name(0x58), "Pallet Town");
    }
    #[test]
    fn viridian_city() {
        assert_eq!(location_name(0x59), "Viridian City");
    }
    #[test]
    fn routes_follow_the_flydup_sections() {
        assert_eq!(location_name(0x63), "Route 4"); // Pokémon Center fly-dup
        assert_eq!(location_name(0x65), "Route 1");
        assert_eq!(location_name(0x7D), "Route 25");
    }
    #[test]
    fn dungeons_and_sevii() {
        assert_eq!(location_name(0x7E), "Viridian Forest");
        assert_eq!(location_name(0x8D), "Cerulean Cave");
        assert_eq!(location_name(0x94), "Seven Island"); // Seven before Six
        assert_eq!(location_name(0x95), "Six Island");
        assert_eq!(location_name(0xB9), "Three Isle Path");
        assert_eq!(location_name(0xC2), "Viapois Chamber");
    }
    #[test]
    fn hoenn_sections_from_traded_mons() {
        assert_eq!(location_name(0x00), "Hoenn");
        assert_eq!(location_name(0x57), "Hoenn");
    }
    #[test]
    fn mapsec_none_returns_dash() {
        assert_eq!(location_name(0xFF), "—");
    }
    #[test]
    fn unknown_location() {
        assert_eq!(location_name(0xFE), "Unknown Location");
    }
    #[test]
    fn towns_present_in_map_area_name() {
        assert_eq!(map_area_name(3, 0x00), "Pallet Town");
        assert_eq!(map_area_name(3, 0x0B), "Saffron City");
        assert_eq!(map_area_name(3, 0x11), "Seven Island");
        assert_eq!(map_area_name(3, 0x12), "Six Island");
    }

    // ── dungeon_floors ────────────────────────────────────────────────────────

    #[test]
    fn outdoor_route_has_no_floors() {
        assert!(dungeon_floors(3, 0x13).is_empty());
    }
    #[test]
    fn unknown_pair_has_no_floors() {
        assert!(dungeon_floors(0xFF, 0xFF).is_empty());
    }
    #[test]
    fn mt_moon_1f_groups_all_floors() {
        let floors = dungeon_floors(1, 0x01);
        assert!(floors.contains(&(1, 0x01)));
        assert!(floors.contains(&(1, 0x02)));
        assert!(floors.contains(&(1, 0x03)));
    }
    #[test]
    fn mt_moon_b2f_same_group_as_1f() {
        assert_eq!(dungeon_floors(1, 0x03), dungeon_floors(1, 0x01));
    }
    #[test]
    fn rock_tunnel_both_floors() {
        let floors = dungeon_floors(1, 0x51);
        assert!(floors.contains(&(1, 0x51)));
        assert!(floors.contains(&(1, 0x52)));
    }
    #[test]
    fn seafoam_islands_b4f_in_group() {
        let floors = dungeon_floors(1, 0x57);
        assert_eq!(floors.len(), 5);
        assert!(floors.contains(&(1, 0x53)));
    }
    #[test]
    fn pokemon_tower_all_seven_floors() {
        let floors = dungeon_floors(1, 0x5A);
        assert_eq!(floors.len(), 7);
    }
    #[test]
    fn lost_cave_room_7_in_group() {
        let floors = dungeon_floors(2, 0x13);
        assert_eq!(floors.len(), 15);
        assert!(floors.contains(&(2, 0x0C)));
    }
    #[test]
    fn tanoby_chamber_groups_all_seven() {
        let floors = dungeon_floors(2, 0x1F);
        assert_eq!(floors.len(), 7);
        assert!(floors.contains(&(2, 0x1B)));
        assert!(floors.contains(&(2, 0x21)));
    }
    #[test]
    fn diglett_cave_entrance_in_group() {
        let floors = dungeon_floors(1, 0x24);
        assert!(floors.contains(&(1, 0x25)));
    }
}
