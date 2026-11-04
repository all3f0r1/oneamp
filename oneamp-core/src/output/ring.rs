//! The SPSC ring between the engine (writer) and a real-time consumer
//! (reader), shared by every output backend.

use super::OutputDiagnostics;
use crate::dsp::channels::map_channels;
use crate::dsp::stages::Limiter;
use crate::resample::Resample;
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

/// Ring capacity in seconds: one decoder packet (50–200 ms) always fits
/// after the refill gate opens; a seek drops a bounded amount.
const RING_SECS: f32 = 1.0;
/// The engine produces more once occupancy falls below this.
const REFILL_SECS: f32 = 0.5;
/// One-pole smoothing of volume changes in the callback.
const VOLUME_SMOOTH_SECS: f32 = 0.005;
/// Silence queued behind the last sample by `finish`, so it has left the
/// device's own buffer by the time the ring reads empty.
// ponytail: fixed pad, not the device's real latency (cpal doesn't
// expose it before the first callback); raise if a backend buffers more.
const DRAIN_PAD_SECS: f32 = 0.2;

/// Atomics shared with the real-time side.
pub(crate) struct Shared {
    clear: AtomicBool,
    paused: AtomicBool,
    volume: AtomicU32,
    exact: AtomicBool,
    /// Data written since the last clear/finish: an empty ring then is
    /// an underrun, not an idle output.
    armed: AtomicBool,
    pub underruns: AtomicU64,
    pub latency_ns: AtomicU64,
    pub errors: AtomicU64,
}

