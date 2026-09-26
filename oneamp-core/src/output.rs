//! Audio output straight on cpal.
//!
//! Engine thread → [`AudioOutput::write_samples`] → (optional sinc
//! resampler) → channel mapping → lock-free SPSC ring → cpal callback
//! (volume, dither, quantization) → device.
//!
//! The stream is opened at the track's native sample rate whenever the
//! device accepts it, in the most precise sample format it offers, so a
//! 16-bit FLAC at unity volume with no DSP reaches the device bit for
//! bit. "Device" is cpal's shared-mode endpoint (WASAPI shared, ALSA
//! default / PipeWire, CoreAudio): the OS mixer behind it may still
//! convert — there is no exclusive mode. Only when the device refuses the native rate does a band-limited
//! FFT resampler (rubato) kick in — never the linear interpolation rodio
//! used to apply silently.

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Ring capacity in seconds. Large enough that a single decoder packet
/// (typ. 50–200 ms) always fits after the refill gate opens, small
/// enough that a seek drops a bounded amount of audio.
const RING_CAPACITY_SECS: f32 = 1.0;
/// The engine decodes more audio once occupancy falls below this.
const NEEDS_DATA_SECS: f32 = 0.5;
/// Input chunk of the fallback resampler, in frames.
const RESAMPLER_CHUNK: usize = 1024;
/// One-pole smoothing time for volume changes in the callback. Short
/// enough to feel instant, long enough to avoid zipper noise.
const VOLUME_SMOOTH_SECS: f32 = 0.005;
/// Silence queued behind the last real sample by [`AudioOutput::finish`],
/// so that sample has left the device's own buffer by the time the ring
/// reads empty and the stream is dropped.
// ponytail: fixed pad, not the device's real latency (cpal doesn't
// expose it); raise if some backend buffers more than this.
const DRAIN_PAD_SECS: f32 = 0.2;

/// Enumerate the output devices of cpal's default host, by name.
pub fn list_output_devices() -> Vec<String> {
    match cpal::default_host().output_devices() {
        Ok(devs) => devs.filter_map(|d| device_name(&d)).collect(),
        Err(_) => Vec::new(),
    }
}

fn device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_string())
}

/// Resolve `name` to a device, falling back to the host default when it
/// is `None` or no longer present (unplugged USB DAC).
fn pick_device(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name
        && let Ok(mut devs) = host.output_devices()
        && let Some(dev) = devs.find(|d| device_name(d).as_deref() == Some(name))
    {
        return Some(dev);
    }
    host.default_output_device()
}

/// Precision rank of a device sample format; 0 = never picked.
fn format_rank(format: SampleFormat) -> u8 {
    match format {
        SampleFormat::F32 | SampleFormat::F64 => 5,
        SampleFormat::I32 => 4,
        SampleFormat::I24 => 3,
        SampleFormat::I16 => 2,
        _ => 0,
    }
}

/// Pick the stream config: native rate first (source channel count,
/// then stereo), best format. `None` rate match → device default config,
/// which the caller then feeds through the resampler.
fn choose_config(
    device: &cpal::Device,
    rate: u32,
    channels: u16,
) -> Result<(StreamConfig, SampleFormat)> {
    let ranges: Vec<_> = device
        .supported_output_configs()
        .map(|r| r.collect())
        .unwrap_or_default();
    for want in [channels, 2] {
        let best = ranges
            .iter()
            .filter(|r| r.channels() == want && r.contains_rate(rate))
            .filter(|r| format_rank(r.sample_format()) > 0)
            .max_by_key(|r| format_rank(r.sample_format()));
        if let Some(range) = best {
            let cfg = (*range).with_sample_rate(rate);
            return Ok((cfg.config(), cfg.sample_format()));
        }
    }
    let cfg = device
        .default_output_config()
        .map_err(|e| anyhow!("{e}"))
        .context("No usable output configuration")?;
    Ok((cfg.config(), cfg.sample_format()))
}

/// Integer grid of a sample format (2^(bits-1)) when it is coarse enough
/// that f32 rounding + dither matter; `None` for float and 32/64-bit ints.
fn quant_step(format: SampleFormat) -> Option<f32> {
    let bits = format.bits_per_sample();
    (format.is_int() || format.is_uint())
        .then_some(bits)
        .filter(|&b| b <= 24)
        .map(|b| (1u32 << (b - 1)) as f32)
}

