//! The one HTTP request path both HTTP sources use: cancellable connect,
//! redirects under the access's [`RedirectPolicy`], status
//! classification, and a body read on a pump thread so a stalled socket
//! never parks the caller.

use crate::access::{Access, RedirectPolicy};
use crate::error::{ErrorCategory, PlayerError, redact_url};
use crate::media::CancelToken;
use std::io::Read;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

/// Transport tuning. Defaults suit both a LAN media server and radio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HttpConfig {
    pub connect_timeout: Duration,
    /// Until the response headers arrive.
    pub response_timeout: Duration,
    /// Longest silence inside a body before the connection is treated as
    /// dropped (and resumed).
    pub idle_timeout: Duration,
    /// Size of each ranged request for on-demand media. Bounds what one
    /// request (and its pump thread) can hold.
    pub request_bytes: u64,
    /// Bytes buffered ahead of the decoder per connection.
    pub read_ahead_bytes: usize,
    /// Reconnect attempts after a transfer is cut, before the read fails.
    pub max_retries: u32,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            response_timeout: Duration::from_secs(15),
            idle_timeout: Duration::from_secs(10),
            request_bytes: 1024 * 1024,
            read_ahead_bytes: 512 * 1024,
            max_retries: 4,
        }
    }
}

const PUMP_CHUNK: usize = 16 * 1024;
const MAX_REDIRECTS: usize = 5;
/// Cancellation is noticed within this delay during connect and reads.
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// A response whose body is being read on a pump thread.
pub(crate) struct Response {
    pub status: u16,
    headers: Vec<(String, String)>,
    pub body: PumpReader,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub(crate) fn agent(cfg: &HttpConfig) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(cfg.connect_timeout))
        .timeout_recv_response(Some(cfg.response_timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent(format!("OneAmp/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn origin(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.to_ascii_lowercase();
    let scheme = scheme.to_ascii_lowercase();
    let host = match (scheme.as_str(), host.rsplit_once(':')) {
        ("http", Some((h, "80"))) | ("https", Some((h, "443"))) => h.to_string(),
        _ => host,
    };
    Some((scheme, host))
}

/// Resolve a `Location` against the URL that returned it.
fn resolve(base: &str, location: &str) -> Option<String> {
    if location.contains("://") {
        return Some(location.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if let Some(rest) = location.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    if location.starts_with('/') {
        return Some(format!("{scheme}://{authority}{location}"));
    }
    let path = rest[authority.len()..]
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    let dir = if dir.is_empty() { "/" } else { dir };
    Some(format!("{scheme}://{authority}{dir}{location}"))
}

/// Error for a final (non-redirect) status, or `None` for success-ish
/// statuses the caller interprets (200, 206, 416).
pub(crate) fn status_error(status: u16, url: &str) -> Option<PlayerError> {
    let url = redact_url(url);
    let (category, recoverable) = match status {
        200..=299 | 416 => return None,
        401 => (ErrorCategory::AccessExpired, true),
        403 => (ErrorCategory::AccessDenied, false),
        404 | 410 => (ErrorCategory::NotFound, false),
        408 | 429 | 500..=599 => (ErrorCategory::Network, true),
        _ => (ErrorCategory::InvalidResponse, false),
    };
    Some(
        PlayerError::new(category, format!("HTTP {status} from {url}"))
            .with_recoverable(recoverable),
    )
}

fn transport_error(e: ureq::Error, url: &str) -> PlayerError {
    let url = redact_url(url);
    let category = match &e {
        ureq::Error::Timeout(_) => ErrorCategory::Timeout,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            ErrorCategory::Network
        }
        ureq::Error::BadUri(_) | ureq::Error::RequireHttpsOnly(_) => ErrorCategory::InvalidRequest,
        _ => ErrorCategory::Network,
    };
    // ureq error texts can quote the URL; describe the kind only.
    let kind = match &e {
        ureq::Error::Timeout(t) => format!("timeout ({t})"),
        ureq::Error::HostNotFound => "host not found".into(),
        ureq::Error::ConnectionFailed => "connection failed".into(),
        ureq::Error::Io(io) => format!("I/O error ({:?})", io.kind()),
        other => format!("{:?}", std::mem::discriminant(other)),
    };
    PlayerError::new(category, format!("GET {url}: {kind}"))
}

/// GET `access` with `extra` headers. Follows redirects per policy,
/// returns as soon as `cancel` fires (an in-flight connect is abandoned
/// to its own thread, which ends within the connect/response timeouts).
/// `body_timeout` bounds the whole body transfer (ranged requests); live
/// streams pass `None`.
pub(crate) fn get(
    agent: &ureq::Agent,
    access: &Access,
    extra: &[(&str, String)],
    body_timeout: Option<Duration>,
    cfg: &HttpConfig,
    cancel: &CancelToken,
) -> Result<Response, PlayerError> {
    let issued_origin = origin(&access.url);
    let mut url = access.url.clone();
    for _ in 0..=MAX_REDIRECTS {
        cancel.check()?;
        let same_origin = origin(&url) == issued_origin;
        let mut req = agent.get(&url);
        for (k, v) in extra {
            req = req.header(*k, v.as_str());
        }
        if same_origin || access.redirects == RedirectPolicy::HeadersToAnyOrigin {
            for (k, v) in &access.headers {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        let req = req.config().timeout_recv_body(body_timeout).build();

        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("http-connect".into())
            .spawn(move || {
                let _ = tx.send(req.call());
            })
            .map_err(|e| PlayerError::internal(format!("spawn: {e}")))?;
        let response = loop {
            match rx.recv_timeout(CANCEL_POLL) {
                Ok(r) => break r.map_err(|e| transport_error(e, &url))?,
                Err(RecvTimeoutError::Timeout) => cancel.check()?,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(PlayerError::internal("http connect thread died"));
                }
            }
        };

        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        if (300..400).contains(&status) && status != 304 {
            let location = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                .map(|(_, v)| v.clone());
            let Some(next) = location.and_then(|l| resolve(&url, &l)) else {
                return Err(PlayerError::new(
                    ErrorCategory::InvalidResponse,
                    format!(
                        "HTTP {status} without a usable Location from {}",
                        redact_url(&url)
                    ),
                ));
            };
            if access.redirects == RedirectPolicy::Refuse {
                return Err(PlayerError::new(
                    ErrorCategory::InvalidResponse,
                    format!("redirect refused by policy: {}", redact_url(&url)),
                ));
            }
            let downgrade = url.to_ascii_lowercase().starts_with("https://")
                && !next.to_ascii_lowercase().starts_with("https://");
            if downgrade {
                return Err(PlayerError::new(
                    ErrorCategory::InvalidResponse,
                    format!("refused https → http redirect from {}", redact_url(&url)),
                ));
            }
            url = next;
            continue;
        }
        let (_, body) = response.into_parts();
        return Ok(Response {
            status,
            headers,
            body: PumpReader::spawn(body.into_reader(), cfg, cancel.clone()),
        });
    }
    Err(PlayerError::new(
        ErrorCategory::InvalidResponse,
        format!("too many redirects from {}", access.redacted_url()),
    ))
}

/// Body reader whose blocking socket reads happen on a pump thread and
/// arrive over a bounded channel, so `read` can give up after the idle
/// timeout or on cancellation. Dropping it stops the pump at its next
/// chunk.
///
/// ponytail: a pump stuck in a stalled socket read outlives its reader
/// until the request's body timeout (ranged requests) or the OS drops the
/// connection (live streams). Reclaiming it sooner needs a closable
/// socket (custom ureq transport).
pub(crate) struct PumpReader {
    // Mutex only for `Sync` (symphonia's `MediaSource` bound); the
    // reader is used from one thread, through `get_mut`.
    rx: std::sync::Mutex<Receiver<std::io::Result<Vec<u8>>>>,
    idle: Duration,
    cancel: CancelToken,
    pending: Vec<u8>,
    pos: usize,
}

impl PumpReader {
    pub fn spawn(
        mut body: impl Read + Send + 'static,
        cfg: &HttpConfig,
        cancel: CancelToken,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel((cfg.read_ahead_bytes / PUMP_CHUNK).max(1));
        let _ = std::thread::Builder::new()
            .name("http-pump".into())
            .spawn(move || Self::pump(&mut body, &tx));
        Self {
            rx: std::sync::Mutex::new(rx),
            idle: cfg.idle_timeout,
            cancel,
            pending: Vec::new(),
            pos: 0,
        }
    }

    fn pump(body: &mut impl Read, tx: &SyncSender<std::io::Result<Vec<u8>>>) {
        let mut buf = vec![0u8; PUMP_CHUNK];
        loop {
            let msg = match body.read(&mut buf) {
                Ok(n) => Ok(buf[..n].to_vec()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => Err(e),
            };
            let last = !matches!(&msg, Ok(v) if !v.is_empty());
            if tx.send(msg).is_err() || last {
                return;
            }
        }
    }

    #[cfg(test)]
    pub fn from_reader(body: impl Read + Send + 'static, idle: Duration) -> Self {
        let cfg = HttpConfig {
            idle_timeout: idle,
            ..Default::default()
        };
        Self::spawn(body, &cfg, CancelToken::new())
    }
}

impl Read for PumpReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos == self.pending.len() {
            let deadline = Instant::now() + self.idle;
            let chunk = loop {
                if self.cancel.is_cancelled() {
                    return Err(PlayerError::cancelled().into_io());
                }
                let rx = self.rx.get_mut().unwrap_or_else(|e| e.into_inner());
                match rx.recv_timeout(CANCEL_POLL.min(self.idle)) {
                    Ok(chunk) => break chunk?,
                    Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => continue,
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "HTTP body idle",
                        ));
                    }
                    // The pump already reported EOF / an error and exited.
                    Err(RecvTimeoutError::Disconnected) => return Ok(0),
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_and_resolution() {
        assert_eq!(
            origin("https://Host:443/a?b"),
            Some(("https".into(), "host".into()))
        );
        assert_ne!(origin("https://host/a"), origin("https://cdn/a"));
        assert_ne!(origin("https://host/a"), origin("http://host/a"));
        assert_eq!(
            resolve("https://h:1/a/b?x=1", "/c").as_deref(),
            Some("https://h:1/c")
        );
        assert_eq!(
            resolve("https://h/a/b?x=1", "c?y").as_deref(),
            Some("https://h/a/c?y")
        );
        assert_eq!(
            resolve("https://h/a", "//cdn/x").as_deref(),
            Some("https://cdn/x")
        );
    }

    #[test]
    fn statuses_are_classified() {
        assert_eq!(
            status_error(401, "u").unwrap().category,
            ErrorCategory::AccessExpired
        );
        assert_eq!(
            status_error(403, "u").unwrap().category,
            ErrorCategory::AccessDenied
        );
        assert!(status_error(503, "u").unwrap().recoverable);
        assert!(status_error(206, "u").is_none());
        assert!(
            !status_error(404, "http://h/x?k=SECRET")
                .unwrap()
                .message
                .contains("SECRET")
        );
    }

    #[test]
    fn pump_reader_passes_bytes_then_eof_and_times_out_when_idle() {
        let mut r = PumpReader::from_reader(&b"hello"[..], Duration::from_secs(5));
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"hello");

        struct Stalled(std::sync::Mutex<Receiver<()>>);
        impl Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.0.get_mut().unwrap().recv();
                Ok(0)
            }
        }
        let (_keep, rx) = mpsc::channel();
        let mut r = PumpReader::from_reader(
            Stalled(std::sync::Mutex::new(rx)),
            Duration::from_millis(60),
        );
        let err = r.read(&mut [0u8; 8]).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }
}
