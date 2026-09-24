//! Adding files, folders and playlist files (M3U, M3U8, PLS) without
//! freezing the player: a worker thread walks folders and reads tags,
//! sending entries back in batches. The toast shows progress; Escape
//! cancels.

use super::playlist_ops::{collect_audio_recursive, is_audio_path};
use super::{OneAmpApp, PlaybackState};
use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;
use oneamp_core::PlaylistEntry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

enum Msg {
    /// Number of files the walk found, for the progress readout.
    Found(usize),
    Entries(Vec<PlaylistEntry>),
    Done,
}

pub(super) struct Import {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    found: usize,
    added: usize,
    /// Play the first new entry even if something is already playing.
    force_play: bool,
    was_playing: bool,
    started_playback: bool,
    known: HashSet<PathBuf>,
    last_toast: std::time::Instant,
}

pub(super) fn is_playlist_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_lowercase().as_str(), "m3u" | "m3u8" | "pls"))
}

/// Entries listed in an M3U / M3U8 / PLS file.
pub(super) fn load_playlist_file(path: &Path) -> anyhow::Result<Vec<PlaylistEntry>> {
    let is_pls = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pls"));
    let pl = if is_pls {
        oneamp_core::Playlist::load_pls(path)?
    } else {
        oneamp_core::Playlist::load_m3u(path)?
    };
    Ok(pl.entries().to_vec())
}

fn worker(paths: Vec<PathBuf>, tx: crossbeam_channel::Sender<Msg>, cancel: Arc<AtomicBool>) {
    const BATCH: usize = 25;
    let mut files = Vec::new();
    for p in &paths {
        if p.is_dir() {
            collect_audio_recursive(p, &mut files, 0);
        } else if is_playlist_file(p) {
            match load_playlist_file(p) {
                // Playlist entries carry their own titles; tags are read
                // when a track plays.
                Ok(entries) => {
                    let _ = tx.send(Msg::Entries(entries));
                }
                Err(e) => eprintln!("Failed to load playlist {}: {}", p.display(), e),
            }
        } else if is_audio_path(p) {
            files.push(p.clone());
        }
        if cancel.load(Ordering::Relaxed) {
            return;
        }
    }
    let _ = tx.send(Msg::Found(files.len()));
    let mut batch = Vec::with_capacity(BATCH);
    for f in files {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        batch.push(PlaylistEntry::from_file(f));
        if batch.len() == BATCH {
            let _ = tx.send(Msg::Entries(std::mem::take(&mut batch)));
        }
    }
    let _ = tx.send(Msg::Entries(batch));
    let _ = tx.send(Msg::Done);
}

impl OneAmpApp {
    /// Add files, folders and playlist files in the background. The first
    /// new track starts if nothing was playing, or always with
    /// `force_play` (the user asked to play these, e.g. "Open with").
    pub(super) fn start_import(&mut self, paths: Vec<PathBuf>, force_play: bool) {
        if paths.is_empty() {
            return;
        }
        if let Some(old) = self.import.take() {
            old.cancel.store(true, Ordering::Relaxed);
        }
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        std::thread::spawn(move || worker(paths, tx, worker_cancel));
        self.import = Some(Import {
            rx,
            cancel,
            found: 0,
            added: 0,
            force_play,
            was_playing: matches!(
                self.state.playback,
                PlaybackState::Playing | PlaybackState::Paused
            ),
            started_playback: false,
            known: self
                .playlist
                .entries()
                .iter()
                .map(|e| e.path.clone())
                .collect(),
            last_toast: std::time::Instant::now(),
        });
    }

    /// Drain the worker's batches into the playlist. Called every frame.
    pub(super) fn poll_import(&mut self, ctx: &egui::Context) {
        let Some(mut imp) = self.import.take() else {
            return;
        };
        let mut done = false;
        let mut first_new: Option<usize> = None;
        while let Ok(msg) = imp.rx.try_recv() {
            match msg {
                Msg::Found(n) => imp.found += n,
                Msg::Entries(entries) => {
                    for e in entries {
                        if imp.known.insert(e.path.clone()) {
                            first_new.get_or_insert(self.playlist.len());
                            self.playlist.add_entry(e);
                            imp.added += 1;
                        }
                    }
                }
                Msg::Done => done = true,
            }
        }
        if let Some(idx) = first_new
            && !imp.started_playback
        {
            imp.started_playback = true;
            if self.playlist.current_index().is_none() {
                self.playlist.set_current_index(Some(idx));
            }
            if !imp.was_playing || imp.force_play {
                self.handle_playlist_action(crate::windows::PlaylistAction::PlayTrack(idx));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if done || imp.cancel.load(Ordering::Relaxed) {
            let msg = match imp.added {
                0 => None,
                1 if imp.force_play => None, // the song starting is the feedback
                1 => Some("1 track added".to_string()),
                n => Some(format!("{n} tracks added")),
            };
            if let Some(msg) = msg {
                self.push_toast(msg, std::time::Duration::from_millis(1800));
            }
            return;
        }
        if imp.found > 50 && imp.last_toast.elapsed().as_millis() > 250 {
            imp.last_toast = std::time::Instant::now();
            self.push_toast(
                format!("Adding {} / {} — Esc to cancel", imp.added, imp.found),
                std::time::Duration::from_millis(1000),
            );
        }
        self.import = Some(imp);
    }

    /// Stop a running import; what was added so far stays.
    pub(super) fn cancel_import(&mut self) -> bool {
        match self.import.as_ref() {
            Some(imp) => {
                imp.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}
