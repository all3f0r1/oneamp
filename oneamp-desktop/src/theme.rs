use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Theme configuration for OneAmp
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub colors: ColorScheme,
    pub fonts: FontConfig,
    pub layout: LayoutConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorScheme {
    // Main window colors
    pub window_bg: [u8; 3],
    pub panel_bg: [u8; 3],
    pub border: [u8; 3],
    
    // Display colors
    pub display_bg: [u8; 3],
    pub display_text: [u8; 3],
    pub display_accent: [u8; 3],
    
    // Button colors
    pub button_normal: [u8; 3],
    pub button_hovered: [u8; 3],
    pub button_active: [u8; 3],
    
    // Progress bar
    pub progress_bg: [u8; 3],
    pub progress_fill: [u8; 3],
    
    // Playlist
    pub playlist_bg: [u8; 3],
    pub playlist_text: [u8; 3],
    pub playlist_selected: [u8; 3],
    pub playlist_playing: [u8; 3],
    
    // Equalizer
    pub eq_slider: [u8; 3],
    pub eq_fill: [u8; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    pub timer_size: f32,
    pub track_info_size: f32,
    pub playlist_size: f32,
    pub button_size: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub window_min_width: f32,
    pub window_min_height: f32,
    pub player_height: f32,
    pub equalizer_height: f32,
    pub spacing: f32,
    pub padding: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::winamp_modern()
    }
}

impl Theme {
    /// Winamp Modern inspired theme (blue/metallic)
    pub fn winamp_modern() -> Self {
        Theme {
            name: "Winamp Modern".to_string(),
            colors: ColorScheme {
                window_bg: [40, 45, 55],
                panel_bg: [30, 35, 45],
                border: [60, 65, 75],
                
                display_bg: [15, 20, 35],
                display_text: [100, 180, 255],
                display_accent: [150, 220, 255],
                
                button_normal: [70, 75, 85],
                button_hovered: [90, 95, 105],
                button_active: [110, 115, 125],
                
                progress_bg: [50, 55, 65],
                progress_fill: [100, 180, 255],
                
                playlist_bg: [25, 30, 40],
                playlist_text: [200, 200, 200],
                playlist_selected: [60, 100, 150],
                playlist_playing: [100, 180, 255],
                
                eq_slider: [70, 75, 85],
                eq_fill: [100, 180, 255],
            },
            fonts: FontConfig {
                timer_size: 32.0,
                track_info_size: 14.0,
                playlist_size: 13.0,
                button_size: 12.0,
            },
            layout: LayoutConfig {
                window_min_width: 600.0,
                window_min_height: 500.0,
                player_height: 150.0,
                equalizer_height: 180.0,
                spacing: 8.0,
                padding: 10.0,
            },
        }
    }
    
    /// Dark theme (original OneAmp style)
    pub fn dark() -> Self {
        Theme {
            name: "Dark".to_string(),
            colors: ColorScheme {
                window_bg: [30, 30, 35],
                panel_bg: [25, 25, 30],
                border: [50, 50, 60],
                
                display_bg: [20, 20, 25],
                display_text: [220, 220, 220],
                display_accent: [0, 150, 200],
                
                button_normal: [50, 50, 60],
                button_hovered: [70, 70, 80],
                button_active: [90, 90, 100],
                
                progress_bg: [40, 40, 50],
                progress_fill: [0, 150, 200],
                
                playlist_bg: [20, 20, 25],
                playlist_text: [200, 200, 200],
                playlist_selected: [50, 50, 70],
                playlist_playing: [0, 150, 200],
                
                eq_slider: [50, 50, 60],
                eq_fill: [0, 150, 200],
            },
            fonts: FontConfig {
                timer_size: 28.0,
                track_info_size: 14.0,
                playlist_size: 13.0,
                button_size: 12.0,
            },
            layout: LayoutConfig {
                window_min_width: 600.0,
                window_min_height: 500.0,
                player_height: 140.0,
                equalizer_height: 180.0,
                spacing: 8.0,
                padding: 10.0,
            },
        }
    }
    
