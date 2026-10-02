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
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
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
/// The sleeps happen inside `Read::read`. That is fine because an
/// `HttpStream` is only ever read on the stream decode worker (see
/// `stream_player`), never on the engine thread: while a reconnect is
/// in flight the worker simply produces no PCM and the output plays
/// silence. Nothing is injected into the compressed stream.
const RECONNECT_BACKOFFS_SECS: &[u64] = &[1, 2, 5, 10];

/// Granularity of the cancellable backoff sleep.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Longest the body may go silent before we call the connection dead
/// and hand it to the reconnect path. ureq's timeouts only cover the
/// request / response headers (its `recv_body` is a total-duration
/// budget, wrong for an endless radio stream), so a server that stops
/// sending without closing would otherwise park the decode worker
/// forever.
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Chunks buffered between the body pump thread and the reader.
/// 32 × 16 KiB ≈ 0.5 MiB: minutes of 128 kbps audio at most, bounded.
const PUMP_CHUNKS: usize = 32;
const PUMP_CHUNK_SIZE: usize = 16 * 1024;

/// Body reader that does the blocking socket reads on its own thread
/// and hands chunks over a bounded channel, so `read` can give up after
/// `BODY_IDLE_TIMEOUT` of inactivity with `TimedOut` (which the
/// `HttpStream` read path treats as a reconnect trigger).
///
/// ponytail: a pump stuck in a stalled socket read outlives its reader
/// until the OS drops the connection — one parked thread per dead
/// stream. Needs a closable socket (custom ureq transport) to reclaim.
struct PumpReader {
    rx: Mutex<Receiver<std::io::Result<Vec<u8>>>>,
    idle: Duration,
    pending: Vec<u8>,
    pos: usize,
}

impl PumpReader {
    fn spawn(mut body: impl Read + Send + 'static, idle: Duration) -> Self {
        let (tx, rx) = mpsc::sync_channel(PUMP_CHUNKS);
        std::thread::Builder::new()
            .name("http-stream-pump".into())
            .spawn(move || Self::pump(&mut body, &tx))
            .expect("spawn http-stream-pump thread");
        Self {
            rx: Mutex::new(rx),
            idle,
            pending: Vec::new(),
            pos: 0,
        }
    }

    fn pump(body: &mut impl Read, tx: &SyncSender<std::io::Result<Vec<u8>>>) {
        let mut buf = vec![0u8; PUMP_CHUNK_SIZE];
        loop {
            let msg = match body.read(&mut buf) {
                Ok(n) => Ok(buf[..n].to_vec()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => Err(e),
            };
            // Stop after EOF (empty chunk) or an error, or once the
            // reader is gone.
            let last = !matches!(&msg, Ok(v) if !v.is_empty());
            if tx.send(msg).is_err() || last {
                return;
            }
        }
    }
}

impl Read for PumpReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos == self.pending.len() {
            let chunk = match self.rx.get_mut().unwrap().recv_timeout(self.idle) {
                Ok(chunk) => chunk?,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "HTTP stream idle",
                    ));
                }
                // Pump already reported EOF / an error and exited.
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            };
            self.pending = chunk;
            self.pos = 0;
        }
        let n = buf.len().min(self.pending.len() - self.pos);
        buf[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

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
    /// Set by the owner (the engine side of `stream_player`) when the
    /// stream is abandoned, so a reconnect backoff stops early instead
    /// of retrying a URL nobody listens to any more.
    cancel: Arc<AtomicBool>,
    /// The response carried a `Content-Length`: a finite file (podcast
    /// episode), where a zero-byte read is the real end rather than a
    /// dropped radio connection.
    finite: bool,
}

/// What one `GET` yields: the body reader plus the response metadata
/// `open` and `reconnect` both need.
struct Connection {
    body: Box<dyn Read + Send + Sync>,
    meta_interval: Option<usize>,
    content_type: Option<String>,
    finite: bool,
}

