//! Decoder front-ends for the engine loop.
//!
//! Local files decode inline on the engine thread ([`SymphoniaPlayer`]):
//! disk reads are short and seeking needs the decoder at hand.
//!
//! HTTP streams are different — connecting, probing and every body read
//! can wait on the network for seconds. That work runs on a dedicated
//! worker thread which hands decoded PCM over a bounded channel, so the
//! engine loop keeps servicing commands (Stop, volume, …) whatever the
//! server is doing. Abandoning a stream is just dropping its handle.

use crate::http_stream::{HttpStream, IcySnapshot, ReconnectSnapshot};
use crate::symphonia_player::SymphoniaPlayer;
use anyhow::Result;
use crossbeam_channel::{Receiver, TryRecvError, bounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Decoded blocks buffered ahead of the engine. A block is one codec
/// frame (~26 ms of MP3), so this is well under a second of audio: just
/// enough to ride out scheduling jitter without adding stream latency.
const PCM_QUEUE_BLOCKS: usize = 16;

/// Tells the worker's `HttpStream` to stop reconnecting once the engine
/// has dropped its side.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A stream that is still connecting. Poll [`StreamOpening::try_ready`]
/// from the engine loop; drop it to abandon the attempt.
pub(crate) struct StreamOpening {
    pub url: String,
    rx: Receiver<Result<StreamReady>>,
    /// Handed to the `StreamPlayer` once the stream is ready, so the
    /// worker is cancelled by whichever of the two is dropped last.
    cancel: Option<CancelOnDrop>,
}

/// A connected, probed stream: its PCM source plus the wait-free
/// snapshots the engine forwards to the UI.
pub(crate) struct StreamReady {
    pub player: StreamPlayer,
    pub icy: IcySnapshot,
    pub reconnect: ReconnectSnapshot,
}

impl StreamOpening {
    /// Start connecting to `url` on a worker thread.
    pub fn start(url: String) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (ready_tx, rx) = bounded(1);
        let worker_url = url.clone();
        let worker_cancel = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("stream-decode".into())
            .spawn(move || {
                let opened = (|| {
                    let stream = HttpStream::open(&worker_url, worker_cancel)?;
                    let icy = stream.icy_title_handle();
                    let reconnect = stream.reconnect_state_handle();
                    // The response's `content-type` seeds symphonia's
                    // probe hint: a plain `audio/mpeg` shoutcast stream
                    // is MP3 even without a `.mp3` URL suffix.
                    let ext = stream
                        .content_type()
                        .and_then(HttpStream::extension_from_content_type);
                    let decoder = SymphoniaPlayer::load_from_source(Box::new(stream), ext)?;
                    Ok((decoder, icy, reconnect))
                })();
                let (mut decoder, icy, reconnect) = match opened {
                    Ok(parts) => parts,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let (pcm_tx, pcm_rx) = bounded(PCM_QUEUE_BLOCKS);
                let ready = StreamReady {
                    player: StreamPlayer {
                        rx: pcm_rx,
                        sample_rate: decoder.sample_rate(),
                        channels: decoder.channels(),
                        source_bits: decoder.source_bits(),
                        frames: 0,
                        _cancel: None,
                    },
                    icy,
                    reconnect,
                };
                if ready_tx.send(Ok(ready)).is_err() {
                    return;
                }
                drop(ready_tx);
                loop {
                    let block = decoder.decode_next();
                    if matches!(&block, Ok(Some(samples)) if samples.is_empty()) {
                        continue;
                    }
                    // End of stream and errors are both final; so is the
                    // engine dropping its receiver.
                    let last = !matches!(block, Ok(Some(_)));
                    if pcm_tx.send(block).is_err() || last {
                        return;
                    }
                }
            });
        // A failed spawn drops `ready_tx`; `try_ready` reports it.
        drop(spawned);
        Self {
            url,
            rx,
            cancel: Some(CancelOnDrop(cancel)),
        }
    }

    /// `None` while still connecting, then the outcome exactly once.
    pub fn try_ready(&mut self) -> Option<Result<StreamReady>> {
        match self.rx.try_recv() {
            Ok(Ok(mut ready)) => {
                ready.player._cancel = self.cancel.take();
                Some(Ok(ready))
            }
            Ok(Err(e)) => Some(Err(e)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("stream worker exited unexpectedly")))
            }
        }
    }
}

