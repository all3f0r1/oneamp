# OneAmp Skin System Architecture

**Author:** Manus AI  
**Date:** December 3, 2025  
**Version:** 1.0

---

## 1. Overview

This document outlines the architecture for a new, extensible skinning system for OneAmp. The goal is to allow users to customize the application's appearance using simple configuration files, moving away from the hardcoded `Theme` struct. This system is inspired by Winamp's classic skins but adapted for a modern, `egui`-based application.

The core of this system is a directory-based structure where each skin is a folder containing a `skin.toml` file and optional assets like fonts or images.

---

## 2. Skin Directory Structure

Skins will be located in a `skins` directory at the root of the application's configuration folder (e.g., `~/.config/oneamp/skins/`). Each subdirectory represents a single skin.

```
~/.config/oneamp/
└── skins/
    ├── oneamp-dark/
    │   └── skin.toml
    ├── winamp-classic/
    │   ├── skin.toml
    │   └── fonts/
    │       └── digital.ttf
    └── my-custom-skin/
        └── skin.toml
```

- The application will scan this directory at startup to discover available skins.
- A default, built-in skin will be used if the `skins` directory is missing or no valid skins are found.

---

## 3. The `skin.toml` File Format

The `skin.toml` file is the heart of each skin, defining all visual properties. It uses the TOML format for its readability and ease of use. The file is divided into logical sections.

### 3.1. `[metadata]` Section

Contains descriptive information about the skin.

```toml
[metadata]
# Name of the skin, displayed in the UI.
name = "Winamp Classic"
# Author of the skin.
author = "Zarko Jovic & GuidoD (Converted by Manus AI)"
# Version of the skin file.
version = "1.0"
# Brief description.
description = "A classic Winamp 2.x look and feel."
```

### 3.2. `[colors]` Section

Defines the color palette for the entire application. Colors are specified as hex strings (e.g., `"#RRGGBB"` or `"#RRGGBBAA"`). This section will directly map to `egui::Visuals`.

```toml
[colors]
# Dark mode (true or false)
dark_mode = true

# egui default colors
background = "#1C3D7D"          # Overall background
text = "#FFFFFF"                # Default text color
window_fill = "#1C3D7D"         # Background of windows
window_stroke = "#6788C9"       # Border of windows
panel_fill = "#102A54"           # Background of panels (e.g., playlist)

# Widget colors
widget_bg = "#2A4B8F"           # Background of buttons, sliders, etc.
widget_stroke = "#6788C9"       # Border of widgets

# Interaction states
hovered_widget_bg = "#4A6BAF"    # Widget background on hover
active_widget_bg = "#6788C9"     # Widget background when active/clicked
inactive_widget_bg = "#102A54"  # Disabled widget background

# Special colors
accent = "#BBD2FF"              # Accent color for selections, progress bars
error = "#FF4444"
warning = "#FFBB33"

# Playlist specific
playlist_current_track = "#BBD2FF"
playlist_selected_bg = "#6788C9"
```

### 3.3. `[fonts]` Section

Defines the fonts to be used. Fonts can be specified by name (if installed on the system) or by a relative path to a font file within the skin's directory.

```toml
[fonts]
# Default font for all text.
# Proportional fonts are preferred for general UI.
proportional = "Arial"

# Monospaced font for things like timers or detailed info.
monospace = "Courier New"

# Path to a custom font file, relative to the skin.toml file.
# This will be used for the main timer display to give a digital look.
timer_font = "fonts/digital.ttf"
```

### 3.4. `[metrics]` Section

Defines sizes, spacing, and other layout-related values. This allows skins to alter the application's density and feel.

```toml
[metrics]
# General spacing
window_rounding = 0.0
widget_rounding = 2.0
scrollbar_width = 8.0

# Padding
window_padding = 8.0
button_padding = [12.0, 4.0] # [x, y]

# Text sizes
body_text_size = 14.0
heading_text_size = 18.0
timer_text_size = 48.0
```

---

## 4. Rust Data Structures

