//! Centralized application state for OneAmp
//!
//! This module serves as the single source of truth for all UI state.
//! All mutations go through methods on AppState, ensuring consistency.

use oneamp_core::{RepeatMode, TrackInfo};

/// Playback state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Volume state with mute and balance
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeState {
    pub level: f32,   // 0.0 to 1.0
    pub muted: bool,  // Mute toggle
    pub balance: f32, // -1.0 (left) to 1.0 (right), 0.0 = center
}

impl Default for VolumeState {
    fn default() -> Self {
        Self {
            level: 1.0,
            muted: false,
            balance: 0.0,
        }
    }
}

/// Equalizer state
#[derive(Debug, Clone)]
pub struct EqualizerState {
    pub enabled: bool,
    pub gains: Vec<f32>, // 10 bands
    /// Master pre-amp gain in dB applied after the band processing.
    /// Persisted via `EqualizerConfig::preamp_db`.
    pub preamp_db: f32,
    /// Name of the last preset the user applied. `None` when the EQ
    /// has been edited manually since the last preset pick (or has
    /// never had one applied). Persisted via
    /// `EqualizerConfig::current_preset` so the dropdown remembers
    /// the selection across sessions.
    pub current_preset: Option<String>,
}

impl Default for EqualizerState {
    fn default() -> Self {
        Self {
            enabled: false,
            gains: vec![0.0; 10],
            preamp_db: 0.0,
            current_preset: None,
        }
    }
}

/// Centralized application state
#[derive(Debug, Clone)]
pub struct AppState {
    /// Playback state
    pub playback: PlaybackState,

    /// Current track information
    pub current_track: Option<TrackInfo>,

    /// Playback position (current seconds, total duration)
    pub position: (f32, f32),

    /// Volume and mute state
    pub volume: VolumeState,

    /// Equalizer state
    pub equalizer: EqualizerState,

    /// Repeat mode
    pub repeat_mode: RepeatMode,

    /// Shuffle enabled
    pub shuffle_enabled: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            playback: PlaybackState::Stopped,
            current_track: None,
            position: (0.0, 0.0),
            volume: VolumeState::default(),
            equalizer: EqualizerState::default(),
            repeat_mode: RepeatMode::Off,
            shuffle_enabled: false,
        }
    }
}

impl AppState {
    /// Update state from audio event
    pub fn handle_audio_event(&mut self, event: oneamp_core::AudioEvent) {
        match event {
            oneamp_core::AudioEvent::TrackLoaded(track) => {
                self.current_track = Some(track);
            }
            oneamp_core::AudioEvent::Playing => {
                self.playback = PlaybackState::Playing;
            }
            oneamp_core::AudioEvent::Paused => {
                self.playback = PlaybackState::Paused;
            }
            oneamp_core::AudioEvent::Stopped => {
                self.playback = PlaybackState::Stopped;
                self.position = (0.0, 0.0);
            }
            oneamp_core::AudioEvent::Position(current, total) => {
                self.position = (current, total);
            }
            oneamp_core::AudioEvent::VolumeUpdated(vol, muted) => {
                self.volume.level = vol;
                self.volume.muted = muted;
            }
            oneamp_core::AudioEvent::BalanceUpdated(balance) => {
                self.volume.balance = balance;
            }
            oneamp_core::AudioEvent::EqualizerUpdated(enabled, gains) => {
                self.equalizer.enabled = enabled;
                self.equalizer.gains = gains;
            }
            oneamp_core::AudioEvent::EqualizerPreampUpdated(db) => {
                self.equalizer.preamp_db = db;
            }
            oneamp_core::AudioEvent::RepeatModeUpdated(mode) => {
                self.repeat_mode = mode;
            }
            _ => {}
        }
    }

    /// Stop playback (sets state, audio command is sent separately)
    pub fn stop(&mut self) {
        self.playback = PlaybackState::Stopped;
        self.position.0 = 0.0;
    }

    /// Set repeat mode (state mirror; audio command is sent separately)
    pub fn set_repeat_mode(&mut self, mode: RepeatMode) {
        self.repeat_mode = mode;
    }

    /// Toggle shuffle (state mirror; audio command is sent separately)
    pub fn toggle_shuffle(&mut self) {
        self.shuffle_enabled = !self.shuffle_enabled;
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = AppState::default();
        assert_eq!(state.playback, PlaybackState::Stopped);
        assert_eq!(state.volume.level, 1.0);
        assert!(!state.volume.muted);
    }

    #[test]
    fn test_handle_audio_event_volume() {
        let mut state = AppState::default();
        state.handle_audio_event(oneamp_core::AudioEvent::VolumeUpdated(0.5, false));
        assert_eq!(state.volume.level, 0.5);
        assert!(!state.volume.muted);
    }

    #[test]
    fn test_handle_audio_event_balance() {
        let mut state = AppState::default();
        state.handle_audio_event(oneamp_core::AudioEvent::BalanceUpdated(-0.4));
        assert!((state.volume.balance + 0.4).abs() < 1e-6);
    }
}