/// State shared with the real-time callback. Atomics only.
struct Shared {
    /// Consumer drops everything buffered on its next visit (seek).
    drain: AtomicBool,
    paused: AtomicBool,
    /// Target volume (f32 bits), ≤ 1.0. Boost above unity happens in the
    /// engine, ahead of the limiter.
    volume: AtomicU32,
    /// Producer's claim that the queued samples are the untouched source
    /// PCM already on the device's integer grid — dither would only add
    /// noise then.
    exact: AtomicBool,
}

/// Tiny lock-free PRNG for TPDF dither. xorshift32, fixed seed so the
/// sequence is reproducible.
pub(crate) struct DitherRng {
    state: u32,
}

impl DitherRng {
    const SEED: u32 = 0x9E37_79B9;

    pub(crate) fn new() -> Self {
        Self { state: Self::SEED }
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Uniform in [-0.5, +0.5).
    #[inline]
    pub(crate) fn next_uniform(&mut self) -> f32 {
        (self.next_u32() as f32) * (1.0 / 4_294_967_296.0) - 0.5
    }

    /// TPDF noise in [-1, +1] LSB (sum of two uniforms), scaled by `lsb`.
    #[inline]
    pub(crate) fn tpdf(&mut self, lsb: f32) -> f32 {
        (self.next_uniform() + self.next_uniform()) * lsb
    }
}

/// Quantize to the device grid with rounding (dasp truncates toward zero
/// and wraps I24 at +1.0), then hand to cpal's converter, which is exact
/// on grid values.
#[inline]
fn to_device<T: Sample + FromSample<f32>>(v: f32, quant: Option<f32>) -> T {
    let v = match quant {
        Some(q) => (v * q).round().clamp(-q, q - 1.0) / q,
        None => v.clamp(-1.0, 1.0),
    };
    T::from_sample(v)
}

fn build_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    format: SampleFormat,
    mut consumer: HeapCons<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels.max(1) as usize;
    let quant = quant_step(format);
    let smooth = 1.0 - (-1.0 / (VOLUME_SMOOTH_SECS * config.sample_rate as f32)).exp();
    let mut gain = f32::from_bits(shared.volume.load(Ordering::Relaxed));
    let mut rng = DitherRng::new();

    let stream = device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _| {
                if shared.drain.swap(false, Ordering::AcqRel) {
                    consumer.clear();
                }
                if shared.paused.load(Ordering::Relaxed) {
                    data.fill(T::EQUILIBRIUM);
                    return;
                }
                let target = f32::from_bits(shared.volume.load(Ordering::Relaxed));
                let exact = shared.exact.load(Ordering::Relaxed);
                for frame in data.chunks_mut(channels) {
                    // Snap once close so unity stays bit-exact.
                    gain += (target - gain) * smooth;
                    if (gain - target).abs() < 1e-6 {
                        gain = target;
                    }
                    let dither = match quant {
                        Some(q) if !exact || gain != 1.0 => Some(1.0 / q),
                        _ => None,
                    };
                    for out in frame {
                        let mut v = consumer.try_pop().unwrap_or(0.0);
                        if gain != 1.0 {
                            v *= gain;
                        }
                        if let Some(lsb) = dither {
                            v += rng.tpdf(lsb);
                        }
                        *out = to_device(v, quant);
                    }
                }
            },
            |e| eprintln!("AudioOutput: stream error: {e}"),
            None,
        )
        .map_err(|e| anyhow!("{e}"))
        .context("Failed to open audio output stream")?;
    Ok(stream)
}

/// Band-limited fixed-ratio resampler (rubato FFT) with the input
/// buffering rubato's fixed chunk size needs.
struct Resample {
    fft: Fft<f32>,
    channels: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
    chunk_out: Vec<f32>,
    /// Frames fed / produced since the last reset, to size the flush.
    frames_in: u64,
    frames_out: u64,
}

impl Resample {
    fn new(from: u32, to: u32, channels: usize) -> Result<Self> {
        let fft = Fft::<f32>::new(
            from as usize,
            to as usize,
            RESAMPLER_CHUNK,
            channels,
            FixedSync::Input,
        )
        .map_err(|e| anyhow!("{e}"))
        .context("Failed to build resampler")?;
        let chunk_out = vec![0.0; fft.output_frames_max() * channels];
        Ok(Self {
            fft,
            channels,
            pending: Vec::new(),
            out: Vec::new(),
            chunk_out,
            frames_in: 0,
            frames_out: 0,
        })
    }

