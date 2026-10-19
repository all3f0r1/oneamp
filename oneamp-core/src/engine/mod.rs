//! The playback engine: one thread that owns the decode workers, the
//! processing chain and the output, driven by commands and observed
//! through events and a snapshot.
//!
//! * Commands go through a bounded queue. [`AudioEngine::send`] returns
//!   a [`CommandId`] once the command is *queued*; whether it *worked*
//!   arrives later as [`AudioEvent::CommandCompleted`] with the same id
//!   (transport commands) or as the matching `…Updated` echo (settings).
//! * Every loaded media gets a [`PlaybackInstance`]. Events about a
//!   media carry it, and results of a preparation that was abandoned are
//!   dropped, so they can never be mistaken for the next media's.
//! * Events go through a bounded queue in which replaceable updates
//!   (position, volume, EQ, …) are merged; when a slow consumer still
//!   lets it fill, the oldest events are dropped and counted. The full
//!   state is always available, coherent, from [`AudioEngine::snapshot`].

mod core;
mod events;

pub use events::EventQueue;

use crate::dsp::ActiveProcessing;
use crate::error::PlayerError;
use crate::media::{MediaDescriptor, MediaId};
use crate::output::{OutputBackend, OutputDiagnostics, OutputInfo};
use crate::settings::{AudioSettings, EqCurve, RepeatMode};
use crate::track::TrackInfo;
use crate::{MeterSnapshot, ReplayGainMode, WaveformSnapshot};
use arc_swap::ArcSwap;
use crossbeam_channel::{Sender, TrySendError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

/// Identity of one load of one media. A media played twice gets two.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct PlaybackInstance(pub u64);

/// Identity of one sent command.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct CommandId(pub u64);

/// How the output format is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OutputPolicy {
    /// Open the output at each media's own rate and layout (the desktop
    /// default: no resampling when the device accepts the rate). Media
    /// of the same format join gaplessly and crossfade; a format change
    /// reopens the output, leaving a short gap.
    #[default]
    NativePerMedia,
    /// One output format for everything; each media is converted
    /// (band-limited resampling, fold-down) before the chain. Gapless
    /// and crossfade work across rates; nothing is ever reopened for a
    /// format change. The usual choice for mobile platforms.
    Fixed { sample_rate: u32, channels: u16 },
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub output_policy: OutputPolicy,
    /// How long before the current media ends the next one starts
    /// preparing (opening, probing, first blocks). Long enough for a
    /// remote source; the crossfade length is added when it is longer.
    pub preload_lead: Duration,
    /// Decoded blocks a worker may hold ahead of the engine.
    pub decode_queue_blocks: usize,
    pub command_capacity: usize,
    pub event_capacity: usize,
    #[cfg(feature = "http")]
    pub http: crate::http_client::HttpConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            output_policy: OutputPolicy::NativePerMedia,
            preload_lead: Duration::from_secs(10),
            decode_queue_blocks: 16,
            command_capacity: 256,
            event_capacity: 256,
            #[cfg(feature = "http")]
            http: Default::default(),
        }
    }
}

