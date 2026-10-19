//! Audio-engine plumbing: routing playback requests, draining engine
//! events back into the app, and telling the engine what plays next.
//!
//! Kept separate from `app/mod.rs` so the audio-side wiring (event
//! enum match arms, MPRIS / notification side-effects, request-next
//! re-entrancy) can evolve without scrolling through hundreds of
//! lines of UI dispatch.

use super::{OneAmpApp, PlaybackState, track_path};
use oneamp_core::{AudioCommand, MediaDescriptor};
use std::path::{Path, PathBuf};
use std::time::Duration;

impl OneAmpApp {
    /// Send `AudioCommand::Play(path)` to the audio engine and record the
    /// path in the "recently played" list. Routing every Play through this
    /// helper keeps the recent-files menu in sync regardless of where the
    /// play was triggered (file dialog, drag-drop, IPC, playlist click,
    /// next/prev navigation, …). The list is persisted on exit and
    /// reloaded on the next launch.
    pub(super) fn play_audio_path(&mut self, path: PathBuf) {
        // Whatever was prepared as the next track is moot: `Load` drops
        // it on the engine side, and the marker must not match this
        // load's `TrackLoaded` and double-advance `current_index`.
        self.pending_next_path = None;
        // Any user-initiated Play implicitly disarms "stop after
        // current" — the user clearly wants to keep playing past this
        // track. play_audio_path funnels every Play (manual button,
        // Next/Previous hotkey, recent-file menu, drag-drop force-play,
        // IPC handoff, double-click) so resetting here is the one place
        // we need to touch.
        self.stop_after_current = false;
        // Playlist rows that came from "Add URL…" store the URL in the
        // PathBuf so the dedupe key stays unique; the descriptor turns
        // them into an HTTP media whose identity is that URL.
        if oneamp_core::is_stream_url(&path) {
            self.recent.add_file(path.clone());
            self.audio.send_command(AudioCommand::Load {
                media: MediaDescriptor::from_path_or_url(&path),
                paused: false,
            });
            return;
        }
        // A restored entry whose file is gone (drive unplugged): say so
        // quietly instead of a blocking error dialog.
        if crate::session::is_unavailable(&path) {
            if let Some(e) = self
                .playlist
                .entries_mut()
                .iter_mut()
                .find(|e| e.path == path)
            {
                e.unavailable = true;
            }
            self.push_toast("File not found", std::time::Duration::from_millis(2000));
            return;
        }
        self.recent.add_file(path.clone());
        // Resume slots are applied as the start position: the engine
        // reaches it before the first sample is output, so the beginning
        // is never heard (the old seek-after-load let it through).
        let mut media = MediaDescriptor::file(&path);
        if let Some(start) = self.resume_position_for(&path) {
            media = media.starting_at(Duration::from_secs_f32(start));
            self.push_toast(
                format!(
                    "Resumed at {:02}:{:02}",
                    (start as u32) / 60,
                    (start as u32) % 60
                ),
                std::time::Duration::from_millis(1800),
            );
        }
        self.audio.send_command(AudioCommand::Load {
            media,
            paused: false,
        });
    }

