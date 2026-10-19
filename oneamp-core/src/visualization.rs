//! Analysis for the visualisations: 16-bin spectrum, oscilloscope and
//! peak/RMS meter. Computed on the engine thread from the post-EQ tap
//! and published wait-free; never on the real-time path.

use crate::{MeterSnapshot, WaveformSnapshot};
use rustfft::Fft;
use rustfft::num_complex::Complex32;

/// FFT window size. 2048 at 44.1 kHz = ~46 ms window, ~21 Hz bin width —
/// fine enough to distinguish the 31 Hz / 63 Hz EQ bands. v1 used 1024
/// (~43 Hz/bin) which conflated the two lowest bands.
pub(crate) const FFT_SIZE: usize = 2048;
/// Number of mono frames retained in the capture buffer (= one FFT
/// window). Stereo interleaved doubles this.
pub(crate) const CAPTURE_FRAMES: usize = FFT_SIZE;
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
pub(crate) fn compute_spectrum(frames: &[f32], fft: &dyn Fft<f32>) -> Vec<f32> {
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
pub(crate) fn compute_waveform(samples: &[f32]) -> WaveformSnapshot {
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

/// Per-channel peak + RMS metering. Peak is the simple max of |s|
/// since the last [`snapshot`] call (reset on snapshot — so each UI
/// frame sees a *short-term* peak); RMS is a one-pole smoothed
/// running mean-square with the configured time constant.
///
/// The smoother runs on the **post-limiter** signal so the meter
/// reflects what's actually heading to the device, not what the
/// decoder produced. v1 had no metering at all.
pub(crate) struct PeakRmsMeter {
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

    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            rms_alpha: (-1.0 / (Self::RMS_TIME_CONSTANT_SECS * sample_rate)).exp(),
            peak_l: 0.0,
            peak_r: 0.0,
            rms_sq_l: 0.0,
            rms_sq_r: 0.0,
        }
    }

    pub(crate) fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() < 0.1 {
            return;
        }
        self.sample_rate = sample_rate;
        self.rms_alpha = (-1.0 / (Self::RMS_TIME_CONSTANT_SECS * sample_rate)).exp();
        // Don't clear state — the meter quickly recovers anyway, and
        // a hard reset would be visible as a "drop to zero" spike.
    }

    pub(crate) fn process(&mut self, samples: &[f32], channels: u16) {
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
    pub(crate) fn snapshot(&mut self) -> MeterSnapshot {
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
