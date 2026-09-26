use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender};
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::equalizer::BiquadFilter;
#[cfg(feature = "audio")]
use crate::output::AudioOutput;
use crate::symphonia_player::SymphoniaPlayer;
use crate::{
    AudioCaptureBuffer, AudioCommand, AudioEvent, Equalizer, MeterSnapshot, RepeatMode, TrackInfo,
    WaveformSnapshot,
};

/// FFT window size. 2048 at 44.1 kHz = ~46 ms window, ~21 Hz bin width —
/// fine enough to distinguish the 31 Hz / 63 Hz EQ bands. v1 used 1024
/// (~43 Hz/bin) which conflated the two lowest bands.
const FFT_SIZE: usize = 2048;
/// Number of mono frames retained in the capture buffer (= one FFT
/// window). Stereo interleaved doubles this.
const CAPTURE_FRAMES: usize = FFT_SIZE;
/// Number of spectrum bars rendered by the analyzer. The visualization
/// area only ever displays 16, so the engine produces 16 directly —
/// v1 produced 64 and the renderer re-averaged to 16, a pure waste.
const SPECTRUM_BINS: usize = 16;
/// Display dB floor for the spectrum. Magnitudes below this read as 0;
/// 0 dBFS reads as 1.0. -60 dB covers ~10 bits of audio dynamic range,
/// the audible region for most music.
const SPECTRUM_DB_FLOOR: f32 = -60.0;
/// Number of pre-decimated (min, max) pairs emitted per oscilloscope
/// refresh. Independent of the display width — the renderer picks
/// representative pairs for the actual pixel count. 256 covers any
/// reasonable skin scale without flooding the channel.
const WAVEFORM_COLUMNS: usize = 256;
/// Number of mono frames consumed per oscilloscope frame. Half the FFT
/// window so the zero-crossing trigger has the other half as search
/// space without ever running short of data.
const WAVEFORM_FRAMES: usize = FFT_SIZE / 2;

/// Compute the dB-correct 16-bin spectrum from interleaved stereo PCM.
///
/// Pipeline:
/// 1. Downmix to mono, remove DC (subtract mean) so a DC-biased file
///    doesn't pin bin 0 high.
/// 2. Apply Hann window to reduce spectral leakage.
/// 3. FFT, take magnitude of the positive-frequency half, cap at
///    `0.95 * Nyquist` so we cover content up to ~21 kHz (v1 capped at
///    `0.6 * Nyquist` ≈ 13 kHz, discarding the brilliance band).
/// 4. Group log-spaced into 16 bins. Per group: take the **max** raw
///    magnitude (gives crisp peaks under transient content; a mean
///    would smear them).
/// 5. Normalize against the Hann window's coherent gain
///    (`FFT_SIZE * 0.5`) so a full-scale sine reads 0 dBFS.
/// 6. Convert to dB and map `[SPECTRUM_DB_FLOOR, 0.0]` linearly to
///    `[0.0, 1.0]`.
///
/// `frames` is interleaved stereo with at least `FFT_SIZE * 2` samples.
/// Shorter inputs are zero-padded.
fn compute_spectrum(frames: &[f32], fft: &dyn Fft<f32>) -> Vec<f32> {
    let take = frames.len().min(FFT_SIZE * 2);
    let mut mono: Vec<f32> = frames[..take]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| 0.5 * (c[0] + c[1]))
        .collect();
    // DC removal — bin 0/1 spikes on tracks with a non-zero offset
    // would otherwise saturate the lowest analyzer bar.
    if !mono.is_empty() {
        let mean = mono.iter().sum::<f32>() / mono.len() as f32;
        for s in mono.iter_mut() {
            *s -= mean;
        }
    }

    let mut buffer: Vec<Complex32> = Vec::with_capacity(FFT_SIZE);
    let n = mono.len().min(FFT_SIZE);
    for (i, &s) in mono[..n].iter().enumerate() {
        let w = 0.5 - 0.5 * ((std::f32::consts::TAU * i as f32) / (FFT_SIZE as f32 - 1.0)).cos();
        buffer.push(Complex32::new(s * w, 0.0));
    }
    buffer.resize(FFT_SIZE, Complex32::new(0.0, 0.0));

    fft.process(&mut buffer);

    let half = FFT_SIZE / 2;
    let mags: Vec<f32> = buffer[..half].iter().map(|c| c.norm()).collect();

    // Hann coherent gain ≈ 0.5; reference at FFT_SIZE * 0.5 so a
    // full-scale sine ends up at 0 dBFS in its bin.
    let ref_mag = FFT_SIZE as f32 * 0.5;
    let inv_db_span = 1.0 / -SPECTRUM_DB_FLOOR;

    // Log-spaced grouping. Skip bin 0 (DC residue after detrending).
    // Cap at 0.95 * Nyquist — content above that is mostly anti-alias
    // ringing or codec noise, no musical value.
    let lo_bin = 1.0_f32;
    let hi_bin = (half as f32 * 0.95).max(2.0);
    let log_step = (hi_bin / lo_bin).ln() / SPECTRUM_BINS as f32;

    let mut bins = vec![0.0_f32; SPECTRUM_BINS];
    for (bi, bin) in bins.iter_mut().enumerate() {
        let lo = (lo_bin * (bi as f32 * log_step).exp()) as usize;
        let hi = (lo_bin * ((bi + 1) as f32 * log_step).exp()).ceil() as usize;
        let hi = hi.max(lo + 1).min(half);
        // Max-of-group: a transient in any FFT bin of the group should
        // light up the bar; averaging would mask short peaks.
        let peak = mags[lo..hi].iter().copied().fold(0.0_f32, f32::max);

        let normalized = (peak / ref_mag).max(1e-10);
        let db = 20.0 * normalized.log10();
        // Map [SPECTRUM_DB_FLOOR, 0.0] dB to [0.0, 1.0].
        *bin = ((db - SPECTRUM_DB_FLOOR) * inv_db_span).clamp(0.0, 1.0);
    }
    bins
}

/// Find the index of the first rising zero-crossing in `mono[..search_len]`.
/// Returns 0 when no transition is found — the caller starts the window
/// at the buffer head, giving a "scrolling" look that's still better
/// than dropping the frame.
fn find_rising_zero_crossing(mono: &[f32], search_len: usize) -> usize {
    let end = search_len.min(mono.len());
    for i in 1..end {
        if mono[i - 1] <= 0.0 && mono[i] > 0.0 {
            return i;
        }
    }
    0
}

/// Pre-decimate interleaved stereo into a phase-stable oscilloscope
/// snapshot. Returns one (min, max) pair per output column over
/// [`WAVEFORM_FRAMES`] mono frames starting at the latest rising
/// zero-crossing.
///
/// Why min/max instead of nearest-neighbor:
/// - v1 picked one sample per output column; a 8 kHz sine at 44.1 kHz
///   (5.5 samples/period) aliased into a slow "phantom" wiggle
///   depending on alignment.
/// - Min/max preserves the extreme excursion within the bucket, so a
///   high-frequency wave shows as a vertical bar rather than a line —
///   visually correct, no aliasing.
///
/// Why trigger on zero-crossing:
/// - A periodic signal at the same frequency from frame to frame would
///   otherwise display at a different phase each refresh, producing
///   the classic "scrolling" oscilloscope. Triggering pins the display
///   to a known reference point.
fn compute_waveform(samples: &[f32]) -> WaveformSnapshot {
    let total_frames = samples.len() / 2;
    if total_frames < 2 {
        return WaveformSnapshot {
            mins: vec![0.0; WAVEFORM_COLUMNS],
            maxs: vec![0.0; WAVEFORM_COLUMNS],
        };
    }

    // Downmix to mono.
    let mut mono = Vec::with_capacity(total_frames);
    for fr in 0..total_frames {
        mono.push(0.5 * (samples[2 * fr] + samples[2 * fr + 1]));
    }

    // Trigger on a rising zero-crossing in the first half of the
    // buffer, leaving the second half as the window we display. With
    // CAPTURE_FRAMES = WAVEFORM_FRAMES * 2, the search and the window
    // tile the capture exactly.
    let search_len = total_frames.saturating_sub(WAVEFORM_FRAMES).max(1);
    let mut start = find_rising_zero_crossing(&mono, search_len);
    // If trigger pushed us past the buffer's tail, fall back to a
    // window that fits.
    let max_start = total_frames.saturating_sub(WAVEFORM_FRAMES);
    if start > max_start {
        start = max_start;
    }
    let window = &mono[start..start + WAVEFORM_FRAMES.min(mono.len() - start)];

    let mut mins = vec![0.0_f32; WAVEFORM_COLUMNS];
    let mut maxs = vec![0.0_f32; WAVEFORM_COLUMNS];
    if window.is_empty() {
        return WaveformSnapshot { mins, maxs };
    }
    for (i, (mn, mx)) in mins.iter_mut().zip(maxs.iter_mut()).enumerate() {
        let lo = (i * window.len()) / WAVEFORM_COLUMNS;
        let hi = ((i + 1) * window.len()) / WAVEFORM_COLUMNS;
        let hi = hi.max(lo + 1).min(window.len());
        let slice = &window[lo..hi];
        let (lo_v, hi_v) = slice
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| {
                (a.min(v), b.max(v))
            });
        *mn = lo_v;
        *mx = hi_v;
    }

    WaveformSnapshot { mins, maxs }
}

