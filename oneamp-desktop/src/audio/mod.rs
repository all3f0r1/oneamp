//! Audio controller module
//!
//! Thin wrapper around [`AudioEngine`]. Spectrum / waveform live on the
//! engine itself as `ArcSwap` snapshots — the audio thread publishes
//! directly, the UI polls; we just proxy the getters here so the rest
//! of the desktop crate doesn't need to know about that detail.

use oneamp_core::{AudioCommand, AudioEngine, AudioEvent, MeterSnapshot, WaveformSnapshot};
use std::sync::Arc;

pub struct AudioController {
    engine: Option<AudioEngine>,
    /// Empty-Vec snapshot returned by `get_spectrum_data` when the
    /// engine failed to construct. Kept here so the accessor can hand
    /// out an `Arc` without allocating per call.
    empty_spectrum: Arc<Vec<f32>>,
    /// Same idea for the waveform side.
    empty_waveform: Arc<WaveformSnapshot>,
    /// Silent meter snapshot used when the engine failed to construct.
    silent_meter: Arc<MeterSnapshot>,
}

impl AudioController {
    pub fn new() -> Self {
        Self {
            engine: AudioEngine::new().ok(),
            empty_spectrum: Arc::new(Vec::new()),
            empty_waveform: Arc::new(WaveformSnapshot::empty()),
            silent_meter: Arc::new(MeterSnapshot::silent()),
        }
    }

    pub fn engine(&self) -> Option<&AudioEngine> {
        self.engine.as_ref()
    }

    pub fn send_command(&self, cmd: AudioCommand) {
        if let Some(ref engine) = self.engine {
            let _ = engine.send_command(cmd);
        }
    }

    pub fn try_recv_event(&self) -> Option<AudioEvent> {
        if let Some(ref engine) = self.engine {
            engine.try_recv_event()
        } else {
            None
        }
    }

    /// Latest spectrum bins. Returns an empty `Vec` when the audio
    /// engine failed to construct (no audio backend on this host).
    pub fn get_spectrum_data(&self) -> Arc<Vec<f32>> {
        match &self.engine {
            Some(engine) => engine.latest_spectrum(),
            None => self.empty_spectrum.clone(),
        }
    }

    /// Latest oscilloscope frame. Returns an empty
    /// [`WaveformSnapshot`] when the audio engine failed to construct.
    pub fn get_waveform_data(&self) -> Arc<WaveformSnapshot> {
        match &self.engine {
            Some(engine) => engine.latest_waveform(),
            None => self.empty_waveform.clone(),
        }
    }

    /// Latest peak / RMS meter snapshot. Returns
    /// [`MeterSnapshot::silent`] when the audio engine failed to
    /// construct.
    pub fn get_meter_data(&self) -> Arc<MeterSnapshot> {
        match &self.engine {
            Some(engine) => engine.latest_meter(),
            None => self.silent_meter.clone(),
        }
    }
}

impl Default for AudioController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_command() {
        let controller = AudioController::new();
        // Should not crash even if engine init failed
        controller.send_command(AudioCommand::Stop);
    }

    #[test]
    fn test_spectrum_data_starts_empty() {
        let controller = AudioController::new();
        // No audio yet → spectrum snapshot is empty regardless of
        // whether the engine started or not.
        assert!(controller.get_spectrum_data().is_empty());
    }

    #[test]
    fn test_waveform_data_starts_empty() {
        let controller = AudioController::new();
        assert!(controller.get_waveform_data().is_empty());
    }
}
