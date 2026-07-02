//! Playlist management module
//!
//! This module provides comprehensive playlist functionality including:
//! - Playlist creation and manipulation
//! - Shuffle mode with reproducible seeds
//! - Playlist persistence (.m3u/.pls formats)
//! - Queue system for temporary playback
//! - Sorting and searching capabilities

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::TrackInfo;

/// Represents a single entry in a playlist
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaylistEntry {
    /// Path to the audio file
    pub path: PathBuf,
    /// Track title (from metadata or filename)
    pub title: Option<String>,
    /// Artist name
    pub artist: Option<String>,
    /// Album name
    pub album: Option<String>,
    /// Duration in seconds
    pub duration: Option<f32>,
    /// Track number as it appears in the file's metadata. Parsed best-
    /// effort: ID3v2 / Vorbis often write `"3/12"` (track 3 of 12); we
    /// keep only the leading integer. Old playlists deserialize with
    /// `None` thanks to `#[serde(default)]`.
    #[serde(default)]
    pub tracknumber: Option<u32>,
    /// Release year. Parsed from ID3v2 `TDRC` / Vorbis `DATE`. We strip
    /// down to the leading 4-digit year so a `"1994-07-15"` tag still
    /// surfaces `1994`. Old playlists deserialize with `None`.
    #[serde(default)]
    pub year: Option<u32>,
    /// Genre tag — taken verbatim from metadata. Old playlists
    /// deserialize with `None`.
    #[serde(default)]
    pub genre: Option<String>,
}

impl PlaylistEntry {
    /// Create a new playlist entry from a path
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            title: None,
            artist: None,
            album: None,
            duration: None,
            tracknumber: None,
            year: None,
            genre: None,
        }
    }

    /// Create a playlist entry with metadata
    pub fn with_metadata(
        path: PathBuf,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        duration: Option<f32>,
    ) -> Self {
        Self {
            path,
            title,
            artist,
            album,
            duration,
            tracknumber: None,
            year: None,
            genre: None,
        }
    }

    /// Filename stem (without extension), used as the last-resort
    /// display value when every metadata field is missing.
    pub fn filename_stem(&self) -> String {
        self.path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string()
    }

    /// Get display name (title or filename). Kept for callers that
    /// haven't been migrated to [`format_display`] yet — the result is
    /// equivalent to `format_display("{title}")`.
    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.filename_stem())
    }

    /// Render the entry against a user-defined template.
    ///
    /// Supported placeholders:
    /// - `{artist}` / `{title}` / `{album}` / `{genre}`
    /// - `{tracknumber}` — two-digit zero-padded, e.g. `03`
    /// - `{year}` — 4-digit year
    /// - `{duration}` — `M:SS` (e.g. `4:07`)
    /// - `{filename}` — file stem without extension
    ///
    /// Missing fields become empty strings, then the result is cleaned
    /// up: runs of whitespace/separator characters that surround a now-
    /// empty token are collapsed, and dangling separators at either end
    /// are trimmed. If the formatted output ends up empty (the template
    /// referenced only missing fields), falls back to the filename stem
    /// — same safety net Winamp/foobar2000 apply, so a freshly-added
    /// untagged file never shows as a blank row.
    pub fn format_display(&self, template: &str) -> String {
        let raw = expand_template(template, self);
        let cleaned = collapse_empty_separators(&raw);
        if cleaned.is_empty() {
            self.filename_stem()
        } else {
            cleaned
        }
    }
}

/// Substitute `{token}` placeholders against `entry`. Unknown tokens
/// are left in place so the user can see the typo instead of having it
/// silently swallowed.
fn expand_template(template: &str, entry: &PlaylistEntry) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{'
            && let Some(end) = template[i + 1..].find('}')
        {
            let token = &template[i + 1..i + 1 + end];
            out.push_str(&render_token(token, entry));
            i += end + 2;
            continue;
        }
        // Push the next char (preserve UTF-8 multi-byte sequences).
        let ch = template[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn render_token(token: &str, entry: &PlaylistEntry) -> String {
    match token {
        "artist" => entry.artist.clone().unwrap_or_default(),
        "title" => entry.title.clone().unwrap_or_default(),
        "album" => entry.album.clone().unwrap_or_default(),
        "genre" => entry.genre.clone().unwrap_or_default(),
        "tracknumber" => entry
            .tracknumber
            .map(|n| format!("{:02}", n))
            .unwrap_or_default(),
        "year" => entry.year.map(|n| n.to_string()).unwrap_or_default(),
        "duration" => entry
            .duration
            .map(|d| {
                let total = d.max(0.0) as u32;
                format!("{}:{:02}", total / 60, total % 60)
            })
            .unwrap_or_default(),
        "filename" => entry.filename_stem(),
        // Unknown placeholder — echo it back so a user typo (`{titl}`)
        // is visible rather than silently swallowed. Keeps the
        // template-debug loop fast.
        other => format!("{{{}}}", other),
    }
}

/// Trim and merge runs of separator-only segments that bracket a
/// missing token. Without this, `"{artist} - {title}"` against an
/// entry with no artist renders as `" - Song"`; we want `"Song"`.
///
/// Heuristic: split on whitespace, drop empty segments, then collapse
/// segments that contain only separator characters (`-`, `–`, `—`, `/`,
/// `|`, `·`) when they sit at the start, the end, or next to another
/// separator segment.
fn collapse_empty_separators(s: &str) -> String {
    fn is_sep_only(seg: &str) -> bool {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| matches!(c, '-' | '–' | '—' | '/' | '|' | '·'))
    }
    let segments: Vec<&str> = s.split_whitespace().collect();
    let mut out: Vec<&str> = Vec::with_capacity(segments.len());
    for seg in segments {
        if is_sep_only(seg) {
            // Drop trailing separator (nothing follows) or back-to-back
            // separator (previous slot is already a separator).
            match out.last().copied() {
                None => continue,
                Some(prev) if is_sep_only(prev) => continue,
                _ => {}
            }
        }
        out.push(seg);
    }
    while out.last().is_some_and(|seg| is_sep_only(seg)) {
        out.pop();
    }
    out.join(" ")
}

/// Resolve a track path from an M3U line against the playlist file's
/// directory. Absolute paths (and anything when there's no base dir) are
/// returned verbatim; relative paths are joined onto `base_dir`. Windows
/// drive-absolute paths written with backslashes are treated as absolute.
fn resolve_track_path(raw: &str, base_dir: Option<&Path>) -> PathBuf {
    let candidate = PathBuf::from(raw);
    let is_absolute = candidate.is_absolute()
        // Catch Windows-style `C:\...` even when parsed on Unix.
        || raw.as_bytes().get(1) == Some(&b':')
        || raw.starts_with('\\')
        // Catch Unix-rooted `/abs/...` on Windows, where `is_absolute()`
        // returns false for want of a drive letter. M3U lists routinely
        // use `/`-rooted paths regardless of host OS — honour them verbatim
        // rather than anchoring to the playlist dir.
        || raw.starts_with('/');
    match base_dir {
        Some(dir) if !is_absolute => dir.join(candidate),
        _ => candidate,
    }
}

/// Express `target` relative to `base_dir` when it lives underneath it,
/// otherwise return it unchanged (absolute). Kept deliberately simple — it
/// strips a shared prefix rather than computing `../` chains, which covers
/// the common "playlist sits at the library root" layout that Winamp
/// produces. No filesystem access, so it's safe for not-yet-existing paths.
fn relativize_path(target: &Path, base_dir: Option<&Path>) -> PathBuf {
    match base_dir {
        Some(dir) => target
            .strip_prefix(dir)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| target.to_path_buf()),
        None => target.to_path_buf(),
    }
}

