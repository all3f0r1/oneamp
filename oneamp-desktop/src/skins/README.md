# OneAmp Skinning System

**Version:** 1.0  
**Author:** Manus AI  
**Date:** December 3, 2025

---

## Overview

This document provides a technical overview of the OneAmp skinning system. This system allows for complete customization of the application's look and feel through simple, human-readable TOML files.

### Key Features

- **TOML-Based:** Skins are defined in simple `.toml` files, making them easy to create and edit.
- **Directory-Based:** Each skin resides in its own directory, which can contain the `skin.toml` file and other assets like custom fonts.
- **Extensible:** The system is designed to be easily extended with new properties (e.g., images, animations) in the future.
- **Dynamic Loading:** Skins are discovered and loaded at runtime, with no need to recompile the application.
- **Fallback Mechanism:** A default, built-in skin ensures the application always has a valid appearance, even with misconfigured or missing skins.

---

## Architecture

The skinning system is composed of three main components:

1.  **Data Structures (`mod.rs`):** A set of `serde`-serializable structs (`Skin`, `Colors`, `Fonts`, `Metrics`) that define the structure of a skin.
2.  **Parser (`parser.rs`):** Responsible for reading a `skin.toml` file from disk, deserializing it into the `Skin` struct, and validating its contents.
3.  **Manager (`manager.rs`):** The central orchestrator. The `SkinManager` handles:
    -   **Discovery:** Scanning the `skins` directory to find all available skins.
    -   **Loading:** Using the parser to load each valid skin into memory.
    -   **State Management:** Keeping track of all available skins and which one is currently active.
    -   **Application:** Applying the active skin's style and visuals to the `egui` context on each frame.
4.  **UI Components (`ui.rs`):** Provides reusable `egui` widgets for skin selection and management, such as a drop-down menu or a settings panel.

### Data Flow

1.  **Startup:** `SkinManager::discover_and_load()` is called.
    -   It scans the `~/.config/oneamp/skins/` directory.
    -   For each subdirectory, it calls `parser::load_skin()`.
    -   Valid skins are added to its `available_skins` list.
2.  **UI Rendering:** On every frame, `OneAmpApp::update()` calls `skin_manager.apply_skin()`.
    -   `apply_skin()` gets the active `Skin`.
    -   It converts the `Skin`'s properties into `egui::Style` and `egui::Visuals`.
    -   It calls `ctx.set_style()` to apply the new look.
3.  **User Interaction:**
    -   The user interacts with a UI component from `skins/ui.rs` (e.g., a menu).
    -   This calls `skin_manager.set_active_skin(index)`.
    -   On the next frame, `apply_skin()` will use the newly selected skin.

---

## How to Create a Skin

Creating a new skin is straightforward.

### 1. Create a Directory

Create a new folder in the `skins` directory (e.g., `~/.config/oneamp/skins/my-awesome-skin/`).

### 2. Create `skin.toml`

Inside your new directory, create a file named `skin.toml`. This file will define your skin.

### 3. Define Metadata

Start with the `[metadata]` section to describe your skin.

```toml
[metadata]
name = "My Awesome Skin"
author = "Your Name"
version = "1.0"
description = "A vibrant, high-contrast theme."
```

### 4. Define Colors

Specify the color palette in the `[colors]` section. All colors must be valid hex strings.

```toml
[colors]
dark_mode = true
background = "#121212"
text = "#E0E0E0"
accent = "#7F00FF" # Electric Violet
# ... and so on for all required color fields.
```

### 5. Define Fonts and Metrics

Customize fonts and spacing in the `[fonts]` and `[metrics]` sections.

```toml
[fonts]
proportional = "Inter"
monospace = "Fira Code"

[metrics]
window_rounding = 8.0
widget_rounding = 4.0
body_text_size = 15.0
```

### 6. (Optional) Add Custom Fonts

If your skin uses a font that isn't installed on the system, you can include it with your skin.

1.  Create a `fonts` subdirectory (e.g., `my-awesome-skin/fonts/`).
2.  Place your font file (e.g., `MyFont.ttf`) inside.
3.  Reference it in your `skin.toml`:

```toml
[fonts]
timer_font = "fonts/MyFont.ttf"
```

---

## API Reference

### `SkinManager`

-   `discover_and_load(skins_dir: &Path) -> Self`
    -   Initializes the manager and loads all skins from the specified directory.
-   `get_active_skin() -> &Skin`
    -   Returns a reference to the currently active skin.
-   `set_active_skin(index: usize) -> bool`
    -   Changes the active skin by its index in the `available_skins` list.
-   `apply_skin(ctx: &egui::Context)`
    -   Applies the active skin's style to the `egui` context.
-   `reload_active_skin() -> Result<()>`
    -   Reloads the currently active skin from its file, useful for live-editing.

### `parser`

-   `load_skin(skin_dir: &Path) -> Result<Skin>`
    -   Loads and validates a single skin from a directory.
-   `hex_to_color32(hex: &str) -> Result<egui::Color32>`
    -   A utility function to convert a hex string to an `egui::Color32`.

---

## Future Development

This system is designed for future expansion. Potential enhancements include:

-   **Image-Based Assets:** Adding support for custom images for backgrounds, buttons, or icons.
-   **Animations:** Defining simple animation properties (e.g., fade-in duration) in `skin.toml`.
-   **Live Editor:** A built-in UI for editing skins in real-time within OneAmp.