    /// Feed interleaved input; returns every full chunk converted so far.
    /// A partial chunk (< [`RESAMPLER_CHUNK`] frames) waits for the next
    /// call — across a gapless swap it simply continues with the next
    /// track.
    fn process(&mut self, input: &[f32]) -> &[f32] {
        self.pending.extend_from_slice(input);
        self.out.clear();
        let ch = self.channels;
        self.frames_in += (input.len() / ch) as u64;
        let mut offset = 0;
        loop {
            let need = self.fft.input_frames_next();
            if self.pending.len() - offset < need * ch {
                break;
            }
            let frames_out = self.chunk_out.len() / ch;
            let inp =
                InterleavedSlice::new(&self.pending[offset..offset + need * ch], ch, need).unwrap();
            let mut outp = InterleavedSlice::new_mut(&mut self.chunk_out, ch, frames_out).unwrap();
            match self.fft.process_into_buffer(&inp, &mut outp, None) {
                Ok((_, produced)) => {
                    self.out.extend_from_slice(&self.chunk_out[..produced * ch]);
                    self.frames_out += produced as u64;
                }
                Err(e) => eprintln!("AudioOutput: resampler error: {e}"),
            }
            offset += need * ch;
        }
        self.pending.drain(..offset);
        &self.out
    }

    /// End of stream: push silence through until every real input frame
    /// (the partial chunk still pending and the filter's delay) has come
    /// out, trim the surplus, and reset.
    fn flush(&mut self) -> Vec<f32> {
        let target = (self.frames_in as f64 * self.fft.resample_ratio()).round() as u64
            + self.fft.output_delay() as u64;
        let silence = vec![0.0; RESAMPLER_CHUNK * self.channels];
        let mut tail = Vec::new();
        while self.frames_out < target {
            tail.extend_from_slice(self.process(&silence));
        }
        let surplus = (self.frames_out - target) as usize * self.channels;
        tail.truncate(tail.len().saturating_sub(surplus));
        self.reset();
        tail
    }

    fn reset(&mut self) {
        self.fft.reset();
        self.pending.clear();
        self.frames_in = 0;
        self.frames_out = 0;
    }
}

/// Map interleaved `src_ch` frames onto `dst_ch` device channels: mono is
/// copied to every channel, otherwise the first channels map 1:1 and any
/// extra device channels stay silent. Known limitation: no downmix —
/// fewer device channels drop the rest (5.1 on stereo loses C/LFE/S).
/// Only reached when the device refuses the source channel count.
fn map_channels(src: &[f32], src_ch: usize, dst_ch: usize, out: &mut Vec<f32>) {
    out.clear();
    for frame in src.chunks_exact(src_ch) {
        for c in 0..dst_ch {
            out.push(match (src_ch, frame.get(c)) {
                (1, _) => frame[0],
                (_, Some(&s)) => s,
                (_, None) => 0.0,
            });
        }
    }
}

/// Map source-channel frames to the device layout and queue them.
fn push(
    producer: &mut HeapProd<f32>,
    mapped: &mut Vec<f32>,
    data: &[f32],
    src_ch: usize,
    dst_ch: usize,
) {
    let data = if src_ch == dst_ch {
        data
    } else {
        map_channels(data, src_ch, dst_ch, mapped);
        mapped
    };
    let pushed = producer.push_slice(data);
    if pushed < data.len() {
        eprintln!(
            "AudioOutput: ring full, dropped {} of {} samples",
            data.len() - pushed,
            data.len()
        );
    }
}

pub struct AudioOutput {
    _stream: cpal::Stream,
    producer: HeapProd<f32>,
    shared: Arc<Shared>,
    /// Rate / channels the engine feeds (the track's).
    sample_rate: u32,
    channels: u16,
    /// What the device actually runs at.
    device_rate: u32,
    device_channels: u16,
    device_format: SampleFormat,
    resampler: Option<Resample>,
    mapped: Vec<f32>,
    needs_data_threshold: usize,
}

