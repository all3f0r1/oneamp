# OneAmp Changelog

All notable changes to OneAmp are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.14.0] - 2025-12-03

### Added

#### Phase 1: Skin System
- **Skin System:** Complete TOML-based skinning framework for customizing OneAmp appearance
- **Default Skins:** OneAmp Dark (modern) and Winamp5 Classified (classic) included
- **Skin Manager:** Dynamic skin discovery, loading, and application
- **Skin UI:** Menu selector, button selector, info panel, and full selection dialog
- **Winamp5 Conversion:** Faithful conversion of Winamp5 Classified skin to OneAmp format

#### Phase 2: Plugin System (Partial)
- **Plugin Architecture:** Extensible plugin system supporting Input, Output, and DSP plugins
- **Plugin Registry:** Central management for plugin discovery and lifecycle
- **Plugin Traits:** Well-defined interfaces for all plugin types
- **Error Handling:** Standardized error types and result handling
- **Example Plugins:** AAC input plugin and Reverb DSP plugin templates
- **Plugin Loader:** Framework for dynamic plugin loading from shared libraries

### Changed
- **Architecture:** Modularized plugin system in oneamp-core
- **Documentation:** Comprehensive guides for skin and plugin systems

### Technical Details
- Added `oneamp-core/src/plugins/` module with 5 submodules
- Added `oneamp-desktop/src/skins/` module with 4 submodules
- Created `oneamp-plugins/` directory for example plugins
- Implemented 50+ unit tests for new systems

---

## [0.13.1] - 2024-12-02

### Added
- OneDrop visualizer integration (Milkdrop-compatible)
- Fullscreen visualizer mode
- Visualizer preset loading

### Fixed
- Visualizer texture management
- Audio synchronization with visualizations

---

## [0.13.0] - 2024-11-30

### Added
- **OneDrop Visualizer:** Advanced audio visualization system
- **Preset System:** Load and manage visualizer presets
- **GPU Acceleration:** WGPU-based rendering for high-performance visuals

### Changed
- **Visualizer:** Replaced simple FFT with advanced Milkdrop-compatible system
- **Performance:** Optimized GPU memory usage

---

## [0.12.1] - 2024-11-28

### Fixed
- Equalizer parameter persistence
- Audio device detection on Linux

---

## [0.12.0] - 2024-11-27

### Added
- **10-Band Equalizer:** Professional audio adjustment with visual feedback
- **Equalizer Presets:** Rock, Pop, Jazz, Classical, and Custom presets
- **Real-time Processing:** DSP effects applied in real-time during playback

### Changed
- **Audio Pipeline:** Integrated equalizer into main audio processing
- **UI:** Added equalizer display with slider controls

---

## [0.11.0] - 2024-11-25

### Added
- **Playlist Management:** Save and load playlists
- **Drag-and-Drop:** Add files to playlist by dragging
- **Playlist Sorting:** Sort by title, artist, date, or custom order
- **Shuffle and Repeat:** Playback modes for variety

### Changed
- **Playlist UI:** Improved layout and responsiveness
- **File Handling:** Better error handling for missing files

---

## [0.10.1] - 2024-11-24

### Fixed
- Memory leak in audio thread
- Incorrect sample rate detection
- Visualizer performance issues

---

## [0.10.0] - 2024-11-23

### Added
- **Symphonia Integration:** Support for multiple audio formats
- **Format Support:** MP3, FLAC, OGG, OPUS, WAV, AIFF
- **Metadata Extraction:** ID3 tags and audio properties
- **Seeking:** Seek to any position in the track

### Changed
- **Audio Engine:** Replaced rodio decoder with Symphonia
- **Performance:** Improved decoding efficiency

---

## [0.9.0] - 2024-11-22

### Added
- **Audio Capture:** Real-time audio capture for visualization
- **Spectrum Analyzer:** FFT-based frequency analysis
- **Oscilloscope:** Real-time waveform display

### Changed
- **Visualizer:** Complete rewrite with better performance
- **FFT:** Optimized FFT implementation (O(n log n))

---

## [0.8.0] - 2024-11-21

### Added
- **Keyboard Shortcuts:** Winamp-style controls (X, C, V, B, Z)
- **Hotkeys:** Global media key support
- **Command Line:** CLI interface for batch operations

### Changed
- **Input Handling:** Improved keyboard event processing
- **Accessibility:** Better keyboard navigation

---

## [0.7.0] - 2024-11-20

### Added
- **Winamp Modern UI:** Complete visual redesign inspired by Winamp Modern
- **Vertical Layout:** Player, Equalizer, and Playlist sections
- **Interactive Progress Bar:** Click and drag to seek
- **ID3 Tag Support:** Read and display file metadata
- **Drag-and-Drop:** Add files by dragging to window
- **Theme System:** TOML-based customizable themes

