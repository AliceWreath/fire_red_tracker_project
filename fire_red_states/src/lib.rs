//! Shared message types used internally by the tracker and aggregator.
//!
//! [`ClientMessage`] is dispatched from the aggregator web layer to the per-slot
//! game-polling thread via mpsc, and is also used directly by the standalone tracker.
//! [`ServerMessage`] is a legacy type retained for bincode index stability;
//! it is not sent over any network connection in the current architecture.

/// The highest valid National Pokédex number in FireRed (Generation III cap).
///
/// Used to filter out placeholder or sentinel species values that appear in
/// ROM tables and EWRAM slots but do not correspond to real Pokémon.
pub const MAX_NATIONAL_DEX_FIRERED: u16 = 386;

/// Messages sent from a client to the server.
///
/// # IMPORTANT — bincode variant ordering
/// bincode encodes enum variants by their **positional index** (0, 1, 2, …).
/// Inserting a new variant anywhere other than the end silently breaks
/// deserialization between old and new binaries.  New variants MUST be
/// appended at the end only.  Current stable indices:
///   0 = RequestTextures
///   1 = EndRun
///   2 = NewRun
///   3 = Hello
///   4 = GiveItem
///   5 = MakeShiny
///   6 = TakeItem
///   7 = ChangeSpecies
///   8 = ChangeAbility
///   9 = ChangeGender
///  10 = ChangeNickname
///  11 = ChangeHeldItem
///  12 = CureStatus
///  13 = ChangeNature
///  14 = RestorePp
///  15 = SetFriendship
///  16 = ChangeMove
///  17 = SetIvs
///  18 = IncreaseIvs
///  19 = SetEvs
///  20 = IncreaseEvs
///  21 = RestoreHp
///  22 = HealParty
///  23 = SetExp
///  24 = SetLevel
///  25 = LearnMove
///  26 = ForgetMove
///  27 = SetPokerus
///  28 = SetPpUps
///  29 = RevivePokemon
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub enum ClientMessage {
    RequestTextures(Vec<u16>), // index 0 — do not reorder
    EndRun,                    // index 1 — do not reorder
    NewRun,                    // index 2 — do not reorder
    Hello(String),             // index 3 — do not reorder
    /// Inject an item into the player's items pocket. `item_id` is the Gen III
    /// FireRed item ID (e.g. 13 = Potion). `quantity` is capped at 99 in-game.
    GiveItem {
        item_id: u16,
        quantity: u16,
    }, // index 4 — do not reorder
    /// Make the party Pokémon at `party_position` (0–5) shiny by rewriting its
    /// stored OT Secret ID so the Gen III shiny formula holds. Personality is
    /// unchanged, preserving nature, ability, gender, and data block order.
    MakeShiny {
        party_position: u8,
    }, // index 5 — do not reorder
    /// Remove `quantity` of `item_id` from the player's bag. If the current
    /// quantity is ≤ `quantity` the item is fully removed and the pocket is
    /// compacted; otherwise the quantity is decremented in place.
    TakeItem {
        item_id: u16,
        quantity: u16,
    }, // index 6 — do not reorder
    /// Change the party Pokémon at `party_position` (0–5) to `new_species`.
    /// Personality, OT ID, nickname, EVs, IVs, and moves are all preserved.
    /// Only the species field in the Growth substructure is updated; the
    /// checksum is recalculated and the data block re-encrypted.
    ChangeSpecies {
        party_position: u8,
        new_species: u16,
    }, // index 7 — do not reorder
    /// Switch the party Pokémon at `party_position` (0–5) to ability slot
    /// `ability_slot` (0 = first ability, 1 = second ability). Sets or clears
    /// bit 31 of the IV/egg/ability word in the Misc substructure and
    /// recalculates the checksum.
    ChangeAbility {
        party_position: u8,
        ability_slot: u8,
    }, // index 8 — do not reorder
    /// Change the party Pokémon at `party_position` (0–5) to `target_gender`
    /// (0 = male, 1 = female) by adjusting the low byte of the personality.
    /// Nature (personality % 25) is always preserved. If the Pokémon is shiny
    /// only personality bytes that keep the shiny formula satisfied are
    /// considered; the call fails if no such byte exists for the requested gender.
    ChangeGender {
        party_position: u8,
        target_gender: u8,
    }, // index 9 — do not reorder
    /// Rename the party Pokémon at `party_position` (0–5). `nickname` is UTF-8;
    /// the tracker converts it to GBA encoding, silently dropping unmapped chars
    /// and truncating to 10 characters. Shiny, nature, gender, and all encrypted
    /// data are untouched.
    ChangeNickname {
        party_position: u8,
        nickname: String,
    }, // index 10 — do not reorder
    /// Set the held item of the party Pokémon at `party_position` (0–5) to
    /// `item_id`. Use `item_id = 0` to remove the held item. The held-item field
    /// in the Growth substructure is updated; checksum is recalculated.
    ChangeHeldItem {
        party_position: u8,
        item_id: u16,
    }, // index 11 — do not reorder
    /// Clear the status condition (burn, sleep, paralysis, poison, freeze) of the
    /// party Pokémon at `party_position` (0–5) by zeroing the 4-byte status word
    /// at bytes 80–83 of the PartyPokemon struct.
    CureStatus {
        party_position: u8,
    }, // index 12 — do not reorder
    /// Change the nature of the party Pokémon at `party_position` (0–5) to
    /// `target_nature` (0–24). Adjusts the low byte of the personality to satisfy
    /// `personality % 25 == target_nature`, preserving gender (for species where
    /// gender is personality-derived) and shiny status. The substructure block
    /// order is rearranged when `personality % 24` changes.
    ChangeNature {
        party_position: u8,
        target_nature: u8,
    }, // index 13 — do not reorder
    /// Restore PP on all four move slots to their current maximum (base PP +
    /// PP-Up bonus). Only slots with a move equipped are affected; empty slots
    /// (move_id = 0) are skipped. Shiny status and all other fields are
    /// untouched.
    RestorePp {
        party_position: u8,
    }, // index 14 — do not reorder
    /// Set the friendship (happiness) byte of the party Pokémon at
    /// `party_position` (0–5) to `friendship` (0–255). Friendship is stored at
    /// Growth substructure offset 9; checksum is recalculated.
    SetFriendship {
        party_position: u8,
        friendship: u8,
    }, // index 15 — do not reorder
    /// Replace the move at `slot` (0–3) of the party Pokémon at
    /// `party_position` (0–5) with `move_id`. PP is set to the maximum for the
    /// new move (base PP + current PP-Up bonus). Use `move_id = 0` to clear the
    /// slot.
    ChangeMove {
        party_position: u8,
        slot: u8,
        move_id: u16,
    }, // index 16 — do not reorder
    /// Set all six IVs of the party Pokémon at `party_position` (0–5). Each
    /// stat is clamped to 0–31. The egg and ability bits in the IV/egg/ability
    /// word (bits 30–31 of the Misc substructure) are preserved.
    SetIvs {
        party_position: u8,
        hp: u8,
        atk: u8,
        def: u8,
        spd: u8,
        spa: u8,
        spdef: u8,
    }, // index 17 — do not reorder
    /// Add to each IV of the party Pokémon at `party_position` (0–5), clamping
    /// each result at 31. Egg and ability bits are preserved.
    IncreaseIvs {
        party_position: u8,
        hp: u8,
        atk: u8,
        def: u8,
        spd: u8,
        spa: u8,
        spdef: u8,
    }, // index 18 — do not reorder
    /// Set all six EVs of the party Pokémon at `party_position` (0–5). Each
    /// stat is stored as a raw byte (0–255); the per-stat game cap is not
    /// enforced by this command.
    SetEvs {
        party_position: u8,
        hp: u8,
        atk: u8,
        def: u8,
        spd: u8,
        spa: u8,
        spdef: u8,
    }, // index 19 — do not reorder
    /// Add to each EV of the party Pokémon at `party_position` (0–5), clamping
    /// each result at 255. The 510-total game cap is not enforced.
    IncreaseEvs {
        party_position: u8,
        hp: u8,
        atk: u8,
        def: u8,
        spd: u8,
        spa: u8,
        spdef: u8,
    }, // index 20 — do not reorder
    /// Restore the current HP of the party Pokémon at `party_position` (0–5)
    /// to its maximum. Reads the calculated max-HP word (PartyPokemon offset
    /// 88–89) and writes it to the current-HP word (offset 86–87). No
    /// encrypted data is touched.
    RestoreHp {
        party_position: u8,
    }, // index 21 — do not reorder
    /// Restore the HP and cure the status condition of every occupied party
    /// slot in one command. Equivalent to calling [`RestoreHp`] + [`CureStatus`]
    /// on each of the six party positions, but reuses a single UDP socket.
    HealParty, // index 22 — do not reorder
    /// Set the experience points of the party Pokémon at `party_position` (0–5)
    /// to exactly `exp`. The Growth substructure is updated, checksum is
    /// recalculated, and the block is re-encrypted. The level byte is NOT
    /// updated — use [`SetLevel`] to change both atomically.
    SetExp {
        party_position: u8,
        exp: u32,
    }, // index 23 — do not reorder
    /// Set the level of the party Pokémon at `party_position` (0–5) to `level`
    /// (1–100). Writes the level byte at PartyMon offset 84, and also updates
    /// the experience in the Growth substructure to the Gen III minimum for that
    /// level and growth rate so the game does not immediately re-sync downwards.
    SetLevel {
        party_position: u8,
        level: u8,
    }, // index 24 — do not reorder
    /// Place `move_id` into the first empty move slot (move_id == 0) of the
    /// party Pokémon at `party_position` (0–5). PP is set to the maximum for
    /// the move. No-op if all four slots are occupied or the move is already
    /// known.
    LearnMove {
        party_position: u8,
        move_id: u16,
    }, // index 25 — do not reorder
    /// Clear the move at `slot` (0–3) of the party Pokémon at `party_position`
    /// (0–5) and compact subsequent moves left. PP bytes are shifted to match.
    ForgetMove {
        party_position: u8,
        slot: u8,
    }, // index 26 — do not reorder
    /// Infect the party Pokémon at `party_position` (0–5) with Pokérus (strain
    /// 1, 4 days remaining). No-op if already actively infected.
    SetPokerus {
        party_position: u8,
    }, // index 27 — do not reorder
    /// Set PP-Up bonus counts (0–3 per slot) for all four move slots and refill
    /// each slot's current PP to the new maximum.
    SetPpUps {
        party_position: u8,
        pp0: u8,
        pp1: u8,
        pp2: u8,
        pp3: u8,
    }, // index 28 — do not reorder
    /// Look up `personality` in the current run's `dead_pokemon` table and write
    /// the revived Pokémon at `party_position` (0–5) with 1 HP.
    RevivePokemon {
        party_position: u8,
        personality: u32,
    }, // index 29 — do not reorder
    /// Revert the last injection command by writing the bytes that were saved
    /// before that write to RetroArch memory.  No-op if no command has been
    /// executed on this connection yet.
    UndoLastCommand, // index 30 — do not reorder
                               // Append new variants here only.
}

