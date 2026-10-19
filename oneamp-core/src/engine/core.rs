//! The engine thread.
//!
//! Each turn: take every queued command, collect what the decode
//! workers sent, feed the output while it wants data, handle the end of
//! the current media (gapless switch, reopen for a new format, or
//! finish), start preparing the next media when it is due, then publish
//! the snapshot, position and visualisations.
//!
//! Timeline bookkeeping: every chunk written to the output records the
//! media time it starts at ([`Segment`]). The audible position is the
//! output frame being heard — frames written minus frames still queued
//! in the output, minus the device latency — mapped back through those
//! records. A transition's `TrackLoaded` is emitted when the first frame
//! of the incoming media reaches that point.

use super::{
    AudioCommand, AudioEvent, Capabilities, CommandId, Diagnostics, EngineConfig, EqStatus,
    EventQueue, NextState, NextStatus, OutputPolicy, PipelineStatus, PlaybackInstance,
    PlaybackSnapshot, PlayerState, SourceFormat,
};
use crate::dsp::channels::ChannelLayout;
use crate::dsp::{Chain, TrackGain};
use crate::error::{ErrorCategory, PlayerError};
use crate::media::{MediaDescriptor, MediaId};
use crate::output::{OutputBackend, OutputFault, OutputInfo, OutputRequest, OutputStream};
use crate::resample::Converter;
use crate::settings::{AudioSettings, EqCurve, RepeatMode};
use crate::source::SourceConfig;
use crate::symphonia_player::Block;
use crate::track::TrackInfo;
use crate::visualization::{
    CAPTURE_FRAMES, FFT_SIZE, PeakRmsMeter, compute_spectrum, compute_waveform,
};
use crate::worker::{DecoderWorker, SourceInfo, WorkerMsg};
use crate::{AudioCaptureBuffer, MeterSnapshot, WaveformSnapshot};
use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, RecvTimeoutError};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(super) struct Shared {
    pub events: Arc<EventQueue>,
    pub snapshot: Arc<ArcSwap<PlaybackSnapshot>>,
    pub spectrum: Arc<ArcSwap<Vec<f32>>>,
    pub waveform: Arc<ArcSwap<WaveformSnapshot>>,
    pub meter: Arc<ArcSwap<MeterSnapshot>>,
    pub shutdown: Arc<AtomicBool>,
}

const ACTIVE_WAIT: Duration = Duration::from_millis(5);
const IDLE_WAIT: Duration = Duration::from_millis(100);
const POSITION_EVERY: Duration = Duration::from_millis(100);
const SNAPSHOT_EVERY: Duration = Duration::from_millis(50);
const VIZ_EVERY: Duration = Duration::from_millis(33);
/// Chunks written per turn at most, so commands stay responsive.
const FILL_BUDGET: usize = 16;
/// Below this much queued audio a starved source shows as buffering.
const BUFFERING_SECS: f64 = 0.05;
const REOPEN_RETRY: Duration = Duration::from_secs(1);
const REOPEN_ATTEMPTS: u32 = 10;

