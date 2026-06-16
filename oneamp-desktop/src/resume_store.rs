//! Per-file playback resume state for long files (audiobooks, lectures,
//! long DJ sets, …).
//!
//! Opt-in via `AppConfig::resume_long_files`. When the engine emits a
//! `TrackLoaded` for a file we recognise *and* the file is long enough
//! (>30 min by default), the app sends `Seek(saved_secs)` once the
//! audio thread is ready, and the user picks up where they left off.
//!
//! Storage is a small JSON map at
//! `<config_dir>/oneamp/resume.json`, keyed by the file's absolute
//! path. Updated every `SAVE_INTERVAL` while playing, and again on
//! pause / stop / track change. Skipped entirely for URLs (radio
//! streams aren't seekable) and for files under
//! `RESUME_MIN_DURATION_SECS` (resuming a 3-minute pop song to 2 m
//! 14 s would be more friction than help).
//!
//! Entries past `MAX_AGE_DAYS` since last write are purged on every
//! load so a one-time foray into a 4-hour audiobook doesn't sit in
//! the JSON forever.
//!
//! Atomicity: writes go through `tmpfile + rename` so a crash mid-
//! save can never leave a half-written file. The same shape config.rs
//! uses.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Minimum track duration before we'll consider persisting / restoring
/// a position. 30 minutes catches audiobooks, podcasts, lecture
/// recordings, and long DJ sets while staying clear of regular
/// album tracks.
pub const RESUME_MIN_DURATION_SECS: f32 = 30.0 * 60.0;

/// How often (in seconds) we persist the current position while the
/// user is playing. Throttle so a 4-hour audiobook doesn't fsync 14400
/// times.
pub const SAVE_INTERVAL_SECS: u64 = 15;

/// How close to the very end counts as "finished" — past this we wipe
/// the resume entry so the next listen starts from the beginning.
pub const FINISH_THRESHOLD_RATIO: f32 = 0.97;

/// Don't try to resume a position within the first this-many seconds.
/// A user who just opened a long file probably wants the cold-start,
/// not a one-second-in resume — and the audio thread's seek timing
/// near `t=0` is finicky on some codecs.
pub const RESUME_MIN_OFFSET_SECS: f32 = 5.0;

/// Drop entries that haven't been touched in this many days. A long
/// file you listened to a year ago has almost certainly aged out —
/// resuming it would be more surprising than helpful.
pub const MAX_AGE_DAYS: u64 = 90;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeEntry {
    /// Position in seconds we should jump to on the next load.
    pub position_secs: f32,
    /// Unix-epoch seconds of the last write — used to age out stale
    /// entries.
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResumeStore {
    /// Path → entry. Strings keep the JSON readable for users who
    /// inspect the file by hand.
    entries: HashMap<String, ResumeEntry>,
}

impl ResumeStore {
    /// Load from disk, dropping entries older than `MAX_AGE_DAYS`. A
    /// missing or malformed file collapses to an empty store —
    /// resume is best-effort, never blocks playback.
    pub fn load(path: &Path) -> Self {
        let mut store: Self = fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        store.prune_stale();
        store
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create resume.json parent dir")?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).context("serialize resume store")?;
        fs::write(&tmp, json).context("write resume.json.tmp")?;
        fs::rename(&tmp, path).context("rename resume.json into place")?;
        Ok(())
    }

    /// Returns the saved position for `path`, or `None` when nothing is
    /// stored. Callers should still check the live duration against
    /// `RESUME_MIN_DURATION_SECS` before applying — a file that was
    /// long when stored but got retagged / replaced shouldn't trigger
    /// a phantom resume.
    pub fn get(&self, path: &Path) -> Option<f32> {
        self.entries.get(&Self::key(path)).map(|e| e.position_secs)
    }

    /// Update or insert. Caller decides cadence; `SAVE_INTERVAL_SECS`
    /// is the recommended floor between writes.
    pub fn upsert(&mut self, path: &Path, position_secs: f32) {
        self.entries.insert(
            Self::key(path),
            ResumeEntry {
                position_secs,
                updated_at: epoch_now(),
            },
        );
    }

    pub fn remove(&mut self, path: &Path) {
        self.entries.remove(&Self::key(path));
    }

    fn key(path: &Path) -> String {
        path.to_string_lossy().to_string()
    }

    fn prune_stale(&mut self) {
        let now = epoch_now();
        let max_age = MAX_AGE_DAYS * 86_400;
        self.entries
            .retain(|_, e| now.saturating_sub(e.updated_at) <= max_age);
    }
}

fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Default file path for the resume store, mirrored against
/// `AppConfig::config_path` so the two stores live in the same
/// per-user directory.
pub fn default_path() -> Option<PathBuf> {
    crate::config::AppConfig::config_path()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("resume.json")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a unique tmp path inside the system temp dir for tests
    /// that need a real file on disk. Cleanup runs unconditionally —
    /// a panicking test still wipes its artefacts because each test
    /// uses a different filename.
    fn tmp_resume_path(stem: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!(
            "oneamp-resume-test-{}-{}-{}.json",
            stem, pid, nanos
        ));
        p
    }

    #[test]
    fn upsert_round_trip_through_disk() {
        let path = tmp_resume_path("round_trip");
        let mut store = ResumeStore::default();
        store.upsert(Path::new("/tmp/song.mp3"), 42.5);
        store.save(&path).unwrap();
        let loaded = ResumeStore::load(&path);
        assert_eq!(loaded.get(Path::new("/tmp/song.mp3")), Some(42.5));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn prune_drops_old_entries() {
        let mut store = ResumeStore::default();
        store.entries.insert(
            "old".into(),
            ResumeEntry {
                position_secs: 1.0,
                updated_at: epoch_now() - (MAX_AGE_DAYS + 1) * 86_400,
            },
        );
        store.entries.insert(
            "fresh".into(),
            ResumeEntry {
                position_secs: 2.0,
                updated_at: epoch_now(),
            },
        );
        store.prune_stale();
        assert!(!store.entries.contains_key("old"));
        assert!(store.entries.contains_key("fresh"));
    }

    #[test]
    fn remove_clears_entry() {
        let mut store = ResumeStore::default();
        store.upsert(Path::new("/x"), 5.0);
        store.remove(Path::new("/x"));
        assert!(store.get(Path::new("/x")).is_none());
    }

    #[test]
    fn malformed_file_yields_empty_store() {
        let path = tmp_resume_path("malformed");
        fs::write(&path, "{not json at all").unwrap();
        let store = ResumeStore::load(&path);
        assert_eq!(store.entries.len(), 0);
        let _ = fs::remove_file(&path);
    }
}
