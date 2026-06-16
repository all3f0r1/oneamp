use std::f32::consts::PI;

/// Linear ramp duration applied to biquad coefficients when a band's
/// configuration changes. ~10 ms is long enough to be inaudible as a
/// click, short enough not to feel laggy on slider drags. At 44.1 kHz
/// this is 441 samples; at 192 kHz it's 1920.
const COEF_RAMP_SECS: f32 = 0.010;

/// 10-band graphic EQ center frequencies, in Hz. ISO 266 / IEC 61260
/// octave series. Used as both the band labels and the actual peaking
/// filter centers — re-exported from `equalizer_presets` so existing
/// callers don't break.
pub const EQ_FREQUENCIES: [f32; 10] = [
    31.5,    // Sub-bass
    63.0,    // Bass
    125.0,   // Low bass
    250.0,   // Low midrange
    500.0,   // Midrange
    1000.0,  // Midrange
    2000.0,  // Upper midrange
    4000.0,  // Presence
    8000.0,  // Brilliance
    16000.0, // Air
];

/// Biquad filter implementation for audio equalization.
///
/// Based on Robert Bristow-Johnson's Audio EQ Cookbook. Coefficient
/// updates are smoothed over [`COEF_RAMP_SECS`] to avoid the "zipper
/// noise" (sample-to-sample discontinuity) that bare assignment
/// produces on slider drags. At any given sample the *active*
/// coefficients (`b0..a2`) are between the previous and `target_*`,
/// stepping linearly by `step_*` until `ramp_remaining` hits zero.
#[derive(Debug, Clone)]
pub struct BiquadFilter {
    // Active coefficients applied to the next sample.
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,

    // Target coefficients the active set ramps toward.
    target_b0: f32,
    target_b1: f32,
    target_b2: f32,
    target_a1: f32,
    target_a2: f32,

    // Per-sample increment toward target. Zero when no ramp is in flight.
    step_b0: f32,
    step_b1: f32,
    step_b2: f32,
    step_a1: f32,
    step_a2: f32,

    /// Samples left in the current ramp. When this hits zero, the active
    /// coefficients are snapped to the targets to eliminate float drift.
    ramp_remaining: u32,

    // IIR state for the left channel.
    x1_l: f32,
    x2_l: f32,
    y1_l: f32,
    y2_l: f32,

    // IIR state for the right channel.
    x1_r: f32,
    x2_r: f32,
    y1_r: f32,
    y2_r: f32,
}

