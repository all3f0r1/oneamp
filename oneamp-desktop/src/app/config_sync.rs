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

        // Mirror every persistable field from live state back onto the
        // config struct. Anything not mutated here (audio_effects,
        // gapless, …) keeps whatever the previous load produced so
        // they survive the round-trip untouched.
        self.config.equalizer.enabled = self.state.equalizer.enabled;
        self.config.equalizer.gains = self.state.equalizer.gains.clone();
        self.config.equalizer.preamp_db = self.state.equalizer.preamp_db;
        self.config.equalizer.current_preset = self.state.equalizer.current_preset.clone();
        self.config.playback.volume = self.state.volume.level;
        self.config.playback.muted = self.state.volume.muted;
        self.config.playback.balance = self.state.volume.balance;
        self.config.playback.repeat_mode = self.state.repeat_mode.into();
        self.config.playback.shuffle_enabled = self.state.shuffle_enabled;
        self.config.first_run = false;
        self.config.always_on_top = self.always_on_top;
        self.config.recent_files = self.recent.clone();
        self.config.user_scale = self.user_scale;
        self.config.shade_mode = self.windows.is_shade_mode();
        self.config.show_equalizer = self.windows.is_equalizer_visible();
        self.config.show_playlist = self.windows.is_playlist_visible();
        self.config.visualizer_mode =
            visualizer_to_config(self.windows.main_window_mut().visualizer_mode());
        self.config.show_remaining = self.windows.main_window_mut().show_remaining();

        if let Err(e) = self.config.save() {
            eprintln!("Failed to save config: {}", e);
        }
        self.config_dirty_since = None;
    }

    /// Compare every persistable live-state field against what's stored in
    /// `self.config`. Any divergence marks the config dirty so the
    /// debounced flush in `update` picks it up later. Avoids the
    /// alternative of sprinkling `mark_dirty()` calls across dozens of
    /// mutation sites — and catches changes pushed by external sources
    /// (MPRIS, multimedia keys, drag-drop) for free.
    pub(super) fn check_persistable_drift(&mut self) {
        let mut changed = false;
        if (self.config.playback.volume - self.state.volume.level).abs() > f32::EPSILON {
            changed = true;
        }
        if self.config.playback.muted != self.state.volume.muted {
            changed = true;
        }
        if (self.config.playback.balance - self.state.volume.balance).abs() > f32::EPSILON {
            changed = true;
        }
        let live_repeat: crate::config::RepeatModeConfig = self.state.repeat_mode.into();
        if self.config.playback.repeat_mode != live_repeat {
            changed = true;
        }
        if self.config.playback.shuffle_enabled != self.state.shuffle_enabled {
            changed = true;
        }
        if self.config.equalizer.enabled != self.state.equalizer.enabled {
            changed = true;
        }
        if (self.config.equalizer.preamp_db - self.state.equalizer.preamp_db).abs() > f32::EPSILON {
            changed = true;
        }
        if self.config.equalizer.gains != self.state.equalizer.gains {
            changed = true;
        }
        if self.config.always_on_top != self.always_on_top {
            changed = true;
        }
        if self.config.user_scale != self.user_scale {
            changed = true;
        }
        if self.config.shade_mode != self.windows.is_shade_mode() {
            changed = true;
        }
        if self.config.show_equalizer != self.windows.is_equalizer_visible() {
            changed = true;
        }
        if self.config.show_playlist != self.windows.is_playlist_visible() {
            changed = true;
        }
        let live_vis = visualizer_to_config(self.windows.main_window_mut().visualizer_mode());
        if self.config.visualizer_mode != live_vis {
            changed = true;
        }
        if self.config.show_remaining != self.windows.main_window_mut().show_remaining() {
            changed = true;
        }
        if self.config.recent_files != self.recent {
            changed = true;
        }
        // Preset name: query the EQ window each tick (the user may
        // have picked a preset since the last flush) and compare
        // against the persisted name.
        let live_preset = self.windows.equalizer_current_preset().map(String::from);
        if self.config.equalizer.current_preset != live_preset {
            changed = true;
        }
        if changed {
            self.mark_dirty();
        }
    }
}