/// Commands to the engine.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    /// Prepare `media` (open, probe, reach its start position) and play
    /// it, or stay [`PlayerState::Ready`] when `paused` (call
    /// [`AudioCommand::Resume`] to start). Replaces what was playing and
    /// drops the prepared next media.
    Load {
        media: MediaDescriptor,
        paused: bool,
    },
    /// Start a prepared media, resume a paused one, or retry a media
    /// stopped by a recoverable error from where it was.
    Resume,
    Pause,
    /// Stop and release the media, the prepared next one and the output.
    Stop,
    /// Seek on the media's own timeline.
    Seek(Duration),
    /// The media to play after the current one (`None` clears it). The
    /// engine prepares it [`EngineConfig::preload_lead`] ahead and joins
    /// it gaplessly or crossfades into it; replacing it cancels the
    /// preparation in flight. The host keeps its own queue and only says
    /// what comes next — after a transition it sets the following one.
    ///
    /// A transition is committed once the current media's last sample
    /// has been written to the output, up to the output buffer (~1 s)
    /// before it is heard; a `SetNext` arriving after that applies to the
    /// media after the committed one.
    SetNext(Option<MediaDescriptor>),
    /// Legacy navigation: stops and asks the host for the next media
    /// ([`AudioEvent::RequestNext`]).
    Next,
    /// Legacy navigation: stops and emits [`AudioEvent::RequestPrevious`].
    Previous,
    SetVolume(f32),
    SetBalance(f32),
    SetMute(bool),
    SetRepeatMode(RepeatMode),
    /// At the end of the current media: stop instead of moving on.
    SetStopAfterCurrent(bool),
    /// Acknowledged only: shuffle order is the queue owner's.
    SetShuffle(bool),
    SetEqualizerEnabled(bool),
    SetEqualizerBand(usize, f32),
    SetEqualizerBands(Vec<f32>),
    ResetEqualizer,
    SetEqualizerPreamp(f32),
    /// Enable/disable and length in seconds (0.5..=30).
    SetCrossfade(bool, f32),
    SetReplayGainEnabled(bool),
    SetReplayGainMode(ReplayGainMode),
    SetReplayGainPreventClipping(bool),
    SetMono(bool),
    SetLoudnessEnabled(bool),
    /// Output device for the next output (re)open. `None` = default.
    SetOutputDevice(Option<String>),
    /// EQ AUTO on/off.
    SetEqAuto(bool),
    /// Add (`Some`) or remove (`None`) the auto preset of a media.
    SetEqAutoPreset(MediaId, Option<EqCurve>),
    /// Replace every setting at once.
    ApplySettings(Box<AudioSettings>),
    /// The host saw the audio route change: reopen the output and keep
    /// the position.
    OutputRouteChanged,
    Shutdown,
}

impl AudioCommand {
    /// `Seek` from seconds as UI code holds them; negative or non-finite
    /// values seek to the start.
    pub fn seek_secs(secs: f32) -> Self {
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        AudioCommand::Seek(Duration::from_secs_f32(secs))
    }
}

/// Engine state, as published in the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PlayerState {
    #[default]
    Idle,
    /// Opening / probing / reaching the start position.
    Preparing,
    /// Prepared and paused before its first sample.
    Ready,
    Playing,
    Paused,
    /// Playing, but the source has no data yet (network, seek).
    Buffering,
    /// The media ended normally.
    Ended,
    /// Stopped by an error; see `last_error.recoverable`.
    Error,
}

