//! HTTP(S) streaming `MediaSource` for symphonia, with ICY metadata
//! support.
//!
//! Used for internet-radio (Shoutcast / Icecast) and direct podcast
//! URLs. The stream is *not* seekable — `Seek::seek` always returns
//! `Err`, and `MediaSource::is_seekable` returns `false`. Symphonia
//! handles non-seekable sources fine for forward-only playback;
//! seeking just becomes a no-op surfaced as a clamp at the engine
//! level.
//!
//! ICY handling:
//!
//! - We send `Icy-MetaData: 1` on the request. Servers that recognise
//!   it respond with `icy-metaint: N`, meaning every N bytes of audio
//!   are followed by a `0..16-byte length × 16` metadata block.
//! - The block starts with a single byte L. If L == 0 → no metadata
//!   this round. Otherwise the next `L * 16` bytes contain
//!   ASCII / UTF-8 fields like `StreamTitle='Artist - Title';
//!   StreamUrl='…';`.
//! - We strip the metadata bytes from the audio stream so symphonia
//!   never sees them, and publish the latest `StreamTitle` through an
//!   `ArcSwap<String>` snapshot so the UI can poll it cheaply.

use anyhow::{Context, Result, anyhow};
use arc_swap::ArcSwap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use symphonia::core::io::MediaSource;

/// Snapshot of the latest `StreamTitle` parsed from an ICY block.
/// Empty string before the first block arrives. Published wait-free —
/// UI can poll without contending with the audio thread's reads.
pub type IcySnapshot = Arc<ArcSwap<String>>;

/// Connection lifecycle of the stream — drives the spinner / toast
/// surface in the UI. `Connected` is the healthy steady state;
/// `Reconnecting(n)` means the underlying body returned EOF or an
/// error and we're on the n-th backoff retry; `Failed` means every
/// retry ran out and we're about to bubble an error back to symphonia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectState {
    Connected,
    Reconnecting { attempt: u32 },
    Failed,
}

/// Wait-free snapshot of the current reconnect state. The audio thread
/// polls this each tick and emits an `AudioEvent` when the value
/// changes, so the UI can surface a toast / spinner without ever
/// touching the read path.
pub type ReconnectSnapshot = Arc<ArcSwap<ReconnectState>>;

/// Backoff schedule for stream reconnect attempts (in seconds).
/// 1 → 2 → 5 → 10 is the convention IceCast / Shoutcast clients use:
/// fast enough that a 1 s blip recovers without the user noticing,
/// slow enough on the tail that we don't hammer a flaky server.
///
/// These sleeps happen on a *background* reconnect thread — never on
/// the audio thread — so the cumulative ~18 s no longer freezes
/// playback while we retry (see `spawn_reconnect` / `Read::read`).
const RECONNECT_BACKOFFS_SECS: &[u64] = &[1, 2, 5, 10];

/// How long the `Read` impl blocks waiting for the reconnect thread to
/// hand off a body before falling back to emitting silence. Kept tiny
/// (tens of ms) so the audio thread is never parked for a perceptible
/// time — long enough to coalesce with the decoder's natural pull rate
/// so we don't spin the CPU, short enough to stay responsive.
const RECONNECT_POLL_TIMEOUT: Duration = Duration::from_millis(50);

/// Size of the zero-filled silence buffer the `Read` impl returns while
/// a reconnect is in flight. Returning a small non-empty buffer (rather
/// than `Ok(0)`, which symphonia reads as EOF) keeps the decoder fed
/// with benign padding without busy-looping; symphonia tolerates the
/// junk bytes as undecodable filler and recovers cleanly once the real
/// body resumes.
const RECONNECT_SILENCE_CHUNK: usize = 4096;

