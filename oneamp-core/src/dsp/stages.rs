//! The individual stages of the processing chain. Each is
//! allocation-free once constructed and bit-exact when idle.

use crate::equalizer::BiquadFilter;

/// Length of every gain ramp (preamp, ReplayGain, balance, volume
/// boost): long enough that a change is click-free, short enough to feel
/// immediate.
const GAIN_RAMP_SECS: f32 = 0.010;

/// A linear gain ramped per frame toward its target. Exactly 1.0 when
/// idle at unity, so an untouched signal passes bit for bit.
#[derive(Debug, Clone)]
pub struct GainRamp {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp_frames: u32,
}

impl GainRamp {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            current: 1.0,
            target: 1.0,
            step: 0.0,
            remaining: 0,
            ramp_frames: Self::frames_for(sample_rate),
        }
    }

    fn frames_for(sample_rate: f32) -> u32 {
        ((sample_rate * GAIN_RAMP_SECS) as u32).max(1)
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.ramp_frames = Self::frames_for(sample_rate);
    }

    /// Ramp toward `gain` (linear).
    pub fn set(&mut self, gain: f32) {
        let gain = if gain.is_finite() { gain.max(0.0) } else { 1.0 };
        if gain == self.target {
            return;
        }
        self.target = gain;
        self.remaining = self.ramp_frames;
        self.step = (gain - self.current) / self.ramp_frames as f32;
    }

    /// Jump to `gain` with no ramp (new media, after a seek).
    pub fn snap(&mut self, gain: f32) {
        self.set(gain);
        self.current = self.target;
        self.remaining = 0;
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Unity with no ramp in flight: the stage changes nothing.
    pub fn is_identity(&self) -> bool {
        self.remaining == 0 && self.current == 1.0
    }

    pub fn process(&mut self, samples: &mut [f32], channels: usize) {
        if self.is_identity() {
            return;
        }
        let ch = channels.max(1);
        for frame in samples.chunks_mut(ch) {
            if self.remaining > 0 {
                self.remaining -= 1;
                self.current = if self.remaining == 0 {
                    self.target
                } else {
                    self.current + self.step
                };
            }
            for s in frame {
                *s *= self.current;
            }
        }
    }
}

/// Stereo balance, Winamp-style: unity at center (bit-exact no-op),
/// panning attenuates only the opposite side linearly down to silence.
/// Both sides ramp, so dragging the slider does not zipper.
#[derive(Debug, Clone)]
pub struct Balance {
    left: GainRamp,
    right: GainRamp,
}

