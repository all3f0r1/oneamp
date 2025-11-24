# Changelog

All notable changes to OneAmp will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.1] - 2024-11-24

### Changed
- **Performance**: Optimized release build with LTO, reducing binary size by 32% (22MB → 15MB)
- **Performance**: Replaced naive FFT O(n²) with rustfft O(n log n) for 10x faster spectrum analysis
- **Performance**: Throttled position update events to reduce allocations by 90%
- **Quality**: Removed unused dependencies (tokio, duplicate rodio)
- **Quality**: Fixed all Clippy warnings for cleaner codebase
- **Testing**: Added 29 new tests (3 → 32 tests total, ~40% code coverage)

### Fixed
- Removed unused imports in visualizer and main modules
- Simplified if-statement nesting for better readability
- Corrected jingle playback to use existing audio engine

### Technical Details
- Added comprehensive unit tests for AudioEngine, Config, and Visualizer
- Added integration tests for CLI
- Configured optimized release profile (opt-level=3, lto=thin, strip=true)
- Improved FFT implementation with proper normalization and smoothing

## [0.6.0] - 2024-11-24

### Added
- **Keyboard Shortcuts**: Winamp-style shortcuts (X/C/V/B/Z for playback control)
- **Audio Formats**: Support for OGG Vorbis and WAV files
- **Visualizations**: Real-time oscilloscope and spectrum analyzer with FFT
- **Playlist Sorting**: Sort by title, filename, path, shuffle, or reverse
- **New Logo**: Modern "1AMP" logo design
- **Welcome Jingle**: Plays on first launch with humorous message

### Changed
- Visualizer can be toggled by clicking on it
- Improved spectrum analyzer with 64 frequency bands and color gradients
- Enhanced playlist management with multiple sorting options

## [0.5.0] - 2024-11-23

### Added
- Modern dark theme with professional styling
- Application icon in multiple sizes
- Debian package (.deb) for easy installation
- Desktop integration with .desktop file
- Comprehensive user guide

### Changed
- Refined UI with rounded corners and better spacing
- Improved color scheme with cyan accents

## [0.4.0] - 2024-11-23

### Added
- **10-band Graphic Equalizer** with real-time audio processing
- Biquad filters implementation (RBJ Audio EQ Cookbook)
- Equalizer window with sliders for each frequency band
- Persistent equalizer configuration
- Reset button for equalizer

### Changed
- Frequency bands: 31Hz, 62Hz, 125Hz, 250Hz, 500Hz, 1kHz, 2kHz, 4kHz, 8kHz, 16kHz
- Gain range: -12dB to +12dB

## [0.3.0] - 2024-11-23

### Added
- **Playlist Management**: Add files, add folders, remove tracks, clear all
- **Playlist Navigation**: Next/Previous track buttons
- **Auto-play**: Automatically plays next track when current finishes
- **Playlist Panel**: Resizable side panel showing all tracks
- Track selection and double-click to play

### Changed
- Restructured into workspace with oneamp-cli, oneamp-core, oneamp-desktop
- Improved architecture with better separation of concerns

## [0.2.0] - 2024-11-22

### Added
- **GUI Application** with egui framework
- File dialog for opening audio files
- Playback controls (Play/Pause/Stop)
- Progress bar with seek functionality
- Metadata display (title, artist, album, sample rate, channels)
- Multi-threaded architecture (GUI thread + Audio thread)

### Changed
- Split project into core library and desktop application
- Improved error handling and user feedback

## [0.1.0] - 2024-11-22

### Added
- **Initial Release**: Command-line audio player
- MP3 and FLAC playback support
- Metadata extraction and display
- Progress bar during playback
- Verbose mode for detailed information

---

## Roadmap

### Future Versions

**v0.7.0** (Planned):
- Milkdrop visualizations with projectM integration
- Preset selection and management
- Fullscreen visualization mode

**v0.8.0** (Planned):
- Additional audio formats (AAC, OPUS)
- Playlist search and filtering
- Keyboard shortcuts customization

**v0.9.0** (Planned):
- Themes and skins support
- Plugin system
- Audio effects (reverb, compression)

**v1.0.0** (Planned):
- Stable API
- Complete documentation
- Production-ready release