/// Audio engine state
#[derive(Clone)]
struct AudioEngineState {
    volume: f32,
    muted: bool,
    balance: f32,
    repeat_mode: RepeatMode,
    shuffle_enabled: bool,
    /// Master pre-amp gain in dB applied after EQ band processing.
    /// Multiplied as `10^(preamp_db / 20)` against every output sample.
    preamp_db: f32,
    /// When true, the last `crossfade_duration_secs` of a track mix with
    /// the head of the queued next track using equal-power weights, so
    /// the listener never hears a hard transition. Falls back to the
    /// existing gapless swap when no pending track exists or formats
    /// don't match.
    crossfade_enabled: bool,
    /// Fade window in seconds. Honored only when `crossfade_enabled`.
    /// Clamped to 0.5..=30 on assignment.
    crossfade_duration_secs: f32,
    /// Legacy on/off toggle, kept so `SetReplayGainEnabled(bool)` keeps
    /// working. `false` forces `replaygain_mode` to be treated as
    /// `Off`; `true` lets `replaygain_mode` drive track/album/auto
    /// selection. Disabled by default — opt-in via Options menu so
    /// users without RG-tagged libraries see no behaviour change.
    replaygain_enabled: bool,
    /// Which ReplayGain figure to apply when `replaygain_enabled`.
    /// Defaults to `Track`, so the historical boolean behaviour
    /// (track-gain on enable) is preserved bit-for-bit.
    replaygain_mode: crate::ReplayGainMode,
    /// Effective ReplayGain in dB for the *currently playing* track,
    /// already resolved through `replaygain_mode`. Set on each
    /// `TrackLoaded`, zero when RG is disabled or the relevant tag is
    /// absent. Independent of `preamp_db` so the user's slider setting
    /// survives across tracks.
    track_gain_db: f32,
    /// When true, stereo frames are downmixed to mono (channel average
    /// written to both channels) before the gain/balance/limiter chain.
    /// Off by default; mirrors Winamp's mono toggle.
    mono_enabled: bool,
    /// cpal output device name to open on the next track load. `None`
    /// resolves to the host's default device. The current `AudioOutput`
    /// is never torn down to honour a switch — applying it on next load
    /// avoids a gap in live audio. Names map to what
    /// `oneamp_core::list_output_devices` returns.
    output_device_name: Option<String>,
    /// Loudness-compensation toggle. When true, the audio thread
    /// derives a volume-dependent low- and high-shelf boost so quiet
    /// listening preserves perceived tonal balance.
    loudness_enabled: bool,
    /// See `AudioCommand::SetStopAfterCurrent`.
    stop_after_current: bool,
}

impl Default for AudioEngineState {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
            balance: 0.0,
            repeat_mode: RepeatMode::Off,
            shuffle_enabled: false,
            preamp_db: 0.0,
            crossfade_enabled: false,
            crossfade_duration_secs: 3.0,
            replaygain_enabled: false,
            replaygain_mode: crate::ReplayGainMode::default(),
            track_gain_db: 0.0,
            mono_enabled: false,
            output_device_name: None,
            loudness_enabled: false,
            stop_after_current: false,
        }
    }
}

impl AudioEngineState {
    /// Part of the volume above unity. Applied in the engine, ahead of
    /// the limiter, so >100 % is limited instead of clipped.
    fn boost(&self) -> f32 {
        if self.muted {
            1.0
        } else {
            self.volume.max(1.0)
        }
    }

    /// Attenuation part, applied in the output callback so volume moves
    /// act immediately instead of after the ~0.5 s ring.
    fn device_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume.min(1.0)
        }
    }

    /// Resolve the effective ReplayGain (dB) for a track given the
    /// current enable flag + mode. Returns 0.0 when RG is disabled —
    /// summing zero leaves the user's preamp untouched. Centralizes the
    /// track/album/auto selection so every site that sets
    /// `track_gain_db` stays consistent.
    fn resolve_replaygain_db(&self, track: &TrackInfo) -> f32 {
        if !self.replaygain_enabled {
            return 0.0;
        }
        self.replaygain_mode.gain_db(
            track.replaygain_track_gain_db,
            track.replaygain_album_gain_db,
        )
    }
}

/// Collapse interleaved stereo to mono in place: each L/R pair is
/// replaced by its average, written back to both channels so the buffer
/// stays stereo-shaped (two identical channels). No-op on non-stereo
/// content — mono is already mono, and we don't downmix surround here
/// (it never enters the stereo gain chain anyway).
///
/// Placed *before* balance so panning a mono'd signal still works, and
/// before the limiter so the meter reflects the actual mono output.
fn apply_mono_downmix(samples: &mut [f32], channels: u16) {
    if channels != 2 {
        return;
    }
    for chunk in samples.as_chunks_mut::<2>().0 {
        let m = 0.5 * (chunk[0] + chunk[1]);
        chunk[0] = m;
        chunk[1] = m;
    }
    // A malformed odd-length stereo buffer leaves one trailing sample;
    // a lone sample is already "mono", so leaving it untouched is the
    // correct identity — it stays in the stream.
}

/// Multiply every sample by the linear gain matching `preamp_db`. No-op
/// when the gain is effectively 1.0 (saves a cycle per sample on the
/// hot path when the user hasn't touched the slider).
fn apply_preamp(samples: &mut [f32], preamp_db: f32) {
    if preamp_db.abs() < 1e-3 {
        return;
    }
    let gain = 10.0_f32.powf(preamp_db / 20.0);
    for s in samples.iter_mut() {
        *s *= gain;
    }
}

/// Per-channel peak + RMS metering. Peak is the simple max of |s|
/// since the last [`snapshot`] call (reset on snapshot — so each UI
/// frame sees a *short-term* peak); RMS is a one-pole smoothed
/// running mean-square with the configured time constant.
///
/// The smoother runs on the **post-limiter** signal so the meter
/// reflects what's actually heading to the device, not what the
/// decoder produced. v1 had no metering at all.
struct PeakRmsMeter {
    sample_rate: f32,
    /// `exp(-1 / (τ · sr))` for the RMS-smoothing one-pole. New input
    /// weight is `1 - rms_alpha`; lower α → faster reaction.
    rms_alpha: f32,
    peak_l: f32,
    peak_r: f32,
    /// Running mean of `sample²`. Square-rooted at snapshot time so
    /// the per-sample math stays multiply-only (no sqrt on the hot
    /// path).
    rms_sq_l: f32,
    rms_sq_r: f32,
}

impl PeakRmsMeter {
    const RMS_TIME_CONSTANT_SECS: f32 = 0.300;

    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            rms_alpha: (-1.0 / (Self::RMS_TIME_CONSTANT_SECS * sample_rate)).exp(),
            peak_l: 0.0,
            peak_r: 0.0,
            rms_sq_l: 0.0,
            rms_sq_r: 0.0,
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() < 0.1 {
            return;
        }
        self.sample_rate = sample_rate;
        self.rms_alpha = (-1.0 / (Self::RMS_TIME_CONSTANT_SECS * sample_rate)).exp();
        // Don't clear state — the meter quickly recovers anyway, and
        // a hard reset would be visible as a "drop to zero" spike.
    }

    fn process(&mut self, samples: &[f32], channels: u16) {
        let ch = channels.max(1) as usize;
        let alpha = self.rms_alpha;
        if ch == 2 {
            for chunk in samples.as_chunks::<2>().0 {
                let l = chunk[0];
                let r = chunk[1];
                let la = l.abs();
                let ra = r.abs();
                if la > self.peak_l {
                    self.peak_l = la;
                }
                if ra > self.peak_r {
                    self.peak_r = ra;
                }
                let lsq = l * l;
                let rsq = r * r;
                self.rms_sq_l = lsq + (self.rms_sq_l - lsq) * alpha;
                self.rms_sq_r = rsq + (self.rms_sq_r - rsq) * alpha;
            }
        } else if ch == 1 {
            for &s in samples {
                let a = s.abs();
                if a > self.peak_l {
                    self.peak_l = a;
                }
                let sq = s * s;
                self.rms_sq_l = sq + (self.rms_sq_l - sq) * alpha;
            }
            // Mirror onto the right channel so the UI shows balanced
            // bars on mono content rather than a dead right side.
            self.peak_r = self.peak_l;
            self.rms_sq_r = self.rms_sq_l;
        }
        // Multichannel ≠ 1 or 2: leave untouched (we don't push that
        // through the rest of the gain chain either).
    }

    /// Build a [`MeterSnapshot`] and reset the *peak* state so the
    /// next call surfaces the short-term peak between refreshes. RMS
    /// keeps its smoothed running value across snapshots — clearing
    /// it would produce a visible "jump" on every refresh tick.
    fn snapshot(&mut self) -> MeterSnapshot {
        let snap = MeterSnapshot {
            peak_l: self.peak_l,
            peak_r: self.peak_r,
            rms_l: self.rms_sq_l.sqrt(),
            rms_r: self.rms_sq_r.sqrt(),
        };
        self.peak_l = 0.0;
        self.peak_r = 0.0;
        snap
    }
}

/// "Loudness" frequency-domain compensation à la Fletcher-Munson /
/// ISO 226. Counters the ear's reduced sensitivity to bass and treble
/// at low playback levels: at master volume = 1.0 the filter is a
/// no-op (0 dB shelves), and as volume drops the low- and high-shelf
/// gains rise so the perceived tonal balance stays roughly constant
/// across the volume range.
///
/// Two biquads in series (low-shelf @ 120 Hz, high-shelf @ 4 kHz),
/// both at S=1 corner-slope. Compensation curve:
///
///   `comp_db = ((-20·log10(volume)) · 0.3).clamp(0, 6)`
///
/// Yields ≈ 1.8 dB at volume = 0.5, 3.1 dB at 0.3, 6 dB at 0.1 (clamp
/// floor). Capped at 6 dB so a normalized track plus loudness boost
/// can't blow past the brickwall limiter on a quiet listening level.
struct LoudnessFilter {
    sample_rate: f32,
    enabled: bool,
    /// Active compensation. 0.0 when disabled.
    comp_db: f32,
    low_shelf: BiquadFilter,
    high_shelf: BiquadFilter,
    /// Set while skipped; see `process_in_place`.
    bypassed: bool,
}

