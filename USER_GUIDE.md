# OneAmp User Guide

## Table of Contents

1. [Introduction](#introduction)
2. [Installation](#installation)
3. [Getting Started](#getting-started)
4. [Interface Overview](#interface-overview)
5. [Playlist Management](#playlist-management)
6. [Equalizer](#equalizer)
7. [Keyboard Shortcuts](#keyboard-shortcuts)
8. [Configuration](#configuration)
9. [Troubleshooting](#troubleshooting)

## Introduction

OneAmp is a modern audio player for Linux inspired by the classic Winamp. It provides a clean, efficient interface for playing your music collection with support for MP3 and FLAC formats.

### Features

- **Audio Playback**: High-quality MP3 and FLAC playback
- **Playlist Management**: Create and manage playlists with ease
- **10-Band Equalizer**: Professional-grade audio equalization
- **Modern Interface**: Clean, dark-themed UI with smooth animations
- **Lightweight**: Minimal resource usage
- **Native Linux**: Built specifically for Linux desktop environments

## Installation

### From Debian Package (.deb)

The easiest way to install OneAmp on Debian-based systems (Debian, Ubuntu, Linux Mint, etc.):

```bash
sudo dpkg -i oneamp_0.5.0_amd64.deb
```

If you encounter dependency issues:

```bash
sudo apt-get install -f
```

### From Source

Requirements:
- Rust 1.70 or later
- ALSA development files: `sudo apt-get install libasound2-dev`

```bash
git clone https://github.com/all3f0r1/oneamp.git
cd oneamp
cargo build --release -p oneamp-desktop
sudo cp target/release/oneamp /usr/local/bin/
```

### Uninstallation

```bash
sudo dpkg -r oneamp
```

## Getting Started

### Launching OneAmp

After installation, you can launch OneAmp in several ways:

1. **From Application Menu**: Look for "OneAmp" in your application launcher
2. **From Terminal**: Type `oneamp`
3. **From File Manager**: Right-click an audio file → Open With → OneAmp

### Playing Your First Track

1. Click the **"📁 Open File"** button in the top-left
2. Navigate to an audio file (MP3 or FLAC)
3. Select the file and click "Open"
4. The track will start playing automatically

## Interface Overview

### Top Panel

- **Application Title**: "🎧 OneAmp" with version number
- **Equalizer Button**: Opens the equalizer window

### Playlist Panel (Left Side)

- **Track List**: Shows all tracks in the current playlist
- **Current Track**: Highlighted in cyan
- **Selected Track**: Shows selection background
- **Double-click**: Play a track immediately

### Playlist Controls

- **➕ Add Files**: Add one or more audio files to the playlist
- **📁 Add Folder**: Recursively scan a folder for audio files
- **➖ Remove**: Remove the selected track from the playlist
- **🗑 Clear All**: Clear the entire playlist

### Main Display (Center)

Shows information about the currently playing track:

- **Track Title**: Extracted from file metadata
- **Artist**: Artist name (if available)
- **Album**: Album name (if available)
- **Technical Info**: Sample rate, channels, and format

### Control Panel (Bottom)

- **Progress Bar**: Shows playback progress
- **Time Display**: Current position / Total duration
- **Playback Controls**:
  - **⏮ Previous**: Play previous track in playlist
  - **▶️ Play / ⏸ Pause**: Start or pause playback
  - **⏹ Stop**: Stop playback completely
  - **⏭ Next**: Play next track in playlist

## Playlist Management

### Adding Tracks

**Add Files**:
1. Click **"➕ Add Files"**
2. Select one or more audio files (Ctrl+Click for multiple)
3. Click "Open"

**Add Folder**:
1. Click **"📁 Add Folder"**
2. Select a folder containing audio files
3. OneAmp will recursively scan for all MP3 and FLAC files

### Navigating Tracks

- **Double-click** a track to play it immediately
- Use **⏭ Next** and **⏮ Previous** buttons to navigate
- When a track finishes, the next track plays automatically

### Removing Tracks

- **Single Track**: Select a track and click **"➖ Remove"**
- **Clear All**: Click **"🗑 Clear All"** to empty the playlist

### Playlist Tips

- Tracks are played in the order they appear in the list
- The currently playing track is highlighted in cyan
- You can add tracks while music is playing
- Removing the currently playing track stops playback

## Equalizer

OneAmp features a professional 10-band graphic equalizer for precise audio control.

### Opening the Equalizer

Click the **"🎵 Equalizer"** button in the top panel.

### Equalizer Bands

The equalizer has 10 frequency bands:

| Band | Frequency | Description |
|------|-----------|-------------|
| 1 | 31 Hz | Sub-bass |
| 2 | 62 Hz | Bass |
| 3 | 125 Hz | Low bass |
| 4 | 250 Hz | Low midrange |
| 5 | 500 Hz | Midrange |
| 6 | 1 kHz | Upper midrange |
| 7 | 2 kHz | Presence |
| 8 | 4 kHz | Brilliance |
| 9 | 8 kHz | High treble |
| 10 | 16 kHz | Air |

### Using the Equalizer

1. **Enable/Disable**: Check or uncheck the "Enable" checkbox
2. **Adjust Bands**: Drag sliders up (boost) or down (cut)
   - Range: -12 dB to +12 dB
   - Step: 0.5 dB
3. **Reset**: Click **"🔄 Reset All"** to return all bands to 0 dB

### Equalizer Presets (Suggestions)

**Rock**:
- Boost: 60 Hz (+4 dB), 250 Hz (+2 dB), 4 kHz (+3 dB)
- Cut: 1 kHz (-2 dB)

**Classical**:
- Boost: 125 Hz (+2 dB), 8 kHz (+2 dB), 16 kHz (+3 dB)
- Cut: 500 Hz (-1 dB)

**Electronic**:
- Boost: 31 Hz (+6 dB), 62 Hz (+4 dB), 8 kHz (+2 dB)

**Vocal**:
- Boost: 1 kHz (+3 dB), 2 kHz (+4 dB), 4 kHz (+2 dB)
- Cut: 125 Hz (-2 dB)

### Equalizer Tips

- Start with small adjustments (±2-3 dB)
- Excessive boosting can cause distortion
- Your settings are automatically saved
- The equalizer works in real-time

## Keyboard Shortcuts

Currently, OneAmp uses mouse-based controls. Keyboard shortcuts may be added in future versions.

## Configuration

### Configuration File

OneAmp stores its configuration in:
```
~/.config/oneamp/config.json
```

### Saved Settings

- Equalizer enabled state
- Equalizer band gains (all 10 bands)
- Window size and position (handled by egui)

### Manual Configuration

You can edit the configuration file manually:

```json
{
  "equalizer_enabled": true,
  "equalizer_gains": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
}
```

## Troubleshooting

### No Sound

1. **Check System Volume**: Ensure your system volume is not muted
2. **Check ALSA**: Run `aplay -l` to verify audio devices
3. **Try Another File**: Some files may be corrupted

### File Won't Play

1. **Check Format**: OneAmp supports MP3 and FLAC only
2. **Check File Integrity**: Try playing the file with another player
3. **Check Permissions**: Ensure you have read access to the file

### Application Won't Start

1. **Check Dependencies**: `ldd $(which oneamp)`
2. **Reinstall**: `sudo dpkg -i --force-overwrite oneamp_0.5.0_amd64.deb`
3. **Check Logs**: Run from terminal to see error messages

### Equalizer Not Working

1. **Enable Equalizer**: Make sure the "Enable" checkbox is checked
2. **Restart Playback**: Stop and play the track again
3. **Reset Settings**: Click "Reset All" and try again

### Performance Issues

1. **Close Equalizer Window**: The equalizer window can be closed when not in use
2. **Reduce Playlist Size**: Very large playlists (1000+ tracks) may slow down the UI
3. **Check System Resources**: Run `top` to check CPU/memory usage

## Getting Help

- **GitHub Issues**: https://github.com/all3f0r1/oneamp/issues
- **Documentation**: https://github.com/all3f0r1/oneamp

## Credits

OneAmp is built with:
- **Rust**: Programming language
- **egui**: Immediate mode GUI framework
- **rodio**: Audio playback library
- **Symphonia**: Audio decoding library

Inspired by the legendary Winamp audio player.

## License

OneAmp is released under the MIT License. See LICENSE file for details.
