# Skin System Integration Guide

This guide explains how to integrate the new skin system into OneAmp's `main.rs`.

## Step 1: Add the Skins Module to main.rs

Add this line to the module declarations at the top of `main.rs`:

```rust
mod skins;
use skins::SkinManager;
```

## Step 2: Add SkinManager to OneAmpApp

Modify the `OneAmpApp` struct to include a `SkinManager`:

```rust
struct OneAmpApp {
    // ... existing fields ...
    
    // Skin system
    skin_manager: SkinManager,
    
    // ... rest of fields ...
}
```

## Step 3: Initialize SkinManager in new()

In the `OneAmpApp::new()` method, initialize the skin manager after creating the theme:

```rust
impl OneAmpApp {
    fn new(cc: &eframe::CreationContext<'_>, use_custom_chrome: bool) -> Self {
        // Initialize skin manager
        let skins_dir = dirs::config_dir()
            .map(|d| d.join("oneamp").join("skins"))
            .unwrap_or_else(|| PathBuf::from("./skins"));
        
        let skin_manager = SkinManager::discover_and_load(&skins_dir);
        
        // ... rest of initialization ...
        
        let mut app = Self {
            // ... existing fields ...
            skin_manager,
            // ... rest of fields ...
        };
        
        // Apply the active skin
        skin_manager.apply_skin(&cc.egui_ctx);
        
        app
    }
}
```

## Step 4: Apply Skin in update()

In the `update` method of the `eframe::App` implementation, apply the skin at the beginning:

```rust
impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Apply the active skin at the beginning of each frame
        self.skin_manager.apply_skin(ctx);
        
        // ... rest of update code ...
    }
}
```

## Step 5: Add Skin Selection UI

Add a skin selection menu to the application. This can be done in the main UI or in a settings panel:

```rust
// In the update method, add a skin selector
if ui.button("Select Skin") {
    ui.menu_button("Skins", |ui| {
        for (index, skin) in self.skin_manager.available_skins.iter().enumerate() {
            if ui.selectable_label(
                index == self.skin_manager.active_skin_index,
                &skin.metadata.name,
            ) {
                self.skin_manager.set_active_skin(index);
            }
        }
    });
}
```

## Step 6: Save Active Skin to Config

Optionally, save the active skin name to the application config so it persists across restarts:

```rust
// In AppConfig struct
#[derive(Serialize, Deserialize, Debug)]
pub struct AppConfig {
    pub active_skin: String,
    // ... other fields ...
}

// When saving config
config.active_skin = self.skin_manager.get_active_skin().metadata.name.clone();

// When loading config
if let Some(skin_index) = self.skin_manager.find_skin_by_name(&config.active_skin) {
    self.skin_manager.set_active_skin(skin_index);
}
```

## Directory Structure

Ensure the following directory structure exists:

```
oneamp/
├── skins/
│   ├── oneamp-dark/
│   │   └── skin.toml
│   ├── winamp5-classified/
│   │   └── skin.toml
│   └── (other skins...)
├── oneamp-desktop/
│   └── src/
│       ├── skins/
│       │   ├── mod.rs
│       │   ├── parser.rs
│       │   └── manager.rs
│       └── main.rs
```

## Testing the Integration

1. Build the project: `cargo build --release`
2. Run the application: `cargo run --release`
3. The application should load with the default "OneAmp Dark" skin
4. You should be able to see the skin selector in the UI
5. Selecting a different skin should immediately apply the colors and styles

## Troubleshooting

- **Skins not loading:** Check that the `skins` directory exists and contains valid `skin.toml` files
- **Colors not applying:** Verify that hex color strings are in the correct format (#RRGGBB or #RRGGBBAA)
- **Fonts not loading:** Ensure font names are valid system fonts or that custom font paths are correct
- **Performance issues:** If the application is slow, check that `apply_skin()` is not being called excessively

## Future Enhancements

- Add a skin editor UI for creating and modifying skins
- Add support for custom fonts in skin directories
- Add support for skin preview images
- Add a skin marketplace or repository for downloading skins
- Add live reload for development
