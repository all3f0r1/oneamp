//! Config persistence and live-state ↔ on-disk drift detection.
//!
//! `flush_config` snapshots the live mutable state (volume, EQ, layout,
//! …) onto `self.config` and writes it out atomically.
//! `check_persistable_drift` runs every frame, compares the live state
//! against the last-saved config, and marks the config dirty when they
//! diverge — so the per-frame debounce in `update` can fire a save
//! without every mutation site having to remember to call `mark_dirty`.
//!
//! Why split out of `app/mod.rs`: this is a tight, self-contained
//! concern that touches the persistable subset of `OneAmpApp`'s
//! state. Keeping it in one file makes it easy to audit "is field X
//! persisted?" without scrolling through audio / playlist / UI code.

use super::{OneAmpApp, visualizer_to_config};

/// Backoff before retrying a failed config write.
const CONFIG_SAVE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

impl OneAmpApp {
    /// Mark the live config as having unsaved changes. The next update
    /// tick that finds `config_dirty_since` older than `CONFIG_SAVE_DEBOUNCE`
    /// will flush to disk via `flush_config()`. Cheap to call on every
    /// mutation; the debounce keeps slider drags off the file system.
    pub(super) fn mark_dirty(&mut self) {
        self.config_dirty_since = Some(std::time::Instant::now());
    }

    /// Snapshot the live mutable state (volume, EQ, balance, layout, …)
    /// onto `self.config` and write it out atomically. Called either from
    /// the debounce timer in `update` or from `on_exit` as the final
    /// flush. All errors are surfaced through stderr but never block the
    /// app — a write failure means the user starts the next session with
    /// slightly stale settings, not a corrupt file (the atomic rename in
    /// `save` rules out a half-written config).
    pub(super) fn flush_config(&mut self) {
        // Pull the EQ window's current_preset (the source of truth —
        // it gets set when the user clicks a preset row, cleared on
        // manual band drag) back into state before flushing.
        self.state.equalizer.current_preset =
            self.windows.equalizer_current_preset().map(String::from);

        self.config = self.live_config();

        match self.config.save() {
            Ok(()) => {
                self.config_dirty_since = None;
                self.config_save_failed = false;
            }
            Err(e) => {
                // `self.config` already mirrors live state, so drift
                // detection would never re-flag it — keep the dirty mark
                // and push it into the future to retry with a backoff.
                eprintln!("Failed to save config: {}", e);
                self.config_dirty_since = Some(std::time::Instant::now() + CONFIG_SAVE_RETRY_DELAY);
                if !self.config_save_failed {
                    self.config_save_failed = true;
                    self.push_toast(
                        "Settings could not be saved — retrying",
                        std::time::Duration::from_millis(3000),
                    );
                }
            }
        }
    }

    /// `self.config` with every persistable live field mirrored in. The
    /// single list of those fields: `flush_config` saves this snapshot and
    /// `check_persistable_drift` compares it, so a new setting can't be
    /// saved without being drift-checked (or the reverse). Anything not
    /// set here (audio_effects, gapless, …) keeps whatever the previous
    /// load produced.
    fn live_config(&mut self) -> crate::config::AppConfig {
        let mut c = self.config.clone();
        c.equalizer.enabled = self.state.equalizer.enabled;
        // The global curve, not an auto-loaded preset that's playing.
        let (gains, preamp_db) = self.global_eq();
        c.equalizer.gains = gains;
        c.equalizer.preamp_db = preamp_db;
        // The EQ window owns the preset name: set on a preset click,
        // cleared on a manual band drag.
        c.equalizer.current_preset = self.windows.equalizer_current_preset().map(String::from);
        c.playback.volume = self.state.volume.level;
        c.playback.muted = self.state.volume.muted;
        c.playback.balance = self.state.volume.balance;
        c.playback.repeat_mode = self.state.repeat_mode.into();
        c.playback.shuffle_enabled = self.state.shuffle_enabled;
        c.first_run = false;
        c.always_on_top = self.always_on_top;
        c.recent_files = self.recent.clone();
        c.user_scale = self.user_scale;
        c.shade_mode = self.windows.is_shade_mode();
        c.show_equalizer = self.windows.is_equalizer_visible();
        c.show_playlist = self.windows.is_playlist_visible();
        c.windows = self.live_window_layout();
        c.visualizer_mode = visualizer_to_config(self.windows.main_window_mut().visualizer_mode());
        c.show_remaining = self.windows.main_window_mut().show_remaining();
        c
    }

    /// Snapshot of the coordinator's window layout in config form.
    fn live_window_layout(&self) -> crate::config::WindowLayoutConfig {
        let [equalizer_offset, playlist_offset] = self.windows.subwindow_offsets();
        crate::config::WindowLayoutConfig {
            detached: self.windows.is_detached(),
            equalizer_offset,
            playlist_offset,
            playlist_height: Some(self.windows.playlist_height()),
        }
    }

    /// Compare every persistable live-state field against what's stored in
    /// `self.config`. Any divergence marks the config dirty so the
    /// debounced flush in `update` picks it up later. Avoids the
    /// alternative of sprinkling `mark_dirty()` calls across dozens of
    /// mutation sites — and catches changes pushed by external sources
    /// (MPRIS, multimedia keys, drag-drop) for free.
    pub(super) fn check_persistable_drift(&mut self) {
        let changed = self.live_config() != self.config;
        // Only start the debounce clock on the first divergence. Calling
        // `mark_dirty()` every frame would keep pushing the deadline
        // forward and the flush in `update` would never fire.
        if changed && self.config_dirty_since.is_none() {
            self.mark_dirty();
        }
    }
}
