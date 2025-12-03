# OneAmp Plugin System - Integration Guide

**Author:** Manus AI  
**Date:** December 3, 2025  
**Version:** 1.0

---

## Overview

This guide explains how to integrate the plugin system into OneAmp's audio engine and user interface. The plugin system has been implemented in `oneamp-core` and is ready for integration with the audio playback pipeline.

---

## Step 1: Verify Plugin Module Installation

Ensure that all plugin system files are in place:

```
oneamp-core/src/plugins/
├── mod.rs          (Main module)
├── traits.rs       (Plugin trait definitions)
├── error.rs        (Error handling)
├── registry.rs     (Plugin registry)
└── loader.rs       (Dynamic plugin loading)
```

The `plugins` module has been added to `oneamp-core/src/lib.rs`.

---

## Step 2: Create a Built-in Input Plugin

The plugin system requires at least one input plugin to function. We'll create a wrapper around the existing Symphonia-based decoder.

### Create `oneamp-core/src/plugins/symphonia_input.rs`

```rust
// Symphonia-based input plugin for OneAmp
// Provides support for MP3, FLAC, OGG, WAV, and other formats via Symphonia.

use std::path::Path;
use std::sync::Arc;
use crate::plugins::traits::{InputPlugin, AudioDecoder, AudioMetadata, AudioBuffer};
use crate::plugins::error::PluginResult;

pub struct SymphoniaInputPlugin;

impl SymphoniaInputPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl InputPlugin for SymphoniaInputPlugin {
    fn name(&self) -> &str {
        "Symphonia Input"
    }

    fn version(&self) -> &str {
        "1.0"
    }

    fn supported_formats(&self) -> Vec<&str> {
        vec!["mp3", "flac", "ogg", "opus", "wav", "aiff"]
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                matches!(
                    ext.to_lowercase().as_str(),
                    "mp3" | "flac" | "ogg" | "opus" | "wav" | "aiff"
                )
            })
            .unwrap_or(false)
    }

    fn open(&self, path: &Path) -> PluginResult<Box<dyn AudioDecoder>> {
        // TODO: Implement using existing symphonia_player module
        Err("Not yet implemented".into())
    }
}

impl Default for SymphoniaInputPlugin {
    fn default() -> Self {
        Self::new()
    }
}
```

### Register the Plugin in AudioEngine

Modify `oneamp-core/src/lib.rs` to initialize the plugin registry:

```rust
use plugins::PluginRegistry;
use std::sync::Arc;

pub struct AudioEngine {
    // ... existing fields ...
    plugin_registry: PluginRegistry,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let mut plugin_registry = PluginRegistry::new(
            dirs::config_dir()
                .map(|d| d.join("oneamp").join("plugins"))
                .unwrap_or_else(|| PathBuf::from("./plugins"))
        );

        // Register built-in plugins
        plugin_registry.register_input_plugin(
            Arc::new(plugins::symphonia_input::SymphoniaInputPlugin::new())
        );

        // Discover external plugins
        let _ = plugin_registry.discover_plugins();

        Ok(Self {
            // ... existing fields ...
            plugin_registry,
        })
    }

    pub fn plugin_registry(&self) -> &PluginRegistry {
        &self.plugin_registry
    }
}
```

---

## Step 3: Update File Loading Logic

Modify the `load_file` method to use the plugin system:

```rust
impl AudioEngine {
    pub fn load_file(&mut self, path: &Path) -> Result<TrackInfo, String> {
        // Find a plugin that can handle this file
        let plugin = self.plugin_registry
            .find_input_plugin(path)
            .ok_or_else(|| format!("No plugin found for file: {:?}", path))?;

        // Open the file with the plugin
        let mut decoder = plugin.open(path)
            .map_err(|e| format!("Failed to open file: {}", e))?;

        let metadata = decoder.metadata().clone();

        // Create TrackInfo from metadata
        let track_info = TrackInfo {
            path: path.to_path_buf(),
            title: metadata.title.unwrap_or_else(|| "Unknown".to_string()),
            artist: metadata.artist.unwrap_or_else(|| "Unknown".to_string()),
            album: metadata.album.unwrap_or_else(|| "Unknown".to_string()),
            duration: metadata.duration,
            sample_rate: metadata.sample_rate,
            channels: metadata.channels,
            bitrate: metadata.bitrate,
        };

        // Store decoder for playback
        self.current_decoder = Some(decoder);

        Ok(track_info)
    }
}
```

---

## Step 4: Add DSP Chain to Audio Engine

Add support for DSP plugins in the audio processing pipeline:

```rust
pub struct AudioEngine {
    // ... existing fields ...
    dsp_chain: Vec<Box<dyn plugins::traits::DSPProcessor>>,
}

impl AudioEngine {
    pub fn add_dsp_effect(&mut self, plugin: Arc<dyn plugins::traits::DSPPlugin>) -> Result<(), String> {
        let processor = plugin.create_processor()
            .map_err(|e| format!("Failed to create DSP processor: {}", e))?;
        self.dsp_chain.push(processor);
        Ok(())
    }

    pub fn remove_dsp_effect(&mut self, index: usize) {
        if index < self.dsp_chain.len() {
            self.dsp_chain.remove(index);
        }
    }

    pub fn process_audio(&mut self, buffer: &mut plugins::traits::AudioBuffer) -> Result<(), String> {
        for processor in &mut self.dsp_chain {
            processor.process(buffer)
                .map_err(|e| format!("DSP processing error: {}", e))?;
        }
        Ok(())
    }
}
```

