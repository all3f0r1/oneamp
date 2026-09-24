//! Listening session restored at launch: playlist, current track,
//! play-next queue and playhead position. Stored in
//! `<config_dir>/oneamp/session.json`, written atomically.
//!
//! Playback never starts on its own: the current track is selected and
//! its position is applied when the user presses Play. Entries whose
//! file has disappeared are kept and flagged, so a disconnected drive
//! doesn't silently shrink the playlist.

use anyhow::{Context, Result};
use oneamp_core::PlaylistEntry;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub entries: Vec<PlaylistEntry>,
    #[serde(default)]
    pub current: Option<usize>,
    #[serde(default)]
    pub queue: Vec<usize>,
    /// Playhead of the current track when the session was saved.
    #[serde(default)]
    pub position_secs: f32,
}

pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("oneamp").join("session.json"))
}

impl Session {
    /// Missing file = empty session. A corrupt file is reported and
    /// treated as empty too: losing a playlist beats refusing to start.
    pub fn load(path: &Path) -> Self {
        let Ok(content) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str::<Self>(&content)
            .map(|mut s| {
                // Clamp indices a hand-edited file could break.
                let n = s.entries.len();
                s.current = s.current.filter(|&c| c < n);
                s.queue.retain(|&q| q < n);
                s
            })
            .unwrap_or_else(|e| {
                eprintln!("Ignoring unreadable session {}: {}", path.display(), e);
                Self::default()
            })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).context("Failed to create config directory")?;
        }
        let content = serde_json::to_string(self).context("Failed to serialize session")?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp).context("Failed to create temp session file")?;
            f.write_all(content.as_bytes())
                .context("Failed to write temp session file")?;
            f.sync_all().context("Failed to fsync temp session file")?;
        }
        fs::rename(&tmp, path).context("Failed to rename temp session file into place")?;
        Ok(())
    }
}

/// Local file that no longer exists. URLs (stored as paths) are never
/// flagged: their availability is only known by connecting.
pub fn is_unavailable(path: &Path) -> bool {
    let s = path.to_string_lossy();
    !(s.starts_with("http://") || s.starts_with("https://")) && !path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_index_clamping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        assert_eq!(Session::load(&path), Session::default());

        let s = Session {
            entries: vec![
                PlaylistEntry::new(PathBuf::from("/a.mp3")),
                PlaylistEntry::new(PathBuf::from("/b.mp3")),
            ],
            current: Some(1),
            queue: vec![0],
            position_secs: 42.5,
        };
        s.save(&path).unwrap();
        assert_eq!(Session::load(&path), s);

        fs::write(
            &path,
            r#"{"entries":[{"path":"/a.mp3","title":null,"artist":null,"album":null,"duration":null}],"current":5,"queue":[0,3]}"#,
        )
        .unwrap();
        let loaded = Session::load(&path);
        assert_eq!(loaded.current, None);
        assert_eq!(loaded.queue, vec![0]);

        fs::write(&path, "not json").unwrap();
        assert_eq!(Session::load(&path), Session::default());
    }

    #[test]
    fn urls_are_never_unavailable() {
        assert!(!is_unavailable(Path::new("http://radio.example/stream")));
        assert!(is_unavailable(Path::new("/definitely/not/here.mp3")));
    }
}