/// Engine-side end of a stream decode worker. Never blocks.
pub(crate) struct StreamPlayer {
    rx: Receiver<Result<Option<Vec<f32>>>>,
    sample_rate: u32,
    channels: u16,
    source_bits: Option<u32>,
    /// Frames handed to the engine so far — a live stream's position.
    frames: u64,
    _cancel: Option<CancelOnDrop>,
}

impl StreamPlayer {
    /// Same contract as [`SymphoniaPlayer::decode_next`], except that an
    /// empty Vec also means "nothing has arrived yet".
    fn decode_next(&mut self) -> Result<Option<Vec<f32>>> {
        match self.rx.try_recv() {
            Ok(Ok(Some(samples))) => {
                self.frames += (samples.len() / self.channels.max(1) as usize) as u64;
                Ok(Some(samples))
            }
            Ok(other) => other,
            Err(TryRecvError::Empty) => Ok(Some(Vec::new())),
            Err(TryRecvError::Disconnected) => Ok(None),
        }
    }
}

/// What the engine decodes from: an inline file decoder or a stream
/// worker.
pub(crate) enum Player {
    File(SymphoniaPlayer),
    Stream(StreamPlayer),
}

impl Player {
    pub fn decode_next(&mut self) -> Result<Option<Vec<f32>>> {
        match self {
            Self::File(p) => p.decode_next(),
            Self::Stream(p) => p.decode_next(),
        }
    }

    /// Live streams have no timeline to seek on.
    pub fn is_seekable(&self) -> bool {
        matches!(self, Self::File(_))
    }

    pub fn seek(&mut self, seconds: f32) -> Result<()> {
        match self {
            Self::File(p) => p.seek(seconds),
            Self::Stream(_) => Err(anyhow::anyhow!("HTTP audio streams are not seekable")),
        }
    }

    pub fn reset_decoder(&mut self) {
        if let Self::File(p) = self {
            p.reset_decoder();
        }
    }

    pub fn current_position(&self) -> f32 {
        match self {
            Self::File(p) => p.current_position(),
            Self::Stream(p) => p.frames as f32 / p.sample_rate.max(1) as f32,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::File(p) => p.sample_rate(),
            Self::Stream(p) => p.sample_rate,
        }
    }

    pub fn channels(&self) -> u16 {
        match self {
            Self::File(p) => p.channels(),
            Self::Stream(p) => p.channels,
        }
    }

    pub fn source_bits(&self) -> Option<u32> {
        match self {
            Self::File(p) => p.source_bits(),
            Self::Stream(p) => p.source_bits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_player(rx: Receiver<Result<Option<Vec<f32>>>>) -> Player {
        Player::Stream(StreamPlayer {
            rx,
            sample_rate: 4,
            channels: 2,
            source_bits: None,
            frames: 0,
            _cancel: None,
        })
    }

    #[test]
    fn stream_player_never_blocks_and_ends_with_its_worker() {
        let (tx, rx) = bounded(4);
        let mut p = stream_player(rx);
        // Nothing decoded yet: an empty block, not a wait and not EOS.
        assert_eq!(p.decode_next().unwrap(), Some(Vec::new()));
        tx.send(Ok(Some(vec![0.0; 8]))).unwrap();
        assert_eq!(p.decode_next().unwrap().unwrap().len(), 8);
        // 8 samples / 2 channels at 4 Hz = 1 s.
        assert_eq!(p.current_position(), 1.0);
        // A worker error is reported once, then the stream is over.
        tx.send(Err(anyhow::anyhow!("boom"))).unwrap();
        assert!(p.decode_next().is_err());
        drop(tx);
        assert_eq!(p.decode_next().unwrap(), None);
    }

    #[test]
    fn failed_open_is_reported_without_blocking_the_caller() {
        let start = std::time::Instant::now();
        // Port 1 on loopback refuses immediately.
        let mut opening = StreamOpening::start("http://127.0.0.1:1/never".into());
        assert!(start.elapsed() < std::time::Duration::from_millis(200));
        let outcome = loop {
            if let Some(res) = opening.try_ready() {
                break res;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(20));
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(outcome.is_err());
    }
}