impl AudioOutput {
    /// Open `device_name` (or the default device) for a `sample_rate` /
    /// `channels` source.
    pub fn new_with_device(
        sample_rate: u32,
        channels: u16,
        device_name: Option<&str>,
    ) -> Result<Self> {
        let device = pick_device(device_name).context("No audio output device")?;
        let (config, format) = choose_config(&device, sample_rate, channels)?;

        let dev_rate = config.sample_rate;
        let dev_ch = config.channels;
        let resampler = if dev_rate != sample_rate {
            Some(Resample::new(
                sample_rate,
                dev_rate,
                channels.max(1) as usize,
            )?)
        } else {
            None
        };

        let capacity = (dev_rate as f32 * dev_ch as f32 * RING_CAPACITY_SECS).round() as usize;
        let needs_data_threshold =
            (dev_rate as f32 * dev_ch as f32 * NEEDS_DATA_SECS).round() as usize;
        let (producer, consumer) = HeapRb::<f32>::new(capacity).split();

        let shared = Arc::new(Shared {
            drain: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            exact: AtomicBool::new(false),
        });

        let s = shared.clone();
        let stream = match format {
            SampleFormat::F32 => build_stream::<f32>(&device, config, format, consumer, s),
            SampleFormat::F64 => build_stream::<f64>(&device, config, format, consumer, s),
            SampleFormat::I32 => build_stream::<i32>(&device, config, format, consumer, s),
            SampleFormat::I24 => build_stream::<cpal::I24>(&device, config, format, consumer, s),
            SampleFormat::I16 => build_stream::<i16>(&device, config, format, consumer, s),
            SampleFormat::U16 => build_stream::<u16>(&device, config, format, consumer, s),
            SampleFormat::I8 => build_stream::<i8>(&device, config, format, consumer, s),
            SampleFormat::U8 => build_stream::<u8>(&device, config, format, consumer, s),
            other => Err(anyhow!("Unsupported device sample format {other:?}")),
        }?;
        stream
            .play()
            .map_err(|e| anyhow!("{e}"))
            .context("Failed to start audio output stream")?;

        eprintln!(
            "AudioOutput: {:?} — source {} Hz/{}ch → device {} Hz/{}ch {:?}{}",
            device_name.unwrap_or("default"),
            sample_rate,
            channels,
            dev_rate,
            dev_ch,
            format,
            if resampler.is_some() {
                " (sinc resampling)"
            } else {
                " (native rate, before the OS mixer)"
            }
        );

        Ok(Self {
            _stream: stream,
            producer,
            shared,
            sample_rate,
            channels,
            device_rate: dev_rate,
            device_channels: dev_ch,
            device_format: format,
            resampler,
            mapped: Vec::new(),
            needs_data_threshold,
        })
    }

    /// Queue interleaved source-rate samples. `exact` = the samples are
    /// the untouched decoded PCM, at a bit depth the device format holds
    /// losslessly; the callback then skips dither at unity volume.
    /// Callers gate on [`needs_data`](Self::needs_data), so the ring has
    /// room; overflow is dropped rather than blocking.
    pub fn write_samples(&mut self, samples: &[f32], exact: bool) {
        let src_ch = self.channels.max(1) as usize;
        let dst_ch = self.device_channels.max(1) as usize;
        self.shared
            .exact
            .store(exact && self.resampler.is_none(), Ordering::Relaxed);
        let data = match self.resampler.as_mut() {
            Some(r) => r.process(samples),
            None => samples,
        };
        push(&mut self.producer, &mut self.mapped, data, src_ch, dst_ch);
    }

    /// End of playback: queue `tail` (the limiter's delay line), flush
    /// the resampler, then pad with silence. Once
    /// [`is_drained`](Self::is_drained) the output can be dropped without
    /// cutting the end of the track.
    pub fn finish(&mut self, tail: &[f32]) {
        let src_ch = self.channels.max(1) as usize;
        let dst_ch = self.device_channels.max(1) as usize;
        let mut data = tail.to_vec();
        if let Some(r) = self.resampler.as_mut() {
            data = r.process(&data).to_vec();
            data.extend(r.flush());
        }
        push(&mut self.producer, &mut self.mapped, &data, src_ch, dst_ch);
        let pad = (self.device_rate as f32 * DRAIN_PAD_SECS) as usize * dst_ch;
        self.producer.push_iter(std::iter::repeat_n(0.0, pad));
    }

