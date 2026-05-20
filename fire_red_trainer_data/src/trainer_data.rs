//! Trainer Data
//! 
//! This module is hold the [`PlayerData`] structure
//! used with fire_red_trainer_data lib.
use fire_red_get_values::*;

/// Maximum length a player name can be
pub const PLAYER_NAME_LENGTH: usize = 7;

/// Length of the trainer ot_id
pub const TRAINER_ID_LENGTH: usize = 4;

/// RAM Address to SaveBlock2
pub static PLAYER_DATA_ADDR: u32 = 0x02024298;

/// struct that mimics a portion of the trainer data structure of 
/// pokemon FireRed
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PlayerData {
    pub trainer_name: [u8; PLAYER_NAME_LENGTH + 1], // 0x000       7 characters + 0xFF terminator
    pub rival_name: [u8; PLAYER_NAME_LENGTH + 1],
    pub trainer_gender: u8,          // 0x008       0 = male, 1 = female
    pub special_save_warp_flags: u8, // 0x009       bitfield, used by various warp routines
    pub player_trainer_id: [u8; TRAINER_ID_LENGTH], // 0x00A       4 bytes: [0-1] public TID, [2-3] SID
    pub player_time_hours: u16,                     // 0x00E
    pub player_time_minutes: u8,                    // 0x010
    pub player_time_seconds: u8,                    // 0x011
    pub player_time_v_blanks: u8, // 0x012       sub-second counter (ticks at 60Hz)
    pub trainer_name_string: String,
    pub rival_name_string: String,
}

impl PlayerData {
    /// parses the PlayerData out of RAM from a passed data slice
    pub fn fill_struct(rom: &[&str], mut offset: usize) -> Option<PlayerData> {
        if offset > rom.len() {
            return None;
        }
        //if offset < 0x02000000 || offset > 0x02000000 {
        //    return None;
        //}

        let mut trainer_name = [0u8; PLAYER_NAME_LENGTH + 1];
        for i in 0..PLAYER_NAME_LENGTH + 1 {
            trainer_name[i] = get_u8(&[rom[offset]]);
            offset += 1;
        }

        let trainer_name_string =
            fire_red_text::gba_string_to_ascii(&trainer_name, PLAYER_NAME_LENGTH, 0);
        let trainer_name_string = trainer_name_string
            .trim_matches('\0')
            .trim_ascii()
            .to_string();

        /*let mut rival_name = [0u8; PLAYER_NAME_LENGTH + 1];
        for i in 0..PLAYER_NAME_LENGTH + 1 {
            rival_name[i] = get_u8(&[rom[offset]]);
            offset += 1;
        }
        dbg!(&rival_name);
        let rival_name_string = fire_red_text::gba_string_to_ascii(&rival_name, PLAYER_NAME_LENGTH, 0);
        dbg!(&rival_name_string);*/
        let trainer_gender = get_u8(&[rom[offset]]);
        offset += 1;

        let special_save_warp_flags = get_u8(&[rom[offset]]);
        offset += 1;

        let mut player_trainer_id = [0u8; 4];
        for i in 0..4 {
            player_trainer_id[i] = get_u8(&[rom[offset]]);
            offset += 1;
        }

        let player_time_hours = get_u16(&rom[offset..offset + 2]);
        offset += 2;
        let player_time_minutes = get_u8(&[rom[offset]]);
        offset += 1;
        let player_time_seconds = get_u8(&[rom[offset]]);
        offset += 1;
        let player_time_v_blanks = get_u8(&[rom[offset]]);

        Some(PlayerData {
            trainer_name,
            rival_name: [0u8; 8],
            trainer_gender,
            special_save_warp_flags,
            player_trainer_id,
            player_time_hours,
            player_time_minutes,
            player_time_seconds,
            player_time_v_blanks,
            trainer_name_string,
            rival_name_string: String::from("not implemented yet"),
        })
    }
}
