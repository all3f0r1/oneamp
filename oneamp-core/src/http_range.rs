//! On-demand HTTP media as a seekable byte source.
//!
//! The resource is read with bounded `Range` requests
//! ([`HttpConfig::request_bytes`] each), so:
//!
//! * a seek drops the current request and the next read asks for the new
//!   offset — time→byte mapping is the demuxer's job (its seek index or
//!   its own bisection), never a duration ratio;
//! * a cut transfer resumes at the exact byte where it stopped, guarded
//!   by `If-Range` with the first response's validator: if the resource
//!   changed in between, the read fails with `ResourceChanged` instead
//!   of splicing two files;
//! * a short body is an interruption, never an end of stream — the end
//!   is the size `Content-Range` announced;
//! * a 401 is answered by one renewal through the [`AccessProvider`].
//!
//! A server that ignores `Range` (plain 200) still plays, forward only:
//! the source then reports itself non-seekable.

use crate::access::{Access, AccessProvider, RenewReason};
use crate::error::{ErrorCategory, PlayerError, redact_url};
use crate::http_client::{self, HttpConfig, PumpReader, Response};
use crate::media::CancelToken;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;
use symphonia::core::io::MediaSource;

/// Forward seeks shorter than this read through the open body instead of
/// issuing a new request.
const SKIP_IN_PLACE: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Validator {
    etag: Option<String>,
    last_modified: Option<String>,
}

impl Validator {
    fn from(resp: &Response) -> Self {
        Self {
            etag: resp.header("etag").map(str::to_string),
            last_modified: resp.header("last-modified").map(str::to_string),
        }
    }

    /// `If-Range` needs a strong validator (a weak ETag never matches).
    fn if_range(&self) -> Option<String> {
        match &self.etag {
            Some(e) if !e.starts_with("W/") => Some(e.clone()),
            _ => self.last_modified.clone(),
        }
    }
}

/// Parse `bytes a-b/total` (total may be `*`).
fn content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (span, total) = rest.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    let (a, b): (u64, u64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    (a <= b && total.is_none_or(|t| b < t)).then_some((a, b, total))
}

struct Body {
    reader: PumpReader,
    /// Absolute offset one past the last byte this request covers
    /// (`None`: until the connection ends — non-ranged mode).
    end: Option<u64>,
}

/// What the first response says about the resource.
pub(crate) struct Probe {
    pub response: Response,
    /// The server honoured `Range`.
    pub ranged: bool,
    pub len: Option<u64>,
}

pub struct RangeSource {
    agent: ureq::Agent,
    cfg: HttpConfig,
    provider: Arc<dyn AccessProvider>,
    access: Access,
    cancel: CancelToken,
    pos: u64,
    len: Option<u64>,
    ranged: bool,
    validator: Validator,
    body: Option<Body>,
    content_type: Option<String>,
    /// Reconnects performed so far (diagnostics).
    pub(crate) reconnects: Arc<std::sync::atomic::AtomicU32>,
    /// Last error handed to the reader. Symphonia's probe turns read
    /// errors into "unsupported format"; the opener reads the real cause
    /// back from here.
    pub(crate) last_error: Arc<std::sync::Mutex<Option<PlayerError>>>,
}

impl RangeSource {
    /// Send the first request: `bytes=0-…`, so the answer tells whether
    /// the server supports ranges and how long the resource is.
    pub(crate) fn probe(
        agent: &ureq::Agent,
        provider: &dyn AccessProvider,
        access: &mut Access,
        extra: &[(&str, String)],
        cfg: &HttpConfig,
        cancel: &CancelToken,
    ) -> Result<Probe, PlayerError> {
        let range = format!("bytes=0-{}", cfg.request_bytes - 1);
        let mut headers = vec![("Range", range)];
        headers.extend(extra.iter().cloned());
        let resp = request(agent, provider, access, &headers, cfg, cancel)?;
        match resp.status {
            206 => {
                let cr = resp.header("content-range").and_then(content_range);
                match cr {
                    Some((0, _, total)) => Ok(Probe {
                        len: total,
                        ranged: true,
                        response: resp,
                    }),
                    _ => Err(PlayerError::new(
                        ErrorCategory::InvalidResponse,
                        format!(
                            "206 with an unusable Content-Range from {}",
                            access.redacted_url()
                        ),
                    )),
                }
            }
            200 => {
                let len = resp
                    .header("content-length")
                    .and_then(|v| v.trim().parse().ok());
                Ok(Probe {
                    len,
                    ranged: false,
                    response: resp,
                })
            }
            // Range starting at 0 unsatisfiable: an empty resource.
            416 => Err(PlayerError::new(
                ErrorCategory::InvalidResponse,
                format!("empty resource at {}", access.redacted_url()),
            )),
            s => Err(PlayerError::new(
                ErrorCategory::InvalidResponse,
                format!("unexpected HTTP {s} from {}", access.redacted_url()),
            )),
        }
    }