impl BiquadFilter {
    /// Create a new biquad filter with neutral coefficients (pass-through).
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            target_b0: 1.0,
            target_b1: 0.0,
            target_b2: 0.0,
            target_a1: 0.0,
            target_a2: 0.0,
            step_b0: 0.0,
            step_b1: 0.0,
            step_b2: 0.0,
            step_a1: 0.0,
            step_a2: 0.0,
            ramp_remaining: 0,
            x1_l: 0.0,
            x2_l: 0.0,
            y1_l: 0.0,
            y2_l: 0.0,
            x1_r: 0.0,
            x2_r: 0.0,
            y1_r: 0.0,
            y2_r: 0.0,
        }
    }

    /// Compute low-shelf coefficients (RBJ cookbook). `S=1` is baked
    /// in — the "max slope at corner" preset gives the gentlest knee
    /// for a loudness-style filter without ringing.
    fn compute_low_shelf(sample_rate: f32, frequency: f32, gain_db: f32) -> [f32; 5] {
        let a = 10_f32.powf(gain_db / 40.0);
        let omega = 2.0 * PI * frequency / sample_rate;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        // With S=1: alpha = sin_w / sqrt(2).
        let alpha = sin_w / std::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let ap1 = a + 1.0;
        let am1 = a - 1.0;

        let b0 = a * (ap1 - am1 * cos_w + two_sqrt_a_alpha);
        let b1 = 2.0 * a * (am1 - ap1 * cos_w);
        let b2 = a * (ap1 - am1 * cos_w - two_sqrt_a_alpha);
        let a0 = ap1 + am1 * cos_w + two_sqrt_a_alpha;
        let a1 = -2.0 * (am1 + ap1 * cos_w);
        let a2 = ap1 + am1 * cos_w - two_sqrt_a_alpha;

        [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
    }

    /// Compute high-shelf coefficients (RBJ cookbook). Same S=1 default.
    fn compute_high_shelf(sample_rate: f32, frequency: f32, gain_db: f32) -> [f32; 5] {
        let a = 10_f32.powf(gain_db / 40.0);
        let omega = 2.0 * PI * frequency / sample_rate;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let alpha = sin_w / std::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let ap1 = a + 1.0;
        let am1 = a - 1.0;

        let b0 = a * (ap1 + am1 * cos_w + two_sqrt_a_alpha);
        let b1 = -2.0 * a * (am1 + ap1 * cos_w);
        let b2 = a * (ap1 + am1 * cos_w - two_sqrt_a_alpha);
        let a0 = ap1 - am1 * cos_w + two_sqrt_a_alpha;
        let a1 = 2.0 * (am1 - ap1 * cos_w);
        let a2 = ap1 - am1 * cos_w - two_sqrt_a_alpha;

        [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
    }

    /// Helper that stages a target / step / ramp_remaining update from a
    /// freshly computed coefficient set. Used by every public setter so
    /// the ramping policy stays consistent across filter shapes.
    fn arm_ramp(&mut self, target: [f32; 5], sample_rate: f32) {
        self.target_b0 = target[0];
        self.target_b1 = target[1];
        self.target_b2 = target[2];
        self.target_a1 = target[3];
        self.target_a2 = target[4];

        let ramp = ((sample_rate * COEF_RAMP_SECS).round() as u32).max(1);
        let inv = (ramp as f32).recip();
        self.step_b0 = (self.target_b0 - self.b0) * inv;
        self.step_b1 = (self.target_b1 - self.b1) * inv;
        self.step_b2 = (self.target_b2 - self.b2) * inv;
        self.step_a1 = (self.target_a1 - self.a1) * inv;
        self.step_a2 = (self.target_a2 - self.a2) * inv;
        self.ramp_remaining = ramp;
    }

    /// Helper for snap setters: write target = active = `target` and
    /// clear the ramp.
    fn snap_to(&mut self, target: [f32; 5]) {
        self.b0 = target[0];
        self.b1 = target[1];
        self.b2 = target[2];
        self.a1 = target[3];
        self.a2 = target[4];
        self.target_b0 = target[0];
        self.target_b1 = target[1];
        self.target_b2 = target[2];
        self.target_a1 = target[3];
        self.target_a2 = target[4];
        self.step_b0 = 0.0;
        self.step_b1 = 0.0;
        self.step_b2 = 0.0;
        self.step_a1 = 0.0;
        self.step_a2 = 0.0;
        self.ramp_remaining = 0;
    }

    /// Configure as a low-shelf filter (boosts/cuts frequencies below
    /// `frequency`). Smooth ramp over [`COEF_RAMP_SECS`].
    pub fn set_low_shelf(&mut self, sample_rate: f32, frequency: f32, gain_db: f32) {
        let target = Self::compute_low_shelf(sample_rate, frequency, gain_db);
        self.arm_ramp(target, sample_rate);
    }

    /// Snap-variant of [`set_low_shelf`]. No ramp.
    pub fn set_low_shelf_snap(&mut self, sample_rate: f32, frequency: f32, gain_db: f32) {
        let target = Self::compute_low_shelf(sample_rate, frequency, gain_db);
        self.snap_to(target);
    }

    /// Configure as a high-shelf filter (boosts/cuts frequencies above
    /// `frequency`). Smooth ramp over [`COEF_RAMP_SECS`].
    pub fn set_high_shelf(&mut self, sample_rate: f32, frequency: f32, gain_db: f32) {
        let target = Self::compute_high_shelf(sample_rate, frequency, gain_db);
        self.arm_ramp(target, sample_rate);
    }

    /// Snap-variant of [`set_high_shelf`]. No ramp.
    pub fn set_high_shelf_snap(&mut self, sample_rate: f32, frequency: f32, gain_db: f32) {
        let target = Self::compute_high_shelf(sample_rate, frequency, gain_db);
        self.snap_to(target);
    }

    /// Compute peaking-EQ coefficients (RBJ cookbook) without touching
    /// state. Returns `(b0, b1, b2, a1, a2)` already normalized by `a0`.
    fn compute_peaking_eq(sample_rate: f32, frequency: f32, gain_db: f32, q: f32) -> [f32; 5] {
        let a = 10_f32.powf(gain_db / 40.0);
        let omega = 2.0 * PI * frequency / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_omega;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha / a;

        [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
    }

    /// Configure as a peaking EQ filter, smoothly ramping the active
    /// coefficients toward the new target over [`COEF_RAMP_SECS`].
    pub fn set_peaking_eq(&mut self, sample_rate: f32, frequency: f32, gain_db: f32, q: f32) {
        let target = Self::compute_peaking_eq(sample_rate, frequency, gain_db, q);
        self.arm_ramp(target, sample_rate);
    }

    /// Same as [`set_peaking_eq`] but jumps to the target immediately.
    /// Use at construction / sample-rate change — places where the
    /// listener isn't yet hearing audio, so the ~10 ms ramp would just
    /// be lost setup time.
    pub fn set_peaking_eq_snap(&mut self, sample_rate: f32, frequency: f32, gain_db: f32, q: f32) {
        let target = Self::compute_peaking_eq(sample_rate, frequency, gain_db, q);
        self.snap_to(target);
    }

    /// Advance the coefficient ramp by one sample. No-op when the ramp
    /// has completed. Inlined so the per-sample cost is one branch when
    /// idle.
    #[inline(always)]
    fn tick_coefs(&mut self) {
        if self.ramp_remaining == 0 {
            return;
        }
        self.b0 += self.step_b0;
        self.b1 += self.step_b1;
        self.b2 += self.step_b2;
        self.a1 += self.step_a1;
        self.a2 += self.step_a2;
        self.ramp_remaining -= 1;
        if self.ramp_remaining == 0 {
            // Snap to the exact target — accumulated float error during
            // the ramp would otherwise leave the active coefs slightly
            // off, which is harmless audibly but messes with tests that
            // assert exact convergence.
            self.b0 = self.target_b0;
            self.b1 = self.target_b1;
            self.b2 = self.target_b2;
            self.a1 = self.target_a1;
            self.a2 = self.target_a2;
        }
    }

    /// Process a stereo sample pair.
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.tick_coefs();

        let left_out = self.b0 * left + self.b1 * self.x1_l + self.b2 * self.x2_l
            - self.a1 * self.y1_l
            - self.a2 * self.y2_l;
        self.x2_l = self.x1_l;
        self.x1_l = left;
        self.y2_l = self.y1_l;
        self.y1_l = left_out;

        let right_out = self.b0 * right + self.b1 * self.x1_r + self.b2 * self.x2_r
            - self.a1 * self.y1_r
            - self.a2 * self.y2_r;
        self.x2_r = self.x1_r;
        self.x1_r = right;
        self.y2_r = self.y1_r;
        self.y1_r = right_out;

        (left_out, right_out)
    }

    /// Process a single mono sample, advancing *only* the left-channel
    /// IIR state. Lets `Equalizer::process_mono_in_place` run a true
    /// single-state filter instead of the v1 dup-to-stereo workaround,
    /// which silently kept the right-channel history in lockstep — fine
    /// audibly but wasteful and a source of subtle state leaks across
    /// channel-count changes.
    pub fn process_mono(&mut self, sample: f32) -> f32 {
        self.tick_coefs();

        let out = self.b0 * sample + self.b1 * self.x1_l + self.b2 * self.x2_l
            - self.a1 * self.y1_l
            - self.a2 * self.y2_l;
        self.x2_l = self.x1_l;
        self.x1_l = sample;
        self.y2_l = self.y1_l;
        self.y1_l = out;
        out
    }

    /// `true` while the coefficient ramp is still converging toward
    /// the most-recently-requested target. Lets callers (e.g. the
    /// loudness filter) decide whether the biquad is safe to
    /// short-circuit out of the audio path.
    pub fn ramp_active(&self) -> bool {
        self.ramp_remaining > 0
    }

    /// `true` when the *target* coefficients are exactly pass-through
    /// — useful in tandem with [`ramp_active`] to distinguish "filter
    /// is doing nothing" from "filter has just been asked to do
    /// something and is still settling".
    pub fn target_is_unity(&self) -> bool {
        (self.target_b0 - 1.0).abs() < 1e-3
            && self.target_b1.abs() < 1e-3
            && self.target_b2.abs() < 1e-3
            && self.target_a1.abs() < 1e-3
            && self.target_a2.abs() < 1e-3
    }

    /// Reset filter state (useful when changing tracks)
    pub fn reset(&mut self) {
        self.x1_l = 0.0;
        self.x2_l = 0.0;
        self.y1_l = 0.0;
        self.y2_l = 0.0;
        self.x1_r = 0.0;
        self.x2_r = 0.0;
        self.y1_r = 0.0;
        self.y2_r = 0.0;
    }

    /// Magnitude of this biquad's frequency response at `omega = 2π·f/sr`,
    /// evaluated from the *target* coefficients so that callers don't need
    /// to wait for the ramp to complete. Returns the linear |H(e^jω)|.
    pub(crate) fn target_magnitude(&self, omega: f32) -> f32 {
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let cos_2w = (2.0 * omega).cos();
        let sin_2w = (2.0 * omega).sin();
        let num_re = self.target_b0 + self.target_b1 * cos_w + self.target_b2 * cos_2w;
        let num_im = -self.target_b1 * sin_w - self.target_b2 * sin_2w;
        let den_re = 1.0 + self.target_a1 * cos_w + self.target_a2 * cos_2w;
        let den_im = -self.target_a1 * sin_w - self.target_a2 * sin_2w;
        let num_mag2 = num_re * num_re + num_im * num_im;
        let den_mag2 = den_re * den_re + den_im * den_im;
        (num_mag2 / den_mag2.max(1e-30)).sqrt()
    }
}

impl Default for BiquadFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// 10-band graphic equalizer
#[derive(Debug, Clone)]
pub struct Equalizer {
    /// Individual band filters
    bands: Vec<BiquadFilter>,
    /// Band frequencies in Hz
    frequencies: Vec<f32>,
    /// Band gains in dB (-12 to +12)
    gains: Vec<f32>,
    /// Current sample rate
    sample_rate: f32,
    /// Whether the equalizer is enabled
    enabled: bool,
}

impl Equalizer {
    /// Create a new 10-band equalizer
    pub fn new(sample_rate: f32) -> Self {
        let mut eq = Self {
            bands: vec![BiquadFilter::new(); 10],
            frequencies: EQ_FREQUENCIES.to_vec(),
            gains: vec![0.0; 10],
            sample_rate,
            enabled: false,
        };

        // Snap-initialize each filter so the first ~10 ms of playback
        // doesn't ramp from pass-through to peaking@0dB (the ramp would
        // be audibly identical thanks to the gain=0 symmetry of the
        // peaking filter, but tests prefer exact coefficients).
        eq.snap_filters();

        eq
    }

    /// Enable or disable the equalizer
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            // Reset all filters when disabling
            for band in &mut self.bands {
                band.reset();
            }
        }
    }

    /// Check if equalizer is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set gain for a specific band (0-9). Smoothly ramps the band's
    /// biquad coefficients over [`COEF_RAMP_SECS`] to avoid clicks
    /// when the user drags a slider.
    ///
    /// # Arguments
    /// * `band_index` - Band index (0-9)
    /// * `gain_db` - Gain in decibels (-20 to +20, matches Winamp's slider range)
    pub fn set_band_gain(&mut self, band_index: usize, gain_db: f32) {
        if band_index < self.gains.len() {
            self.gains[band_index] = gain_db.clamp(-20.0, 20.0);
            self.update_filter(band_index);
        } else {
            // Out-of-bounds index is a no-op in release builds (we must
            // never panic on the audio path), but a silent no-op hides
            // a caller bug. Surface it loudly in debug and log it either
            // way so the dropped write is visible.
            debug_assert!(
                band_index < self.gains.len(),
                "set_band_gain: band_index {} out of range (have {} bands)",
                band_index,
                self.gains.len()
            );
            eprintln!(
                "[equalizer] set_band_gain ignored: band_index {} out of range (have {} bands)",
                band_index,
                self.gains.len()
            );
        }
    }

    /// Get gain for a specific band
    pub fn get_band_gain(&self, band_index: usize) -> f32 {
        self.gains.get(band_index).copied().unwrap_or(0.0)
    }

    /// Get all band gains
    pub fn get_all_gains(&self) -> &[f32] {
        &self.gains
    }

    /// Set all band gains at once. Smoothly ramps each band toward its
    /// new gain over [`COEF_RAMP_SECS`].
    pub fn set_all_gains(&mut self, gains: &[f32]) {
        for (i, &gain) in gains.iter().enumerate().take(self.gains.len()) {
            self.gains[i] = gain.clamp(-20.0, 20.0);
        }
        self.update_filters();
    }

    /// Reset all bands to 0 dB (flat response). Drops the user's
    /// per-band settings; leaves enabled/sample-rate intact. The
    /// transition to flat is smoothed.
    pub fn reset_all_bands(&mut self) {
        for gain in &mut self.gains {
            *gain = 0.0;
        }
        self.update_filters();
    }

    /// Reset internal biquad state (x/y history) on every band without
    /// touching gain, frequency, or enabled state. Call this on track
    /// boundaries / format changes so filter memory from a previous
    /// stream can't bleed into the new one's first packets — especially
    /// relevant when going stereo → mono → stereo, where the unused R
    /// channel state would otherwise carry a stale tail.
    pub fn reset_state(&mut self) {
        for band in &mut self.bands {
            band.reset();
        }
    }

    /// Get band frequencies
    pub fn get_frequencies(&self) -> &[f32] {
        &self.frequencies
    }

    /// Constant-Q policy for a graphic EQ: Q grows with the magnitude
    /// of the gain so heavy boosts / cuts stay focused instead of
    /// bleeding into adjacent bands. Symmetric in boost/cut via the
    /// `abs()` on `gain_db`. At 0 dB the Q is gentle (~0.707), at
    /// ±20 dB it tightens to ~1.07.
    ///
    /// Formula: `Q = √2 / (1 + 1/A)`, with `A = 10^(|gain_db|/40)`.
    /// At 0 dB the peaking filter is mathematically pass-through
    /// regardless of Q, so the gentle 0 dB Q only matters as a starting
    /// point for ramps toward non-zero gain — the audible character of
    /// the EQ comes from the high-gain end of the curve.
    fn q_for_gain(gain_db: f32) -> f32 {
        let mag = 10_f32.powf(gain_db.abs() / 40.0);
        std::f32::consts::SQRT_2 / (1.0 + 1.0 / mag)
    }

    /// Smooth update of a single filter's coefficients.
    fn update_filter(&mut self, band_index: usize) {
        if band_index < self.bands.len() {
            let gain = self.gains[band_index];
            self.bands[band_index].set_peaking_eq(
                self.sample_rate,
                self.frequencies[band_index],
                gain,
                Self::q_for_gain(gain),
            );
        }
    }

    /// Smooth update of every filter's coefficients.
    fn update_filters(&mut self) {
        for i in 0..self.bands.len() {
            self.update_filter(i);
        }
    }

    /// Snap-update every filter without ramp — used at construction
    /// and on sample-rate change.
    fn snap_filters(&mut self) {
        for i in 0..self.bands.len() {
            let gain = self.gains[i];
            self.bands[i].set_peaking_eq_snap(
                self.sample_rate,
                self.frequencies[i],
                gain,
                Self::q_for_gain(gain),
            );
        }
    }

    /// Process a stereo sample through all bands. Kept as a per-sample
    /// API for callers that have one frame at a time (tests, single-shot
    /// processing); audio-thread bulk processing should use
    /// [`process_stereo_in_place`].
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        if !self.enabled {
            return (left, right);
        }

        let mut l = left;
        let mut r = right;
        for band in &mut self.bands {
            let (lo, ro) = band.process_stereo(l, r);
            l = lo;
            r = ro;
        }
        (l, r)
    }

    /// Process a buffer of interleaved stereo samples in place. Zero
    /// allocation per call — caller owns the buffer.
    pub fn process_stereo_in_place(&mut self, samples: &mut [f32]) {
        if !self.enabled {
            return;
        }
        for chunk in samples.chunks_exact_mut(2) {
            let mut l = chunk[0];
            let mut r = chunk[1];
            for band in &mut self.bands {
                let (lo, ro) = band.process_stereo(l, r);
                l = lo;
                r = ro;
            }
            chunk[0] = l;
            chunk[1] = r;
        }
    }

    /// Process a buffer of mono samples in place. Uses each band's
    /// dedicated `process_mono` path — only the left-channel IIR state
    /// advances, so a subsequent stereo track starts with a clean
    /// right-channel history (provided [`reset_state`] runs on track
    /// boundaries, which the player does).
    pub fn process_mono_in_place(&mut self, samples: &mut [f32]) {
        if !self.enabled {
            return;
        }
        for s in samples.iter_mut() {
            let mut x = *s;
            for band in &mut self.bands {
                x = band.process_mono(x);
            }
            *s = x;
        }
    }

    /// Update sample rate (call when track changes). Snaps the
    /// coefficients without ramp — a track boundary is the wrong place
    /// to slew through coefficient space, and the filter state has
    /// just been reset anyway.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() > 0.1 {
            self.sample_rate = sample_rate;
            self.snap_filters();
        }
    }

    /// Return the negative of the worst-case peak gain of the cascaded
    /// filter chain across the audio band, in dB. Useful to derive a
    /// preamp value that prevents the EQ from clipping a normalized
    /// signal.
    ///
    /// - Flat (all gains 0 dB) → 0 dB (no headroom needed).
    /// - One band at +12 dB → ≈ −12 dB (drop the preamp by that much
    ///   and the boosted band can hit unity without clipping).
    /// - Multiple boosts that interact constructively (adjacent bands)
    ///   → can exceed the per-band gain; this function captures that.
    ///
    /// Uses the **target** coefficients (not the live ones) so the
    /// answer is the steady-state response of whatever gains were last
    /// requested — independent of where the coefficient ramp currently
    /// sits. Sample rate dependence is small (the peaking filter is
    /// normalized in normalized frequency); we evaluate against the
    /// equalizer's current `sample_rate`.
    pub fn headroom_db(&self) -> f32 {
        // 4096 log-spaced probe points cover the audio band
        // densely enough that no peaking-EQ lobe sneaks between two
        // samples (worst case Q ≈ 1.4, -3 dB bandwidth ≈ 1 octave;
        // 4096 points across 10 octaves = >400 per octave).
        const PROBE_POINTS: usize = 4096;
        const F_MIN: f32 = 20.0;
        const F_MAX: f32 = 20000.0;

        let log_min = F_MIN.ln();
        let log_max = F_MAX.ln();
        let log_step = (log_max - log_min) / (PROBE_POINTS - 1) as f32;

        let mut peak_db = 0.0_f32;
        let two_pi_over_sr = std::f32::consts::TAU / self.sample_rate;

        for i in 0..PROBE_POINTS {
            let freq = (log_min + i as f32 * log_step).exp();
            // The peaking biquad is only meaningful up to Nyquist; clamp
            // probe frequencies to a hair below it so the magnitude
            // formula doesn't degenerate.
            let nyquist = self.sample_rate * 0.5;
            let probe = freq.min(nyquist - 1.0);
            let omega = probe * two_pi_over_sr;

            // Cascaded magnitude in dB = sum of per-band magnitudes in dB.
            let mut total_db = 0.0_f32;
            for band in &self.bands {
                let mag = band.target_magnitude(omega);
                total_db += 20.0 * mag.max(1e-10).log10();
            }
            if total_db > peak_db {
                peak_db = total_db;
            }
        }

        // Headroom = -peak, never below zero (no headroom needed for a
        // chain that doesn't boost anywhere).
        (-peak_db).min(0.0)
    }
}

