//! An output the host drives: it pulls frames on its own audio thread
//! (Android `AudioTrack`/AAudio callback, an offline render, a test).
//!
//! ```no_run
//! use oneamp_core::output::pull::{PullConfig, pull_output};
//! use oneamp_core::{AudioEngine, EngineConfig};
//!
//! let (backend, handle) = pull_output(PullConfig::default());
//! let engine = AudioEngine::with_backend(Box::new(backend), EngineConfig::default()).unwrap();
//! // On the host's audio thread:
//! let mut buf = vec![0.0f32; 2 * 256];
//! let frames = handle.render(&mut buf); // real-time safe
//! # let _ = (engine, frames);
//! ```

use super::ring::{RingReader, RingWriter, ring};
use super::{
    OutputBackend, OutputDiagnostics, OutputFault, OutputInfo, OutputRequest, OutputStream,
};
use crate::error::PlayerError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How a pull output presents itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PullConfig {
    /// The host's device runs at a fixed rate/channel count: the output
    /// converts to it. `None` takes whatever the engine requests (the
    /// host reconfigures its device on [`PullHandle::format`] changes).
    pub device_format: Option<(u32, u16)>,
}

struct Slot {
    reader: RingReader,
    rate: u32,
    channels: u16,
}

#[derive(Default)]
struct Bridge {
    slot: Mutex<Option<Slot>>,
    fault: Mutex<Option<OutputFault>>,
    latency_ns: std::sync::atomic::AtomicU64,
}

/// Create a backend and the host's handle to it.
pub fn pull_output(config: PullConfig) -> (PullBackend, PullHandle) {
    let bridge = Arc::new(Bridge::default());
    (
        PullBackend {
            bridge: bridge.clone(),
            config,
        },
        PullHandle { bridge },
    )
}

pub struct PullBackend {
    bridge: Arc<Bridge>,
    config: PullConfig,
}

/// The host's end. Cheap to clone; `render` is real-time safe.
#[derive(Clone)]
pub struct PullHandle {
    bridge: Arc<Bridge>,
}

impl PullHandle {
    /// Fill `out` with interleaved frames in [`format`](Self::format).
    /// Never blocks: while the engine swaps outputs (or before one is
    /// open) it writes silence. Returns the frames carrying audio.
    pub fn render(&self, out: &mut [f32]) -> usize {
        match self.bridge.slot.try_lock() {
            Ok(mut guard) => match guard.as_mut() {
                Some(slot) => slot.reader.render(out),
                None => {
                    out.fill(0.0);
                    0
                }
            },
            Err(_) => {
                out.fill(0.0);
                0
            }
        }
    }

    /// Take only the frames already queued (no silence, no underrun
    /// accounting): offline rendering and tests. Returns frames written.
    pub fn read_available(&self, out: &mut [f32]) -> usize {
        match self.bridge.slot.try_lock() {
            Ok(mut guard) => guard.as_mut().map_or(0, |s| s.reader.read_available(out)),
            Err(_) => 0,
        }
    }

    /// Current (rate, channels), `None` while no output is open.
    pub fn format(&self) -> Option<(u32, u16)> {
        let guard = self.bridge.slot.lock().ok()?;
        guard.as_ref().map(|s| (s.rate, s.channels))
    }

    /// The host's measured output latency (e.g. `AudioTrack` timestamp
    /// math). Used for the audible position.
    pub fn set_latency(&self, latency: Duration) {
        self.bridge.latency_ns.store(
            latency.as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        if let Ok(g) = self.bridge.slot.lock()
            && let Some(s) = g.as_ref()
        {
            s.reader.set_latency(latency);
        }
    }

    /// Report a route change or device loss; the engine reopens and
    /// resumes from the audible position.
    pub fn report_fault(&self, fault: OutputFault) {
        if let Ok(mut f) = self.bridge.fault.lock() {
            *f = Some(fault);
        }
    }
}

impl OutputBackend for PullBackend {
    fn open(&mut self, req: &OutputRequest) -> Result<Box<dyn OutputStream>, PlayerError> {
        let (rate, channels) = self
            .config
            .device_format
            .unwrap_or((req.sample_rate, req.channels));
        let (writer, reader) = ring(req.sample_rate, req.channels, rate, channels)?;
        reader.set_latency(Duration::from_nanos(
            self.bridge
                .latency_ns
                .load(std::sync::atomic::Ordering::Relaxed),
        ));
        let info = OutputInfo {
            backend: "pull".into(),
            device: "host".into(),
            input_rate: req.sample_rate,
            input_channels: req.channels,
            device_rate: rate,
            device_channels: channels,
            sample_format: "F32".into(),
            resampled: writer.resampling(),
            channel_mapping: writer.channel_mapping(),
            dither: false,
            device_bits: None,
            shared_mode: true,
        };
        *self
            .bridge
            .slot
            .lock()
            .map_err(|_| PlayerError::internal("pull output poisoned"))? = Some(Slot {
            reader,
            rate,
            channels,
        });
        Ok(Box::new(PullStream {
            writer,
            bridge: self.bridge.clone(),
            info,
        }))
    }

    fn name(&self) -> &str {
        "pull"
    }
}

struct PullStream {
    writer: RingWriter,
    bridge: Arc<Bridge>,
    info: OutputInfo,
}

impl Drop for PullStream {
    fn drop(&mut self) {
        if let Ok(mut g) = self.bridge.slot.lock() {
            *g = None;
        }
    }
}

impl OutputStream for PullStream {
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
        self.writer.diagnostics()
    }
    fn take_fault(&mut self) -> Option<OutputFault> {
        self.bridge.fault.lock().ok()?.take()
    }
}