/// Legacy server-to-client message type. Retained for bincode index stability; not
/// transmitted over any active network connection in the current architecture.
///
/// # IMPORTANT — bincode variant ordering
/// Same constraint as [`ClientMessage`].  Current stable indices:
///   0 = State
///   1 = Textures
///   2 = RunChanged
///   3 = BoxData
///   4 = Bag
#[derive(serde::Serialize, serde::Deserialize)]
pub enum ServerMessage {
    State(Box<GameState>),     // index 0 — do not reorder
    Textures(Vec<SpriteData>), // index 1 — do not reorder
    RunChanged(Option<u32>),   // index 2 — do not reorder
    BoxData(Vec<BoxEntry>),    // index 3 — do not reorder
    Bag(BagPockets),           // index 4 — do not reorder
                               // Append new variants here only.
}

/// One item slot read from the player's bag (quantity already XOR-decrypted).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ItemSlot {
    pub item_id: u16,
    pub quantity: u16,
}

/// All four bag pockets decoded from the player's SaveBlock1.
///
/// Sent by the tracker every 2 seconds as [`ServerMessage::Bag`].
/// Contains only occupied slots (item_id != 0); empty slots are omitted.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct BagPockets {
    pub items: Vec<ItemSlot>,
    pub key_items: Vec<ItemSlot>,
    pub balls: Vec<ItemSlot>,
    pub tms: Vec<ItemSlot>,
}

