//! Direct mode: the aggregator polls one or more RetroArch instances directly
//! over UDP.
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
    /// `run_id`: `None` = start a new run; `Some(id)` = resume an existing run.
    ///
    /// Returns `true` if the host was newly accepted, `false` if it is already
    /// being polled.  The actual connection (ROM fetch + game-loop start) happens
    /// in a background thread; the slot may take a few seconds to appear.
    pub fn connect(&self, host: String, port: u16, run_id: Option<u32>, user_id: Option<u32>) -> bool {
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
            run_id,
            user_id,
        )
    }

    /// List of host:port strings currently being polled or set up.
    pub fn active_hosts(&self) -> Vec<String> {
        self.known.lock_or_recover().iter().cloned().collect()
    }

    /// Signal the slot for `host:port` to stop and remove it from the active
    /// set so the same host can be reconnected later.
    ///
    /// Returns `true` if the host was found and disconnected, `false` if it
    /// was not in the active set.
    ///
    /// After 20 minutes the disconnected slot's cached ROM bytes are freed.
    /// If the user reconnects before the timer fires, the new slot has its own
    /// `rom_bytes` Arc so the eviction only clears the old (already-unused) buffer.
    pub fn disconnect(&self, host: &str, port: u16) -> bool {
        let key = format!("{}:{}", host, port);
        // Signal the slot's shutdown flag BEFORE removing from `known`.
        // This closes the race where a concurrent connect() sees the key absent
        // but finds shutdown=false on the old slot — without this ordering it
        // would fall into the else-branch and push a duplicate slot entry.
        let mut evict_rom: Option<Arc<Mutex<Vec<u8>>>> = None;
        {
            let lock = self.slots.lock_or_recover();
            for slot in lock.iter() {
                if slot.direct_host.as_deref() == Some(key.as_str()) {
                    slot.shutdown.store(true, std::sync::atomic::Ordering::Release);
                    evict_rom = Some(slot.rom_bytes.clone());
                    break;
                }
            }
        }
        let removed = self.known.lock_or_recover().remove(&key);
        if let Some(rom_bytes) = evict_rom {
            let log_key = key.clone();
            tokio::task::spawn(async move {
                tokio::time::sleep(Duration::from_secs(20 * 60)).await;
                let mut buf = rom_bytes.lock_or_recover();
                if !buf.is_empty() {
                    *buf = Vec::new();
                    tracing::info!("ROM cache evicted for {} after 20-minute disconnect timeout", log_key);
                }
            });
        }
        removed
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
            None, // pre-configured hosts always start a new run
            None, // no authenticated user at startup
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
    run_id: Option<u32>,
    user_id: Option<u32>,
) -> bool {
    let key = format!("{}:{}", host, retroarch_port);
    if !known.lock_or_recover().insert(key.clone()) {
        return false; // already claimed
    }

    std::thread::spawn(move || {
        // Resolve the run ID for this slot (create a new one or resume an existing one).
        let slot_run_id: Option<u32> = if db.is_some() {
            match run_id {
                None => {
                    match fire_red_database::create_run_for_slot("Unknown") {
                        Ok(id) => {
                            tracing::info!("Direct mode: created run #{} for {}", id, host);
                            if let Some(uid) = user_id
                                && let Err(e) = fire_red_database::link_run_to_user(id, uid)
                            {
                                tracing::warn!("Direct mode: could not link run #{} to user {}: {}", id, uid, e);
                            }
                            Some(id)
                        }
                        Err(e) => {
                            tracing::warn!("Direct mode: could not create run for {}: {}", host, e);
                            None
                        }
                    }
                }
                Some(id) => {
                    match fire_red_database::run_exists(id) {
                        Ok(true) => {
                            tracing::info!("Direct mode: resuming run #{} for {}", id, host);
                            Some(id)
                        }
                        Ok(false) => {
                            tracing::error!("Direct mode: run #{} not found; creating new run for {}", id, host);
                            let new_id = fire_red_database::create_run_for_slot("Unknown").ok();
                            if let (Some(new_id), Some(uid)) = (new_id, user_id) {
                                let _ = fire_red_database::link_run_to_user(new_id, uid);
                            }
                            new_id
                        }
                        Err(e) => {
                            tracing::warn!("Direct mode: could not verify run #{}: {}", id, e);
                            None
                        }
                    }
                }
            }
        } else {
            None
        };

        // Resolve ROM — use the configured path or fetch from RetroArch.
        // Cache: runs/<run_id>/<host>_<port>/rom.gba (or connections/<host>_<port>/rom.gba)
        let resolved_rom = match rom_path {
            Some(ref p) => p.clone(),
            None => match crate::rom_fetch::fetch_or_load_rom(
                &host,
                retroarch_port,
                slot_run_id,
            ) {
                Ok(path) => path.to_string_lossy().into_owned(),
                Err(e) => {
                    tracing::error!(
                        "Direct mode: ROM fetch failed for {} — {}. \
                         Re-connect from the /join page to retry.",
                        host, e
                    );
                    known.lock_or_recover().remove(&key);
                    // Remove any run row we created above so it doesn't become
                    // a permanent orphan in the database.
                    if let Some(id) = slot_run_id {
                        let _ = fire_red_database::delete_run(id);
                    }
                    return;
                }
            }
        };

        // Allocate a slot — reuse a dead slot at the same index if one exists
        // so that overlay routes (/:index/party etc.) remain stable across reconnects.
        let (slot, slot_idx) = {
            let mut lock = slots.lock_or_recover();
            let reuse_idx = lock.iter().position(|s| {
                s.direct_host.as_deref() == Some(&key)
                    && s.shutdown.load(std::sync::atomic::Ordering::Acquire)
            });
            if let Some(idx) = reuse_idx {
                let s = Arc::new(MonitorSlot::new(
                    idx,
                    format!("direct:{}", host),
                    db.clone(),
                    Some(key.clone()),
                    slot_run_id,
                ));
                lock[idx] = s.clone();
                (s, idx)
            } else {
                let idx = lock.len();
                let s = Arc::new(MonitorSlot::new(
                    idx,
                    format!("direct:{}", host),
                    db.clone(),
                    Some(key.clone()),
                    slot_run_id,
                ));
                lock.push(s.clone());
                (s, idx)
            }
        };

        tracing::info!("Direct mode: slot {} → RetroArch at {}:{}", slot_idx, host, retroarch_port);

        // Pin the DbReader to the chosen run so sync_player doesn't silently
        // switch to the newest run in the database.
        if let (Some(db), Some(id)) = (slot.db.as_ref(), slot_run_id) {
            db.set_forced_run_id(id);
        }

        let loop_state = Arc::new(GameLoopState::new());

        // Load per-user OBS config for this slot's run owner (if any).
        let per_user_obs: Option<(fire_red_game_loop::config::WebhookConfig, fire_red_game_loop::config::ObsConfig)> = slot_run_id
            .and_then(fire_red_database::get_run_owner_id)
            .and_then(|uid| {
                let cfg_json = fire_red_database::get_user_integration(db.as_deref()?, uid, "obs")?;
                let obs_cfg: fire_red_game_loop::config::ObsConfig = serde_json::from_str(&cfg_json).ok()?;
                Some((fire_red_game_loop::config::WebhookConfig::default(), obs_cfg))
            });

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
            thread_run_id: slot_run_id,
            shutdown: Some(slot.shutdown.clone()),
            obs_config: per_user_obs,
        };

        fire_red_game_loop::spawn_game_loop(loop_cfg, loop_state.clone());

        // Seed the shared ROM bytes so the sprite loader and the refresh API
        // both operate on the same buffer.
        let initial_rom = match std::fs::read(&rom_path_for_sprites) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("Direct mode: could not read ROM at {}: {}", rom_path_for_sprites, e);
                Vec::new()
            }
        };
        *slot.rom_identity.lock_or_recover() = rom_identity_from_bytes(&initial_rom);
        *slot.rom_bytes.lock_or_recover() = initial_rom;

        // Sprite-loader thread: decodes species sprites from the shared ROM
        // buffer.  The buffer can be atomically replaced at runtime by
        // POST /api/slot/:index/refresh_rom without restarting this thread.
        {
            let rom_bytes      = slot.rom_bytes.clone();
            let sprite_queue   = slot.texture_request_queue.clone();
            let sprite_pending = slot.pending_textures.clone();
            let sprite_shutdown = slot.shutdown.clone();

            std::thread::spawn(move || {
                loop {
                    if sprite_shutdown.load(std::sync::atomic::Ordering::Acquire) { break; }

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
        let slot_state    = slot.state.clone();
        let slot_label    = slot.label.clone();
        let slot_box      = slot.box_data.clone();
        let slot_bag      = slot.bag_data.clone();
        let slot_cmds     = slot.command_queue.clone();
        let slot_run_chg  = slot.run_changed.clone();
        let bridge_shutdown = slot.shutdown.clone();
        let loop_br       = loop_state;

        std::thread::spawn(move || {
            let mut last_box_send = std::time::Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now);
            let mut last_bag_send = std::time::Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now);

            loop {
                if bridge_shutdown.load(std::sync::atomic::Ordering::Acquire) {
                    *slot_state.lock_or_recover() = None;
                    break;
                }

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
                if !gs.player_name.is_empty() {
                    *slot_label.lock_or_recover() = gs.player_name.clone();
                }
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
