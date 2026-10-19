//! OneAmp's audio core, embeddable.
//!
//! Two ways in:
//!
//! * **Full player** ([`AudioEngine`], feature `player`): give it
//!   [`MediaDescriptor`]s — local files, on-demand or live HTTP with an
//!   injectable [`access::AccessProvider`], or a byte source of your own —
//!   and it decodes, buffers, processes, joins tracks gaplessly or
//!   crossfades, and writes to an output you choose (cpal, a host pull
//!   ring, memory for tests). No UI anywhere.
//! * **PCM processor** ([`dsp::Processor`], always available): the same
//!   EQ / preamp / ReplayGain / loudness / balance / mono / limiter
//!   chain over blocks a platform player hands you. No thread, no
//!   device, no network; builds for `wasm32-unknown-unknown`.
//!
//! See `docs/embedding/` in the repository for the responsibilities
//! split, the capability matrix and the Tesserafin integration guide.

pub mod atomic_write;
pub mod audio_capture;
pub mod dsp;
pub mod eqf;
pub mod equalizer;
pub mod equalizer_presets;
pub mod error;
#[cfg(feature = "desktop")]
pub mod playlist;
pub mod queue;
#[cfg(feature = "desktop")]
pub mod recent_files;
pub mod settings;

#[cfg(feature = "http")]
pub mod access;
#[cfg(feature = "player")]
pub mod engine;
#[cfg(feature = "http")]
mod http_client;
#[cfg(feature = "http")]
pub mod http_range;
#[cfg(feature = "http")]
pub mod http_stream;
#[cfg(feature = "player")]
pub mod media;
#[cfg(feature = "player")]
pub mod output;
#[cfg(feature = "player")]
pub mod resample;
#[cfg(feature = "player")]
mod source;
#[cfg(feature = "player")]
pub mod symphonia_player;
#[cfg(feature = "tags")]
pub mod tag_editor;
#[cfg(feature = "player")]
pub mod track;
#[cfg(feature = "player")]
mod visualization;
#[cfg(feature = "player")]
mod worker;
#[cfg(feature = "skins")]
pub mod wsz;

pub use atomic_write::write_atomic;
pub use audio_capture::AudioCaptureBuffer;
pub use dsp::{ListeningContext, MAX_VOLUME, MediaGain, ReplayGainMode};
pub use equalizer::{EQ_MAX_DB, Equalizer};
pub use equalizer_presets::{BuiltinPresets, EQ_FREQUENCIES, EqualizerPreset, PresetManager};
pub use error::{ErrorCategory, PlayerError};
#[cfg(feature = "desktop")]
pub use playlist::{Playlist, PlaylistEntry, SortOrder};
pub use queue::PlayQueue;
#[cfg(feature = "desktop")]
pub use recent_files::{RecentFile, RecentFiles};
pub use settings::{AudioSettings, EqCurve, RepeatMode};

#[cfg(feature = "http")]
pub use access::{Access, AccessProvider, RenewReason, StaticAccess};
#[cfg(feature = "player")]
pub use engine::{
    AudioCommand, AudioEngine, AudioEvent, CommandId, EngineConfig, OutputPolicy, PlaybackInstance,
    PlaybackSnapshot, PlayerState, SendError,
};
#[cfg(feature = "http")]
pub use http_client::HttpConfig;
#[cfg(feature = "http")]
pub use http_stream::is_stream_url;
#[cfg(feature = "player")]
pub use media::{
    CancelToken, Delivery, MediaDescriptor, MediaId, MediaLocation, MediaMetadata, SourceFactory,
};
#[cfg(feature = "cpal-output")]
pub use output::cpal::list_output_devices;
#[cfg(feature = "player")]
pub use track::TrackInfo;

/// First 4-digit run in a date-ish tag (`"1994-07-15"`, `"(1994)"`).
#[cfg(any(feature = "player", feature = "tags"))]
pub(crate) fn parse_year(raw: &str) -> Option<u32> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit()) {
            return std::str::from_utf8(&bytes[i..i + 4]).ok()?.parse().ok();
        }
        i += 1;
    }
    None
}

/// One frame of pre-decimated oscilloscope data, paired (min, max) per
/// output column. v1 sent a single value per column (nearest-neighbor
/// downsample) which aliased high-frequency content. The min/max pair
/// preserves the extreme excursion within each bucket so a 8 kHz sine
/// renders as a visibly oscillating waveform rather than a flat
/// nearest-sample line, regardless of the buffer-to-display ratio.
///
/// `mins` and `maxs` always have identical length; the producer
/// guarantees `mins[i] <= maxs[i]`. Both arrays are in chronological
/// order (oldest first), starting at the latest rising zero-crossing
/// in the capture so the displayed waveform stays phase-stable on
/// periodic content instead of "scrolling".
#[derive(Debug, Clone)]
pub struct WaveformSnapshot {
    pub mins: Vec<f32>,
    pub maxs: Vec<f32>,
}

impl WaveformSnapshot {
    /// Empty snapshot. Used as the initial value before the audio
    /// thread has produced any data.
    pub fn empty() -> Self {
        Self {
            mins: Vec::new(),
            maxs: Vec::new(),
        }
    }

    /// Number of (min, max) columns in this snapshot.
    pub fn len(&self) -> usize {
        self.mins.len()
    }

    /// `true` when no columns are available yet.
    pub fn is_empty(&self) -> bool {
        self.mins.is_empty()
    }
}

/// Per-channel peak + RMS snapshot for the VU-style meter. All values
/// are linear amplitudes in `[0.0, +∞)` (typically clamped to 1.0 by
/// the upstream brickwall limiter); converting to dB is the
/// renderer's job so the on-screen scale stays in one place.
///
/// `peak_*` is the maximum |sample| seen since the last snapshot —
/// reset on each refresh tick so the bar tracks short-term transients
/// instead of an ever-decaying running max. `rms_*` is a one-pole
/// smoothed root-mean-square with ~300 ms time constant: long enough
/// to feel like a "loudness" indicator, short enough to react to a
/// dynamic mix.
#[derive(Debug, Clone, Copy)]
pub struct MeterSnapshot {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

impl MeterSnapshot {
    /// All channels silent — the default published value before the
    /// audio thread has produced any samples.
    pub const fn silent() -> Self {
        Self {
            peak_l: 0.0,
            peak_r: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_capture_buffer() {
        let mut buffer = AudioCaptureBuffer::new(1024);
        assert_eq!(buffer.capacity(), 1024, "Capacity should match request");

        // Newly-constructed buffer: snapshot is all zeros.
        let mut out = vec![0.5_f32; 1024];
        buffer.snapshot_into(&mut out);
        assert!(
            out.iter().all(|&s| s == 0.0),
            "Empty buffer snapshots to zeros"
        );

        // Push 512 samples, then snapshot 1024 — leading half is the zero
        // pre-roll, trailing half is the pushed data.
        let pushed: Vec<f32> = (0..512).map(|i| (i as f32) / 512.0).collect();
        buffer.update(&pushed, 44100, 2);
        buffer.snapshot_into(&mut out);
        assert!(out[..512].iter().all(|&s| s == 0.0));
        assert_eq!(&out[512..], pushed.as_slice());
        assert_eq!(buffer.sample_rate(), 44100);
        assert_eq!(buffer.channels(), 2);
    }

    #[cfg(any(feature = "player", feature = "tags"))]
    #[test]
    fn parse_year_finds_four_digits() {
        assert_eq!(parse_year("1994-07-15"), Some(1994));
        assert_eq!(parse_year("(1994)"), Some(1994));
        assert_eq!(parse_year("94"), None);
    }
}
