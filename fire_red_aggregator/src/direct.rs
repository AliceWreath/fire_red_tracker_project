//! Direct mode: the aggregator polls one or more RetroArch instances instead
//! of waiting for incoming tracker connections.
//!
//! # Startup
//!
//! [`spawn`] immediately starts polling all hosts listed in the config /
//! supplied on the CLI.  It returns a [`DirectConnector`] handle that lets the
//! web layer add further hosts at runtime via the `/join` page.
//!
//! # ROM
//!
//! If `rom_path` is `None`, [`crate::rom_fetch`] is called for each new host
//! to download the ROM directly from RetroArch's emulated memory and cache it
//! locally.  If the fetch fails (e.g. the game is not yet loaded) the host is
//! released from the known-host set so it can be retried later.

use crate::client::{MonitorSlot, PendingTexture, SharedSlots};
use fire_red_game_loop::{
    GameLoopConfig, GameLoopState, assemble_game_state, config::DupesClauseMode,
};
use fire_red_states::{LockOrRecover, SpriteVariant};
use std::collections::HashSet;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public handle
// ---------------------------------------------------------------------------

/// Handle that allows the web layer to add new RetroArch hosts at runtime.
///
/// Obtained from [`spawn`].  Thread-safe; wrap in `Arc` to share between
/// the web server and any other code that needs to add connections.
pub struct DirectConnector {
    pub default_port: u16,
    pub rom_path: Option<String>,
    pub poll_ms: u64,
    pub dupes_clause: DupesClauseMode,
    pub allow_species_repeats: bool,
    pub run_start_balls: u32,
    pub db: Option<String>,
    pub slots: SharedSlots,
    pub known: Arc<Mutex<HashSet<String>>>,
}

impl DirectConnector {
    /// Attempt to connect to `host:port` as a new slot.
    ///
    /// Returns `true` if the host was newly accepted, `false` if it is already
    /// being polled.  The actual connection (ROM fetch + game-loop start) happens
    /// in a background thread; the slot may take a few seconds to appear.
    pub fn connect(&self, host: String, port: u16) -> bool {
        try_add_host(
            host,
            port,
            self.rom_path.clone(),
            self.poll_ms,
            self.dupes_clause,
            self.allow_species_repeats,
            self.run_start_balls,
            self.db.clone(),
            self.slots.clone(),
            self.known.clone(),
        )
    }

    /// List of host:port strings currently being polled or set up.
    pub fn active_hosts(&self) -> Vec<String> {
        self.known.lock_or_recover().iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

/// Start direct-mode polling for all pre-configured hosts and return a
/// [`DirectConnector`] handle so new hosts can be added later via the web UI.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    initial_hosts: Vec<String>,
    retroarch_port: u16,
    rom_path: Option<String>,
    poll_ms: u64,
    dupes_clause: DupesClauseMode,
    allow_species_repeats: bool,
    run_start_balls: u32,
    db: Option<String>,
    slots: SharedSlots,
) -> Arc<DirectConnector> {
    let known: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    let connector = Arc::new(DirectConnector {
        default_port: retroarch_port,
        rom_path: rom_path.clone(),
        poll_ms,
        dupes_clause,
        allow_species_repeats,
        run_start_balls,
        db: db.clone(),
        slots: slots.clone(),
        known: known.clone(),
    });

    for host in initial_hosts {
        try_add_host(
            host,
            retroarch_port,
            rom_path.clone(),
            poll_ms,
            dupes_clause,
            allow_species_repeats,
            run_start_balls,
            db.clone(),
            slots.clone(),
            known.clone(),
        );
    }

    connector
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a ROM identity string from raw ROM bytes.
///
/// Combines the GBA header fields with a truncated SHA-256 digest so that
/// both header-level changes (different game code / version) and binary-level
/// changes (same header, patched content) are detected.
///
/// Format: `"<title>/<code>/<version>/<first-8-bytes-of-sha256-as-hex>"`,
/// e.g. `"POKEMON FIRE RED/BPRE/1/a1b2c3d4e5f60708"`.
/// Returns `"unknown"` if the bytes are too short to contain a GBA header.
pub fn rom_identity_from_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    if bytes.len() <= 0xBC {
        return "unknown".to_string();
    }
    let title = std::str::from_utf8(&bytes[0xA0..0xAC])
        .unwrap_or("")
        .trim_end_matches('\0')
        .trim()
        .to_string();
    let code = std::str::from_utf8(&bytes[0xAC..0xB0])
        .unwrap_or("????")
        .trim_end_matches('\0')
        .to_string();
    let version = bytes[0xBC];

    let digest = Sha256::digest(bytes);
    let short_hash = digest[..8]
        .iter()
        .fold(String::with_capacity(16), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{:02x}", b);
            s
        });

    format!("{}/{}/{}/{}", title, code, version, short_hash)
}

// ---------------------------------------------------------------------------
// Per-host setup
// ---------------------------------------------------------------------------

