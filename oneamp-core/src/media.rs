//! What the host hands the engine to play: an identity, where the bytes
//! come from, and everything the host already knows about the media.
//!
//! The identity ([`MediaId`]) is the host's — a library item id, a file
//! path, a station URL the user saved. It is never derived from a
//! temporary delivery URL: an access that expires and is renewed keeps
//! the same media identity.

use crate::dsp::{ListeningContext, MediaGain};
use crate::error::PlayerError;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Opaque, stable identity of a media, chosen by the host. Shown in
/// diagnostics, so it must not embed a credential.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct MediaId(Arc<str>);

impl MediaId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(Arc::from(id.into()))
    }

    /// The id the desktop player uses for a local file.
    pub fn from_path(path: &Path) -> Self {
        Self::new(path.to_string_lossy().into_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for MediaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MediaId({:?})", &*self.0)
    }
}

impl std::fmt::Display for MediaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Cooperative cancellation shared between the engine and the threads
/// working for one media (connect, read, decode, prepare).
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn check(&self) -> Result<(), PlayerError> {
        if self.is_cancelled() {
            Err(PlayerError::cancelled())
        } else {
            Ok(())
        }
    }
}

/// Descriptive metadata the host already has. Takes precedence over
/// tags read from the stream, field by field.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MediaMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
}

/// Format knowledge that speeds up or steers probing. All optional;
/// probing sniffs the content when nothing is given.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FormatHint {
    /// File extension without the dot ("flac").
    pub extension: Option<String>,
    /// MIME type ("audio/flac"). Used when `extension` is absent.
    pub mime_type: Option<String>,
}

impl FormatHint {
    pub(crate) fn extension(&self) -> Option<String> {
        self.extension.clone().or_else(|| {
            self.mime_type
                .as_deref()
                .and_then(extension_for_mime)
                .map(str::to_string)
        })
    }
}

/// Map a MIME type to the extension symphonia's probe keys on. `None`
/// for unknown types — the probe then sniffs the content.
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    let primary = mime.split(';').next()?.trim().to_ascii_lowercase();
    Some(match primary.as_str() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/aac" | "audio/aacp" => "aac",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/ogg" | "application/ogg" | "audio/opus" => "ogg",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/x-matroska" | "audio/webm" => "mka",
        "audio/aiff" | "audio/x-aiff" => "aiff",
        _ => return None,
    })
}

/// MIME types of segmented deliveries the HTTP transport refuses: they
/// are a playlist of segments, not a byte stream, and need an adapter
/// supplied through [`MediaLocation::Custom`].
pub fn is_segmented_mime(mime: &str) -> bool {
    let primary = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        primary.as_str(),
        "application/vnd.apple.mpegurl"
            | "application/x-mpegurl"
            | "audio/mpegurl"
            | "audio/x-mpegurl"
            | "application/dash+xml"
    )
}

/// How an HTTP resource is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Delivery {
    /// Decide from the response: honoured `Range` ⇒ on demand; ICY
    /// headers or no length ⇒ live; a length without range support ⇒
    /// on demand, forward-only.
    #[default]
    Auto,
    /// A finite file. Never treated as live, whatever the headers say.
    OnDemand,
    /// Radio / direct live stream: forward-only, reconnects at the live
    /// edge, ICY metadata.
    Live,
}

/// A byte source the host builds itself: an Android file descriptor, a
/// content URI, a decrypting reader, a segmented-delivery adapter.
/// `open` may block (it runs on a worker) and is called again for each
/// independent reader (repeat, resume), so it must return a fresh
/// reader positioned at the start.
pub trait SourceFactory: Send + Sync + std::fmt::Debug {
    fn open(
        &self,
        cancel: &CancelToken,
    ) -> Result<Box<dyn symphonia::core::io::MediaSource>, PlayerError>;
}

/// Where the bytes come from.
#[derive(Clone)]
pub enum MediaLocation {
    File(PathBuf),
    #[cfg(feature = "http")]
    Http {
        access: Arc<dyn crate::access::AccessProvider>,
        delivery: Delivery,
    },
    Custom(Arc<dyn SourceFactory>),
}

impl std::fmt::Debug for MediaLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaLocation::File(p) => f.debug_tuple("File").field(p).finish(),
            // The provider holds credentials: never printed.
            #[cfg(feature = "http")]
            MediaLocation::Http { delivery, .. } => f
                .debug_struct("Http")
                .field("delivery", delivery)
                .finish_non_exhaustive(),
            MediaLocation::Custom(c) => f.debug_tuple("Custom").field(c).finish(),
        }
    }
}

