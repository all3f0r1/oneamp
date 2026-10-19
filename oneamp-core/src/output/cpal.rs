//! Audio output straight on cpal.
//!
//! Engine thread → [`OutputStream::write`] → (optional sinc resampler)
//! → channel mapping → lock-free SPSC ring → cpal callback (volume,
//! dither, quantization) → device.
//!
//! The stream is opened at the track's native sample rate whenever the
//! device accepts it, in the most precise sample format it offers, so a
//! 16-bit FLAC at unity volume with no DSP reaches the device bit for
//! bit. "Device" is cpal's shared-mode endpoint (WASAPI shared, ALSA
//! default / PipeWire, CoreAudio): the OS mixer behind it may still
//! convert — there is no exclusive mode, so the path is "untouched up
//! to the OS mixer", never claimed bit-perfect end to end. Only when the
//! device refuses the native rate does a band-limited FFT resampler
//! (rubato) kick in.

use super::ring::{RingReader, RingWriter, ring};
use super::{
    OutputBackend, OutputDiagnostics, OutputFault, OutputInfo, OutputRequest, OutputStream,
};
use crate::error::{ErrorCategory, PlayerError};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn output_error(message: impl Into<String>) -> PlayerError {
    PlayerError::new(ErrorCategory::Output, message)
}

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
) -> Result<(StreamConfig, SampleFormat), PlayerError> {
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
        .map_err(|e| output_error(format!("no usable output configuration: {e}")))?;
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
    mut reader: RingReader,
    fault: Arc<Mutex<Option<OutputFault>>>,
) -> Result<cpal::Stream, PlayerError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let quant = quant_step(format);
    let mut rng = DitherRng::new();
    // Grown once if a backend hands a bigger buffer than this.
    let mut scratch = vec![0.0_f32; 8192 * usize::from(config.channels.max(1))];
    let error_fault = fault.clone();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                reader.set_latency(ts.playback.duration_since(ts.callback));
                if scratch.len() < data.len() {
                    scratch.resize(data.len(), 0.0);
                }
                let buf = &mut scratch[..data.len()];
                reader.render(buf);
                let dither = quant.filter(|_| !reader.last_exact).map(|q| 1.0 / q);
                for (out, &v) in data.iter_mut().zip(buf.iter()) {
                    let v = match dither {
                        Some(lsb) => v + rng.tpdf(lsb),
                        None => v,
                    };
                    *out = to_device(v, quant);
                }
            },
            move |e: cpal::Error| {
                use cpal::ErrorKind::*;
                let f = match e.kind() {
                    // Informational: the stream keeps running.
                    Xrun | RealtimeDenied => return,
                    DeviceChanged => OutputFault::RouteChanged,
                    DeviceNotAvailable | HostUnavailable | StreamInvalidated => {
                        OutputFault::DeviceLost(e.to_string())
                    }
                    _ => OutputFault::Failed(e.to_string()),
                };
                if let Ok(mut slot) = error_fault.lock() {
                    *slot = Some(f);
                }
            },
            None,
        )
        .map_err(|e| output_error(format!("failed to open audio output stream: {e}")))
}

/// The desktop output: cpal's default host.
#[derive(Default)]
pub struct CpalBackend;

impl OutputBackend for CpalBackend {
    fn open(&mut self, req: &OutputRequest) -> Result<Box<dyn OutputStream>, PlayerError> {
        let device = pick_device(req.device.as_deref())
            .ok_or_else(|| output_error("no audio output device"))?;
        let (config, format) = choose_config(&device, req.sample_rate, req.channels)?;
        let dev_rate = config.sample_rate;
        let dev_ch = config.channels;
        let (writer, reader) = ring(req.sample_rate, req.channels, dev_rate, dev_ch)?;
        let fault = Arc::new(Mutex::new(None));
        let f = fault.clone();
        let stream = match format {
            SampleFormat::F32 => build_stream::<f32>(&device, config, format, reader, f),
            SampleFormat::F64 => build_stream::<f64>(&device, config, format, reader, f),
            SampleFormat::I32 => build_stream::<i32>(&device, config, format, reader, f),
            SampleFormat::I24 => build_stream::<cpal::I24>(&device, config, format, reader, f),
            SampleFormat::I16 => build_stream::<i16>(&device, config, format, reader, f),
            SampleFormat::U16 => build_stream::<u16>(&device, config, format, reader, f),
            SampleFormat::I8 => build_stream::<i8>(&device, config, format, reader, f),
            SampleFormat::U8 => build_stream::<u8>(&device, config, format, reader, f),
            other => Err(output_error(format!(
                "unsupported device sample format {other:?}"
            ))),
        }?;
        stream
            .play()
            .map_err(|e| output_error(format!("failed to start audio output stream: {e}")))?;
        let info = OutputInfo {
            backend: "cpal".into(),
            device: device_name(&device).unwrap_or_else(|| "default".into()),
            input_rate: req.sample_rate,
            input_channels: req.channels,
            device_rate: dev_rate,
            device_channels: dev_ch,
            sample_format: format!("{format:?}"),
            resampled: writer.resampling(),
            channel_mapping: writer.channel_mapping(),
            dither: quant_step(format).is_some(),
            device_bits: (format.is_int() || format.is_uint()).then(|| format.bits_per_sample()),
            shared_mode: true,
        };
        eprintln!(
            "AudioOutput: {} — source {} Hz/{}ch → device {} Hz/{}ch {:?}{}",
            info.device,
            req.sample_rate,
            req.channels,
            dev_rate,
            dev_ch,
            format,
            if info.resampled {
                " (sinc resampling)"
            } else {
                " (native rate, before the OS mixer)"
            }
        );
        Ok(Box::new(CpalStream {
            _stream: stream,
            writer,
            info,
            fault,
        }))
    }

    fn name(&self) -> &str {
        "cpal"
    }
}

struct CpalStream {
    _stream: cpal::Stream,
    writer: RingWriter,
    info: OutputInfo,
    fault: Arc<Mutex<Option<OutputFault>>>,
}

impl OutputStream for CpalStream {
    fn info(&self) -> OutputInfo {
        self.info.clone()
    }
    fn write(&mut self, samples: &[f32]) -> usize {
        self.writer.write(samples)
    }
    fn wants_data(&mut self) -> bool {
        self.writer.wants_data()
    }
    fn buffered_frames(&self) -> u64 {
        self.writer.buffered_frames()
    }
    fn latency(&self) -> Duration {
        self.writer.latency()
    }
    fn set_paused(&mut self, paused: bool) {
        self.writer.set_paused(paused);
    }
    fn set_volume(&mut self, volume: f32) {
        self.writer.set_volume(volume);
    }
    fn set_exact(&mut self, exact: bool) {
        self.writer.set_exact(exact);
    }
    fn clear(&mut self) {
        self.writer.clear();
    }
    fn finish(&mut self) {
        self.writer.finish();
    }
    fn is_drained(&mut self) -> bool {
        self.writer.is_drained()
    }
    fn diagnostics(&self) -> OutputDiagnostics {
        let mut d = self.writer.diagnostics();
        if let Ok(f) = self.fault.lock()
            && let Some(f) = f.as_ref()
        {
            d.last_error = Some(format!("{f:?}"));
        }
        d
    }
    fn take_fault(&mut self) -> Option<OutputFault> {
        self.fault.lock().ok()?.take()
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
}