    /// Load theme from file
    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let theme: Theme = toml::from_str(&content)?;
        Ok(theme)
    }
    
    /// Save theme to file
    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
    
    /// Apply theme to egui context
    pub fn apply_to_egui(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        
        // Dark mode
        style.visuals.dark_mode = true;
        
        // Window colors
        style.visuals.window_fill = Self::color32(&self.colors.window_bg);
        style.visuals.panel_fill = Self::color32(&self.colors.panel_bg);
        
        // Text color
        style.visuals.override_text_color = Some(Self::color32(&self.colors.display_text));
        
        // Button colors
        style.visuals.widgets.inactive.weak_bg_fill = Self::color32(&self.colors.button_normal);
        style.visuals.widgets.hovered.weak_bg_fill = Self::color32(&self.colors.button_hovered);
        style.visuals.widgets.active.weak_bg_fill = Self::color32(&self.colors.button_active);
        
        // Selection color
        style.visuals.selection.bg_fill = Self::color32(&self.colors.playlist_selected);
        
        // Spacing
        style.spacing.item_spacing = egui::vec2(self.layout.spacing, self.layout.spacing);
        style.spacing.window_margin = egui::Margin::same(self.layout.padding);
        
        ctx.set_style(style);
    }
    
    /// Convert RGB array to egui Color32
    pub fn color32(rgb: &[u8; 3]) -> egui::Color32 {
        egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_theme() {
        let theme = Theme::default();
        assert_eq!(theme.name, "Winamp Modern");
    }
    
    #[test]
    fn test_winamp_modern_theme() {
        let theme = Theme::winamp_modern();
        assert_eq!(theme.name, "Winamp Modern");
        assert_eq!(theme.colors.window_bg, [40, 45, 55]);
        assert_eq!(theme.fonts.timer_size, 32.0);
        assert_eq!(theme.layout.window_min_width, 600.0);
    }
    
    #[test]
    fn test_dark_theme() {
        let theme = Theme::dark();
        assert_eq!(theme.name, "Dark");
        assert_eq!(theme.colors.window_bg, [30, 30, 35]);
    }
    
    #[test]
    fn test_theme_serialization() {
        let theme = Theme::winamp_modern();
        let toml = toml::to_string(&theme).unwrap();
        let deserialized: Theme = toml::from_str(&toml).unwrap();
        assert_eq!(theme.name, deserialized.name);
        assert_eq!(theme.colors.window_bg, deserialized.colors.window_bg);
        assert_eq!(theme.fonts.timer_size, deserialized.fonts.timer_size);
    }
    
    #[test]
    fn test_color32_conversion() {
        let rgb = [100, 150, 200];
        let color = Theme::color32(&rgb);
        assert_eq!(color.r(), 100);
        assert_eq!(color.g(), 150);
        assert_eq!(color.b(), 200);
    }
    
    #[test]
    fn test_theme_save_load() {
        use std::path::PathBuf;
        use std::fs;
        
        let theme = Theme::winamp_modern();
        let temp_path = PathBuf::from("/tmp/test_theme.toml");
        
        // Save
        theme.save(&temp_path).unwrap();
        
        // Load
        let loaded = Theme::load(&temp_path).unwrap();
        assert_eq!(theme.name, loaded.name);
        
        // Cleanup
        let _ = fs::remove_file(temp_path);
    }
    
    #[test]
    fn test_all_themes_have_valid_colors() {
        for theme in [Theme::winamp_modern(), Theme::dark()] {
            // All RGB values should be 0-255
            for &val in &theme.colors.window_bg {
                assert!(val <= 255);
            }
            for &val in &theme.colors.display_text {
                assert!(val <= 255);
            }
        }
    }
    
    #[test]
    fn test_font_sizes_are_positive() {
        let theme = Theme::default();
        assert!(theme.fonts.timer_size > 0.0);
        assert!(theme.fonts.track_info_size > 0.0);
        assert!(theme.fonts.playlist_size > 0.0);
        assert!(theme.fonts.button_size > 0.0);
    }
    
    #[test]
    fn test_layout_dimensions_are_positive() {
        let theme = Theme::default();
        assert!(theme.layout.window_min_width > 0.0);
        assert!(theme.layout.window_min_height > 0.0);
        assert!(theme.layout.player_height > 0.0);
        assert!(theme.layout.equalizer_height > 0.0);
        assert!(theme.layout.spacing >= 0.0);
        assert!(theme.layout.padding >= 0.0);
    }
}
