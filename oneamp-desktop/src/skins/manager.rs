// Skin Manager
// Responsible for discovering, loading, and applying skins.

use super::{Skin, parser};
use anyhow::Result;
use std::path::Path;
use std::fs;

/// Manages the discovery, loading, and application of skins.
pub struct SkinManager {
    /// List of available skins discovered from the skins directory.
    pub available_skins: Vec<Skin>,

    /// Index of the currently active skin in the `available_skins` list.
    pub active_skin_index: usize,
}

impl SkinManager {
    /// Creates a new SkinManager and discovers all available skins.
    /// 
    /// # Arguments
    /// * `skins_dir` - Path to the directory containing skin subdirectories
    /// 
    /// # Returns
    /// A new `SkinManager` with discovered skins. If no skins are found or the
    /// directory doesn't exist, the manager will contain only the default built-in skin.
    pub fn discover_and_load(skins_dir: &Path) -> Self {
        let mut available_skins = vec![Skin::default_builtin()];

        if !skins_dir.exists() {
            eprintln!("Skins directory not found: {:?}", skins_dir);
            return Self {
                available_skins,
                active_skin_index: 0,
            };
        }

        // Scan the skins directory for subdirectories
        match fs::read_dir(skins_dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        match parser::load_skin(&path) {
                            Ok(skin) => {
                                available_skins.push(skin);
                            }
                            Err(e) => {
                                eprintln!("Failed to load skin from {:?}: {}", path, e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to read skins directory: {}", e);
            }
        }

        Self {
            available_skins,
            active_skin_index: 0,
        }
    }

    /// Gets a reference to the currently active skin.
    pub fn get_active_skin(&self) -> &Skin {
        self.available_skins
            .get(self.active_skin_index)
            .unwrap_or(&self.available_skins[0])
    }

    /// Gets a mutable reference to the currently active skin.
    pub fn get_active_skin_mut(&mut self) -> &mut Skin {
        let index = self.active_skin_index;
        &mut self.available_skins[index]
    }

    /// Sets the active skin by index.
    /// 
    /// # Arguments
    /// * `index` - Index of the skin in the `available_skins` list
    /// 
    /// # Returns
    /// `true` if the skin was successfully changed, `false` if the index is out of bounds.
    pub fn set_active_skin(&mut self, index: usize) -> bool {
        if index < self.available_skins.len() {
            self.active_skin_index = index;
            true
        } else {
            false
        }
    }

    /// Finds the index of a skin by name.
    /// 
    /// # Arguments
    /// * `name` - Name of the skin to find
    /// 
    /// # Returns
    /// The index of the skin if found, or `None` if not found.
    pub fn find_skin_by_name(&self, name: &str) -> Option<usize> {
        self.available_skins
            .iter()
            .position(|skin| skin.metadata.name == name)
    }

    /// Applies the active skin to the egui context.
    /// 
    /// This method constructs an `egui::Visuals` and `egui::Style` from the active skin
    /// and applies them to the provided context.
    pub fn apply_skin(&self, ctx: &egui::Context) {
        let skin = self.get_active_skin();

        // Create visuals from the skin's colors
        let mut visuals = if skin.colors.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        // Apply custom colors
        if let Ok(bg) = parser::hex_to_color32(&skin.colors.background) {
            visuals.panel_fill = bg;
        }
        if let Ok(text) = parser::hex_to_color32(&skin.colors.text) {
            visuals.text_color = text;
        }
        if let Ok(window_fill) = parser::hex_to_color32(&skin.colors.window_fill) {
            visuals.window_fill = window_fill;
        }
        if let Ok(window_stroke) = parser::hex_to_color32(&skin.colors.window_stroke) {
            visuals.window_stroke = egui::Stroke::new(1.0, window_stroke);
        }
        if let Ok(accent) = parser::hex_to_color32(&skin.colors.accent) {
            visuals.selection.bg_fill = accent;
            visuals.selection.stroke.color = accent;
        }

        // Apply widget colors
        visuals.widgets.inactive.bg_fill = parser::hex_to_color32(&skin.colors.widget_bg)
            .unwrap_or(visuals.widgets.inactive.bg_fill);
        visuals.widgets.hovered.bg_fill = parser::hex_to_color32(&skin.colors.hovered_widget_bg)
            .unwrap_or(visuals.widgets.hovered.bg_fill);
        visuals.widgets.active.bg_fill = parser::hex_to_color32(&skin.colors.active_widget_bg)
            .unwrap_or(visuals.widgets.active.bg_fill);

        // Create style from the skin's metrics
        let mut style = (*ctx.style()).clone();
        style.spacing.window_margin = egui::Margin::same(skin.metrics.window_padding);
        style.spacing.button_padding = egui::Vec2::new(
            skin.metrics.button_padding[0],
            skin.metrics.button_padding[1],
        );
        style.visuals = visuals;

        // Apply the style to the context
        ctx.set_style(style);
    }

    /// Reloads the active skin from disk.
    /// 
    /// This is useful for development when skin files are being edited.
    pub fn reload_active_skin(&mut self) -> Result<()> {
        let skin_path = self.get_active_skin().path.clone();
        let reloaded_skin = parser::load_skin(&skin_path)?;
        self.available_skins[self.active_skin_index] = reloaded_skin;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_skin_manager_creation() {
        let manager = SkinManager::discover_and_load(Path::new("/nonexistent"));
        assert!(manager.available_skins.len() >= 1);
        assert_eq!(manager.active_skin_index, 0);
    }

    #[test]
    fn test_get_active_skin() {
        let manager = SkinManager::discover_and_load(Path::new("/nonexistent"));
        let skin = manager.get_active_skin();
        assert_eq!(skin.metadata.name, "OneAmp Dark");
    }

    #[test]
    fn test_set_active_skin() {
        let mut manager = SkinManager::discover_and_load(Path::new("/nonexistent"));
        assert!(!manager.set_active_skin(999)); // Out of bounds
        assert!(manager.set_active_skin(0)); // Valid index
    }

    #[test]
    fn test_find_skin_by_name() {
        let manager = SkinManager::discover_and_load(Path::new("/nonexistent"));
        let index = manager.find_skin_by_name("OneAmp Dark");
        assert_eq!(index, Some(0));
        assert_eq!(manager.find_skin_by_name("Nonexistent"), None);
    }
}
