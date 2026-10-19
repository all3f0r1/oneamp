//! Replaceable audio outputs.
//!
//! The engine writes processed `f32` frames into an [`OutputStream`]
//! obtained from an [`OutputBackend`]. Shipped backends:
//!
//! * [`cpal`] (feature `cpal-output`) — the desktop device, opened at
//!   the stream's native rate when the device accepts it.
//! * [`pull`] — the host pulls frames from a lock-free ring on its own
//!   audio thread (Android `AudioTrack`/AAudio, tests, offline capture).
//!
//! Every backend shares [`ring`]: an SPSC ring between the engine and a
//! real-time consumer that never blocks and never allocates. Writes
//! never drop audio: what does not fit stays pending in the writer and
//! the engine stops producing until it drains.

#[cfg(feature = "cpal-output")]
pub mod cpal;
pub mod pull;
pub(crate) mod ring;

use crate::error::PlayerError;
use std::time::Duration;

/// What the engine asks an output to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRequest {
    pub sample_rate: u32,
    pub channels: u16,
    /// Backend-specific device name; `None` = default device.
    pub device: Option<String>,
}

/// What an output actually does with the samples — reported so nothing
/// claims a bit-perfect path the backend does not provide.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct OutputInfo {
    pub backend: String,
    pub device: String,
    /// Format the engine writes.
    pub input_rate: u32,
    pub input_channels: u16,
    /// Format the device runs at.
    pub device_rate: u32,
    pub device_channels: u16,
    /// Device sample format ("F32", "I16", …), or "F32" for pull hosts.
    pub sample_format: String,
    /// The output resamples (device refused the rate).
    pub resampled: bool,
    /// How channels are placed when counts differ ("mono→all", …).
    pub channel_mapping: Option<String>,
    /// TPDF dither is added when quantising to an integer format,
    /// except while the pipeline is untouched and at unity volume.
    pub dither: bool,
    /// Integer bit depth of the device format, `None` for float.
    pub device_bits: Option<u32>,
    /// The device is shared: an OS mixer downstream may still resample,
    /// mix or apply its own volume. True for every shipped backend.
    pub shared_mode: bool,
}

/// Counters for diagnosing glitches.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct OutputDiagnostics {
    /// Callbacks that found the ring empty while playing.
    pub underruns: u64,
    /// Writes that had to wait because the ring was full (audio kept,
    /// not dropped).
    pub overflow_waits: u64,
    /// Errors the backend reported.
    pub errors: u64,
    pub last_error: Option<String>,
}

/// Something the engine has to react to.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputFault {
    /// The device disappeared or the stream must be rebuilt: reopen.
    DeviceLost(String),
    /// The route changed (headphones unplugged, Bluetooth connected).
    /// The host decides whether a reopen is needed; the engine reopens
    /// on its default device and keeps the position.
    RouteChanged,
    /// Any other backend failure.
    Failed(String),
}

pub trait OutputBackend: Send {
    fn open(&mut self, request: &OutputRequest) -> Result<Box<dyn OutputStream>, PlayerError>;
    fn name(&self) -> &str;
}

/// An open output. Lives on the engine thread.
pub trait OutputStream {
    fn info(&self) -> OutputInfo;
    /// Queue interleaved frames in the requested format. Returns how
    /// many samples were taken (whole frames); the rest is the caller's
    /// to offer again. Never blocks.
    fn write(&mut self, samples: &[f32]) -> usize;
    /// The ring is below its refill threshold and nothing is pending.
    fn wants_data(&mut self) -> bool;
    /// Input-format frames written but not yet handed to the device.
    fn buffered_frames(&self) -> u64;
    /// Delay between handing a frame to the device and hearing it, as
    /// far as the backend knows (0 when it cannot tell).
    fn latency(&self) -> Duration;
    fn set_paused(&mut self, paused: bool);
    /// Attenuation in the real-time callback, 0..=1.
    fn set_volume(&mut self, volume: f32);
    /// The queued samples are the untouched decoded PCM.
    fn set_exact(&mut self, exact: bool);
    /// Drop everything buffered (seek).
    fn clear(&mut self);
    /// End of playback: flush internal buffers so everything written
    /// reaches the device; then [`is_drained`](Self::is_drained).
    fn finish(&mut self);
    fn is_drained(&mut self) -> bool;
    fn diagnostics(&self) -> OutputDiagnostics;
    fn take_fault(&mut self) -> Option<OutputFault>;
}
