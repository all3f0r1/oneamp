//! ReplayGain analysis of playlist files that carry no ReplayGain tags.
//!
//! Audio → ReplayGain → "Analyze untagged files" measures them in the
//! background (EBU R128, see `oneamp_core::loudness_scan`), grouped by
//! album so album gain keeps the tracks' relative levels. The figures go
//! to `replaygain.json` next to the config, never into the files, and are
//! handed to the engine with each load. A file changed since (size or
//! modification time) is treated as unanalysed.

use super::OneAmpApp;
use crossbeam_channel::{Receiver, Sender, unbounded};
use oneamp_core::MediaGain;
use oneamp_core::loudness_scan::{Measurement, album_gains, measure_file};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

/// A path paired with an album tag (a file to measure, or an album key).
type PathAlbum = (PathBuf, Option<String>);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    mtime: u64,
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let md = std::fs::metadata(path).ok()?;
        let mtime = md
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(Self {
            size: md.len(),
            mtime,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    stamp: Stamp,
    gain: MediaGain,
}

/// Figures measured by OneAmp, keyed by file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct GainCache {
    files: HashMap<PathBuf, Entry>,
}

impl GainCache {
    fn path() -> Option<PathBuf> {
        crate::config::AppConfig::config_path()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("replaygain.json")))
    }

    pub(super) fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        match serde_json::to_vec(self) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&path, bytes) {
                    eprintln!("Failed to save {}: {e}", path.display());
                }
            }
            Err(e) => eprintln!("Failed to encode ReplayGain cache: {e}"),
        }
    }

    /// Figures for `path`, if measured and the file is unchanged since.
    fn get(&self, path: &Path) -> Option<MediaGain> {
        let e = self.files.get(path)?;
        (Stamp::of(path) == Some(e.stamp)).then_some(e.gain)
    }
}

enum Msg {
    /// Files to measure (untagged, not yet analysed).
    Found(usize),
    Measured(Vec<(PathBuf, Entry)>),
    Done,
}

pub(super) struct Scan {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    todo: usize,
    done: usize,
    last_toast: std::time::Instant,
}

/// Album key: folder plus album tag; an untagged album is its own track.
fn album_key(path: &Path, album: Option<&str>) -> (PathBuf, Option<String>) {
    let album = album
        .map(|a| a.trim().to_lowercase())
        .filter(|a| !a.is_empty());
    match album {
        Some(a) => (
            path.parent().map(Path::to_path_buf).unwrap_or_default(),
            Some(a),
        ),
        None => (path.to_path_buf(), None),
    }
}

fn has_tags(path: &Path) -> bool {
    oneamp_core::TrackInfo::from_file(path)
        .is_ok_and(|i| i.gain.track_gain_db.is_some() || i.gain.album_gain_db.is_some())
}

fn worker(files: Vec<(PathBuf, Option<String>)>, tx: Sender<Msg>, cancel: Arc<AtomicBool>) {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let mut albums: BTreeMap<(PathBuf, Option<String>), Vec<PathBuf>> = BTreeMap::new();
    for (path, album) in files {
        if cancelled() {
            return;
        }
        if !has_tags(&path) {
            albums
                .entry(album_key(&path, album.as_deref()))
                .or_default()
                .push(path);
        }
    }
    let _ = tx.send(Msg::Found(albums.values().map(Vec::len).sum()));
    for paths in albums.into_values() {
        let mut measured: Vec<(PathBuf, Stamp, Measurement)> = Vec::new();
        for path in paths {
            let Some(stamp) = Stamp::of(&path) else {
                continue;
            };
            match measure_file(&path, &cancelled) {
                Ok(m) => measured.push((path, stamp, m)),
                Err(_) if cancelled() => return,
                Err(e) => eprintln!("ReplayGain analysis of {} failed: {e}", path.display()),
            }
        }
        let (files, meters): (Vec<(PathBuf, Stamp)>, Vec<Measurement>) = measured
            .into_iter()
            .map(|(path, stamp, m)| ((path, stamp), m))
            .unzip();
        let gains = album_gains(&meters);
        let batch = files
            .into_iter()
            .zip(gains)
            .map(|((path, stamp), gain)| (path, Entry { stamp, gain }))
            .collect();
        let _ = tx.send(Msg::Measured(batch));
    }
    let _ = tx.send(Msg::Done);
}

