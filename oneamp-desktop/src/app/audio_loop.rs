//! Audio-engine plumbing: routing playback requests, draining engine
//! events back into the app, and the gapless pre-queue heuristic.
//!
//! Kept separate from `app/mod.rs` so the audio-side wiring (event
//! enum match arms, MPRIS / notification side-effects, request-next
//! re-entrancy) can evolve without scrolling through hundreds of
//! lines of UI dispatch.

use super::{OneAmpApp, PlaybackState};
use oneamp_core::AudioCommand;
use std::path::{Path, PathBuf};

/// Heuristic for "this path is actually an HTTP URL stashed in a
/// `PathBuf`". Playlist entries built from `Open URL…` use the URL as
/// their path (so the dedup key stays unique), but for resume / file
/// I/O purposes those rows aren't real paths — they can't be seeked
/// and can't have positions persisted.
fn is_url_path(p: &Path) -> bool {
    p.to_str()
        .map(|s| s.starts_with("http://") || s.starts_with("https://"))
        .unwrap_or(false)
}

impl OneAmpApp {
    /// Send `AudioCommand::Play(path)` to the audio engine and record the
    /// path in the "recently played" list. Routing every Play through this
    /// helper keeps the recent-files menu in sync regardless of where the
    /// play was triggered (file dialog, drag-drop, IPC, playlist click,
    /// next/prev navigation, …). The list is persisted on exit and
    /// reloaded on the next launch.
    pub(super) fn play_audio_path(&mut self, path: PathBuf) {
        // Any in-flight gapless preload marker is stale the moment we
        // start a brand-new track: the audio thread will either discard
        // the preloaded decoder or finish swapping into it before
        // honoring this Play. If we leave `pending_next_path` set, the
        // upcoming `TrackLoaded` event for *this* path could spuriously
        // match it (when Play happens to target the same path the
        // preload pointed at) and double-advance `current_index` —
        // leaving the playlist's "current row" pointing one track past
        // what's actually audible.
        self.pending_next_path = None;
        // Any user-initiated Play implicitly disarms "stop after
        // current" — the user clearly wants to keep playing past this
        // track. play_audio_path funnels every Play (manual button,
        // Next/Previous hotkey, recent-file menu, drag-drop force-play,
        // IPC handoff, double-click) so resetting here is the one place
        // we need to touch.
        self.stop_after_current = false;
        // Playlist rows that came from "Add URL…" store the URL in the
        // PathBuf so the dedupe key stays unique. Route them through
        // PlayUrl instead of Play — the file-open path would just fail
        // with a "No such file" error.
        let url = path.to_str().and_then(|s| {
            if s.starts_with("http://") || s.starts_with("https://") {
                Some(s.to_string())
            } else {
                None
            }
        });
        if let Some(url) = url {
            self.recent.add_file(path);
            self.audio.send_command(AudioCommand::PlayUrl(url));
            return;
        }
        self.recent.add_file(path.clone());
        self.audio.send_command(AudioCommand::Play(path));
    }

    /// Smart Play/Pause that mirrors the Space-key semantics in
    /// `handle_keyboard`: toggle if playing or paused, start the
    /// current playlist entry if stopped. Shared between the keyboard
    /// shortcut, the macOS menu bar's `Play / Pause` item and the
    /// tray icon so they never disagree about what "Play" means now.
    pub(super) fn toggle_playback(&mut self) {
        match self.state.playback {
            PlaybackState::Playing => {
                self.audio.send_command(AudioCommand::Pause);
                self.state.playback = PlaybackState::Paused;
            }
            PlaybackState::Paused => {
                self.audio.send_command(AudioCommand::Resume);
                self.state.playback = PlaybackState::Playing;
            }
            PlaybackState::Stopped => {
                let current = self.playlist.current_entry().map(|e| e.path.clone());
                if let Some(path) = current {
                    self.play_audio_path(path);
                }
            }
        }
    }

    /// Winamp's `X` ("play") semantics: start the current entry when
    /// stopped, resume when paused, and **restart from the top** when
    /// already playing — matching the muscle memory of every classic
    /// Winamp user, for whom `X` is a hard "play this from the
    /// beginning". Distinct from `Space`/`C`, which only toggle.
    pub(super) fn transport_play(&mut self) {
        match self.state.playback {
            PlaybackState::Paused => {
                self.audio.send_command(AudioCommand::Resume);
                self.state.playback = PlaybackState::Playing;
            }
            // Playing → restart, Stopped → start: both funnel through
            // play_audio_path on the current entry, which (re)issues a
            // fresh Play from position zero.
            PlaybackState::Playing | PlaybackState::Stopped => {
                let current = self.playlist.current_entry().map(|e| e.path.clone());
                if let Some(path) = current {
                    self.play_audio_path(path);
                }
            }
        }
    }