/// What can be done with the current media right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Capabilities {
    pub seek: bool,
    pub pause: bool,
    /// After an interruption, `Resume` continues from the position
    /// (seekable) or rejoins the live edge (live).
    pub resume_after_error: bool,
    pub live: bool,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SourceFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub layout: String,
    pub codec: Option<String>,
    /// Integer bit depth of a lossless source.
    pub bits: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NextState {
    /// Set, preparation not started yet (starts `preload_lead` ahead).
    Pending,
    Preparing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NextStatus {
    pub media_id: MediaId,
    pub state: NextState,
}

/// Processing actually applied, beyond the chain's own report.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PipelineStatus {
    #[serde(flatten)]
    pub chain: ActiveProcessing,
    pub crossfading: bool,
    /// The engine converts the media's rate (fixed output policy).
    pub resampling: bool,
    /// Nothing between the decoder and the output touched the samples
    /// and the output format holds the source depth. The output (and an
    /// OS mixer after it) may still convert: see `OutputInfo`.
    pub untouched_to_output: bool,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct EqStatus {
    pub enabled: bool,
    /// The hand-set curve (what to persist).
    pub global: EqCurve,
    /// The curve playing now.
    pub active: EqCurve,
    /// Media whose auto preset is playing.
    pub auto_for: Option<MediaId>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Diagnostics {
    pub output: OutputDiagnostics,
    /// Events dropped because the consumer fell behind.
    pub events_dropped: u64,
    /// Corrupt packets skipped in the current media.
    pub decode_errors: u64,
    /// Transport reconnects of the current media.
    pub reconnects: u32,
    /// Output reopenings after a device loss or route change.
    pub output_reopens: u32,
}

/// A coherent picture of the engine, published on every state change and
/// at least every 50 ms while playing.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PlaybackSnapshot {
    /// Increases with every publication.
    pub sequence: u64,
    pub instance: Option<PlaybackInstance>,
    pub media_id: Option<MediaId>,
    pub state: PlayerState,
    /// Best estimate of what is audible now: decoded position minus
    /// everything queued between the decoder and the speaker (chain
    /// lookahead, output ring, resampler, device latency as reported).
    /// Accuracy is bounded by what the backend reports (cpal: callback
    /// timestamps; pull hosts: `PullHandle::set_latency`).
    pub position: Duration,
    /// Where the decoder is (ahead of `position`).
    pub decoded_position: Duration,
    pub duration: Option<Duration>,
    /// Media time ranges available without waiting: what is decoded and
    /// queued ahead of the position.
    pub available: Vec<(Duration, Duration)>,
    pub capabilities: Capabilities,
    pub source: Option<SourceFormat>,
    pub output: Option<OutputInfo>,
    pub processing: PipelineStatus,
    pub eq: EqStatus,
    pub volume: f32,
    pub muted: bool,
    pub next: Option<NextStatus>,
    pub last_error: Option<PlayerError>,
    pub diagnostics: Diagnostics,
}

/// Events from the engine.
#[derive(Debug, Clone)]
pub enum AudioEvent {
    /// Outcome of a transport command (`Load` once ready, `Seek` once
    /// reached, `Resume`, `Pause`, `Stop`, `SetNext`, `ApplySettings`,
    /// `OutputRouteChanged`).
    CommandCompleted {
        id: CommandId,
        instance: Option<PlaybackInstance>,
        result: Result<(), PlayerError>,
    },
    /// A media became current: right away for `Load`, when its first
    /// sample is about to be heard for a transition.
    TrackLoaded {
        instance: PlaybackInstance,
        track: TrackInfo,
    },
    Playing,
    Paused,
    Stopped,
    /// Playing, but waiting for data.
    Buffering,
    /// The last media ended and nothing follows.
    Finished,
    /// Legacy: nothing was queued at the end with Repeat All, or `Next`.
    RequestNext,
    RequestPrevious,
    /// Audible position (merged: only the latest is queued).
    Position {
        position: Duration,
        duration: Option<Duration>,
    },
    Seeked {
        instance: PlaybackInstance,
        position: Duration,
    },
    NextPrepared {
        media_id: MediaId,
    },
    NextFailed {
        media_id: MediaId,
        error: PlayerError,
    },
    /// EQ enabled + the gains playing now (merged).
    EqualizerUpdated(bool, Vec<f32>),
    /// Preamp playing now (merged).
    EqualizerPreampUpdated(f32),
    /// An auto preset started (`Some`) or stopped (`None`) applying.
    EqAutoApplied(Option<MediaId>),
    /// (volume, muted) (merged).
    VolumeUpdated(f32, bool),
    BalanceUpdated(f32),
    ReplayGainUpdated(bool),
    LoudnessUpdated(bool),
    OutputDeviceUpdated(Option<String>),
    /// An output was opened (load, format change, reopen after a fault).
    OutputChanged(OutputInfo),
    /// ICY `StreamTitle` of a live stream.
    IcyMetadata(String),
    StreamReconnecting {
        attempt: u32,
    },
    StreamReconnected,
    StreamReconnectFailed,
    RepeatModeUpdated(RepeatMode),
    ShuffleUpdated(bool),
    Error {
        instance: Option<PlaybackInstance>,
        error: PlayerError,
    },
    /// The consumer fell behind and this many events were dropped. The
    /// snapshot still holds the current state.
    EventsDropped(u64),
}

/// Why a command could not be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// The command queue is full: the engine is not keeping up.
    QueueFull,
    /// The engine thread is gone.
    Stopped,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::QueueFull => f.write_str("audio engine command queue is full"),
            SendError::Stopped => f.write_str("audio engine has stopped"),
        }
    }
}

impl std::error::Error for SendError {}

/// Handle to a running engine.
pub struct AudioEngine {
    command_tx: Sender<(CommandId, AudioCommand)>,
    next_id: AtomicU64,
    shutdown: Arc<AtomicBool>,
    events: Arc<EventQueue>,
    snapshot: Arc<ArcSwap<PlaybackSnapshot>>,
    thread_handle: Option<thread::JoinHandle<()>>,
    spectrum: Arc<ArcSwap<Vec<f32>>>,
    waveform: Arc<ArcSwap<WaveformSnapshot>>,
    meter: Arc<ArcSwap<MeterSnapshot>>,
}

