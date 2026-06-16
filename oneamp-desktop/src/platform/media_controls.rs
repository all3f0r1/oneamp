//! Cross-platform media controls via `souvlaki`.
//!
//! Exposes OneAmp on each OS's media bus so:
//!   - physical multimedia keys (XF86Audio* on Linux, F7/F8/F9 on Mac,
//!     Bluetooth headset AVRCP on all three) drive playback,
//!   - the OS lock-screen / notification-area widget shows the current
//!     track with transport controls (MPRIS on Linux, MediaRemote on
//!     macOS, SMTC on Windows),
//!   - third-party utilities (`playerctl`, Stream Deck plugins, the
//!     Windows volume overlay) can read state and post commands.
//!
//! Souvlaki picks the backend by cfg:
//!   - `cfg(target_os = "linux")` — D-Bus (`org.mpris.MediaPlayer2.oneamp`).
//!     `dbus_name` field becomes the bus suffix.
//!   - `cfg(target_os = "macos")` — `MediaRemote` framework.
//!   - `cfg(target_os = "windows")` — `SystemMediaTransportControls`.
//!     `hwnd` field must be a non-null window handle.
//!
//! Souvlaki runs its own blocking event thread; the GUI thread talks
//! to it through `MediaControls::set_*` setters and receives incoming
//! events through a crossbeam channel that the GUI drains once per
//! frame.
//!
//! When the platform refuses our registration (rare — headless test
//! environment, denied bus, missing pipe) `new` returns `None` and the
//! rest of the app keeps working unchanged. We never make media
//! controls load-bearing.

use crate::audio::AudioController;
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use oneamp_core::{AudioCommand, AudioEvent};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use std::path::PathBuf;
use std::time::Duration;

/// Last known playback status, kept here so:
/// - `Toggle` events from the desktop can flip the right way without us
///   having to round-trip through the audio thread,
/// - repeat events from the audio thread (a quick Pause → Resume can
///   emit Playing twice) don't re-queue `ChangePlayback` on souvlaki's
///   internal channel — its loop processes at most one item per
///   `conn.process` iteration so duplicates pile up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LastStatus {
    Playing,
    Paused,
    Stopped,
}

pub struct MediaControlsService {
    controls: MediaControls,
    event_rx: Receiver<MediaControlEvent>,
    last_status: LastStatus,
    last_track_path: Option<PathBuf>,
    last_volume_milli: Option<i32>,
}