impl Shared {
    fn new() -> Self {
        Self {
            clear: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            exact: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            latency_ns: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

/// Engine side.
pub(crate) struct RingWriter {
    prod: HeapProd<f32>,
    pub shared: Arc<Shared>,
    in_ch: usize,
    dev_ch: usize,
    in_rate: u32,
    dev_rate: u32,
    resampler: Option<Resample>,
    /// Peak guard after the resampler: band-limiting a signal the chain
    /// left at full scale can overshoot it (Gibbs, inter-sample peaks
    /// landing on the new grid), and `to_device` would clip those hard.
    /// Present only while resampling, so the native path stays untouched.
    guard: Option<Limiter>,
    guarded: Vec<f32>,
    mapped: Vec<f32>,
    /// Device-format samples converted but not yet in the ring.
    pending: Vec<f32>,
    threshold: usize,
    overflow_waits: u64,
}

/// Real-time side. `render` never blocks or allocates.
pub(crate) struct RingReader {
    cons: HeapCons<f32>,
    shared: Arc<Shared>,
    channels: usize,
    gain: f32,
    smooth: f32,
    /// The last rendered block was untouched PCM at unity gain.
    pub last_exact: bool,
}

/// Build a ring for `in_rate`/`in_ch` input played at
/// `dev_rate`/`dev_ch`. Resamples when the rates differ.
pub(crate) fn ring(
    in_rate: u32,
    in_ch: u16,
    dev_rate: u32,
    dev_ch: u16,
) -> Result<(RingWriter, RingReader), crate::error::PlayerError> {
    let dev_ch_n = usize::from(dev_ch.max(1));
    let capacity = (dev_rate as f32 * dev_ch_n as f32 * RING_SECS).round() as usize;
    let threshold = (dev_rate as f32 * dev_ch_n as f32 * REFILL_SECS).round() as usize;
    let (prod, cons) = HeapRb::<f32>::new(capacity.max(dev_ch_n)).split();
    let shared = Arc::new(Shared::new());
    let resampler = (in_rate != dev_rate)
        .then(|| Resample::new(in_rate, dev_rate, usize::from(in_ch.max(1)), false))
        .transpose()?;
    let guard = resampler
        .is_some()
        .then(|| Limiter::new(dev_rate as f32, usize::from(in_ch.max(1))));
    Ok((
        RingWriter {
            prod,
            shared: shared.clone(),
            in_ch: usize::from(in_ch.max(1)),
            dev_ch: dev_ch_n,
            in_rate,
            dev_rate,
            resampler,
            guard,
            guarded: Vec::new(),
            mapped: Vec::new(),
            pending: Vec::new(),
            threshold,
            overflow_waits: 0,
        },
        RingReader {
            cons,
            shared,
            channels: dev_ch_n,
            gain: 1.0,
            smooth: 1.0 - (-1.0 / (VOLUME_SMOOTH_SECS * dev_rate as f32)).exp(),
            last_exact: false,
        },
    ))
}

impl RingWriter {
    pub fn resampling(&self) -> bool {
        self.resampler.is_some()
    }

    pub fn channel_mapping(&self) -> Option<String> {
        match (self.in_ch, self.dev_ch) {
            (a, b) if a == b => None,
            (1, n) => Some(format!("mono→{n} copies")),
            (a, b) if a < b => Some(format!("{a}ch→first {a} of {b}")),
            // Wider inputs are folded down by the chain before they get
            // here; reaching this means a caller skipped the fold.
            (a, b) => Some(format!("{a}ch→first {b} (truncated)")),
        }
    }

    fn convert_into_pending(&mut self, samples: &[f32]) {
        let data: &[f32] = match (self.resampler.as_mut(), self.guard.as_mut()) {
            (Some(r), Some(g)) => {
                self.guarded.clear();
                self.guarded.extend_from_slice(r.process(samples));
                g.process(&mut self.guarded);
                &self.guarded
            }
            (Some(r), None) => r.process(samples),
            (None, _) => samples,
        };
        if self.in_ch == self.dev_ch {
            self.pending.extend_from_slice(data);
        } else {
            map_channels(data, self.in_ch, self.dev_ch, &mut self.mapped);
            self.pending.extend_from_slice(&self.mapped);
        }
    }

    /// Push pending samples; true when everything is in the ring.
    fn push_pending(&mut self) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let n = self.prod.push_slice(&self.pending);
        self.pending.drain(..n);
        self.pending.is_empty()
    }

    pub fn write(&mut self, samples: &[f32]) -> usize {
        let usable = samples.len() - samples.len() % self.in_ch;
        if !self.push_pending() {
            self.overflow_waits += 1;
            return 0;
        }
        self.convert_into_pending(&samples[..usable]);
        if !self.push_pending() {
            self.overflow_waits += 1;
        }
        self.shared.armed.store(true, Ordering::Relaxed);
        usable
    }

    pub fn wants_data(&mut self) -> bool {
        self.push_pending() && self.prod.occupied_len() < self.threshold
    }

    pub fn buffered_frames(&self) -> u64 {
        let dev_frames = (self.prod.occupied_len() + self.pending.len()) / self.dev_ch;
        let ratio = f64::from(self.in_rate) / f64::from(self.dev_rate);
        let mut frames = (dev_frames as f64 * ratio) as u64;
        if let Some(r) = &self.resampler {
            frames += r.pending_frames() as u64 + (r.delay_frames() as f64 * ratio) as u64;
        }
        if let Some(g) = &self.guard {
            frames += (g.latency_frames() as f64 * ratio) as u64;
        }
        frames
    }

    pub fn latency(&self) -> Duration {
        Duration::from_nanos(self.shared.latency_ns.load(Ordering::Relaxed))
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_exact(&self, exact: bool) {
        self.shared
            .exact
            .store(exact && self.resampler.is_none(), Ordering::Relaxed);
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.shared.armed.store(false, Ordering::Relaxed);
        self.shared.clear.store(true, Ordering::Release);
        if let Some(r) = self.resampler.as_mut() {
            r.reset();
        }
        if let Some(g) = self.guard.as_mut() {
            g.reset();
        }
    }

    pub fn finish(&mut self) {
        if let Some(r) = self.resampler.as_mut() {
            let mut tail = r.flush();
            if let Some(g) = self.guard.as_mut() {
                g.process(&mut tail);
                // Push out what the guard's lookahead still holds.
                let mut held = vec![0.0; g.latency_frames() * self.in_ch];
                g.process(&mut held);
                tail.extend_from_slice(&held);
            }
            if self.in_ch == self.dev_ch {
                self.pending.extend_from_slice(&tail);
            } else {
                map_channels(&tail, self.in_ch, self.dev_ch, &mut self.mapped);
                self.pending.extend_from_slice(&self.mapped);
            }
        }
        let pad = (self.dev_rate as f32 * DRAIN_PAD_SECS) as usize * self.dev_ch;
        self.pending.extend(std::iter::repeat_n(0.0, pad));
        self.push_pending();
        self.shared.armed.store(false, Ordering::Relaxed);
    }

    pub fn is_drained(&mut self) -> bool {
        self.push_pending() && self.prod.is_empty()
    }

    pub fn diagnostics(&self) -> OutputDiagnostics {
        OutputDiagnostics {
            underruns: self.shared.underruns.load(Ordering::Relaxed),
            overflow_waits: self.overflow_waits,
            errors: self.shared.errors.load(Ordering::Relaxed),
            last_error: None,
        }
    }
}

impl RingReader {
    pub fn set_latency(&self, latency: Duration) {
        self.shared
            .latency_ns
            .store(latency.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Pop only the whole frames already queued, volume applied, no
    /// silence padding and no underrun accounting (offline rendering,
    /// tests). Returns the frames written to the front of `out`.
    pub fn read_available(&mut self, out: &mut [f32]) -> usize {
        if self.shared.clear.swap(false, Ordering::AcqRel) {
            self.cons.clear();
        }
        if self.shared.paused.load(Ordering::Relaxed) {
            return 0;
        }
        let frames = (self.cons.occupied_len() / self.channels).min(out.len() / self.channels);
        let n = frames * self.channels;
        let got = self.cons.pop_slice(&mut out[..n]);
        debug_assert_eq!(got, n);
        let target = f32::from_bits(self.shared.volume.load(Ordering::Relaxed));
        self.gain = target;
        if target != 1.0 {
            for s in &mut out[..n] {
                *s *= target;
            }
        }
        frames
    }

    /// Fill `out` (interleaved device frames) with volume applied.
    /// Silence when paused or starved. Returns the frames that carried
    /// real audio.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        if self.shared.clear.swap(false, Ordering::AcqRel) {
            self.cons.clear();
        }
        if self.shared.paused.load(Ordering::Relaxed) {
            out.fill(0.0);
            self.last_exact = false;
            return 0;
        }
        let target = f32::from_bits(self.shared.volume.load(Ordering::Relaxed));
        let exact = self.shared.exact.load(Ordering::Relaxed);
        let mut unity = true;
        let mut real = 0;
        let mut starved = false;
        for frame in out.chunks_mut(self.channels) {
            // Snap once close so unity stays bit-exact.
            self.gain += (target - self.gain) * self.smooth;
            if (self.gain - target).abs() < 1e-6 {
                self.gain = target;
            }
            unity &= self.gain == 1.0;
            let mut got = true;
            for s in frame.iter_mut() {
                match self.cons.try_pop() {
                    Some(v) => *s = if self.gain != 1.0 { v * self.gain } else { v },
                    None => {
                        *s = 0.0;
                        got = false;
                    }
                }
            }
            if got {
                real += 1;
            } else {
                starved = true;
            }
        }
        if starved && self.shared.armed.load(Ordering::Relaxed) {
            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.last_exact = exact && unity;
        real
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_never_drop_and_volume_ramps() {
        let (mut w, mut r) = ring(1000, 1, 1000, 1).unwrap();
        // Ring holds 1000 samples; write 1500 → all taken, 500 pending.
        assert_eq!(w.write(&vec![0.5; 1500]), 1500);
        assert!(!w.wants_data());
        assert_eq!(w.write(&[0.5; 10]), 0, "full: nothing taken");
        let mut out = vec![0.0; 1000];
        assert_eq!(r.render(&mut out), 1000);
        assert!(out.iter().all(|&s| s == 0.5));
        // Pending made it in once room appeared.
        assert!(!w.is_drained());
        let mut out = vec![0.0; 600];
        assert_eq!(r.render(&mut out), 500);
        assert_eq!(w.diagnostics().underruns, 1);
        assert!(w.diagnostics().overflow_waits >= 1);
    }

    #[test]
    fn maps_mono_to_stereo_device() {
        let (mut w, mut r) = ring(48000, 1, 48000, 2).unwrap();
        w.write(&[0.1, 0.2]);
        let mut out = [0.0; 4];
        r.render(&mut out);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2]);
        assert_eq!(w.channel_mapping().as_deref(), Some("mono→2 copies"));
    }

    /// A full-scale square wave: legal for the chain (every sample is
    /// within ±1.0), but band-limiting it rings past full scale.
    fn full_scale_square(rate: u32, freq: u32, frames: usize) -> Vec<f32> {
        let half = (rate / freq / 2) as usize;
        (0..frames)
            .flat_map(|n| {
                let s = if (n / half).is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                };
                [s, s]
            })
            .collect()
    }

    #[test]
    fn resampling_overshoot_is_limited_not_clipped() {
        let input = full_scale_square(44_100, 1_000, 22_050);

        // The premise: the bare resampler does overshoot.
        let mut bare = Resample::new(44_100, 48_000, 2, false).unwrap();
        let mut raw = bare.process(&input).to_vec();
        raw.extend(bare.flush());
        let raw_peak = raw.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(raw_peak > 1.02, "expected an overshoot, peak {raw_peak}");

        let (mut w, mut r) = ring(44_100, 2, 48_000, 2).unwrap();
        assert_eq!(w.write(&input), input.len());
        w.finish();
        let mut peak = 0.0_f32;
        let mut frames = 0;
        let mut out = vec![0.0; 4096];
        loop {
            let n = r.read_available(&mut out);
            if n == 0 && w.is_drained() {
                break;
            }
            peak = out[..n * 2].iter().fold(peak, |m, s| m.max(s.abs()));
            frames += n;
        }
        assert!(peak <= Limiter::CEILING + 1e-6, "guarded peak {peak}");
        // Nothing lost: the guard's lookahead was flushed by `finish`.
        assert!(frames >= 24_000, "{frames} frames out");
    }
}