impl LoudnessFilter {
    /// Corner frequencies for the two shelves. 120 Hz / 4 kHz roughly
    /// brackets where ISO 226 contours diverge most at low SPL.
    const LOW_FREQ_HZ: f32 = 120.0;
    const HIGH_FREQ_HZ: f32 = 4000.0;
    const MAX_COMP_DB: f32 = 6.0;
    /// Scaling: master attenuation (dB) → comp (dB). 0.3 × means
    /// halving the volume (~-6 dB) adds ~1.8 dB of comp.
    const COMP_RATIO: f32 = 0.3;

    fn new(sample_rate: f32) -> Self {
        let mut f = Self {
            sample_rate,
            enabled: false,
            comp_db: 0.0,
            low_shelf: BiquadFilter::new(),
            high_shelf: BiquadFilter::new(),
            bypassed: true,
        };
        // Snap to flat pass-through at the right sample rate so any
        // future ramp starts from a sane reference.
        f.low_shelf
            .set_low_shelf_snap(sample_rate, Self::LOW_FREQ_HZ, 0.0);
        f.high_shelf
            .set_high_shelf_snap(sample_rate, Self::HIGH_FREQ_HZ, 0.0);
        f
    }

    /// Translate a master volume in [0, 1] to a compensation in dB.
    fn comp_db_for_volume(volume: f32) -> f32 {
        let v = volume.clamp(0.001, 1.0);
        let attenuation = -20.0 * v.log10();
        (attenuation * Self::COMP_RATIO).clamp(0.0, Self::MAX_COMP_DB)
    }

    /// Recompute shelf gains for the given master volume. No-op when
    /// the filter is disabled (active gains stay at 0 dB; the shelves
    /// keep coasting toward pass-through if they were mid-ramp).
    fn set_for_volume(&mut self, volume: f32) {
        let target = if self.enabled {
            Self::comp_db_for_volume(volume)
        } else {
            0.0
        };
        if (target - self.comp_db).abs() < 0.01 {
            return;
        }
        self.comp_db = target;
        self.low_shelf
            .set_low_shelf(self.sample_rate, Self::LOW_FREQ_HZ, target);
        self.high_shelf
            .set_high_shelf(self.sample_rate, Self::HIGH_FREQ_HZ, target);
    }

    /// Toggle the loudness curve. Pairs with [`set_for_volume`] —
    /// callers should follow this with `set_for_volume(current_volume)`
    /// so the new state takes effect immediately.
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Snap-update on sample-rate change. Filter state would otherwise
    /// be left at coefficients meant for a different rate.
    fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() < 0.1 {
            return;
        }
        self.sample_rate = sample_rate;
        self.low_shelf
            .set_low_shelf_snap(sample_rate, Self::LOW_FREQ_HZ, self.comp_db);
        self.high_shelf
            .set_high_shelf_snap(sample_rate, Self::HIGH_FREQ_HZ, self.comp_db);
    }

    /// No compensation and no ramp in flight. 0 dB shelves are unity in
    /// theory but not bit-exact in float, so they are skipped entirely.
    fn is_transparent(&self) -> bool {
        self.comp_db.abs() < 1e-3 && !self.low_shelf.ramp_active() && !self.high_shelf.ramp_active()
    }

    /// Apply the low- and high-shelf in series to interleaved stereo /
    /// mono samples. Short-circuits when the filter has effectively
    /// nothing to do: disabled, target at pass-through, AND the ramp
    /// has fully converged. The third check matters during a
    /// disable-toggle so the user doesn't hear a click while the
    /// filter is still ramping from a previous compensation toward
    /// zero.
    fn process_in_place(&mut self, samples: &mut [f32], channels: u16) {
        if self.is_transparent() {
            self.bypassed = true;
            return;
        }
        if self.bypassed {
            // Start from clean history, not the state left seconds ago.
            self.low_shelf.reset();
            self.high_shelf.reset();
            self.bypassed = false;
        }
        let ch = channels.max(1) as usize;
        if ch == 2 {
            let (frames, rest) = samples.as_chunks_mut::<2>();
            for chunk in frames {
                let (l, r) = self.low_shelf.process_stereo(chunk[0], chunk[1]);
                let (l, r) = self.high_shelf.process_stereo(l, r);
                chunk[0] = l;
                chunk[1] = r;
            }
            // A malformed/odd-length stereo buffer would otherwise drop
            // its trailing lone sample. Run it through the mono path so
            // no sample escapes the shelves unfiltered.
            if let [last] = rest {
                let v = self.low_shelf.process_mono(*last);
                *last = self.high_shelf.process_mono(v);
            }
        } else if ch == 1 {
            for s in samples.iter_mut() {
                let v = self.low_shelf.process_mono(*s);
                *s = self.high_shelf.process_mono(v);
            }
        }
        // Multichannel ≠ 1 or 2: leave untouched (matches the rest of
        // the audio pipeline, which only routes stereo / mono through
        // the EQ-shaped chain).
    }
}

/// Lookahead brickwall limiter with a 0 dBFS ceiling.
///
/// Transparent (bit-exact, only delayed by the lookahead) for any
/// signal that stays within full scale — i.e. every untouched PCM
/// source. It only engages when the DSP chain (EQ/preamp/ReplayGain
/// boost, >100 % volume, crossfade sum) or a lossy decoder's
/// inter-sample overs push a sample past ±1.0.
///
/// Gain computer: per-frame required gain `min(1, ceiling / peak)`,
/// sliding minimum over the lookahead window, one-pole release, then a
/// moving average over the same window. The min-then-average pair turns
/// every gain drop into a linear ramp that *finishes* by the time the
/// peak leaves the delay line, so there is no overshoot and no
/// sample-level gain step (which is what the old zero-attack design
/// produced — audible as distortion).
struct PeakLimiter {
    sample_rate: f32,
    channels: usize,
    /// Window length in frames.
    window: usize,
    release: f32,
    /// Audio delay line, `window - 1` frames (interleaved).
    delay: std::collections::VecDeque<f32>,
    /// Monotonic deque of (frame index, required gain) for the sliding min.
    mins: std::collections::VecDeque<(u64, f32)>,
    /// Released gain history for the moving average.
    avg: std::collections::VecDeque<f32>,
    avg_sum: f64,
    /// `1 - released gain`. Kept as the deficit because a gain stored
    /// near 1.0 in f32 stalls: the release step drops below its ulp.
    deficit: f32,
    frame: u64,
    /// Whether the last `process` call attenuated anything.
    engaged: bool,
}

impl PeakLimiter {
    const CEILING: f32 = 1.0;
    const LOOKAHEAD_SECS: f32 = 0.002;
    const RELEASE_SECS: f32 = 0.100;

    fn new(sample_rate: f32) -> Self {
        let mut l = Self {
            sample_rate,
            channels: 2,
            window: ((sample_rate * Self::LOOKAHEAD_SECS).round() as usize).max(2),
            release: (-1.0 / (Self::RELEASE_SECS * sample_rate)).exp(),
            delay: Default::default(),
            mins: Default::default(),
            avg: Default::default(),
            avg_sum: 0.0,
            deficit: 0.0,
            frame: 0,
            engaged: false,
        };
        l.reset(2);
        l
    }

    fn reset(&mut self, channels: usize) {
        self.channels = channels.max(1);
        self.delay.clear();
        self.delay.resize((self.window - 1) * self.channels, 0.0);
        self.mins.clear();
        self.avg.clear();
        self.avg.resize(self.window, 1.0);
        self.avg_sum = self.window as f64;
        self.deficit = 0.0;
        self.engaged = false;
    }

    /// Retune for a new track: a clean break, the previous envelope and
    /// delayed audio don't carry over.
    fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() > 0.1 {
            *self = Self::new(sample_rate);
        } else {
            self.reset(self.channels);
        }
    }

    fn process(&mut self, samples: &mut [f32], channels: u16) {
        let ch = channels.max(1) as usize;
        if ch != self.channels {
            self.reset(ch);
        }
        self.engaged = false;
        for frame in samples.chunks_exact_mut(ch) {
            let peak = frame.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
            let need = if peak > Self::CEILING {
                Self::CEILING / peak
            } else {
                1.0
            };

            // Sliding minimum over the last `window` frames.
            while self.mins.back().is_some_and(|&(_, g)| g >= need) {
                self.mins.pop_back();
            }
            self.mins.push_back((self.frame, need));
            while self
                .mins
                .front()
                .is_some_and(|&(i, _)| i + self.window as u64 <= self.frame)
            {
                self.mins.pop_front();
            }
            let wmin = self.mins.front().map_or(1.0, |&(_, g)| g);

            // Release toward unity, never above the window minimum. Snap
            // to exactly 1.0 so the idle path stays bit-exact.
            let mut deficit = self.deficit * self.release;
            if deficit < 1e-6 {
                deficit = 0.0;
            }
            self.deficit = deficit.max(1.0 - wmin);
            let held = 1.0 - self.deficit;

            // Moving average → linear attack ramp.
            self.avg_sum += held as f64 - self.avg.pop_front().unwrap_or(1.0) as f64;
            self.avg.push_back(held);
            let gain = if self.avg_sum >= self.window as f64 {
                1.0
            } else {
                (self.avg_sum / self.window as f64) as f32
            };
            self.engaged |= gain < 1.0;

            for s in frame.iter_mut() {
                self.delay.push_back(*s);
                let out = self.delay.pop_front().unwrap_or(0.0);
                *s = if gain < 1.0 { out * gain } else { out };
            }
            self.frame += 1;
        }
    }
}

/// Stereo balance, Winamp-style: unity at center (bit-exact no-op),
/// panning attenuates only the opposite side linearly down to silence.
/// The v1.2 constant-power law kept both sides at −3 dB even at center.
fn apply_balance(samples: &mut [f32], channels: u16, balance: f32) {
    if channels != 2 || balance == 0.0 {
        return;
    }
    let b = balance.clamp(-1.0, 1.0);
    let (left_gain, right_gain) = if b > 0.0 {
        (1.0 - b, 1.0)
    } else {
        (1.0, 1.0 + b)
    };
    let (frames, rest) = samples.as_chunks_mut::<2>();
    for chunk in frames {
        chunk[0] *= left_gain;
        chunk[1] *= right_gain;
    }
    // Odd-length stereo buffer: treat the orphan as centered.
    if let [last] = rest {
        *last *= (left_gain * right_gain).sqrt();
    }
}