pub(super) fn run(
    cfg: EngineConfig,
    backend: Box<dyn OutputBackend>,
    rx: Receiver<(CommandId, AudioCommand)>,
    sh: Shared,
) {
    let mut core = Core::new(cfg, backend, sh);
    'outer: loop {
        if core.sh.shutdown.load(Ordering::Relaxed) {
            break;
        }
        let wait = if core.active() {
            ACTIVE_WAIT
        } else {
            IDLE_WAIT
        };
        match rx.recv_timeout(wait) {
            Ok((id, cmd)) => {
                if !core.command(id, cmd) {
                    break;
                }
                while let Ok((id, cmd)) = rx.try_recv() {
                    if !core.command(id, cmd) {
                        break 'outer;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        core.tick();
    }
    // Release workers (cancelled on drop) and the device before the
    // thread ends, so `shutdown()` returning means everything is closed.
    core.next = None;
    core.current = None;
    core.pipeline = None;
}

/// What a worker reported besides audio.
enum Notice {
    Ready,
    Seeked,
    SeekFailed(PlayerError),
    Failed(PlayerError),
}

/// One media being decoded into the pipeline.
struct Media {
    instance: PlaybackInstance,
    desc: MediaDescriptor,
    worker: DecoderWorker,
    info: Option<Box<SourceInfo>>,
    gain: TrackGain,
    conv: Option<Converter>,
    /// Blocks received but not converted yet (bounded by the worker
    /// queue size).
    queued: VecDeque<Block>,
    /// Pipeline-format samples ready to mix.
    carry: Vec<f32>,
    /// Media time of `carry[0]`.
    carry_secs: f64,
    /// Media time just past the last block received.
    decoded_secs: f64,
    worker_ended: bool,
    failed: Option<PlayerError>,
    seeking: Option<(u32, Option<CommandId>)>,
    load_cmd: Option<CommandId>,
    /// Belongs to the pipeline format (conversion set up).
    attached: bool,
    #[cfg(feature = "http")]
    last_icy: String,
    #[cfg(feature = "http")]
    last_reconnect: crate::http_stream::ReconnectState,
}

impl Media {
    fn spawn(
        instance: PlaybackInstance,
        desc: MediaDescriptor,
        cfg: &EngineConfig,
        src: &SourceConfig,
    ) -> Self {
        let start = desc.start_position.as_secs_f64();
        Self {
            instance,
            worker: DecoderWorker::spawn(desc.clone(), src.clone(), cfg.decode_queue_blocks),
            desc,
            info: None,
            gain: TrackGain::new(44100.0),
            conv: None,
            queued: VecDeque::new(),
            carry: Vec::new(),
            carry_secs: start,
            decoded_secs: start,
            worker_ended: false,
            failed: None,
            seeking: None,
            load_cmd: None,
            attached: false,
            #[cfg(feature = "http")]
            last_icy: String::new(),
            #[cfg(feature = "http")]
            last_reconnect: crate::http_stream::ReconnectState::Connected,
        }
    }

    fn id(&self) -> &MediaId {
        &self.desc.id
    }

    fn offset(&self) -> f64 {
        self.desc.timeline_offset.as_secs_f64()
    }

    fn rate(&self) -> u32 {
        self.info.as_ref().map_or(44100, |i| i.rate)
    }

    fn duration_secs(&self) -> Option<f64> {
        let info = self.info.as_ref()?;
        info.track.duration.map(|d| d.as_secs_f64()).or_else(|| {
            info.total_frames
                .map(|f| f as f64 / f64::from(info.rate) + self.offset())
        })
    }

    fn exhausted(&self) -> bool {
        self.worker_ended
            && self.queued.is_empty()
            && self.carry.is_empty()
            && self.seeking.is_none()
    }

    /// Collect worker messages. Blocks of the current epoch are queued
    /// (up to `max_queue`); the rest is returned.
    fn poll(&mut self, max_queue: usize) -> Vec<Notice> {
        let mut notices = Vec::new();
        while self.queued.len() < max_queue {
            let Some(msg) = self.worker.try_recv() else {
                break;
            };
            let epoch = self.worker.epoch();
            match msg {
                WorkerMsg::Ready(info) => {
                    let rate = f64::from(info.rate);
                    let start = self.offset() + info.start_frame as f64 / rate;
                    self.carry_secs = start;
                    self.decoded_secs = start;
                    self.info = Some(info);
                    notices.push(Notice::Ready);
                }
                WorkerMsg::Block { epoch: e, block } if e == epoch => {
                    self.decoded_secs = self.offset()
                        + (block.start_frame + block.frames() as u64) as f64
                            / f64::from(block.rate);
                    self.queued.push_back(block);
                }
                WorkerMsg::Block { .. } => {}
                WorkerMsg::Seeked { epoch: e, frame } if e == epoch => {
                    let t = self.offset() + frame as f64 / f64::from(self.rate());
                    self.carry_secs = t;
                    self.decoded_secs = t;
                    notices.push(Notice::Seeked);
                }
                WorkerMsg::SeekFailed { epoch: e, error } if e == epoch => {
                    notices.push(Notice::SeekFailed(error));
                }
                WorkerMsg::End { epoch: e } if e == epoch => {
                    self.worker_ended = true;
                }
                WorkerMsg::Failed { epoch: e, error } if e == epoch => {
                    self.worker_ended = true;
                    notices.push(Notice::Failed(error));
                }
                // Stale: from before the last seek.
                _ => {}
            }
        }
        notices
    }

    /// Move one queued block into `carry`, converted. False when nothing
    /// is queued. At the worker's end the converter's tail is added.
    fn refill(&mut self) -> bool {
        let Some(block) = self.queued.pop_front() else {
            if self.worker_ended
                && let Some(conv) = self.conv.as_mut()
            {
                let tail = conv.flush();
                if !tail.is_empty() {
                    if self.carry.is_empty() {
                        self.carry_secs = self.decoded_secs;
                    }
                    self.carry.extend(tail);
                    return true;
                }
            }
            return false;
        };
        if self.carry.is_empty() {
            self.carry_secs = self.offset() + block.start_frame as f64 / f64::from(block.rate);
        }
        let samples = match self.conv.as_mut() {
            Some(c) => {
                if !c.matches(block.rate, &block.layout) {
                    // Mid-stream format change: rebuild the conversion to
                    // the same pipeline format.
                    let (r, ch) = c.target();
                    if let Ok(n) = Converter::new(block.rate, block.layout, r, ch) {
                        *c = n;
                    }
                }
                c.convert(block.samples)
            }
            None => block.samples,
        };
        self.carry.extend(samples);
        true
    }

    /// Take the carry (at most `limit` frames of it).
    fn take_carry(&mut self, channels: usize, rate: u32, limit: Option<usize>) -> (Vec<f32>, f64) {
        let chunk = match limit {
            Some(frames) => {
                let rest = self.carry.split_off(frames * channels.max(1));
                std::mem::replace(&mut self.carry, rest)
            }
            None => std::mem::take(&mut self.carry),
        };
        let start = self.carry_secs;
        self.carry_secs += (chunk.len() / channels.max(1)) as f64 / f64::from(rate);
        (chunk, start)
    }
}

struct Segment {
    out_start: u64,
    instance: PlaybackInstance,
    media_secs: f64,
}

struct Pipeline {
    output: Box<dyn OutputStream>,
    info: OutputInfo,
    chain: Chain,
    rate: u32,
    layout: ChannelLayout,
    out_channels: u16,
    written: u64,
    segments: VecDeque<Segment>,
    pending_out: Vec<f32>,
    finish_requested: bool,
    finished: bool,
}

impl Pipeline {
    fn audible_out(&self) -> u64 {
        let lat = (self.output.latency().as_secs_f64() * f64::from(self.rate)) as u64;
        let pending = (self.pending_out.len() / usize::from(self.out_channels.max(1))) as u64;
        self.written
            .saturating_sub(self.output.buffered_frames() + pending + lat)
    }

    fn audible(&mut self) -> Option<(PlaybackInstance, f64)> {
        let out = self.audible_out();
        while self.segments.len() > 1 && self.segments[1].out_start <= out {
            self.segments.pop_front();
        }
        let s = self.segments.front()?;
        let secs = s.media_secs + out.saturating_sub(s.out_start) as f64 / f64::from(self.rate);
        Some((s.instance, secs.max(0.0)))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EndAction {
    Finished,
    RequestNext,
    StartNext,
}

struct Core {
    cfg: EngineConfig,
    src: SourceConfig,
    backend: Box<dyn OutputBackend>,
    sh: Shared,
    settings: AudioSettings,
    eq_active: EqCurve,
    eq_auto_for: Option<MediaId>,
    pipeline: Option<Pipeline>,
    current: Option<Media>,
    next_desc: Option<MediaDescriptor>,
    next: Option<Media>,
    next_failed: bool,
    state: PlayerState,
    paused: bool,
    instance_seq: u64,
    last_error: Option<PlayerError>,
    /// Transitions waiting to become audible.
    pending_loaded: VecDeque<(u64, PlaybackInstance, TrackInfo)>,
    /// The media the snapshot and position describe.
    shown: Option<(PlaybackInstance, TrackInfo)>,
    end_action: Option<EndAction>,
    crossfading: bool,
    untouched: bool,
    stop_after_current: bool,
    output_reopens: u32,
    reopen: Option<(Instant, u32, f64)>,
    capture: Mutex<AudioCaptureBuffer>,
    vis_scratch: Vec<f32>,
    fft: Arc<dyn rustfft::Fft<f32>>,
    meter: PeakRmsMeter,
    last_position: Instant,
    last_snapshot: Instant,
    last_viz: Instant,
    seq: u64,
    dirty: bool,
    /// Events of this turn, released after the snapshot that reflects
    /// them is published: a consumer never sees an event the snapshot
    /// does not show yet.
    outbox: std::cell::RefCell<Vec<AudioEvent>>,
}

fn layout_for(channels: u16) -> ChannelLayout {
    ChannelLayout::default_for(channels).unwrap_or(ChannelLayout::STEREO)
}

impl Core {
    fn new(cfg: EngineConfig, backend: Box<dyn OutputBackend>, sh: Shared) -> Self {
        let mut planner = rustfft::FftPlanner::<f32>::new();
        let src = SourceConfig {
            #[cfg(feature = "http")]
            http: cfg.http,
        };
        let settings = AudioSettings::default();
        Self {
            src,
            backend,
            sh,
            eq_active: settings.eq,
            eq_auto_for: None,
            settings,
            pipeline: None,
            current: None,
            next_desc: None,
            next: None,
            next_failed: false,
            state: PlayerState::Idle,
            paused: false,
            instance_seq: 0,
            last_error: None,
            pending_loaded: VecDeque::new(),
            shown: None,
            end_action: None,
            crossfading: false,
            untouched: false,
            stop_after_current: false,
            output_reopens: 0,
            reopen: None,
            capture: Mutex::new(AudioCaptureBuffer::new(CAPTURE_FRAMES * 2)),
            vis_scratch: vec![0.0; CAPTURE_FRAMES * 2],
            fft: planner.plan_fft_forward(FFT_SIZE),
            meter: PeakRmsMeter::new(44100.0),
            last_position: Instant::now(),
            last_snapshot: Instant::now(),
            last_viz: Instant::now(),
            seq: 0,
            dirty: true,
            outbox: Default::default(),
            cfg,
        }
    }

    fn emit(&self, e: AudioEvent) {
        self.outbox.borrow_mut().push(e);
    }

    fn complete(
        &self,
        id: CommandId,
        instance: Option<PlaybackInstance>,
        result: Result<(), PlayerError>,
    ) {
        if id.0 != 0 {
            self.emit(AudioEvent::CommandCompleted {
                id,
                instance,
                result,
            });
        }
    }

    fn set_state(&mut self, s: PlayerState) {
        if self.state != s {
            self.state = s;
            self.dirty = true;
        }
    }

    fn active(&self) -> bool {
        let preparing = |m: &Option<Media>| {
            m.as_ref()
                .is_some_and(|m| m.info.is_none() && m.failed.is_none())
        };
        (self.current.is_some() && !self.paused)
            || preparing(&self.current)
            || preparing(&self.next)
            || self.current.as_ref().is_some_and(|c| c.seeking.is_some())
            || self.pipeline.as_ref().is_some_and(|p| p.finish_requested)
            || self.reopen.is_some()
    }

    fn new_instance(&mut self) -> PlaybackInstance {
        self.instance_seq += 1;
        PlaybackInstance(self.instance_seq)
    }

    // ---------------------------------------------------------------
    // Parameters

    fn params(&self) -> crate::dsp::DspParams {
        self.settings.dsp_params(&self.eq_active)
    }

    fn apply_params(&mut self) {
        let p = self.params();
        let vol = self.settings.device_volume();
        if let Some(pl) = self.pipeline.as_mut() {
            pl.chain.set_params(&p);
            pl.output.set_volume(vol);
        }
        for m in [self.current.as_mut(), self.next.as_mut()]
            .into_iter()
            .flatten()
        {
            m.gain.update(&p);
        }
        self.dirty = true;
    }

    fn echo_eq(&self) {
        self.emit(AudioEvent::EqualizerUpdated(
            self.settings.eq_enabled,
            self.eq_active.gains_db.to_vec(),
        ));
        self.emit(AudioEvent::EqualizerPreampUpdated(self.eq_active.preamp_db));
    }

    /// Pick the curve for `id`: its auto preset when AUTO is on, else
    /// the global curve.
    fn resolve_eq(&mut self, id: Option<&MediaId>) {
        let preset = id.and_then(|id| self.settings.eq_auto.resolve(id.as_str()));
        let (curve, auto_for) = match preset {
            Some(c) => (c, id.cloned()),
            None => (self.settings.eq, None),
        };
        let changed_auto = auto_for != self.eq_auto_for;
        let changed_curve = curve != self.eq_active;
        self.eq_active = curve;
        self.eq_auto_for = auto_for.clone();
        if changed_auto {
            self.emit(AudioEvent::EqAutoApplied(auto_for));
        }
        if changed_curve || changed_auto {
            self.apply_params();
            self.echo_eq();
        }
    }

    /// A hand edit: while an auto preset plays, the edited curve becomes
    /// the global one (the contract of EQ AUTO).
    fn edit_eq(&mut self, edit: impl FnOnce(&mut EqCurve)) {
        let mut curve = self.eq_active;
        edit(&mut curve);
        curve = curve.sanitized();
        self.settings.eq = curve;
        self.eq_active = curve;
        if self.eq_auto_for.take().is_some() {
            self.emit(AudioEvent::EqAutoApplied(None));
        }
        self.apply_params();
    }

    // ---------------------------------------------------------------
    // Commands

    fn command(&mut self, id: CommandId, cmd: AudioCommand) -> bool {
        use AudioCommand::*;
        match cmd {
            Load { media, paused } => self.load(id, media, paused),
            Resume => self.resume(id),
            Pause => self.pause(id),
            Stop => {
                self.stop_all();
                self.set_state(PlayerState::Idle);
                self.emit(AudioEvent::Stopped);
                self.complete(id, None, Ok(()));
            }
            Seek(pos) => self.seek(id, pos),
            SetNext(desc) => {
                self.set_next(desc);
                self.complete(id, None, Ok(()));
            }
            Next | Previous => {
                let is_next = matches!(cmd, Next);
                self.stop_all();
                self.set_state(PlayerState::Idle);
                self.emit(if is_next {
                    AudioEvent::RequestNext
                } else {
                    AudioEvent::RequestPrevious
                });
            }
            SetVolume(v) => {
                self.settings.volume = if v.is_finite() {
                    v.clamp(0.0, crate::MAX_VOLUME)
                } else {
                    1.0
                };
                self.apply_params();
                self.emit(AudioEvent::VolumeUpdated(
                    self.settings.volume,
                    self.settings.muted,
                ));
            }
            SetMute(m) => {
                self.settings.muted = m;
                self.apply_params();
                self.emit(AudioEvent::VolumeUpdated(self.settings.volume, m));
            }
            SetBalance(b) => {
                self.settings.balance = if b.is_finite() {
                    b.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                self.apply_params();
                self.emit(AudioEvent::BalanceUpdated(self.settings.balance));
            }
            SetRepeatMode(mode) => {
                if mode != self.settings.repeat {
                    // The prepared successor depends on the mode.
                    self.next = None;
                    self.next_failed = false;
                }
                self.settings.repeat = mode;
                self.dirty = true;
                self.emit(AudioEvent::RepeatModeUpdated(mode));
            }
            SetStopAfterCurrent(armed) => {
                self.stop_after_current = armed;
                if armed {
                    self.next = None;
                }
                self.dirty = true;
            }
            SetShuffle(on) => self.emit(AudioEvent::ShuffleUpdated(on)),
            SetEqualizerEnabled(on) => {
                self.settings.eq_enabled = on;
                self.apply_params();
                self.echo_eq();
            }
            SetEqualizerBand(i, db) => {
                if i < 10 {
                    self.edit_eq(|c| c.gains_db[i] = db);
                }
                self.emit(AudioEvent::EqualizerUpdated(
                    self.settings.eq_enabled,
                    self.eq_active.gains_db.to_vec(),
                ));
            }
            SetEqualizerBands(gains) => {
                self.edit_eq(|c| {
                    for (d, s) in c.gains_db.iter_mut().zip(&gains) {
                        *d = *s;
                    }
                });
                self.emit(AudioEvent::EqualizerUpdated(
                    self.settings.eq_enabled,
                    self.eq_active.gains_db.to_vec(),
                ));
            }
            ResetEqualizer => {
                self.edit_eq(|c| c.gains_db = [0.0; 10]);
                self.emit(AudioEvent::EqualizerUpdated(
                    self.settings.eq_enabled,
                    self.eq_active.gains_db.to_vec(),
                ));
            }
            SetEqualizerPreamp(db) => {
                self.edit_eq(|c| c.preamp_db = db);
                self.emit(AudioEvent::EqualizerPreampUpdated(self.eq_active.preamp_db));
            }
            SetCrossfade(on, secs) => {
                self.settings.crossfade.enabled = on;
                self.settings.crossfade.seconds = if secs.is_finite() {
                    secs.clamp(0.5, 30.0)
                } else {
                    3.0
                };
                self.dirty = true;
            }
            SetReplayGainEnabled(on) => {
                self.settings.replaygain_enabled = on;
                self.apply_params();
                self.emit(AudioEvent::ReplayGainUpdated(on));
            }
            SetReplayGainMode(mode) => {
                // Selecting a mode implies on; Off implies off.
                self.settings.replaygain_enabled = mode != crate::ReplayGainMode::Off;
                if mode != crate::ReplayGainMode::Off {
                    self.settings.replaygain_mode = mode;
                }
                self.apply_params();
                self.emit(AudioEvent::ReplayGainUpdated(
                    self.settings.replaygain_enabled,
                ));
            }
            SetReplayGainPreventClipping(on) => {
                self.settings.replaygain_prevent_clipping = on;
                self.apply_params();
            }
            SetMono(on) => {
                self.settings.mono = on;
                self.apply_params();
            }
            SetLoudnessEnabled(on) => {
                self.settings.loudness = on;
                self.apply_params();
                self.emit(AudioEvent::LoudnessUpdated(on));
            }
            SetOutputDevice(name) => {
                self.settings.output_device = name.clone();
                self.emit(AudioEvent::OutputDeviceUpdated(name));
            }
            SetEqAuto(on) => {
                self.settings.eq_auto.enabled = on;
                let id = self.shown_id();
                self.resolve_eq(id.as_ref());
                self.dirty = true;
            }
            SetEqAutoPreset(media, curve) => {
                match curve {
                    Some(c) => {
                        self.settings
                            .eq_auto
                            .presets
                            .insert(media.as_str().to_string(), c.sanitized());
                    }
                    None => {
                        self.settings.eq_auto.presets.remove(media.as_str());
                    }
                }
                if self.shown_id().as_ref() == Some(&media) {
                    self.resolve_eq(Some(&media));
                }
                self.dirty = true;
            }
            ApplySettings(s) => {
                let s = s.sanitized();
                let repeat_changed = s.repeat != self.settings.repeat;
                self.settings = s;
                if repeat_changed {
                    self.next = None;
                }
                self.eq_active = self.settings.eq;
                self.eq_auto_for = None;
                let media = self.shown_id();
                self.resolve_eq(media.as_ref());
                self.apply_params();
                self.echo_eq();
                self.emit(AudioEvent::VolumeUpdated(
                    self.settings.volume,
                    self.settings.muted,
                ));
                self.emit(AudioEvent::BalanceUpdated(self.settings.balance));
                self.emit(AudioEvent::ReplayGainUpdated(
                    self.settings.replaygain_enabled,
                ));
                self.emit(AudioEvent::LoudnessUpdated(self.settings.loudness));
                self.emit(AudioEvent::RepeatModeUpdated(self.settings.repeat));
                self.complete(id, None, Ok(()));
            }
            OutputRouteChanged => {
                let r = self.reopen_output();
                self.complete(id, self.current.as_ref().map(|m| m.instance), r);
            }
            Shutdown => return false,
        }
        true
    }

    fn shown_id(&self) -> Option<MediaId> {
        self.shown
            .as_ref()
            .map(|(_, t)| t.id.clone())
            .or_else(|| self.current.as_ref().map(|m| m.id().clone()))
    }

    /// Commands still waiting on a media that is being dropped get their
    /// answer now: cancelled.
    fn cancel_pending(&mut self) {
        if let Some(c) = self.current.as_mut() {
            let instance = c.instance;
            let pending = [c.load_cmd.take(), c.seeking.take().and_then(|s| s.1)];
            for id in pending.into_iter().flatten() {
                self.complete(id, Some(instance), Err(PlayerError::cancelled()));
            }
        }
    }

    fn stop_all(&mut self) {
        self.cancel_pending();
        self.current = None;
        self.next = None;
        self.next_desc = None;
        self.next_failed = false;
        self.pipeline = None;
        self.pending_loaded.clear();
        self.shown = None;
        self.end_action = None;
        self.crossfading = false;
        self.paused = false;
        self.stop_after_current = false;
        self.reopen = None;
        self.dirty = true;
    }

    fn load(&mut self, id: CommandId, desc: MediaDescriptor, paused: bool) {
        self.cancel_pending();
        // A previous media's buffered audio must not play under the new
        // one; the output itself stays open if the format allows it.
        if let Some(p) = self.pipeline.as_mut() {
            p.output.clear();
            p.chain.reset();
            p.segments.clear();
            p.pending_out.clear();
            p.finish_requested = false;
            p.finished = false;
        }
        self.next = None;
        self.next_desc = None;
        self.next_failed = false;
        self.pending_loaded.clear();
        self.end_action = None;
        self.crossfading = false;
        self.stop_after_current = false;
        self.last_error = None;
        self.reopen = None;
        let instance = self.new_instance();
        let mut m = Media::spawn(instance, desc, &self.cfg, &self.src);
        m.load_cmd = Some(id);
        self.current = Some(m);
        self.paused = paused;
        if let Some(p) = self.pipeline.as_mut() {
            p.output.set_paused(true);
        }
        self.set_state(PlayerState::Preparing);
    }

    fn resume(&mut self, id: CommandId) {
        let Some(cur) = self.current.as_mut() else {
            return self.complete(id, None, Err(PlayerError::invalid("nothing loaded")));
        };
        let instance = cur.instance;
        if let Some(err) = cur.failed.clone() {
            if !err.recoverable {
                return self.complete(id, Some(instance), Err(err));
            }
            // Retry from where the listener was (seekable) or rejoin the
            // live edge, under the same instance.
            let pos = self.position_secs();
            self.retry_current(Duration::from_secs_f64(pos), Some(id));
            self.paused = false;
            return;
        }
        match self.state {
            PlayerState::Preparing => {
                self.paused = false;
                self.complete(id, Some(instance), Ok(()));
            }
            PlayerState::Ready
            | PlayerState::Paused
            | PlayerState::Buffering
            | PlayerState::Playing => {
                self.paused = false;
                if let Some(p) = self.pipeline.as_mut() {
                    p.output.set_paused(false);
                }
                let seeking = self.current.as_ref().is_some_and(|c| c.seeking.is_some());
                self.set_state(if seeking {
                    PlayerState::Buffering
                } else {
                    PlayerState::Playing
                });
                self.emit(AudioEvent::Playing);
                self.complete(id, Some(instance), Ok(()));
            }
            _ => self.complete(
                id,
                Some(instance),
                Err(PlayerError::invalid("nothing to resume")),
            ),
        }
    }

    fn pause(&mut self, id: CommandId) {
        let Some(instance) = self.current.as_ref().map(|c| c.instance) else {
            return self.complete(id, None, Err(PlayerError::invalid("nothing loaded")));
        };
        if !self.paused {
            self.paused = true;
            if let Some(p) = self.pipeline.as_mut() {
                p.output.set_paused(true);
            }
            if matches!(self.state, PlayerState::Playing | PlayerState::Buffering) {
                self.set_state(PlayerState::Paused);
                self.emit(AudioEvent::Paused);
            }
        }
        self.complete(id, Some(instance), Ok(()));
    }

    /// Restart the current media's worker at `at` (resume after an
    /// error). Same instance; completion reported on readiness.
    fn retry_current(&mut self, at: Duration, cmd: Option<CommandId>) {
        let Some(old) = self.current.take() else {
            return;
        };
        let seekable = old.info.as_ref().is_some_and(|i| i.seekable);
        let desc = if seekable {
            old.desc
                .clone()
                .starting_at(at.max(old.desc.timeline_offset))
        } else {
            old.desc.clone()
        };
        let mut m = Media::spawn(old.instance, desc, &self.cfg, &self.src);
        m.load_cmd = cmd;
        self.current = Some(m);
        if let Some(p) = self.pipeline.as_mut() {
            p.output.clear();
            p.chain.reset();
            p.segments.clear();
            p.pending_out.clear();
        }
        self.last_error = None;
        self.set_state(PlayerState::Preparing);
    }

    fn seek(&mut self, id: CommandId, pos: Duration) {
        let Some(cur) = self.current.as_ref() else {
            return self.complete(id, None, Err(PlayerError::invalid("nothing loaded")));
        };
        let instance = cur.instance;
        let Some(info) = cur.info.as_ref() else {
            return self.complete(
                id,
                Some(instance),
                Err(PlayerError::invalid(
                    "media still preparing; set start_position instead",
                )),
            );
        };
        if !info.seekable {
            return self.complete(
                id,
                Some(instance),
                Err(PlayerError::unsupported("this media cannot seek")),
            );
        }
        if pos < cur.desc.timeline_offset {
            return self.complete(
                id,
                Some(instance),
                Err(PlayerError::invalid(
                    "position is before the delivered part of the media; request a new delivery",
                )),
            );
        }
        if cur.failed.is_some() {
            return self.retry_current(pos, Some(id));
        }
        let rate = info.rate;
        let frame =
            ((pos - cur.desc.timeline_offset).as_secs_f64() * f64::from(rate)).round() as u64;
        let cur = self.current.as_mut().expect("checked");
        let epoch = cur.worker.seek(frame);
        cur.seeking = Some((epoch, Some(id)));
        cur.queued.clear();
        cur.carry.clear();
        cur.worker_ended = false;
        if let Some(c) = cur.conv.as_mut() {
            c.reset();
        }
        // A fade in progress is abandoned: the incoming media will be
        // prepared again from its start.
        if self.crossfading {
            self.next = None;
            self.crossfading = false;
            let id = self.current.as_ref().map(|c| c.id().clone());
            self.resolve_eq(id.as_ref());
        }
        self.end_action = None;
        if let Some(p) = self.pipeline.as_mut() {
            p.output.clear();
            p.chain.reset();
            p.segments.clear();
            p.pending_out.clear();
            p.finish_requested = false;
            p.finished = false;
        }
        self.pending_loaded.clear();
        if !self.paused {
            self.set_state(PlayerState::Buffering);
        }
    }

    fn set_next(&mut self, desc: Option<MediaDescriptor>) {
        let same = match (&desc, &self.next_desc) {
            (Some(a), Some(b)) => a.id == b.id && a.start_position == b.start_position,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        if self.crossfading {
            // The fade was into the media being replaced: back to the
            // current media's curve, the new successor fades in later.
            self.crossfading = false;
            let id = self.current.as_ref().map(|c| c.id().clone());
            self.resolve_eq(id.as_ref());
        }
        self.next = None;
        self.next_failed = false;
        self.next_desc = desc;
        self.dirty = true;
    }

    /// What follows the current media: itself under Repeat One, else
    /// what the host set.
    fn effective_next(&self) -> Option<MediaDescriptor> {
        if self.stop_after_current {
            return None;
        }
        if self.settings.repeat == RepeatMode::One {
            let cur = self.current.as_ref()?;
            return Some(cur.desc.clone().starting_at(cur.desc.timeline_offset));
        }
        self.next_desc.clone()
    }

    // ---------------------------------------------------------------
    // Pipeline

    /// Format the pipeline needs for `m`: (rate, chain input layout).
    fn pipeline_format(&self, info: &SourceInfo) -> (u32, ChannelLayout) {
        match self.cfg.output_policy {
            OutputPolicy::NativePerMedia => (info.rate, info.layout),
            OutputPolicy::Fixed {
                sample_rate,
                channels,
            } => (sample_rate, layout_for(channels)),
        }
    }

    fn compatible(&self, info: &SourceInfo) -> bool {
        let Some(p) = self.pipeline.as_ref() else {
            return false;
        };
        let (rate, layout) = self.pipeline_format(info);
        p.rate == rate && p.layout == layout
    }

    fn open_pipeline(&mut self, rate: u32, layout: ChannelLayout) -> Result<(), PlayerError> {
        self.pipeline = None;
        let params = self.params();
        let chain = Chain::new(rate, layout, &params)
            .map_err(|e| PlayerError::unsupported(e.to_string()))?;
        let out_channels = chain.output_channels();
        let req = OutputRequest {
            sample_rate: rate,
            channels: out_channels,
            device: self.settings.output_device.clone(),
        };
        let mut output = self.backend.open(&req)?;
        output.set_volume(self.settings.device_volume());
        output.set_paused(self.paused);
        let info = output.info();
        self.meter.set_sample_rate(rate as f32);
        self.emit(AudioEvent::OutputChanged(info.clone()));
        self.pipeline = Some(Pipeline {
            output,
            info,
            chain,
            rate,
            layout,
            out_channels,
            written: 0,
            segments: VecDeque::new(),
            pending_out: Vec::new(),
            finish_requested: false,
            finished: false,
        });
        Ok(())
    }

    /// Set up `m`'s conversion to the pipeline format.
    fn attach(&self, m: &mut Media) -> Result<(), PlayerError> {
        let info = m.info.as_ref().expect("ready");
        let (rate, layout) = self.pipeline_format(info);
        let conv = Converter::new(info.rate, info.layout, rate, layout.channels())?;
        m.conv = (!conv.is_identity()).then_some(conv);
        m.gain = TrackGain::new(rate as f32);
        m.gain
            .set_media(info.track.gain, m.desc.listening_context, &self.params());
        m.attached = true;
        Ok(())
    }

    /// The current media became ready: make sure the output fits it and
    /// announce it.
    fn start_current(&mut self) -> Result<(), PlayerError> {
        let mut cur = self.current.take().expect("current");
        let result = (|| {
            let info = cur.info.as_ref().expect("ready");
            if !self.compatible(info) || self.pipeline.as_ref().is_some_and(|p| p.finished) {
                let (rate, layout) = self.pipeline_format(info);
                self.open_pipeline(rate, layout)?;
            }
            self.attach(&mut cur)
        })();
        let instance = cur.instance;
        let track = cur.info.as_ref().map(|i| i.track.clone());
        self.current = Some(cur);
        result?;
        if let Some(p) = self.pipeline.as_mut() {
            p.output.set_paused(self.paused);
        }
        let track = track.expect("ready");
        let id = track.id.clone();
        self.shown = Some((instance, track.clone()));
        self.emit(AudioEvent::TrackLoaded { instance, track });
        self.resolve_eq(Some(&id));
        Ok(())
    }

    fn reopen_output(&mut self) -> Result<(), PlayerError> {
        let Some(p) = self.pipeline.as_ref() else {
            return Ok(());
        };
        let (rate, layout) = (p.rate, p.layout);
        let pos = self.position_secs();
        self.pipeline = None;
        match self.open_pipeline(rate, layout) {
            Ok(()) => {
                self.output_reopens += 1;
                self.reopen = None;
                self.resync_after_reopen(pos);
                Ok(())
            }
            Err(e) => {
                let attempts = self.reopen.map_or(0, |r| r.1) + 1;
                self.reopen = Some((Instant::now() + REOPEN_RETRY, attempts, pos));
                Err(e)
            }
        }
    }

    /// After a new output replaced a lost one: what was queued in the
    /// old one is gone, so a seekable media goes back to what was being
    /// heard; a live one just continues.
    fn resync_after_reopen(&mut self, pos: f64) {
        self.pending_loaded.clear();
        let Some(cur) = self.current.as_mut() else {
            return;
        };
        if let Some(info) = cur.info.as_ref()
            && info.seekable
            && cur.failed.is_none()
        {
            let off = cur.offset();
            let frame = ((pos - off).max(0.0) * f64::from(info.rate)).round() as u64;
            let epoch = cur.worker.seek(frame);
            cur.seeking = Some((epoch, None));
            cur.queued.clear();
            cur.carry.clear();
            cur.worker_ended = false;
            if let Some(c) = cur.conv.as_mut() {
                c.reset();
            }
        }
        self.next = None;
        self.crossfading = false;
    }

    // ---------------------------------------------------------------
    // Turn

    fn tick(&mut self) {
        self.poll_current();
        self.poll_next();
        self.check_output();
        self.fill();
        self.handle_end();
        self.maybe_prepare_next();
        self.track_audible();
        self.publish();
    }

    fn poll_current(&mut self) {
        let max = self.cfg.decode_queue_blocks;
        let Some(cur) = self.current.as_mut() else {
            return;
        };
        let notices = cur.poll(max);
        for n in notices {
            let cur = self.current.as_mut().expect("current");
            let instance = cur.instance;
            match n {
                Notice::Ready => {
                    let cmd = cur.load_cmd.take();
                    match self.start_current() {
                        Ok(()) => {
                            self.complete(cmd.unwrap_or(CommandId(0)), Some(instance), Ok(()));
                            if self.paused {
                                self.set_state(PlayerState::Ready);
                            } else {
                                self.set_state(PlayerState::Playing);
                                self.emit(AudioEvent::Playing);
                            }
                        }
                        Err(e) => self.fail_current(e, cmd),
                    }
                }
                Notice::Seeked => {
                    let (cmd, pos) = (cur.seeking.take().and_then(|s| s.1), cur.carry_secs);
                    self.complete(cmd.unwrap_or(CommandId(0)), Some(instance), Ok(()));
                    self.emit(AudioEvent::Seeked {
                        instance,
                        position: Duration::from_secs_f64(pos.max(0.0)),
                    });
                    if !self.paused {
                        self.set_state(PlayerState::Playing);
                    }
                    self.dirty = true;
                }
                Notice::SeekFailed(e) => {
                    let cmd = cur.seeking.take().and_then(|s| s.1);
                    // The worker went back to decoding where it was; let
                    // the caller know the target was not reached.
                    self.complete(cmd.unwrap_or(CommandId(0)), Some(instance), Err(e.clone()));
                    self.emit(AudioEvent::Error {
                        instance: Some(instance),
                        error: e,
                    });
                    if !self.paused {
                        self.set_state(PlayerState::Playing);
                    }
                }
                Notice::Failed(e) => {
                    let cmd = cur.load_cmd.take().or(cur.seeking.take().and_then(|s| s.1));
                    self.fail_current(e, cmd);
                }
            }
        }
    }

    fn fail_current(&mut self, e: PlayerError, cmd: Option<CommandId>) {
        let instance = self.current.as_ref().map(|c| c.instance);
        if let Some(c) = self.current.as_mut() {
            c.failed = Some(e.clone());
        }
        self.complete(cmd.unwrap_or(CommandId(0)), instance, Err(e.clone()));
        self.last_error = Some(e.clone());
        self.set_state(PlayerState::Error);
        self.emit(AudioEvent::Error { instance, error: e });
    }

    fn poll_next(&mut self) {
        let max = self.cfg.decode_queue_blocks;
        let Some(next) = self.next.as_mut() else {
            return;
        };
        for n in next.poll(max) {
            match n {
                Notice::Ready => {
                    let next = self.next.take().expect("next");
                    let mut next = next;
                    let ok = next.info.as_ref().is_some_and(|i| {
                        matches!(self.cfg.output_policy, OutputPolicy::Fixed { .. })
                            || self.compatible(i)
                    });
                    if ok && let Err(e) = self.attach(&mut next) {
                        next.failed = Some(e);
                    }
                    let id = next.id().clone();
                    match next.failed.clone() {
                        None => self.emit(AudioEvent::NextPrepared { media_id: id }),
                        Some(error) => {
                            self.emit(AudioEvent::NextFailed {
                                media_id: id,
                                error,
                            });
                            self.next_failed = true;
                            self.dirty = true;
                            return;
                        }
                    }
                    self.next = Some(next);
                    self.dirty = true;
                }
                Notice::Failed(error) => {
                    let id = self.next.as_ref().map(|n| n.id().clone()).expect("next");
                    self.next = None;
                    self.next_failed = true;
                    self.dirty = true;
                    self.emit(AudioEvent::NextFailed {
                        media_id: id,
                        error,
                    });
                    return;
                }
                Notice::Seeked | Notice::SeekFailed(_) => {}
            }
        }
    }

    fn check_output(&mut self) {
        if let Some((at, attempts, pos)) = self.reopen
            && Instant::now() >= at
            && self.pipeline.is_none()
        {
            let fmt = self
                .current
                .as_ref()
                .and_then(|c| c.info.as_ref())
                .map(|i| self.pipeline_format(i));
            if let Some((rate, layout)) = fmt {
                match self.open_pipeline(rate, layout) {
                    Ok(()) => {
                        self.output_reopens += 1;
                        self.reopen = None;
                        self.resync_after_reopen(pos);
                    }
                    Err(e) if attempts >= REOPEN_ATTEMPTS => {
                        self.reopen = None;
                        self.fail_current(e.with_recoverable(true), None);
                    }
                    Err(_) => {
                        self.reopen = Some((Instant::now() + REOPEN_RETRY, attempts + 1, pos))
                    }
                }
            } else {
                self.reopen = None;
            }
        }
        let fault = self.pipeline.as_mut().and_then(|p| p.output.take_fault());
        if let Some(f) = fault {
            let msg = match &f {
                OutputFault::DeviceLost(m) | OutputFault::Failed(m) => m.clone(),
                OutputFault::RouteChanged => "audio route changed".into(),
            };
            if !matches!(f, OutputFault::RouteChanged) {
                self.emit(AudioEvent::Error {
                    instance: self.current.as_ref().map(|c| c.instance),
                    error: PlayerError::new(ErrorCategory::Output, msg),
                });
            }
            if self.reopen_output().is_err() {
                self.pipeline = None;
            }
        }
    }

    fn position_secs(&mut self) -> f64 {
        let fallback = self.current.as_ref().map_or(0.0, |c| c.carry_secs);
        let shown = self.shown.as_ref().map(|s| s.0);
        match self.pipeline.as_mut().and_then(|p| p.audible()) {
            Some((inst, secs)) if Some(inst) == shown => secs,
            _ => fallback,
        }
    }

    /// Produce and write chunks while the output wants them.
    fn fill(&mut self) {
        if self.paused {
            return;
        }
        let Some(cur) = self.current.as_ref() else {
            return;
        };
        if !cur.attached || cur.failed.is_some() || cur.seeking.is_some() {
            return;
        }
        for _ in 0..FILL_BUDGET {
            let Some(p) = self.pipeline.as_mut() else {
                return;
            };
            if p.finish_requested {
                if !p.finished && p.pending_out.is_empty() {
                    p.output.finish();
                    p.finished = true;
                }
                if !p.pending_out.is_empty() {
                    let n = p.output.write(&p.pending_out);
                    p.pending_out.drain(..n);
                }
                return;
            }
            if !p.pending_out.is_empty() {
                let n = p.output.write(&p.pending_out);
                p.pending_out.drain(..n);
                if !p.pending_out.is_empty() {
                    return;
                }
            }
            if !p.output.wants_data() {
                return;
            }
            match self.next_chunk() {
                Some((chunk, secs, instance)) => {
                    if self.state == PlayerState::Buffering {
                        self.set_state(PlayerState::Playing);
                        self.emit(AudioEvent::Playing);
                    }
                    self.write_chunk(chunk, secs, instance);
                }
                None => {
                    let starving = self.current.as_ref().is_some_and(|c| !c.worker_ended);
                    if starving
                        && let Some(p) = self.pipeline.as_ref()
                        && (p.output.buffered_frames() as f64) < BUFFERING_SECS * f64::from(p.rate)
                        && self.state == PlayerState::Playing
                    {
                        self.set_state(PlayerState::Buffering);
                        self.emit(AudioEvent::Buffering);
                    }
                    return;
                }
            }
        }
    }

    /// Fill a media's carry from its queue; false when it has nothing.
    fn ensure_carry(m: &mut Media) -> bool {
        while m.carry.is_empty() {
            if !m.refill() {
                return false;
            }
        }
        true
    }

    /// Next chunk to process: the current media's carry, mixed with the
    /// incoming media inside the crossfade window. Per-media gains are
    /// applied before mixing.
    fn next_chunk(&mut self) -> Option<(Vec<f32>, f64, PlaybackInstance)> {
        let max = self.cfg.decode_queue_blocks;
        let (rate, ch) = {
            let p = self.pipeline.as_ref()?;
            (p.rate, usize::from(p.layout.channels()))
        };
        let xf_secs = f64::from(self.settings.crossfade.seconds);
        let xf_on = self.settings.crossfade.enabled
            && !self.stop_after_current
            && self.settings.repeat != RepeatMode::One
            && self
                .next
                .as_ref()
                .is_some_and(|n| n.attached && n.failed.is_none());

        let cur = self.current.as_mut()?;
        if !Self::ensure_carry(cur) {
            // Pull fresh blocks from the worker before giving up. Only
            // blocks can arrive here: notices were handled this turn.
            let _ = cur.poll(max);
            if !Self::ensure_carry(cur) {
                return None;
            }
        }
        let remaining = cur.duration_secs().map(|d| d - cur.carry_secs);
        let instance = cur.instance;
        // Cut the chunk where the fade starts, so the overlap is exactly
        // the fade length rather than rounded to the decoder's blocks.
        let half_frame = 0.5 / f64::from(rate);
        let mut limit = None;
        if xf_on
            && let Some(r) = remaining
            && r > xf_secs + half_frame
        {
            let until = ((r - xf_secs) * f64::from(rate)).round() as usize;
            if until < cur.carry.len() / ch {
                limit = Some(until);
            }
        }
        let (mut chunk, secs) = cur.take_carry(ch, rate, limit);
        cur.gain.process(&mut chunk, ch);

        let fade = match remaining {
            Some(r) if xf_on && limit.is_none() && r > 0.0 && r < xf_secs + half_frame => Some((
                ((xf_secs - r) / xf_secs).max(0.0),
                1.0 / (xf_secs * f64::from(rate)),
            )),
            _ => None,
        };
        if let Some((pos0, step)) = fade {
            let first = !self.crossfading;
            let next = self.next.as_mut().expect("checked");
            while next.carry.len() < chunk.len() {
                if !next.refill() {
                    let _ = next.poll(max);
                    if !next.refill() {
                        break;
                    }
                }
            }
            let take = chunk.len().min(next.carry.len());
            let mut incoming: Vec<f32> = next.carry.drain(..take).collect();
            next.carry_secs += (take / ch) as f64 / f64::from(rate);
            next.gain.process(&mut incoming, ch);
            for (i, frame) in chunk.chunks_mut(ch).enumerate() {
                let pos = (pos0 + i as f64 * step).clamp(0.0, 1.0) as f32;
                let fade_in = (pos * std::f32::consts::FRAC_PI_2).sin();
                let fade_out = ((1.0 - pos) * std::f32::consts::FRAC_PI_2).sin();
                for (j, s) in frame.iter_mut().enumerate() {
                    let inc = incoming.get(i * ch + j).copied().unwrap_or(0.0);
                    *s = *s * fade_out + inc * fade_in;
                }
            }
            if first {
                // The EQ follows the incoming media from the first sample
                // it contributes; the change is ramped by the chain.
                self.crossfading = true;
                let id = self.next.as_ref().map(|n| n.id().clone());
                self.resolve_eq(id.as_ref());
            }
        }
        Some((chunk, secs, instance))
    }

    fn write_chunk(&mut self, mut chunk: Vec<f32>, secs: f64, instance: PlaybackInstance) {
        let untouched_src = self.current.as_ref().is_some_and(|c| {
            c.conv.is_none()
                && c.gain.is_identity()
                && c.info.as_ref().is_some_and(|i| i.source_bits.is_some())
        });
        let src_bits = self
            .current
            .as_ref()
            .and_then(|c| c.info.as_ref())
            .and_then(|i| i.source_bits);
        let crossfading = self.crossfading;
        let Some(p) = self.pipeline.as_mut() else {
            return;
        };
        let rate = p.rate;
        let capture = &self.capture;
        let n = p.chain.process_with_tap(&mut chunk, |s, ch| {
            if let Ok(mut b) = capture.lock() {
                b.update(s, rate, ch);
            }
        });
        chunk.truncate(n);
        self.meter.process(&chunk, p.out_channels);
        let untouched = untouched_src
            && !crossfading
            && p.chain.is_bypassed()
            && src_bits.is_some_and(|b| p.info.device_bits.is_none_or(|d| b <= d));
        self.untouched = untouched;
        p.output.set_exact(untouched);
        let lat = p.chain.latency_frames() as f64 / f64::from(rate);
        p.segments.push_back(Segment {
            out_start: p.written,
            instance,
            media_secs: secs - lat,
        });
        p.written += (n / usize::from(p.out_channels.max(1))) as u64;
        let accepted = p.output.write(&chunk);
        if accepted < chunk.len() {
            p.pending_out.extend_from_slice(&chunk[accepted..]);
        }
    }

    /// Ask the chain for its lookahead tail and the output to flush.
    fn request_finish(&mut self, action: EndAction) {
        if let Some(p) = self.pipeline.as_mut() {
            let in_ch = usize::from(p.layout.channels());
            let mut tail = vec![0.0; p.chain.latency_frames() * in_ch];
            let n = p.chain.process(&mut tail);
            tail.truncate(n);
            p.written += (n / usize::from(p.out_channels.max(1))) as u64;
            p.pending_out.extend_from_slice(&tail);
            p.finish_requested = true;
        }
        self.end_action = Some(action);
    }

    fn handle_end(&mut self) {
        // Finishing: wait for the output to play everything out.
        if let Some(action) = self.end_action {
            let drained = match self.pipeline.as_mut() {
                Some(p) => p.finished && p.pending_out.is_empty() && p.output.is_drained(),
                None => true,
            };
            if !drained {
                return;
            }
            self.end_action = None;
            match action {
                EndAction::StartNext => {
                    let next = self.next.take().expect("prepared next");
                    self.next_desc = None;
                    self.next_failed = false;
                    self.pipeline = None;
                    self.current = Some(next);
                    let instance = self.current.as_ref().map(|c| c.instance);
                    match self.start_current() {
                        Ok(()) => {
                            self.set_state(PlayerState::Playing);
                            self.emit(AudioEvent::Playing);
                        }
                        Err(e) => {
                            let _ = instance;
                            self.fail_current(e, None);
                        }
                    }
                }
                EndAction::Finished | EndAction::RequestNext => {
                    self.current = None;
                    self.next = None;
                    self.pipeline = None;
                    self.shown = None;
                    self.crossfading = false;
                    self.set_state(PlayerState::Ended);
                    self.emit(if action == EndAction::Finished {
                        AudioEvent::Finished
                    } else {
                        AudioEvent::RequestNext
                    });
                }
            }
            return;
        }

        let Some(cur) = self.current.as_ref() else {
            return;
        };
        if !cur.attached || !cur.exhausted() || cur.failed.is_some() {
            return;
        }
        if self.stop_after_current {
            self.stop_after_current = false;
            self.next = None;
            self.request_finish(EndAction::Finished);
            return;
        }
        let next_state = self
            .next
            .as_ref()
            .map(|n| (n.info.is_some(), n.attached, n.failed.is_some()));
        match next_state {
            // Same pipeline format: switch without touching the output.
            Some((true, true, false)) => self.switch_to_next(),
            // Prepared but needs another output format.
            Some((true, false, false)) => self.request_finish(EndAction::StartNext),
            // Still preparing (network): wait, keeping the limiter tail
            // in place so the join stays gapless if it arrives in time.
            Some((false, _, false)) => {
                if self.state == PlayerState::Playing
                    && self.pipeline.as_ref().is_some_and(|p| {
                        (p.output.buffered_frames() as f64) < BUFFERING_SECS * f64::from(p.rate)
                    })
                {
                    self.set_state(PlayerState::Buffering);
                    self.emit(AudioEvent::Buffering);
                }
            }
            _ => {
                let pending_next =
                    self.effective_next().is_some() && !self.next_failed && self.next.is_none();
                if pending_next {
                    // Successor known but not started yet (short media):
                    // start it now and wait.
                    self.spawn_next();
                    return;
                }
                let action = if self.settings.repeat == RepeatMode::All {
                    EndAction::RequestNext
                } else {
                    EndAction::Finished
                };
                self.request_finish(action);
            }
        }
    }

    fn switch_to_next(&mut self) {
        let next = self.next.take().expect("next");
        let instance = next.instance;
        let track = next.info.as_ref().map(|i| i.track.clone()).expect("ready");
        let id = track.id.clone();
        let at = self
            .pipeline
            .as_ref()
            .map_or(0, |p| p.written + p.chain.latency_frames() as u64);
        self.current = Some(next);
        self.next_desc = None;
        self.next_failed = false;
        if !self.crossfading {
            self.resolve_eq(Some(&id));
        }
        self.crossfading = false;
        self.pending_loaded.push_back((at, instance, track));
        self.dirty = true;
    }

    fn spawn_next(&mut self) {
        let Some(desc) = self.effective_next() else {
            return;
        };
        let instance = self.new_instance();
        self.next = Some(Media::spawn(instance, desc, &self.cfg, &self.src));
        self.dirty = true;
    }

    fn maybe_prepare_next(&mut self) {
        if self.next.is_some() || self.next_failed || self.end_action.is_some() {
            return;
        }
        let Some(cur) = self.current.as_ref() else {
            return;
        };
        if !cur.attached || cur.failed.is_some() || self.effective_next().is_none() {
            return;
        }
        let lead = self
            .cfg
            .preload_lead
            .as_secs_f64()
            .max(f64::from(self.settings.crossfade.seconds) + 1.0);
        let due = match cur.duration_secs() {
            Some(d) => d - cur.carry_secs <= lead,
            // Unknown length: only once the source has ended.
            None => cur.worker_ended,
        };
        if due {
            self.spawn_next();
        }
    }

    fn track_audible(&mut self) {
        let Some(p) = self.pipeline.as_mut() else {
            return;
        };
        let out = p.audible_out();
        let _ = p.audible();
        while self
            .pending_loaded
            .front()
            .is_some_and(|(at, _, _)| *at <= out)
        {
            let (_, instance, track) = self.pending_loaded.pop_front().expect("front");
            self.shown = Some((instance, track.clone()));
            self.emit(AudioEvent::TrackLoaded { instance, track });
            self.dirty = true;
        }
    }

    // ---------------------------------------------------------------
    // Publication

    fn publish(&mut self) {
        let now = Instant::now();
        let playing = matches!(self.state, PlayerState::Playing | PlayerState::Buffering);
        if playing && now.duration_since(self.last_position) >= POSITION_EVERY {
            self.last_position = now;
            let position = Duration::from_secs_f64(self.position_secs());
            let duration = self.shown.as_ref().and_then(|s| s.1.duration);
            self.emit(AudioEvent::Position { position, duration });
            self.poll_stream_status();
        }
        if playing && now.duration_since(self.last_viz) >= VIZ_EVERY {
            self.last_viz = now;
            let ok = match self.capture.lock() {
                Ok(b) => {
                    b.snapshot_into(&mut self.vis_scratch);
                    true
                }
                Err(_) => false,
            };
            if ok {
                self.sh.spectrum.store(Arc::new(compute_spectrum(
                    &self.vis_scratch,
                    self.fft.as_ref(),
                )));
                self.sh
                    .waveform
                    .store(Arc::new(compute_waveform(&self.vis_scratch)));
                self.sh.meter.store(Arc::new(self.meter.snapshot()));
            }
        }
        let periodic = self.active() && now.duration_since(self.last_snapshot) >= SNAPSHOT_EVERY;
        let has_events = !self.outbox.borrow().is_empty();
        if self.dirty || periodic || has_events {
            self.last_snapshot = now;
            self.dirty = false;
            let snap = self.build_snapshot();
            self.sh.snapshot.store(Arc::new(snap));
        }
        for e in self.outbox.borrow_mut().drain(..) {
            self.sh.events.push(e);
        }
    }

    #[cfg(feature = "http")]
    fn poll_stream_status(&mut self) {
        use crate::http_stream::ReconnectState;
        let Some(cur) = self.current.as_mut() else {
            return;
        };
        let Some(info) = cur.info.as_ref() else {
            return;
        };
        let mut events = Vec::new();
        if let Some(h) = info.icy.as_ref() {
            let title = h.load_full();
            if !title.is_empty() && title.as_str() != cur.last_icy {
                cur.last_icy = title.as_str().to_string();
                events.push(AudioEvent::IcyMetadata(cur.last_icy.clone()));
            }
        }
        if let Some(h) = info.reconnect.as_ref() {
            let s = **h.load();
            if s != cur.last_reconnect {
                cur.last_reconnect = s;
                events.push(match s {
                    ReconnectState::Reconnecting { attempt } => {
                        AudioEvent::StreamReconnecting { attempt }
                    }
                    ReconnectState::Connected => AudioEvent::StreamReconnected,
                    ReconnectState::Failed => AudioEvent::StreamReconnectFailed,
                });
            }
        }
        for e in events {
            self.emit(e);
        }
    }

    #[cfg(not(feature = "http"))]
    fn poll_stream_status(&mut self) {}

    fn build_snapshot(&mut self) -> PlaybackSnapshot {
        self.seq += 1;
        let position = self.position_secs();
        let shown = self.shown.clone();
        let cur = self.current.as_ref();
        let info = cur.and_then(|c| c.info.as_ref());
        let decoded = cur.map_or(0.0, |c| c.decoded_secs);
        let p = self.pipeline.as_ref();
        let chain = p.map(|p| p.chain.active()).unwrap_or_default();
        let mut processing = PipelineStatus {
            chain,
            crossfading: self.crossfading,
            resampling: cur.is_some_and(|c| c.conv.as_ref().is_some_and(|c| c.resampling())),
            untouched_to_output: self.untouched,
        };
        processing.chain.replaygain_db = cur.map_or(0.0, |c| c.gain.applied_db());
        let next = if let Some(n) = self.next.as_ref() {
            Some(NextStatus {
                media_id: n.id().clone(),
                state: match (&n.failed, n.info.is_some()) {
                    (Some(_), _) => NextState::Failed,
                    (None, true) => NextState::Ready,
                    (None, false) => NextState::Preparing,
                },
            })
        } else {
            self.effective_next().map(|d| NextStatus {
                media_id: d.id,
                state: if self.next_failed {
                    NextState::Failed
                } else {
                    NextState::Pending
                },
            })
        };
        let seekable = info.is_some_and(|i| i.seekable);
        let live = info.is_some_and(|i| i.live);
        PlaybackSnapshot {
            sequence: self.seq,
            instance: shown.as_ref().map(|s| s.0).or(cur.map(|c| c.instance)),
            media_id: shown
                .as_ref()
                .map(|s| s.1.id.clone())
                .or(cur.map(|c| c.id().clone())),
            state: self.state,
            position: Duration::from_secs_f64(position.max(0.0)),
            decoded_position: Duration::from_secs_f64(decoded.max(0.0)),
            duration: shown.as_ref().and_then(|s| s.1.duration),
            available: if cur.is_some() {
                vec![(
                    Duration::from_secs_f64(position.max(0.0)),
                    Duration::from_secs_f64(decoded.max(position).max(0.0)),
                )]
            } else {
                Vec::new()
            },
            capabilities: Capabilities {
                seek: seekable,
                pause: cur.is_some(),
                resume_after_error: seekable || live,
                live,
            },
            source: info.map(|i| SourceFormat {
                sample_rate: i.rate,
                channels: i.layout.channels(),
                layout: i.layout.label(),
                codec: i.track.codec.clone(),
                bits: i.source_bits,
            }),
            output: p.map(|p| p.info.clone()),
            processing,
            eq: EqStatus {
                enabled: self.settings.eq_enabled,
                global: self.settings.eq,
                active: self.eq_active,
                auto_for: self.eq_auto_for.clone(),
            },
            volume: self.settings.volume,
            muted: self.settings.muted,
            next,
            last_error: self.last_error.clone(),
            diagnostics: Diagnostics {
                output: p.map(|p| p.output.diagnostics()).unwrap_or_default(),
                events_dropped: self.sh.events.total_dropped(),
                decode_errors: cur
                    .map_or(0, |c| c.worker.stats.decode_errors.load(Ordering::Relaxed)),
                reconnects: cur.map_or(0, |c| c.worker.stats.reconnects.load(Ordering::Relaxed)),
                output_reopens: self.output_reopens,
            },
        }
    }
}