    /// Where `path` should start: the restored session's playhead for the
    /// track that was current at exit (once), else the long-file resume
    /// slot when the user opted in.
    fn resume_position_for(&mut self, path: &Path) -> Option<f32> {
        if let Some((p, pos)) = self.session_resume.take()
            && p == path
        {
            return Some(pos);
        }
        if !self.config.resume_long_files {
            return None;
        }
        // Slots only exist for files past the long-file threshold
        // (`maybe_persist_resume`), and are removed near the end.
        self.resume
            .get(path)
            .filter(|&saved| saved >= crate::resume_store::RESUME_MIN_OFFSET_SECS)
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

    /// The current track is the playlist's last and Repeat is off:
    /// playback ends with it.
    fn repeat_off_at_end(&self) -> bool {
        matches!(self.state.repeat_mode, oneamp_core::RepeatMode::Off) && self.playlist.at_end()
    }

    /// "Play" from the OS media bus (media keys, MPRIS, lock screen):
    /// resume when paused, start the current entry when stopped, and
    /// leave a playing track alone — unlike Winamp's `X`, a media-key
    /// Play must never restart the song.
    pub(super) fn media_play(&mut self) {
        if !matches!(self.state.playback, PlaybackState::Playing) {
            self.toggle_playback();
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

    /// Arm/disarm "stop after current". The engine gets the flag too so
    /// it can refuse gapless swaps, crossfades and repeat restarts —
    /// none of those emit `Finished`, which is where the UI side stops.
    pub(super) fn toggle_stop_after_current(&mut self) {
        self.stop_after_current = !self.stop_after_current;
        self.audio
            .send_command(AudioCommand::SetStopAfterCurrent(self.stop_after_current));
        let msg = if self.stop_after_current {
            "Will stop after current track"
        } else {
            "Stop after current — cancelled"
        };
        self.push_toast(msg, std::time::Duration::from_millis(1800));
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
                oneamp_core::AudioEvent::Error { error, .. } => {
                    // Cancellations are the engine confirming a stop or a
                    // replaced load, not something to tell the user.
                    if error.category != oneamp_core::ErrorCategory::Cancelled {
                        crate::dialog_util::show_error(&error.message);
                    }
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
                        self.resume.remove(&track_path(track));
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
                        // Track ended without repeat — auto-advance through
                        // the playlist, but don't wrap: Repeat off stops
                        // after the last track, which stays current.
                        let next = if self.repeat_off_at_end() {
                            None
                        } else {
                            self.playlist.next_entry().map(|e| e.path.clone())
                        };
                        if let Some(path) = next {
                            self.play_audio_path(path);
                        } else {
                            // End of playlist: route through a real Stop so
                            // the engine's `Stopped` reaches every window,
                            // not just the ones that also read `Finished`.
                            self.audio.send_command(AudioCommand::Stop);
                            self.state.stop();
                        }
                    }
                }
                // Spectrum + waveform are no longer routed through the
                // event channel — the audio thread publishes directly
                // into `AudioEngine`'s ArcSwap and UI consumers poll it
                // each frame. See O17 in AUDIO_OBJECTIVES.md.
                oneamp_core::AudioEvent::Position { position, duration } => {
                    self.sync_next();
                    let total = duration.map_or(0.0, |d| d.as_secs_f32());
                    self.maybe_persist_resume(position.as_secs_f32(), total);
                }
                oneamp_core::AudioEvent::TrackLoaded { track, .. } => {
                    let path = track_path(track);
                    // A gapless/crossfade transition to the track we told
                    // the engine comes next: advance the playlist to match
                    // (the engine switched without going through
                    // RequestNext). Otherwise just clear the marker.
                    if let Some(pending) = self.pending_next_path.take()
                        && pending == path
                    {
                        self.playlist.next_entry();
                    }
                    // Fire a desktop toast for the newly-loaded track —
                    // opt-in via Options menu so a multi-hour playlist
                    // doesn't carpet-bomb the user's notification stack.
                    if self.config.track_notifications_enabled {
                        self.notifications.notify_track(track);
                    }
                    // EQ AUTO is resolved by the engine from the media id.
                    // A file that loads is available again.
                    if let Some(e) = self
                        .playlist
                        .entries_mut()
                        .iter_mut()
                        .find(|e| e.path == path)
                    {
                        e.unavailable = false;
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
        let path = track_path(track);
        if oneamp_core::is_stream_url(&path) {
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
        // Write through on every throttled tick so a crash loses at most
        // SAVE_INTERVAL_SECS of progress (the file is a few hundred bytes).
        if let Some(path) = crate::resume_store::default_path()
            && let Err(e) = self.resume.save(&path)
        {
            eprintln!("Failed to save resume store: {}", e);
        }
    }

    /// Keep the engine's "next media" in step with the playlist. The
    /// engine prepares it ahead of time and joins it gaplessly (or
    /// crossfades) on its own; this only says *what* comes next. No next
    /// under shuffle (the playlist rolls at advance time), for a
    /// one-track playlist, or at the end with Repeat off — the engine
    /// then finishes and `Finished` decides.
    pub(super) fn sync_next(&mut self) {
        let wanted = if self.state.shuffle_enabled
            || self.playlist.entries().len() < 2
            || self.repeat_off_at_end()
        {
            None
        } else {
            self.playlist
                .peek_next_index()
                .and_then(|i| self.playlist.entries().get(i))
                .filter(|e| !e.unavailable)
                .map(|e| e.path.clone())
        };
        if wanted == self.pending_next_path {
            return;
        }
        self.audio.send_command(AudioCommand::SetNext(
            wanted.as_deref().map(MediaDescriptor::from_path_or_url),
        ));
        self.pending_next_path = wanted;
    }
}