/// Live HTTP audio stream that filters ICY metadata blocks out of the
/// byte stream and publishes the latest `StreamTitle` separately.
pub struct HttpStream {
    body: Box<dyn Read + Send + Sync>,
    /// Original URL — held so we can reopen it transparently when the
    /// underlying socket drops. Pure radio streams don't have a
    /// resumable position, so reconnect is just `GET` the URL again.
    url: String,
    /// Bytes between metadata blocks. `None` when the server didn't
    /// honour `Icy-MetaData: 1` — the stream is then a plain audio
    /// pipe.
    meta_interval: Option<usize>,
    /// Bytes left in the current audio chunk before the next metadata
    /// block. Reset to `meta_interval` after parsing a block.
    bytes_until_meta: usize,
    /// Wait-free publication slot for the latest `StreamTitle`. Cloned
    /// once at construction; the audio thread `store()`s on each new
    /// block, UI consumers `load_full()` for the current title.
    icy_title: IcySnapshot,
    /// MIME type from the response's `content-type` header — used by
    /// the caller to seed symphonia's probe `Hint` (e.g.
    /// `audio/mpeg` → `mp3`).
    content_type: Option<String>,
    /// Wait-free reconnect-state snapshot. Updated on every state
    /// transition; the audio thread polls and forwards changes
    /// upstream as `AudioEvent::StreamReconnect`.
    reconnect_state: ReconnectSnapshot,
    /// Guard against double-spawning the reconnect worker. Set `true`
    /// when a background reconnect is in flight; cleared once its
    /// result has been consumed by the read path. Lives in an `Arc` so
    /// the worker thread can clear it on the way out even if the
    /// `HttpStream` outlives a given attempt.
    reconnecting: Arc<AtomicBool>,
    /// Handoff channel from the background reconnect worker. The worker
    /// sends exactly one `ReconnectResult`; the `Read` impl polls this
    /// non-blocking (with a tiny timeout) and swaps in the new body on
    /// success. `None` between attempts — re-armed each time a fresh
    /// worker is spawned.
    ///
    /// Wrapped in a `Mutex` purely to keep `HttpStream: Sync` (symphonia's
    /// `MediaSource` requires it) — `Receiver` is `Send` but not `Sync`.
    /// Every access is through `&mut self`, so we use `get_mut()` and
    /// never actually take the lock on the hot path.
    reconnect_rx: Mutex<Option<Receiver<ReconnectResult>>>,
}

/// What the background reconnect worker hands back to the `Read` impl.
/// On success it carries the freshly opened body plus its ICY interval
/// and content type (mirrors `connect_body`'s tuple); on failure it
/// signals that every backoff was exhausted so the read path can
/// surface a hard error to symphonia.
enum ReconnectResult {
    Connected {
        body: Box<dyn Read + Send + Sync>,
        meta_interval: Option<usize>,
        content_type: Option<String>,
    },
    Failed,
}

impl HttpStream {
    /// Open an HTTP(S) stream. The connection is established
    /// synchronously; on success the returned struct is ready to be
    /// wrapped in `symphonia::core::io::MediaSourceStream`.
    ///
    /// Times out after 15 s on connect to avoid stalling the audio
    /// thread on a dead URL. After connect, the per-read timeout is
    /// generous — radio streams can have multi-second packet gaps
    /// during reconnects.
    pub fn open(url: &str) -> Result<Self> {
        let (body, meta_interval, content_type) = Self::connect_body(url)?;
        let icy_title: IcySnapshot = Arc::new(ArcSwap::from_pointee(String::new()));
        let reconnect_state: ReconnectSnapshot =
            Arc::new(ArcSwap::from_pointee(ReconnectState::Connected));

        Ok(Self {
            body,
            url: url.to_string(),
            bytes_until_meta: meta_interval.unwrap_or(0),
            meta_interval,
            icy_title,
            content_type,
            reconnect_state,
            reconnecting: Arc::new(AtomicBool::new(false)),
            reconnect_rx: Mutex::new(None),
        })
    }

