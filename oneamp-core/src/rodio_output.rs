use anyhow::{Context, Result};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source, cpal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Ring buffer headroom in seconds. The engine refills as soon as
/// occupancy drops below [`NEEDS_DATA_SECS`]; capacity sets the upper
/// bound — large enough that a single decoder packet (typ. 50–200 ms)
/// always fits even when the engine reaches the gate during the
/// refill, but small enough that a `clear()` on seek drops a bounded
/// amount of audio.
const RING_CAPACITY_SECS: f32 = 1.0;
/// Lower threshold below which the engine decodes and pushes more
/// audio. ~0.5 s matches the v1 mutex-deque implementation; together
/// with the 1.0 s capacity it leaves ~0.5 s of producer headroom.
const NEEDS_DATA_SECS: f32 = 0.5;

/// Enumerate every audio output device exposed by cpal's default host.
/// Returns one name per device — duplicate names (cpal sometimes reports
/// them) are preserved because the user-facing dropdown is the only
/// place where we can disambiguate. Unreadable device names get dropped
/// silently rather than papered over with a placeholder.
pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    match host.output_devices() {
        Ok(devs) => devs.filter_map(|d| d.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Resolve `name` to a cpal Device on the default host. Falls back to
/// `default_output_device` when `name` is `None` or doesn't match any
/// enumerable device — useful when the user picked a USB DAC that was
/// later unplugged. Returns `None` only when even the default device
/// can't be opened (truly headless / no audio stack).
fn pick_device(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name
        && let Ok(mut devs) = host.output_devices()
        && let Some(dev) = devs.find(|d| d.name().ok().as_deref() == Some(name))
    {
        return Some(dev);
    }
    host.default_output_device()
}

/// Audio output using rodio (which wraps cpal with better ALSA handling).
///
/// Decoder → ring buffer → cpal callback. The ring is a lock-free SPSC
/// queue ([`ringbuf::HeapRb`]) split into a producer kept here and a
/// consumer owned by [`StreamingSource`]. The cpal callback thread —
/// which runs at real-time priority — never blocks on a mutex; it just
/// pops samples via wait-free atomics. v1 routed samples through
/// `Arc<Mutex<VecDeque<f32>>>` and held the lock once per sample, which
/// risked priority inversion on busy systems (compositor stalls, GC
/// pauses on the UI thread) and produced occasional underrun clicks.
pub struct RodioOutput {
    _stream: OutputStream,
    sink: Arc<Mutex<Sink>>,
    /// Sole producer half of the SPSC ring. Written from the audio engine
    /// thread only. Held as `Mutex<_>` so `write_samples` / `clear` can
    /// take `&self` (preserves the v1 public API). Contention is
    /// engine-vs-engine — never producer-vs-consumer — so this lock has
    /// no real-time consequences.
    producer: Mutex<HeapProd<f32>>,
    /// Set to `true` by `clear()` to ask the consumer (cpal callback) to
    /// drop everything it has buffered on its next visit. The consumer
    /// clears the flag with an `AcqRel` swap. The producer side can't
    /// drain the ring directly — `ringbuf`'s SPSC split routes `skip`
    /// through the consumer only — so we cross the thread boundary with
    /// this lightweight flag instead.
    drain_signal: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
    /// Cached ring capacity (samples). Used for `needs_data` math
    /// without re-querying the producer.
    capacity_samples: usize,
    /// Cached threshold: number of samples below which we treat the
    /// ring as "starving". Derived from `NEEDS_DATA_SECS` at construction.
    needs_data_threshold: usize,
}

/// `rodio::Source` that drains the SPSC ring on the cpal callback
/// thread. Returns `Some(0.0)` on empty rather than `None`: the rodio
/// queue would otherwise replace our source with `Zero::new_samples`
/// and drop us on the first underrun (see `rodio::queue::go_next`),
/// silently breaking playback for the rest of the track.
struct StreamingSource {
    consumer: HeapCons<f32>,
    drain_signal: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
}

impl Iterator for StreamingSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // Cheap check — `AcqRel` makes sure any non-atomic state the
        // producer touched before setting the flag is visible here too,
        // though in practice we only consume from the ring (already
        // synchronized internally by ringbuf).
        if self.drain_signal.swap(false, Ordering::AcqRel) {
            self.consumer.clear();
        }
        Some(self.consumer.try_pop().unwrap_or(0.0))
    }
}

