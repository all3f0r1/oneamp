//! Descriptive information about the media that is loaded: the host's
//! metadata, completed field by field from the stream's own tags.

use crate::dsp::MediaGain;
use crate::error::PlayerError;
use crate::media::{MediaDescriptor, MediaId};
use std::path::Path;
use std::time::Duration;
use symphonia::core::formats::FormatReader;

/// Track metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub id: MediaId,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    /// Whole-media duration; `None` for live or unknown.
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    /// Channel layout label ("stereo", "5.1").
    pub layout: Option<String>,
    /// Codec short name ("MP3", "FLAC").
    pub codec: Option<String>,
    /// Average bitrate in bit/s, when the size and duration are known.
    pub bitrate: Option<u32>,
    pub tracknumber: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    /// Normalisation figures in use (host-supplied or from tags).
    pub gain: MediaGain,
    pub live: bool,
}

impl TrackInfo {
    pub fn duration_secs(&self) -> Option<f32> {
        self.duration.map(|d| d.as_secs_f32())
    }

    /// Read a local file's tags and format (no playback).
    pub fn from_file(path: &Path) -> Result<Self, PlayerError> {
        let mut player = crate::symphonia_player::SymphoniaPlayer::load(path)?;
        let desc = MediaDescriptor::file(path);
        let size = std::fs::metadata(path).ok().map(|m| m.len());
        Ok(Self::assemble(&desc, &mut player, size))
    }

    /// Merge the host's knowledge (`desc`) with what the stream says.
    pub(crate) fn assemble(
        desc: &MediaDescriptor,
        player: &mut crate::symphonia_player::SymphoniaPlayer,
        byte_len: Option<u64>,
    ) -> Self {
        let tags = read_tags(player.format_reader_mut());
        let m = &desc.metadata;
        let duration = desc
            .duration
            .or_else(|| player.duration().map(|d| d + desc.timeline_offset));
        // Average bitrate: exact for CBR, approximate for VBR — what
        // Winamp's 3-digit field shows anyway.
        let bitrate = match (byte_len, player.duration()) {
            (Some(len), Some(d)) if d.as_secs_f64() > 0.0 => {
                Some(((len as f64 * 8.0) / d.as_secs_f64()).round() as u32)
            }
            _ => None,
        };
        let gain = desc.gain.unwrap_or(MediaGain {
            track_gain_db: tags.rg_track,
            track_peak: tags.rg_track_peak,
            album_gain_db: tags.rg_album,
            album_peak: tags.rg_album_peak,
        });
        Self {
            id: desc.id.clone(),
            title: m.title.clone().or(tags.title),
            artist: m.artist.clone().or(tags.artist),
            album: m.album.clone().or(tags.album),
            album_artist: m.album_artist.clone().or(tags.album_artist),
            duration,
            sample_rate: Some(player.sample_rate()),
            channels: Some(player.channels()),
            layout: Some(player.layout().label()),
            codec: Some(player.codec().to_string()),
            bitrate,
            tracknumber: m.track_number.or(tags.tracknumber),
            year: m.year.or(tags.year),
            genre: m.genre.clone().or(tags.genre),
            gain,
            live: desc.is_live(),
        }
    }
}

/// Parse a ReplayGain value (`"-6.78 dB"`, `"-6.78"`, `"+1.23 DB "`).
/// `None` on malformed input rather than guessing 0 dB, which would let
/// a broken tag pass for "no gain needed".
pub(crate) fn parse_replaygain_db(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    // Measure the suffix in chars: a byte offset can land mid-codepoint
    // when the string ends with a multi-byte character.
    let suffix_len: usize = trimmed.chars().rev().take(2).map(char::len_utf8).sum();
    let tail = &trimmed[trimmed.len() - suffix_len..];
    let numeric = if tail.eq_ignore_ascii_case("db") {
        trimmed[..trimmed.len() - suffix_len].trim()
    } else {
        trimmed
    };
    numeric.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn parse_peak(raw: &str) -> Option<f32> {
    raw.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
}

#[derive(Default)]
struct Tags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    tracknumber: Option<u32>,
    year: Option<u32>,
    genre: Option<String>,
    rg_track: Option<f32>,
    rg_album: Option<f32>,
    rg_track_peak: Option<f32>,
    rg_album_peak: Option<f32>,
}

