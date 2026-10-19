//! The JSON messages exchanged with Kotlin. Times are milliseconds,
//! names are camelCase, every message is an object with a `type`.
//! Version: [`PROTOCOL_VERSION`]; additions are backward compatible,
//! anything else bumps it.

use oneamp_core::access::{Access, AccessProvider};
use oneamp_core::engine::{AudioCommand, AudioEvent, PlaybackSnapshot};
use oneamp_core::{
    AudioSettings, Delivery, ErrorCategory, ListeningContext, MediaDescriptor, MediaGain, MediaId,
    MediaMetadata, PlayerError, RepeatMode, TrackInfo,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 1;

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn from_ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WireMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track_number: Option<u32>,
    year: Option<u32>,
    genre: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WireGain {
    track_gain_db: Option<f32>,
    track_peak: Option<f32>,
    album_gain_db: Option<f32>,
    album_peak: Option<f32>,
}

/// A media as Kotlin describes it. Exactly one of `url`, `path`, `fd`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WireMedia {
    pub id: String,
    pub url: Option<String>,
    pub headers: std::collections::BTreeMap<String, String>,
    /// Resolve/renew the access through the player's `AccessCallback`.
    pub access_callback: bool,
    pub path: Option<String>,
    /// A file descriptor the host detached (`ParcelFileDescriptor
    /// .detachFd()`); the player owns and closes it.
    pub fd: Option<i32>,
    pub delivery: Option<String>,
    pub mime_type: Option<String>,
    pub extension: Option<String>,
    pub duration_ms: Option<u64>,
    pub start_ms: u64,
    pub timeline_offset_ms: u64,
    pub seekable: Option<bool>,
    pub metadata: WireMetadata,
    pub gain: Option<WireGain>,
    pub context: Option<String>,
}

/// Builds the access provider for a media that asked for the callback.
pub type AccessFactory<'a> = &'a dyn Fn(&str) -> Option<Box<dyn AccessProvider>>;

struct Boxed(Box<dyn AccessProvider>);

impl AccessProvider for Boxed {
    fn access(&self) -> Result<Access, PlayerError> {
        self.0.access()
    }
    fn renew(
        &self,
        rejected: &Access,
        reason: oneamp_core::RenewReason,
    ) -> Result<Access, PlayerError> {
        self.0.renew(rejected, reason)
    }
}

