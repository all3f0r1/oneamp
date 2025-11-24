# OneAmp v0.7.0 - Changelog

## 🎨 Major UI Overhaul - Winamp Modern Inspired

Version 0.7.0 brings a complete visual redesign inspired by Winamp Modern, with a focus on usability and aesthetics.

## ✨ New Features

### 1. **Winamp-Style Vertical Layout**
- **Player Section** (top): Large digital timer, scrolling track info, spectrum visualizer
- **Equalizer Section** (middle): Collapsible 10-band equalizer with visual feedback
- **Playlist Section** (bottom): Clean track list with drag-and-drop support

### 2. **Interactive Progress Bar**
- Click anywhere on the progress bar to seek to that position
- Drag to scrub through the track
- No more percentage display or spinning indicator - clean and minimal

### 3. **Full ID3 Tag Support**
- Reads artist, title, and album from audio files
- Displays in "ARTIST - TITLE" format in playlist
- Falls back to filename if tags are missing
- Technical info display (sample rate, channels)

### 4. **Drag-and-Drop Support**
- Drag audio files directly onto the window
- Automatically adds to playlist
- Supports MP3, FLAC, OGG, and WAV formats

### 5. **Data-Driven Theme System**
- Themes are now fully customizable via TOML files
- Two built-in themes: "Winamp Modern" (default) and "Dark"
- Easy to create custom themes by editing `theme.toml`
- Supports custom colors, fonts, and layout dimensions

### 6. **Improved Track Display**
- Scrolling text animation for long track titles
- Color-coded playlist items (playing track highlighted)
- Better visual hierarchy

## 🔧 Technical Improvements

- **Modular Architecture**: Code split into logical modules (theme, ui_components, track_display)
- **Reduced main.rs**: From 990 lines to 490 lines through better organization
- **Better Performance**: Optimized rendering and event handling
- **Cleaner Dependencies**: Added `toml` for theme config, `lofty` for ID3 tags

## 📦 New Dependencies

- `toml = "0.8"` - Theme configuration
- `lofty = "0.21"` - ID3 tag reading (via symphonia in core)

## 🎹 Keyboard Shortcuts

- `Space` - Play/Pause
- `Ctrl+O` - Open file

## 🎨 Theme Customization

To create a custom theme:

1. Copy `theme.toml.example` to `theme.toml`
2. Edit colors, fonts, and layout settings
3. Restart OneAmp to apply changes

Example theme structure:
```toml
[colors]
display_text = [100, 180, 255]  # RGB values
progress_fill = [100, 180, 255]

[fonts]
timer_size = 32.0
playlist_size = 13.0

[layout]
player_height = 150.0
spacing = 8.0
```

## 🐛 Bug Fixes

- Fixed icon size issue (v0.6.2 fix carried forward)
- Improved error handling for missing audio files
- Better playlist state management

## 📝 Breaking Changes

- UI layout completely redesigned (no longer side-by-side panels)
- Theme system replaces hard-coded colors
- Some keyboard shortcuts changed

## 🔮 Future Plans

- Volume control slider
- Playlist save/load
- More built-in themes
- Plugin system for visualizers
- Mini-mode (compact player)

## 🙏 Credits

- Inspired by [Winamp Modern](https://github.com/0x5066/WinampModernForked)
- Built with [egui](https://github.com/emilk/egui) and [eframe](https://github.com/emilk/egui/tree/master/crates/eframe)
- Audio engine powered by [rodio](https://github.com/RustAudio/rodio) and [symphonia](https://github.com/pdeljanov/Symphonia)