impl HttpStream {
    /// Open an HTTP(S) stream. The connection is established
    /// synchronously (up to 15 s for connect and for the response
    /// headers), so call it off the engine thread. After connect, a body
    /// silent for `BODY_IDLE_TIMEOUT` is treated as dropped and
    /// reconnected. `cancel` aborts a reconnect in progress.
    pub fn open(url: &str, cancel: Arc<AtomicBool>) -> Result<Self> {
        let conn = Self::connect_body(url)?;
        Ok(Self {
            body: conn.body,
            url: url.to_string(),
            bytes_until_meta: conn.meta_interval.unwrap_or(0),
            meta_interval: conn.meta_interval,
            icy_title: Arc::new(ArcSwap::from_pointee(String::new())),
            content_type: conn.content_type,
            reconnect_state: Arc::new(ArcSwap::from_pointee(ReconnectState::Connected)),
            cancel,
            finite: conn.finite,
        })
    }

    /// Shared GET path used by `open` (cold start) and `reconnect`
    /// (warm retry).
    fn connect_body(url: &str) -> Result<Connection> {
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

        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
        let meta_interval = header("icy-metaint").and_then(|s| s.parse::<usize>().ok());
        let content_type = header("content-type").map(|s| s.to_string());
        let finite = header("content-length").is_some();

        let (_parts, body) = response.into_parts();
        Ok(Connection {
            body: Box::new(PumpReader::spawn(body.into_reader(), BODY_IDLE_TIMEOUT)),
            meta_interval,
            content_type,
            finite,
        })
    }

    /// Re-`GET` the URL after the body dropped, walking the backoff
    /// schedule and publishing `Reconnecting { attempt }` before each
    /// wait. Blocks the calling (decode worker) thread. `Ok` means a
    /// fresh body is in place; `Err` means every attempt failed or the
    /// stream was cancelled, and is surfaced to symphonia as the end of
    /// the stream.
    fn reconnect(&mut self) -> std::io::Result<()> {
        for (i, delay_secs) in RECONNECT_BACKOFFS_SECS.iter().enumerate() {
            self.reconnect_state
                .store(Arc::new(ReconnectState::Reconnecting {
                    attempt: (i + 1) as u32,
                }));
            let wake = std::time::Instant::now() + Duration::from_secs(*delay_secs);
            while std::time::Instant::now() < wake {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err(std::io::Error::other("HTTP stream cancelled"));
                }
                std::thread::sleep(CANCEL_POLL);
            }
            if let Ok(conn) = Self::connect_body(&self.url) {
                // The ICY title snapshot is left untouched — the same
                // logical stream continues, so the last-seen title stays
                // valid until the new body's first metadata block lands.
                self.body = conn.body;
                self.meta_interval = conn.meta_interval;
                self.bytes_until_meta = conn.meta_interval.unwrap_or(0);
                self.content_type = conn.content_type;
                self.reconnect_state
                    .store(Arc::new(ReconnectState::Connected));
                return Ok(());
            }
        }
        self.reconnect_state.store(Arc::new(ReconnectState::Failed));
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "HTTP stream reconnect failed after all retries",
        ))
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
    let title = match after.split_once("';") {
        Some((s, _)) => s,
        None => {
            // Some encoders omit the trailing `;` but still close the
            // quoted value with a bare `'` before the NUL padding — strip
            // that closing quote so it doesn't leak into the title text.
            let trimmed = after.trim_end_matches('\0');
            trimmed.strip_suffix('\'').unwrap_or(trimmed)
        }
    };
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
        loop {
            match self.read_once(buf) {
                // A finite file (podcast) really ended.
                Ok(0) if self.finite => return Ok(0),
                // A radio stream never legitimately ends mid-listen: a
                // zero read means the upstream closed the socket.
                Ok(0) => self.reconnect()?,
                Ok(n) => return Ok(n),
                Err(e) => {
                    // Only the transient / connection-class errors a
                    // network blip produces are retried. Hard errors
                    // (invalid data in `consume_metadata_block`, …)
                    // propagate.
                    use std::io::ErrorKind::*;
                    match e.kind() {
                        UnexpectedEof | ConnectionReset | ConnectionAborted | BrokenPipe
                        | TimedOut | Interrupted | WouldBlock => self.reconnect()?,
                        _ => return Err(e),
                    }
                }
            }
        }
    }
}