---

## Step 5: Add UI for Plugin Management

In `oneamp-desktop/src/main.rs`, add UI components for managing plugins:

```rust
mod plugins_ui;
use plugins_ui::PluginsPanel;

struct OneAmpApp {
    // ... existing fields ...
    show_plugins_panel: bool,
    plugins_panel: PluginsPanel,
}

impl OneAmpApp {
    fn new(cc: &eframe::CreationContext<'_>, use_custom_chrome: bool) -> Self {
        // ... existing initialization ...

        let mut app = Self {
            // ... existing fields ...
            show_plugins_panel: false,
            plugins_panel: PluginsPanel::new(&self.audio_engine),
        };

        app
    }
}

impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // ... existing code ...

        // Add plugins panel button
        if ui.button("🔌 Plugins") {
            self.show_plugins_panel = !self.show_plugins_panel;
        }

        // Show plugins panel
        if self.show_plugins_panel {
            egui::Window::new("Plugins")
                .open(&mut self.show_plugins_panel)
                .show(ctx, |ui| {
                    self.plugins_panel.ui(ui, &self.audio_engine);
                });
        }
    }
}
```

### Create `oneamp-desktop/src/plugins_ui.rs`

```rust
// Plugin management UI

use egui::{Ui, RichText};
use oneamp_core::plugins::PluginRegistry;

pub struct PluginsPanel;

impl PluginsPanel {
    pub fn new(_engine: &AudioEngine) -> Self {
        Self
    }

    pub fn ui(&mut self, ui: &mut Ui, registry: &PluginRegistry) {
        ui.heading("Installed Plugins");
        ui.separator();

        // Input plugins
        ui.label(RichText::new("Input Plugins").strong());
        for plugin in registry.input_plugins() {
            ui.label(format!("  • {} v{}", plugin.name(), plugin.version()));
            ui.label(format!("    Formats: {}", plugin.supported_formats().join(", ")));
        }

        ui.separator();

        // Output plugins
        ui.label(RichText::new("Output Plugins").strong());
        for plugin in registry.output_plugins() {
            ui.label(format!("  • {} v{}", plugin.name(), plugin.version()));
        }

        ui.separator();

        // DSP plugins
        ui.label(RichText::new("DSP Plugins").strong());
        for plugin in registry.dsp_plugins() {
            ui.label(format!("  • {} v{} ({})", plugin.name(), plugin.version(), plugin.category()));
        }
    }
}
```

---

## Step 6: Create Plugin Directory Structure

Create the plugin directories:

```bash
mkdir -p ~/.config/oneamp/plugins
mkdir -p oneamp-plugins
```

---

## Step 7: Testing

### Test Plugin Registry

```rust
#[test]
fn test_plugin_registry_integration() {
    let mut registry = PluginRegistry::new(PathBuf::from("./plugins"));
    registry.register_input_plugin(
        Arc::new(SymphoniaInputPlugin::new())
    );

    assert_eq!(registry.input_plugin_count(), 1);
    assert!(registry.find_input_plugin(Path::new("test.mp3")).is_some());
}
```

### Test Audio Engine Integration

```rust
#[test]
fn test_audio_engine_with_plugins() {
    let engine = AudioEngine::new().expect("Failed to create engine");
    assert!(engine.plugin_registry().input_plugin_count() > 0);
}
```

---

## Step 8: Documentation

Update the main README with plugin information:

```markdown
## Plugin System

OneAmp supports extending functionality through plugins:

### Input Plugins
- Decode audio files in various formats
- Built-in: Symphonia (MP3, FLAC, OGG, WAV, etc.)

### Output Plugins
- Interface with audio hardware
- Extensible for different platforms

### DSP Plugins
- Apply audio effects and processing
- Examples: Reverb, Compression, EQ

### Installing Plugins

Place plugin files in `~/.config/oneamp/plugins/` and restart OneAmp.
```

---

## Troubleshooting

### Plugins Not Loading

1. Check that plugin directory exists: `~/.config/oneamp/plugins/`
2. Verify plugin files have correct extension (.so, .dll, .dylib)
3. Check console output for error messages
4. Ensure plugin API version matches OneAmp version

### Audio Engine Crashes

1. Plugins run in the same process as OneAmp
2. A crashing plugin will crash the application
3. Check plugin error handling
4. Consider implementing process isolation in future versions

---

## Next Steps

1. Implement Symphonia-based input plugin wrapper
2. Create example output plugin (PulseAudio or WASAPI)
3. Create example DSP plugin (Reverb or Compression)
4. Add plugin configuration UI
5. Implement dynamic plugin loading from .so/.dll files

