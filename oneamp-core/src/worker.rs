//! One decode worker per media. Opening, probing, the initial seek and
//! every read run here, so the engine thread never waits on a disk or a
//! socket; decoded blocks cross a bounded channel. Dropping the handle
//! cancels the worker: its source sees the cancellation within ~50 ms
//! and the thread exits.

use crate::error::{ErrorCategory, PlayerError};
use crate::media::{CancelToken, MediaDescriptor};
use crate::source::{OpenedSource, SourceConfig};
use crate::symphonia_player::Block;
use crate::track::TrackInfo;
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, select, unbounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[cfg(feature = "http")]
use crate::http_stream::{IcySnapshot, ReconnectSnapshot};

/// What the engine learns once the source is open.
pub(crate) struct SourceInfo {
    pub track: TrackInfo,
    pub rate: u32,
    pub layout: crate::dsp::channels::ChannelLayout,
    pub seekable: bool,
    pub live: bool,
    pub source_bits: Option<u32>,
    pub total_frames: Option<u64>,
    /// Frame playback actually starts at (after the initial seek).
    pub start_frame: u64,
    #[cfg(feature = "http")]
    pub icy: Option<IcySnapshot>,
    #[cfg(feature = "http")]
    pub reconnect: Option<ReconnectSnapshot>,
}

pub(crate) enum WorkerMsg {
    Ready(Box<SourceInfo>),
    Block { epoch: u32, block: Block },
    Seeked { epoch: u32, frame: u64 },
    SeekFailed { epoch: u32, error: PlayerError },
    End { epoch: u32 },
    Failed { epoch: u32, error: PlayerError },
}

enum Ctrl {
    Seek { frame: u64, epoch: u32 },
}

/// Counters the worker updates for diagnostics.
#[derive(Default)]
pub(crate) struct WorkerStats {
    pub decode_errors: AtomicU64,
    pub reconnects: AtomicU32,
}

pub(crate) struct DecoderWorker {
    ctrl: Sender<Ctrl>,
    rx: Receiver<WorkerMsg>,
    cancel: CancelToken,
    epoch: u32,
    /// The thread has exited and that was reported once.
    gone: bool,
    pub stats: Arc<WorkerStats>,
}

impl Drop for DecoderWorker {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn to_frames(d: std::time::Duration, rate: u32) -> u64 {
    (d.as_secs_f64() * f64::from(rate)).round() as u64
}

impl DecoderWorker {
    pub fn spawn(desc: MediaDescriptor, cfg: SourceConfig, queue_blocks: usize) -> Self {
        let cancel = CancelToken::new();
        let (ctrl, ctrl_rx) = unbounded();
        let (tx, rx) = bounded(queue_blocks.max(2));
        let stats = Arc::new(WorkerStats::default());
        let worker_cancel = cancel.clone();
        let worker_stats = stats.clone();
        let spawned = std::thread::Builder::new()
            .name("oneamp-decode".into())
            .spawn(move || run(desc, cfg, worker_cancel, ctrl_rx, tx, worker_stats));
        if let Err(e) = spawned {
            // Report through the channel shape the engine already reads.
            let (tx, rx2) = bounded(1);
            let _ = tx.send(WorkerMsg::Failed {
                epoch: 0,
                error: PlayerError::internal(format!("cannot start decoder thread: {e}")),
            });
            return Self {
                ctrl,
                rx: rx2,
                cancel,
                epoch: 0,
                gone: false,
                stats,
            };
        }
        Self {
            ctrl,
            rx,
            cancel,
            epoch: 0,
            gone: false,
            stats,
        }
    }