/// A compact snapshot of one PC box slot for network transmission.
///
/// Built by the tracker from the live EWRAM snapshot and sent to the aggregator
/// every ~5 seconds so the web overlay can display the full box contents.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct BoxEntry {
    /// Zero-based index of the PC box (0–13).
    pub box_index: u8,
    /// Zero-based slot within the box (0–29).
    pub slot_index: u8,
    pub species: u16,
    pub species_name: String,
    pub nickname: String,
    pub personality: u32,
    pub ot_id: u32,
    pub is_shiny: bool,
    pub nature: String,
    pub iv_hp: u8,
    pub iv_atk: u8,
    pub iv_def: u8,
    pub iv_spe: u8,
    pub iv_spa: u8,
    pub iv_spd: u8,
    pub is_egg: bool,
    /// `0` = male, `1` = female, `2` = genderless.
    pub gender: u8,
}

/// Which sprite image a [`SpriteData`] packet carries.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum SpriteVariant {
    /// Standard front-facing battle sprite (default).
    #[default]
    Front,
    /// Rear-facing sprite used on the player's side of battle.
    Back,
}

/// Serialized Pokemon sprite texture data for network transmission.
///
/// Pixel data is stored as zlib-compressed RGBA bytes.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SpriteData {
    pub species: u16,
    pub shiny: bool,
    /// Which image this packet carries; defaults to [`SpriteVariant::Front`] when
    /// deserializing packets from older server versions that lack the field.
    #[serde(default)]
    pub variant: SpriteVariant,
    pub pixels: Vec<u8>, // zlib-compressed RGBA bytes
    pub width: u32,
    pub height: u32,
}

