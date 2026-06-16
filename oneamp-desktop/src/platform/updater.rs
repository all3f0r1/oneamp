//! Background update checker.
//!
//! At startup we spawn one thread that hits the GitHub Releases API,
//! parses the `tag_name`, and compares it semver-wise to the build's
//! `CARGO_PKG_VERSION`. When the latest tag is strictly newer the
//! main thread receives an `UpdateInfo` over a bounded channel, fires
//! one desktop notification, and persists the version it announced so
//! the same release doesn't re-prompt on every launch.
//!
//! Errors are silent — a flaky network, a 5xx from GitHub, or a tag
//! that doesn't parse all surface as "no update found". The user
//! never sees a failure; they just don't get a toast that session.
//!
//! No download, no auto-install. The notification points at the
//! release URL; package-manager users (apt / dnf / flatpak / snap /
//! brew once it lands) take it from there, ZIP / DMG users grab the
//! artifact from the page.

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use std::thread;
use std::time::Duration;

const RELEASES_URL: &str = "https://api.github.com/repos/all3f0r1/oneamp/releases/latest";
const USER_AGENT: &str = concat!("OneAmp-update-check/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub release_url: String,
}

/// Holds the receiver for a one-shot background update check. `poll`
/// drains the channel; once a result is read (or the channel hangs
/// up), the receiver is dropped so subsequent polls return `None`
/// without retrying.
pub struct UpdateChecker {
    rx: Option<Receiver<UpdateInfo>>,
}

impl UpdateChecker {
    /// Spawn a detached thread that queries the GitHub Releases API
    /// and pushes an `UpdateInfo` onto the channel iff a strictly
    /// newer version is available. The thread terminates after one
    /// request — the channel is bounded(1) so a slow GUI thread
    /// doesn't OOM us with replies.
    pub fn spawn() -> Self {
        let (tx, rx) = bounded::<UpdateInfo>(1);
        let current = env!("CARGO_PKG_VERSION").to_string();
        let _ = thread::Builder::new()
            .name("oneamp-update-check".into())
            .spawn(move || {
                if let Some(info) = fetch_latest(&current) {
                    let _ = tx.send(info);
                }
            });
        Self { rx: Some(rx) }
    }

    /// Returns `Some(info)` exactly once when the background check
    /// completes with a newer version, then `None` forever after.
    /// Returns `None` when the check is still running, when the
    /// server reports no newer release, or when the request failed.
    pub fn poll(&mut self) -> Option<UpdateInfo> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(info) => {
                self.rx = None;
                Some(info)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                // Thread finished without sending (no newer release,
                // or fetch failed). Drop the receiver so we never
                // wake up checking it again.
                self.rx = None;
                None
            }
        }
    }

    /// True once the background check has finished — either delivered
    /// an `UpdateInfo` (which `poll` consumed) or terminated without
    /// sending one (no newer release / network failure). Used by the
    /// app to distinguish "still in flight" from "definitively done with
    /// nothing to show" — the latter is what a user-triggered "Check
    /// for updates" needs to surface as an "Already up to date" toast.
    pub fn is_done(&self) -> bool {
        self.rx.is_none()
    }
}

fn fetch_latest(current: &str) -> Option<UpdateInfo> {
    // GitHub's REST API requires a User-Agent. ureq's default TLS is
    // rustls + webpki-roots which carries no system OpenSSL
    // dependency, so the build stays self-contained on macOS /
    // Windows. Timeouts live on the Agent config in ureq 3.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let body = agent
        .get(RELEASES_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    // Tags are `v0.21.0`; strip the leading `v` for semver parsing.
    let latest = tag.trim_start_matches('v');
    if version_is_newer(latest, current) {
        let url = json
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://github.com/all3f0r1/oneamp/releases/latest");
        Some(UpdateInfo {
            latest_version: latest.to_string(),
            release_url: url.to_string(),
        })
    } else {
        None
    }
}

/// Strictly-greater semver comparison. Anything unparseable on either
/// side returns `false` — we'd rather miss a release than spam a
/// user with a malformed tag.
fn version_is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(l), Ok(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_minor_is_newer() {
        assert!(version_is_newer("0.22.0", "0.21.0"));
    }

    #[test]
    fn newer_patch_is_newer() {
        assert!(version_is_newer("0.21.1", "0.21.0"));
    }

    #[test]
    fn newer_major_is_newer() {
        assert!(version_is_newer("1.0.0", "0.21.0"));
    }

    #[test]
    fn equal_is_not_newer() {
        assert!(!version_is_newer("0.21.0", "0.21.0"));
    }

    #[test]
    fn older_is_not_newer() {
        assert!(!version_is_newer("0.20.0", "0.21.0"));
    }

    #[test]
    fn invalid_latest_returns_false() {
        // Future-proof against tags like `nightly` or `latest`.
        assert!(!version_is_newer("nightly", "0.21.0"));
    }

    #[test]
    fn invalid_current_returns_false() {
        // Shouldn't happen in practice (CARGO_PKG_VERSION is always
        // semver), but exercise the edge anyway.
        assert!(!version_is_newer("0.22.0", "garbage"));
    }

    #[test]
    fn prerelease_is_older_than_release() {
        // 0.22.0-rc1 < 0.22.0 per semver — a stable user should NOT
        // be re-notified about a pre-release as if it were newer
        // than their own stable.
        assert!(!version_is_newer("0.22.0-rc1", "0.22.0"));
        assert!(version_is_newer("0.22.0", "0.22.0-rc1"));
    }
}
