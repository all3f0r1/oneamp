# Changelog

All notable changes to OneAmp are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.3.0] — 2026-09-26 — The audio quality release

Audiophile audio path: bit-perfect by default, native sample rate, no
hidden processing.

### Changed
- Output goes straight to cpal 0.18 (rodio removed). The stream opens at
  the track's native sample rate in the most precise format the device
  offers; rodio resampled everything to the device's default rate with
  linear interpolation. A band-limited FFT resampler (rubato) is used
  only when the device refuses the native rate.
- Bit-perfect at neutral settings: balance is unity at centre (the
  constant-power law cost −3 dB on both channels), the limiter ceiling is
  0 dBFS with 2 ms lookahead (it was always shaving masters peaking above
  −1 dBFS, with a zero-attack gain step), the flat EQ and idle loudness
  shelves are skipped, and TPDF dither is only added when the samples are
  actually requantized — matched to the device's bit depth.
- Volume ≤ 100 % acts in the output callback (instant, smoothed); the
  boost above 100 % goes before the limiter, so it no longer clips.
- Symphonia 0.6: gapless trimming of MP3/AAC encoder delay and padding,
  SIMD, hardened demuxers, MP3 silent-frame fix; codec names in the info
  readout (FLAC, MP3, …).
- EQ biquads run in f64.
- The audio callback thread gets real-time priority on Linux (rtkit).
- Dependencies: lofty 0.25, souvlaki 0.8, dirs 7, indicatif 0.18.

### Added
- AIFF, CAF and Matroska audio (.mka) playback.

### Fixed
- Crossfade dropped part of the incoming track whenever the two files
  used different packet sizes (e.g. MP3 into FLAC), and both decoders
  shared one set of EQ filter states.
- Conversion to 16/24-bit devices rounds instead of truncating toward
  zero, and full scale no longer wraps on 24-bit output.

## [1.2.0] — 2026-09-26

Playlist editor polish: Winamp's popup menus, time fields and sizing, with
every menu entry wired.

### Added
- Playlist menus: ADD URL, ADD DIR, REM CROP (keep only the selection) and
  REM MISC (remove files that no longer exist) now work. Undo with
  `Ctrl+Z`.
- Master volume goes up to 150 % (default stays 100 %, two thirds of the
  slider). Above 100 % loud tracks can clip.

### Changed
- Playlist popup menus behave like Winamp's: the bottom row covers the
  button, the hovered row lights up, and press-drag-release picks an entry
  in one gesture (a plain click still opens the menu).
- The playlist's upper time field shows the selection / playlist length
  (`+` when some lengths are unknown); the small lower field shows the
  current track's elapsed time, blank when stopped.
- The playlist resizes in 29-px steps, like Winamp.
- Equalizer range is ±12 dB (was ±20 dB), matching the skin's labels.
  `.eqf` files map to the same range, so imported presets are gentler than
  before; saved gains beyond ±12 dB are clamped.
- The playlist's mini play button starts a stopped track, pause toggles,
  and eject replaces the playlist like the main window's.

### Fixed
- ADD/REM/SEL/MISC/LIST buttons stopped responding after the playlist was
  resized.
- The pressed scrollbar thumb showed a cyan column; LIST OPTS was one
  pixel off.
- The pressed Eject sprite was read two pixels too low.
- Clicking a popup entry no longer also selects the playlist row beneath.

## [1.1.0] — 2026-09-24

Winamp Classic habits: arrange your windows, get your session back and
drive everything from the keyboard.

### Added
- **Session restore**: the playlist, its order, the current track, the
  play-next queue and the playhead come back at launch. Playback never
  starts on its own; the saved position is applied when you press Play.
  Files that can't be found stay in the list, dimmed, and are counted in
  a toast. Window layout, playlist height and shade mode are restored.
- **Detached windows** (View › Detached windows, or Preferences):
  equalizer and playlist in their own windows, moved by their title
  bars. Windows docked to the player follow it; a moved window snaps to
  nearby edges. X11, Windows and macOS (Wayland can't place windows).
- **Winamp Classic keyboard profile**, on by default, from one shortcut
  table shared with the menus: `Z X C V B` everywhere, `S` shuffle, `R`
  repeat, `J` / `F3` Jump to file, `Ctrl+P` preferences, `Alt+E` /
  `Alt+G` panels, `Ctrl+W` shade, `Ctrl+A` always on top, `Ctrl+T`
  elapsed/remaining. Focus-aware: in the playlist, arrows / Home / End /
  Page keys select (Shift extends), `Alt+↑/↓` move the selection, `Enter`
  plays, `Del` removes, `Q` queues, `Ctrl+A` selects all; in the
  equalizer, `1`–`0` / `Q`–`P` raise and lower bands, `` ` `` / `Tab`
  the preamp, `N` toggles the EQ, `A` AUTO. No shortcut fires while you
  type. The OneAmp 1.0 layout stays available (Preferences › Shortcuts).
  `F1` lists the active shortcuts.
- **Jump to file** box: type words, pick with the arrows, `Enter` plays,
  `Shift+Enter` queues. The playlist view is left untouched.
- **Undo** (`Ctrl+Z`) for playlist remove, clear, sort, reorder and
  replace, 20 steps.
- **EQ AUTO**: per-track auto-load presets (PRESETS › Auto-load for this
  track / Remove track auto-load). An auto-loaded preset never replaces
  your saved curve; editing the EQ by hand makes the edited curve the
  saved one. Double-click a slider to reset it; hovering a slider shows
  its exact frequency and gain.
- **Preferences** window (`Ctrl+P`): General, Playback, Playlist,
  Equalizer (rename / delete your presets), Appearance, Shortcuts.
- **Playlists**: M3U8 and PLS load and save alongside M3U, from the file
  picker, drag-drop and the file manager (MIME types registered).
- Large folders are added in the background with progress in the toast;
  `Escape` cancels.

### Changed
- Open file (`L`, `Ctrl+O`, Eject) replaces the playlist and plays, like
  Winamp's Play file; Add files still appends. `Ctrl+Z` restores the
  previous list.
- In the Winamp Classic profile, `S` toggles shuffle (it was Stop) and
  `J` opens Jump to file instead of the inline filter (still on
  `Ctrl+F`).

### Fixed
- Opening a dialog (URL, tags, presets, welcome) on a HiDPI screen shrank
  the player until restart: egui's zoom is global to every window. The
  player is now magnified by its own render scale and egui's zoom stays
  at 1.
- Playing a missing file opened a blocking error dialog; it's now a
  toast.

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
