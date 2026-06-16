//! Playlist-side concerns: M3U save, folder walk, the playlist-window
//! action dispatch, type-to-jump, and drag-drop ingest.
//!
//! Why split: the playlist surface is one of the loudest editing
//! surfaces in the app (context menu, drag-drop, hotkeys, IPC
//! handoff) and was the single biggest contributor to `app/mod.rs`
//! sprawl. Keeping all of it in one place makes "where is the path
//! that adds tracks from X?" answerable by reading a single file.

use super::{AUDIO_EXTENSIONS, OneAmpApp, PlaybackState};
use crate::windows::PlaylistAction;
use oneamp_core::{AudioCommand, PlaylistEntry};
use std::path::{Path, PathBuf};

/// Cap on recursion depth when walking a dropped or picked folder. Five
/// levels is enough for nested-by-decade collections (`/music/2010s/2014/
/// artist/album/track.mp3`) without letting a stray `~` drag wander into
/// the user's whole home directory.
const FOLDER_WALK_MAX_DEPTH: u32 = 5;

/// Write the desktop playlist as an extended M3U (`#EXTM3U`) file.
pub(super) fn save_playlist_m3u(path: &Path, entries: &[PlaylistEntry]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    writeln!(file, "#EXTM3U")?;
    for entry in entries {
        let duration_secs = entry.duration.unwrap_or(0.0).round() as i64;
        writeln!(file, "#EXTINF:{},{}", duration_secs, entry.display_name())?;
        writeln!(file, "{}", entry.path.display())?;
    }
    Ok(())
}

