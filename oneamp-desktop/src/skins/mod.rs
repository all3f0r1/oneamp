// OneAmp Skin System
// This module provides a flexible, TOML-based skinning system for OneAmp.

pub mod manager;
pub mod parser;
pub mod ui;
#[cfg(test)]
mod tests;

pub use manager::SkinManager;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents a complete skin configuration.
/// A skin defines colors, fonts, metrics, and metadata for the OneAmp UI.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Skin {
    pub metadata: Metadata,
    pub colors: Colors,
    pub fonts: Fonts,
    pub metrics: Metrics,

    #[serde(skip)]
    pub path: PathBuf,
}

impl Skin {
    /// Creates a default built-in skin with OneAmp's original colors.
    pub fn default_builtin() -> Self {
        Self {
            metadata: Metadata {
                name: "OneAmp Dark".to_string(),
                author: "Manus AI".to_string(),
                version: "1.0".to_string(),
                description: "The default OneAmp dark theme.".to_string(),
            },
            colors: Colors::default(),
            fonts: Fonts::default(),
            metrics: Metrics::default(),
            path: PathBuf::new(),
        }
    }
}

/// Metadata about a skin (name, author, version, etc.)
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Metadata {
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            author: "OneAmp".to_string(),
            version: "1.0".to_string(),
            description: "Default skin".to_string(),
        }
    }
}

/// Color palette for the entire application.
/// All colors are specified as hex strings (e.g., "#RRGGBB" or "#RRGGBBAA").
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Colors {
    pub dark_mode: bool,

    // Main UI colors
    pub background: String,
    pub text: String,
    pub window_fill: String,
    pub window_stroke: String,
    pub panel_fill: String,

    // Widget colors
    pub widget_bg: String,
    pub widget_stroke: String,
    pub hovered_widget_bg: String,
    pub active_widget_bg: String,
    pub inactive_widget_bg: String,

    // Special colors
    pub accent: String,
    pub error: String,
    pub warning: String,

    // Playlist specific
    pub playlist_current_track: String,
    pub playlist_selected_bg: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            dark_mode: true,
            background: "#0a0a0a".to_string(),
            text: "#ffffff".to_string(),
            window_fill: "#1a1a1a".to_string(),
            window_stroke: "#404040".to_string(),
            panel_fill: "#0f0f0f".to_string(),
            widget_bg: "#2a2a2a".to_string(),
            widget_stroke: "#404040".to_string(),
            hovered_widget_bg: "#3a3a3a".to_string(),
            active_widget_bg: "#4a4a4a".to_string(),
            inactive_widget_bg: "#1a1a1a".to_string(),
            accent: "#00d4ff".to_string(),
            error: "#ff4444".to_string(),
            warning: "#ffbb33".to_string(),
            playlist_current_track: "#00d4ff".to_string(),
            playlist_selected_bg: "#404040".to_string(),
        }
    }
}

/// Font configuration for the application.
/// Fonts can be specified by system name or by a path relative to the skin directory.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Fonts {
    /// Default proportional font for general UI text.
    pub proportional: String,

    /// Monospaced font for code-like text (timers, detailed info).
    pub monospace: String,

    /// Optional path to a custom font file for the timer display.
    pub timer_font: Option<PathBuf>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            proportional: "Arial".to_string(),
            monospace: "Courier New".to_string(),
            timer_font: None,
        }
    }
}

/// Layout and spacing metrics for the UI.
/// These values control the "density" and feel of the interface.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Metrics {
    /// Rounding radius for window corners.
    pub window_rounding: f32,

    /// Rounding radius for widget corners (buttons, sliders, etc.).
    pub widget_rounding: f32,

    /// Width of scrollbars.
    pub scrollbar_width: f32,

    /// Padding inside windows.
    pub window_padding: f32,

    /// Padding inside buttons [x, y].
    pub button_padding: [f32; 2],

    /// Font size for body text.
    pub body_text_size: f32,

    /// Font size for headings.
    pub heading_text_size: f32,

    /// Font size for the timer display.
    pub timer_text_size: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            window_rounding: 4.0,
            widget_rounding: 2.0,
            scrollbar_width: 8.0,
            window_padding: 8.0,
            button_padding: [12.0, 4.0],
            body_text_size: 14.0,
            heading_text_size: 18.0,
            timer_text_size: 48.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_skin_creation() {
        let skin = Skin::default_builtin();
        assert_eq!(skin.metadata.name, "OneAmp Dark");
        assert!(skin.colors.dark_mode);
    }

    #[test]
    fn test_colors_default() {
        let colors = Colors::default();
        assert_eq!(colors.background, "#0a0a0a");
        assert_eq!(colors.text, "#ffffff");
    }

    #[test]
    fn test_fonts_default() {
        let fonts = Fonts::default();
        assert_eq!(fonts.proportional, "Arial");
        assert_eq!(fonts.monospace, "Courier New");
    }

    #[test]
    fn test_metrics_default() {
        let metrics = Metrics::default();
        assert_eq!(metrics.window_rounding, 4.0);
        assert_eq!(metrics.timer_text_size, 48.0);
    }
}
