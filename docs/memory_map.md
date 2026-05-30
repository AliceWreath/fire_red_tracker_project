# Emulator Memory Map

All addresses are GBA bus addresses as seen by the game.  RetroArch exposes
these directly via `READ_CORE_MEMORY`.  The tracker snapshots EWRAM and IWRAM
in full via UDP every 100 ms; all subsequent reads are pure Rust array indexing
into those snapshots (no live UDP access after the snapshot is taken).

## GBA Memory Regions Used

```
GBA Bus Address     Size      Region     Usage in tracker
─────────────────────────────────────────────────────────────────────
0x02000000          256 KiB   EWRAM      External Work RAM (snapshotted)
0x03000000           32 KiB   IWRAM      Internal Work RAM (snapshotted)
0x08000000          ~32 MiB   ROM        Loaded from .gba file at startup
                                          (not via UDP — read directly)
```

> IWRAM and EWRAM snapshots are stored as `ArcSwap<Vec<u8>>` globals in
> `fire_red_memory`.  Callers compute `address - base` to get the byte offset
> into the snapshot slice.

---

## EWRAM Layout (FireRed USA Rev 1)

```
EWRAM offset   Bus address      Size      Symbol / Field
(0x02000000 base)
────────────────────────────────────────────────────────────────────────
0x00024029     0x02024029        1 byte   gPlayerPartyCount
                                          valid range: 0–6

0x00024284     0x02024284      600 bytes  gPlayerParty[6]
                                          6 × 100-byte Pokemon structs
                                          see Party Layout below

0x0002402C     0x0202402C      100 bytes  gEnemyParty[0]
                                          wild Pokémon slot
                                          personality word at +0x00
                                          never cleared between battles —
                                          change detection is how new
                                          encounters are identified

0x00024298     0x02024298       19 bytes  SaveBlock2 header
                                          [0..7]  trainer name (GBA-encoded)
                                          [8]     gender
                                          [10..11] trainer OT ID (little-endian)
                                          [12..15] play time (h, m, s, frames)

0x00031DBC     0x02031DBC        2 bytes  gMapHeader current position
                                          [0] map_group
                                          [1] map_name (bank)

  SaveBlock1   resolved via IWRAM pointer at 0x03005008:
                               varies    SaveBlock1 structure in EWRAM
    + 0x0EE0                  256 bytes  gSaveBlock1.flags[]
      bit index 0x820..0x827             Boulder Badge through Earth Badge
      byte index 0x104, bits 0..7

  SaveBlock3   resolved via IWRAM pointer at 0x03005010:
                               varies    gPokemonStorage in EWRAM
    + 0x0004                 33600 bytes gPokemonStorage.boxes
                                          14 boxes × 30 slots × 80 bytes
```

### Party Pokemon Layout (100 bytes, little-endian)

Each of the 6 party slots at `gPlayerParty`:

```
Offset  Size  Field
──────────────────────────────────────
+0x00    4    personality (u32)
+0x04    4    ot_id (u32)
+0x08   10    nickname (GBA-encoded, 10 chars max)
+0x12    2    species (u16, national dex number)
+0x14    1    level
+0x22    2    current HP (u16)
+0x24    2    max HP (u16)
+0x28    2    attack (u16)
+0x2A    2    defense (u16)
+0x2C    2    speed (u16)
+0x2E    2    sp_attack (u16)
+0x30    2    sp_defense (u16)
(other fields present but not all read by tracker)
```

### gEnemyParty[0] (100 bytes, same layout)

Only `personality` (+0x00) and `ot_id` (+0x04) are read for encounter
detection.  Wild Pokémon have `ot_id = 0`; trainer Pokémon have `ot_id`
matching the trainer.  The tracker distinguishes wild from trainer battles by
comparing `enemy.ot_id` to the lead party member's `ot_id`.

---

## IWRAM Layout

