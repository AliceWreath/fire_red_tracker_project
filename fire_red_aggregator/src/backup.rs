//! Scheduled full-database JSON backups.
//!
//! The game-clear auto-backup (`BroadcastLoop::tick`) only fires when a run is
//! finished, so a mid-run database loss loses everything since the run began.
//! [`spawn_scheduled`] runs a background thread that every `interval_hours`
//! writes a snapshot of every run to `backup_dir/db_backup_<unix_ts>.json`
//! (via `fire_red_database::export_all_runs`) and prunes old snapshots so at
//! most `keep` files remain.
//!
//! Enabled by setting `backup_interval_hours` (and `backup_dir`) in the
//! aggregator config; `POST /api/backup` triggers the same snapshot on demand.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Filename prefix for scheduled snapshots. Distinct from the game-clear
/// backups (`run_<id>_<ts>.json`) so pruning never touches those.
const SNAPSHOT_PREFIX: &str = "db_backup_";

/// Default number of snapshot files retained when `backup_keep` is unset.
pub const DEFAULT_KEEP: usize = 10;

/// Writes one snapshot of every run to `dir` and prunes old snapshots down to
/// `keep` files. Returns the path of the file written.
pub fn write_snapshot(conn: &str, dir: &str, keep: usize) -> Result<PathBuf, String> {
    let json = fire_red_database::export_all_runs(conn)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create backup_dir: {e}"))?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = Path::new(dir).join(format!("{SNAPSHOT_PREFIX}{ts}.json"));
    std::fs::write(&path, json.to_string()).map_err(|e| format!("write failed: {e}"))?;
    let pruned = prune(Path::new(dir), keep);
    if pruned > 0 {
        tracing::info!("scheduled backup: pruned {pruned} old snapshot(s)");
    }
    Ok(path)
}

/// Deletes the oldest `db_backup_*.json` files in `dir` so at most `keep`
/// remain. Files are ordered by the timestamp embedded in the name. Returns
/// the number of files deleted. Game-clear backups (`run_*.json`) and any
/// other files are never touched.
fn prune(dir: &Path, keep: usize) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut snapshots: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let ts: u64 = name
                .strip_prefix(SNAPSHOT_PREFIX)?
                .strip_suffix(".json")?
                .parse()
                .ok()?;
            Some((ts, e.path()))
        })
        .collect();
    if snapshots.len() <= keep {
        return 0;
    }
    snapshots.sort_by_key(|(ts, _)| *ts);
    let excess = snapshots.len() - keep;
    let mut removed = 0;
    for (_, path) in snapshots.into_iter().take(excess) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!("scheduled backup: could not prune {}: {e}", path.display()),
        }
    }
    removed
}

/// Spawns the scheduled-backup thread. The first snapshot is written one
/// interval after startup, not immediately, so a crash-looping process does
/// not churn out files. Returns a stop flag; store `false` to end the thread
/// after its current sleep slice (checked every 30 s).
pub fn spawn_scheduled(
    conn: String,
    dir: String,
    interval_hours: u32,
    keep: usize,
) -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    std::thread::spawn(move || {
        let interval = Duration::from_secs(u64::from(interval_hours) * 3600);
        tracing::info!(
            "scheduled backup: every {interval_hours}h to {dir}, keeping {keep} snapshots"
        );
        loop {
            // Sleep in short slices so config reloads/shutdown are honoured
            // without waiting out the full interval.
            let mut slept = Duration::ZERO;
            while slept < interval {
                if !flag.load(Ordering::Acquire) {
                    return;
                }
                let slice = Duration::from_secs(30).min(interval - slept);
                std::thread::sleep(slice);
                slept += slice;
            }
            if !flag.load(Ordering::Acquire) {
                return;
            }
            match write_snapshot(&conn, &dir, keep) {
                Ok(path) => tracing::info!("scheduled backup: wrote {}", path.display()),
                Err(e) => tracing::warn!("scheduled backup: {e}"),
            }
        }
    });
    running
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "frt_backup_test_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "{}").unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn prune_removes_oldest_snapshots_beyond_keep() {
        let dir = temp_dir("oldest");
        for ts in [100u64, 200, 300, 400, 500] {
            touch(&dir, &format!("db_backup_{ts}.json"));
        }
        assert_eq!(prune(&dir, 2), 3);
        assert_eq!(names(&dir), vec!["db_backup_400.json", "db_backup_500.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_is_noop_at_or_below_keep() {
        let dir = temp_dir("noop");
        touch(&dir, "db_backup_100.json");
        touch(&dir, "db_backup_200.json");
        assert_eq!(prune(&dir, 2), 0);
        assert_eq!(prune(&dir, 5), 0);
        assert_eq!(names(&dir).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_ignores_game_clear_backups_and_other_files() {
        let dir = temp_dir("ignore");
        touch(&dir, "run_3_100.json"); // game-clear backup
        touch(&dir, "notes.txt");
        touch(&dir, "db_backup_bogus.json"); // non-numeric ts — not ours
        for ts in [100u64, 200, 300] {
            touch(&dir, &format!("db_backup_{ts}.json"));
        }
        assert_eq!(prune(&dir, 1), 2);
        let remaining = names(&dir);
        assert!(remaining.contains(&"run_3_100.json".to_string()));
        assert!(remaining.contains(&"notes.txt".to_string()));
        assert!(remaining.contains(&"db_backup_bogus.json".to_string()));
        assert!(remaining.contains(&"db_backup_300.json".to_string()));
        assert!(!remaining.contains(&"db_backup_100.json".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_missing_dir_returns_zero() {
        assert_eq!(prune(Path::new("/nonexistent/frt_backup_test"), 3), 0);
    }

    #[test]
    fn prune_orders_by_timestamp_not_name_length() {
        let dir = temp_dir("order");
        // 900 < 1000 numerically but "900" > "1000" lexically.
        touch(&dir, "db_backup_900.json");
        touch(&dir, "db_backup_1000.json");
        assert_eq!(prune(&dir, 1), 1);
        assert_eq!(names(&dir), vec!["db_backup_1000.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