impl AudioEngine {
    /// The desktop engine: cpal output, default configuration.
    #[cfg(feature = "cpal-output")]
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self::with_backend(
            Box::new(crate::output::cpal::CpalBackend),
            EngineConfig::default(),
        )?)
    }

    /// An engine on any output. No device is opened until a media is
    /// loaded; no UI is involved at any point.
    pub fn with_backend(
        backend: Box<dyn OutputBackend>,
        config: EngineConfig,
    ) -> Result<Self, PlayerError> {
        let (command_tx, command_rx) = crossbeam_channel::bounded(config.command_capacity.max(8));
        let events = Arc::new(EventQueue::new(config.event_capacity.max(16)));
        let snapshot = Arc::new(ArcSwap::from_pointee(PlaybackSnapshot::default()));
        let spectrum = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let waveform = Arc::new(ArcSwap::from_pointee(WaveformSnapshot::empty()));
        let meter = Arc::new(ArcSwap::from_pointee(MeterSnapshot::silent()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let shared = core::Shared {
            events: events.clone(),
            snapshot: snapshot.clone(),
            spectrum: spectrum.clone(),
            waveform: waveform.clone(),
            meter: meter.clone(),
            shutdown: shutdown.clone(),
        };
        let thread_handle = thread::Builder::new()
            .name("oneamp-engine".into())
            .spawn(move || core::run(config, backend, command_rx, shared))
            .map_err(|e| PlayerError::internal(format!("cannot start engine thread: {e}")))?;
        Ok(Self {
            command_tx,
            next_id: AtomicU64::new(1),
            shutdown,
            events,
            snapshot,
            thread_handle: Some(thread_handle),
            spectrum,
            waveform,
            meter,
        })
    }

    /// Queue a command without blocking. `Ok` means queued, not done.
    pub fn send(&self, cmd: AudioCommand) -> Result<CommandId, SendError> {
        let id = CommandId(self.next_id.fetch_add(1, Ordering::Relaxed));
        if matches!(cmd, AudioCommand::Shutdown) {
            self.shutdown.store(true, Ordering::Relaxed);
        }
        match self.command_tx.try_send((id, cmd)) {
            Ok(()) => Ok(id),
            Err(TrySendError::Full(_)) => Err(SendError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(SendError::Stopped),
        }
    }

    /// [`send`](Self::send) for callers that only care whether it was
    /// queued.
    pub fn send_command(&self, cmd: AudioCommand) -> anyhow::Result<()> {
        self.send(cmd).map(|_| ()).map_err(anyhow::Error::from)
    }

    pub fn try_recv_event(&self) -> Option<AudioEvent> {
        self.events.try_pop()
    }

    pub fn recv_event_timeout(&self, timeout: Duration) -> Option<AudioEvent> {
        self.events.pop_timeout(timeout)
    }

    /// The latest coherent state. Cheap (an `Arc` clone).
    pub fn snapshot(&self) -> Arc<PlaybackSnapshot> {
        self.snapshot.load_full()
    }

    /// Latest 16-bin spectrum (empty before the first audio).
    pub fn latest_spectrum(&self) -> Arc<Vec<f32>> {
        self.spectrum.load_full()
    }

    /// Latest oscilloscope frame.
    pub fn latest_waveform(&self) -> Arc<WaveformSnapshot> {
        self.waveform.load_full()
    }

    /// Latest peak/RMS meter values.
    pub fn latest_meter(&self) -> Arc<MeterSnapshot> {
        self.meter.load_full()
    }

    /// Stop the engine and wait for its thread. Workers and outputs are
    /// released before this returns.
    pub fn shutdown(mut self) -> anyhow::Result<()> {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self
            .command_tx
            .try_send((CommandId(0), AudioCommand::Shutdown));
        if let Some(handle) = self.thread_handle.take() {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Failed to join audio thread"))?;
        }
        Ok(())
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self
            .command_tx
            .try_send((CommandId(0), AudioCommand::Shutdown));
        if let Some(handle) = self.thread_handle.take() {
            // Bounded wait: the engine thread itself never blocks on I/O
            // (workers do), but a stuck device driver must not hang exit.
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                eprintln!("audio thread did not stop within 2 s; detaching");
            }
        }
    }
}
