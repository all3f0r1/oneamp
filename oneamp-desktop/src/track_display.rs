use oneamp_core::TrackInfo;
use std::path::Path;

/// Format track information for display
pub struct TrackDisplay;

impl TrackDisplay {
    /// Get display title for a track
    /// Format: "ARTIST - TITLE" if both available, otherwise filename
    pub fn get_title(track: &TrackInfo) -> String {
        match (&track.artist, &track.title) {
            (Some(artist), Some(title)) => format!("{} - {}", artist, title),
            (None, Some(title)) => title.clone(),
            (Some(artist), None) => artist.clone(),
            (None, None) => Self::filename_fallback(&track.path),
        }
    }

    /// Get display artist for a track
    pub fn get_artist(track: &TrackInfo) -> String {
        track
            .artist
            .clone()
            .unwrap_or_else(|| "Unknown Artist".to_string())
    }

    /// Get display title (song name only) for a track
    pub fn get_title_only(track: &TrackInfo) -> String {
        track
            .title
            .clone()
            .unwrap_or_else(|| Self::filename_fallback(&track.path))
    }

    /// Get display album for a track
    pub fn get_album(track: &TrackInfo) -> String {
        track
            .album
            .clone()
            .unwrap_or_else(|| "Unknown Album".to_string())
    }

    /// Get filename without extension as fallback
    fn filename_fallback(path: &Path) -> String {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string()
    }

    /// Format duration as MM:SS
    pub fn format_duration(seconds: f32) -> String {
        let total_secs = seconds as u32;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    /// Format duration as digital display (MM:SS or HH:MM:SS for long tracks)
    pub fn format_duration_digital(seconds: f32) -> String {
        let total_secs = seconds as u32;
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;

        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, mins, secs)
        } else {
            format!("{:02}:{:02}", mins, secs)
        }
    }

    /// Get technical info string (codec, bitrate, sample rate, channels)
    /// Format: "MP3 • 320kbps • 44.1kHz • Stereo"
    pub fn get_technical_info(track: &TrackInfo) -> String {
        track.format_audio_info()
    }

    /// Get legacy technical info string (sample rate and channels only)
    /// Used for backward compatibility
    pub fn get_technical_info_legacy(track: &TrackInfo) -> String {
        let mut parts = Vec::new();

        if let Some(sr) = track.sample_rate {
            parts.push(format!("{}kHz", sr / 1000));
        }

        if let Some(ch) = track.channels {
            parts.push(match ch {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                _ => format!("{}ch", ch),
            });
        }

        if parts.is_empty() {
            "No info".to_string()
        } else {
            parts.join(" • ")
        }
    }

    /// Create a scrolling text effect for long titles
    /// Returns a substring that can be animated
    pub fn scroll_text(text: &str, max_width: usize, offset: usize) -> String {
        if text.len() <= max_width {
            return text.to_string();
        }

        // Add padding for smooth scrolling
        let padded = format!("{}   ", text);
        let len = padded.len();
        let start = offset % len;

        // Create a circular buffer effect
        let mut result = String::new();
        for i in 0..max_width {
            let idx = (start + i) % len;
            result.push(padded.chars().nth(idx).unwrap_or(' '));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_format_duration() {
        assert_eq!(TrackDisplay::format_duration(0.0), "00:00");
        assert_eq!(TrackDisplay::format_duration(65.0), "01:05");
        assert_eq!(TrackDisplay::format_duration(3661.0), "01:01:01");
    }

    #[test]
    fn test_scroll_text() {
        let text = "Long Song Title";
        let scrolled = TrackDisplay::scroll_text(text, 10, 0);
        assert_eq!(scrolled.len(), 10);
    }

    #[test]
    fn test_get_title() {
        let track = TrackInfo {
            path: PathBuf::from("/music/song.mp3"),
            title: Some("Test Song".to_string()),
            artist: Some("Test Artist".to_string()),
            album: None,
            duration_secs: Some(180.0),
            sample_rate: Some(44100),
            channels: Some(2),
            codec: Some("MP3".to_string()),
            bitrate: Some(320000),
        };

        assert_eq!(TrackDisplay::get_title(&track), "Test Artist - Test Song");
    }

    #[test]
    fn test_get_technical_info() {
        let track = TrackInfo {
            path: PathBuf::from("/music/song.mp3"),
            title: Some("Test Song".to_string()),
            artist: Some("Test Artist".to_string()),
            album: None,
            duration_secs: Some(180.0),
            sample_rate: Some(44100),
            channels: Some(2),
            codec: Some("MP3".to_string()),
            bitrate: Some(320000),
        };

        let info = TrackDisplay::get_technical_info(&track);
        assert!(info.contains("MP3"));
        assert!(info.contains("320kbps"));
        assert!(info.contains("44.1kHz"));
        assert!(info.contains("Stereo"));
    }
}