impl Balance {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            left: GainRamp::new(sample_rate),
            right: GainRamp::new(sample_rate),
        }
    }

    pub fn gains(balance: f32) -> (f32, f32) {
        let b = if balance.is_finite() {
            balance.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        if b > 0.0 {
            (1.0 - b, 1.0)
        } else {
            (1.0, 1.0 + b)
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.left.set_sample_rate(sample_rate);
        self.right.set_sample_rate(sample_rate);
    }

    pub fn set(&mut self, balance: f32) {
        let (l, r) = Self::gains(balance);
        self.left.set(l);
        self.right.set(r);
    }

    pub fn snap(&mut self, balance: f32) {
        let (l, r) = Self::gains(balance);
        self.left.snap(l);
        self.right.snap(r);
    }

    pub fn is_identity(&self) -> bool {
        self.left.is_identity() && self.right.is_identity()
    }

    /// Stereo only; any other width passes through.
    pub fn process(&mut self, samples: &mut [f32], channels: usize) {
        if channels != 2 || self.is_identity() {
            return;
        }
        for frame in samples.as_chunks_mut::<2>().0 {
            let (mut l, mut r) = ([frame[0]], [frame[1]]);
            self.left.process(&mut l, 1);
            self.right.process(&mut r, 1);
            frame[0] = l[0];
            frame[1] = r[0];
        }
    }
}

/// Collapse interleaved stereo to mono in place: each L/R pair becomes
/// its average on both channels. No-op on anything but stereo.
pub fn mono_downmix(samples: &mut [f32], channels: usize) {
    if channels != 2 {
        return;
    }
    for chunk in samples.as_chunks_mut::<2>().0 {
        let m = 0.5 * (chunk[0] + chunk[1]);
        chunk[0] = m;
        chunk[1] = m;
    }
}

/// "Loudness" compensation à la Fletcher-Munson / ISO 226. At volume 1.0
/// it is a no-op; as volume drops the low and high shelves rise so the
/// perceived tonal balance stays roughly constant.
///
/// `comp_db = ((-20·log10(volume)) · 0.3).clamp(0, 6)`: ≈ 1.8 dB at 0.5,
/// 3.1 dB at 0.3, 6 dB at 0.1.
#[derive(Debug, Clone)]
pub struct Loudness {
    sample_rate: f32,
    enabled: bool,
    comp_db: f32,
    pub(crate) low_shelf: BiquadFilter,
    pub(crate) high_shelf: BiquadFilter,
    bypassed: bool,
}

impl Loudness {
    const LOW_FREQ_HZ: f32 = 120.0;
    const HIGH_FREQ_HZ: f32 = 4000.0;
    pub const MAX_COMP_DB: f32 = 6.0;
    const COMP_RATIO: f32 = 0.3;

    pub fn new(sample_rate: f32) -> Self {
        let mut f = Self {
            sample_rate,
            enabled: false,
            comp_db: 0.0,
            low_shelf: BiquadFilter::new(),
            high_shelf: BiquadFilter::new(),
            bypassed: true,
        };
        f.low_shelf
            .set_low_shelf_snap(sample_rate, Self::LOW_FREQ_HZ, 0.0);
        f.high_shelf
            .set_high_shelf_snap(sample_rate, Self::HIGH_FREQ_HZ, 0.0);
        f
    }

    pub fn comp_db_for_volume(volume: f32) -> f32 {
        let v = volume.clamp(0.001, 1.0);
        let attenuation = -20.0 * v.log10();
        (attenuation * Self::COMP_RATIO).clamp(0.0, Self::MAX_COMP_DB)
    }

    /// Enable/disable and follow `volume` in one step.
    pub fn configure(&mut self, enabled: bool, volume: f32) {
        self.enabled = enabled;
        let target = if enabled {
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

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() < 0.1 {
            return;
        }
        self.sample_rate = sample_rate;
        self.low_shelf
            .set_low_shelf_snap(sample_rate, Self::LOW_FREQ_HZ, self.comp_db);
        self.high_shelf
            .set_high_shelf_snap(sample_rate, Self::HIGH_FREQ_HZ, self.comp_db);
    }

    pub fn reset(&mut self) {
        self.low_shelf.reset();
        self.high_shelf.reset();
        self.low_shelf.snap_to_target();
        self.high_shelf.snap_to_target();
    }

    /// No compensation and no ramp in flight: skipped entirely (0 dB
    /// shelves are unity in theory, not bit-exact in float).
    pub fn is_transparent(&self) -> bool {
        self.comp_db.abs() < 1e-3 && !self.low_shelf.ramp_active() && !self.high_shelf.ramp_active()
    }

    /// Mono or stereo; wider buffers pass through (the chain folds them
    /// to stereo before this stage).
    pub fn process(&mut self, samples: &mut [f32], channels: usize) {
        if self.is_transparent() {
            self.bypassed = true;
            return;
        }
        if self.bypassed {
            self.reset();
            self.bypassed = false;
        }
        if channels == 2 {
            for chunk in samples.as_chunks_mut::<2>().0 {
                let (l, r) = self.low_shelf.process_stereo(chunk[0], chunk[1]);
                let (l, r) = self.high_shelf.process_stereo(l, r);
                chunk[0] = l;
                chunk[1] = r;
            }
        } else if channels == 1 {
            for s in samples.iter_mut() {
                let v = self.low_shelf.process_mono(*s);
                *s = self.high_shelf.process_mono(v);
            }
        }
    }
}

/// Lookahead brickwall limiter with a 0 dBFS ceiling.
///
/// Transparent (bit-exact, only delayed by the lookahead) for any signal
/// within full scale. Gain computer: per-frame required gain
/// `min(1, ceiling / peak)`, sliding minimum over the lookahead window,
/// one-pole release, then a moving average over the same window — every
/// gain drop becomes a linear ramp that finishes by the time the peak
/// leaves the delay line. Works on any channel count.
#[derive(Debug, Clone)]
pub struct Limiter {
    sample_rate: f32,
    channels: usize,
    pub(crate) window: usize,
    release: f32,
    delay: std::collections::VecDeque<f32>,
    mins: std::collections::VecDeque<(u64, f32)>,
    avg: std::collections::VecDeque<f32>,
    avg_sum: f64,
    /// `1 - released gain` (a gain stored near 1.0 in f32 stalls).
    deficit: f32,
    frame: u64,
    /// Whether the last `process` call attenuated anything.
    pub engaged: bool,
}

impl Limiter {
    pub const CEILING: f32 = 1.0;
    const LOOKAHEAD_SECS: f32 = 0.002;
    const RELEASE_SECS: f32 = 0.100;

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let window = ((sample_rate * Self::LOOKAHEAD_SECS).round() as usize).max(2);
        let mut l = Self {
            sample_rate,
            channels: channels.max(1),
            window,
            release: (-1.0 / (Self::RELEASE_SECS * sample_rate)).exp(),
            delay: Default::default(),
            mins: std::collections::VecDeque::with_capacity(window + 1),
            avg: Default::default(),
            avg_sum: 0.0,
            deficit: 0.0,
            frame: 0,
            engaged: false,
        };
        l.reset();
        l
    }

    /// Clear the envelope and the delay line (seek, new media).
    pub fn reset(&mut self) {
        self.delay.clear();
        self.delay.resize((self.window - 1) * self.channels, 0.0);
        self.mins.clear();
        self.avg.clear();
        self.avg.resize(self.window, 1.0);
        self.avg_sum = self.window as f64;
        self.deficit = 0.0;
        self.engaged = false;
    }

    /// Re-target a rate / width. A clean break: nothing carries over.
    pub fn configure(&mut self, sample_rate: f32, channels: usize) {
        if (self.sample_rate - sample_rate).abs() > 0.1 || channels.max(1) != self.channels {
            *self = Self::new(sample_rate, channels);
        } else {
            self.reset();
        }
    }

    /// Frames the signal is delayed by.
    pub fn latency_frames(&self) -> usize {
        self.window - 1
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        let ch = self.channels;
        self.engaged = false;
        for frame in samples.chunks_exact_mut(ch) {
            let peak = frame.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
            let need = if peak > Self::CEILING {
                Self::CEILING / peak
            } else {
                1.0
            };
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

            let mut deficit = self.deficit * self.release;
            if deficit < 1e-6 {
                deficit = 0.0;
            }
            self.deficit = deficit.max(1.0 - wmin);
            let held = 1.0 - self.deficit;

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

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn stereo_sine(freq_hz: f32, sample_rate: f32, frames: usize, amp: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let s = amp * (std::f32::consts::TAU * freq_hz * n as f32 / sample_rate).sin();
                [s, s]
            })
            .collect()
    }

    fn limit_aligned(lim: &mut Limiter, input: &[f32]) -> Vec<f32> {
        let delay = lim.latency_frames() * 2;
        let mut buf = input.to_vec();
        buf.extend(std::iter::repeat_n(0.0, delay));
        lim.process(&mut buf);
        buf[delay..].to_vec()
    }

    #[test]
    fn limiter_is_bit_exact_within_full_scale() {
        let mut lim = Limiter::new(44100.0, 2);
        let input = stereo_sine(1000.0, 44100.0, 4096, 0.999);
        assert_eq!(limit_aligned(&mut lim, &input), input);
        assert!(!lim.engaged);
    }

    #[test]
    fn limiter_never_exceeds_ceiling() {
        let mut lim = Limiter::new(44100.0, 2);
        let mut input = stereo_sine(200.0, 44100.0, 2048, 0.3);
        input.extend(stereo_sine(3000.0, 44100.0, 4096, 2.0));
        let out = limit_aligned(&mut lim, &input);
        let max = out.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!(max <= Limiter::CEILING + 1e-6, "peak {max}");
    }

    #[test]
    fn limiter_gain_moves_without_steps() {
        let mut lim = Limiter::new(44100.0, 2);
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
        let mut lim = Limiter::new(44100.0, 2);
        let mut input = stereo_sine(1000.0, 44100.0, 441, 2.0);
        input.extend(stereo_sine(1000.0, 44100.0, 3 * 44100, 0.5));
        let out = limit_aligned(&mut lim, &input);
        assert_eq!(
            &out[out.len() - 2000..],
            &input[input.len() - 2000..],
            "gain should have released to exactly 1.0"
        );
    }

    #[test]
    fn limiter_handles_six_channels() {
        let mut lim = Limiter::new(48000.0, 6);
        let mut buf = vec![1.5_f32; 6 * 2000];
        lim.process(&mut buf);
        assert!(buf.iter().all(|s| s.abs() <= 1.0 + 1e-6));
    }

    #[test]
    fn gain_ramp_is_identity_at_unity_and_reaches_target_smoothly() {
        let mut g = GainRamp::new(1000.0); // 10-frame ramp
        let mut buf = vec![0.5_f32; 4];
        g.process(&mut buf, 2);
        assert_eq!(buf, vec![0.5; 4]);
        g.set(0.5);
        let mut buf = vec![1.0_f32; 30];
        g.process(&mut buf, 1);
        // Monotone, no jump bigger than one ramp step, lands exactly.
        for w in buf.windows(2) {
            assert!(w[1] <= w[0] && w[0] - w[1] <= 0.05 + 1e-6);
        }
        assert_eq!(buf[29], 0.5);
        g.snap(1.0);
        assert!(g.is_identity());
    }

    #[test]
    fn balance_center_is_bit_exact_and_sides_attenuate() {
        let mut b = Balance::new(44100.0);
        let mut s = vec![0.5_f32, -0.25, 0.123_456_7, -0.987_654_3];
        let orig = s.clone();
        b.process(&mut s, 2);
        assert_eq!(s, orig);
        b.snap(-1.0);
        let mut s = vec![0.5_f32, 0.5];
        b.process(&mut s, 2);
        assert_eq!(s, vec![0.5, 0.0]);
        b.snap(0.25);
        let mut s = vec![1.0_f32, 1.0];
        b.process(&mut s, 2);
        assert_eq!(s, vec![0.75, 1.0]);
    }

    #[test]
    fn mono_downmix_averages_stereo() {
        let mut samples = vec![1.0_f32, 0.0, -0.4, 0.4];
        mono_downmix(&mut samples, 2);
        assert_eq!(samples, vec![0.5, 0.5, 0.0, 0.0]);
        let mut m = vec![0.1_f32, -0.2];
        mono_downmix(&mut m, 1);
        assert_eq!(m, vec![0.1, -0.2]);
    }

    #[test]
    fn loudness_curve_shape() {
        assert!(Loudness::comp_db_for_volume(1.0).abs() < 1e-4);
        assert!((Loudness::comp_db_for_volume(0.05) - Loudness::MAX_COMP_DB).abs() < 0.01);
        assert!(Loudness::comp_db_for_volume(0.8) < Loudness::comp_db_for_volume(0.4));
    }

    #[test]
    fn loudness_disabled_is_passthrough_and_boosts_when_quiet() {
        let mut f = Loudness::new(44100.0);
        let mut samples = [0.5_f32, -0.3, 0.7, -0.7];
        let original = samples;
        f.process(&mut samples, 2);
        assert_eq!(samples, original);
        f.configure(true, 0.3);
        let tau = std::f32::consts::TAU;
        let low_db = 20.0 * f.low_shelf.target_magnitude(tau * 50.0 / 44100.0).log10();
        let high_db = 20.0
            * f.high_shelf
                .target_magnitude(tau * 10000.0 / 44100.0)
                .log10();
        assert!(low_db > 1.5 && low_db < Loudness::MAX_COMP_DB + 0.5);
        assert!(high_db > 1.5 && high_db < Loudness::MAX_COMP_DB + 0.5);
    }
}