    /// Winamp's `C` ("pause") semantics: toggle pause when playing or
    /// paused, no-op when stopped. Shares the pause/resume edges with
    /// `toggle_playback` but, unlike it, never *starts* a stopped track
    /// — pressing pause on a silent player should stay silent.
    pub(super) fn transport_pause(&mut self) {
        match self.state.playback {
            PlaybackState::Playing => {
                self.audio.send_command(AudioCommand::Pause);
                self.state.playback = PlaybackState::Paused;
            }
            PlaybackState::Paused => {
                self.audio.send_command(AudioCommand::Resume);
                self.state.playback = PlaybackState::Playing;
            }
            PlaybackState::Stopped => {}
        }
    }

    /// Stop playback now and clear any armed "stop after current".
    /// Shared by the `V` transport hotkey and the bare-`S` shortcut so
    /// they can't drift apart.
    pub(super) fn transport_stop(&mut self) {
        self.audio.send_command(AudioCommand::Stop);
        self.state.stop();
        // An explicit Stop trumps any armed "stop after current" — the
        // user wants playback ended *now*, not at the end of this track.
        self.stop_after_current = false;
    }

    /// Process audio events from engine. Returns the collected events so
    /// they can be forwarded to windows that need playback state.
    pub(super) fn process_audio_events(&mut self) -> Vec<oneamp_core::AudioEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.audio.try_recv_event() {
            // Mirror every audio-side state change onto the MPRIS bus so
            // the desktop's media widget tracks the player. This runs
            // before the local match because `RequestNext` / `Finished`
            // re-enter `play_audio_path` which would otherwise update
            // metadata before MPRIS knew the old track ended.
            if let Some(mpris) = self.mpris.as_mut() {
                mpris.sync_audio_event(&event);
            }
            match &event {
                oneamp_core::AudioEvent::Error(msg) => {
                    crate::dialog_util::show_error(msg);
                }
                oneamp_core::AudioEvent::RequestNext => {
                    let next = self.playlist.next_entry().map(|e| e.path.clone());
                    if let Some(path) = next {
                        self.play_audio_path(path);
                    }
                }
                oneamp_core::AudioEvent::RequestPrevious => {
                    let prev = self.playlist.previous_entry().map(|e| e.path.clone());
                    if let Some(path) = prev {
                        self.play_audio_path(path);
                    }
                }
                oneamp_core::AudioEvent::Finished => {
                    // A track that ran to completion has no resume slot
                    // to keep — wipe the entry so the next listen starts
                    // fresh. `current_track` was set by the most recent
                    // TrackLoaded and is still pointing at the file
                    // that just ended.
                    if let Some(track) = self.state.current_track.as_ref() {
                        self.resume.remove(&track.path);
                    }
                    // "Stop after current" intercept: when armed, the user
                    // told us to halt at the end of this track. Disarm
                    // before sending Stop so the next Play starts clean.
                    if self.stop_after_current {
                        self.stop_after_current = false;
                        self.audio.send_command(AudioCommand::Stop);
                        self.state.stop();
                        self.push_toast(
                            "Stopped at end of track",
                            std::time::Duration::from_millis(2000),
                        );
                    } else {
                        // Track ended without repeat — auto-advance through the playlist
                        let next = self.playlist.next_entry().map(|e| e.path.clone());
                        if let Some(path) = next {
                            self.play_audio_path(path);
                        }
                    }
                }
                // Spectrum + waveform are no longer routed through the
                // event channel — the audio thread publishes directly
                // into `AudioEngine`'s ArcSwap and UI consumers poll it
                // each frame. See O17 in AUDIO_OBJECTIVES.md.
                oneamp_core::AudioEvent::Position(current, total) => {
                    self.maybe_queue_next_for_gapless(*current, *total);
                    self.maybe_apply_resume(*current);
                    self.maybe_persist_resume(*current, *total);
                }
                oneamp_core::AudioEvent::TrackLoaded(track) => {
                    // If this was a gapless swap (path matches what we
                    // pre-queued), advance the playlist's current_index to
                    // match — the audio thread did the swap silently
                    // without going through RequestNext. Otherwise just
                    // clear the pending marker; user-initiated Play
                    // already manages playlist state on its own.
                    if let Some(pending) = self.pending_next_path.take()
                        && pending == track.path
                    {
                        self.playlist.next_entry();
                    }
                    // Fire a desktop toast for the newly-loaded track —
                    // opt-in via Options menu so a multi-hour playlist
                    // doesn't carpet-bomb the user's notification stack.
                    if self.config.track_notifications_enabled {
                        self.notifications.notify_track(track);
                    }
                    // Resume-on-load: only for local files (HTTP streams
                    // store their URL as a PathBuf — `to_str()` shows
                    // `http://…`), only when the user opted in, and only
                    // when the duration crosses the long-file threshold.
                    // The actual Seek is deferred until the first
                    // Position event so the audio thread is past its
                    // load latency window.
                    if self.config.resume_long_files
                        && let Some(duration) = track.duration_secs
                        && duration >= crate::resume_store::RESUME_MIN_DURATION_SECS
                        && let Some(saved) = self.resume.get(&track.path)
                        && saved >= crate::resume_store::RESUME_MIN_OFFSET_SECS
                        && saved < duration - 5.0
                        && !is_url_path(&track.path)
                    {
                        self.pending_resume = Some((track.path.clone(), saved));
                    }
                }
                oneamp_core::AudioEvent::IcyMetadata(title) => {
                    // Internet-radio "now playing" update. Rewrite the
                    // currently-playing playlist entry's title so the
                    // playlist row + title scroller surface the
                    // upstream song instead of the stream URL. The
                    // entry's path stays the URL — it's the dedupe
                    // key, not a display field.
                    if let Some(idx) = self.playlist.current_index() {
                        let entries = self.playlist.entries_mut();
                        if let Some(entry) = entries.get_mut(idx) {
                            entry.title = Some(title.clone());
                        }
                    }
                    if self.config.track_notifications_enabled {
                        self.notifications.notify_icy(title);
                    }
                }
                oneamp_core::AudioEvent::StreamReconnecting { attempt } => {
                    self.push_toast(
                        format!("Stream lost — reconnecting (try {})…", attempt),
                        std::time::Duration::from_millis(2000),
                    );
                }
                oneamp_core::AudioEvent::StreamReconnected => {
                    self.push_toast("Stream reconnected", std::time::Duration::from_millis(1800));
                }
                oneamp_core::AudioEvent::StreamReconnectFailed => {
                    self.push_toast(
                        "Stream lost — gave up",
                        std::time::Duration::from_millis(2400),
                    );
                }
                _ => {}
            }
            self.state.handle_audio_event(event.clone());
            events.push(event);
        }
        events
    }

    /// Drain the background GitHub Releases check at most once per
    /// frame. On the first `Some(info)` from the channel we compare
    /// against `config.last_notified_update_version` — same version
    /// means the user has already seen this toast on a previous
    /// launch and we silently move on. New version means: fire a
    /// notification, persist the version (saving config so subsequent
    /// launches respect the dedup), and we're done forever for this
    /// session (the checker is consumed after one read).
    ///
    /// All errors are swallowed: a failed config.save just means the
    /// user gets a duplicate toast on next launch — minor papercut,
    /// not worth surfacing.
    pub(super) fn poll_update_checker(&mut self) {
        match self.update_checker.poll() {
            Some(info) => {
                if self.config.last_notified_update_version.as_deref()
                    == Some(info.latest_version.as_str())
                {
                    // Already announced this version on a previous launch.
                    // A manual click still deserves explicit feedback —
                    // the user just asked us to check.
                    if self.manual_update_check_pending.take().is_some() {
                        self.push_toast(
                            format!("Update {} is available", info.latest_version),
                            std::time::Duration::from_millis(2400),
                        );
                    }
                    return;
                }
                self.notifications
                    .notify_update(&info.latest_version, &info.release_url);
                // A manual click also gets the in-app toast so the user
                // doesn't have to wait for the OS notification to land
                // (some desktops batch them out by several seconds).
                if self.manual_update_check_pending.take().is_some() {
                    self.push_toast(
                        format!("Update {} is available", info.latest_version),
                        std::time::Duration::from_millis(2400),
                    );
                }
                self.config.last_notified_update_version = Some(info.latest_version);
                // Route through the canonical persist path so the save mirrors
                // live state (volume / EQ / balance / …) onto self.config first.
                // A direct `self.config.save()` here would race the audio echo —
                // any slider the user nudged in the first few seconds before the
                // GitHub check returns would get clobbered with the boot-time
                // value sitting in `self.config`.
                self.flush_config();
            }
            None => {
                // Check finished without a newer version. Stay silent on
                // the startup poll (boot is noisy enough), but if the
                // user explicitly asked via the menu, give them the "all
                // clear" signal so they know the check ran.
                if self.update_checker.is_done()
                    && self.manual_update_check_pending.take().is_some()
                {
                    self.push_toast(
                        "You're on the latest version",
                        std::time::Duration::from_millis(2200),
                    );
                }
            }
        }
    }

    /// If `pending_resume` was set by the last TrackLoaded, fire the
    /// Seek now that the audio thread is producing Position events
    /// (i.e. the decoder is actually rolling). Drops the pending
    /// marker after the first attempt — we never retry, partly so a
    /// failed seek doesn't loop forever and partly because most
    /// codecs that drop the first Seek will accept the next user-
    /// driven one fine.
    pub(super) fn maybe_apply_resume(&mut self, current_pos: f32) {
        // Only act once the engine has actually started reporting
        // position (i.e. the file is decoding). The current_pos cap
        // dodges firing the seek before the codec has settled — at
        // `t≈0` the decoder is sometimes still attaching its packet
        // demuxer and a Seek lands in a blackhole.
        if current_pos < 0.05 {
            return;
        }
        let Some((path, saved)) = self.pending_resume.take() else {
            return;
        };
        let cur = self.state.current_track.as_ref().map(|t| &t.path);
        if cur != Some(&path) {
            // The user already advanced past the long file before the
            // engine got a chance to settle — abandon the resume.
            return;
        }
        self.audio.send_command(AudioCommand::Seek(saved));
        self.push_toast(
            format!(
                "Resumed at {:02}:{:02}",
                (saved as u32) / 60,
                (saved as u32) % 60
            ),
            std::time::Duration::from_millis(1800),
        );
    }

    /// Update the resume store with the current playhead position,
    /// throttled to once every `SAVE_INTERVAL_SECS`. Only writes for
    /// local files long enough to justify a resume slot; URLs and
    /// short tracks are skipped. Past `FINISH_THRESHOLD_RATIO` we
    /// remove the entry instead of updating — listening to the last
    /// 3 % of an audiobook should not pin the slot at 99 % forever.
    pub(super) fn maybe_persist_resume(&mut self, current: f32, total: f32) {
        if !self.config.resume_long_files {
            return;
        }
        if total < crate::resume_store::RESUME_MIN_DURATION_SECS {
            return;
        }
        let Some(track) = self.state.current_track.as_ref() else {
            return;
        };
        let path = track.path.clone();
        if is_url_path(&path) {
            return;
        }
        // Throttle: skip the upsert until SAVE_INTERVAL_SECS have
        // elapsed since the last write.
        let now = std::time::Instant::now();
        if let Some(last) = self.last_resume_save_at
            && now.duration_since(last).as_secs() < crate::resume_store::SAVE_INTERVAL_SECS
        {
            return;
        }
        let ratio = current / total;
        if ratio >= crate::resume_store::FINISH_THRESHOLD_RATIO {
            self.resume.remove(&path);
        } else {
            self.resume.upsert(&path, current);
        }
        self.last_resume_save_at = Some(now);
        // We don't fsync per upsert — the in-memory store is the live
        // truth; the JSON is flushed on Finished (next iter), Stop,
        // and on_exit. A crash mid-playback loses at most ~15 s of
        // progress, which is the throttle floor.
    }

    /// Send `AudioCommand::QueueNext` for the upcoming playlist track when
    /// the current one is in its last ~2 seconds. The audio thread uses
    /// the preloaded decoder to swap into the running rodio stream
    /// without rebuilding the device — that's the no-gap path. We only
    /// queue when shuffle is off; under shuffle the "next" track depends
    /// on a roll the audio thread does on its own, so guessing here
    /// would queue the wrong track. Multi-track playlists only — for a
    /// single track we'd just be re-queueing itself.
    pub(super) fn maybe_queue_next_for_gapless(&mut self, current: f32, total: f32) {
        if self.state.shuffle_enabled || total <= 0.0 {
            return;
        }
        let remaining = total - current;
        if !(0.0..2.0).contains(&remaining) {
            return;
        }
        if self.playlist.entries().len() < 2 {
            return;
        }
        let Some(idx) = self.playlist.peek_next_index() else {
            return;
        };
        let Some(entry) = self.playlist.entries().get(idx) else {
            return;
        };
        let path = entry.path.clone();
        if self.pending_next_path.as_ref() == Some(&path) {
            return;
        }
        self.audio
            .send_command(AudioCommand::QueueNext(path.clone()));
        self.pending_next_path = Some(path);
    }
}
