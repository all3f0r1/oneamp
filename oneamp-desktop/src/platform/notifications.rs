//! Track-change desktop notifications via `notify-rust`.
//!
//! Fires a transient notification whenever a new track loads. The body
//! line joins artist and album with " · " — same shape as the MPRIS
//! metadata, so the player presents identically across the lock screen
//! widget and the toast.
//!
//! `notify-rust` picks the backend per OS:
//!   - Linux / BSD — `org.freedesktop.Notifications` over D-Bus.
//!   - macOS — `NSUserNotification` / `UNUserNotification`.
//!   - Windows — XML toast via the Windows Runtime.
//!
//! Failures are non-fatal: when no daemon is reachable, `notify_track`
//! silently no-ops after the first error log so we don't spam stderr.

use notify_rust::Notification;
use oneamp_core::TrackInfo;
use std::time::Duration;

const APP_NAME: &str = "OneAmp";
const APP_ICON: &str = "io.github.all3f0r1.OneAmp";
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(4);

pub struct NotificationService {
    /// One-shot guard so a broken bus (no daemon, permission denied,
    /// sandbox) only logs a single error line instead of one per track.
    error_logged: bool,
}

impl NotificationService {
    pub fn new() -> Self {
        Self {
            error_logged: false,
        }
    }

    /// Fire a "Now playing" toast for `track`. Title falls back to the
    /// filename stem (matching the MPRIS metadata code path) so a
    /// tagless file never shows a blank summary. The body line joins
    /// artist and album with " · " when both are present; either alone
    /// shows by itself; neither leaves the body empty.
    pub fn notify_track(&mut self, track: &TrackInfo) {
        let fallback_title = track
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        let summary = track.title.clone().or(fallback_title).unwrap_or_default();
        let body = match (track.artist.as_deref(), track.album.as_deref()) {
            (Some(a), Some(b)) => format!("{a} · {b}"),
            (Some(a), None) => a.to_string(),
            (None, Some(b)) => b.to_string(),
            (None, None) => String::new(),
        };

        let mut notification = Notification::new();
        notification
            .summary(&summary)
            .body(&body)
            .appname(APP_NAME)
            .icon(APP_ICON)
            .timeout(NOTIFY_TIMEOUT);

        // GNOME / KDE special-case `x-gnome.music`: render album art
        // when a hint points at one, suppress the default sound, and
        // group with other media notifications. Category hints are
        // a freedesktop convention — notify-rust silently drops them
        // on the macOS / Windows backends, so no cfg needed for the
        // call itself; we gate to skip the alloc on platforms where
        // it would be discarded.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use notify_rust::Hint;
            notification.hint(Hint::Category("x-gnome.music".to_string()));
        }

        if let Err(e) = notification.show()
            && !self.error_logged
        {
            eprintln!("oneamp: desktop notifications unavailable ({e})");
            self.error_logged = true;
        }
    }

    /// Fire a "Now playing on radio" toast for an ICY-published
    /// `StreamTitle`. Same backend as [`notify_track`]; the body line
    /// is left blank because shoutcast titles already pack `artist -
    /// title` into the single string.
    pub fn notify_icy(&mut self, title: &str) {
        let mut notification = Notification::new();
        notification
            .summary("Now playing")
            .body(title)
            .appname(APP_NAME)
            .icon(APP_ICON)
            .timeout(NOTIFY_TIMEOUT);

        if let Err(e) = notification.show()
            && !self.error_logged
        {
            eprintln!("oneamp: desktop notifications unavailable ({e})");
            self.error_logged = true;
        }
    }

    /// Fire a one-shot "update available" toast. Body includes the
    /// release URL so users on platforms with clickable bodies (most
    /// Linux daemons, macOS notification center on hover) can jump to
    /// the page directly; on platforms that strip URLs the user
    /// still sees the version and knows to check the release page.
    pub fn notify_update(&mut self, version: &str, release_url: &str) {
        let summary = format!("OneAmp {version} is available");
        let body = format!("Open {release_url} to download.");

        let mut notification = Notification::new();
        notification
            .summary(&summary)
            .body(&body)
            .appname(APP_NAME)
            .icon(APP_ICON)
            // 10 s on Linux daemons that honour timeout; macOS /
            // Windows toast lifetimes are OS-controlled.
            .timeout(Duration::from_secs(10));

        if let Err(e) = notification.show()
            && !self.error_logged
        {
            eprintln!("oneamp: desktop notifications unavailable ({e})");
            self.error_logged = true;
        }
    }
}

impl Default for NotificationService {
    fn default() -> Self {
        Self::new()
    }
}