/// Point every rate-dependent stage at a new track's sample rate and
/// drop the previous track's filter state.
fn retune_chain(
    rate: f32,
    equalizer: &Mutex<Equalizer>,
    loudness: &mut LoudnessFilter,
    limiter: &mut PeakLimiter,
    meter: &mut PeakRmsMeter,
    volume: f32,
) {
    if let Ok(mut eq) = equalizer.lock() {
        eq.set_sample_rate(rate);
        eq.reset_state();
    }
    limiter.set_sample_rate(rate);
    loudness.set_sample_rate(rate);
    loudness.set_for_volume(volume);
    meter.set_sample_rate(rate);
}

/// Audio playback state
struct PlaybackState {
    player: SymphoniaPlayer,
    output: AudioOutput,
    is_paused: bool,
    /// Decoded audio to play before asking the decoder for more — the
    /// incoming track's leftover after a crossfade swap.
    carry: Vec<f32>,
}

/// A track preloaded by `AudioCommand::QueueNext`. Held alongside the active
/// playback so end-of-stream can swap it in without rebuilding the output
/// device — the no-gap path. Only swapped if its sample rate and channel
/// count match the live output; otherwise we drop it and fall back to the
/// standard `RequestNext` rebuild.
struct PendingNext {
    player: SymphoniaPlayer,
    track: TrackInfo,
    /// Incoming audio decoded during a crossfade but not mixed yet.
    /// Packet sizes differ between tracks (1152-frame MP3 vs 4096-frame
    /// FLAC), so this FIFO keeps the surplus instead of dropping it.
    carry: Vec<f32>,
    /// Path used as a dedupe key — the app may resend `QueueNext` every
    /// frame as the current track approaches its end.
    path: PathBuf,
}

