//! ReplayGain figures for files that carry no ReplayGain tags.
//!
//! ReplayGain 2.0: EBU R128 integrated loudness (ITU-R BS.1770 K-weighting
//! and gating) against a −18 LUFS reference. Album loudness gates every
//! track's blocks together, as if the album were one file, so the gain
//! keeps the tracks' relative levels. Peaks are sample peaks, like the
//! `REPLAYGAIN_*_PEAK` tags most taggers write.
//!
//! Nothing here writes to the files: the host keeps the figures and
//! hands them to the engine through [`crate::MediaDescriptor::with_gain`].

use crate::dsp::MediaGain;
use crate::error::PlayerError;
use crate::symphonia_player::SymphoniaPlayer;
use ebur128::{Channel, EbuR128, Mode};
use std::path::Path;

/// ReplayGain 2.0 reference loudness.
pub const REFERENCE_LUFS: f64 = -18.0;

/// One measured track.
pub struct Measurement {
    meter: EbuR128,
    peak: f32,
}

fn meter_error(e: ebur128::Error) -> PlayerError {
    PlayerError::internal(format!("loudness meter: {e}"))
}

/// Gain from a loudness, `None` for silence (no gated block).
fn gain_db(lufs: f64) -> Option<f32> {
    lufs.is_finite()
        .then(|| (REFERENCE_LUFS - lufs).clamp(-51.0, 51.0) as f32)
}

fn new_meter(channels: u32, rate: u32) -> Result<EbuR128, PlayerError> {
    let mut m = EbuR128::new(channels, rate, Mode::I | Mode::HISTOGRAM).map_err(meter_error)?;
    map_mono(&mut m, channels)?;
    Ok(m)
}

/// A mono file counts as the same signal on two speakers, so a mono and
/// a stereo master of one recording come out at one loudness.
fn map_mono(m: &mut EbuR128, channels: u32) -> Result<(), PlayerError> {
    if channels == 1 {
        m.set_channel(0, Channel::DualMono).map_err(meter_error)?;
    }
    Ok(())
}

impl Measurement {
    fn new() -> Self {
        Self {
            meter: new_meter(2, 48_000).expect("valid default format"),
            peak: 0.0,
        }
    }

    /// Feed one interleaved block. A format change (chained Ogg)
    /// re-targets the meter; the blocks already gated are kept.
    fn add(&mut self, rate: u32, channels: u32, samples: &[f32]) -> Result<(), PlayerError> {
        if channels == 0 || rate == 0 {
            return Ok(());
        }
        if self.meter.channels() != channels || self.meter.rate() != rate {
            self.meter
                .change_parameters(channels, rate)
                .map_err(meter_error)?;
            map_mono(&mut self.meter, channels)?;
        }
        self.meter.add_frames_f32(samples).map_err(meter_error)?;
        self.peak = samples.iter().fold(self.peak, |p, s| p.max(s.abs()));
        Ok(())
    }

    /// Integrated loudness, LUFS (−∞ for silence).
    pub fn loudness_lufs(&self) -> f64 {
        self.meter.loudness_global().unwrap_or(f64::NEG_INFINITY)
    }

    /// Track gain, dB; `None` for a silent file.
    pub fn track_gain_db(&self) -> Option<f32> {
        gain_db(self.loudness_lufs())
    }

    /// Sample peak, linear.
    pub fn peak(&self) -> f32 {
        self.peak
    }
}

/// Decode `path` to the end and measure it. `cancelled` is polled between
/// blocks; a cancelled scan returns an error.
pub fn measure_file(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Measurement, PlayerError> {
    let mut player = SymphoniaPlayer::load(path)?;
    let mut m = Measurement::new();
    while let Some(block) = player.decode_next()? {
        if cancelled() {
            return Err(PlayerError::internal("loudness scan cancelled"));
        }
        let channels = u32::from(block.layout.channels());
        m.add(block.rate, channels, &block.samples)?;
    }
    Ok(m)
}

/// Track and album figures for tracks of one album, in input order.
/// Album loudness gates all the tracks' blocks together.
pub fn album_gains(tracks: &[Measurement]) -> Vec<MediaGain> {
    let album_gain = EbuR128::loudness_global_multiple(tracks.iter().map(|t| &t.meter))
        .ok()
        .and_then(gain_db);
    let album_peak = tracks.iter().map(|t| t.peak).fold(0.0_f32, f32::max);
    tracks
        .iter()
        .map(|t| MediaGain {
            track_gain_db: t.track_gain_db(),
            track_peak: Some(t.peak),
            album_gain_db: album_gain,
            album_peak: album_gain.map(|_| album_peak),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, channels: u32, secs: f32, amp: f32) -> Vec<f32> {
        let frames = (rate as f32 * secs) as usize;
        (0..frames)
            .flat_map(|n| {
                let s = amp * (std::f32::consts::TAU * 1000.0 * n as f32 / rate as f32).sin();
                std::iter::repeat_n(s, channels as usize)
            })
            .collect()
    }

    fn measure(rate: u32, channels: u32, samples: &[f32]) -> Measurement {
        let mut m = Measurement::new();
        m.add(rate, channels, samples).unwrap();
        m
    }

    #[test]
    fn half_scale_1k_sine_reads_minus_6_lufs() {
        let m = measure(48_000, 2, &sine(48_000, 2, 5.0, 0.5));
        assert!(
            (m.loudness_lufs() + 6.02).abs() < 0.1,
            "{}",
            m.loudness_lufs()
        );
        let g = m.track_gain_db().unwrap();
        assert!((g + 11.98).abs() < 0.1, "gain {g}");
        assert!((m.peak() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn mono_measures_like_the_same_signal_in_stereo() {
        let mono = measure(44_100, 1, &sine(44_100, 1, 5.0, 0.5));
        let stereo = measure(44_100, 2, &sine(44_100, 2, 5.0, 0.5));
        assert!((mono.loudness_lufs() - stereo.loudness_lufs()).abs() < 0.05);
    }

    #[test]
    fn album_gain_keeps_relative_levels() {
        let loud = measure(48_000, 2, &sine(48_000, 2, 5.0, 0.5));
        let quiet = measure(48_000, 2, &sine(48_000, 2, 5.0, 0.125));
        let g = album_gains(&[loud, quiet]);
        let album = g[0].album_gain_db.unwrap();
        assert_eq!(g[1].album_gain_db, Some(album));
        // One gain for both: the 12 dB step between them survives.
        let (t0, t1) = (g[0].track_gain_db.unwrap(), g[1].track_gain_db.unwrap());
        assert!((t1 - t0 - 12.04).abs() < 0.1);
        assert!(album < t1 && album > t0, "album {album}, tracks {t0} {t1}");
        assert_eq!(g[1].album_peak, Some(0.5));
    }

    #[test]
    fn silence_has_no_gain() {
        let m = measure(48_000, 2, &vec![0.0; 48_000 * 2 * 3]);
        assert_eq!(m.track_gain_db(), None);
        assert_eq!(album_gains(&[m])[0].album_gain_db, None);
    }

    #[test]
    fn measures_a_real_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/a.flac");
        let m = measure_file(&path, &|| false).unwrap();
        assert!(m.track_gain_db().is_some());
        assert!(m.peak() > 0.0 && m.peak() <= 1.0);
    }
}
