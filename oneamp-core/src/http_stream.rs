//! Live HTTP(S) streams (internet radio, direct live feeds) as a
//! symphonia `MediaSource`, with ICY metadata support.
//!
//! Finite files go through [`crate::http_range::RangeSource`] instead;
//! [`crate::source`] decides which from the delivery and the response.
//! A live stream is *not* seekable and has no resumable position: a
//! dropped connection reconnects at the live edge.
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

use crate::access::{Access, AccessProvider};
use crate::error::{ErrorCategory, PlayerError};
use crate::http_client::{HttpConfig, Response};
use crate::media::CancelToken;
use anyhow::{Result, anyhow};
use arc_swap::ArcSwap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReconnectState {
    Connected,
    Reconnecting { attempt: u32 },
    Failed,
}

/// Wait-free snapshot of the current reconnect state. The engine polls
/// it and forwards transitions as events.
pub type ReconnectSnapshot = Arc<ArcSwap<ReconnectState>>;

/// Backoff schedule for stream reconnect attempts (in seconds).
/// 1 → 2 → 5 → 10 is the convention IceCast / Shoutcast clients use:
/// fast enough that a 1 s blip recovers without the user noticing,
/// slow enough on the tail that we don't hammer a flaky server.
///
/// The sleeps happen inside `Read::read`, which only ever runs on a
/// decode worker, never on the engine thread: while a reconnect is in
/// flight the worker produces no PCM and the output plays silence.
/// Nothing is injected into the compressed stream.
const RECONNECT_BACKOFFS_SECS: &[u64] = &[1, 2, 5, 10];

/// Granularity of the cancellable backoff sleep.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Live HTTP audio stream that filters ICY metadata blocks out of the
/// byte stream and publishes the latest `StreamTitle` separately.
pub struct HttpStream {
    body: Box<dyn Read + Send + Sync>,
    agent: ureq::Agent,
    cfg: HttpConfig,
    /// Reconnects re-ask the provider, so an access renewed meanwhile is
    /// picked up. A live stream has no resumable position: reconnecting
    /// joins the live edge.
    provider: Arc<dyn AccessProvider>,
    access: Access,
    /// Bytes between metadata blocks. `None` when the server didn't
    /// honour `Icy-MetaData: 1` — the stream is then a plain audio
    /// pipe.
    meta_interval: Option<usize>,
    /// Bytes left in the current audio chunk before the next metadata
    /// block. Reset to `meta_interval` after parsing a block.
    bytes_until_meta: usize,
    /// Wait-free publication slot for the latest `StreamTitle`.
    icy_title: IcySnapshot,
    /// MIME type from the response's `content-type` header — used to
    /// seed symphonia's probe hint (`audio/mpeg` → `mp3`).
    content_type: Option<String>,
    /// Wait-free reconnect-state snapshot.
    reconnect_state: ReconnectSnapshot,
    /// Set by the owner when the stream is abandoned, so a reconnect
    /// backoff stops early instead of retrying a URL nobody listens to.
    cancel: CancelToken,
}

/// Request headers of a live connection.
pub(crate) fn live_headers() -> Vec<(&'static str, String)> {
    vec![("Icy-MetaData", "1".into()), ("Accept", "*/*".into())]
}