/// Main audio thread function using Symphonia + cpal.
///
/// The `spectrum_pub` / `waveform_pub` ArcSwaps are wait-free publish
/// targets — the audio thread `store()`s a fresh `Arc<…>` on each
/// visualization tick, the UI reads via `load_full()`. v1 sent these
/// over the event channel, which would queue forever if the UI was
/// throttled.
pub fn audio_thread_main_symphonia(
    command_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    spectrum_pub: Arc<ArcSwap<Vec<f32>>>,
    waveform_pub: Arc<ArcSwap<WaveformSnapshot>>,
    meter_pub: Arc<ArcSwap<MeterSnapshot>>,
) -> Result<()> {
    let mut playback: Option<PlaybackState> = None;
    let mut current_track: Option<TrackInfo> = None;
    let mut next_pending: Option<PendingNext> = None;
    // Set when the running playback was opened from an HTTP URL. Holds
    // the wait-free `StreamTitle` snapshot the `HttpStream` publishes
    // ICY metadata into; the position-update tick re-reads it and
    // forwards changes as `AudioEvent::IcyMetadata` so the UI never
    // touches the audio thread directly. Cleared on Stop / Play /
    // PlayUrl so it always reflects the *current* stream.
    let mut current_icy: Option<crate::http_stream::IcySnapshot> = None;
    let mut last_icy_title: String = String::new();
    // Reconnect snapshot parallel to `current_icy`. Same wait-free
    // poll model — we forward transitions between Connected /
    // Reconnecting / Failed up as `AudioEvent::StreamReconnect*` so the
    // UI can surface a toast without ever touching the audio thread.
    let mut current_reconnect: Option<crate::http_stream::ReconnectSnapshot> = None;
    let mut last_reconnect_state: crate::http_stream::ReconnectState =
        crate::http_stream::ReconnectState::Connected;

    // Create equalizer (shared between audio processing and command handling)
    let equalizer = Arc::new(Mutex::new(Equalizer::new(44100.0)));

    // Pre-buffer brickwall limiter — see PeakLimiter docs for rationale.
    // Sample rate gets updated on every track load (it's a no-op when
    // the rate matches), so the initial 44.1 kHz here is just a
    // placeholder for the silent-startup period.
    let mut limiter = PeakLimiter::new(44100.0);

    // Volume-dependent loudness compensation. Disabled by default;
    // the user opts in via SetLoudnessEnabled. Sample rate tracks
    // the live device alongside `limiter` and the EQ.
    let mut loudness = LoudnessFilter::new(44100.0);

    // Peak / RMS meter — see `PeakRmsMeter`. Runs on the post-limiter
    // signal so the bars match what the device receives.
    let mut meter = PeakRmsMeter::new(44100.0);

    // Capture buffer: one full FFT window of stereo PCM (interleaved
    // L/R, so `CAPTURE_FRAMES * 2` samples). Doubles as the source for
    // the oscilloscope, which consumes `WAVEFORM_FRAMES = FFT_SIZE / 2`
    // frames from it and uses the leading half as zero-crossing search
    // space.
    const CAPTURE_SAMPLES: usize = CAPTURE_FRAMES * 2;
    let capture_buffer = Arc::new(Mutex::new(AudioCaptureBuffer::new(CAPTURE_SAMPLES)));
    let capture_buffer_clone = capture_buffer.clone();
    // Scratch buffer the snapshot copies into. Allocated once so the
    // visualization tick doesn't allocate on the hot path.
    let mut vis_scratch = vec![0.0_f32; CAPTURE_SAMPLES];

    // Initialize audio engine state
    let mut engine_state = AudioEngineState::default();

    // Throttle position updates to reduce allocations
    let mut last_position_update = std::time::Instant::now();
    let position_update_interval = Duration::from_millis(100);

    // Throttle spectrum updates (~30 fps is plenty for a Winamp-style analyzer)
    let mut last_spectrum_update = std::time::Instant::now();
    let spectrum_update_interval = Duration::from_millis(33);
    let mut fft_planner = FftPlanner::<f32>::new();
    let fft = fft_planner.plan_fft_forward(FFT_SIZE);

    loop {
        // Wait briefly for the next command, OR for the timeout to
        // fire so we can refill the audio buffer. v1 busy-looped at
        // 1 ms intervals via `thread::sleep` — this thread parked
        // ~1000 times/sec even at idle. `recv_timeout` parks the
        // thread efficiently and wakes immediately on a command, so
        // idle CPU drops to ~0 % and command latency stays at <1 ms
        // under load. 5 ms is well under the audio buffer's 0.5 s
        // headroom — refill never falls behind.
        if let Ok(cmd) = command_rx.recv_timeout(Duration::from_millis(5)) {
            match cmd {
                AudioCommand::QueueNext(path) => {
                    // Skip if this exact path is already queued or already
                    // playing — the app may keep resending QueueNext as the
                    // current track approaches its end.
                    let already_queued = next_pending.as_ref().is_some_and(|p| p.path == path);
                    let already_playing = current_track.as_ref().is_some_and(|t| t.path == path);
                    if !already_queued && !already_playing && !engine_state.stop_after_current {
                        // load_for_preload: do NOT touch the equalizer's
                        // sample rate. Changing it now would corrupt the
                        // currently-playing track's filter state. We only
                        // adjust the rate at swap time, and then only if
                        // it actually differs from the live one.
                        match (TrackInfo::from_file(&path), SymphoniaPlayer::load(&path)) {
                            (Ok(track), Ok(player)) => {
                                next_pending = Some(PendingNext {
                                    player,
                                    track,
                                    carry: Vec::new(),
                                    path,
                                });
                            }
                            _ => {
                                // Preload failed — drop silently. EOS will
                                // fall back to the standard RequestNext path.
                                next_pending = None;
                            }
                        }
                    }
                }
                AudioCommand::PlayUrl(url) => {
                    // Tear down any active playback / queue / ICY
                    // publisher before we open the new stream.
                    engine_state.stop_after_current = false;
                    playback = None;
                    next_pending = None;
                    current_icy = None;
                    last_icy_title.clear();
                    current_reconnect = None;
                    last_reconnect_state = crate::http_stream::ReconnectState::Connected;
                    engine_state.track_gain_db = 0.0;

                    match crate::http_stream::HttpStream::open(&url) {
                        Ok(stream) => {
                            let icy_handle = stream.icy_title_handle();
                            let reconnect_handle = stream.reconnect_state_handle();
                            // Pick a hint extension from the response's
                            // `content-type` header so symphonia
                            // short-circuits codec sniffing — a plain
                            // `audio/mpeg` shoutcast stream is
                            // unambiguously MP3 even without a `.mp3`
                            // URL suffix.
                            let ext_hint = stream.content_type().and_then(|ct| {
                                crate::http_stream::HttpStream::extension_from_content_type(ct)
                            });
                            // Hand the stream to symphonia. The
                            // SymphoniaPlayer + RodioOutput pair is the
                            // same plumbing the file path uses; the
                            // only branch is the stream's origin.
                            match SymphoniaPlayer::load_from_source(Box::new(stream), ext_hint) {
                                Ok(player) => {
                                    let sr = player.sample_rate();
                                    let ch = player.channels();
                                    let output_res = AudioOutput::new_with_device(
                                        sr,
                                        ch,
                                        engine_state.output_device_name.as_deref(),
                                    );
                                    match output_res {
                                        Ok(output) => {
                                            // Build a stand-in `TrackInfo`
                                            // — we don't have a path or
                                            // duration, but the UI still
                                            // wants codec/sample-rate to
                                            // surface in the readout.
                                            let track_info = TrackInfo {
                                                path: PathBuf::from(&url),
                                                title: Some(url.clone()),
                                                artist: None,
                                                album: None,
                                                duration_secs: None,
                                                sample_rate: Some(sr),
                                                channels: Some(ch as u8),
                                                codec: Some("HTTP STREAM".to_string()),
                                                bitrate: None,
                                                tracknumber: None,
                                                year: None,
                                                genre: None,
                                                replaygain_track_gain_db: None,
                                                replaygain_album_gain_db: None,
                                            };
                                            current_track = Some(track_info.clone());
                                            let _ =
                                                event_tx.send(AudioEvent::TrackLoaded(track_info));

                                            output.set_volume(engine_state.device_volume());
                                            retune_chain(
                                                output.sample_rate() as f32,
                                                &equalizer,
                                                &mut loudness,
                                                &mut limiter,
                                                &mut meter,
                                                engine_state.volume,
                                            );
                                            playback = Some(PlaybackState {
                                                player,
                                                output,
                                                is_paused: false,
                                                carry: Vec::new(),
                                            });
                                            current_icy = Some(icy_handle);
                                            current_reconnect = Some(reconnect_handle);
                                            last_reconnect_state =
                                                crate::http_stream::ReconnectState::Connected;
                                            let _ = event_tx.send(AudioEvent::Playing);
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(AudioEvent::Error(format!(
                                                "Audio device init failed: {}",
                                                e
                                            )));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AudioEvent::Error(format!(
                                        "Stream decode setup failed: {}",
                                        e
                                    )));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(AudioEvent::Error(format!("Stream open failed: {}", e)));
                        }
                    }
                }
                AudioCommand::Play(path) => {
                    // Stop current playback. A user-initiated Play
                    // invalidates any preloaded next track — the new path
                    // may have nothing to do with what was queued.
                    engine_state.stop_after_current = false;
                    playback = None;
                    next_pending = None;
                    current_icy = None;
                    last_icy_title.clear();
                    current_reconnect = None;
                    last_reconnect_state = crate::http_stream::ReconnectState::Connected;

                    // Load track metadata
                    match TrackInfo::from_file(&path) {
                        Ok(track_info) => {
                            // Snapshot the track's ReplayGain into engine
                            // state. We resolve to zero when RG is disabled
                            // or the file isn't tagged — it's cheaper to
                            // keep one effective number than to gate every
                            // sample on a bool branch. The user's preamp
                            // setting stays untouched.
                            engine_state.track_gain_db =
                                engine_state.resolve_replaygain_db(&track_info);
                            current_track = Some(track_info.clone());
                            let _ = event_tx.send(AudioEvent::TrackLoaded(track_info));

                            // Load and play the file
                            match load_and_play(&path, engine_state.output_device_name.as_deref()) {
                                Ok(state) => {
                                    // Apply current volume and mute state
                                    state.output.set_volume(engine_state.device_volume());
                                    // Track-rate update so the release
                                    // time constant matches the cpal
                                    // device. Also clears the previous
                                    // track's envelope.
                                    retune_chain(
                                        state.output.sample_rate() as f32,
                                        &equalizer,
                                        &mut loudness,
                                        &mut limiter,
                                        &mut meter,
                                        engine_state.volume,
                                    );

                                    playback = Some(state);
                                    let _ = event_tx.send(AudioEvent::Playing);
                                }
                                Err(e) => {
                                    let _ = event_tx
                                        .send(AudioEvent::Error(format!("Failed to play: {}", e)));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(AudioEvent::Error(format!("Failed to load track: {}", e)));
                        }
                    }
                }
                AudioCommand::Pause => {
                    if let Some(ref mut state) = playback
                        && !state.is_paused
                    {
                        state.output.pause();
                        state.is_paused = true;
                        let _ = event_tx.send(AudioEvent::Paused);
                    }
                }
                AudioCommand::Resume => {
                    if let Some(ref mut state) = playback
                        && state.is_paused
                    {
                        state.output.play();
                        state.is_paused = false;
                        let _ = event_tx.send(AudioEvent::Playing);
                    }
                }
                AudioCommand::Stop => {
                    engine_state.stop_after_current = false;
                    playback = None;
                    current_track = None;
                    next_pending = None;
                    current_icy = None;
                    last_icy_title.clear();
                    current_reconnect = None;
                    last_reconnect_state = crate::http_stream::ReconnectState::Connected;
                    let _ = event_tx.send(AudioEvent::Stopped);
                }
                AudioCommand::Seek(pos) => {
                    if let Some(ref mut state) = playback {
                        // Clamp into [0, duration - 2s]. Symphonia sometimes
                        // refuses to seek into the last seconds of a stream
                        // and the failure leaves the format reader in a
                        // partially-advanced state where subsequent decodes
                        // also error — so we keep a generous tail buffer.
                        let duration = current_track.as_ref().and_then(|t| t.duration_secs);
                        let max_pos = duration.map(|d| (d - 2.0).max(0.0)).unwrap_or(pos);
                        let safe_pos = pos.clamp(0.0, max_pos);

                        // Try the requested position first; on failure step
                        // back toward 0. ALWAYS clear the output and reset
                        // the decoder via Player::seek (which calls
                        // decoder.reset() internally on success). If every
                        // attempt fails, we surface Stopped so the UI can
                        // recover instead of leaving the audio thread in a
                        // zombie state.
                        let mut seek_ok = false;
                        for attempt in [safe_pos, (safe_pos - 5.0).max(0.0), 0.0] {
                            match state.player.seek(attempt) {
                                Ok(()) => {
                                    state.output.clear();
                                    // Same rate: just flushes the
                                    // lookahead delay line.
                                    limiter.set_sample_rate(state.player.sample_rate() as f32);
                                    seek_ok = true;
                                    break;
                                }
                                Err(e) => {
                                    eprintln!(
                                        "[seek] attempt {:.2}s failed: {} — retrying",
                                        attempt, e
                                    );
                                }
                            }
                        }

                        if seek_ok {
                            if !state.is_paused {
                                let _ = event_tx.send(AudioEvent::Playing);
                            }
                        } else {
                            // Every seek attempt failed. Symphonia may have
                            // advanced the decoder partway into a rejected
                            // seek, leaving it in a zombie state where even a
                            // plain decode_next would error on residue. Reset
                            // the decoder explicitly to a known-good state
                            // before tearing the playback down, so anything
                            // holding a reference (or a future reuse) sees a
                            // clean decoder rather than the half-advanced one.
                            eprintln!(
                                "[seek] all attempts failed; resetting decoder and stopping playback"
                            );
                            state.player.reset_decoder();
                            playback = None;
                            current_track = None;
                            let _ = event_tx.send(AudioEvent::Stopped);
                        }
                    }
                }
                AudioCommand::Next => {
                    // Stop current playback and request next track from GUI
                    playback = None;
                    current_track = None;
                    current_icy = None;
                    last_icy_title.clear();
                    current_reconnect = None;
                    last_reconnect_state = crate::http_stream::ReconnectState::Connected;
                    let _ = event_tx.send(AudioEvent::RequestNext);
                }
                AudioCommand::Previous => {
                    // Stop current playback and request previous track from GUI
                    playback = None;
                    current_track = None;
                    current_icy = None;
                    last_icy_title.clear();
                    current_reconnect = None;
                    last_reconnect_state = crate::http_stream::ReconnectState::Connected;
                    let _ = event_tx.send(AudioEvent::RequestPrevious);
                }
                AudioCommand::SetEqualizerEnabled(enabled) => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.set_enabled(enabled);
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::SetEqualizerBand(band_index, gain_db) => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.set_band_gain(band_index, gain_db);
                        let enabled = eq.is_enabled();
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::SetEqualizerBands(gains) => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.set_all_gains(&gains);
                        let enabled = eq.is_enabled();
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::ResetEqualizer => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.reset_all_bands();
                        let enabled = eq.is_enabled();
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::SetEqualizerPreamp(db) => {
                    // Match the band slider range; the EQ window clamps
                    // its visual to [-20, 20] dB. Going further would
                    // just clip the output without giving extra room.
                    engine_state.preamp_db = db.clamp(-crate::EQ_MAX_DB, crate::EQ_MAX_DB);
                    let _ =
                        event_tx.send(AudioEvent::EqualizerPreampUpdated(engine_state.preamp_db));
                }
                AudioCommand::SetCrossfade(enabled, duration_secs) => {
                    engine_state.crossfade_enabled = enabled;
                    // Clamp generously so a buggy UI can't request a
                    // 100-second fade and starve the queue. Keep the
                    // duration even when disabled so toggling back on
                    // restores the prior choice.
                    engine_state.crossfade_duration_secs = duration_secs.clamp(0.5, 30.0);
                }
                AudioCommand::SetVolume(volume) => {
                    engine_state.volume = volume.clamp(0.0, crate::MAX_VOLUME);
                    if let Some(ref mut state) = playback {
                        state.output.set_volume(engine_state.device_volume());
                    }
                    // Refresh the loudness comp curve to track the new
                    // volume. The filter no-ops cheaply when loudness
                    // is off (and the comp stays at 0 dB), so we can
                    // call this unconditionally.
                    loudness.set_for_volume(engine_state.volume);
                    let _ = event_tx.send(AudioEvent::VolumeUpdated(
                        engine_state.volume,
                        engine_state.muted,
                    ));
                }
                AudioCommand::SetBalance(balance) => {
                    engine_state.balance = balance.clamp(-1.0, 1.0);
                    let _ = event_tx.send(AudioEvent::BalanceUpdated(engine_state.balance));
                }
                AudioCommand::SetMute(muted) => {
                    engine_state.muted = muted;
                    if let Some(ref mut state) = playback {
                        state.output.set_volume(engine_state.device_volume());
                    }
                    let _ = event_tx.send(AudioEvent::VolumeUpdated(
                        engine_state.volume,
                        engine_state.muted,
                    ));
                }
                AudioCommand::SetRepeatMode(mode) => {
                    engine_state.repeat_mode = mode;
                    let _ = event_tx.send(AudioEvent::RepeatModeUpdated(mode));
                }
                AudioCommand::SetStopAfterCurrent(armed) => {
                    engine_state.stop_after_current = armed;
                    if armed {
                        // A preloaded next track would otherwise be
                        // crossfaded in or gapless-swapped at EOS.
                        next_pending = None;
                    }
                }
                AudioCommand::SetShuffle(enabled) => {
                    engine_state.shuffle_enabled = enabled;
                    let _ = event_tx.send(AudioEvent::ShuffleUpdated(enabled));
                }
                AudioCommand::SetOutputDevice(name) => {
                    engine_state.output_device_name = name.clone();
                    let _ = event_tx.send(AudioEvent::OutputDeviceUpdated(name));
                }
                AudioCommand::SetLoudnessEnabled(enabled) => {
                    engine_state.loudness_enabled = enabled;
                    loudness.set_enabled(enabled);
                    // Refresh comp curve so the toggle takes effect on
                    // the currently-playing track without waiting for
                    // the next SetVolume.
                    loudness.set_for_volume(engine_state.volume);
                    let _ = event_tx.send(AudioEvent::LoudnessUpdated(enabled));
                }
                AudioCommand::SetReplayGainEnabled(enabled) => {
                    // Backward-compat path: the boolean toggles the
                    // enable flag but leaves `replaygain_mode` alone, so
                    // `false` => Off and `true` => the current mode
                    // (default Track — identical to the historical
                    // behaviour). Mode itself is set via
                    // SetReplayGainMode.
                    engine_state.replaygain_enabled = enabled;
                    // Re-derive the active track's gain so the toggle takes
                    // effect on the currently-playing track without
                    // waiting for the next TrackLoaded.
                    engine_state.track_gain_db = current_track
                        .as_ref()
                        .map(|t| engine_state.resolve_replaygain_db(t))
                        .unwrap_or(0.0);
                    let _ = event_tx.send(AudioEvent::ReplayGainUpdated(enabled));
                }
                AudioCommand::SetReplayGainMode(mode) => {
                    // The mode supersedes the boolean toggle: selecting a
                    // mode other than Off implies "enabled", and Off
                    // implies "disabled" — so the legacy `enabled` flag
                    // stays consistent with whatever the UI exposes.
                    engine_state.replaygain_mode = mode;
                    engine_state.replaygain_enabled = mode != crate::ReplayGainMode::Off;
                    // Re-derive immediately so the mode change takes effect
                    // on the currently-playing track.
                    engine_state.track_gain_db = current_track
                        .as_ref()
                        .map(|t| engine_state.resolve_replaygain_db(t))
                        .unwrap_or(0.0);
                    let _ = event_tx.send(AudioEvent::ReplayGainUpdated(
                        engine_state.replaygain_enabled,
                    ));
                }
                AudioCommand::SetMono(enabled) => {
                    engine_state.mono_enabled = enabled;
                }
                AudioCommand::Shutdown => {
                    break;
                }
            }
        }

        // Decode and feed audio to output
        let mut end_of_stream = false;
        if let Some(ref mut state) = playback
            && !state.is_paused
        {
            // Decide whether this iteration is in the crossfade window.
            // Conditions: feature on, queued track exists, formats match
            // the live output, current track has a known duration, and
            // we're inside the last `crossfade_duration_secs`. A linear
            // ramp through `fade_pos` ∈ [0, 1] drives equal-power weights
            // (`sin/cos` of pos·π/2) so combined power stays at unity.
            let crossfade_state = if let (true, Some(pending)) =
                (engine_state.crossfade_enabled, next_pending.as_ref())
            {
                let same_format = pending.player.sample_rate() == state.player.sample_rate()
                    && pending.player.channels() == state.player.channels();
                let total_secs = current_track
                    .as_ref()
                    .and_then(|t| t.duration_secs)
                    .unwrap_or(0.0);
                let remaining = total_secs - state.player.current_position();
                if same_format
                    && total_secs > 0.0
                    && remaining > 0.0
                    && remaining < engine_state.crossfade_duration_secs
                {
                    let fade_pos = ((engine_state.crossfade_duration_secs - remaining)
                        / engine_state.crossfade_duration_secs)
                        .clamp(0.0, 1.0);
                    let fade_in = (fade_pos * std::f32::consts::FRAC_PI_2).sin();
                    let fade_out = ((1.0 - fade_pos) * std::f32::consts::FRAC_PI_2).sin();
                    Some((fade_in, fade_out))
                } else {
                    None
                }
            } else {
                None
            };

            // Check if output needs more data
            if state.output.needs_data() {
                let decoded = if state.carry.is_empty() {
                    state.player.decode_next()
                } else {
                    Ok(Some(std::mem::take(&mut state.carry)))
                };
                match decoded {
                    Ok(Some(mut samples)) => {
                        if !samples.is_empty() {
                            let channels = state.player.channels();
                            // Mix in the next track during the fade
                            // window. The incoming decoder runs ahead
                            // into `pending.carry` until it covers this
                            // chunk; the surplus stays queued and the
                            // EOS swap plays it first, so no incoming
                            // sample is lost at either seam.
                            let mut crossfading = false;
                            if let Some((fade_in, fade_out)) = crossfade_state
                                && let Some(pending) = next_pending.as_mut()
                            {
                                while pending.carry.len() < samples.len() {
                                    match pending.player.decode_next() {
                                        Ok(Some(more)) => pending.carry.extend_from_slice(&more),
                                        _ => break,
                                    }
                                }
                                let len = samples.len().min(pending.carry.len());
                                for s in samples.iter_mut() {
                                    *s *= fade_out;
                                }
                                for (s, inc) in samples.iter_mut().zip(pending.carry.drain(..len)) {
                                    *s += inc * fade_in;
                                }
                                crossfading = true;
                            }
                            // EQ runs here, after the mix, so the two
                            // crossfading decoders never interleave
                            // through one set of filter states.
                            let eq_transparent = match equalizer.lock() {
                                Ok(mut eq) => {
                                    match channels {
                                        1 => eq.process_mono_in_place(&mut samples),
                                        2 => eq.process_stereo_in_place(&mut samples),
                                        _ => {} // surround passes through
                                    }
                                    eq.is_transparent()
                                }
                                Err(_) => true,
                            };
                            if let Ok(mut buffer) = capture_buffer.lock() {
                                buffer.update(&samples, state.player.sample_rate(), channels);
                            }
                            // Loudness compensation, after EQ and before
                            // gain/balance/limiter.
                            let loudness_transparent = loudness.is_transparent();
                            loudness.process_in_place(&mut samples, channels);
                            // Mono downmix before gain/balance/limiter so
                            // panning still applies to the mono'd signal
                            // and the meter/limiter see the true output.
                            if engine_state.mono_enabled {
                                apply_mono_downmix(&mut samples, channels);
                            }
                            // Preamp + ReplayGain + the >100 % part of
                            // the volume, summed in dB, all ahead of the
                            // limiter.
                            let gain_db = engine_state.preamp_db
                                + engine_state.track_gain_db
                                + 20.0 * engine_state.boost().log10();
                            apply_preamp(&mut samples, gain_db);
                            apply_balance(&mut samples, channels, engine_state.balance);
                            // 0 dBFS lookahead limiter: a pure delay
                            // unless something above pushed past full
                            // scale.
                            limiter.process(&mut samples, channels);
                            meter.process(&samples, channels);
                            // Bit-perfect when nothing touched the
                            // samples and the device format holds the
                            // source depth: the output then skips dither.
                            let exact = !crossfading
                                && eq_transparent
                                && loudness_transparent
                                && !(engine_state.mono_enabled && channels == 2)
                                && gain_db.abs() < 1e-3
                                && (engine_state.balance == 0.0 || channels != 2)
                                && !limiter.engaged
                                && state.player.source_bits().is_some_and(|b| {
                                    state.output.device_bits().is_none_or(|d| b <= d)
                                });
                            state.output.write_samples(&samples, exact);
                        }
                    }
                    Ok(None) => {
                        // End of stream
                        end_of_stream = true;
                    }
                    Err(e) => {
                        eprintln!("Decode error: {}", e);
                        // Continue playback despite errors
                    }
                }
            }

            // Send position update (throttled)
            if last_position_update.elapsed() >= position_update_interval {
                if let Some(ref track) = current_track {
                    let current_pos = state.player.current_position();
                    let total_duration = track.duration_secs.unwrap_or(0.0);
                    let _ = event_tx.send(AudioEvent::Position(current_pos, total_duration));
                }
                // ICY title polling lives on the same tick. We poll
                // the wait-free snapshot rather than getting woken on
                // change — the audio thread already pulses every
                // 100 ms for position, and ICY updates per ~10 s on a
                // typical radio stream, so missing a frame here is
                // invisible.
                if let Some(handle) = current_icy.as_ref() {
                    let snap = handle.load_full();
                    if !snap.is_empty() && snap.as_str() != last_icy_title.as_str() {
                        last_icy_title = snap.as_str().to_string();
                        let _ = event_tx.send(AudioEvent::IcyMetadata(last_icy_title.clone()));
                    }
                }
                // Reconnect-state polling. Each transition between
                // Connected ↔ Reconnecting ↔ Failed is forwarded
                // exactly once — the snapshot itself is wait-free, but
                // the equality check against `last_reconnect_state`
                // means a stable "Connected" tick doesn't spam events.
                if let Some(handle) = current_reconnect.as_ref() {
                    let snap = *handle.load_full();
                    if snap != last_reconnect_state {
                        last_reconnect_state = snap;
                        match snap {
                            crate::http_stream::ReconnectState::Reconnecting { attempt } => {
                                let _ = event_tx.send(AudioEvent::StreamReconnecting { attempt });
                            }
                            crate::http_stream::ReconnectState::Connected => {
                                let _ = event_tx.send(AudioEvent::StreamReconnected);
                            }
                            crate::http_stream::ReconnectState::Failed => {
                                let _ = event_tx.send(AudioEvent::StreamReconnectFailed);
                            }
                        }
                    }
                }
                last_position_update = std::time::Instant::now();
            }

            // Compute and send spectrum + waveform data (throttled). One
            // lock per refresh: snapshot into `vis_scratch`, drop the
            // guard, then compute outside the lock so the producer side
            // (decoder writes) isn't blocked by FFT/binning work.
            if last_spectrum_update.elapsed() >= spectrum_update_interval {
                let snapshot_ok = if let Ok(buffer) = capture_buffer_clone.lock() {
                    buffer.snapshot_into(&mut vis_scratch);
                    true
                } else {
                    false
                };
                if snapshot_ok {
                    // Wait-free publish: UI threads just `load_full()`
                    // off the ArcSwap. v1 pushed these through the
                    // event channel; a paused / throttled UI piled up
                    // unread events until memory pressure bit. The
                    // ArcSwap path keeps memory bounded at one Arc per
                    // refresh and never blocks the audio thread on
                    // consumer state.
                    let bins = compute_spectrum(&vis_scratch, fft.as_ref());
                    spectrum_pub.store(Arc::new(bins));
                    let waveform = compute_waveform(&vis_scratch);
                    waveform_pub.store(Arc::new(waveform));
                    // Peak/RMS meter snapshot — independent of the
                    // spectrum FFT, but cadenced the same so all three
                    // visu channels refresh in lock-step (~30 Hz).
                    meter_pub.store(Arc::new(meter.snapshot()));
                }
                last_spectrum_update = std::time::Instant::now();
            }
        }

        // Handle end of stream outside the borrow
        if end_of_stream && engine_state.stop_after_current {
            // "Stop after current" beats repeat and gapless: halt here
            // and let the app's `Finished` handler do the UI side.
            engine_state.stop_after_current = false;
            next_pending = None;
            playback = None;
            current_track = None;
            let _ = event_tx.send(AudioEvent::Finished);
        } else if end_of_stream {
            match engine_state.repeat_mode {
                RepeatMode::One => {
                    // Restart current track if it exists. RepeatMode::One
                    // means "keep replaying this track", so any preloaded
                    // next is moot — drop it.
                    next_pending = None;
                    if let Some(ref track) = current_track {
                        match load_and_play(&track.path, engine_state.output_device_name.as_deref())
                        {
                            Ok(state) => {
                                // Apply current volume and mute state
                                state.output.set_volume(engine_state.device_volume());
                                retune_chain(
                                    state.output.sample_rate() as f32,
                                    &equalizer,
                                    &mut loudness,
                                    &mut limiter,
                                    &mut meter,
                                    engine_state.volume,
                                );

                                playback = Some(state);
                                let _ = event_tx.send(AudioEvent::Playing);
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::Error(format!(
                                    "Failed to restart track: {}",
                                    e
                                )));
                                playback = None;
                                let _ = event_tx.send(AudioEvent::Finished);
                            }
                        }
                    } else {
                        playback = None;
                        let _ = event_tx.send(AudioEvent::Finished);
                    }
                }
                RepeatMode::All | RepeatMode::Off => {
                    // Try the gapless swap path. The output keeps
                    // consuming from its existing buffer (~0.5 s queued)
                    // while we replace the decoder, so the device never
                    // starves — no audible gap. Falls back to the
                    // RequestNext/Finished path if no track is queued or
                    // its format doesn't match the live output.
                    let live_format = playback
                        .as_ref()
                        .map(|s| (s.output.sample_rate(), s.output.channels()));
                    let swap = match (next_pending.take(), live_format) {
                        (Some(pending), Some((sr, ch)))
                            if pending.player.sample_rate() == sr
                                && pending.player.channels() == ch =>
                        {
                            Some(pending)
                        }
                        // Format mismatch (or no pending). Drop the pending
                        // player — its decoder + reader would otherwise
                        // leak until the next QueueNext.
                        _ => None,
                    };

                    match (swap, playback.as_mut()) {
                        (Some(pending), Some(state)) => {
                            state.player = pending.player;
                            state.carry = pending.carry;
                            // Gapless swap brings in a new track — refresh
                            // the cached ReplayGain so the next sample batch
                            // is normalized against the new file's tag.
                            engine_state.track_gain_db =
                                engine_state.resolve_replaygain_db(&pending.track);
                            current_track = Some(pending.track.clone());
                            let _ = event_tx.send(AudioEvent::TrackLoaded(pending.track));
                            let _ = event_tx.send(AudioEvent::Playing);
                        }
                        _ => {
                            playback = None;
                            match engine_state.repeat_mode {
                                RepeatMode::All => {
                                    let _ = event_tx.send(AudioEvent::RequestNext);
                                }
                                RepeatMode::Off => {
                                    current_track = None;
                                    let _ = event_tx.send(AudioEvent::Finished);
                                }
                                RepeatMode::One => unreachable!(),
                            }
                        }
                    }
                }
            }
        }

        // No tail sleep — recv_timeout above already parks the thread
        // for up to 5 ms while waiting for the next command.
    }

    Ok(())
}

/// Load and start playing an audio file. `device_name` selects the cpal
/// output device; `None` falls back to the host default.
fn load_and_play(path: &Path, device_name: Option<&str>) -> Result<PlaybackState> {
    let player = SymphoniaPlayer::load(path).context("Failed to load audio file")?;

    let output = AudioOutput::new_with_device(player.sample_rate(), player.channels(), device_name)
        .context("Failed to create audio output")?;

    Ok(PlaybackState {
        player,
        output,
        is_paused: false,
        carry: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustfft::FftPlanner;

    fn stereo_sine(freq_hz: f32, sample_rate: f32, frames: usize, amp: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        let two_pi = std::f32::consts::TAU;
        for n in 0..frames {
            let s = amp * (two_pi * freq_hz * n as f32 / sample_rate).sin();
            out.push(s);
            out.push(s);
        }
        out
    }

    #[test]
    fn compute_spectrum_returns_16_bins() {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let samples = stereo_sine(1000.0, 44100.0, FFT_SIZE, 0.5);
        let bins = compute_spectrum(&samples, fft.as_ref());
        assert_eq!(bins.len(), SPECTRUM_BINS);
        assert_eq!(SPECTRUM_BINS, 16);
        for &v in &bins {
            assert!((0.0..=1.0).contains(&v), "bin out of range: {}", v);
        }
    }

    #[test]
    fn compute_spectrum_dc_offset_doesnt_saturate_low_bin() {
        // Pure DC: 1.0 sustained. v1 would have parked bin 0/1 at 1.0;
        // after detrending it should be near zero everywhere.
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let dc: Vec<f32> = vec![1.0; FFT_SIZE * 2];
        let bins = compute_spectrum(&dc, fft.as_ref());
        for (i, &v) in bins.iter().enumerate() {
            assert!(
                v < 0.05,
                "DC input should produce near-zero spectrum, bin {} = {}",
                i,
                v
            );
        }
    }

    #[test]
    fn compute_spectrum_full_scale_sine_reads_near_zero_db() {
        // A full-scale 1 kHz sine should peak its containing bin at the
        // top of the display range (≈1.0, i.e. 0 dBFS on the [-60, 0]
        // scale).
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let samples = stereo_sine(1000.0, 44100.0, FFT_SIZE, 1.0);
        let bins = compute_spectrum(&samples, fft.as_ref());
        let max = bins.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            max > 0.85,
            "full-scale sine should light up ≥0.85 on the [-60, 0] dB display, got max {}",
            max
        );
    }

    #[test]
    fn compute_spectrum_silence_reads_zero() {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let silence = vec![0.0_f32; FFT_SIZE * 2];
        let bins = compute_spectrum(&silence, fft.as_ref());
        for &v in &bins {
            assert_eq!(v, 0.0, "silence should produce a zero spectrum");
        }
    }

    #[test]
    fn compute_waveform_shape_is_well_formed() {
        let samples = stereo_sine(440.0, 44100.0, FFT_SIZE, 0.7);
        let snap = compute_waveform(&samples);
        assert_eq!(snap.mins.len(), WAVEFORM_COLUMNS);
        assert_eq!(snap.maxs.len(), WAVEFORM_COLUMNS);
        for i in 0..WAVEFORM_COLUMNS {
            assert!(
                snap.mins[i] <= snap.maxs[i],
                "min must not exceed max at col {}: {} > {}",
                i,
                snap.mins[i],
                snap.maxs[i]
            );
        }
    }

    #[test]
    fn compute_waveform_silence_is_all_zeros() {
        let silence = vec![0.0_f32; FFT_SIZE * 2];
        let snap = compute_waveform(&silence);
        assert!(snap.mins.iter().all(|&v| v == 0.0));
        assert!(snap.maxs.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn compute_waveform_captures_full_amplitude_via_min_max() {
        // A full-scale 8 kHz sine at 44.1 kHz has ~5.5 samples per
        // period — far less than one period per column at
        // WAVEFORM_FRAMES / WAVEFORM_COLUMNS = 4 samples per column. The
        // nearest-neighbor decimation (v1) would have picked one sample
        // per column and showed an alias; min/max decimation must
        // preserve the ±1.0 excursion on most columns.
        let samples = stereo_sine(8000.0, 44100.0, FFT_SIZE, 1.0);
        let snap = compute_waveform(&samples);
        let max_amp = snap.maxs.iter().copied().fold(0.0_f32, f32::max);
        let min_amp = snap.mins.iter().copied().fold(0.0_f32, f32::min);
        assert!(max_amp > 0.95, "lost positive excursion: {}", max_amp);
        assert!(min_amp < -0.95, "lost negative excursion: {}", min_amp);
    }

    #[test]
    fn meter_peak_captures_max_abs_across_frames() {
        let mut m = PeakRmsMeter::new(44100.0);
        let samples = [0.2_f32, -0.4, 0.9, -0.1, 0.5, 0.5];
        m.process(&samples, 2);
        let snap = m.snapshot();
        // L peaks: |0.2|, |0.9|, |0.5| → 0.9
        // R peaks: |-0.4|, |-0.1|, |0.5| → 0.5
        assert!((snap.peak_l - 0.9).abs() < 1e-5, "peak_l = {}", snap.peak_l);
        assert!((snap.peak_r - 0.5).abs() < 1e-5, "peak_r = {}", snap.peak_r);
    }

    #[test]
    fn meter_snapshot_resets_peak_but_keeps_rms() {
        let mut m = PeakRmsMeter::new(44100.0);
        let samples: Vec<f32> = (0..2000).map(|_| 0.7_f32).collect();
        m.process(&samples, 2);
        let first = m.snapshot();
        // Second snapshot on quiet input — peak should reset to 0,
        // RMS should still reflect the long preceding loud signal.
        m.process(&[0.0_f32; 2], 2);
        let second = m.snapshot();
        assert_eq!(second.peak_l, 0.0);
        assert_eq!(second.peak_r, 0.0);
        // RMS persists (decaying smoothly, but still > 0 a few ms in).
        assert!(second.rms_l > 0.1, "rms_l = {}", second.rms_l);
        // First snapshot's peak should reflect the loud input.
        assert!(first.peak_l > 0.69 && first.peak_l < 0.71);
    }

    #[test]
    fn meter_rms_converges_to_steady_state_for_sine() {
        // RMS of a full-scale sine ≈ 1/√2 ≈ 0.707. With τ = 300 ms,
        // after ~1.5 s (5 time constants) the smoother should be
        // within 1% of that target.
        let mut m = PeakRmsMeter::new(44100.0);
        let sr = 44100.0_f32;
        let freq = 1000.0_f32;
        let frames = (sr * 1.5) as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let s = (std::f32::consts::TAU * freq * n as f32 / sr).sin();
            samples.push(s);
            samples.push(s);
        }
        m.process(&samples, 2);
        let snap = m.snapshot();
        let expected = 1.0 / std::f32::consts::SQRT_2;
        assert!(
            (snap.rms_l - expected).abs() < 0.02,
            "RMS should converge to ≈ {} after 1.5s, got {}",
            expected,
            snap.rms_l
        );
    }

    #[test]
    fn loudness_curve_is_zero_at_full_volume() {
        let c = LoudnessFilter::comp_db_for_volume(1.0);
        assert!(c.abs() < 1e-4, "expected 0 dB at volume=1.0, got {}", c);
    }

    #[test]
    fn loudness_curve_caps_at_max_at_quiet_volume() {
        let c = LoudnessFilter::comp_db_for_volume(0.05);
        assert!(
            (c - LoudnessFilter::MAX_COMP_DB).abs() < 0.01,
            "expected clamp at MAX_COMP_DB for very quiet volume, got {}",
            c
        );
    }

    #[test]
    fn loudness_curve_grows_with_attenuation() {
        let c_loud = LoudnessFilter::comp_db_for_volume(0.8);
        let c_mid = LoudnessFilter::comp_db_for_volume(0.4);
        let c_quiet = LoudnessFilter::comp_db_for_volume(0.2);
        assert!(c_loud < c_mid);
        assert!(c_mid < c_quiet);
    }

    #[test]
    fn loudness_filter_disabled_is_passthrough() {
        let mut f = LoudnessFilter::new(44100.0);
        // Default: enabled = false, comp = 0.
        let mut samples = [0.5_f32, -0.3, 0.7, -0.7];
        let original = samples;
        f.process_in_place(&mut samples, 2);
        assert_eq!(samples, original);
    }

    #[test]
    fn loudness_filter_boosts_when_enabled_at_low_volume() {
        // Enable + set comp from a quiet volume. Inspect the shelf
        // magnitudes via the BiquadFilter `target_magnitude` accessor
        // — proves the filter is actually configured (no need to run
        // PCM through it and reverse-engineer the dB).
        let mut f = LoudnessFilter::new(44100.0);
        f.set_enabled(true);
        f.set_for_volume(0.3);
        let tau = std::f32::consts::TAU;
        // Low shelf at 50 Hz should boost noticeably.
        let low_db = 20.0 * f.low_shelf.target_magnitude(tau * 50.0 / 44100.0).log10();
        // High shelf at 10 kHz should boost noticeably.
        let high_db = 20.0
            * f.high_shelf
                .target_magnitude(tau * 10000.0 / 44100.0)
                .log10();
        assert!(
            low_db > 1.5 && low_db < LoudnessFilter::MAX_COMP_DB + 0.5,
            "expected ~3 dB at 50 Hz when volume=0.3, got {}",
            low_db
        );
        assert!(
            high_db > 1.5 && high_db < LoudnessFilter::MAX_COMP_DB + 0.5,
            "expected ~3 dB at 10 kHz when volume=0.3, got {}",
            high_db
        );
    }

    #[test]
    fn apply_balance_center_is_bit_exact() {
        let mut s = vec![0.5_f32, -0.25, 0.123_456_7, -0.987_654_3];
        let orig = s.clone();
        apply_balance(&mut s, 2, 0.0);
        assert_eq!(s, orig);
    }

    #[test]
    fn apply_balance_full_left_silences_right_and_keeps_left() {
        let mut s = vec![0.5_f32, 0.5];
        apply_balance(&mut s, 2, -1.0);
        assert_eq!(s, vec![0.5, 0.0]);
    }

    #[test]
    fn apply_balance_full_right_silences_left_and_keeps_right() {
        let mut s = vec![0.5_f32, 0.5];
        apply_balance(&mut s, 2, 1.0);
        assert_eq!(s, vec![0.0, 0.5]);
    }

    #[test]
    fn apply_balance_only_attenuates_opposite_side() {
        let mut s = vec![1.0_f32, 1.0];
        apply_balance(&mut s, 2, 0.25);
        assert_eq!(s, vec![0.75, 1.0]);
    }

    /// Run `input` through the limiter and drop the lookahead delay so
    /// output index n lines up with input index n.
    fn limit_aligned(lim: &mut PeakLimiter, input: &[f32]) -> Vec<f32> {
        let delay = (lim.window - 1) * 2;
        let mut buf = input.to_vec();
        buf.extend(std::iter::repeat_n(0.0, delay));
        lim.process(&mut buf, 2);
        buf[delay..].to_vec()
    }

    #[test]
    fn limiter_is_bit_exact_within_full_scale() {
        let mut lim = PeakLimiter::new(44100.0);
        let input = stereo_sine(1000.0, 44100.0, 4096, 0.999);
        let out = limit_aligned(&mut lim, &input);
        assert_eq!(out, input);
        assert!(!lim.engaged);
    }

    #[test]
    fn limiter_never_exceeds_ceiling() {
        let mut lim = PeakLimiter::new(44100.0);
        // +6 dB over full scale, with a sudden jump the lookahead must
        // catch before it leaves the delay line.
        let mut input = stereo_sine(200.0, 44100.0, 2048, 0.3);
        input.extend(stereo_sine(3000.0, 44100.0, 4096, 2.0));
        let out = limit_aligned(&mut lim, &input);
        let max = out.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!(max <= PeakLimiter::CEILING + 1e-6, "peak {max}");
    }

    #[test]
    fn limiter_gain_moves_without_steps() {
        // A single over should be approached by a ramp: consecutive
        // output gains never jump by more than 1/window of the reduction.
        let mut lim = PeakLimiter::new(44100.0);
        let mut input = vec![0.5_f32; 2 * 1000];
        input[2 * 500] = 2.0;
        input[2 * 500 + 1] = 2.0;
        let out = limit_aligned(&mut lim, &input);
        let gains: Vec<f32> = (0..1000)
            .filter(|&n| n != 500)
            .map(|n| out[2 * n] / 0.5)
            .collect();
        let max_step = gains
            .windows(2)
            .fold(0.0_f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(
            max_step <= 0.5 / lim.window as f32 + 1e-4,
            "step {max_step}"
        );
        assert!(out[2 * 500] <= 1.0 + 1e-6);
    }

    #[test]
    fn limiter_releases_back_to_unity() {
        let mut lim = PeakLimiter::new(44100.0);
        let mut input = stereo_sine(1000.0, 44100.0, 441, 2.0);
        input.extend(stereo_sine(1000.0, 44100.0, 3 * 44100, 0.5));
        let out = limit_aligned(&mut lim, &input);
        let tail = &out[out.len() - 2000..];
        let src = &input[input.len() - 2000..];
        assert_eq!(tail, src, "gain should have released to exactly 1.0");
    }

    #[test]
    fn mono_downmix_averages_stereo_into_both_channels() {
        let mut samples = vec![1.0_f32, 0.0, -0.4, 0.4, 0.5, 0.5];
        apply_mono_downmix(&mut samples, 2);
        // Each frame becomes (L+R)/2 in both slots.
        assert_eq!(samples, vec![0.5, 0.5, 0.0, 0.0, 0.5, 0.5]);
    }

    #[test]
    fn mono_downmix_is_noop_on_mono_input() {
        let mut samples = vec![0.1_f32, -0.2, 0.3];
        let original = samples.clone();
        apply_mono_downmix(&mut samples, 1);
        assert_eq!(samples, original);
    }

    #[test]
    fn find_rising_zero_crossing_picks_first_transition() {
        // negative → positive in slot 4
        let buf = [-0.5, -0.3, -0.1, -0.05, 0.2, 0.4];
        assert_eq!(find_rising_zero_crossing(&buf, buf.len()), 4);
    }

    #[test]
    fn find_rising_zero_crossing_returns_zero_on_no_transition() {
        let buf = [-0.5, -0.4, -0.3, -0.2, -0.1];
        assert_eq!(find_rising_zero_crossing(&buf, buf.len()), 0);
    }

    #[test]
    fn find_rising_zero_crossing_ignores_falling_edges() {
        let buf = [0.5, 0.2, -0.1, -0.3, -0.2, 0.1];
        // First rising edge is from -0.2 → 0.1 at index 5.
        assert_eq!(find_rising_zero_crossing(&buf, buf.len()), 5);
    }
}