/// Sort order for playlist entries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// Sort by title
    Title,
    /// Sort by artist
    Artist,
    /// Sort by album
    Album,
    /// Sort by file path
    Path,
    /// Sort by duration
    Duration,
}

/// Playlist structure with advanced features
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    /// Name of the playlist
    pub name: String,
    /// List of entries
    entries: Vec<PlaylistEntry>,
    /// Current playing index
    current_index: Option<usize>,
    /// Shuffle state. Skipped from serialization — the desktop owns the
    /// authoritative shuffle flag in `AppConfig.playback.shuffle_enabled`
    /// and pushes it to the audio engine via `AudioCommand::SetShuffle`.
    /// The `Playlist`'s own shuffle bookkeeping is left in place for the
    /// `next_track`/`previous_track` API surface but no longer round-trips
    /// through `playlist.json`, which kept a stale duplicate before.
    #[serde(skip)]
    shuffle_enabled: bool,
    #[serde(skip)]
    shuffle_order: Vec<usize>,
    #[serde(skip)]
    shuffle_seed: u64,
    /// Explicit "play next" queue (Winamp's jump-to-file queue). Holds entry
    /// indices in the order the user requested. When non-empty, `next_entry`
    /// pops the FRONT and makes it current, bypassing both sequential and
    /// shuffle order. Skipped from serialization — the queue is transient
    /// session state, like selection.
    ///
    /// Identity strategy: entries are referenced by their *index* into
    /// `entries`. Because indices shift when the playlist mutates, the queue
    /// is re-mapped on every structural change (removal/sort) exactly the
    /// same way `selected` and `last_clicked` already are — entries above a
    /// removed index shift down by 1, the removed entry drops out. This keeps
    /// the queue pointing at the same logical tracks across mutation without
    /// the overhead of storing/looking up paths.
    #[serde(skip)]
    queue: Vec<usize>,
    /// Played-history stack of entry indices, most-recent last. Drives true
    /// back-navigation in shuffle mode (`previous_entry` pops this) and
    /// anti-repeat at the reshuffle/loop boundary. Capped to bound memory
    /// (see [`Playlist::HISTORY_CAP`]). Re-mapped on mutation just like the
    /// queue. Skipped from serialization — transient session state.
    #[serde(skip)]
    history: Vec<usize>,
    /// UI selection (multi-select). Skipped from serialization — selection
    /// is transient view state, not a property of the playlist.
    #[serde(skip)]
    selected: BTreeSet<usize>,
    /// Anchor for shift+click range selection. Skipped from serialization
    /// for the same reason as `selected`.
    #[serde(skip)]
    last_clicked: Option<usize>,
}

impl Default for Playlist {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl Playlist {
    /// Create a new empty playlist
    pub fn new(name: String) -> Self {
        Self {
            name,
            entries: Vec::new(),
            current_index: None,
            shuffle_enabled: false,
            shuffle_order: Vec::new(),
            shuffle_seed: 0,
            queue: Vec::new(),
            history: Vec::new(),
            selected: BTreeSet::new(),
            last_clicked: None,
        }
    }

    /// Upper bound on the played-history stack. Bounds memory on very long
    /// listening sessions while leaving plenty of room for realistic
    /// back-navigation. We never grow past `min(entries.len().max(1), CAP)`,
    /// dropping the oldest entry when full.
    const HISTORY_CAP: usize = 1000;

    /// Add a file path as a new entry, dedup'd on path. Auto-loads metadata
    /// via `TrackInfo::from_file`; on failure the entry still gets added with
    /// just the filename. Sets `current_index` to 0 when the list was empty
    /// so a fresh playlist is immediately playable.
    pub fn add_track(&mut self, path: PathBuf) {
        if self.entries.iter().any(|e| e.path == path) {
            return;
        }

        let mut entry = PlaylistEntry::new(path.clone());
        match TrackInfo::from_file(&path) {
            Ok(info) => {
                // Title falls back to the filename so an untagged file still
                // shows *something*. Keep the metadata-derived title when
                // present — the template formatter prefers `{artist} -
                // {title}` and would otherwise drop into the filename
                // fallback for half the library.
                entry.title = info.title.or_else(|| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .map(str::to_string)
                });
                entry.artist = info.artist;
                entry.album = info.album;
                entry.tracknumber = info.tracknumber;
                entry.year = info.year;
                entry.genre = info.genre;
                entry.duration = info.duration_secs;
            }
            _ => {
                entry.title = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string);
            }
        }

