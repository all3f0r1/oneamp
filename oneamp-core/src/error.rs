//! Structured errors shared by every layer, and the redaction applied to
//! anything that may carry a credential before it reaches a message.

use std::fmt;

/// What kind of failure, independent of the layer that hit it. Hosts
/// branch on this, never on message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ErrorCategory {
    /// Connection refused/reset, DNS, 5xx, a transfer cut mid-body.
    Network,
    /// A connect, response or idle deadline passed.
    Timeout,
    /// The access was rejected as expired or unknown (HTTP 401) and the
    /// host could not supply a fresh one.
    AccessExpired,
    /// The access is valid but does not cover this request (HTTP 403),
    /// or the host refused to renew it. Renewal will not help.
    AccessDenied,
    /// HTTP 404 / missing file. Servers that hide existence (Tesserafin
    /// answers refusals with 404) land here too.
    NotFound,
    /// The resource changed between two requests (validator or size
    /// mismatch): continuing would splice two different files.
    ResourceChanged,
    /// The server broke the protocol (bad `Content-Range`, a 200 where a
    /// range was required, …).
    InvalidResponse,
    /// Format, codec, delivery (HLS) or operation not supported.
    Unsupported,
    /// The media could not be decoded.
    Decode,
    /// The audio output failed or disappeared.
    Output,
    /// The command made no sense in the current state.
    InvalidRequest,
    /// Superseded or abandoned (stop, replaced preparation).
    Cancelled,
    Internal,
}

/// An error with its category, whether retrying can help, and a message
/// that never contains a credential (URLs go through [`redact_url`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlayerError {
    pub category: ErrorCategory,
    /// Retrying the same operation (resume, reconnect, re-prepare) may
    /// succeed. `false` means the host has to change something first.
    pub recoverable: bool,
    pub message: String,
}

impl PlayerError {
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        use ErrorCategory::*;
        let recoverable = matches!(category, Network | Timeout | Output | Cancelled);
        Self {
            category,
            recoverable,
            message: message.into(),
        }
    }

    pub fn with_recoverable(mut self, recoverable: bool) -> Self {
        self.recoverable = recoverable;
        self
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorCategory::Cancelled, "cancelled")
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Unsupported, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::InvalidRequest, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Internal, message)
    }

    /// Wrap for transport through `std::io` (symphonia reads sources
    /// through `Read`); [`PlayerError::from_io`] unwraps it again.
    pub fn into_io(self) -> std::io::Error {
        std::io::Error::other(self)
    }

    /// Recover a `PlayerError` that travelled through `std::io`, or
    /// classify a plain I/O error.
    pub fn from_io(e: &std::io::Error) -> Self {
        if let Some(inner) = e.get_ref().and_then(|r| r.downcast_ref::<PlayerError>()) {
            return inner.clone();
        }
        use std::io::ErrorKind::*;
        let category = match e.kind() {
            NotFound => ErrorCategory::NotFound,
            PermissionDenied => ErrorCategory::AccessDenied,
            TimedOut => ErrorCategory::Timeout,
            ConnectionReset | ConnectionAborted | ConnectionRefused | BrokenPipe
            | UnexpectedEof => ErrorCategory::Network,
            Unsupported => ErrorCategory::Unsupported,
            InvalidData => ErrorCategory::Decode,
            _ => ErrorCategory::Internal,
        };
        Self::new(category, e.to_string())
    }
}

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.category, self.message)
    }
}

impl std::error::Error for PlayerError {}

/// A URL safe to log: user-info removed, every query value and the
/// fragment replaced. Parameter names stay, so a diagnostic still shows
/// *which* parameters were sent (`?playbackCapability=…`).
pub fn redact_url(url: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (url.split('#').next().unwrap_or(url), None),
    };
    let base = match base.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = match rest.find('/') {
                Some(i) => rest.split_at(i),
                None => (rest, ""),
            };
            let host = authority.rsplit('@').next().unwrap_or(authority);
            format!("{scheme}://{host}{path}")
        }
        None => base.to_string(),
    };
    match query {
        Some(q) => {
            let q = q.split('#').next().unwrap_or(q);
            let names: Vec<String> = q
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| match p.split_once('=') {
                    Some((k, _)) => format!("{k}=…"),
                    None => p.to_string(),
                })
                .collect();
            format!("{base}?{}", names.join("&"))
        }
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_hides_secrets_but_keeps_shape() {
        assert_eq!(
            redact_url(
                "https://user:pw@host:8096/Audio/1/stream?playbackCapability=SECRET&static=true#frag"
            ),
            "https://host:8096/Audio/1/stream?playbackCapability=…&static=…"
        );
        assert_eq!(
            redact_url("http://radio.example/live"),
            "http://radio.example/live"
        );
        assert!(!redact_url("http://h/x?api_key=abc").contains("abc"));
    }

    #[test]
    fn io_round_trip_keeps_category() {
        let e = PlayerError::new(ErrorCategory::ResourceChanged, "etag changed").into_io();
        assert_eq!(
            PlayerError::from_io(&e).category,
            ErrorCategory::ResourceChanged
        );
        let plain = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        let p = PlayerError::from_io(&plain);
        assert_eq!(p.category, ErrorCategory::Network);
        assert!(p.recoverable);
    }
}
