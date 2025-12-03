# Changelog

All notable changes to OneAmp will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.15.0] - 2025-12-03

### Added
- Audio Format Detection: Improved metadata extraction to display codec, bitrate, sample rate, and channels
- TrackInfo Enhancements: Added codec and bitrate fields to TrackInfo struct
- Format Audio Info: New format_audio_info() method for human-readable audio information
- Technical Info Display: Enhanced technical info display (e.g., MP3 • 320kbps • 44.1kHz • Stereo)

### Changed
- Audio Metadata: Now extracts codec information from symphonia codec parameters
- Display Format: Technical info now shows complete audio details instead of just sample rate and channels
- TrackDisplay: Updated to use new format_audio_info() method

### Technical
- Added codec and bitrate extraction in TrackInfo::from_file()
- Implemented format_audio_info() with proper formatting and channel name mapping
- Updated tests to include new codec and bitrate fields
- Backward compatible with existing code

## [0.14.8] - 2025-12-03

### Fixed
- **Borrow Checker Error E0500**: Fixed closure requiring unique access to `show_skin_selector`
- **Window Parameter Conflict**: Removed `show_dialog` parameter from skin_selector_dialog
- **Closure Borrow Issue**: Window.open() and closure no longer conflict over show_skin_selector

### Changed
- **Skin Selector Dialog**: Simplified to work directly within Window context
- **Function Signature**: skin_selector_dialog now takes only (ui, skin_manager)
- **Window Management**: Window.open() handles show_skin_selector state in main.rs

### Technical
- Window wrapper in main.rs manages the open/close state
- Dialog function focuses on rendering content only
- Cleaner separation of concerns between window state and content

## [0.14.7] - 2025-12-03

### Fixed
- **Unused Imports**: Removed unused imports (Colors, Fonts, Metadata, Metrics) from parser.rs
- **Type Mismatch**: Fixed skin selector dialog call in main.rs (ctx vs ui parameter)
- **Borrow Checker**: Fixed immutable/mutable borrow conflicts in skin selector UI
- **Deprecated API**: Replaced deprecated allocate_ui_at_rect with allocate_new_ui
- **Compilation Errors**: All 3 compilation errors resolved
- **Warnings**: Reduced warnings from 3 to 0

### Changed
- **Skin Selector**: Refactored to collect skin data before UI rendering
- **UI Components**: Updated to use current egui API (allocate_new_ui)
- **Code Quality**: Improved borrow checker compliance

### Technical
- Skin data now cloned before menu/window rendering to avoid borrow conflicts
- Window wrapper added for skin selector dialog in main.rs
- UiBuilder used for proper rect allocation

## [0.14.6] - 2025-12-03

### Added
- **Audio Feature Flag**: New optional "audio" feature in Cargo that enables ALSA/rodio support
- **Minimal Builds**: Support for building without audio dependencies via --no-default-features
- **Feature Documentation**: Metadata for docs.rs to build with all features

### Fixed
- **ALSA Compilation**: Permanently resolved ALSA compilation issues by making audio optional
- **Documentation Builds**: Doc generation now works without ALSA/rodio dependencies
- **CI/CD Reliability**: Separated audio feature from core compilation

### Changed
- **Cargo Dependencies**: rodio and cpal now optional, controlled by "audio" feature
- **Default Behavior**: "audio" feature enabled by default for normal builds
- **Doc Tests**: Now run with --no-default-features to avoid ALSA issues
- **Workflows**: Updated CI and docs workflows to use feature flags appropriately

### Architecture
- **Feature Gates**: Added #[cfg(feature = "audio")] to audio-related modules
- **Conditional Compilation**: cpal_output and rodio_output only compiled with audio feature
- **Flexible Building**: Users can now compile OneAmp without audio system dependencies

## [0.14.5] - 2025-12-03

### Added
- **Documentation Generation**: New GitHub Actions workflow to generate Rust documentation with cargo doc
- **GitHub Pages Publishing**: Automatic deployment of generated documentation to GitHub Pages
- **Documentation Index**: Custom HTML index page for easy navigation between modules
- **Docs Workflow**: Triggers on master branch pushes and version tags

### Fixed
- **ALSA in Doc Tests**: Disabled default features for doc tests to avoid ALSA compilation issues
- **Documentation Build**: Separated doc generation from regular builds for better reliability

### Changed
- **CI Workflow**: Modified doc tests to use --no-default-features flag
- **Documentation Deployment**: Added peaceiris/actions-gh-pages action for automatic publishing

## [0.14.4] - 2025-12-03

### Fixed
- **ALSA Detection**: Fixed persistent pkg-config issues by explicitly setting PKG_CONFIG_PATH
- **CI/CD Pipeline**: Added build-essential to system dependencies for proper compilation
- **Environment Variables**: Configured PKG_CONFIG_PATH in both CI and release workflows
- **Workflow Configuration**: Applied ALSA fixes to both ci.yml and release.yml

### Changed
- **Build Dependencies**: Added build-essential alongside pkg-config and libasound2-dev
- **Environment Setup**: Explicitly export PKG_CONFIG_PATH to standard Linux library paths

## [0.14.3] - 2025-12-03

### Fixed
- **Compilation Error**: Fixed type mismatch in symphonia_player.rs (decoder_opts reference)
- **CI/CD Pipeline**: Added pkg-config to system dependencies for proper ALSA detection
- **Unused Imports**: Removed unused PathBuf import in audio_thread_symphonia.rs
- **Type Hints**: Changed PathBuf to Path for better type consistency

### Changed
- **Build Dependencies**: Added pkg-config to GitHub Actions workflow for ALSA support

## [0.14.2] - 2025-12-03

### Fixed
- **CI/CD Pipeline**: Added ALSA system dependencies to GitHub Actions CI workflow
- **Clippy Warnings**: Fixed 8+ clippy warnings (unused imports, dead code, type improvements)
- **Rustfmt**: Applied rustfmt to all source files for consistent formatting
- **Integration Tests**: Fixed borrowed expression issues in CLI tests
- **Code Quality**: Removed unnecessary references and improved type hints

### Changed
- **Build Dependencies**: Added libasound2-dev to CI/CD pipeline for ALSA support
- **Code Style**: All code now passes strict clippy checks (-D warnings)

## [0.14.1] - 2025-12-03

### Added
- **CI/CD Pipeline**: GitHub Actions for automated testing, linting, and releases
- **5 Skins**: OneAmp Dark, Winamp5 Classified, OneAmp Light, Cyberpunk Neon, Retro Terminal
- **Skin Selector**: UI to switch between skins in real-time
- **Skin Persistence**: Saves active skin in config
- **18+ Tests**: Comprehensive test suite for skin system
- **Documentation**: Guides for creating custom skins and API reference

### Changed
- **Version Bump**: All packages updated to v0.14.1
- **Code Quality**: All Clippy warnings fixed
- **Formatting**: All code formatted with rustfmt

### Fixed
- **Compilation Errors**: Fixed 9+ compilation errors related to egui API changes
- **Borrow Checker**: Fixed borrow checker issues in UI code

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