pub(super) fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk `dir` up to `FOLDER_WALK_MAX_DEPTH` levels deep and append every
/// audio-extension file we find to `out`. Sibling entries at each level
/// are sorted before recursion so playback order matches what the user
/// sees in their file manager. Unreadable subdirectories are skipped
/// silently — dropping a folder shouldn't fail loudly because one nested
/// folder is permission-locked.
pub(super) fn collect_audio_recursive(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > FOLDER_WALK_MAX_DEPTH {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for entry in entries {
        if entry.is_file() {
            if is_audio_path(&entry) {
                out.push(entry);
            }
        } else if entry.is_dir() {
            collect_audio_recursive(&entry, out, depth + 1);
        }
    }
}

impl OneAmpApp {
    /// Dispatch an action emitted by the playlist window
    pub(super) fn handle_playlist_action(&mut self, action: PlaylistAction) {
        match action {
            PlaylistAction::None | PlaylistAction::Close => {}
            PlaylistAction::SelectTrack(idx) => {
                self.playlist.set_selected(idx);
            }
            PlaylistAction::ToggleSelectTrack(idx) => {
                self.playlist.toggle_selected(idx);
            }
            PlaylistAction::RangeSelectTrack(idx) => {
                self.playlist.extend_selected_to(idx);
            }
            PlaylistAction::SelectAll => {
                self.playlist.select_all();
            }
            PlaylistAction::SelectNone => {
                self.playlist.clear_selection();
            }
            PlaylistAction::InvertSelection => {
                self.playlist.invert_selection();
            }
            PlaylistAction::SortByTitle => {
                self.playlist.sort_by_title();
            }
            PlaylistAction::MoveTrack { from, to } => {
                self.playlist.move_entry(from, to);
            }
            PlaylistAction::QueueTrack(idx) => {
                self.playlist.toggle_queued(idx);
            }
            PlaylistAction::PlayTrack(idx) => {
                self.playlist.set_current(idx);
                let current = self.playlist.current_entry().map(|e| e.path.clone());
                if let Some(path) = current {
                    self.play_audio_path(path);
                }
            }
            PlaylistAction::AddFiles => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Audio", AUDIO_EXTENSIONS)
                    .pick_files()
                {
                    for path in paths {
                        self.playlist.add_track(path);
                    }
                }
            }
            PlaylistAction::RemoveSelected => {
                // Remove all selected tracks. Iterate descending so each
                // removal doesn't shift later indices out from under us
                // (remove_track already adjusts the set, but doing it
                // descending avoids re-snapshotting on every iteration).
                let to_remove: Vec<usize> = self
                    .playlist
                    .selected_indices()
                    .iter()
                    .rev()
                    .copied()
                    .collect();
                for idx in to_remove {
                    self.playlist.remove_track(idx);
                }
            }
            PlaylistAction::RemoveAt(idx) => {
                self.playlist.remove_track(idx);
            }
            PlaylistAction::EditTags(idx) => {
                if let Some(entry) = self.playlist.entries().get(idx) {
                    self.tag_editor = Some(crate::tag_editor_dialog::TagEditorDialog::open(
                        idx,
                        entry.path.clone(),
                    ));
                }
            }
            PlaylistAction::AddUrl => {
                if self.url_dialog.is_none() {
                    self.url_dialog = Some(crate::url_dialog::UrlDialog::new());
                }
            }
            PlaylistAction::EditPlaylistFormat => {
                if self.format_dialog.is_none() {
                    self.format_dialog = Some(crate::format_dialog::FormatDialog::new(
                        &self.config.playlist_display_format,
                    ));
                }
            }
            PlaylistAction::Clear => {
                self.playlist.clear();
                self.audio.send_command(AudioCommand::Stop);
            }
            PlaylistAction::SaveM3u => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("M3U playlist", &["m3u"])
                    .save_file()
                    && let Err(e) = save_playlist_m3u(&path, self.playlist.entries())
                {
                    crate::dialog_util::show_error(&format!("Failed to save playlist: {}", e));
                }
            }
            PlaylistAction::LoadM3u => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("M3U playlist", &["m3u"])
                    .pick_file()
                {
                    match oneamp_core::Playlist::load_m3u(&path) {
                        Ok(loaded) => {
                            self.playlist.clear();
                            for entry in loaded.entries() {
                                self.playlist.add_track(entry.path.clone());
                            }
                        }
                        Err(e) => {
                            crate::dialog_util::show_error(&format!(
                                "Failed to load playlist: {}",
                                e
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Move the playlist selection to the next entry whose title starts
    /// with `c` (case-insensitive). Repeated presses cycle through
    /// matches. When the playlist is empty or no entry matches, the
    /// call is a no-op. The current playback index is left alone — the
    /// user is browsing, not necessarily switching tracks.
    pub(super) fn jump_in_playlist(&mut self, c: char) {
        let entries = self.playlist.entries();
        if entries.is_empty() {
            return;
        }
        let lower = c.to_ascii_lowercase();
        let n = entries.len();
        // Start point: if the user repeated the same letter, continue
        // from one past the previous hit; otherwise start at index 0.
        // Modulo `n` keeps `start` in range even if the playlist
        // shrunk under us since the last jump (we wipe the cached
        // index lazily here rather than chasing every mutation site).
        let start = if self.jump_last_char == Some(lower) {
            self.jump_last_index.map(|i| (i + 1) % n).unwrap_or(0)
        } else {
            0
        };
        let matches_letter = |entry: &oneamp_core::PlaylistEntry| {
            let name = entry.display_name();
            name.chars()
                .next()
                .map(|first| first.to_ascii_lowercase() == lower)
                .unwrap_or(false)
        };
        // Search wraps once: [start..n) then [0..start).
        let hit = (start..n)
            .chain(0..start)
            .find(|&i| matches_letter(&entries[i]));
        if let Some(idx) = hit {
            self.playlist.set_selected(idx);
            self.jump_last_char = Some(lower);
            self.jump_last_index = Some(idx);
        }
    }

    /// Add files (and audio files inside dropped folders) to the playlist
    /// and start playback if nothing is playing yet. Used by drag-drop,
    /// argv-initial files, and incoming IPC handoff alike.
    ///
    /// When the engine was idle (Stopped, or no track loaded), playback
    /// starts on the first newly-added track. When something was already
    /// playing, behaviour depends on `force_play`:
    /// - `false` (drag-drop, "Add Folder…"): append silently — Winamp parity.
    /// - `true` (argv / IPC handoff from a "Open with OneAmp" double-click):
    ///   switch to the first newly-added track and start playing it. The
    ///   user just told the OS "open this file in OneAmp"; merely appending
    ///   without playing would feel broken.
    pub(super) fn ingest_files(
        &mut self,
        paths: &[std::path::PathBuf],
        ctx: &egui::Context,
        force_play: bool,
    ) {
        let was_playing = matches!(
            self.state.playback,
            PlaybackState::Playing | PlaybackState::Paused
        );
        let pre_count = self.playlist.entries().len();
        // Tracks the (playlist index, path) of the first entry we actually
        // appended. We need the index too so we can `set_current` it before
        // playing — otherwise the playlist UI keeps highlighting the old
        // track even though a new one is audible.
        let mut first_new: Option<(usize, std::path::PathBuf)> = None;

        let mut add = |this: &mut Self, p: std::path::PathBuf| {
            let before = this.playlist.entries().len();
            this.playlist.add_track(p.clone());
            if this.playlist.entries().len() > before && first_new.is_none() {
                // `add_track` pushes to the end, so the new entry's index
                // is `before`.
                first_new = Some((before, p));
            }
        };

        for path in paths {
            if path.is_file() {
                if is_audio_path(path) {
                    add(self, path.clone());
                }
            } else if path.is_dir() {
                // Recurse so an album drop with nested `disc 1` / `disc 2`
                // subdirs gets all its tracks. Depth is capped at
                // FOLDER_WALK_MAX_DEPTH; the helper sorts each level so
                // playback order tracks the file manager.
                let mut found = Vec::new();
                collect_audio_recursive(path, &mut found, 0);
                for entry in found {
                    add(self, entry);
                }
            }
        }

        // Auto-play the first new track when the engine was idle, or when
        // the caller forced playback (OS handoff). When something was
        // already playing and the caller didn't force, we append silently
        // to match drag-drop in Winamp.
        if let Some((idx, path)) = first_new
            && (!was_playing || force_play)
        {
            self.playlist.set_current(idx);
            self.play_audio_path(path);
        }

        // Pull the window to the foreground when we received files from
        // a "Open with OneAmp" handoff or from a fresh launch — the user
        // just asked for these tracks, they should see the player.
        let added = self.playlist.entries().len() - pre_count;
        if added > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // Drag-drop is silent in Winamp, but a 50-track folder drop
            // with nothing to confirm the count feels broken. Toast it.
            // Skip when the caller forced playback AND it's a single
            // track — the song starting *is* the feedback in that case.
            let mute_toast = force_play && added == 1;
            if !mute_toast {
                let msg = if added == 1 {
                    "1 track added".to_string()
                } else {
                    format!("{} tracks added", added)
                };
                self.push_toast(msg, std::time::Duration::from_millis(1800));
            }
        }
    }
}
