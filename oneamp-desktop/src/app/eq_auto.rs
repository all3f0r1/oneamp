//! EQ AUTO, Winamp's per-track auto-load presets.
//!
//! Contract:
//! - The global curve is the last one set by hand; it's what gets saved.
//! - AUTO on + the track has an auto-load preset: that preset plays.
//! - No preset for the track, or AUTO off: the global curve plays.
//! - An auto-loaded preset never becomes the global curve. Editing the
//!   EQ by hand while one is applied makes the edited curve global.

use super::OneAmpApp;
use crate::config::AutoEqPreset;
use oneamp_core::AudioCommand;
use std::path::Path;

impl OneAmpApp {
    /// The curve to persist: the global one, even while an auto-load
    /// preset is playing.
    pub(super) fn global_eq(&self) -> (Vec<f32>, f32) {
        self.eq_global_stash.clone().unwrap_or_else(|| {
            (
                self.state.equalizer.gains.clone(),
                self.state.equalizer.preamp_db,
            )
        })
    }

    pub(super) fn toggle_eq_auto(&mut self) {
        let on = !self.config.equalizer.auto;
        self.config.equalizer.auto = on;
        self.mark_dirty();
        if on {
            let path = self.state.current_track.as_ref().map(|t| t.path.clone());
            if let Some(path) = path {
                self.apply_eq_auto(&path);
            }
        } else {
            self.restore_global_eq();
        }
    }

    /// Track `path` just loaded (or AUTO turned on while it plays).
    pub(super) fn apply_eq_auto(&mut self, path: &Path) {
        let preset = self
            .config
            .equalizer
            .auto_presets
            .get(&path.to_string_lossy().into_owned())
            .cloned();
        match preset.filter(|_| self.config.equalizer.auto) {
            Some(p) => {
                if self.eq_global_stash.is_none() {
                    self.eq_global_stash = Some(self.global_eq());
                }
                self.send_eq_curve(&p.gains, p.preamp_db);
                self.eq_auto_applied = Some(p.gains);
            }
            None => self.restore_global_eq(),
        }
    }

    fn restore_global_eq(&mut self) {
        self.eq_auto_applied = None;
        if let Some((gains, preamp)) = self.eq_global_stash.take() {
            self.send_eq_curve(&gains, preamp);
        }
    }

    fn send_eq_curve(&self, gains: &[f32], preamp_db: f32) {
        self.audio
            .send_command(AudioCommand::SetEqualizerBands(gains.to_vec()));
        self.audio
            .send_command(AudioCommand::SetEqualizerPreamp(preamp_db));
    }

    /// Engine echoed new gains. If they differ from the auto-loaded
    /// preset, the user edited the EQ by hand: that curve is now global.
    pub(super) fn note_eq_update(&mut self, gains: &[f32]) {
        let differs = |a: &[f32]| {
            a.len() != gains.len() || a.iter().zip(gains).any(|(x, y)| (x - y).abs() > 0.01)
        };
        if self.eq_auto_applied.as_deref().is_some_and(differs) {
            self.eq_auto_applied = None;
            self.eq_global_stash = None;
        }
    }

    pub(super) fn save_eq_auto_preset(&mut self) {
        let Some(path) = self.state.current_track.as_ref().map(|t| t.path.clone()) else {
            self.push_toast("Play a track first", std::time::Duration::from_millis(1800));
            return;
        };
        let preset = AutoEqPreset {
            gains: self.state.equalizer.gains.clone(),
            preamp_db: self.state.equalizer.preamp_db,
        };
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
        let Some(path) = self.state.current_track.as_ref().map(|t| t.path.clone()) else {
            return;
        };
        if self
            .config
            .equalizer
            .auto_presets
            .remove(&*path.to_string_lossy())
            .is_some()
        {
            self.mark_dirty();
            self.restore_global_eq();
            self.push_toast(
                "Auto-load preset removed",
                std::time::Duration::from_millis(1800),
            );
        }
    }
}