    /// Shared GET path used by `open` (cold start) and `reconnect`
    /// (warm retry). Returns the bits each path needs to wire up its
    /// own `HttpStream` state — body reader + ICY interval + content
    /// type. Errors propagate the same way `open` did before this
    /// helper got factored out, so callers' error context survives.
    #[allow(clippy::type_complexity)]
    fn connect_body(
        url: &str,
    ) -> Result<(Box<dyn Read + Send + Sync>, Option<usize>, Option<String>)> {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(15)))
            .user_agent(format!("OneAmp/{}", env!("CARGO_PKG_VERSION")))
            .build();
        let agent: ureq::Agent = agent.into();

        let response = agent
            .get(url)
            .header("Icy-MetaData", "1")
            .header("Accept", "*/*")
            .call()
            .with_context(|| format!("HTTP GET failed for {}", url))?;

        let meta_interval = response
            .headers()
            .get("icy-metaint")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok());
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let (_parts, body) = response.into_parts();
        let reader = body.into_reader();
        let body: Box<dyn Read + Send + Sync> = Box::new(reader);
        Ok((body, meta_interval, content_type))
    }

    /// Kick off a reconnect on a *background* thread if one isn't
    /// already running. The expensive part — the `sum(RECONNECT_BACKOFFS_SECS)`
    /// ≈ 18 s of cumulative backoff sleeps plus the blocking `GET` —
    /// runs entirely off the audio thread, so `Read::read` is never
    /// parked for seconds during a network blip. The worker walks the
    /// backoff schedule, publishing `Reconnecting { attempt }` before
    /// each sleep, and hands its outcome back over `reconnect_rx`.
    ///
    /// The `reconnecting` flag guards against double-spawning: a body
    /// that keeps erroring on every poll would otherwise launch a new
    /// worker per `read` call. Cloned `Arc`s (url, state, title slot,
    /// flag) keep the worker independent of `&mut self`, so the audio
    /// thread can return immediately.
    fn spawn_reconnect(&mut self) {
        // Already mid-reconnect — leave the in-flight worker to finish.
        if self.reconnecting.swap(true, Ordering::AcqRel) {
            return;
        }

        let (tx, rx) = mpsc::channel::<ReconnectResult>();
        *self.reconnect_rx.get_mut().unwrap() = Some(rx);

        let url = self.url.clone();
        let reconnect_state = self.reconnect_state.clone();
        let reconnecting = self.reconnecting.clone();

        std::thread::Builder::new()
            .name("http-stream-reconnect".into())
            .spawn(move || {
                let mut result = ReconnectResult::Failed;
                for (i, delay_secs) in RECONNECT_BACKOFFS_SECS.iter().enumerate() {
                    reconnect_state.store(Arc::new(ReconnectState::Reconnecting {
                        attempt: (i + 1) as u32,
                    }));
                    std::thread::sleep(Duration::from_secs(*delay_secs));
                    if let Ok((body, meta_interval, content_type)) = Self::connect_body(&url) {
                        result = ReconnectResult::Connected {
                            body,
                            meta_interval,
                            content_type,
                        };
                        break;
                    }
                }

                // Only publish `Connected`/`Failed` here; the read path
                // re-arms `reconnecting` to `false` once it has actually
                // consumed this result, so a slow consumer can't race a
                // second worker into existence before the swap happens.
                match &result {
                    ReconnectResult::Connected { .. } => {
                        reconnect_state.store(Arc::new(ReconnectState::Connected));
                    }
                    ReconnectResult::Failed => {
                        reconnect_state.store(Arc::new(ReconnectState::Failed));
                    }
                }

                // If the receiver was dropped (HttpStream torn down mid
                // reconnect) the send just errors out harmlessly.
                let _ = tx.send(result);
                // Clear the guard last so a fresh failure after a failed
                // reconnect is allowed to spawn another worker.
                reconnecting.store(false, Ordering::Release);
            })
            .expect("spawn http-stream-reconnect thread");
    }

    /// Non-blocking check for a body handed off by the background
    /// reconnect worker. Blocks at most `RECONNECT_POLL_TIMEOUT` (tens
    /// of ms) — never the multi-second backoff — so the audio thread
    /// stays responsive while still parking briefly instead of
    /// busy-spinning between polls.
    ///
    /// - `Ok(true)`  → a fresh body was swapped in; the caller should
    ///   retry the real read.
    /// - `Ok(false)` → still reconnecting; the caller should emit a
    ///   short silence chunk and try again next read.
    /// - `Err(..)`   → every retry was exhausted; surface to symphonia.
    fn poll_reconnect(&mut self) -> std::io::Result<bool> {
        // Scope the receiver borrow to just the poll so the match arms
        // are free to reassign `reconnect_rx`.
        let recv_result = match self.reconnect_rx.get_mut().unwrap().as_ref() {
            Some(rx) => rx.recv_timeout(RECONNECT_POLL_TIMEOUT),
            // No worker armed (shouldn't happen on this path) — treat as
            // still-pending so we emit silence rather than EOF.
            None => return Ok(false),
        };

        match recv_result {
            Ok(ReconnectResult::Connected {
                body,
                meta_interval,
                content_type,
            }) => {
                // Swap the fresh body in. The ICY title snapshot is left
                // untouched — from the listener's perspective the same
                // logical stream continues, so the last-seen title stays
                // valid until the new body's first metadata block lands.
                self.body = body;
                self.meta_interval = meta_interval;
                self.bytes_until_meta = meta_interval.unwrap_or(0);
                self.content_type = content_type;
                *self.reconnect_rx.get_mut().unwrap() = None;
                Ok(true)
            }
            Ok(ReconnectResult::Failed) => {
                *self.reconnect_rx.get_mut().unwrap() = None;
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "HTTP stream reconnect failed after all retries",
                ))
            }
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => {
                // Worker thread vanished without sending (panic on
                // spawn, etc.). Treat as a hard failure.
                *self.reconnect_rx.get_mut().unwrap() = None;
                self.reconnect_state.store(Arc::new(ReconnectState::Failed));
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "HTTP stream reconnect worker disconnected",
                ))
            }
        }
    }

    /// Fill `buf` with a short run of zeroed silence and report how many
    /// bytes were written. Used as backpressure while a reconnect is in
    /// flight: returning a small non-empty buffer keeps symphonia
    /// pulling (a `Ok(0)` would be read as EOF and stop the decoder)
    /// without ever blocking the audio thread on the backoff. The bytes
    /// are undecodable filler the decoder discards; real audio resumes
    /// the moment the worker hands off a new body.
    fn fill_silence(buf: &mut [u8]) -> usize {
        let n = buf.len().min(RECONNECT_SILENCE_CHUNK);
        for b in &mut buf[..n] {
            *b = 0;
        }
        n
    }

    /// Handle to the reconnect-state snapshot. Same wait-free poll
    /// model as `icy_title_handle` — the audio thread reads this each
    /// tick and forwards transitions to the UI as
    /// `AudioEvent::StreamReconnect`.
    pub fn reconnect_state_handle(&self) -> ReconnectSnapshot {
        self.reconnect_state.clone()
    }

    /// Cheap clone of the title-publication slot. Hand this to the UI
    /// (or the audio thread → UI event channel) so a poller can read
    /// the current title without taking any lock.
    pub fn icy_title_handle(&self) -> IcySnapshot {
        self.icy_title.clone()
    }

    /// Server-reported MIME type, useful for `symphonia::core::probe::Hint`.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Map `audio/mpeg` / `audio/aac` / `audio/ogg` / … to the
    /// extension symphonia uses for the same content. Returns `None`
    /// for unknown MIME types — the probe then falls back to
    /// content-sniffing.
    pub fn extension_from_content_type(ct: &str) -> Option<&'static str> {
        // Trim any `;charset=…` suffix the server might attach.
        let primary = ct.split(';').next()?.trim().to_ascii_lowercase();
        Some(match primary.as_str() {
            "audio/mpeg" | "audio/mp3" => "mp3",
            "audio/aac" | "audio/aacp" => "aac",
            "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
            "audio/flac" | "audio/x-flac" => "flac",
            "audio/ogg" | "application/ogg" => "ogg",
            "audio/wav" | "audio/x-wav" => "wav",
            _ => return None,
        })
    }

    /// Read the next metadata block from `body` and overwrite the
    /// published title. `length_byte` is the first byte of the block
    /// — multiplied by 16 to get the payload size. The payload is
    /// ASCII / UTF-8 text like `StreamTitle='...';StreamUrl='...';`.
    /// Malformed blocks (non-UTF-8, missing fields) just leave the
    /// previous title in place.
    fn consume_metadata_block(&mut self, length_byte: u8) -> std::io::Result<()> {
        if length_byte == 0 {
            return Ok(());
        }
        let payload_len = (length_byte as usize) * 16;
        let mut buf = vec![0u8; payload_len];
        self.body.read_exact(&mut buf)?;

        // ICY blocks are conventionally Latin-1 / ASCII. Lossy UTF-8
        // is the safest fallback — we don't fail playback over a
        // stray non-UTF-8 byte in the title.
        let s = String::from_utf8_lossy(&buf);
        if let Some(title) = parse_stream_title(&s) {
            self.icy_title.store(Arc::new(title));
        }
        Ok(())
    }
}

