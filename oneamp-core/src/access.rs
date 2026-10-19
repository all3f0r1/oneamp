//! Injectable access to remote media: the host says which URL and which
//! headers reach the bytes, and supplies a new access when the server
//! rejects the current one.
//!
//! The engine never mints, stores or widens a credential. Everything a
//! server-specific scheme needs (Tesserafin's playback capabilities, a
//! signed CDN URL) lives in the host's [`AccessProvider`]. The rules the
//! engine keeps on its side:
//!
//! * a rejected access is renewed **once** per request; a renewed access
//!   that is rejected again fails the read with `AccessExpired` /
//!   `AccessDenied` — no reconnect loop;
//! * `403` is a refusal, not an expiry: it is never answered by renewal;
//! * concurrent rejections of one access trigger one renewal
//!   ([`CoordinatedAccess`]);
//! * the media identity never changes on renewal;
//! * headers are only sent to the origin they were issued for unless the
//!   access says otherwise, and an `https` → `http` redirect is refused;
//! * `Debug` output and error messages never contain a URL query value or
//!   a header value.

use crate::error::{ErrorCategory, PlayerError, redact_url};
use std::sync::Mutex;
use std::time::SystemTime;

/// What to do with redirects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum RedirectPolicy {
    /// Follow up to 5 redirects; the access's headers are only attached
    /// while the target keeps the original scheme, host and port.
    #[default]
    HeadersSameOriginOnly,
    /// Follow redirects and keep sending the headers to the new origin.
    /// Only for hosts that know their redirect targets (a CDN in front
    /// of their own server).
    HeadersToAnyOrigin,
    /// Treat any redirect as an error.
    Refuse,
}

/// One way in to a resource.
#[derive(Clone, PartialEq)]
pub struct Access {
    pub url: String,
    /// Extra request headers (`Authorization`, …). Sent per
    /// [`RedirectPolicy`].
    pub headers: Vec<(String, String)>,
    /// When the host knows the access stops working. Informational: the
    /// engine still relies on the server's verdict.
    pub expires_at: Option<SystemTime>,
    pub redirects: RedirectPolicy,
    /// Bumped by [`CoordinatedAccess`] on every renewal; lets concurrent
    /// readers tell whether the access they saw rejected is still current.
    pub generation: u64,
}

impl Access {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            expires_at: None,
            redirects: RedirectPolicy::default(),
            generation: 0,
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The URL with every secret replaced, for logs and errors.
    pub fn redacted_url(&self) -> String {
        redact_url(&self.url)
    }
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<String> = self
            .headers
            .iter()
            .map(|(k, _)| format!("{k}: <redacted>"))
            .collect();
        f.debug_struct("Access")
            .field("url", &self.redacted_url())
            .field("headers", &headers)
            .field("expires_at", &self.expires_at)
            .field("redirects", &self.redirects)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Why the engine asks for a new access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenewReason {
    /// The server answered 401 (expired, unknown or revoked credential).
    Unauthorized,
    /// `expires_at` has passed before a request was made.
    Expired,
}

/// Supplies and renews the access to one media. Called from worker
/// threads, never from the audio callback; may block on the network.
///
/// `renew` must return an access with **no more rights** than the one
/// rejected (same media, same scope). Return `AccessExpired` when the
/// credential cannot be renewed and `AccessDenied` when the user lost the
/// right; never fall back to a broader credential.
pub trait AccessProvider: Send + Sync {
    fn access(&self) -> Result<Access, PlayerError>;
    fn renew(&self, rejected: &Access, reason: RenewReason) -> Result<Access, PlayerError>;
}

/// An access that never changes (radio stations, public files). A
/// rejection is final.
pub struct StaticAccess(Access);

impl StaticAccess {
    pub fn new(url: impl Into<String>) -> Self {
        Self(Access::new(url))
    }

    pub fn from_access(access: Access) -> Self {
        Self(access)
    }
}

impl AccessProvider for StaticAccess {
    fn access(&self) -> Result<Access, PlayerError> {
        Ok(self.0.clone())
    }

    fn renew(&self, _rejected: &Access, _reason: RenewReason) -> Result<Access, PlayerError> {
        Err(PlayerError::new(
            ErrorCategory::AccessExpired,
            format!(
                "access to {} rejected and not renewable",
                self.0.redacted_url()
            ),
        ))
    }
}

/// Serialises renewals and shares their result: when several readers
/// see the same access rejected, the first renews and the others reuse
/// what it got.
pub struct CoordinatedAccess<P> {
    inner: P,
    current: Mutex<Option<Access>>,
}

impl<P: AccessProvider> CoordinatedAccess<P> {
    pub fn new(inner: P) -> Self {
        Self {
            inner,
            current: Mutex::new(None),
        }
    }
}

impl<P: AccessProvider> AccessProvider for CoordinatedAccess<P> {
    fn access(&self) -> Result<Access, PlayerError> {
        let mut cur = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = cur.as_ref() {
            return Ok(a.clone());
        }
        let a = self.inner.access()?;
        *cur = Some(a.clone());
        Ok(a)
    }

    fn renew(&self, rejected: &Access, reason: RenewReason) -> Result<Access, PlayerError> {
        // Held across the host call on purpose: a second reader blocks
        // here, then finds the generation moved and takes the new access.
        let mut cur = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = cur.as_ref()
            && a.generation != rejected.generation
        {
            return Ok(a.clone());
        }
        let mut fresh = self.inner.renew(rejected, reason)?;
        fresh.generation = rejected.generation + 1;
        *cur = Some(fresh.clone());
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Counting(AtomicU32);

    impl AccessProvider for Counting {
        fn access(&self) -> Result<Access, PlayerError> {
            Ok(Access::new("https://h/a?cap=0"))
        }
        fn renew(&self, _: &Access, _: RenewReason) -> Result<Access, PlayerError> {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Access::new(format!("https://h/a?cap={n}")))
        }
    }

    #[test]
    fn concurrent_rejections_renew_once() {
        let p = Arc::new(CoordinatedAccess::new(Counting(AtomicU32::new(0))));
        let seen = p.access().unwrap();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let p = p.clone();
                let seen = seen.clone();
                std::thread::spawn(move || p.renew(&seen, RenewReason::Unauthorized).unwrap())
            })
            .collect();
        let got: Vec<Access> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(p.inner.0.load(Ordering::SeqCst), 1);
        assert!(
            got.iter()
                .all(|a| a.url.ends_with("cap=1") && a.generation == 1)
        );
        // Rejecting the renewed access renews again (and only then).
        let again = p.renew(&got[0], RenewReason::Unauthorized).unwrap();
        assert_eq!(again.generation, 2);
    }

    #[test]
    fn static_access_is_not_renewable_and_debug_is_redacted() {
        let s = StaticAccess::from_access(
            Access::new("https://h/x?token=SECRET").with_header("Authorization", "Bearer SECRET"),
        );
        let a = s.access().unwrap();
        let dbg = format!("{a:?}");
        assert!(!dbg.contains("SECRET"), "{dbg}");
        let e = s.renew(&a, RenewReason::Unauthorized).unwrap_err();
        assert_eq!(e.category, ErrorCategory::AccessExpired);
        assert!(!e.message.contains("SECRET"));
    }
}