impl OneAmpApp {
    /// Figures OneAmp measured for `path` (an untagged file), if any.
    pub(super) fn analyzed_gain_for(&self, path: &Path) -> Option<MediaGain> {
        self.rg_cache.get(path)
    }

    /// Measure every playlist file that has no ReplayGain tags and no
    /// up-to-date measurement.
    pub(super) fn start_replaygain_scan(&mut self) {
        if self.rg_scan.is_some() {
            self.push_toast(
                "ReplayGain analysis already running",
                Duration::from_millis(1800),
            );
            return;
        }
        // A whole album is measured again when one of its files lacks
        // figures: album loudness needs every track.
        let mut albums: HashMap<PathAlbum, (bool, Vec<PathAlbum>)> = HashMap::new();
        for e in self.playlist.entries() {
            if e.unavailable || oneamp_core::is_stream_url(&e.path) {
                continue;
            }
            let slot = albums
                .entry(album_key(&e.path, e.album.as_deref()))
                .or_default();
            slot.0 |= self.rg_cache.get(&e.path).is_none();
            slot.1.push((e.path.clone(), e.album.clone()));
        }
        let files: Vec<PathAlbum> = albums
            .into_values()
            .filter(|(missing, _)| *missing)
            .flat_map(|(_, files)| files)
            .collect();
        if files.is_empty() {
            self.push_toast(
                "ReplayGain: nothing to analyze",
                Duration::from_millis(1800),
            );
            return;
        }
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        std::thread::spawn(move || worker(files, tx, worker_cancel));
        self.rg_scan = Some(Scan {
            rx,
            cancel,
            todo: 0,
            done: 0,
            last_toast: std::time::Instant::now(),
        });
        self.push_toast("ReplayGain: reading tags…", Duration::from_millis(1800));
    }

    /// Stop a running analysis (Escape). Files measured so far are kept.
    pub(super) fn cancel_replaygain_scan(&mut self) -> bool {
        let Some(scan) = self.rg_scan.take() else {
            return false;
        };
        scan.cancel.store(true, Ordering::Relaxed);
        for msg in scan.rx.try_iter() {
            if let Msg::Measured(batch) = msg {
                self.rg_cache.files.extend(batch);
            }
        }
        self.rg_cache.save();
        self.push_toast("ReplayGain analysis cancelled", Duration::from_millis(1800));
        true
    }

    /// Collect the worker's results. Called every frame.
    pub(super) fn poll_replaygain_scan(&mut self) {
        let Some(mut scan) = self.rg_scan.take() else {
            return;
        };
        let mut finished = false;
        while let Ok(msg) = scan.rx.try_recv() {
            match msg {
                Msg::Found(n) => scan.todo = n,
                Msg::Measured(batch) => {
                    scan.done += batch.len();
                    self.rg_cache.files.extend(batch);
                }
                Msg::Done => finished = true,
            }
        }
        if finished {
            self.rg_cache.save();
            // The prepared next track was described before its figures
            // existed: describe it again.
            self.pending_next_path = None;
            self.sync_next();
            let msg = match scan.done {
                0 => "ReplayGain: every file already has figures".to_string(),
                n => format!("ReplayGain: analyzed {n} file(s)"),
            };
            self.push_toast(msg, Duration::from_millis(2500));
            return;
        }
        if scan.todo > 0 && scan.last_toast.elapsed() >= Duration::from_secs(1) {
            scan.last_toast = std::time::Instant::now();
            self.push_toast(
                format!("ReplayGain: analyzing {}/{}…", scan.done, scan.todo),
                Duration::from_millis(1500),
            );
        }
        self.rg_scan = Some(scan);
    }
}