impl WireMedia {
    pub fn into_descriptor(self, access: AccessFactory) -> Result<MediaDescriptor, PlayerError> {
        let id = MediaId::new(self.id.clone());
        if self.id.is_empty() {
            return Err(PlayerError::invalid("media.id is required"));
        }
        let sources = [self.url.is_some(), self.path.is_some(), self.fd.is_some()];
        if sources.iter().filter(|&&s| s).count() != 1 {
            return Err(PlayerError::invalid(
                "media needs exactly one of url, path, fd",
            ));
        }
        let mut d = if let Some(url) = &self.url {
            if self.access_callback {
                let provider = access(&self.id).ok_or_else(|| {
                    PlayerError::invalid("accessCallback set but the player has no AccessCallback")
                })?;
                MediaDescriptor::http_with(id, Boxed(provider))
            } else {
                let mut a = Access::new(url.clone());
                a.headers = self.headers.clone().into_iter().collect();
                MediaDescriptor::http_with(id, oneamp_core::StaticAccess::from_access(a))
            }
        } else if let Some(path) = &self.path {
            let mut d = MediaDescriptor::file(path);
            d.id = id;
            d
        } else {
            let fd = self.fd.expect("checked");
            MediaDescriptor::custom(id, crate::fd::FdFactory::adopt(fd)?)
        };
        d = d.with_delivery(match self.delivery.as_deref() {
            Some("live") => Delivery::Live,
            Some("onDemand") => Delivery::OnDemand,
            _ => Delivery::Auto,
        });
        if self.mime_type.is_some() || self.extension.is_some() {
            d.format.mime_type = self.mime_type;
            if self.extension.is_some() {
                d.format.extension = self.extension;
            }
        }
        let m = self.metadata;
        d.metadata = MediaMetadata {
            title: m.title,
            artist: m.artist,
            album: m.album,
            album_artist: m.album_artist,
            track_number: m.track_number,
            year: m.year,
            genre: m.genre,
        };
        d.duration = self.duration_ms.map(from_ms);
        d.start_position = from_ms(self.start_ms);
        d.timeline_offset = from_ms(self.timeline_offset_ms);
        d.seekable = self.seekable;
        d.gain = self.gain.map(|g| MediaGain {
            track_gain_db: g.track_gain_db,
            track_peak: g.track_peak,
            album_gain_db: g.album_gain_db,
            album_peak: g.album_peak,
        });
        d.listening_context = match self.context.as_deref() {
            Some("album") => ListeningContext::Album,
            Some("mixed") => ListeningContext::Mixed,
            _ => ListeningContext::Unknown,
        };
        Ok(d)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WireCommand {
    Load {
        media: WireMedia,
        #[serde(default)]
        paused: bool,
    },
    Resume,
    Pause,
    Stop,
    Seek {
        #[serde(rename = "positionMs")]
        position_ms: u64,
    },
    SetNext {
        media: Option<WireMedia>,
    },
    SetVolume {
        value: f32,
    },
    SetRepeat {
        mode: String,
    },
    /// The whole audio settings document (`oneamp_core::AudioSettings`
    /// JSON, snake_case as the core defines it).
    Settings {
        settings: Value,
    },
    RouteChanged,
}

pub fn parse_command(json: &str, access: AccessFactory) -> Result<AudioCommand, PlayerError> {
    let cmd: WireCommand = serde_json::from_str(json)
        .map_err(|e| PlayerError::invalid(format!("bad command JSON: {e}")))?;
    Ok(match cmd {
        WireCommand::Load { media, paused } => AudioCommand::Load {
            media: media.into_descriptor(access)?,
            paused,
        },
        WireCommand::Resume => AudioCommand::Resume,
        WireCommand::Pause => AudioCommand::Pause,
        WireCommand::Stop => AudioCommand::Stop,
        WireCommand::Seek { position_ms } => AudioCommand::Seek(from_ms(position_ms)),
        WireCommand::SetNext { media } => {
            AudioCommand::SetNext(media.map(|m| m.into_descriptor(access)).transpose()?)
        }
        WireCommand::SetVolume { value } => AudioCommand::SetVolume(value),
        WireCommand::SetRepeat { mode } => AudioCommand::SetRepeatMode(match mode.as_str() {
            "one" => RepeatMode::One,
            "all" => RepeatMode::All,
            _ => RepeatMode::Off,
        }),
        WireCommand::Settings { settings } => {
            let s = AudioSettings::from_json(&settings.to_string())
                .map_err(|e| PlayerError::invalid(e.to_string()))?;
            AudioCommand::ApplySettings(Box::new(s))
        }
        WireCommand::RouteChanged => AudioCommand::OutputRouteChanged,
    })
}

pub fn error_json(e: &PlayerError) -> Value {
    json!({
        "category": format!("{:?}", e.category),
        "recoverable": e.recoverable,
        "message": e.message,
    })
}

fn track_json(t: &TrackInfo) -> Value {
    json!({
        "id": t.id.as_str(),
        "title": t.title, "artist": t.artist, "album": t.album, "albumArtist": t.album_artist,
        "durationMs": t.duration.map(ms),
        "sampleRate": t.sample_rate, "channels": t.channels, "layout": t.layout,
        "codec": t.codec, "bitrate": t.bitrate, "trackNumber": t.tracknumber, "year": t.year,
        "genre": t.genre, "live": t.live,
        "gain": {
            "trackGainDb": t.gain.track_gain_db, "trackPeak": t.gain.track_peak,
            "albumGainDb": t.gain.album_gain_db, "albumPeak": t.gain.album_peak,
        },
    })
}

pub fn event_json(e: &AudioEvent) -> Value {
    use AudioEvent::*;
    match e {
        CommandCompleted {
            id,
            instance,
            result,
        } => json!({
            "type": "commandCompleted", "id": id.0, "instance": instance.map(|i| i.0),
            "ok": result.is_ok(), "error": result.as_ref().err().map(error_json),
        }),
        TrackLoaded { instance, track } => {
            json!({"type": "trackLoaded", "instance": instance.0, "track": track_json(track)})
        }
        Playing => json!({"type": "playing"}),
        Paused => json!({"type": "paused"}),
        Stopped => json!({"type": "stopped"}),
        Buffering => json!({"type": "buffering"}),
        Finished => json!({"type": "finished"}),
        RequestNext => json!({"type": "requestNext"}),
        RequestPrevious => json!({"type": "requestPrevious"}),
        Position { position, duration } => json!({
            "type": "position", "positionMs": ms(*position), "durationMs": duration.map(ms),
        }),
        Seeked { instance, position } => json!({
            "type": "seeked", "instance": instance.0, "positionMs": ms(*position),
        }),
        NextPrepared { media_id } => json!({"type": "nextPrepared", "mediaId": media_id.as_str()}),
        NextFailed { media_id, error } => json!({
            "type": "nextFailed", "mediaId": media_id.as_str(), "error": error_json(error),
        }),
        EqualizerUpdated(on, gains) => {
            json!({"type": "equalizer", "enabled": on, "gainsDb": gains})
        }
        EqualizerPreampUpdated(db) => json!({"type": "preamp", "db": db}),
        EqAutoApplied(id) => json!({"type": "eqAuto", "mediaId": id.as_ref().map(|i| i.as_str())}),
        VolumeUpdated(v, muted) => json!({"type": "volume", "value": v, "muted": muted}),
        BalanceUpdated(b) => json!({"type": "balance", "value": b}),
        ReplayGainUpdated(on) => json!({"type": "replayGain", "enabled": on}),
        LoudnessUpdated(on) => json!({"type": "loudness", "enabled": on}),
        OutputDeviceUpdated(d) => json!({"type": "outputDevice", "name": d}),
        OutputChanged(info) => json!({"type": "outputChanged", "output": info}),
        IcyMetadata(t) => json!({"type": "icyMetadata", "title": t}),
        StreamReconnecting { attempt } => json!({"type": "streamReconnecting", "attempt": attempt}),
        StreamReconnected => json!({"type": "streamReconnected"}),
        StreamReconnectFailed => json!({"type": "streamReconnectFailed"}),
        RepeatModeUpdated(m) => json!({"type": "repeat", "mode": format!("{m:?}").to_lowercase()}),
        ShuffleUpdated(on) => json!({"type": "shuffle", "enabled": on}),
        Error { instance, error } => json!({
            "type": "error", "instance": instance.map(|i| i.0), "error": error_json(error),
        }),
        EventsDropped(n) => json!({"type": "eventsDropped", "count": n}),
    }
}

pub fn snapshot_json(s: &PlaybackSnapshot) -> Value {
    json!({
        "protocol": PROTOCOL_VERSION,
        "sequence": s.sequence,
        "instance": s.instance.map(|i| i.0),
        "mediaId": s.media_id.as_ref().map(|m| m.as_str()),
        "state": format!("{:?}", s.state).to_lowercase(),
        "positionMs": ms(s.position),
        "decodedPositionMs": ms(s.decoded_position),
        "durationMs": s.duration.map(ms),
        "availableMs": s.available.iter().map(|(a, b)| [ms(*a), ms(*b)]).collect::<Vec<_>>(),
        "capabilities": {
            "seek": s.capabilities.seek, "pause": s.capabilities.pause,
            "resumeAfterError": s.capabilities.resume_after_error, "live": s.capabilities.live,
        },
        "source": s.source,
        "output": s.output,
        "processing": s.processing,
        "eq": s.eq,
        "volume": s.volume,
        "muted": s.muted,
        "next": s.next.as_ref().map(|n| json!({
            "mediaId": n.media_id.as_str(),
            "state": format!("{:?}", n.state).to_lowercase(),
        })),
        "lastError": s.last_error.as_ref().map(error_json),
        "diagnostics": s.diagnostics,
    })
}

/// What a Kotlin `AccessCallback` returns: an access, or a refusal.
pub fn parse_access(json: &str) -> Result<Access, PlayerError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct WireError {
        category: String,
        message: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct WireAccess {
        url: Option<String>,
        #[serde(default)]
        headers: std::collections::BTreeMap<String, String>,
        expires_at_ms: Option<u64>,
        #[serde(default)]
        forward_headers_cross_origin: bool,
        error: Option<WireError>,
    }
    let w: WireAccess = serde_json::from_str(json)
        .map_err(|e| PlayerError::invalid(format!("bad access JSON: {e}")))?;
    if let Some(e) = w.error {
        let category = match e.category.as_str() {
            "AccessExpired" => ErrorCategory::AccessExpired,
            "AccessDenied" => ErrorCategory::AccessDenied,
            "NotFound" => ErrorCategory::NotFound,
            "Network" => ErrorCategory::Network,
            _ => ErrorCategory::AccessDenied,
        };
        return Err(PlayerError::new(
            category,
            e.message.unwrap_or_else(|| "access refused by host".into()),
        ));
    }
    let url = w
        .url
        .ok_or_else(|| PlayerError::invalid("access JSON without url or error"))?;
    let mut a = Access::new(url);
    a.headers = w.headers.into_iter().collect();
    a.expires_at = w
        .expires_at_ms
        .map(|t| std::time::UNIX_EPOCH + Duration::from_millis(t));
    if w.forward_headers_cross_origin {
        a.redirects = oneamp_core::access::RedirectPolicy::HeadersToAnyOrigin;
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<Box<dyn AccessProvider>> {
        None
    }

    #[test]
    fn parses_commands_and_media() {
        let c = parse_command(
            r#"{"type":"load","paused":true,"media":{"id":"item-1","url":"https://h/a?cap=x",
               "headers":{"X":"y"},"startMs":1500,"durationMs":200000,
               "metadata":{"title":"T"},"gain":{"trackGainDb":-3.5},"context":"album"}}"#,
            &none,
        )
        .unwrap();
        let AudioCommand::Load { media, paused } = c else {
            panic!()
        };
        assert!(paused);
        assert_eq!(media.id.as_str(), "item-1");
        assert_eq!(media.start_position, Duration::from_millis(1500));
        assert_eq!(media.metadata.title.as_deref(), Some("T"));
        assert_eq!(media.gain.unwrap().track_gain_db, Some(-3.5));
        assert_eq!(media.listening_context, ListeningContext::Album);

        assert!(matches!(
            parse_command(r#"{"type":"seek","positionMs":42}"#, &none).unwrap(),
            AudioCommand::Seek(d) if d == Duration::from_millis(42)
        ));
        assert!(matches!(
            parse_command(r#"{"type":"setNext","media":null}"#, &none).unwrap(),
            AudioCommand::SetNext(None)
        ));
        let err = parse_command(r#"{"type":"load","media":{"id":"x"}}"#, &none).unwrap_err();
        assert_eq!(err.category, ErrorCategory::InvalidRequest);
        let err = parse_command(
            r#"{"type":"load","media":{"id":"x","url":"u","accessCallback":true}}"#,
            &none,
        )
        .unwrap_err();
        assert!(err.message.contains("AccessCallback"));
        assert!(parse_command(r#"{"type":"settings","settings":{"mono":true}}"#, &none).is_ok());
        assert!(parse_command("{", &none).is_err());
    }

    #[test]
    fn access_answers() {
        let a = parse_access(
            r#"{"url":"https://h/x","headers":{"Authorization":"Bearer t"},"expiresAtMs":1}"#,
        )
        .unwrap();
        assert_eq!(a.headers.len(), 1);
        assert!(a.expires_at.is_some());
        let e =
            parse_access(r#"{"error":{"category":"AccessDenied","message":"no"}}"#).unwrap_err();
        assert_eq!(e.category, ErrorCategory::AccessDenied);
    }

    #[test]
    fn events_are_camel_case_with_milliseconds() {
        let v = event_json(&AudioEvent::Position {
            position: Duration::from_millis(1234),
            duration: None,
        });
        assert_eq!(v["type"], "position");
        assert_eq!(v["positionMs"], 1234);
        let v = event_json(&AudioEvent::Error {
            instance: None,
            error: PlayerError::new(ErrorCategory::Network, "x"),
        });
        assert_eq!(v["error"]["category"], "Network");
        assert_eq!(v["error"]["recoverable"], true);
        let s = snapshot_json(&PlaybackSnapshot::default());
        assert_eq!(s["state"], "idle");
        assert_eq!(s["protocol"], 1);
    }
}