/// Shared game state transmitted between server and clients.
///
/// Contains both the current player party and wild encounter data.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct GameState {
    /// Current player party pokemon.
    pub party: Vec<fire_red_party_monitor::Pokemon>,

    /// Wild encounter table/header data.
    pub encounters: fire_red_pokemon_data::WildPokemonHeader,

    /// Trainer name
    pub player_name: String,

    /// Current collected badges
    pub badge_state: Option<fire_red_badge::BadgeState>,

    /// Human-readable name for the current wild-encounter zone, resolved by the
    /// tracker from the ROM's `gMapGroupsAndMaps` table. Empty when the current
    /// map has no wild encounters.
    pub zone_name: String,

    /// Actual player map-group ID read directly from EWRAM (0x02031DBC[0]).
    /// This is the true map position and is independent of the encounter header,
    /// so it must be used — not `encounters.map_group` — to key zone transitions.
    pub current_map_group: u8,

    /// Actual player map-name ID read directly from EWRAM (0x02031DBC[1]).
    pub current_map_name: u8,

    /// Preferred display slot index (1 = first column, 2 = second, …).
    /// `None` means no preference; the aggregator places those slots last,
    /// then breaks ties alphabetically by player name.
    pub preferred_player: Option<u8>,

    /// One-shot clause enforcement warnings generated by the encounter tracker
    /// this tick (e.g. "Species clause: Pidgey already caught").
    /// Drained after each send; never repeated across ticks.
    pub warnings: Vec<String>,

    /// Current Pokédollars read from SaveBlock1 and decrypted with the
    /// security key.  Defaults to 0 for older tracker versions.
    #[serde(default)]
    pub money: u32,

    /// In-game save-file play time: hours component (can exceed 999).
    #[serde(default)]
    pub play_time_hours: u16,

    /// In-game save-file play time: minutes component (0–59).
    #[serde(default)]
    pub play_time_minutes: u8,

    /// In-game save-file play time: seconds component (0–59).
    #[serde(default)]
    pub play_time_seconds: u8,
}

// ---------------------------------------------------------------------------
// Mutex poison recovery
// ---------------------------------------------------------------------------

/// Extension trait for [`std::sync::Mutex`] that recovers from poison instead
/// of propagating it. A poisoned mutex means a thread panicked while holding
/// the lock; for this tracker's display-only state, stale data is safer than
/// crashing.
pub trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for std::sync::Mutex<T> {
    #[track_caller]
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| {
            let loc = std::panic::Location::caller();
            tracing::warn!("mutex poisoned at {}:{}: {e}", loc.file(), loc.line());
            e.into_inner()
        })
    }
}

// ---------------------------------------------------------------------------
// Base64 encoding
// ---------------------------------------------------------------------------

/// Encodes `data` as standard Base64 (RFC 4648 alphabet, `=` padding).
///
/// Hand-rolled to avoid adding a dependency for a single, tight use-site.
/// Used by the OBS WebSocket auth flow in the tracker and by the web overlay
/// sprite pipeline in the aggregator — kept here so both crates share one copy.
pub fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            CHARS[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ---------------------------------------------------------------------------
// GBA value helpers
// ---------------------------------------------------------------------------

/// Returns `true` if the Pokémon with `personality` and `ot_id` is shiny.
///
/// Gen III formula: `(p_high ^ p_low ^ id_high ^ id_low) < 8`.
pub fn is_shiny(personality: u32, ot_id: u32) -> bool {
    let p_high = (personality >> 16) as u16;
    let p_low = (personality & 0xFFFF) as u16;
    let id_high = (ot_id >> 16) as u16;
    let id_low = (ot_id & 0xFFFF) as u16;
    (p_high ^ p_low ^ id_high ^ id_low) < 8
}

#[cfg(test)]
mod base64_tests {
    use super::base64_encode;

    #[test]
    fn empty_input() {
        assert_eq!(base64_encode(&[]), "");
    }

    #[test]
    fn three_bytes_no_padding() {
        // RFC 4648 test vector: "Man" → "TWFu"
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn one_byte_two_padding_chars() {
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn two_bytes_one_padding_char() {
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn output_always_multiple_of_four() {
        for len in 0..=9usize {
            let data: Vec<u8> = (0..len as u8).collect();
            let encoded = base64_encode(&data);
            assert_eq!(
                encoded.len() % 4,
                0,
                "length {len} gave non-multiple-of-4 output"
            );
        }
    }

    #[test]
    fn hello_world() {
        assert_eq!(base64_encode(b"Hello, World!"), "SGVsbG8sIFdvcmxkIQ==");
    }
}