To parse the `skin.toml` file, we will define a set of Rust structs using `serde` for deserialization. These structs will mirror the TOML structure.

```rust
// In a new file: oneamp-desktop/src/skins/mod.rs

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
pub struct Skin {
    pub metadata: Metadata,
    pub colors: Colors,
    pub fonts: Fonts,
    pub metrics: Metrics,

    #[serde(skip)]
    pub path: PathBuf, // The path to the skin's directory
}

#[derive(Deserialize, Debug)]
pub struct Metadata {
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
}

#[derive(Deserialize, Debug)]
pub struct Colors {
    pub dark_mode: bool,
    pub background: String,
    pub text: String,
    pub window_fill: String,
    pub window_stroke: String,
    pub panel_fill: String,
    pub widget_bg: String,
    pub widget_stroke: String,
    pub hovered_widget_bg: String,
    pub active_widget_bg: String,
    pub inactive_widget_bg: String,
    pub accent: String,
    pub error: String,
    pub warning: String,
    pub playlist_current_track: String,
    pub playlist_selected_bg: String,
}

#[derive(Deserialize, Debug)]
pub struct Fonts {
    pub proportional: String,
    pub monospace: String,
    pub timer_font: Option<PathBuf>,
}

#[derive(Deserialize, Debug)]
pub struct Metrics {
    pub window_rounding: f32,
    pub widget_rounding: f32,
    pub scrollbar_width: f32,
    pub window_padding: f32,
    pub button_padding: [f32; 2],
    pub body_text_size: f32,
    pub heading_text_size: f32,
    pub timer_text_size: f32,
}
```

---

## 5. Implementation Plan

### 5.1. `SkinManager`

A new `SkinManager` struct will be responsible for:
1.  **Discovery:** Scanning the `skins` directory for valid `skin.toml` files at startup.
2.  **Loading & Parsing:** Reading the TOML file and deserializing it into the `Skin` struct.
3.  **Management:** Storing the list of available skins and tracking the currently active skin.
4.  **Applying:** Providing a method to generate an `egui::Style` object from the active `Skin`.

```rust
// In oneamp-desktop/src/skins/manager.rs

pub struct SkinManager {
    pub available_skins: Vec<Skin>,
    pub active_skin_index: usize,
}

impl SkinManager {
    pub fn discover_and_load(skins_dir: &Path) -> Self { ... }
    pub fn get_active_skin(&self) -> &Skin { ... }
    pub fn apply_skin(&self, ctx: &egui::Context) { ... }
}
```

### 5.2. Application of the Skin

The `apply_skin` method will be the bridge between our `Skin` struct and `egui`. It will construct an `egui::Style` and an `egui::Visuals` object and apply them to the `egui::Context`.

- **Colors:** Hex color strings from `skin.colors` will be parsed into `egui::Color32`.
- **Fonts:** A new `egui::FontDefinitions` object will be created. Custom fonts will be loaded from their file paths.
- **Metrics:** Values from `skin.metrics` will be used to populate the `egui::Style`'s `spacing` and `window` fields.

This method will be called once at application startup and whenever the user selects a new skin from the UI.

### 5.3. Integration into `main.rs`

The main application struct (`OneAmpApp`) will own the `SkinManager`. The `update` method will call `skin_manager.apply_skin(&ctx)` at the beginning of each frame to ensure the style is always up-to-date.

```rust
// In oneamp-desktop/src/main.rs

struct OneAmpApp {
    // ... other fields
    skin_manager: SkinManager,
}

impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.skin_manager.apply_skin(ctx);
        // ... rest of the UI code
    }
}
```

---

## 6. Challenges and Considerations

- **Error Handling:** The system must be robust against malformed `skin.toml` files or missing assets. It should log errors and fall back to a default skin.
- **Performance:** Loading and parsing TOML files is fast, but font loading can be slower. This should only be done at startup or when a skin changes, not per frame.
- **Default Skin:** A default `Skin` struct should be hardcoded into the application to be used as a fallback.
- **Live Reloading:** For development, a feature to automatically reload the active skin when its `skin.toml` file changes would be highly beneficial.
