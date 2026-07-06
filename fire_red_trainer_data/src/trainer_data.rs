//! Trainer Data
//!
//! Holds the [`PlayerData`] structure used by the `fire_red_trainer_data` crate.
//! Parses directly from raw bytes sliced out of the EWRAM snapshot.

use fire_red_get_values::*;

/// Maximum number of characters in a player or rival name.
pub const PLAYER_NAME_LENGTH: usize = 7;

/// Length in bytes of the packed trainer ID field.
pub const TRAINER_ID_LENGTH: usize = 4;

/// Number of raw bytes that make up the [`PlayerData`] block in memory.
///
/// Spans the first 19 bytes of SaveBlock2:
/// - 8 bytes: trainer name (7 chars + 0xFF terminator)
/// - 1 byte:  trainer gender
/// - 1 byte:  special save warp flags
/// - 4 bytes: trainer ID (2 public TID + 2 SID)
/// - 2 bytes: play time hours
/// - 1 byte:  play time minutes
/// - 1 byte:  play time seconds
/// - 1 byte:  play time V-blank counter
pub const PLAYER_DATA_SIZE: usize = 19;

/// Trainer and play-time metadata read from SaveBlock2.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PlayerData {
    /// Raw trainer name bytes (7 chars + 0xFF terminator).
    pub trainer_name: [u8; PLAYER_NAME_LENGTH + 1],

    /// Raw rival name bytes (7 chars + 0xFF terminator).
    ///
    /// Not yet implemented — always zero.
    pub rival_name: [u8; PLAYER_NAME_LENGTH + 1],

    /// Trainer gender: 0 = male, 1 = female.
    pub trainer_gender: u8,

    /// Bitfield used by various save/warp routines.
    pub special_save_warp_flags: u8,

    /// Packed trainer ID: bytes 0–1 are the public TID, bytes 2–3 are the SID.
    pub player_trainer_id: [u8; TRAINER_ID_LENGTH],

    /// Play time: hours component.
    pub player_time_hours: u16,

    /// Play time: minutes component (0–59).
    pub player_time_minutes: u8,

    /// Play time: seconds component (0–59).
    pub player_time_seconds: u8,

    /// Play time: sub-second V-blank counter (increments at 60 Hz).
    pub player_time_v_blanks: u8,

    /// Decoded trainer name as a Rust `String`.
    pub trainer_name_string: String,

    /// Decoded rival name as a Rust `String`.
    ///
    /// Not yet implemented.
    pub rival_name_string: String,
}

impl PlayerData {
    /// Parses a [`PlayerData`] from a raw byte slice.
    ///
    /// `buffer` must be at least [`PLAYER_DATA_SIZE`] bytes long, starting
    /// at the byte corresponding to [`PLAYER_DATA_ADDR`] within the EWRAM
    /// snapshot. Returns `None` if the buffer is too short.
    pub fn from_bytes(buffer: &[u8]) -> Option<Self> {
        if buffer.len() < PLAYER_DATA_SIZE {
            return None;
        }

        let mut offset = 0;

        let mut trainer_name = [0u8; PLAYER_NAME_LENGTH + 1];
        trainer_name.copy_from_slice(&buffer[offset..offset + PLAYER_NAME_LENGTH + 1]);
        offset += PLAYER_NAME_LENGTH + 1;

        let trainer_name_string =
            fire_red_text::gba_string_to_ascii(&trainer_name, PLAYER_NAME_LENGTH, 0)
                .trim_matches('\0')
                .trim_ascii()
                .to_string();

        // Rival name is not yet located in SaveBlock2 — placeholder for now.
        let rival_name = [0u8; PLAYER_NAME_LENGTH + 1];
        let rival_name_string = String::from("not implemented yet");

        let trainer_gender = read_u8(buffer, offset);
        offset += 1;
        let special_save_warp_flags = read_u8(buffer, offset);
        offset += 1;

        let mut player_trainer_id = [0u8; TRAINER_ID_LENGTH];
        player_trainer_id.copy_from_slice(&buffer[offset..offset + TRAINER_ID_LENGTH]);
        offset += TRAINER_ID_LENGTH;

        let player_time_hours = read_u16(buffer, offset);
        offset += 2;
        let player_time_minutes = read_u8(buffer, offset);
        offset += 1;
        let player_time_seconds = read_u8(buffer, offset);
        offset += 1;
        let player_time_v_blanks = read_u8(buffer, offset);

        Some(PlayerData {
            trainer_name,
            rival_name,
            trainer_gender,
            special_save_warp_flags,
            player_trainer_id,
            player_time_hours,
            player_time_minutes,
            player_time_seconds,
            player_time_v_blanks,
            trainer_name_string,
            rival_name_string,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic 19-byte SaveBlock2 prefix: name "RED" (GBA encoding:
    /// R=0xCC, E=0xBF, D=0xBE) terminated by 0xFF and padded with 0xFF as the
    /// game does, female, warp flags 0x02, TID 12345 / SID 0xDEAD,
    /// play time 123:45:59 + 33 v-blanks.
    fn sample_buffer() -> [u8; PLAYER_DATA_SIZE] {
        [
            0xCC, 0xBF, 0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // trainer name
            0x01, // gender: female
            0x02, // special save warp flags
            0x39, 0x30, 0xAD, 0xDE, // TID 12345 (0x3039), SID 0xDEAD
            0x7B, 0x00, // hours = 123
            45,   // minutes
            59,   // seconds
            33,   // v-blanks
        ]
    }

    #[test]
    fn from_bytes_decodes_trainer_name() {
        let data = PlayerData::from_bytes(&sample_buffer()).unwrap();
        assert_eq!(data.trainer_name_string, "RED");
        assert_eq!(data.trainer_name, [0xCC, 0xBF, 0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn from_bytes_decodes_scalar_fields() {
        let data = PlayerData::from_bytes(&sample_buffer()).unwrap();
        assert_eq!(data.trainer_gender, 1);
        assert_eq!(data.special_save_warp_flags, 0x02);
        assert_eq!(data.player_trainer_id, [0x39, 0x30, 0xAD, 0xDE]);
        assert_eq!(data.player_time_hours, 123);
        assert_eq!(data.player_time_minutes, 45);
        assert_eq!(data.player_time_seconds, 59);
        assert_eq!(data.player_time_v_blanks, 33);
    }

    #[test]
    fn from_bytes_full_length_name_uses_all_seven_chars() {
        let mut buf = sample_buffer();
        // "MISTYAB": M=0xC7, I=0xC3, S=0xCD, T=0xCE, Y=0xD3, A=0xBB, B=0xBC
        buf[..8].copy_from_slice(&[0xC7, 0xC3, 0xCD, 0xCE, 0xD3, 0xBB, 0xBC, 0xFF]);
        let data = PlayerData::from_bytes(&buf).unwrap();
        assert_eq!(data.trainer_name_string, "MISTYAB");
    }

    #[test]
    fn from_bytes_rejects_short_buffer() {
        let buf = sample_buffer();
        assert_eq!(PlayerData::from_bytes(&buf[..PLAYER_DATA_SIZE - 1]), None);
        assert_eq!(PlayerData::from_bytes(&[]), None);
    }

    #[test]
    fn from_bytes_accepts_oversized_buffer() {
        let mut buf = sample_buffer().to_vec();
        buf.extend_from_slice(&[0xAA; 32]); // trailing SaveBlock2 bytes
        let data = PlayerData::from_bytes(&buf).unwrap();
        assert_eq!(data.trainer_name_string, "RED");
        assert_eq!(data.player_time_hours, 123);
    }
}
