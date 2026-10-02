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

        Ok(Self::from_tag(tag))
    }

    fn from_tag(tag: &Tag) -> Self {
        Self {
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
        }
    }

    /// Write `self` back to `path`. Only fields that differ from what
    /// the file holds are touched; a changed field set to `None` is
    /// *removed* from the tag — the editor's "clear this field" gesture
    /// has to map to something.
    ///
    /// Leaving unchanged fields alone matters because this struct is
    /// lossy: a `1994-07-15` recording date reads as year `1994` and a
    /// `3/12` track number as `3`, so rewriting them on a title-only
    /// edit would throw the rest away. Every other tag field (ReplayGain,
    /// MusicBrainz IDs, …) is untouched too.
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

        let current = Self::from_tag(tag);
        let mut text = |key: ItemKey, new: &Option<String>, old: &Option<String>| {
            if new != old {
                set_or_clear(tag, key, new.as_deref());
            }
        };
        text(ItemKey::TrackTitle, &self.title, &current.title);
        text(ItemKey::TrackArtist, &self.artist, &current.artist);
        text(ItemKey::AlbumTitle, &self.album, &current.album);
        text(
            ItemKey::AlbumArtist,
            &self.album_artist,
            &current.album_artist,
        );
        text(ItemKey::Genre, &self.genre, &current.genre);
        text(ItemKey::Comment, &self.comment, &current.comment);
        if self.year != current.year {
            let year = self.year.map(|y| y.to_string());
            // Some taggers keep the year only in `TDRC` (RecordingDate):
            // set both so the edit isn't shadowed by the old date.
            set_or_clear(tag, ItemKey::Year, year.as_deref());
            set_or_clear(tag, ItemKey::RecordingDate, year.as_deref());
        }
        if self.tracknumber != current.tracknumber {
            let track = self.tracknumber.map(|n| n.to_string());
            set_or_clear(tag, ItemKey::TrackNumber, track.as_deref());
        }

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
pub(crate) fn parse_year(raw: &str) -> Option<u32> {
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
    fn write_keeps_the_fields_the_user_did_not_change() {
        let path = std::env::temp_dir().join(format!("oneamp_tags_{}.flac", std::process::id()));
        std::fs::write(&path, include_bytes!("../tests/fixtures/tone16.flac")).unwrap();
        let raw = |key: ItemKey| {
            let tagged = Probe::open(&path).unwrap().read().unwrap();
            get_text(tagged.primary_tag().unwrap(), key)
        };
        {
            let mut tagged = Probe::open(&path).unwrap().read().unwrap();
            if tagged.primary_tag().is_none() {
                let kind = tagged.primary_tag_type();
                tagged.insert_tag(Tag::new(kind));
            }
            let tag = tagged.primary_tag_mut().unwrap();
            tag.insert_text(ItemKey::RecordingDate, "1994-07-15".into());
            tag.insert_text(ItemKey::TrackNumber, "3/12".into());
            tag.save_to_path(&path, WriteOptions::default()).unwrap();
        }
        let date_before = raw(ItemKey::RecordingDate);
        let track_before = raw(ItemKey::TrackNumber);
        assert_eq!(date_before.as_deref(), Some("1994-07-15"));

        // Title-only edit: date and track number survive verbatim.
        let mut tags = EditableTags::read(&path).unwrap();
        assert_eq!((tags.year, tags.tracknumber), (Some(1994), Some(3)));
        tags.title = Some("New title".into());
        tags.write(&path).unwrap();
        assert_eq!(raw(ItemKey::TrackTitle).as_deref(), Some("New title"));
        assert_eq!(raw(ItemKey::RecordingDate), date_before);
        assert_eq!(raw(ItemKey::TrackNumber), track_before);

        // Changing the year does replace the date.
        tags.year = Some(2001);
        tags.write(&path).unwrap();
        assert_eq!(EditableTags::read(&path).unwrap().year, Some(2001));
        assert_eq!(raw(ItemKey::RecordingDate).as_deref(), Some("2001"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parse_tracknumber_handles_x_of_y() {
        assert_eq!(parse_tracknumber("3/12"), Some(3));
        assert_eq!(parse_tracknumber(" 03 "), Some(3));
        assert_eq!(parse_tracknumber("none"), None);
    }
}
