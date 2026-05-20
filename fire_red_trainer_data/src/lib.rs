//! FireRed Trainer Data
//!
//! Gets trainer information that is needed for the codebase

mod trainer_data;

use arc_swap::ArcSwap;
use fire_red_retroarch_interfacing::{generate_command, get_from_retroarch};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use trainer_data::*;

/// static that holds various player data. Does not completely work yet.
static PLAYER_DATA: OnceLock<ArcSwap<PlayerData>> = OnceLock::new();

/// is true while the loop should be running, false will end the loop
static RUNNING: AtomicBool = AtomicBool::new(false);

/// holds how long the monitor loop sleeps in seconds between runs
const SLEEP_TIMER_IN_SECONDS: u64 = 15;

/// Initializes and/or returns the [`PlayerData`] static
pub fn initialize_static_trainer_data() -> &'static ArcSwap<PlayerData> {
    PLAYER_DATA.get_or_init(|| ArcSwap::from_pointee(PlayerData::default()))
}

/// Returns the [`PlayerData`] static
pub fn get_static_trainer_data() -> &'static ArcSwap<PlayerData> {
    initialize_static_trainer_data()
}

/// starts loop for monitoring changes in player data
pub fn start_loop() {
    RUNNING.store(true, Ordering::SeqCst);
    let _handle = std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            update_player_data();
            std::thread::sleep(std::time::Duration::from_secs(SLEEP_TIMER_IN_SECONDS));
        }
    });
}

/// Ends the monitoring loop
pub fn end_loop() {
    RUNNING.store(false, Ordering::SeqCst);
}

/// Used to check for changes in the player struct
fn update_player_data() {
    let mut got_return: bool = false;
    let mut ret: Option<Vec<String>> = None;
    while got_return == false {
        let command = generate_command(PLAYER_DATA_ADDR, 19);
        ret = get_from_retroarch(command.as_str(), 21);
        if ret.is_none() {
            eprintln!("Failed to read player data, retrying...");
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }
        got_return = true
    }
    let ret = ret.unwrap();
    let buffer: Vec<&str> = ret.iter().map(|s| s.as_str()).collect();
    let player = PlayerData::fill_struct(&buffer, 2);
    if player.is_none() {
        return;
    }
    let player = player.unwrap();
    let static_player = get_static_trainer_data().load();
    if player != **static_player {
        get_static_trainer_data().store(Arc::new(player));
    }
}

/// Was used to locate a known player name in RAM
pub fn find_player_name() -> Option<u32> {
    // Temporary debug: print bytes at the known player name address
    let debug_addr: u32 = 0x02024000;
    let command = generate_command(debug_addr, 16);
    if let Some(parts) = get_from_retroarch(&command, 18) {
        let bytes: Vec<u8> = parts[2..]
            .iter()
            .filter_map(|s| u8::from_str_radix(s, 16).ok())
            .collect();
        println!("Bytes at player name addr: {:02X?}", bytes);
    }
    dbg!("Start find_player_name()");
    let needle: &[u8] = &[0xBB, 0xE0, 0xDD, 0xD7, 0xD9]; // "Alice"
    let ewram_start: u32 = 0x02000000;
    let ewram_end: u32 = 0x02040000;
    let chunk_size: usize = 0x800;

    let mut addr = ewram_start;
    while addr < ewram_end {
        let scan = format!("{:08X}", &addr);
        dbg!(scan);
        let command = generate_command(addr, chunk_size);
        let expected = chunk_size + 2;
        if let Some(parts) = get_from_retroarch(&command, expected) {
            let bytes: Vec<u8> = parts[2..]
                .iter()
                .filter_map(|s| u8::from_str_radix(s, 16).ok())
                .collect();
            if let Some(offset) = bytes.windows(1).position(|w| w[0] == 0xBB) {
                let slice = &bytes[offset..std::cmp::min(offset + 8, bytes.len())];
                if slice.iter().any(|&b| b == 0xFF) {
                    println!("Candidate at {:08X}: {:02X?}", addr + offset as u32, slice);
                }
            }
            if let Some(offset) = bytes.windows(needle.len()).position(|w| w == needle) {
                return Some(addr + offset as u32);
            }
        }
        addr += chunk_size as u32;
    }
    None
}
