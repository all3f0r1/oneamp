# Changelog

All notable changes to OneAmp are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.3] — 2026-09-24

### Fixed
- Settings autosave never fired while the app was running: the per-frame
  drift check re-armed the 300 ms debounce every frame, so EQ bands,
  preamp, EQ on/off, volume and other preferences only reached disk on a
  clean exit. The debounce now starts on the first divergence only.
- A failed config write was treated as done, so nothing retried it. The
  pending state is now kept, retried every 5 s, and a toast reports it.
- Launching OneAmp without files while it was already running opened a
  second instance, which could later overwrite the first one's settings
  with stale values. A bare relaunch now raises the existing window.
- "Stop after current track" was bypassed by gapless transitions,
  crossfades and Repeat One/All. The engine now honours it at end of
  stream and drops any preloaded next track.
- Long-file resume positions were only written on exit; a crash lost the
  whole session's progress. They are now written on each 15 s tick.
- EQ sliders: clicking a thumb changed its value (input used the 63 px
  track instead of the 52 px thumb travel). Input and painting now share
  one conversion and the grab point is kept during the drag.
- EQ fills: positive and negative gains of the same magnitude shared a
  texture-cache key, so a band could show the wrong-sign fill colour.

### Changed
- Dependencies: `crossbeam-epoch`, `rustls`, `ringbuf`, `webbrowser`,
  `quick-xml` (wayland), `notify-rust` and others bumped for RustSec
  advisories; remaining unfixable transitive advisories triaged in
  `.cargo/audit.toml`.
- Code adjusted for Rust 1.98 Clippy lints (`as_chunks`, float literal
  fallback).

## [1.0.2] — 2026-07-15

### Fixed
- Playlist sort: the UI's "Sort by title" action (`sort_by_title`) went
  through a second sort implementation that cleared the selection and the
  shift-click anchor. It now delegates to the canonical `sort_by`, so a
  sort preserves every index-backed pointer — currently-playing track,
  play queue, history, multi-selection and anchor — by track identity.
- Config: lenient (partial) config loading silently reset `show_remaining`
  and `resume_long_files` to their defaults whenever another field forced
  the recovery path; both settings are now preserved.
- Playlist and equalizer-preset edge cases hardened (see #1).

### Changed
- `AudioEvent::ShuffleUpdated` is now handled by the app state reducer,
  keeping the UI shuffle indicator in sync with the audio engine.
- Config file IO split into path-explicit `load_from`/`save_to`; the
  save/load round-trip test runs against a temp dir instead of the real
  user config.

## [1.0.0] — 2026-06-16

First public release. A cross-platform, Winamp-faithful audio player written
in Rust — native on Linux, macOS, and Windows.

### Audio engine
- Symphonia decoding: MP3, FLAC, OGG/Vorbis, WAV, AAC, M4A/MP4, ALAC.
- rodio output over a lock-free SPSC ring buffer — no mutex on the cpal
  real-time callback, no priority inversion under compositor load.
- Gapless transitions between same-format tracks via a preloaded decoder
  swapped into the live stream (no device rebuild, no audible silence).
- Pre-buffer brickwall limiter (−1 dBFS ceiling, zero-attack clamp, 100 ms
  one-pole release) at the end of the per-sample chain.
- Stream position read from each packet's PTS, not a frame-count accumulator,
  so skipped frames don't drift the position slider.

### Equalizer & DSP
- 10-band equalizer: RBJ biquads at ISO 266 / IEC 61260 octave centres,
  ±20 dB, zero-alloc on the decode path, constant-Q gain-dependent bandwidth,
  ~10 ms coefficient ramping (click-free).
- 16 built-in presets with auto-computed per-preset preamp (headroom derived
  from the cascaded transfer function) so the limiter isn't pumped.
- Opt-in loudness compensation (Fletcher-Munson / ISO 226) via two RBJ shelves.
- Constant-power sin/cos stereo balance (no centre loudness hump).

### Playback & sources
- Internet radio + podcast streaming over a custom Symphonia `MediaSource`
  (ureq 3 + rustls + webpki-roots, no system OpenSSL), with inline ICY
  metadata parsing and wait-free "now playing" publication.
- Playlist: drag-drop files and folders, M3U save/load, native-skinned frame.
- In-place tag editor (lofty 0.24): ID3v1/v2, Vorbis Comments, MP4 atoms,
  RIFF, APE — preserving cover art, ReplayGain, and MusicBrainz IDs.
- Customisable playlist row format with collapsing separators for missing tags.

### Interface
- Native `.wsz` skin support with a bundled default embedded in the binary.
- Custom window chrome (no OS title bar), pixel-perfect drag and window shade.
- Visualizer: spectrum analyzer (Hann-windowed FFT), oscilloscope (zero-crossing
  triggered), and peak/RMS meter — all in the skin's authentic viscolor palette.
- Authentic Winamp hotkeys.

### Platform integration
- Single-instance handoff (Unix socket / named pipe), media keys (MPRIS2 /
  MediaRemote / SMTC), and track notifications behind a per-OS abstraction layer
  (`interprocess`, `souvlaki`, `notify-rust`).
- One-shot startup update check against the GitHub Releases API.

### Distribution
- Linux: `.deb`, `.rpm`, Flatpak, Snap, tarball, and an AUR `PKGBUILD`.
- Windows: per-machine MSI (WiX 3) + portable ZIP.
- macOS: universal DMG (Intel + Apple Silicon).

[1.0.0]: https://github.com/all3f0r1/oneamp/releases/tag/v1.0.0