        self.entries.push(entry);
        if self.entries.len() == 1 {
            self.current_index = Some(0);
        }
        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Re-read tags from disk for the entry at `index` and overwrite
    /// the cached metadata. Called by the tag editor after a successful
    /// write so the playlist row picks up the user's edits without a
    /// restart. Out-of-range indices and unreadable files are no-ops.
    pub fn refresh_entry_metadata(&mut self, index: usize) {
        let Some(entry) = self.entries.get_mut(index) else {
            return;
        };
        let path = entry.path.clone();
        if let Ok(info) = TrackInfo::from_file(&path) {
            entry.title = info.title.or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            });
            entry.artist = info.artist;
            entry.album = info.album;
            entry.tracknumber = info.tracknumber;
            entry.year = info.year;
            entry.genre = info.genre;
            entry.duration = info.duration_secs;
        }
    }

    /// Remove the entry at `index`, fixing up the selection set (entries
    /// above `index` shift down by 1) and the shift-anchor. Out-of-range
    /// indices are no-ops.
    pub fn remove_track(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        self.entries.remove(index);

        if let Some(current) = self.current_index {
            if current == index {
                self.current_index = if index < self.entries.len() {
                    Some(index)
                } else if !self.entries.is_empty() {
                    Some(0)
                } else {
                    None
                };
            } else if current > index {
                self.current_index = Some(current - 1);
            }
        }

        self.selected.remove(&index);
        self.selected = self
            .selected
            .iter()
            .map(|&i| if i > index { i - 1 } else { i })
            .collect();
        if let Some(anchor) = self.last_clicked {
            self.last_clicked = match anchor {
                a if a == index => None,
                a if a > index => Some(a - 1),
                a => Some(a),
            };
        }

        self.remap_after_removal(index);

        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Move the entry at `from` to position `to`, sliding the entries in
    /// between over by one. Backs playlist drag-reorder. Every stable
    /// index reference (`current_index`, the selection set, the
    /// shift-anchor, and the queue / history stacks) is remapped so they
    /// keep pointing at the same logical tracks after the shuffle.
    /// Out-of-range indices and `from == to` are no-ops.
    pub fn move_entry(&mut self, from: usize, to: usize) {
        let len = self.entries.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);

        // Map an old index onto its post-move slot. The moved entry lands
        // on `to`; everything strictly between the old and new positions
        // slides one step toward the vacated slot.
        let remap = move |i: usize| -> usize {
            if i == from {
                to
            } else if from < to {
                if i > from && i <= to { i - 1 } else { i }
            } else if i >= to && i < from {
                i + 1
            } else {
                i
            }
        };

        self.current_index = self.current_index.map(remap);
        self.last_clicked = self.last_clicked.map(remap);
        self.selected = self.selected.iter().map(|&i| remap(i)).collect();
        for i in self.queue.iter_mut() {
            *i = remap(*i);
        }
        for i in self.history.iter_mut() {
            *i = remap(*i);
        }

        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Drop `index` from the queue and history and shift any surviving
    /// references above it down by one, keeping both stacks pointing at the
    /// same logical tracks after an entry is removed from `entries`.
    fn remap_after_removal(&mut self, index: usize) {
        self.queue.retain(|&i| i != index);
        for i in self.queue.iter_mut() {
            if *i > index {
                *i -= 1;
            }
        }
        self.history.retain(|&i| i != index);
        for i in self.history.iter_mut() {
            if *i > index {
                *i -= 1;
            }
        }
    }

    /// Push the just-played `index` onto the history stack (most-recent
    /// last), collapsing an immediate duplicate and capping the depth.
    fn record_history(&mut self, index: usize) {
        if self.history.last() == Some(&index) {
            return;
        }
        self.history.push(index);
        let cap = Self::HISTORY_CAP.min(self.entries.len().max(1));
        while self.history.len() > cap {
            self.history.remove(0);
        }
    }

    /// Mutating advance to the next track. None when the playlist is empty.
    ///
    /// Priority order:
    /// 1. **Queue** — if the "play next" queue is non-empty, pop its FRONT
    ///    and make it current, bypassing both sequential and shuffle order.
    /// 2. **Shuffle** — pick the next track from the shuffle order, biasing
    ///    away from recently-played tracks (anti-repeat) until the pool is
    ///    exhausted, at which point it reshuffles and wraps.
    /// 3. **Sequential** — linear advance with wrap-around to index 0.
    ///
    /// Whatever becomes current is recorded in the played-history stack so
    /// `previous_entry` (in shuffle) and anti-repeat both see it.
    pub fn next_entry(&mut self) -> Option<&PlaylistEntry> {
        if self.entries.is_empty() {
            return None;
        }

        // The track we're leaving belongs in history before we advance.
        if let Some(current) = self.current_index {
            self.record_history(current);
        }

        // 1. Queue takes absolute priority.
        if !self.queue.is_empty() {
            let next = self.queue.remove(0);
            self.current_index = Some(next);
            self.record_history(next);
            return self.entries.get(next);
        }

        let next = if self.shuffle_enabled {
            self.next_shuffle_index()
        } else {
            match self.current_index {
                Some(current) => (current + 1) % self.entries.len(),
                None => 0,
            }
        };

        self.current_index = Some(next);
        self.record_history(next);
        self.entries.get(next)
    }

    /// Pick the next shuffle index with anti-repeat. Walks the shuffle order
    /// forward from the current track and skips entries that appear in the
    /// recent history, so freshly-played tracks don't recur until the pool
    /// is drained. When every candidate has been played recently (pool
    /// exhausted) we reshuffle deterministically (advancing the seed) and
    /// return the new first entry — the loop/reshuffle boundary.
    fn next_shuffle_index(&mut self) -> usize {
        let len = self.entries.len();
        if self.shuffle_order.len() != len {
            self.regenerate_shuffle();
        }
        let pos = self
            .current_index
            .and_then(|c| self.shuffle_order.iter().position(|&i| i == c))
            .unwrap_or(0);

        // Scan the remainder of the shuffle order for an unplayed track.
        for step in 1..self.shuffle_order.len() {
            let cand = self.shuffle_order[(pos + step) % self.shuffle_order.len()];
            if !self.history.contains(&cand) {
                return cand;
            }
        }

        // Pool exhausted: reshuffle for a fresh pass and clear stale history
        // so the next cycle has room to avoid repeats again. Advance the
        // seed so the new order differs from the old one yet stays
        // deterministic for a given starting seed.
        self.shuffle_seed = self
            .shuffle_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.regenerate_shuffle();
        self.history.clear();
        self.shuffle_order.first().copied().unwrap_or(0)
    }

    /// Index that `next_entry` would advance to, without mutating state.
    /// Used by gapless preload to prefetch the upcoming track — it MUST
    /// mirror `next_entry`'s priority order (queue, then shuffle, then
    /// sequential), or the track gapless-preloads can diverge from the
    /// one `next_entry` actually swaps to on transition.
    pub fn peek_next_index(&self) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }

        // 1. Queue takes absolute priority, same as `next_entry`.
        if let Some(&next) = self.queue.first() {
            return Some(next);
        }

        if self.shuffle_enabled {
            return Some(self.peek_next_shuffle_index());
        }

        Some(match self.current_index {
            Some(current) => (current + 1) % self.entries.len(),
            None => 0,
        })
    }

    /// Non-mutating version of `next_shuffle_index` for `peek_next_index`.
    /// Unlike the mutating version, this never reshuffles or advances the
    /// seed — a peek must not have side effects. When the anti-repeat pool
    /// is exhausted (every candidate played recently), it reports what the
    /// reshuffle's first pick *would* currently be, without committing to
    /// it, since the real reshuffle only happens inside `next_entry`.
    fn peek_next_shuffle_index(&self) -> usize {
        let len = self.entries.len();
        if self.shuffle_order.len() != len {
            // Order is stale relative to the playlist; without mutating we
            // can't regenerate it, so fall back to the current shuffle
            // order's first unplayed entry, or index 0.
            return self
                .shuffle_order
                .iter()
                .find(|i| !self.history.contains(i))
                .copied()
                .or_else(|| self.shuffle_order.first().copied())
                .unwrap_or(0);
        }
        let pos = self
            .current_index
            .and_then(|c| self.shuffle_order.iter().position(|&i| i == c))
            .unwrap_or(0);

        for step in 1..self.shuffle_order.len() {
            let cand = self.shuffle_order[(pos + step) % self.shuffle_order.len()];
            if !self.history.contains(&cand) {
                return cand;
            }
        }

        self.shuffle_order.first().copied().unwrap_or(0)
    }

    /// Mutating retreat to the previous track.
    ///
    /// In **shuffle** mode this walks BACK through the actual played history
    /// (popping the history stack), so "previous" truly undoes the last
    /// jump instead of picking another random track — matching Winamp. When
    /// the history is exhausted it falls back to the shuffle-order neighbour.
    ///
    /// In **sequential** mode it's a plain linear retreat with wrap-around
    /// (mirror of `next_entry`).
    pub fn previous_entry(&mut self) -> Option<&PlaylistEntry> {
        if self.entries.is_empty() {
            return None;
        }

        if self.shuffle_enabled {
            // history layout: [..older, prev, current]. Drop `current`, then
            // the new tail is the track to go back to.
            if self.history.last() == self.current_index.as_ref() {
                self.history.pop();
            }
            if let Some(&prev) = self.history.last() {
                self.current_index = Some(prev);
                return self.entries.get(prev);
            }
            // No recorded history: fall back to the shuffle-order neighbour.
            let pos = self
                .current_index
                .and_then(|c| self.shuffle_order.iter().position(|&i| i == c))
                .unwrap_or(0);
            let prev = if pos == 0 {
                self.shuffle_order.len().saturating_sub(1)
            } else {
                pos - 1
            };
            let idx = self.shuffle_order.get(prev).copied().unwrap_or(0);
            self.current_index = Some(idx);
            return self.entries.get(idx);
        }

        let prev = match self.current_index {
            Some(current) => {
                if current == 0 {
                    self.entries.len() - 1
                } else {
                    current - 1
                }
            }
            None => self.entries.len() - 1,
        };
        self.current_index = Some(prev);
        self.entries.get(prev)
    }

    // ---- Play queue ("play next") ------------------------------------
    //
    // The queue is an explicit, ordered list of entries the user wants
    // played next, independent of sequential/shuffle order. Entries are
    // referenced by index and re-mapped on playlist mutation (see the
    // `queue` field docs). `next_entry` drains the FRONT of the queue.

    /// Append the entry at `index` to the play queue. Out-of-bounds indices
    /// are ignored; an already-queued entry is left untouched (no dups).
    pub fn queue_track(&mut self, index: usize) {
        if index >= self.entries.len() || self.queue.contains(&index) {
            return;
        }
        self.queue.push(index);
    }

    /// Remove the entry at `index` from the queue if present.
    pub fn unqueue_track(&mut self, index: usize) {
        self.queue.retain(|&i| i != index);
    }

    /// Queue `index` if it isn't queued, otherwise un-queue it.
    pub fn toggle_queued(&mut self, index: usize) {
        if self.is_queued(index) {
            self.unqueue_track(index);
        } else {
            self.queue_track(index);
        }
    }

    /// Empty the play queue.
    pub fn clear_queue(&mut self) {
        self.queue.clear();
    }

    /// Whether the entry at `index` is currently in the queue.
    pub fn is_queued(&self, index: usize) -> bool {
        self.queue.contains(&index)
    }

    /// 1-based position of `index` within the queue (for UI badges), or
    /// `None` if it isn't queued.
    pub fn queued_position(&self, index: usize) -> Option<usize> {
        self.queue.iter().position(|&i| i == index).map(|p| p + 1)
    }

    /// Number of entries currently queued.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Replace the selection with this single index — what a plain click
    /// does. Out-of-range indices clear the selection.
    pub fn set_selected(&mut self, index: usize) {
        self.selected.clear();
        if index < self.entries.len() {
            self.selected.insert(index);
            self.last_clicked = Some(index);
        } else {
            self.last_clicked = None;
        }
    }

    /// Ctrl+click semantics: flip whether `index` is selected. Updates the
    /// shift-anchor so future shift-click ranges originate here.
    pub fn toggle_selected(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        if !self.selected.insert(index) {
            self.selected.remove(&index);
        }
        self.last_clicked = Some(index);
    }

    /// Shift+click semantics: extend the selection from `last_clicked`
    /// (or 0 if none) to `index` inclusive. Doesn't clear existing
    /// selection — matches Winamp.
    pub fn extend_selected_to(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        let anchor = self.last_clicked.unwrap_or(0).min(self.entries.len() - 1);
        let (lo, hi) = if anchor <= index {
            (anchor, index)
        } else {
            (index, anchor)
        };
        for i in lo..=hi {
            self.selected.insert(i);
        }
    }

    /// SEL → ALL.
    pub fn select_all(&mut self) {
        self.selected.clear();
        self.selected.extend(0..self.entries.len());
    }

    /// SEL → NONE.
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// SEL → INV.
    pub fn invert_selection(&mut self) {
        let total = self.entries.len();
        let mut next = BTreeSet::new();
        for i in 0..total {
            if !self.selected.contains(&i) {
                next.insert(i);
            }
        }
        self.selected = next;
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }

    pub fn selected_indices(&self) -> &BTreeSet<usize> {
        &self.selected
    }

    /// "Primary" selected index for legacy single-select callers — returns
    /// the most recently clicked entry, falling back to the first in the
    /// set. None when the selection is empty.
    pub fn selected_index(&self) -> Option<usize> {
        self.last_clicked
            .filter(|i| self.selected.contains(i))
            .or_else(|| self.selected.iter().next().copied())
    }

    /// Set the currently playing track by index. Out-of-range indices are
    /// silently ignored.
    pub fn set_current(&mut self, index: usize) {
        if index < self.entries.len() {
            self.current_index = Some(index);
        }
    }

    /// MISC → SORT (by title, case-insensitive). The currently-playing
    /// track stays current — its index moves with the sort. Selection is
    /// reset for simplicity.
    pub fn sort_by_title(&mut self) {
        let current_path = self
            .current_index
            .and_then(|i| self.entries.get(i))
            .map(|e| e.path.clone());
        let queue_paths = self.indices_to_paths(&self.queue);
        let history_paths = self.indices_to_paths(&self.history);

        self.entries.sort_by(|a, b| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
                .then_with(|| a.path.cmp(&b.path))
        });

        if let Some(path) = current_path {
            self.current_index = self.entries.iter().position(|e| e.path == path);
        }
        self.queue = self.paths_to_indices(&queue_paths);
        self.history = self.paths_to_indices(&history_paths);
        self.selected.clear();
        self.last_clicked = None;
        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Snapshot the entry paths a list of indices currently points at, used
    /// to re-anchor the queue/history across a reorder (sort) where indices
    /// alone would no longer be meaningful.
    fn indices_to_paths(&self, indices: &[usize]) -> Vec<PathBuf> {
        indices
            .iter()
            .filter_map(|&i| self.entries.get(i).map(|e| e.path.clone()))
            .collect()
    }

    /// Re-resolve a list of paths back to indices after the entry order has
    /// changed. Paths no longer present are dropped.
    fn paths_to_indices(&self, paths: &[PathBuf]) -> Vec<usize> {
        paths
            .iter()
            .filter_map(|p| self.entries.iter().position(|e| &e.path == p))
            .collect()
    }

    /// Add an entry to the playlist
    pub fn add_entry(&mut self, entry: PlaylistEntry) {
        self.entries.push(entry);
        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Add multiple entries
    pub fn add_entries(&mut self, entries: Vec<PlaylistEntry>) {
        self.entries.extend(entries);
        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Remove entry at index
    pub fn remove_entry(&mut self, index: usize) -> Option<PlaylistEntry> {
        if index < self.entries.len() {
            let entry = self.entries.remove(index);

            // Adjust current index if needed
            if let Some(current) = self.current_index {
                if current == index {
                    self.current_index = None;
                } else if current > index {
                    self.current_index = Some(current - 1);
                }
            }

            self.remap_after_removal(index);

            if self.shuffle_enabled {
                self.regenerate_shuffle();
            }

            Some(entry)
        } else {
            None
        }
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current_index = None;
        self.shuffle_order.clear();
        self.queue.clear();
        self.history.clear();
        self.selected.clear();
        self.last_clicked = None;
    }

    /// Get entry at index
    pub fn get_entry(&self, index: usize) -> Option<&PlaylistEntry> {
        self.entries.get(index)
    }

    /// Get all entries
    pub fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }

    /// Mutable access to the entry slice. Used by ICY metadata
    /// handling so the live-radio "now playing" title overwrites the
    /// playlist row's cached title without rebuilding the playlist
    /// (and losing selection / current-index state).
    pub fn entries_mut(&mut self) -> &mut [PlaylistEntry] {
        &mut self.entries
    }

    /// Get number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if playlist is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get current index
    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Set current index
    pub fn set_current_index(&mut self, index: Option<usize>) {
        if let Some(idx) = index {
            if idx < self.entries.len() {
                self.current_index = Some(idx);
            }
        } else {
            self.current_index = None;
        }
    }

    /// Get current entry
    pub fn current_entry(&self) -> Option<&PlaylistEntry> {
        self.current_index.and_then(|idx| self.entries.get(idx))
    }

    /// Enable or disable shuffle
    pub fn set_shuffle(&mut self, enabled: bool) {
        self.shuffle_enabled = enabled;
        if enabled {
            self.regenerate_shuffle();
        }
    }

    /// Check if shuffle is enabled
    pub fn is_shuffle_enabled(&self) -> bool {
        self.shuffle_enabled
    }

    /// Set shuffle seed for reproducibility
    pub fn set_shuffle_seed(&mut self, seed: u64) {
        self.shuffle_seed = seed;
        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Regenerate shuffle order
    fn regenerate_shuffle(&mut self) {
        let len = self.entries.len();
        if len == 0 {
            self.shuffle_order.clear();
            return;
        }

        // Create indices
        let mut indices: Vec<usize> = (0..len).collect();

        // Simple Fisher-Yates shuffle with seed
        let mut seed = self.shuffle_seed;
        for i in (1..len).rev() {
            // Simple LCG for pseudo-random numbers
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let j = (seed as usize) % (i + 1);
            indices.swap(i, j);
        }

        self.shuffle_order = indices;
    }

    /// Get next track index (considering shuffle and repeat)
    pub fn next_index(&self, repeat_all: bool) -> Option<usize> {
        let current = self.current_index?;
        let len = self.entries.len();

        if len == 0 {
            return None;
        }

        if self.shuffle_enabled {
            // Find current position in shuffle order
            let shuffle_pos = self.shuffle_order.iter().position(|&idx| idx == current)?;
            let next_shuffle_pos = shuffle_pos + 1;

            if next_shuffle_pos < self.shuffle_order.len() {
                Some(self.shuffle_order[next_shuffle_pos])
            } else if repeat_all {
                // Wrap around to beginning
                self.shuffle_order.first().copied()
            } else {
                None
            }
        } else {
            // Sequential playback
            let next = current + 1;
            if next < len {
                Some(next)
            } else if repeat_all {
                Some(0)
            } else {
                None
            }
        }
    }

    /// Get previous track index (considering shuffle)
    pub fn previous_index(&self) -> Option<usize> {
        let current = self.current_index?;

        if self.shuffle_enabled {
            // Find current position in shuffle order
            let shuffle_pos = self.shuffle_order.iter().position(|&idx| idx == current)?;

            if shuffle_pos > 0 {
                Some(self.shuffle_order[shuffle_pos - 1])
            } else {
                // Wrap to end
                self.shuffle_order.last().copied()
            }
        } else {
            // Sequential playback
            if current > 0 {
                Some(current - 1)
            } else {
                // Wrap to end
                Some(self.entries.len().saturating_sub(1))
            }
        }
    }

    /// Sort playlist by specified order
    pub fn sort_by(&mut self, order: SortOrder) {
        let queue_paths = self.indices_to_paths(&self.queue);
        let history_paths = self.indices_to_paths(&self.history);
        match order {
            SortOrder::Title => {
                self.entries.sort_by(|a, b| {
                    a.display_name()
                        .to_lowercase()
                        .cmp(&b.display_name().to_lowercase())
                });
            }
            SortOrder::Artist => {
                self.entries.sort_by(|a, b| {
                    let a_artist = a.artist.as_deref().unwrap_or("").to_lowercase();
                    let b_artist = b.artist.as_deref().unwrap_or("").to_lowercase();
                    a_artist.cmp(&b_artist)
                });
            }
            SortOrder::Album => {
                self.entries.sort_by(|a, b| {
                    let a_album = a.album.as_deref().unwrap_or("").to_lowercase();
                    let b_album = b.album.as_deref().unwrap_or("").to_lowercase();
                    a_album.cmp(&b_album)
                });
            }
            SortOrder::Path => {
                self.entries.sort_by(|a, b| a.path.cmp(&b.path));
            }
            SortOrder::Duration => {
                self.entries.sort_by(|a, b| {
                    let a_dur = a.duration.unwrap_or(0.0);
                    let b_dur = b.duration.unwrap_or(0.0);
                    a_dur
                        .partial_cmp(&b_dur)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }

        self.queue = self.paths_to_indices(&queue_paths);
        self.history = self.paths_to_indices(&history_paths);

        if self.shuffle_enabled {
            self.regenerate_shuffle();
        }
    }

    /// Search entries by query (searches title, artist, album)
    pub fn search(&self, query: &str) -> Vec<usize> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.display_name().to_lowercase().contains(&query_lower)
                    || entry
                        .artist
                        .as_ref()
                        .map(|a| a.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
                    || entry
                        .album
                        .as_ref()
                        .map(|a| a.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Persist the full playlist (entries + current index + shuffle
    /// state) to a JSON file. Used by the desktop app to keep the
    /// queue across sessions. Unlike `save_m3u`, this preserves *all*
    /// metadata cached on each entry — title, artist, album, year,
    /// genre, track number, duration — so the saved file can be
    /// reopened without re-probing every track's tags.
    pub fn save_state<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize playlist state")?;
        let tmp = path.as_ref().with_extension("json.tmp");
        {
            let mut f = File::create(&tmp).context("Failed to create temp playlist state file")?;
            f.write_all(content.as_bytes())
                .context("Failed to write temp playlist state file")?;
            f.sync_all()
                .context("Failed to fsync temp playlist state file")?;
        }
        std::fs::rename(&tmp, path.as_ref())
            .context("Failed to rename temp playlist state file into place")?;
        Ok(())
    }

    /// Load a playlist previously written by [`save_state`]. Returns
    /// `Ok(None)` when the file doesn't exist (fresh install, never
    /// saved a session); returns `Err` only on corrupt content so the
    /// caller can surface the problem without confusing it with a
    /// first-launch state.
    pub fn load_state<P: AsRef<Path>>(path: P) -> Result<Option<Self>> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read playlist state from {}", path.display()))?;
        let playlist: Self = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse playlist state at {}", path.display()))?;
        Ok(Some(playlist))
    }

    /// Save playlist to .m3u file
    pub fn save_m3u<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut file = File::create(path).context("Failed to create .m3u file")?;

        writeln!(file, "#EXTM3U")?;

        for entry in &self.entries {
            if let Some(duration) = entry.duration {
                let title = entry.display_name();
                let artist = entry.artist.as_deref().unwrap_or("");
                writeln!(
                    file,
                    "#EXTINF:{},{} - {}",
                    duration.round() as i32,
                    artist,
                    title
                )?;
            }
            writeln!(file, "{}", entry.path.display())?;
        }

        Ok(())
    }

    /// Load playlist from .m3u file.
    ///
    /// Relative track paths are resolved against the playlist file's parent
    /// directory, so M3U files exported by Winamp (or any tool that writes
    /// paths relative to the playlist location) load correctly. Absolute
    /// paths are kept verbatim. `#EXTM3U` / `#EXTINF` metadata is honoured.
    pub fn load_m3u<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref()).context("Failed to open .m3u file")?;
        let reader = BufReader::new(file);

        let base_dir = path.as_ref().parent().map(Path::to_path_buf);
        let name = path
            .as_ref()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Playlist")
            .to_string();

        let mut playlist = Playlist::new(name);
        let mut current_duration: Option<f32> = None;
        let mut current_title: Option<String> = None;

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();

            if line.is_empty() || line.starts_with("#EXTM3U") {
                continue;
            }

            if let Some(extinf) = line.strip_prefix("#EXTINF:") {
                // Parse EXTINF line: #EXTINF:duration,artist - title
                let parts: Vec<&str> = extinf.splitn(2, ',').collect();
                if parts.len() == 2 {
                    if let Ok(dur) = parts[0].trim().parse::<f32>() {
                        current_duration = Some(dur);
                    }
                    current_title = Some(parts[1].trim().to_string());
                }
            } else if !line.starts_with('#') {
                // This is a file path — resolve relative paths against the
                // playlist file's directory so portable M3Us load right.
                let track_path = resolve_track_path(line, base_dir.as_deref());
                let entry = PlaylistEntry::with_metadata(
                    track_path,
                    current_title.take(),
                    None,
                    None,
                    current_duration.take(),
                );
                playlist.add_entry(entry);
            }
        }

        Ok(playlist)
    }

    /// Save playlist to a .m3u file, writing each track path *relative* to
    /// the playlist file's directory when possible. This produces portable
    /// M3Us (the Winamp "relative paths" save option) that survive being
    /// moved alongside their audio files. Tracks that can't be expressed
    /// relative to `path` (e.g. on a different drive/root) fall back to
    /// their absolute path. Mirror of [`save_m3u`] otherwise.
    pub fn save_m3u_relative<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let base_dir = path.as_ref().parent().map(Path::to_path_buf);
        let mut file = File::create(path.as_ref()).context("Failed to create .m3u file")?;

        writeln!(file, "#EXTM3U")?;

        for entry in &self.entries {
            if let Some(duration) = entry.duration {
                let title = entry.display_name();
                let artist = entry.artist.as_deref().unwrap_or("");
                writeln!(
                    file,
                    "#EXTINF:{},{} - {}",
                    duration.round() as i32,
                    artist,
                    title
                )?;
            }
            let rel = relativize_path(&entry.path, base_dir.as_deref());
            writeln!(file, "{}", rel.display())?;
        }

        Ok(())
    }

    /// Save playlist to .pls file
    pub fn save_pls<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut file = File::create(path).context("Failed to create .pls file")?;

        writeln!(file, "[playlist]")?;
        writeln!(file, "NumberOfEntries={}", self.entries.len())?;

        for (i, entry) in self.entries.iter().enumerate() {
            let num = i + 1;
            writeln!(file, "File{}={}", num, entry.path.display())?;

            if let Some(ref title) = entry.title {
                writeln!(file, "Title{}={}", num, title)?;
            }

            if let Some(duration) = entry.duration {
                writeln!(file, "Length{}={}", num, duration.round() as i32)?;
            }
        }

        writeln!(file, "Version=2")?;

        Ok(())
    }

    /// Load playlist from .pls file
    pub fn load_pls<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref()).context("Failed to open .pls file")?;
        let reader = BufReader::new(file);

        let name = path
            .as_ref()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Playlist")
            .to_string();

        let mut playlist = Playlist::new(name);
        let mut entries_map: std::collections::HashMap<usize, PlaylistEntry> =
            std::collections::HashMap::new();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();

            if line.is_empty() || line.starts_with('[') {
                continue;
            }

            if let Some(eq_pos) = line.find('=') {
                let key = &line[..eq_pos];
                let value = &line[eq_pos + 1..];

                if let Some(num_str) = key.strip_prefix("File") {
                    if let Ok(num) = num_str.parse::<usize>() {
                        let entry = entries_map
                            .entry(num)
                            .or_insert_with(|| PlaylistEntry::new(PathBuf::from(value)));
                        entry.path = PathBuf::from(value);
                    }
                } else if let Some(num_str) = key.strip_prefix("Title") {
                    if let Ok(num) = num_str.parse::<usize>() {
                        let entry = entries_map
                            .entry(num)
                            .or_insert_with(|| PlaylistEntry::new(PathBuf::new()));
                        entry.title = Some(value.to_string());
                    }
                } else if let Some(num_str) = key.strip_prefix("Length")
                    && let Ok(num) = num_str.parse::<usize>()
                    && let Ok(dur) = value.parse::<f32>()
                {
                    let entry = entries_map
                        .entry(num)
                        .or_insert_with(|| PlaylistEntry::new(PathBuf::new()));
                    entry.duration = Some(dur);
                }
            }
        }

        // Sort by index and add to playlist
        let mut indices: Vec<usize> = entries_map.keys().copied().collect();
        indices.sort();

        for idx in indices {
            if let Some(entry) = entries_map.remove(&idx) {
                playlist.add_entry(entry);
            }
        }

        Ok(playlist)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a playlist entry inline so test cases can read like fixture
    /// definitions instead of stacking constructor boilerplate.
    fn entry(path: &str, title: Option<&str>, artist: Option<&str>) -> PlaylistEntry {
        PlaylistEntry {
            path: PathBuf::from(path),
            title: title.map(String::from),
            artist: artist.map(String::from),
            album: None,
            duration: None,
            tracknumber: None,
            year: None,
            genre: None,
        }
    }

    #[test]
    fn format_display_substitutes_tokens() {
        let e = entry("/m/x.mp3", Some("Song"), Some("Band"));
        assert_eq!(e.format_display("{artist} - {title}"), "Band - Song");
    }

    #[test]
    fn format_display_collapses_missing_artist() {
        let e = entry("/m/x.mp3", Some("Song"), None);
        // No artist → leading " - " must not survive.
        assert_eq!(e.format_display("{artist} - {title}"), "Song");
    }

    #[test]
    fn format_display_collapses_missing_title() {
        let e = entry("/m/x.mp3", None, Some("Band"));
        // No title → trailing " - " must not survive.
        assert_eq!(e.format_display("{artist} - {title}"), "Band");
    }

    #[test]
    fn format_display_falls_back_to_filename_when_everything_missing() {
        let e = entry("/m/secret-track.mp3", None, None);
        assert_eq!(e.format_display("{artist} - {title}"), "secret-track");
    }

    #[test]
    fn format_display_handles_tracknumber_padding_and_duration() {
        let mut e = entry("/m/x.mp3", Some("Song"), Some("Band"));
        e.tracknumber = Some(3);
        e.duration = Some(247.5);
        assert_eq!(
            e.format_display("{tracknumber}. {artist} - {title} [{duration}]"),
            "03. Band - Song [4:07]"
        );
    }

    #[test]
    fn format_display_echoes_unknown_token() {
        let e = entry("/m/x.mp3", Some("Song"), None);
        // Typos surface as visible `{titl}` rather than disappearing —
        // saves a round trip to figure out why the template is broken.
        assert_eq!(e.format_display("{titl}"), "{titl}");
    }

    #[test]
    fn test_playlist_creation() {
        let playlist = Playlist::new("Test Playlist".to_string());
        assert_eq!(playlist.name, "Test Playlist");
        assert_eq!(playlist.len(), 0);
        assert!(playlist.is_empty());
    }

    #[test]
    fn test_add_entries() {
        let mut playlist = Playlist::new("Test".to_string());

        let entry1 = PlaylistEntry::new(PathBuf::from("/path/to/song1.mp3"));
        let entry2 = PlaylistEntry::new(PathBuf::from("/path/to/song2.mp3"));

        playlist.add_entry(entry1);
        playlist.add_entry(entry2);

        assert_eq!(playlist.len(), 2);
        assert!(!playlist.is_empty());
    }

    #[test]
    fn test_shuffle_mode() {
        let mut playlist = Playlist::new("Test".to_string());

        for i in 0..10 {
            playlist.add_entry(PlaylistEntry::new(PathBuf::from(format!("song{}.mp3", i))));
        }

        playlist.set_shuffle(true);
        assert!(playlist.is_shuffle_enabled());

        // With same seed, shuffle order should be reproducible
        playlist.set_shuffle_seed(12345);
        let order1 = playlist.shuffle_order.clone();

        playlist.set_shuffle_seed(12345);
        let order2 = playlist.shuffle_order.clone();

        assert_eq!(order1, order2);
    }

    #[test]
    fn test_next_previous() {
        let mut playlist = Playlist::new("Test".to_string());

        for i in 0..5 {
            playlist.add_entry(PlaylistEntry::new(PathBuf::from(format!("song{}.mp3", i))));
        }

        playlist.set_current_index(Some(0));

        // Test sequential next
        assert_eq!(playlist.next_index(false), Some(1));
        playlist.set_current_index(Some(1));
        assert_eq!(playlist.next_index(false), Some(2));

        // Test previous
        assert_eq!(playlist.previous_index(), Some(0));
    }

    #[test]
    fn peek_next_index_matches_next_entry_sequential() {
        let mut playlist = Playlist::new("Test".to_string());
        for i in 0..5 {
            playlist.add_entry(PlaylistEntry::new(PathBuf::from(format!("song{}.mp3", i))));
        }
        playlist.set_current_index(Some(1));

        let peeked = playlist.peek_next_index();
        playlist.next_entry();
        let advanced = playlist.current_index;
        assert_eq!(peeked, advanced);
        assert_eq!(peeked, Some(2));
    }

    #[test]
    fn peek_next_index_honors_play_next_queue() {
        let mut playlist = Playlist::new("Test".to_string());
        for i in 0..5 {
            playlist.add_entry(PlaylistEntry::new(PathBuf::from(format!("song{}.mp3", i))));
        }
        playlist.set_current_index(Some(0));
        // Queue index 4 to play next — sequential order would say 1.
        playlist.queue_track(4);

        let peeked = playlist.peek_next_index();
        assert_eq!(peeked, Some(4));

        playlist.next_entry();
        let advanced = playlist.current_index;
        assert_eq!(peeked, advanced, "gapless preload must match next_entry");
    }

    #[test]
    fn peek_next_index_matches_next_entry_shuffle() {
        let mut playlist = Playlist::new("Test".to_string());
        for i in 0..8 {
            playlist.add_entry(PlaylistEntry::new(PathBuf::from(format!("song{}.mp3", i))));
        }
        playlist.set_shuffle(true);
        playlist.set_shuffle_seed(42);
        playlist.set_current_index(Some(0));

        for _ in 0..6 {
            let peeked = playlist.peek_next_index();
            playlist.next_entry();
            let advanced = playlist.current_index;
            assert_eq!(
                peeked, advanced,
                "gapless preload must match next_entry in shuffle mode"
            );
        }
    }

    #[test]
    fn test_search() {
        let mut playlist = Playlist::new("Test".to_string());

        playlist.add_entry(PlaylistEntry::with_metadata(
            PathBuf::from("song1.mp3"),
            Some("Hello World".to_string()),
            Some("Artist A".to_string()),
            None,
            None,
        ));

        playlist.add_entry(PlaylistEntry::with_metadata(
            PathBuf::from("song2.mp3"),
            Some("Goodbye Moon".to_string()),
            Some("Artist B".to_string()),
            None,
            None,
        ));

        let results = playlist.search("hello");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], 0);

        let results = playlist.search("artist");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_sort() {
        let mut playlist = Playlist::new("Test".to_string());

        playlist.add_entry(PlaylistEntry::with_metadata(
            PathBuf::from("c.mp3"),
            Some("Charlie".to_string()),
            None,
            None,
            None,
        ));

        playlist.add_entry(PlaylistEntry::with_metadata(
            PathBuf::from("a.mp3"),
            Some("Alpha".to_string()),
            None,
            None,
            None,
        ));

        playlist.add_entry(PlaylistEntry::with_metadata(
            PathBuf::from("b.mp3"),
            Some("Bravo".to_string()),
            None,
            None,
            None,
        ));

        playlist.sort_by(SortOrder::Title);

        assert_eq!(playlist.entries[0].title, Some("Alpha".to_string()));
        assert_eq!(playlist.entries[1].title, Some("Bravo".to_string()));
        assert_eq!(playlist.entries[2].title, Some("Charlie".to_string()));
    }

    /// Build a playlist of N synthetic tracks (`0.mp3`..`{n}.mp3`).
    fn playlist_of(n: usize) -> Playlist {
        let mut p = Playlist::new("Test".to_string());
        for i in 0..n {
            p.add_entry(PlaylistEntry::new(PathBuf::from(format!("{}.mp3", i))));
        }
        p
    }

    #[test]
    fn queue_basic_dedup_and_position() {
        let mut p = playlist_of(5);
        p.queue_track(3);
        p.queue_track(1);
        p.queue_track(3); // duplicate ignored
        p.queue_track(99); // out of bounds ignored

        assert_eq!(p.queue_len(), 2);
        assert!(p.is_queued(3));
        assert!(p.is_queued(1));
        assert_eq!(p.queued_position(3), Some(1));
        assert_eq!(p.queued_position(1), Some(2));
        assert_eq!(p.queued_position(0), None);

        p.toggle_queued(3); // un-queues
        assert!(!p.is_queued(3));
        assert_eq!(p.queued_position(1), Some(1));
        p.toggle_queued(4); // queues
        assert!(p.is_queued(4));

        p.clear_queue();
        assert_eq!(p.queue_len(), 0);
    }

    #[test]
    fn queue_takes_priority_over_sequential() {
        let mut p = playlist_of(5);
        p.set_current_index(Some(0));
        p.queue_track(4);
        p.queue_track(2);

        // FRONT of the queue plays next, bypassing sequential order.
        assert_eq!(
            p.next_entry().map(|e| e.path.clone()),
            Some(PathBuf::from("4.mp3"))
        );
        assert_eq!(
            p.next_entry().map(|e| e.path.clone()),
            Some(PathBuf::from("2.mp3"))
        );
        assert_eq!(p.queue_len(), 0);
        // Queue drained — resumes sequential from the last-played index (2).
        assert_eq!(
            p.next_entry().map(|e| e.path.clone()),
            Some(PathBuf::from("3.mp3"))
        );
    }

    #[test]
    fn queue_survives_removal_remap() {
        let mut p = playlist_of(5);
        p.queue_track(3); // "3.mp3"
        p.queue_track(4); // "4.mp3"

        // Remove index 1 ("1.mp3"): queued entries above shift down by one
        // but still point at the same files.
        p.remove_track(1);
        assert!(p.is_queued(2)); // was 3 → "3.mp3"
        assert!(p.is_queued(3)); // was 4 → "4.mp3"
        assert_eq!(
            p.get_entry(2).map(|e| e.path.clone()),
            Some(PathBuf::from("3.mp3"))
        );

        // Removing a queued entry drops it from the queue.
        p.remove_track(2); // removes "3.mp3"
        assert_eq!(p.queue_len(), 1);
        assert_eq!(
            p.get_entry(p.queue[0]).map(|e| e.path.clone()),
            Some(PathBuf::from("4.mp3"))
        );
    }

    #[test]
    fn shuffle_previous_walks_back_through_history() {
        let mut p = playlist_of(8);
        p.set_shuffle_seed(42);
        p.set_shuffle(true);
        p.set_current_index(Some(0));

        // Advance a few times and capture the actual play order.
        let mut played = vec![p.current_index().unwrap()];
        for _ in 0..4 {
            assert!(p.next_entry().is_some());
            played.push(p.current_index().unwrap());
        }

        // Going back must retrace the history in reverse, not jump randomly.
        for expected in played.iter().rev().skip(1) {
            assert!(p.previous_entry().is_some());
            let got = p.current_index().unwrap();
            assert_eq!(got, *expected);
        }
    }

    #[test]
    fn shuffle_next_avoids_recent_repeats() {
        let mut p = playlist_of(6);
        p.set_shuffle_seed(7);
        p.set_shuffle(true);
        p.set_current_index(Some(0));

        // Walk one full cycle; within a single pass no track should repeat.
        let mut seen = std::collections::HashSet::new();
        seen.insert(p.current_index().unwrap());
        for _ in 0..(p.len() - 1) {
            p.next_entry();
            let idx = p.current_index().unwrap();
            assert!(
                seen.insert(idx),
                "track {} repeated within one shuffle pass",
                idx
            );
        }
    }

    #[test]
    fn load_m3u_resolves_relative_paths() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!("oneamp_m3u_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m3u = dir.join("list.m3u");
        {
            let mut f = File::create(&m3u).unwrap();
            // Mix of relative and absolute paths.
            writeln!(f, "#EXTM3U").unwrap();
            writeln!(f, "#EXTINF:120,Band - Song").unwrap();
            writeln!(f, "music/track1.mp3").unwrap();
            writeln!(f, "/abs/track2.mp3").unwrap();
        }

        let pl = Playlist::load_m3u(&m3u).unwrap();
        assert_eq!(pl.len(), 2);
        // Relative path is anchored to the playlist's directory.
        assert_eq!(pl.get_entry(0).unwrap().path, dir.join("music/track1.mp3"));
        // Absolute path is left verbatim.
        assert_eq!(
            pl.get_entry(1).unwrap().path,
            PathBuf::from("/abs/track2.mp3")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_m3u_relative_strips_base_dir() {
        use std::io::Read as _;
        let dir = std::env::temp_dir().join(format!("oneamp_m3u_save_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m3u = dir.join("list.m3u");

        let mut pl = Playlist::new("Test".to_string());
        pl.add_entry(PlaylistEntry::new(dir.join("a/song.mp3")));
        pl.add_entry(PlaylistEntry::new(PathBuf::from("/elsewhere/other.mp3")));
        pl.save_m3u_relative(&m3u).unwrap();

        let mut content = String::new();
        File::open(&m3u)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        // Under the base dir → relative; outside → absolute fallback.
        assert!(content.contains("a/song.mp3"));
        assert!(content.contains("/elsewhere/other.mp3"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