impl MediaControlsService {
    /// Register with the OS media bus. `cc` is the eframe creation
    /// context — needed on Windows to fetch the HWND that SMTC binds
    /// to. Returns `None` on any setup failure; the caller logs once
    /// and continues without it.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let config = PlatformConfig {
            dbus_name: "oneamp",
            display_name: "OneAmp",
            hwnd: extract_hwnd(cc),
        };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("oneamp: media controls unavailable ({e:?}) — multimedia keys disabled");
                return None;
            }
        };
        let (tx, rx): (Sender<MediaControlEvent>, Receiver<MediaControlEvent>) = unbounded();
        if let Err(e) = controls.attach(move |event| {
            // The callback runs on souvlaki's event thread. Channel
            // send is cheap and non-blocking; if the receiver is gone
            // (app tearing down) we silently drop the event.
            let _ = tx.send(event);
        }) {
            eprintln!("oneamp: media controls attach failed ({e:?})");
            return None;
        }
        Some(Self {
            controls,
            event_rx: rx,
            last_status: LastStatus::Stopped,
            last_track_path: None,
            last_volume_milli: None,
        })
    }

    /// Drain pending events that the OS posted (Play, Next, SetVolume,
    /// …) and translate them into the engine's command vocabulary.
    /// `Raise` and `Quit` go to the viewport instead — they're window
    /// management, not playback.
    pub fn poll_events(&mut self, audio: &AudioController, ctx: &egui::Context) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                MediaControlEvent::Play => audio.send_command(AudioCommand::Resume),
                MediaControlEvent::Pause => audio.send_command(AudioCommand::Pause),
                MediaControlEvent::Toggle => match self.last_status {
                    LastStatus::Playing => audio.send_command(AudioCommand::Pause),
                    LastStatus::Paused | LastStatus::Stopped => {
                        audio.send_command(AudioCommand::Resume)
                    }
                },
                MediaControlEvent::Next => audio.send_command(AudioCommand::Next),
                MediaControlEvent::Previous => audio.send_command(AudioCommand::Previous),
                MediaControlEvent::Stop => audio.send_command(AudioCommand::Stop),
                MediaControlEvent::SetPosition(MediaPosition(pos)) => {
                    audio.send_command(AudioCommand::Seek(pos.as_secs_f32()))
                }
                MediaControlEvent::SetVolume(v) => {
                    let clamped = v.clamp(0.0, 1.0) as f32;
                    audio.send_command(AudioCommand::SetVolume(clamped));
                    // Spec: the player must ack a SetVolume by writing
                    // the property back so the client sees the value
                    // land. `VolumeUpdated` from the audio thread will
                    // re-call sync_audio_event below; that's the
                    // durable ack, but pushing the value here gives
                    // the client an immediate visual response without
                    // a round-trip.
                    self.push_volume_clamped(clamped);
                }
                MediaControlEvent::Raise => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                MediaControlEvent::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // Relative seek (Seek / SeekBy) doesn't map onto our
                // absolute Seek(secs) command and clients can use
                // SetPosition instead, so we ignore. OpenUri is out of
                // scope — the engine doesn't take URIs.
                MediaControlEvent::Seek(_)
                | MediaControlEvent::SeekBy(_, _)
                | MediaControlEvent::OpenUri(_) => {}
            }
        }
    }

    /// Push an audio engine event into the media-controls service so
    /// the OS widget reflects current state. Called once per
    /// `AudioEvent` the GUI consumes. We dedup at status / value level
    /// — souvlaki's internal dispatch loop blocks up to 1 second
    /// waiting for bus I/O between channel reads on Linux, so a flood
    /// of duplicate "Playing → Playing" updates can queue minutes
    /// worth of work and starve a legitimate "Stopped" from landing.
    pub fn sync_audio_event(&mut self, event: &AudioEvent) {
        match event {
            AudioEvent::TrackLoaded(track)
                if self.last_track_path.as_ref() != Some(&track.path) =>
            {
                let duration = track.duration_secs.and_then(|s| {
                    if s.is_finite() && s > 0.0 {
                        Some(Duration::from_secs_f32(s))
                    } else {
                        None
                    }
                });
                // Fallback title from filename so the widget never
                // shows a blank line — matches what Winamp does when
                // tags are missing.
                let fallback_title = track
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string());
                let title_str = track.title.clone().or(fallback_title);
                let meta = MediaMetadata {
                    title: title_str.as_deref(),
                    album: track.album.as_deref(),
                    artist: track.artist.as_deref(),
                    cover_url: None,
                    duration,
                };
                let _ = self.controls.set_metadata(meta);
                self.last_track_path = Some(track.path.clone());
            }
            AudioEvent::Playing if self.last_status != LastStatus::Playing => {
                let _ = self
                    .controls
                    .set_playback(MediaPlayback::Playing { progress: None });
                self.last_status = LastStatus::Playing;
            }
            AudioEvent::Paused if self.last_status != LastStatus::Paused => {
                let _ = self
                    .controls
                    .set_playback(MediaPlayback::Paused { progress: None });
                self.last_status = LastStatus::Paused;
            }
            AudioEvent::Stopped | AudioEvent::Finished
                if self.last_status != LastStatus::Stopped =>
            {
                let _ = self.controls.set_playback(MediaPlayback::Stopped);
                self.last_status = LastStatus::Stopped;
                self.last_track_path = None;
            }
            AudioEvent::VolumeUpdated(level, _muted) => {
                self.push_volume_clamped(*level);
            }
            _ => {}
        }
    }

    /// Publish a volume to the media bus, deduping at milli-unit
    /// precision so a slider drag that emits 1000 micro-changes
    /// doesn't spam the OS.
    fn push_volume_clamped(&mut self, level: f32) {
        let clamped = level.clamp(0.0, 1.0);
        let milli = (clamped * 1000.0).round() as i32;
        if self.last_volume_milli == Some(milli) {
            return;
        }
        // souvlaki exposes `set_volume` only on the MPRIS (Linux) backend;
        // Windows SMTC and macOS MediaRemote have no volume setter, so the
        // method simply doesn't exist there. `clamped` is still consumed by
        // the `milli` computation above, so no unused-var warning off-Linux.
        #[cfg(target_os = "linux")]
        let _ = self.controls.set_volume(clamped as f64);
        self.last_volume_milli = Some(milli);
    }
}

/// Pull the Win32 HWND out of eframe's creation context. SMTC binds
/// to a window handle; without it `MediaControls::new` returns an
/// error on Windows. On macOS / Linux souvlaki's `PlatformConfig.hwnd`
/// is ignored — we return `None` and skip the import.
#[cfg(target_os = "windows")]
fn extract_hwnd(cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = cc.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn extract_hwnd(_cc: &eframe::CreationContext<'_>) -> Option<*mut std::ffi::c_void> {
    None
}