impl Source for StreamingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl RodioOutput {
    /// Create a new audio output using rodio's default device.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        Self::new_with_device(sample_rate, channels, None)
    }

    /// Create a new audio output. When `device_name` is `Some(name)`,
    /// open the matching cpal output device (falling back to default
    /// if `name` no longer exists — covers USB DACs that got unplugged
    /// between sessions). `None` always uses the host default.
    pub fn new_with_device(
        sample_rate: u32,
        channels: u16,
        device_name: Option<&str>,
    ) -> Result<Self> {
        eprintln!(
            "RodioOutput::new - sample_rate={}, channels={}, device={:?}",
            sample_rate, channels, device_name
        );

        // rodio 0.21 collapsed `(OutputStream, OutputStreamHandle)` into
        // a single `OutputStream` that owns the mixer. Sink connects
        // directly to `stream.mixer()` (no `try_new` Result anymore).
        let stream = match pick_device(device_name) {
            Some(device) => OutputStreamBuilder::from_device(device)
                .context("Failed to address audio output device")?
                .open_stream()
                .context("Failed to open audio output device")?,
            None => OutputStreamBuilder::open_default_stream()
                .context("Failed to open default audio output device")?,
        };

        let sink = Sink::connect_new(stream.mixer());

        let capacity_samples =
            (sample_rate as f32 * channels as f32 * RING_CAPACITY_SECS).round() as usize;
        let needs_data_threshold =
            (sample_rate as f32 * channels as f32 * NEEDS_DATA_SECS).round() as usize;

        let rb = HeapRb::<f32>::new(capacity_samples);
        let (producer, consumer) = rb.split();

        let drain_signal = Arc::new(AtomicBool::new(false));

        // No source-level limiter here — the engine thread applies a
        // pre-buffer brickwall limiter (see PeakLimiter in
        // audio_thread_symphonia.rs) so the protection sees the true
        // worst-case signal regardless of the user's volume slider.
        // v1 wrapped this in `rodio::source::Limit`, which sat past
        // the sink volume and was effectively useless at high listener
        // volume.
        let source = StreamingSource {
            consumer,
            drain_signal: drain_signal.clone(),
            sample_rate,
            channels,
        };
        sink.append(source);

        eprintln!(
            "RodioOutput created successfully (ring cap = {} samples, refill below {})",
            capacity_samples, needs_data_threshold
        );

        Ok(Self {
            _stream: stream,
            sink: Arc::new(Mutex::new(sink)),
            producer: Mutex::new(producer),
            drain_signal,
            sample_rate,
            channels,
            capacity_samples,
            needs_data_threshold,
        })
    }

    /// Write samples into the ring. Caller is expected to gate writes
    /// on [`needs_data`] so the ring always has room — if it doesn't,
    /// the trailing samples are dropped silently rather than blocking
    /// the engine thread.
    pub fn write_samples(&self, samples: &[f32]) {
        if let Ok(mut producer) = self.producer.lock() {
            let pushed = producer.push_slice(samples);
            if pushed < samples.len() {
                eprintln!(
                    "RodioOutput: ring full, dropped {} of {} samples",
                    samples.len() - pushed,
                    samples.len()
                );
            }
        }
    }

    /// Resume sink playback. Outstanding samples in the ring stay put.
    pub fn play(&self) -> Result<()> {
        if let Ok(sink) = self.sink.lock() {
            sink.play();
        }
        Ok(())
    }

    /// Pause sink playback. The cpal callback stops pulling from the
    /// ring; samples already pushed are preserved for resume.
    pub fn pause(&self) -> Result<()> {
        if let Ok(sink) = self.sink.lock() {
            sink.pause();
        }
        Ok(())
    }

    /// Drop everything currently buffered. Used on seek so the listener
    /// doesn't hear ~0.5 s of pre-seek audio before the new position
    /// kicks in. Implemented as an atomic flag picked up by the consumer
    /// on its next sample — the producer side of `ringbuf`'s SPSC split
    /// can't drain the ring directly.
    pub fn clear(&self) {
        self.drain_signal.store(true, Ordering::Release);
    }

    /// Number of samples currently visible to the consumer.
    pub fn buffer_len(&self) -> usize {
        self.producer.lock().map(|p| p.occupied_len()).unwrap_or(0)
    }

    /// Whether the producer should refill the ring. Returns `true` when
    /// occupancy drops below ~`NEEDS_DATA_SECS`.
    pub fn needs_data(&self) -> bool {
        self.buffer_len() < self.needs_data_threshold
    }

    /// Sample rate (Hz) declared at construction.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel count declared at construction.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Ring capacity in samples. Always larger than `needs_data` to
    /// give the producer headroom past the refill threshold.
    pub fn capacity(&self) -> usize {
        self.capacity_samples
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) -> Result<()> {
        if let Ok(sink) = self.sink.lock() {
            sink.set_volume(volume.clamp(0.0, 1.0));
        }
        Ok(())
    }

    /// Get current volume
    pub fn volume(&self) -> f32 {
        self.sink.lock().map(|s| s.volume()).unwrap_or(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the producer / consumer / drain-signal protocol used
    /// by `StreamingSource::next` without instantiating a real cpal
    /// stream (which would require an audio device on the test host).
    /// We verify three properties:
    /// 1. `push_slice` followed by `try_pop` round-trips chronologically.
    /// 2. `drain_signal` swap+clear empties the ring on the consumer
    ///    side without the producer holding any lock.
    /// 3. After draining, `try_pop().unwrap_or(0.0)` produces silence —
    ///    proving the `next()` policy (never return `None`) is preserved.
    #[test]
    fn drain_signal_clears_ring_on_consumer_side() {
        let rb = HeapRb::<f32>::new(16);
        let (mut prod, mut cons) = rb.split();
        let signal = Arc::new(AtomicBool::new(false));

        let pushed = prod.push_slice(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(pushed, 4);
        assert_eq!(cons.occupied_len(), 4);

        // Same swap-then-clear policy as StreamingSource::next.
        signal.store(true, Ordering::Release);
        if signal.swap(false, Ordering::AcqRel) {
            cons.clear();
        }
        assert_eq!(cons.occupied_len(), 0);

        // Empty ring → silence, not None.
        let sample = cons.try_pop().unwrap_or(0.0);
        assert_eq!(sample, 0.0);

        // Producer can resume pushing immediately; consumer reads the
        // post-drain data in order.
        prod.push_slice(&[10.0, 11.0]);
        assert_eq!(cons.try_pop().unwrap_or(0.0), 10.0);
        assert_eq!(cons.try_pop().unwrap_or(0.0), 11.0);
        assert_eq!(cons.try_pop().unwrap_or(0.0), 0.0);
    }
}