/// The probe folds leading (ID3v2) and trailing (ID3v1, APE) tags into
/// the reader's metadata log ahead of container metadata, so walking the
/// revisions oldest-first keeps ID3v2 as the source of truth.
fn read_tags(format: &mut dyn FormatReader) -> Tags {
    let mut tags = Tags::default();
    let mut metadata = format.metadata();
    while let Some(rev) = metadata.pop() {
        tags.consume(&rev);
    }
    if let Some(rev) = metadata.current() {
        tags.consume(rev);
    }
    tags
}

impl Tags {
    fn consume(&mut self, rev: &symphonia::core::meta::MetadataRevision) {
        use symphonia::core::meta::StandardTag;
        fn set(slot: &mut Option<String>, v: &str) {
            if slot.is_none() {
                *slot = Some(v.to_string());
            }
        }
        for tag in &rev.media.tags {
            match &tag.std {
                Some(StandardTag::TrackTitle(v)) => set(&mut self.title, v),
                Some(StandardTag::Artist(v)) => set(&mut self.artist, v),
                Some(StandardTag::Album(v)) => set(&mut self.album, v),
                Some(StandardTag::AlbumArtist(v)) => set(&mut self.album_artist, v),
                Some(StandardTag::Genre(v)) => set(&mut self.genre, v),
                Some(StandardTag::TrackNumber(n)) if self.tracknumber.is_none() => {
                    self.tracknumber = u32::try_from(*n).ok();
                }
                Some(
                    StandardTag::RecordingDate(v)
                    | StandardTag::ReleaseDate(v)
                    | StandardTag::OriginalRecordingDate(v)
                    | StandardTag::OriginalReleaseDate(v),
                ) if self.year.is_none() => {
                    self.year = crate::parse_year(v);
                }
                Some(
                    StandardTag::RecordingYear(y)
                    | StandardTag::ReleaseYear(y)
                    | StandardTag::OriginalReleaseYear(y),
                ) if self.year.is_none() => {
                    self.year = Some(u32::from(*y));
                }
                Some(StandardTag::ReplayGainTrackGain(v)) if self.rg_track.is_none() => {
                    self.rg_track = parse_replaygain_db(v);
                }
                Some(StandardTag::ReplayGainAlbumGain(v)) if self.rg_album.is_none() => {
                    self.rg_album = parse_replaygain_db(v);
                }
                Some(StandardTag::ReplayGainTrackPeak(v)) if self.rg_track_peak.is_none() => {
                    self.rg_track_peak = parse_peak(v);
                }
                Some(StandardTag::ReplayGainAlbumPeak(v)) if self.rg_album_peak.is_none() => {
                    self.rg_album_peak = parse_peak(v);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_replaygain_db_handles_real_world_inputs() {
        assert_eq!(parse_replaygain_db("-6.78 dB"), Some(-6.78));
        assert_eq!(parse_replaygain_db("+1.23 dB"), Some(1.23));
        assert_eq!(parse_replaygain_db("-3.5"), Some(-3.5));
        assert_eq!(parse_replaygain_db("-2.0 DB"), Some(-2.0));
        assert_eq!(parse_replaygain_db("0.0 Db"), Some(0.0));
        assert_eq!(parse_replaygain_db("  -4.4 dB  "), Some(-4.4));
        assert_eq!(parse_replaygain_db("not a number"), None);
        assert_eq!(parse_replaygain_db(""), None);
        assert_eq!(parse_replaygain_db("dB"), None);
        assert_eq!(parse_replaygain_db("inf"), None);
        assert_eq!(parse_replaygain_db("NaN dB"), None);
    }

    #[test]
    fn parse_replaygain_db_does_not_panic_on_multibyte_suffix() {
        assert_eq!(parse_replaygain_db("é"), None);
        assert_eq!(parse_replaygain_db("-6.78 é"), None);
        assert_eq!(parse_replaygain_db("日"), None);
        assert_eq!(parse_replaygain_db("d"), None);
        assert_eq!(parse_replaygain_db(" "), None);
    }

    #[test]
    fn host_metadata_wins_over_tags() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tone16.flac"
        ));
        let mut desc = MediaDescriptor::file(path);
        desc.metadata.title = Some("From the library".into());
        let mut p = crate::symphonia_player::SymphoniaPlayer::load(path).unwrap();
        let info = TrackInfo::assemble(&desc, &mut p, None);
        assert_eq!(info.title.as_deref(), Some("From the library"));
        assert_eq!(info.sample_rate, Some(22050));
        assert_eq!(info.codec.as_deref(), Some("FLAC"));
        assert!(!info.live);
    }
}
