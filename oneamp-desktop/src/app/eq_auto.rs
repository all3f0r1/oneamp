//! EQ AUTO, Winamp's per-track auto-load presets.
//!
//! The policy lives in the engine (`oneamp_core::settings::EqAutoSettings`):
//! it resolves the preset by media id when a track becomes current —
//! including gapless and crossfaded transitions, without a UI round
//! trip — and never lets a preset overwrite the global curve. This file
//! only persists the presets and forwards changes.

use super::OneAmpApp;
use super::track_path;
use crate::config::AutoEqPreset;
use oneamp_core::{AudioCommand, MediaId};

impl OneAmpApp {
    /// The curve to persist: the global one, even while an auto-load
    /// preset is playing.
    pub(super) fn global_eq(&self) -> (Vec<f32>, f32) {
        match self.audio.snapshot() {
            Some(snap) if snap.eq.auto_for.is_some() => {
                (snap.eq.global.gains_db.to_vec(), snap.eq.global.preamp_db)
            }
            _ => (
                self.state.equalizer.gains.clone(),
                self.state.equalizer.preamp_db,
            ),
        }
    }

    pub(super) fn toggle_eq_auto(&mut self) {
        let on = !self.config.equalizer.auto;
        self.config.equalizer.auto = on;
        self.mark_dirty();
        self.audio.send_command(AudioCommand::SetEqAuto(on));
    }

    pub(super) fn save_eq_auto_preset(&mut self) {
        let Some(path) = self.state.current_track.as_ref().map(track_path) else {
            self.push_toast("Play a track first", std::time::Duration::from_millis(1800));
            return;
        };
        let preset = AutoEqPreset {
            gains: self.state.equalizer.gains.clone(),
            preamp_db: self.state.equalizer.preamp_db,
        };
        self.audio.send_command(AudioCommand::SetEqAutoPreset(
            MediaId::from_path(&path),
            Some(preset.curve()),
        ));
        self.config
            .equalizer
            .auto_presets
            .insert(path.to_string_lossy().into_owned(), preset);
        self.mark_dirty();
        self.push_toast(
            "EQ will auto-load for this track (AUTO on)",
            std::time::Duration::from_millis(2000),
        );
    }

    pub(super) fn remove_eq_auto_preset(&mut self) {
        let Some(path) = self.state.current_track.as_ref().map(track_path) else {
            return;
        };
        if self
            .config
            .equalizer
            .auto_presets
            .remove(&*path.to_string_lossy())
            .is_some()
        {
            self.audio.send_command(AudioCommand::SetEqAutoPreset(
                MediaId::from_path(&path),
                None,
            ));
            self.mark_dirty();
            self.push_toast(
                "Auto-load preset removed",
                std::time::Duration::from_millis(1800),
            );
        }
    }
}