### Changed
- **Architecture:** Modularized code into logical components
- **UI:** Redesigned from scratch with better layout
- **Main.rs:** Reduced from 990 to 490 lines

### Technical Details
- Added `toml` and `lofty` dependencies
- Implemented theme configuration system
- Better code organization and modularity

---

## [0.6.1] - 2024-11-19

### Changed
- **Performance:** Optimized release build with LTO (32% size reduction)
- **Performance:** Replaced O(n²) FFT with O(n log n) rustfft (10x faster)
- **Performance:** Throttled position updates (90% fewer allocations)
- **Quality:** Removed unused dependencies
- **Quality:** Fixed all Clippy warnings

### Added
- **Testing:** 29 new tests (3 → 32 total, ~40% code coverage)

### Technical Details
- Optimized release profile (opt-level=3, lto=thin, strip=true)
- Improved FFT with proper normalization
- Comprehensive unit tests

---

## [0.6.0] - 2024-11-18

### Added
- **Keyboard Shortcuts:** Winamp-style (X/C/V/B/Z)
- **Audio Formats:** OGG Vorbis and WAV support
- **Visualizations:** Real-time oscilloscope and spectrum analyzer
- **Playlist Sorting:** Multiple sort options
- **New Logo:** Modern "1AMP" design
- **Welcome Jingle:** First-launch greeting

### Changed
- **Visualizer:** Improved with 64 frequency bands and gradients
- **Playlist:** Enhanced management and sorting

---

## [0.5.0] - 2024-11-17

### Added
- **Dark Theme:** Modern professional styling
- **Application Icon:** Multiple sizes for different contexts
- **Debian Package:** Easy installation (.deb)
- **Desktop Integration:** .desktop file for menu integration
- **Volume Control:** Slider for audio volume adjustment
- **Persistent Config:** Save and restore user settings

### Changed
- **UI:** Complete redesign with modern aesthetics
- **Installation:** Simplified package distribution

---

## [0.4.0] - 2024-11-16

### Added
- **Playlist Support:** Load and manage multiple tracks
- **Track Navigation:** Next/Previous controls
- **Playback Status:** Display current track and position
- **File Browser:** Simple file selection dialog

### Changed
- **UI Layout:** Added playlist panel
- **Controls:** More intuitive playback controls

---

## [0.3.0] - 2024-11-15

### Added
- **Equalizer:** 10-band parametric equalizer
- **Visual Feedback:** Real-time spectrum display
- **Preset Management:** Save and load EQ presets

### Changed
- **Audio Processing:** Added DSP pipeline
- **Performance:** Optimized audio processing

---

## [0.2.0] - 2024-11-14

### Added
- **MP3 Support:** Basic MP3 playback
- **FLAC Support:** Lossless audio format
- **Playback Controls:** Play, pause, stop buttons
- **Progress Bar:** Visual track progress indicator

### Changed
- **Audio Engine:** Improved audio handling
- **UI:** Added basic controls

---

## [0.1.0] - 2024-11-13

### Added
- **Initial Release:** Basic audio player functionality
- **Core Engine:** Audio playback using rodio
- **Simple UI:** egui-based interface
- **File Loading:** Basic file open dialog
- **Playback Controls:** Play and pause buttons

---

## Version History Summary

| Version | Date | Focus |
|---------|------|-------|
| 0.14.0 | 2025-12-03 | Skin System & Plugin Architecture |
| 0.13.1 | 2024-12-02 | OneDrop Visualizer |
| 0.13.0 | 2024-11-30 | Advanced Visualizations |
| 0.12.1 | 2024-11-28 | Bug Fixes |
| 0.12.0 | 2024-11-27 | Equalizer System |
| 0.11.0 | 2024-11-25 | Playlist Management |
| 0.10.1 | 2024-11-24 | Bug Fixes |
| 0.10.0 | 2024-11-23 | Symphonia Integration |
| 0.9.0 | 2024-11-22 | Audio Capture & FFT |
| 0.8.0 | 2024-11-21 | Keyboard Shortcuts |
| 0.7.0 | 2024-11-20 | Winamp Modern UI |
| 0.6.1 | 2024-11-19 | Performance Optimization |
| 0.6.0 | 2024-11-18 | Visualizations |
| 0.5.0 | 2024-11-17 | Dark Theme |
| 0.4.0 | 2024-11-16 | Playlist Support |
| 0.3.0 | 2024-11-15 | Equalizer |
| 0.2.0 | 2024-11-14 | Format Support |
| 0.1.0 | 2024-11-13 | Initial Release |

