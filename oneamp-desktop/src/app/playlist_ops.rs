//! Playlist-side concerns: M3U save, folder walk, the playlist-window
//! action dispatch, type-to-jump, and drag-drop ingest.
//!
//! Why split: the playlist surface is one of the loudest editing
//! surfaces in the app (context menu, drag-drop, hotkeys, IPC
//! handoff) and was the single biggest contributor to `app/mod.rs`
//! sprawl. Keeping all of it in one place makes "where is the path
//! that adds tracks from X?" answerable by reading a single file.

use super::{AUDIO_EXTENSIONS, OneAmpApp};
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
                self.remember_for_undo();
                // `sort_by_title()` now just delegates to
                // `Playlist::sort_by(SortOrder::Title)`, which is the
                // canonical sort: current track, queue, history,
                // selection, and shift-click anchor all survive the
                // reorder (by path). `OneAmpApp` can't be constructed in
                // a unit test without an egui/eframe context, so that
                // invariant is covered at the `Playlist` level instead —
                // see `sort_by_preserves_current_queue_history_selection_and_anchor`
                // in `oneamp-core/src/playlist.rs`.
                self.playlist.sort_by_title();
            }
            PlaylistAction::MoveTrack { from, to } => {
                self.remember_for_undo();
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
                    self.start_import(paths, false);
                }
            }
            PlaylistAction::AddDir => self.add_folder(),
            PlaylistAction::OpenFile => self.open_files_replace(),
            PlaylistAction::TransportPlay => self.transport_play(),
            PlaylistAction::TransportPause => self.transport_pause(),
            PlaylistAction::Crop => {
                let keep = self.playlist.selected_indices().clone();
                self.remove_where(|i, _| !keep.contains(&i));
            }
            PlaylistAction::RemoveDead => {
                let removed = self.remove_where(|_, e| crate::session::is_unavailable(&e.path));
                self.push_toast(
                    format!("Removed {removed} dead file(s)"),
                    std::time::Duration::from_millis(1500),
                );
            }
            PlaylistAction::RemoveSelected => {
                self.remember_for_undo();
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
                self.remember_for_undo();
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
                self.remember_for_undo();
                self.playlist.clear();
                self.audio.send_command(AudioCommand::Stop);
            }
            PlaylistAction::SaveM3u => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("M3U8 playlist", &["m3u8"])
                    .add_filter("M3U playlist", &["m3u"])
                    .add_filter("PLS playlist", &["pls"])
                    .save_file()
                {
                    let is_pls = path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("pls"));
                    let result = if is_pls {
                        self.playlist.save_pls(&path)
                    } else {
                        save_playlist_m3u(&path, self.playlist.entries())
                    };
                    if let Err(e) = result {
                        crate::dialog_util::show_error(&format!("Failed to save playlist: {}", e));
                    }
                }
            }
            PlaylistAction::LoadM3u => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Playlist", &["m3u", "m3u8", "pls"])
                    .pick_file()
                {
                    // Load = replace, like Winamp; Ctrl+Z brings the old
                    // list back.
                    self.remember_for_undo();
                    self.playlist.clear();
                    self.start_import(vec![path], false);
                }
            }
        }
    }

    /// Remove every entry matching `pred` (undoable). Returns how many.
    fn remove_where(&mut self, pred: impl Fn(usize, &PlaylistEntry) -> bool) -> usize {
        let doomed: Vec<usize> = (0..self.playlist.entries().len())
            .rev()
            .filter(|&i| pred(i, &self.playlist.entries()[i]))
            .collect();
        if !doomed.is_empty() {
            self.remember_for_undo();
            for &idx in &doomed {
                self.playlist.remove_track(idx);
            }
        }
        doomed.len()
    }

    /// Winamp's "Play file": the picked files replace the playlist and
    /// start playing. Ctrl+Z restores the list.
    pub(super) fn open_files_replace(&mut self) {
        let exts: Vec<&str> = AUDIO_EXTENSIONS
            .iter()
            .copied()
            .chain(["m3u", "m3u8", "pls"])
            .collect();
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Audio and playlists", &exts)
            .pick_files()
        {
            self.remember_for_undo();
            self.playlist.clear();
            self.start_import(paths, true);
        }
    }

    /// Winamp's "Add folder": append a picked folder's audio files. Reuses
    /// the drag-drop ingest path (dedupe, start playback when idle);
    /// `force_play = false` so a folder added mid-playback doesn't
    /// interrupt the song.
    pub(super) fn add_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            self.start_import(vec![folder], false);
        }
    }

    /// Snapshot the playlist before a destructive edit (remove, clear,
    /// reorder, replace) so Ctrl+Z can bring it back.
    pub(super) fn remember_for_undo(&mut self) {
        const UNDO_DEPTH: usize = 20;
        if self.undo.len() == UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push(self.playlist.clone());
    }

    /// Restore the playlist as it was before the last destructive edit.
    /// Playback isn't touched.
    pub(super) fn undo_playlist(&mut self) {
        match self.undo.pop() {
            Some(prev) => {
                self.playlist = prev;
                self.push_toast("Undone", std::time::Duration::from_millis(1200));
            }
            None => self.push_toast("Nothing to undo", std::time::Duration::from_millis(1200)),
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
}