    /// Wrap a probed response.
    pub(crate) fn from_probe(
        agent: ureq::Agent,
        cfg: HttpConfig,
        provider: Arc<dyn AccessProvider>,
        access: Access,
        probe: Probe,
        cancel: CancelToken,
    ) -> Self {
        let end = if probe.ranged {
            probe
                .response
                .header("content-range")
                .and_then(content_range)
                .map(|(_, b, _)| b + 1)
        } else {
            None
        };
        Self {
            agent,
            cfg,
            provider,
            access,
            cancel,
            pos: 0,
            len: probe.len,
            ranged: probe.ranged,
            validator: Validator::from(&probe.response),
            content_type: probe.response.header("content-type").map(str::to_string),
            body: Some(Body {
                reader: probe.response.body,
                end,
            }),
            reconnects: Default::default(),
            last_error: Default::default(),
        }
    }

    fn record(&self, e: PlayerError) -> std::io::Error {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(e.clone());
        }
        e.into_io()
    }

    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Request the resource from `self.pos`. Ranged mode only.
    fn open_at(&mut self) -> Result<(), PlayerError> {
        let last = match self.len {
            Some(len) => (self.pos + self.cfg.request_bytes).min(len) - 1,
            None => self.pos + self.cfg.request_bytes - 1,
        };
        let mut headers = vec![("Range", format!("bytes={}-{last}", self.pos))];
        if let Some(v) = self.validator.if_range() {
            headers.push(("If-Range", v));
        }
        // Long enough for one request on a slow link (≥ 16 KiB/s).
        let body_timeout = Duration::from_secs(30.max(self.cfg.request_bytes / (16 * 1024)));
        let resp = request_with_timeout(
            &self.agent,
            self.provider.as_ref(),
            &mut self.access,
            &headers,
            Some(body_timeout),
            &self.cfg,
            &self.cancel,
        )?;
        let url = redact_url(&self.access.url);
        let changed = |why: &str| {
            PlayerError::new(
                ErrorCategory::ResourceChanged,
                format!("{url} changed while reading ({why})"),
            )
        };
        match resp.status {
            206 => {
                let Some((a, b, total)) = resp.header("content-range").and_then(content_range)
                else {
                    return Err(PlayerError::new(
                        ErrorCategory::InvalidResponse,
                        format!("bad Content-Range from {url}"),
                    ));
                };
                if a != self.pos {
                    return Err(PlayerError::new(
                        ErrorCategory::InvalidResponse,
                        format!("{url} answered offset {a} for {}", self.pos),
                    ));
                }
                if total.is_some() && self.len.is_some() && total != self.len {
                    return Err(changed("size"));
                }
                let v = Validator::from(&resp);
                if v.etag.is_some()
                    && self.validator.etag.is_some()
                    && v.etag != self.validator.etag
                {
                    return Err(changed("ETag"));
                }
                self.body = Some(Body {
                    reader: resp.body,
                    end: Some(b + 1),
                });
                Ok(())
            }
            // With If-Range, a full 200 is the server saying "not the
            // same resource any more"; without a validator it means
            // ranges stopped working — neither can be spliced.
            200 => Err(changed("full response to a range request")),
            416 => Err(changed("range no longer satisfiable")),
            s => Err(PlayerError::new(
                ErrorCategory::InvalidResponse,
                format!("unexpected HTTP {s} from {url}"),
            )),
        }
    }

    /// Reopen after an interruption: backoff, bounded attempts, abort on
    /// cancellation. Non-recoverable failures (changed resource, refused
    /// access) stop immediately.
    fn reopen(&mut self, cause: PlayerError) -> Result<(), PlayerError> {
        if !self.ranged {
            return Err(PlayerError::new(
                ErrorCategory::Network,
                format!(
                    "transfer of {} cut at byte {} and the server does not support resuming",
                    redact_url(&self.access.url),
                    self.pos
                ),
            ));
        }
        let mut last = cause;
        for attempt in 0..self.cfg.max_retries {
            if attempt > 0 {
                sleep_cancellable(Duration::from_millis(250 << attempt.min(4)), &self.cancel)?;
            }
            self.reconnects
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match self.open_at() {
                Ok(()) => return Ok(()),
                Err(e) if e.recoverable && e.category != ErrorCategory::Cancelled => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
}

fn sleep_cancellable(d: Duration, cancel: &CancelToken) -> Result<(), PlayerError> {
    let until = std::time::Instant::now() + d;
    while std::time::Instant::now() < until {
        cancel.check()?;
        std::thread::sleep(Duration::from_millis(20));
    }
    cancel.check()
}

fn request(
    agent: &ureq::Agent,
    provider: &dyn AccessProvider,
    access: &mut Access,
    headers: &[(&str, String)],
    cfg: &HttpConfig,
    cancel: &CancelToken,
) -> Result<Response, PlayerError> {
    request_with_timeout(agent, provider, access, headers, None, cfg, cancel)
}

/// GET with the access, renewing it once on 401 (or when it is known to
/// have expired). A second rejection is final.
pub(crate) fn request_with_timeout(
    agent: &ureq::Agent,
    provider: &dyn AccessProvider,
    access: &mut Access,
    headers: &[(&str, String)],
    body_timeout: Option<Duration>,
    cfg: &HttpConfig,
    cancel: &CancelToken,
) -> Result<Response, PlayerError> {
    if access
        .expires_at
        .is_some_and(|t| t <= std::time::SystemTime::now())
    {
        *access = provider.renew(access, RenewReason::Expired)?;
    }
    let mut renewed = false;
    loop {
        let resp = http_client::get(agent, access, headers, body_timeout, cfg, cancel)?;
        match http_client::status_error(resp.status, &access.url) {
            None => return Ok(resp),
            Some(e) if e.category == ErrorCategory::AccessExpired && !renewed => {
                drop(resp);
                *access = provider.renew(access, RenewReason::Unauthorized)?;
                renewed = true;
            }
            Some(e) if e.category == ErrorCategory::AccessExpired => {
                return Err(e
                    .with_recoverable(false)
                    .tap_message("access rejected again after renewal"));
            }
            Some(e) => return Err(e),
        }
    }
}

trait TapMessage {
    fn tap_message(self, note: &str) -> Self;
}

impl TapMessage for PlayerError {
    fn tap_message(mut self, note: &str) -> Self {
        self.message = format!("{} ({note})", self.message);
        self
    }
}

impl Read for RangeSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            self.cancel.check().map_err(PlayerError::into_io)?;
            if self.len.is_some_and(|len| self.pos >= len) {
                return Ok(0);
            }
            if self.body.is_none()
                && let Err(e) = self.open_at().or_else(|e| {
                    if e.recoverable {
                        self.reopen(e)
                    } else {
                        Err(e)
                    }
                })
            {
                return Err(self.record(e));
            }
            let body = self.body.as_mut().expect("opened above");
            let want = match body.end {
                Some(end) => buf.len().min((end - self.pos) as usize),
                None => buf.len(),
            };
            match body.reader.read(&mut buf[..want]) {
                Ok(0) => {
                    let complete = match body.end {
                        Some(end) => self.pos >= end,
                        // Non-ranged: the connection's end is the
                        // resource's end only if it matches the length.
                        None => self.len.is_none_or(|len| self.pos >= len),
                    };
                    self.body = None;
                    if complete {
                        if !self.ranged {
                            return Ok(0);
                        }
                        continue;
                    }
                    let cut = PlayerError::new(
                        ErrorCategory::Network,
                        format!("transfer cut at byte {}", self.pos),
                    );
                    if let Err(e) = self.reopen(cut) {
                        return Err(self.record(e));
                    }
                }
                Ok(n) => {
                    self.pos += n as u64;
                    if body.end.is_some_and(|end| self.pos >= end) {
                        self.body = None;
                    }
                    return Ok(n);
                }
                Err(e) => {
                    let err = PlayerError::from_io(&e);
                    if err.category == ErrorCategory::Cancelled {
                        return Err(e);
                    }
                    self.body = None;
                    if let Err(e) = self.reopen(err) {
                        return Err(self.record(e));
                    }
                }
            }
        }
    }
}