/// Claim `host:port` in `known` and spawn a thread to set up its slot.
///
/// Returns `true` if the host was newly claimed, `false` if it was already
/// known.  If setup fails (ROM fetch error, etc.) the host is removed from
/// `known` so the caller can retry.
#[allow(clippy::too_many_arguments)]
fn try_add_host(
    host: String,
    retroarch_port: u16,
    rom_path: Option<String>,
    poll_ms: u64,
    dupes_clause: DupesClauseMode,
    allow_species_repeats: bool,
    run_start_balls: u32,
    db: Option<String>,
    slots: SharedSlots,
    known: Arc<Mutex<HashSet<String>>>,
) -> bool {
    let key = format!("{}:{}", host, retroarch_port);
    if !known.lock_or_recover().insert(key.clone()) {
        return false; // already claimed
    }

    std::thread::spawn(move || {
        // Resolve ROM — use the configured path or fetch from RetroArch.
        let resolved_rom = match rom_path {
            Some(ref p) => p.clone(),
            None => match crate::rom_fetch::fetch_or_load_rom(&host, retroarch_port) {
                Ok(path) => path.to_string_lossy().into_owned(),
                Err(e) => {
                    tracing::error!(
                        "Direct mode: ROM fetch failed for {} — {}. \
                         Re-connect from the /join page to retry.",
                        host, e
                    );
                    known.lock_or_recover().remove(&key);
                    return;
                }
            }
        };

        // Allocate a new slot.
        let (slot, slot_idx) = {
            let mut lock = slots.lock_or_recover();
            let idx = lock.len();
            let s = Arc::new(MonitorSlot::new(
                idx,
                format!("direct:{}", host),
                db.clone(),
                Some(format!("{}:{}", host, retroarch_port)),
            ));
            lock.push(s.clone());
            (s, idx)
        };

        tracing::info!("Direct mode: slot {} → RetroArch at {}:{}", slot_idx, host, retroarch_port);

        let loop_state = Arc::new(GameLoopState::new());

        let rom_path_for_sprites = resolved_rom.clone();
        let loop_cfg = GameLoopConfig {
            retroarch_host: host.clone(),
            retroarch_port,
            rom_path: resolved_rom,
            is_clean: false,
            dupes_clause,
            allow_species_repeats,
            run_start_balls,
            preferred_player: None,
            poll_ms: Arc::new(AtomicU64::new(poll_ms)),
            livesplit_split_on_badges: false,
            livesplit_split_on_clear: true,
        };

        fire_red_game_loop::spawn_game_loop(loop_cfg, loop_state.clone());

        // Seed the shared ROM bytes so the sprite loader and the refresh API
        // both operate on the same buffer.
        let initial_rom = std::fs::read(&rom_path_for_sprites).unwrap_or_default();
        *slot.rom_identity.lock_or_recover() = rom_identity_from_bytes(&initial_rom);
        *slot.rom_bytes.lock_or_recover() = initial_rom;

        // Sprite-loader thread: decodes species sprites from the shared ROM
        // buffer.  The buffer can be atomically replaced at runtime by
        // POST /api/slot/:index/refresh_rom without restarting this thread.
        {
            let rom_bytes    = slot.rom_bytes.clone();
            let sprite_queue = slot.texture_request_queue.clone();
            let sprite_pending = slot.pending_textures.clone();

            std::thread::spawn(move || {
                loop {
                    let batch: Vec<u16> = {
                        let mut q = sprite_queue.lock_or_recover();
                        let mut all: Vec<u16> = q.drain(..).flatten().collect();
                        all.sort();
                        all.dedup();
                        all
                    };

                    if !batch.is_empty() {
                        let rom = rom_bytes.lock_or_recover();
                        if !rom.is_empty() {
                            for species in batch {
                                for shiny in [false, true] {
                                    match fire_red_image_data::get_pokemon_sprite(&rom, species, shiny) {
                                        Ok(img) => {
                                            let (w, h) = (img.width(), img.height());
                                            sprite_pending.lock_or_recover().push(PendingTexture {
                                                species,
                                                shiny,
                                                variant: SpriteVariant::Front,
                                                pixels: img.into_raw(),
                                                width: w,
                                                height: h,
                                            });
                                        }
                                        Err(e) => {
                                            tracing::debug!(
                                                "Direct mode: sprite decode failed \
                                                 for species {} shiny={}: {}",
                                                species, shiny, e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    std::thread::sleep(Duration::from_millis(50));
                }
            });
        }

        // Wire the game loop's encounter buffer into the slot so the refresh
        // endpoint can reset it without restarting the game loop thread.
        *slot.game_encounters.lock_or_recover() = Some(loop_state.encounters.clone());

        // Bridge thread: assemble GameState → slot and forward commands.
        let slot_state   = slot.state.clone();
        let slot_box     = slot.box_data.clone();
        let slot_bag     = slot.bag_data.clone();
        let slot_cmds    = slot.command_queue.clone();
        let slot_run_chg = slot.run_changed.clone();
        let loop_br      = loop_state;

        std::thread::spawn(move || {
            let mut last_box_send = std::time::Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now);
            let mut last_bag_send = std::time::Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now);

            loop {
                // Forward injection commands from the web layer to the game loop.
                {
                    let cmds: Vec<_> = slot_cmds.lock_or_recover().drain(..).collect();
                    if !cmds.is_empty() {
                        loop_br.command_queue.lock_or_recover().extend(cmds);
                    }
                }

                // Propagate run_changed signal.
                if loop_br.run_changed.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    slot_run_chg.store(true, std::sync::atomic::Ordering::Release);
                }

                // Assemble and publish the current GameState.
                let gs = assemble_game_state(&loop_br, None);
                *slot_state.lock_or_recover() = Some(gs);

                // Forward box snapshot every 5 s.
                if last_box_send.elapsed() >= Duration::from_secs(5) {
                    *slot_box.lock_or_recover() = loop_br.box_entries.lock_or_recover().clone();
                    last_box_send = std::time::Instant::now();
                }

                // Forward bag snapshot every 2 s.
                if last_bag_send.elapsed() >= Duration::from_secs(2) {
                    *slot_bag.lock_or_recover() = loop_br.bag.lock_or_recover().clone();
                    last_bag_send = std::time::Instant::now();
                }

                std::thread::sleep(Duration::from_millis(100));
            }
        });
    });

    true
}
