//! Read and write user-facing audio file tags.
//!
//! Symphonia (used for playback) can *read* tags but exposes no write
//! path. lofty fills that gap with a uniform API across ID3v1/v2,
//! Vorbis Comments, MP4 atoms, RIFF and APE — the same formats this
//! player can decode — so the editable surface lines up with what
//! users can actually load.
//!
//! Why `EditableTags` rather than reusing `TrackInfo`:
//! - `TrackInfo` aggregates *playback*-relevant data (codec, sample
//!   rate, ReplayGain) that the user can't edit. Mixing the two would
//!   suggest fields like `bitrate` are mutable.
//! - Tracks round-trip through this struct, so any field that's
//!   `None` on read is also `None` on write — the editor doesn't
//!   accidentally clear an album-artist or year just because the UI
//!   didn't render that field.

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagExt};
use std::path::Path;

/// Editable subset of an audio file's tags. Every field is independently
/// `Option<…>` so the UI can clear individual tags by saving `None`.
/// Numeric fields parse from / format to plain decimal — the wrapper
/// hides lofty's per-format encoding quirks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditableTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub tracknumber: Option<u32>,
    pub comment: Option<String>,
}

impl EditableTags {
    /// Read the primary tag from `path`. Returns an empty struct (all
    /// fields `None`) when the file has no tags — distinct from an I/O
    /// error so the editor can still present a blank form to fill in.
    pub fn read<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let tagged = Probe::open(path)
            .with_context(|| format!("lofty failed to open {} for tag reading", path.display()))?
            .read()
            .with_context(|| format!("lofty failed to parse tags in {}", path.display()))?;

        let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
            return Ok(Self::default());
        };

        Ok(Self {
            title: get_text(tag, ItemKey::TrackTitle),
            artist: get_text(tag, ItemKey::TrackArtist),
            album: get_text(tag, ItemKey::AlbumTitle),
            album_artist: get_text(tag, ItemKey::AlbumArtist),
            genre: get_text(tag, ItemKey::Genre),
            // ID3v2.4 stores the year as a `TDRC` (RecordingDate)
            // timestamp like `"1994-07-15"`; ID3v2.3 / RIFF / APE keep
            // it under the dedicated `Year` key. Read both, falling
            // back from one to the other and parsing the leading 4
            // digits if necessary.
            year: get_text(tag, ItemKey::Year)
                .and_then(|s| parse_year(&s))
                .or_else(|| get_text(tag, ItemKey::RecordingDate).and_then(|s| parse_year(&s))),
            tracknumber: get_text(tag, ItemKey::TrackNumber).and_then(|s| parse_tracknumber(&s)),
            comment: get_text(tag, ItemKey::Comment),
        })
    }

    /// Write `self` back to `path`, replacing the matching tag fields
    /// in place. Fields set to `None` are *removed* from the tag — the
    /// editor's "clear this field" gesture has to map to something. We
    /// keep every other tag field (e.g. ReplayGain, MusicBrainz IDs)
    /// untouched so a quick title edit doesn't strip pre-existing
    /// metadata.
    pub fn write<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        let mut tagged = Probe::open(path)
            .with_context(|| format!("lofty failed to open {} for tag writing", path.display()))?
            .read()
            .with_context(|| format!("lofty failed to parse tags in {}", path.display()))?;

        // `primary_tag_mut` returns the format's "native" tag (ID3v2
        // on MP3, Vorbis comments on FLAC, MP4 atoms on M4A, …). If
        // the file has no tag yet, lofty needs us to insert an empty
        // one of the native type before we can mutate it.
        if tagged.primary_tag().is_none() {
            let kind = tagged.primary_tag_type();
            tagged.insert_tag(Tag::new(kind));
        }
        let Some(tag) = tagged.primary_tag_mut() else {
            anyhow::bail!(
                "{} has no writeable tag (unsupported container?)",
                path.display()
            );
        };

        set_or_clear(tag, ItemKey::TrackTitle, self.title.as_deref());
        set_or_clear(tag, ItemKey::TrackArtist, self.artist.as_deref());
        set_or_clear(tag, ItemKey::AlbumTitle, self.album.as_deref());
        set_or_clear(tag, ItemKey::AlbumArtist, self.album_artist.as_deref());
        set_or_clear(tag, ItemKey::Genre, self.genre.as_deref());
        set_or_clear(
            tag,
            ItemKey::Year,
            self.year.as_ref().map(|y| y.to_string()).as_deref(),
        );
        // Some MP3 taggers write the year only into `TDRC`
        // (RecordingDate). Clear that too so the user's edit isn't
        // shadowed by stale data — but only when the user actually
        // touched the year field (Some(_) overwrites; None just
        // clears). We always remove the dangling RecordingDate so the
        // canonical Year field is the single source of truth.
        tag.remove_key(ItemKey::RecordingDate);
        if let Some(y) = self.year {
            tag.insert_text(ItemKey::RecordingDate, y.to_string());
        }
        set_or_clear(
            tag,
            ItemKey::TrackNumber,
            self.tracknumber.as_ref().map(|n| n.to_string()).as_deref(),
        );
        set_or_clear(tag, ItemKey::Comment, self.comment.as_deref());

        // lofty's WriteOptions default already does the right thing
        // (preserve padding, keep the same tag types that were
        // present); pass it explicitly so a future API change doesn't
        // silently flip a flag.
        tag.save_to_path(path, WriteOptions::default())
            .with_context(|| format!("lofty failed to write tags to {}", path.display()))?;
        Ok(())
    }
}

fn get_text(tag: &Tag, key: ItemKey) -> Option<String> {
    tag.get_string(key)
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn set_or_clear(tag: &mut Tag, key: ItemKey, value: Option<&str>) {
    match value {
        Some(v) if !v.is_empty() => {
            tag.insert_text(key, v.to_string());
        }
        _ => {
            tag.remove_key(key);
        }
    }
}

/// `"3/12"`, `"03"`, etc. — keep the leading run of ASCII digits.
fn parse_tracknumber(raw: &str) -> Option<u32> {
    let digits: String = raw
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// First 4-digit year inside a date-like string.
fn parse_year(raw: &str) -> Option<u32> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit()) {
            return std::str::from_utf8(&bytes[i..i + 4]).ok()?.parse().ok();
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editable_tags_default_is_all_none() {
        let t = EditableTags::default();
        assert!(t.title.is_none());
        assert!(t.year.is_none());
        assert!(t.tracknumber.is_none());
    }

    #[test]
    fn parse_year_strips_extras() {
        assert_eq!(parse_year("1994"), Some(1994));
        assert_eq!(parse_year("1994-07-15"), Some(1994));
        assert_eq!(parse_year("recorded 2003 by"), Some(2003));
        assert_eq!(parse_year("not a year"), None);
    }

    #[test]
    fn parse_tracknumber_handles_x_of_y() {
        assert_eq!(parse_tracknumber("3/12"), Some(3));
        assert_eq!(parse_tracknumber(" 03 "), Some(3));
        assert_eq!(parse_tracknumber("none"), None);
    }
}