/// Extract `StreamTitle='…'` from an ICY metadata payload. Stops at
/// the first unescaped `';` terminator. Returns `None` when the field
/// is absent.
fn parse_stream_title(payload: &str) -> Option<String> {
    let after = payload.split_once("StreamTitle='")?.1;
    let title = after.split_once("';").map(|(s, _)| s).unwrap_or(after);
    // The trailing zero-pad bytes lofty servers add show up as NUL
    // chars in the payload — trim them so the published title doesn't
    // carry invisible junk.
    let cleaned = title.trim_end_matches('\0').trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // If a reconnect is already in flight, don't even touch the
        // (dead) body — poll the worker for a tiny bounded window and
        // either resume on the fresh body, surface the exhausted-retries
        // error, or emit a short silence chunk as backpressure. This is
        // the path that used to block the audio thread for ~18 s; it now
        // returns within `RECONNECT_POLL_TIMEOUT` at worst.
        if self.reconnect_rx.get_mut().unwrap().is_some() {
            return if self.poll_reconnect()? {
                self.read_once(buf)
            } else {
                Ok(Self::fill_silence(buf))
            };
        }

        // Steady state: read against the live body. On a zero-byte read
        // (server-side EOF / socket close) or a transient connection
        // error we kick off a *background* reconnect and immediately
        // return silence — the audio thread never sleeps on the backoff.
        match self.read_once(buf) {
            Ok(0) => {
                // A radio stream never legitimately ends mid-listen;
                // a zero read means the upstream closed the socket.
                self.spawn_reconnect();
                Ok(Self::fill_silence(buf))
            }
            Ok(n) => Ok(n),
            Err(e) => {
                // We only retry on the transient/connection-class
                // errors symphonia is most likely to see when a
                // network blip kills the body. Hard errors (invalid
                // data inside our own consume_metadata_block, etc.)
                // shouldn't trigger a reconnect.
                use std::io::ErrorKind::*;
                match e.kind() {
                    UnexpectedEof | ConnectionReset | ConnectionAborted | BrokenPipe | TimedOut
                    | Interrupted | WouldBlock => {
                        self.spawn_reconnect();
                        Ok(Self::fill_silence(buf))
                    }
                    _ => Err(e),
                }
            }
        }
    }
}