    /// Everything queued has been handed to the device.
    pub fn is_drained(&self) -> bool {
        self.producer.is_empty()
    }

    pub fn play(&self) {
        self.shared.paused.store(false, Ordering::Relaxed);
    }

    /// Callback outputs silence and stops consuming; queued audio stays
    /// for resume.
    pub fn pause(&self) {
        self.shared.paused.store(true, Ordering::Relaxed);
    }

    /// Drop everything buffered (seek).
    pub fn clear(&mut self) {
        self.shared.drain.store(true, Ordering::Release);
        if let Some(r) = self.resampler.as_mut() {
            r.reset();
        }
    }

    pub fn needs_data(&self) -> bool {
        self.producer.occupied_len() < self.needs_data_threshold
    }

    /// Source sample rate this output was opened for.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Source channel count this output was opened for.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Integer bit depth of the device format, `None` for float.
    pub fn device_bits(&self) -> Option<u32> {
        (self.device_format.is_int() || self.device_format.is_uint())
            .then(|| self.device_format.bits_per_sample())
    }

    /// Whether the device runs at the source rate (no resampling).
    pub fn is_native_rate(&self) -> bool {
        self.device_rate == self.sample_rate
    }

    /// Attenuation applied in the callback, clamped to [0, 1].
    pub fn set_volume(&self, volume: f32) {
        self.shared
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_rounds_and_clamps_to_grid() {
        let q = quant_step(SampleFormat::I16);
        assert_eq!(q, Some(32768.0));
        // Exact 16-bit values survive untouched.
        let v = 1234.0 / 32768.0;
        assert_eq!(to_device::<i16>(v, q), 1234);
        assert_eq!(to_device::<i16>(-v, q), -1234);
        // Off-grid values round to nearest, not toward zero.
        assert_eq!(to_device::<i16>(1234.7 / 32768.0, q), 1235);
        assert_eq!(to_device::<i16>(-1234.7 / 32768.0, q), -1235);
        // Full scale clamps instead of wrapping.
        assert_eq!(to_device::<i16>(1.5, q), i16::MAX);
        let q24 = quant_step(SampleFormat::I24);
        assert_eq!(to_device::<cpal::I24>(1.0, q24).inner(), (1 << 23) - 1);
        // Float and 32-bit formats need no grid.
        assert_eq!(quant_step(SampleFormat::F32), None);
        assert_eq!(quant_step(SampleFormat::I32), None);
    }

    #[test]
    fn tpdf_dither_is_bounded_and_deterministic() {
        let (mut a, mut b) = (DitherRng::new(), DitherRng::new());
        let lsb = 1.0 / 32768.0;
        for _ in 0..10_000 {
            let x = a.tpdf(lsb);
            assert_eq!(x, b.tpdf(lsb));
            assert!(x.abs() <= lsb);
        }
    }

    #[test]
    fn map_channels_upmixes_mono_and_pads_extra() {
        let mut out = Vec::new();
        map_channels(&[0.1, 0.2], 1, 2, &mut out);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2]);
        map_channels(&[0.1, 0.2], 2, 4, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.0, 0.0]);
        map_channels(&[0.1, 0.2, 0.3, 0.4], 4, 2, &mut out);
        assert_eq!(out, vec![0.1, 0.2]);
    }

    #[test]
    fn resampler_preserves_tone_frequency_and_level() {
        // 1 kHz at 44.1 kHz → 48 kHz, fed in odd-sized pieces: output
        // length follows the ratio and the tone keeps its amplitude.
        let mut r = Resample::new(44_100, 48_000, 1).unwrap();
        let input: Vec<f32> = (0..44_100)
            .map(|n| 0.5 * (std::f32::consts::TAU * 1000.0 * n as f32 / 44_100.0).sin())
            .collect();
        let mut out = Vec::new();
        for chunk in input.chunks(777) {
            out.extend_from_slice(r.process(chunk));
        }
        // Everything but the last partial chunk comes out.
        assert!(
            (46_000..=48_000).contains(&out.len()),
            "{} frames",
            out.len()
        );
        let peak = out[4000..].iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
        // Flush releases the held-back partial chunk and filter delay:
        // delay + every input frame at the new rate, nothing more.
        let delay = r.fft.output_delay();
        out.extend(r.flush());
        assert_eq!(out.len(), 48_000 + delay);
    }
}
