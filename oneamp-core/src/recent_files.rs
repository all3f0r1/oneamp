//! Recent files history management
//!
//! This module provides functionality to track and persist recently played files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// Maximum number of recent files to keep
const MAX_RECENT_FILES: usize = 20;

/// Recent file entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentFile {
    /// Path to the file
    pub path: PathBuf,
    /// Last played timestamp (Unix timestamp)
    pub last_played: u64,
    /// Play count
    pub play_count: u32,
}

impl RecentFile {
    /// Create a new recent file entry
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_played: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            play_count: 1,
        }
    }
}

/// Recent files manager
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentFiles {
    /// List of recent files (most recent first)
    files: VecDeque<RecentFile>,
    /// Maximum number of files to keep
    max_files: usize,
}

impl RecentFiles {
    /// Create a new recent files manager
    pub fn new() -> Self {
        Self {
            files: VecDeque::new(),
            max_files: MAX_RECENT_FILES,
        }
    }

    /// Create with custom max files limit
    pub fn with_max_files(max_files: usize) -> Self {
        Self {
            files: VecDeque::new(),
            max_files,
        }
    }

    /// Add a file to recent files
    pub fn add_file(&mut self, path: PathBuf) {
        // Check if file already exists
        if let Some(pos) = self.files.iter().position(|f| f.path == path) {
            // Move to front and update metadata
            let mut file = self.files.remove(pos).unwrap();
            file.last_played = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            file.play_count = file.play_count.saturating_add(1);
            self.files.push_front(file);
        } else {
            // Add new file
            self.files.push_front(RecentFile::new(path));

            // Trim to max size
            while self.files.len() > self.max_files {
                self.files.pop_back();
            }
        }
    }

    /// Get all recent files
    pub fn files(&self) -> Vec<&RecentFile> {
        self.files.iter().collect()
    }

    /// Get recent file at index
    pub fn get(&self, index: usize) -> Option<&RecentFile> {
        self.files.get(index)
    }

    /// Clear all recent files
    pub fn clear(&mut self) {
        self.files.clear();
    }

    /// Remove a specific file
    pub fn remove(&mut self, path: &Path) -> bool {
        if let Some(pos) = self.files.iter().position(|f| f.path == path) {
            self.files.remove(pos);
            true
        } else {
            false
        }
    }

    /// Get number of recent files
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Check if recent files is empty
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Save recent files to JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize recent files")?;
        fs::write(path, json).context("Failed to write recent files")?;
        Ok(())
    }

    /// Load recent files from JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let json = fs::read_to_string(path).context("Failed to read recent files")?;
        let recent_files =
            serde_json::from_str(&json).context("Failed to deserialize recent files")?;
        Ok(recent_files)
    }

    /// Load or create new if file doesn't exist
    pub fn load_or_new<P: AsRef<Path>>(path: P) -> Self {
        Self::load(path).unwrap_or_else(|_| Self::new())
    }
}

impl Default for RecentFiles {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_file() {
        let mut recent = RecentFiles::new();

        recent.add_file(PathBuf::from("/path/to/song1.mp3"));
        recent.add_file(PathBuf::from("/path/to/song2.mp3"));

        assert_eq!(recent.len(), 2);
        assert_eq!(recent.files()[0].path, PathBuf::from("/path/to/song2.mp3"));
    }

    #[test]
    fn test_duplicate_file() {
        let mut recent = RecentFiles::new();

        recent.add_file(PathBuf::from("/path/to/song.mp3"));
        recent.add_file(PathBuf::from("/path/to/song.mp3"));

        assert_eq!(recent.len(), 1);
        assert_eq!(recent.files()[0].play_count, 2);
    }

    #[test]
    fn duplicate_file_play_count_saturates() {
        let mut recent = RecentFiles::new();
        recent.files.push_front(RecentFile {
            path: PathBuf::from("/path/to/song.mp3"),
            last_played: 0,
            play_count: u32::MAX,
        });

        recent.add_file(PathBuf::from("/path/to/song.mp3"));

        assert_eq!(recent.len(), 1);
        assert_eq!(recent.files()[0].play_count, u32::MAX);
    }

    #[test]
    fn test_max_files() {
        let mut recent = RecentFiles::with_max_files(3);

        recent.add_file(PathBuf::from("song1.mp3"));
        recent.add_file(PathBuf::from("song2.mp3"));
        recent.add_file(PathBuf::from("song3.mp3"));
        recent.add_file(PathBuf::from("song4.mp3"));

        assert_eq!(recent.len(), 3);
        assert_eq!(recent.files()[0].path, PathBuf::from("song4.mp3"));
    }

    #[test]
    fn test_remove() {
        let mut recent = RecentFiles::new();

        recent.add_file(PathBuf::from("song1.mp3"));
        recent.add_file(PathBuf::from("song2.mp3"));

        assert!(recent.remove(Path::new("song1.mp3")));
        assert_eq!(recent.len(), 1);
        assert!(!recent.remove(Path::new("song3.mp3")));
    }

    #[test]
    fn test_clear() {
        let mut recent = RecentFiles::new();

        recent.add_file(PathBuf::from("song1.mp3"));
        recent.add_file(PathBuf::from("song2.mp3"));

        recent.clear();
        assert_eq!(recent.len(), 0);
        assert!(recent.is_empty());
    }
}