impl HttpStream {
    /// One pass of the read logic without any reconnect wrapper —
    /// extracted so the `Read::read` impl can retry exactly once
    /// after a successful reconnect without re-implementing the
    /// metadata-boundary handling.
    fn read_once(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // No ICY metadata interleaving — straight pass-through.
        let Some(interval) = self.meta_interval else {
            return self.body.read(buf);
        };

        // Time to consume a metadata block before the next audio
        // chunk. We do this in a *separate* read call so the next
        // call returns audio bytes — symphonia is sensitive to short
        // reads here, but returning 0 would signal EOF, which is
        // worse.
        if self.bytes_until_meta == 0 {
            let mut len_byte = [0u8; 1];
            self.body.read_exact(&mut len_byte)?;
            self.consume_metadata_block(len_byte[0])?;
            self.bytes_until_meta = interval;
        }

        // Cap the read at the audio chunk's remaining bytes so the
        // metadata boundary never lands mid-buffer (which would
        // require splitting the metadata read across two `read`
        // calls).
        let want = buf.len().min(self.bytes_until_meta);
        let n = self.body.read(&mut buf[..want])?;
        self.bytes_until_meta -= n;
        Ok(n)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "HTTP audio streams are not seekable",
        ))
    }
}

impl MediaSource for HttpStream {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Sanity check: a URL must have a scheme symphonia / our HTTP layer
/// can actually open. Returns the canonicalised scheme so the caller
/// can either gate behaviour or reject early. Anything other than
/// `http` / `https` is rejected.
pub fn validate_stream_url(url: &str) -> Result<()> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Ok(())
    } else {
        Err(anyhow!(
            "Only http:// and https:// URLs are supported (got {})",
            url
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stream_title_extracts_content() {
        let payload = "StreamTitle='Artist - Title';StreamUrl='http://x';\0\0\0";
        assert_eq!(
            parse_stream_title(payload),
            Some("Artist - Title".to_string())
        );
    }

    #[test]
    fn parse_stream_title_returns_none_for_empty_title() {
        let payload = "StreamTitle='';StreamUrl='';";
        assert_eq!(parse_stream_title(payload), None);
    }

    #[test]
    fn parse_stream_title_returns_none_for_missing_field() {
        let payload = "OtherField='nope';";
        assert_eq!(parse_stream_title(payload), None);
    }

    #[test]
    fn parse_stream_title_handles_no_terminator() {
        // Some encoders omit the trailing ';
        let payload = "StreamTitle='Just a title'\0\0";
        assert_eq!(
            parse_stream_title(payload),
            Some("Just a title'".to_string())
        );
    }

    #[test]
    fn extension_from_content_type_handles_known_types() {
        assert_eq!(
            HttpStream::extension_from_content_type("audio/mpeg"),
            Some("mp3")
        );
        assert_eq!(
            HttpStream::extension_from_content_type("audio/aac;charset=utf-8"),
            Some("aac")
        );
        assert_eq!(
            HttpStream::extension_from_content_type("AUDIO/OGG"),
            Some("ogg")
        );
    }

    #[test]
    fn extension_from_content_type_returns_none_for_unknown() {
        assert_eq!(HttpStream::extension_from_content_type("text/html"), None);
    }

    #[test]
    fn validate_stream_url_accepts_http_and_https() {
        assert!(validate_stream_url("http://example.com/stream.mp3").is_ok());
        assert!(validate_stream_url("HTTPS://example.com/x").is_ok());
    }

    #[test]
    fn validate_stream_url_rejects_other_schemes() {
        assert!(validate_stream_url("ftp://example.com/x").is_err());
        assert!(validate_stream_url("file:///tmp/x.mp3").is_err());
        assert!(validate_stream_url("rtmp://x").is_err());
    }

    /// Build an `HttpStream` around an arbitrary in-memory body without
    /// touching the network — lets us exercise the read / reconnect
    /// state machine deterministically. The URL is bogus on purpose so
    /// any background reconnect attempt fails fast through the backoff.
    fn stream_from_body(body: Box<dyn Read + Send + Sync>, url: &str) -> HttpStream {
        HttpStream {
            body,
            url: url.to_string(),
            meta_interval: None,
            bytes_until_meta: 0,
            icy_title: Arc::new(ArcSwap::from_pointee(String::new())),
            content_type: None,
            reconnect_state: Arc::new(ArcSwap::from_pointee(ReconnectState::Connected)),
            reconnecting: Arc::new(AtomicBool::new(false)),
            reconnect_rx: Mutex::new(None),
        }
    }

    #[test]
    fn fill_silence_zeroes_and_bounds() {
        // Smaller than the cap → fills the whole buffer with zeros.
        let mut small = vec![0xAAu8; 16];
        assert_eq!(HttpStream::fill_silence(&mut small), 16);
        assert!(small.iter().all(|&b| b == 0));

        // Larger than the cap → capped at RECONNECT_SILENCE_CHUNK.
        let mut big = vec![0xAAu8; RECONNECT_SILENCE_CHUNK + 1024];
        let n = HttpStream::fill_silence(&mut big);
        assert_eq!(n, RECONNECT_SILENCE_CHUNK);
        assert!(big[..n].iter().all(|&b| b == 0));
    }

    #[test]
    fn read_on_eof_returns_silence_without_blocking_then_spawns_reconnect() {
        // Empty body → first read sees EOF (Ok(0)). The read must NOT
        // block on the backoff; it should return a silence chunk
        // immediately and arm a background reconnect.
        let body: Box<dyn Read + Send + Sync> = Box::new(std::io::Cursor::new(Vec::new()));
        // Unroutable URL so the worker's GET fails fast each attempt.
        let mut stream = stream_from_body(body, "http://127.0.0.1:1/never");

        let start = std::time::Instant::now();
        let mut buf = vec![0xAAu8; 8192];
        let n = stream
            .read(&mut buf)
            .expect("read should not error on first EOF");

        // Returned promptly with silence (no multi-second backoff sleep
        // on the audio thread).
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(n, RECONNECT_SILENCE_CHUNK);
        assert!(buf[..n].iter().all(|&b| b == 0));

        // A reconnect worker is now armed.
        assert!(stream.reconnect_rx.lock().unwrap().is_some());
        assert!(stream.reconnecting.load(Ordering::Acquire));
    }

    #[test]
    fn reconnect_failure_surfaces_error_after_retries() {
        // Empty body + unroutable URL: the worker exhausts every backoff
        // and reports Failed, which the read path must surface as a hard
        // io::Error so symphonia stops. We poll `read` until it either
        // errors (expected) or we time out the test.
        let body: Box<dyn Read + Send + Sync> = Box::new(std::io::Cursor::new(Vec::new()));
        let mut stream = stream_from_body(body, "http://127.0.0.1:1/never");

        // Shorten our patience: the real backoff sums to ~18 s, but each
        // poll only blocks RECONNECT_POLL_TIMEOUT, so we loop returning
        // silence until the worker finally reports Failed.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut buf = vec![0u8; 8192];
        loop {
            match stream.read(&mut buf) {
                Ok(n) => {
                    // Pre-failure reads are silence chunks.
                    assert_eq!(n, RECONNECT_SILENCE_CHUNK);
                }
                Err(e) => {
                    assert_eq!(e.kind(), std::io::ErrorKind::ConnectionAborted);
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "reconnect never surfaced a failure"
            );
        }

        // Final published state is Failed.
        assert_eq!(**stream.reconnect_state.load(), ReconnectState::Failed);
    }
}