/// Everything about one media the engine needs. Cheap to clone.
#[derive(Clone, Debug)]
pub struct MediaDescriptor {
    pub id: MediaId,
    pub location: MediaLocation,
    pub metadata: MediaMetadata,
    /// Known duration of the *whole* media (not of the delivered part
    /// when `timeline_offset` is set). `None` = unknown/live.
    pub duration: Option<Duration>,
    pub format: FormatHint,
    /// Normalisation figures; when set they replace the stream's tags.
    pub gain: Option<MediaGain>,
    pub listening_context: ListeningContext,
    /// Media time playback starts at. Reached before any audio is
    /// output: the beginning is never heard.
    pub start_position: Duration,
    /// `Some(false)` forbids seeking even when the source could;
    /// `None` uses what the source supports.
    pub seekable: Option<bool>,
    /// Media time of the first sample the source delivers. Non-zero when
    /// a server started a transcode mid-media: positions are reported on
    /// the media's own timeline, and a seek before the offset fails with
    /// `InvalidRequest` (the host must request a new delivery).
    pub timeline_offset: Duration,
}

impl MediaDescriptor {
    pub fn new(id: MediaId, location: MediaLocation) -> Self {
        Self {
            id,
            location,
            metadata: MediaMetadata::default(),
            duration: None,
            format: FormatHint::default(),
            gain: None,
            listening_context: ListeningContext::Unknown,
            start_position: Duration::ZERO,
            seekable: None,
            timeline_offset: Duration::ZERO,
        }
    }

    /// A local file; the id is its path.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut d = Self::new(MediaId::from_path(&path), MediaLocation::File(path.clone()));
        d.format.extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_string);
        d
    }

    /// A fixed URL (radio station, public file). For expiring access use
    /// [`MediaDescriptor::http_with`].
    #[cfg(feature = "http")]
    pub fn http(id: MediaId, url: impl Into<String>) -> Self {
        Self::http_with(id, crate::access::StaticAccess::new(url))
    }

    /// An HTTP resource whose access the host resolves and renews.
    /// Concurrent renewals for this media are coordinated: readers that
    /// share the descriptor never mint twice for one rejection.
    #[cfg(feature = "http")]
    pub fn http_with(id: MediaId, provider: impl crate::access::AccessProvider + 'static) -> Self {
        Self::new(
            id,
            MediaLocation::Http {
                access: Arc::new(crate::access::CoordinatedAccess::new(provider)),
                delivery: Delivery::Auto,
            },
        )
    }

    pub fn custom(id: MediaId, factory: impl SourceFactory + 'static) -> Self {
        Self::new(id, MediaLocation::Custom(Arc::new(factory)))
    }

    /// The desktop player's playlist rows: a path, or an `http(s)://`
    /// URL saved by the user (whose identity is that URL).
    #[cfg(feature = "http")]
    pub fn from_path_or_url(path: &Path) -> Self {
        match path
            .to_str()
            .filter(|s| crate::http_stream::is_stream_url_str(s))
        {
            Some(url) => Self::http(MediaId::new(url), url),
            None => Self::file(path),
        }
    }

    /// How an HTTP media is delivered (no effect on other locations).
    #[cfg(feature = "http")]
    pub fn with_delivery(mut self, delivery: Delivery) -> Self {
        if let MediaLocation::Http { delivery: d, .. } = &mut self.location {
            *d = delivery;
        }
        self
    }

    pub fn with_metadata(mut self, metadata: MediaMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    pub fn with_gain(mut self, gain: MediaGain) -> Self {
        self.gain = Some(gain);
        self
    }

    pub fn with_format(mut self, format: FormatHint) -> Self {
        self.format = format;
        self
    }

    pub fn starting_at(mut self, position: Duration) -> Self {
        self.start_position = position;
        self
    }

    pub fn with_timeline_offset(mut self, offset: Duration) -> Self {
        self.timeline_offset = offset;
        self
    }

    pub fn with_listening_context(mut self, context: ListeningContext) -> Self {
        self.listening_context = context;
        self
    }

    pub fn is_live(&self) -> bool {
        #[cfg(feature = "http")]
        if let MediaLocation::Http { delivery, .. } = &self.location {
            return *delivery == Delivery::Live;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_descriptor_uses_path_identity_and_extension() {
        let d = MediaDescriptor::file("/music/a.flac");
        assert_eq!(d.id.as_str(), "/music/a.flac");
        assert_eq!(d.format.extension(), Some("flac".into()));
    }

    #[test]
    fn mime_hints_and_segmented_detection() {
        let hint = FormatHint {
            extension: None,
            mime_type: Some("audio/flac; foo=bar".into()),
        };
        assert_eq!(hint.extension(), Some("flac".into()));
        assert!(is_segmented_mime("application/vnd.apple.mpegURL"));
        assert!(!is_segmented_mime("audio/mpeg"));
    }

    #[cfg(feature = "http")]
    #[test]
    fn http_identity_is_independent_of_the_url() {
        let d = MediaDescriptor::http(MediaId::new("item-42"), "https://h/x?playbackCapability=s");
        assert_eq!(d.id.as_str(), "item-42");
        assert!(!format!("{d:?}").contains("playbackCapability=s"));
    }
}