impl HttpStream {
    /// One pass of the read logic without the reconnect wrapper.
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
    if is_stream_url_str(url.trim()) {
        Ok(())
    } else {
        Err(anyhow!(
            "Only http:// and https:// URLs are supported (got {})",
            url
        ))
    }
}

fn is_stream_url_str(s: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        s.get(..scheme.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(scheme))
    })
}

/// Playlist entries, session rows and argv keep stream URLs in a
/// `PathBuf`. This is the one test for "that path is really an HTTP(S)
/// URL" — it must not be joined onto a directory, stat'ed, seeked or
/// handed to the file decoder.
pub fn is_stream_url(path: &std::path::Path) -> bool {
    path.to_str().is_some_and(is_stream_url_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pump_reader_passes_bytes_then_eof_and_times_out_when_idle() {
        let mut r = PumpReader::spawn(&b"hello"[..], Duration::from_secs(5));
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"hello");

        // A body that never sends (sender kept alive, nothing written).
        struct Stalled(Mutex<Receiver<()>>);
        impl Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.0.get_mut().unwrap().recv();
                Ok(0)
            }
        }
        let (_keep, rx) = mpsc::channel();
        let mut r = PumpReader::spawn(Stalled(Mutex::new(rx)), Duration::from_millis(50));
        let err = r.read(&mut [0u8; 8]).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

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
        // Some encoders omit the trailing `;` but still close the quoted
        // value with a bare `'` — that closing quote must not leak into
        // the parsed title.
        let payload = "StreamTitle='Just a title'\0\0";
        assert_eq!(
            parse_stream_title(payload),
            Some("Just a title".to_string())
        );
    }

    #[test]
    fn parse_stream_title_no_terminator_and_no_closing_quote() {
        // Degenerate case: no `;` AND no closing `'` at all. Nothing to
        // strip, so the raw (NUL-trimmed) remainder is used as-is.
        let payload = "StreamTitle='Truncated mid title\0\0";
        assert_eq!(
            parse_stream_title(payload),
            Some("Truncated mid title".to_string())
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
    /// touching the network. The URL is unroutable on purpose so any
    /// reconnect attempt fails fast.
    fn stream_from_body(body: Vec<u8>, finite: bool) -> HttpStream {
        HttpStream {
            body: Box::new(std::io::Cursor::new(body)),
            url: "http://127.0.0.1:1/never".to_string(),
            meta_interval: None,
            bytes_until_meta: 0,
            icy_title: Arc::new(ArcSwap::from_pointee(String::new())),
            content_type: None,
            reconnect_state: Arc::new(ArcSwap::from_pointee(ReconnectState::Connected)),
            cancel: Arc::new(AtomicBool::new(false)),
            finite,
        }
    }

    #[test]
    fn finite_body_ends_instead_of_reconnecting() {
        // A podcast file: bytes, then a genuine EOF. No reconnect, no
        // filler bytes.
        let mut stream = stream_from_body(b"abc".to_vec(), true);
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"abc");
        assert_eq!(**stream.reconnect_state.load(), ReconnectState::Connected);
    }

    #[test]
    fn dropped_radio_body_reconnects_until_cancelled() {
        // A radio body that hits EOF blocks in the reconnect backoff —
        // it never hands symphonia filler bytes — and gives up as soon
        // as the owner cancels.
        let mut stream = stream_from_body(Vec::new(), false);
        let cancel = stream.cancel.clone();
        let state = stream.reconnect_state.clone();
        let reader = std::thread::spawn(move || stream.read(&mut [0u8; 64]));

        let start = std::time::Instant::now();
        while **state.load() == ReconnectState::Connected {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "never reconnected"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(**state.load(), ReconnectState::Reconnecting { attempt: 1 });

        cancel.store(true, Ordering::Relaxed);
        assert!(reader.join().unwrap().is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