impl HttpStream {
    /// Wrap an already-open response (the probe that found the stream
    /// is live).
    pub(crate) fn from_response(
        resp: Response,
        agent: ureq::Agent,
        cfg: HttpConfig,
        provider: Arc<dyn AccessProvider>,
        access: Access,
        cancel: CancelToken,
    ) -> Self {
        let meta_interval = resp
            .header("icy-metaint")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|&n| n > 0);
        Self {
            content_type: resp.header("content-type").map(str::to_string),
            body: Box::new(resp.body),
            agent,
            cfg,
            provider,
            access,
            bytes_until_meta: meta_interval.unwrap_or(0),
            meta_interval,
            icy_title: Arc::new(ArcSwap::from_pointee(String::new())),
            reconnect_state: Arc::new(ArcSwap::from_pointee(ReconnectState::Connected)),
            cancel,
        }
    }

    /// Re-`GET` after the body dropped, walking the backoff schedule and
    /// publishing `Reconnecting { attempt }` before each wait. `Err`
    /// means every attempt failed, the access was refused, or the stream
    /// was cancelled.
    fn reconnect(&mut self) -> std::io::Result<()> {
        let mut last = PlayerError::new(ErrorCategory::Network, "live stream dropped");
        for (i, delay_secs) in RECONNECT_BACKOFFS_SECS.iter().enumerate() {
            self.reconnect_state
                .store(Arc::new(ReconnectState::Reconnecting {
                    attempt: (i + 1) as u32,
                }));
            let wake = std::time::Instant::now() + Duration::from_secs(*delay_secs);
            while std::time::Instant::now() < wake {
                self.cancel.check().map_err(PlayerError::into_io)?;
                std::thread::sleep(CANCEL_POLL);
            }
            if let Ok(a) = self.provider.access() {
                self.access = a;
            }
            match crate::http_range::request_with_timeout(
                &self.agent,
                self.provider.as_ref(),
                &mut self.access,
                &live_headers(),
                None,
                &self.cfg,
                &self.cancel,
            ) {
                Ok(resp) => {
                    // The ICY title is left untouched — the same logical
                    // stream continues.
                    let meta = resp
                        .header("icy-metaint")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                        .filter(|&n| n > 0);
                    self.content_type = resp.header("content-type").map(str::to_string);
                    self.body = Box::new(resp.body);
                    self.meta_interval = meta;
                    self.bytes_until_meta = meta.unwrap_or(0);
                    self.reconnect_state
                        .store(Arc::new(ReconnectState::Connected));
                    return Ok(());
                }
                Err(e) if e.recoverable && e.category != ErrorCategory::Cancelled => last = e,
                Err(e) => {
                    self.reconnect_state.store(Arc::new(ReconnectState::Failed));
                    return Err(e.into_io());
                }
            }
        }
        self.reconnect_state.store(Arc::new(ReconnectState::Failed));
        Err(PlayerError::new(
            last.category,
            format!("live stream lost after all retries: {}", last.message),
        )
        .into_io())
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
        crate::media::extension_for_mime(ct)
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
                // A live stream never legitimately ends mid-listen: a
                // zero read means the upstream closed the socket.
                Ok(0) => self.reconnect()?,
                Ok(n) => return Ok(n),
                Err(e) => {
                    if PlayerError::from_io(&e).category == ErrorCategory::Cancelled {
                        return Err(e);
                    }
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

pub(crate) fn is_stream_url_str(s: &str) -> bool {
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
    fn stream_from_body(body: Vec<u8>) -> HttpStream {
        HttpStream {
            body: Box::new(std::io::Cursor::new(body)),
            agent: crate::http_client::agent(&HttpConfig::default()),
            cfg: HttpConfig::default(),
            provider: Arc::new(crate::access::StaticAccess::new("http://127.0.0.1:1/never")),
            access: Access::new("http://127.0.0.1:1/never"),
            meta_interval: None,
            bytes_until_meta: 0,
            icy_title: Arc::new(ArcSwap::from_pointee(String::new())),
            content_type: None,
            reconnect_state: Arc::new(ArcSwap::from_pointee(ReconnectState::Connected)),
            cancel: CancelToken::new(),
        }
    }

    #[test]
    fn icy_blocks_are_stripped_and_published() {
        // metaint 4: "abcd" + [len=1]["StreamTitle='T';" padded to 16] + "efgh"
        let mut meta = b"StreamTitle='T';".to_vec();
        meta.resize(16, 0);
        let mut body = b"abcd".to_vec();
        body.push(1);
        body.extend(meta);
        body.extend(b"efgh");
        body.push(0);
        let mut stream = stream_from_body(body);
        stream.meta_interval = Some(4);
        stream.bytes_until_meta = 4;
        let mut out = [0u8; 8];
        let mut got = Vec::new();
        while got.len() < 8 {
            let n = stream.read(&mut out).unwrap();
            got.extend_from_slice(&out[..n]);
        }
        assert_eq!(got, b"abcdefgh");
        assert_eq!(stream.icy_title.load().as_str(), "T");
    }

    #[test]
    fn dropped_radio_body_reconnects_until_cancelled() {
        // A radio body that hits EOF blocks in the reconnect backoff —
        // it never hands symphonia filler bytes — and gives up as soon
        // as the owner cancels.
        let mut stream = stream_from_body(Vec::new());
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

        cancel.cancel();
        let err = reader.join().unwrap().unwrap_err();
        assert_eq!(
            PlayerError::from_io(&err).category,
            ErrorCategory::Cancelled
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