    pub fn try_recv(&mut self) -> Option<WorkerMsg> {
        match self.rx.try_recv() {
            Ok(m) => {
                // After a reported failure the thread may exit; its
                // disconnection is then expected, not a second failure.
                if matches!(m, WorkerMsg::Failed { .. }) {
                    self.gone = true;
                }
                Some(m)
            }
            Err(TryRecvError::Empty) => None,
            // The thread ends on its own only after reporting why (open
            // failure); anything else is unexpected — reported once.
            Err(TryRecvError::Disconnected) if !self.gone => {
                self.gone = true;
                Some(WorkerMsg::Failed {
                    epoch: self.epoch,
                    error: PlayerError::internal("decoder thread exited"),
                })
            }
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Ask for a seek; returns the epoch blocks of the new position
    /// carry. Blocks from older epochs are stale.
    pub fn seek(&mut self, frame: u64) -> u32 {
        self.epoch += 1;
        let _ = self.ctrl.send(Ctrl::Seek {
            frame,
            epoch: self.epoch,
        });
        self.epoch
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }
}

fn run(
    desc: MediaDescriptor,
    cfg: SourceConfig,
    cancel: CancelToken,
    ctrl: Receiver<Ctrl>,
    tx: Sender<WorkerMsg>,
    stats: Arc<WorkerStats>,
) {
    let fail = |epoch, error: PlayerError| {
        let _ = tx.send(WorkerMsg::Failed { epoch, error });
    };
    let opened = match crate::source::open(&desc, &cfg, &cancel) {
        Ok(o) => o,
        Err(e) => return fail(0, e),
    };
    let OpenedSource {
        mut player,
        track,
        live,
        #[cfg(feature = "http")]
        icy,
        #[cfg(feature = "http")]
        reconnect,
        reconnects,
    } = opened;
    let rate = player.sample_rate();
    let seekable = player.is_seekable() && !live && desc.seekable != Some(false);

    // Start position: reached before anything is decoded for output.
    if desc.start_position < desc.timeline_offset {
        return fail(
            0,
            PlayerError::invalid("start position is before the delivered part of the media"),
        );
    }
    let start = desc.start_position - desc.timeline_offset;
    let mut start_frame = 0;
    if !start.is_zero() {
        if !seekable {
            return fail(
                0,
                PlayerError::unsupported("cannot start mid-media on a source that cannot seek"),
            );
        }
        match player.seek_to_frame(to_frames(start, rate)) {
            Ok(f) => start_frame = f,
            Err(e) => return fail(0, e),
        }
    }

    let info = SourceInfo {
        rate,
        layout: player.layout(),
        seekable,
        live,
        source_bits: player.source_bits(),
        total_frames: player.total_frames(),
        start_frame,
        track,
        #[cfg(feature = "http")]
        icy,
        #[cfg(feature = "http")]
        reconnect,
    };
    if tx.send(WorkerMsg::Ready(Box::new(info))).is_err() {
        return;
    }

    let mut epoch = 0;
    let mut idle = false; // ended or failed: wait for a seek
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Some(r) = &reconnects {
            stats
                .reconnects
                .store(r.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        // A pending seek wins over decoding.
        let cmd = if idle {
            match ctrl.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        } else {
            ctrl.try_recv().ok()
        };
        if let Some(Ctrl::Seek { frame, epoch: e }) = cmd {
            epoch = e;
            idle = false;
            let msg = match player.seek_to_frame(frame) {
                Ok(f) => WorkerMsg::Seeked { epoch, frame: f },
                Err(error) => WorkerMsg::SeekFailed { epoch, error },
            };
            if tx.send(msg).is_err() {
                return;
            }
            continue;
        }
        let decoded = player.decode_next();
        stats
            .decode_errors
            .store(player.decode_errors, Ordering::Relaxed);
        match decoded {
            Ok(Some(block)) => {
                let msg = WorkerMsg::Block { epoch, block };
                // Wait for room, but let a seek interrupt the wait (the
                // block belongs to the old position then).
                select! {
                    send(tx, msg) -> r => if r.is_err() { return },
                    recv(ctrl) -> c => match c {
                        Ok(Ctrl::Seek { frame, epoch: e }) => {
                            epoch = e;
                            let msg = match player.seek_to_frame(frame) {
                                Ok(f) => WorkerMsg::Seeked { epoch, frame: f },
                                Err(error) => WorkerMsg::SeekFailed { epoch, error },
                            };
                            if tx.send(msg).is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    },
                }
            }
            Ok(None) => {
                if tx.send(WorkerMsg::End { epoch }).is_err() {
                    return;
                }
                idle = true;
            }
            Err(e) if e.category == ErrorCategory::Cancelled || cancel.is_cancelled() => return,
            Err(error) => {
                if tx.send(WorkerMsg::Failed { epoch, error }).is_err() {
                    return;
                }
                idle = true;
            }
        }
    }
}