```
IWRAM offset   Bus address      Size      Symbol
(0x03000000 base)
────────────────────────────────────────────────────────────────────────
0x00005008     0x03005008        4 bytes  gSaveBlock1Ptr
                                          pointer to SaveBlock1 in EWRAM
                                          subtract 0x02000000 for offset

0x00005010     0x03005010        4 bytes  gPokemonStoragePtr
                                          pointer to gPokemonStorage in EWRAM
                                          subtract 0x02000000 for offset
```

Pointer resolution (pseudocode):

```
fn resolve_ewram_ptr(iwram: &[u8], iwram_offset: usize) -> Option<usize> {
    let ptr = u32::from_le_bytes(iwram[iwram_offset..+4]);
    if ptr < 0x02000000 || ptr > 0x0203FFFF { return None; }
    Some((ptr - 0x02000000) as usize)
}
```

---

## ROM Layout (FireRed USA Rev 1)

These offsets are fixed for the unmodified ROM; the scanner is used to locate
the wild encounter table because its position varies across patches.

```
ROM offset    Size               Field
──────────────────────────────────────────────────────────────────────
0x00000128    4 bytes            FRONT_SPRITE_TABLE_PTR
                                 two-level pointer → front sprite data
                                 used to locate compressed sprite tiles

0x00245F5B    ~11 bytes/entry    Pokémon name table
                                 GBA-encoded, 0xFF-terminated entries
                                 entry 0 = "????????" (bad egg)
                                 entry 1 = "BULBASAUR", etc.

0x0024FCB0    13 bytes/entry     Ability name table
                                 0x019B entries (411 abilities)

0x002547F4    28 bytes/entry     Base stats table
                                 +0x16 = ability slot 0
                                 +0x17 = ability slot 1

Scanned        20 bytes/entry    WildMonHeader table
                                 Location found at runtime by fire_red_scanner
                                 scanning for ≥50 consecutive valid headers
                                 followed by a 0xFF sentinel.
```

### WildMonHeader Entry (20 bytes, ROM format)

```
Offset  Size  Field
──────────────────────────────────────────────────────
+0x00    1    map_group
+0x01    1    map_num
+0x02    2    padding (must be 0x0000)
+0x04    4    land_encounters_ptr   (ROM pointer or 0)
+0x08    4    water_encounters_ptr  (ROM pointer or 0)
+0x0C    4    rock_smash_ptr        (ROM pointer or 0)
+0x10    4    fishing_ptr           (ROM pointer or 0)
```

A pointer is valid if it is either 0 (no encounters for that type) or in the
range `[0x08000000, 0x09FFFFFF]`.  The scanner requires all four fields of
every entry to satisfy this check.

### WildPokemonInfo Entry (pointed to by WildMonHeader)

```
Offset  Size  Field
──────────────────────────────────────
+0x00    1    encounter_rate
+0x01    3    padding
+0x04    4    pokemon_list_ptr  (ROM pointer)
```

### WildPokemon Entry (6 bytes per slot)

```
Offset  Size  Field
──────────────────────────────────────
+0x00    1    min_level
+0x01    1    max_level
+0x02    2    species (national dex)
```

The list is terminated by a sentinel: `min_level == 0x15 && max_level == 0`, or
when the running pointer would go past the `0x08000000` boundary, or after
`MAX_ENTRIES = 200` slots (overflow guard).

---

## Address Quick Reference

| Purpose | Bus Address | Source |
|---------|-------------|--------|
| Party size | `0x02024029` | EWRAM |
| Party data (6 × 100 B) | `0x02024284` | EWRAM |
| Wild enemy Pokémon slot | `0x0202402C` | EWRAM |
| Trainer name / OT ID | `0x02024298` | EWRAM |
| Current map (group, name) | `0x02031DBC` | EWRAM |
| SaveBlock1 pointer | `0x03005008` | IWRAM |
| gPokemonStorage pointer | `0x03005010` | IWRAM |
| Badge flags (via SaveBlock1) | SaveBlock1 + `0x0EE0` | EWRAM |
| Pokémon names | ROM `0x00245F5B` | .gba file |
| Ability names | ROM `0x0024FCB0` | .gba file |
| Base stats | ROM `0x002547F4` | .gba file |
| Wild encounter table | ROM (scanned) | .gba file |