impl Seek for RangeSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self.len.and_then(|l| l.checked_add_signed(d)),
        }
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad seek"))?;
        if target == self.pos {
            return Ok(target);
        }
        let forward = target.saturating_sub(self.pos);
        let in_body = target > self.pos
            && self
                .body
                .as_ref()
                .is_some_and(|b| b.end.is_none_or(|e| target < e));
        if in_body && (forward <= SKIP_IN_PLACE || !self.ranged) {
            let mut sink = [0u8; 8192];
            while self.pos < target {
                let n = (target - self.pos).min(sink.len() as u64) as usize;
                let got = self.read(&mut sink[..n])?;
                if got == 0 {
                    break;
                }
            }
            return Ok(self.pos);
        }
        if !self.ranged {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "server does not support byte ranges",
            ));
        }
        self.body = None;
        self.pos = target;
        Ok(target)
    }
}

impl MediaSource for RangeSource {
    fn is_seekable(&self) -> bool {
        self.ranged
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parsing() {
        assert_eq!(content_range("bytes 0-99/1000"), Some((0, 99, Some(1000))));
        assert_eq!(content_range("bytes 5-9/*"), Some((5, 9, None)));
        assert_eq!(content_range("bytes 9-5/100"), None);
        assert_eq!(content_range("bytes 0-100/100"), None);
        assert_eq!(content_range("items 0-1/2"), None);
    }
}