impl Default for Equalizer {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_biquad_passthrough() {
        let mut filter = BiquadFilter::new();
        let (l, r) = filter.process_stereo(1.0, -1.0);
        assert!((l - 1.0).abs() < 0.001);
        assert!((r + 1.0).abs() < 0.001);
    }

    #[test]
    fn test_equalizer_disabled() {
        let mut eq = Equalizer::new(44100.0);
        eq.set_band_gain(0, 6.0);
        let (l, r) = eq.process_stereo(1.0, 1.0);
        // Should pass through unchanged when disabled
        assert!((l - 1.0).abs() < 0.001);
        assert!((r - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_equalizer_gain_clamping() {
        let mut eq = Equalizer::new(44100.0);
        eq.set_band_gain(0, 30.0); // Should clamp to +20.0
        assert_eq!(eq.get_band_gain(0), 20.0);
        eq.set_band_gain(1, -30.0); // Should clamp to -20.0
        assert_eq!(eq.get_band_gain(1), -20.0);
    }

    #[test]
    fn reset_state_zeros_biquad_history_without_touching_gains() {
        let mut eq = Equalizer::new(44100.0);
        eq.set_enabled(true);
        eq.set_band_gain(4, 12.0);

        // Drive the filter for longer than the coefficient ramp window
        // so the bands are in their target state and the IIR history
        // has fully populated.
        let mut samples = vec![0.0_f32; 2 * 1024];
        for chunk in samples.chunks_exact_mut(2) {
            chunk[0] = 1.0;
            chunk[1] = -1.0;
        }
        eq.process_stereo_in_place(&mut samples);

        for band in &eq.bands {
            assert!(
                band.x1_l != 0.0 || band.y1_l != 0.0 || band.x1_r != 0.0 || band.y1_r != 0.0,
                "biquad state should be non-zero after driving with signal"
            );
        }

        eq.reset_state();

        for band in &eq.bands {
            assert_eq!(band.x1_l, 0.0);
            assert_eq!(band.x2_l, 0.0);
            assert_eq!(band.y1_l, 0.0);
            assert_eq!(band.y2_l, 0.0);
            assert_eq!(band.x1_r, 0.0);
            assert_eq!(band.x2_r, 0.0);
            assert_eq!(band.y1_r, 0.0);
            assert_eq!(band.y2_r, 0.0);
        }
        assert_eq!(eq.get_band_gain(4), 12.0);
        assert!(eq.is_enabled());

        // Zeros in → zeros out: proof the state was really cleared.
        let mut zeros = vec![0.0; 128];
        eq.process_stereo_in_place(&mut zeros);
        assert!(zeros.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn coefficient_ramp_converges_to_target_within_window() {
        // After exactly `ramp_samples` calls, the active coefficients
        // must equal the targets (snap-on-end eliminates float drift).
        let mut filter = BiquadFilter::new();
        let sr = 44100.0;
        let ramp_samples = (sr * COEF_RAMP_SECS).round() as u32;
        filter.set_peaking_eq(sr, 1000.0, 6.0, 1.0);

        // One tick happens per `process_stereo` call. Run exactly the
        // ramp length.
        for _ in 0..ramp_samples {
            let _ = filter.process_stereo(0.0, 0.0);
        }
        assert_eq!(filter.ramp_remaining, 0);
        assert_eq!(filter.b0, filter.target_b0);
        assert_eq!(filter.b1, filter.target_b1);
        assert_eq!(filter.b2, filter.target_b2);
        assert_eq!(filter.a1, filter.target_a1);
        assert_eq!(filter.a2, filter.target_a2);
    }

    #[test]
    fn coefficient_ramp_eliminates_step_discontinuity() {
        // Drive a steady 1 kHz sine at 44.1 kHz, change the band gain
        // mid-stream, and verify no single sample-to-sample delta blows
        // up. v1 (instant coef swap) produced a ~0.5+ delta at the swap
        // point; the smoothed version stays well under 0.1.
        let sr = 44100.0_f32;
        let mut eq = Equalizer::new(sr);
        eq.set_enabled(true);

        let freq = 1000.0_f32;
        let mut sine = Vec::with_capacity(2 * 1024);
        for n in 0..1024 {
            let s = (2.0 * PI * freq * (n as f32) / sr).sin();
            sine.push(s);
            sine.push(s);
        }
        // Pre-roll 512 frames at gain 0 dB (still active filter, but
        // peaking@0dB is mathematically pass-through).
        eq.process_stereo_in_place(&mut sine[..2 * 512]);
        let pre_last = sine[2 * 512 - 2];

        // Mid-stream: jump band 5 (1 kHz) up to +12 dB. The ramp
        // smooths the transition over ~10 ms.
        eq.set_band_gain(5, 12.0);
        eq.process_stereo_in_place(&mut sine[2 * 512..]);

        let post_first = sine[2 * 512];
        let discontinuity = (post_first - pre_last).abs();
        assert!(
            discontinuity < 0.3,
            "sample-to-sample jump at gain change too large: {}",
            discontinuity
        );

        // No NaN/inf creeps in from the ramp math.
        assert!(sine.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn process_mono_in_place_advances_only_left_state() {
        let mut eq = Equalizer::new(44100.0);
        eq.set_enabled(true);
        eq.set_band_gain(5, 6.0);

        let mut mono = vec![1.0_f32; 1024];
        eq.process_mono_in_place(&mut mono);

        for band in &eq.bands {
            // Mono path is forbidden from touching the right-channel
            // state — that's the whole point of the dedicated path.
            assert_eq!(band.x1_r, 0.0);
            assert_eq!(band.x2_r, 0.0);
            assert_eq!(band.y1_r, 0.0);
            assert_eq!(band.y2_r, 0.0);
        }
    }

    #[test]
    fn q_for_gain_is_symmetric_around_zero() {
        // The constant-Q policy uses |gain_db| so boost and cut at the
        // same magnitude end up with the same Q — symmetric bandwidth.
        for db in [3.0_f32, 6.0, 12.0, 18.0, 20.0] {
            let q_pos = Equalizer::q_for_gain(db);
            let q_neg = Equalizer::q_for_gain(-db);
            assert!(
                (q_pos - q_neg).abs() < 1e-6,
                "Q must be symmetric: {} dB -> {}, {} dB -> {}",
                db,
                q_pos,
                -db,
                q_neg
            );
        }
    }

    #[test]
    fn q_for_gain_grows_with_magnitude() {
        // The whole point of constant-Q is that high gains get tighter
        // bands — verify the monotone progression in |gain|.
        let q0 = Equalizer::q_for_gain(0.0);
        let q3 = Equalizer::q_for_gain(3.0);
        let q12 = Equalizer::q_for_gain(12.0);
        let q20 = Equalizer::q_for_gain(20.0);
        assert!(q0 < q3, "Q at 0 dB ({}) should be < Q at 3 dB ({})", q0, q3);
        assert!(q3 < q12);
        assert!(q12 < q20);

        // Sanity checks on the actual values — keeps a regression here
        // if anyone touches the formula.
        assert!(
            (q0 - 0.707).abs() < 0.01,
            "Q at 0 dB should be ~0.707, got {}",
            q0
        );
        assert!(
            (q20 - 1.074).abs() < 0.01,
            "Q at 20 dB should be ~1.074, got {}",
            q20
        );
    }

    #[test]
    fn headroom_db_flat_eq_is_zero() {
        let eq = Equalizer::new(44100.0);
        // All gains 0 dB → cascaded peak gain 0 dB → no headroom needed.
        let h = eq.headroom_db();
        assert!(
            h.abs() < 0.01,
            "flat EQ should report 0 dB headroom, got {}",
            h
        );
    }

    #[test]
    fn headroom_db_single_band_boost_returns_negative_of_peak() {
        let mut eq = Equalizer::new(44100.0);
        // +12 dB at band 5 (1 kHz). Constant-Q at 12 dB is roughly Q=0.94,
        // so the peak should be very close to +12 dB.
        eq.set_band_gain(5, 12.0);
        let h = eq.headroom_db();
        // Headroom is the negative of the peak — allow ±0.7 dB slack
        // because the probe grid isn't infinitely fine.
        assert!(
            (h + 12.0).abs() < 0.7,
            "single +12 dB band should yield ≈ -12 dB headroom, got {}",
            h
        );
    }

    #[test]
    fn headroom_db_is_never_positive() {
        // A pure cut shouldn't claim "positive headroom" — that would be
        // a no-op preamp boost on apply.
        let mut eq = Equalizer::new(44100.0);
        for i in 0..10 {
            eq.set_band_gain(i, -6.0);
        }
        let h = eq.headroom_db();
        assert!(
            h <= 0.0,
            "headroom must clamp at 0 for cut-only EQ, got {}",
            h
        );
    }

    #[test]
    fn headroom_db_uses_target_coefs_not_active_ones() {
        // Call set_band_gain (which triggers a ramp), then immediately
        // query headroom_db without processing a single sample. The
        // answer must reflect the target (+12 dB), not the active
        // mid-ramp coefs (~0 dB).
        let mut eq = Equalizer::new(44100.0);
        eq.set_band_gain(5, 12.0);
        let h = eq.headroom_db();
        assert!(
            h < -10.0,
            "headroom should see the target, not the in-flight coefs (got {})",
            h
        );
    }

    #[test]
    fn eq_frequencies_match_iso_octave_series() {
        // Same series the presets module re-exports — single source of
        // truth.
        assert_eq!(EQ_FREQUENCIES.len(), 10);
        assert_eq!(EQ_FREQUENCIES[0], 31.5);
        assert_eq!(EQ_FREQUENCIES[5], 1000.0);
        assert_eq!(EQ_FREQUENCIES[9], 16000.0);

        let eq = Equalizer::new(44100.0);
        assert_eq!(eq.get_frequencies(), &EQ_FREQUENCIES[..]);
    }

    fn db_at(filter: &BiquadFilter, sample_rate: f32, freq: f32) -> f32 {
        let omega = std::f32::consts::TAU * freq / sample_rate;
        20.0 * filter.target_magnitude(omega).log10()
    }

    #[test]
    fn low_shelf_at_zero_db_is_unity_at_all_frequencies() {
        let mut f = BiquadFilter::new();
        f.set_low_shelf_snap(44100.0, 120.0, 0.0);
        for &freq in &[40.0_f32, 200.0, 1000.0, 8000.0, 16000.0] {
            let db = db_at(&f, 44100.0, freq);
            assert!(
                db.abs() < 0.05,
                "low_shelf @ 0 dB should be flat at {} Hz, got {} dB",
                freq,
                db
            );
        }
    }

    #[test]
    fn low_shelf_boosts_below_corner_only() {
        let mut f = BiquadFilter::new();
        f.set_low_shelf_snap(44100.0, 120.0, 6.0);
        // Well below the 120 Hz corner: roughly the full +6 dB.
        let db_low = db_at(&f, 44100.0, 30.0);
        assert!(
            (db_low - 6.0).abs() < 0.5,
            "low_shelf +6 dB @ 30 Hz should be ≈ +6 dB, got {}",
            db_low
        );
        // Well above the corner: roughly 0 dB.
        let db_high = db_at(&f, 44100.0, 5000.0);
        assert!(
            db_high.abs() < 0.5,
            "low_shelf +6 dB @ 5 kHz should be ≈ 0 dB, got {}",
            db_high
        );
    }

    #[test]
    fn high_shelf_boosts_above_corner_only() {
        let mut f = BiquadFilter::new();
        f.set_high_shelf_snap(44100.0, 4000.0, 6.0);
        // Well above the 4 kHz corner: roughly +6 dB.
        let db_high = db_at(&f, 44100.0, 12000.0);
        assert!(
            (db_high - 6.0).abs() < 0.5,
            "high_shelf +6 dB @ 12 kHz should be ≈ +6 dB, got {}",
            db_high
        );
        // Well below the corner: roughly 0 dB.
        let db_low = db_at(&f, 44100.0, 100.0);
        assert!(
            db_low.abs() < 0.5,
            "high_shelf +6 dB @ 100 Hz should be ≈ 0 dB, got {}",
            db_low
        );
    }

    #[test]
    fn shelf_ramp_converges_on_set_smooth() {
        // The smooth setter should arm a ramp; after `ramp_samples`
        // calls the active coefs match the target byte-for-byte.
        let mut f = BiquadFilter::new();
        let sr = 44100.0_f32;
        let ramp_samples = (sr * COEF_RAMP_SECS).round() as u32;
        f.set_low_shelf(sr, 120.0, 6.0);
        for _ in 0..ramp_samples {
            let _ = f.process_stereo(0.0, 0.0);
        }
        assert_eq!(f.ramp_remaining, 0);
        assert_eq!(f.b0, f.target_b0);
    }

    #[test]
    fn snap_init_leaves_ramp_idle() {
        // Equalizer::new should NOT leave the bands in a mid-ramp
        // state. Otherwise the first ~10 ms of audio go through
        // gradually-evolving coefficients even though we never asked
        // for a change.
        let eq = Equalizer::new(44100.0);
        for band in &eq.bands {
            assert_eq!(band.ramp_remaining, 0);
            assert_eq!(band.b0, band.target_b0);
            assert_eq!(band.b1, band.target_b1);
            assert_eq!(band.b2, band.target_b2);
            assert_eq!(band.a1, band.target_a1);
            assert_eq!(band.a2, band.target_a2);
        }
    }
}
