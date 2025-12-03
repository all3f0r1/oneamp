// Skin TOML Parser
// Responsible for loading and validating skin.toml files.

use super::{Colors, Fonts, Metadata, Metrics, Skin};
use anyhow::{anyhow, Result};
use std::fs;
use std::path::Path;

/// Loads a skin from a skin.toml file.
///
/// # Arguments
/// * `skin_dir` - Path to the directory containing the skin.toml file
///
/// # Returns
/// A `Skin` struct if successful, or an error if the file is missing or invalid.
pub fn load_skin(skin_dir: &Path) -> Result<Skin> {
    let skin_file = skin_dir.join("skin.toml");

    if !skin_file.exists() {
        return Err(anyhow!("skin.toml not found in {:?}", skin_dir));
    }

    let content =
        fs::read_to_string(&skin_file).map_err(|e| anyhow!("Failed to read skin.toml: {}", e))?;

    let mut skin: Skin =
        toml::from_str(&content).map_err(|e| anyhow!("Failed to parse skin.toml: {}", e))?;

    // Set the skin's path for relative asset resolution
    skin.path = skin_dir.to_path_buf();

    // Validate the skin
    validate_skin(&skin)?;

    Ok(skin)
}

/// Validates a skin's configuration.
/// Checks for required fields and valid color formats.
fn validate_skin(skin: &Skin) -> Result<()> {
    // Check metadata
    if skin.metadata.name.is_empty() {
        return Err(anyhow!("Skin name cannot be empty"));
    }

    // Check colors are valid hex strings
    validate_hex_color(&skin.colors.background)?;
    validate_hex_color(&skin.colors.text)?;
    validate_hex_color(&skin.colors.window_fill)?;
    validate_hex_color(&skin.colors.window_stroke)?;
    validate_hex_color(&skin.colors.panel_fill)?;
    validate_hex_color(&skin.colors.widget_bg)?;
    validate_hex_color(&skin.colors.widget_stroke)?;
    validate_hex_color(&skin.colors.hovered_widget_bg)?;
    validate_hex_color(&skin.colors.active_widget_bg)?;
    validate_hex_color(&skin.colors.inactive_widget_bg)?;
    validate_hex_color(&skin.colors.accent)?;
    validate_hex_color(&skin.colors.error)?;
    validate_hex_color(&skin.colors.warning)?;
    validate_hex_color(&skin.colors.playlist_current_track)?;
    validate_hex_color(&skin.colors.playlist_selected_bg)?;

    // Check fonts
    if skin.fonts.proportional.is_empty() {
        return Err(anyhow!("Proportional font cannot be empty"));
    }
    if skin.fonts.monospace.is_empty() {
        return Err(anyhow!("Monospace font cannot be empty"));
    }

    // Check metrics are positive
    if skin.metrics.window_rounding < 0.0 {
        return Err(anyhow!("window_rounding must be non-negative"));
    }
    if skin.metrics.widget_rounding < 0.0 {
        return Err(anyhow!("widget_rounding must be non-negative"));
    }
    if skin.metrics.scrollbar_width <= 0.0 {
        return Err(anyhow!("scrollbar_width must be positive"));
    }
    if skin.metrics.window_padding < 0.0 {
        return Err(anyhow!("window_padding must be non-negative"));
    }
    if skin.metrics.body_text_size <= 0.0 {
        return Err(anyhow!("body_text_size must be positive"));
    }
    if skin.metrics.heading_text_size <= 0.0 {
        return Err(anyhow!("heading_text_size must be positive"));
    }
    if skin.metrics.timer_text_size <= 0.0 {
        return Err(anyhow!("timer_text_size must be positive"));
    }

    Ok(())
}

/// Validates that a string is a valid hex color.
/// Accepts formats: #RGB, #RRGGBB, #RRGGBBAA
fn validate_hex_color(color: &str) -> Result<()> {
    if !color.starts_with('#') {
        return Err(anyhow!("Color must start with '#': {}", color));
    }

    let hex_part = &color[1..];
    if hex_part.len() != 3 && hex_part.len() != 6 && hex_part.len() != 8 {
        return Err(anyhow!(
            "Color must be #RGB, #RRGGBB, or #RRGGBBAA: {}",
            color
        ));
    }

    if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!("Color contains invalid hex digits: {}", color));
    }

    Ok(())
}

/// Converts a hex color string to an egui::Color32.
/// Accepts formats: #RGB, #RRGGBB, #RRGGBBAA
pub fn hex_to_color32(hex: &str) -> Result<egui::Color32> {
    if !hex.starts_with('#') {
        return Err(anyhow!("Color must start with '#': {}", hex));
    }

    let hex_part = &hex[1..];

    let (r, g, b, a) = match hex_part.len() {
        3 => {
            // #RGB format
            let r = u8::from_str_radix(&hex_part[0..1], 16)? * 17;
            let g = u8::from_str_radix(&hex_part[1..2], 16)? * 17;
            let b = u8::from_str_radix(&hex_part[2..3], 16)? * 17;
            (r, g, b, 255)
        }
        6 => {
            // #RRGGBB format
            let r = u8::from_str_radix(&hex_part[0..2], 16)?;
            let g = u8::from_str_radix(&hex_part[2..4], 16)?;
            let b = u8::from_str_radix(&hex_part[4..6], 16)?;
            (r, g, b, 255)
        }
        8 => {
            // #RRGGBBAA format
            let r = u8::from_str_radix(&hex_part[0..2], 16)?;
            let g = u8::from_str_radix(&hex_part[2..4], 16)?;
            let b = u8::from_str_radix(&hex_part[4..6], 16)?;
            let a = u8::from_str_radix(&hex_part[6..8], 16)?;
            (r, g, b, a)
        }
        _ => return Err(anyhow!("Invalid color format: {}", hex)),
    };

    Ok(egui::Color32::from_rgba_unmultiplied(r, g, b, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_hex_color_valid() {
        assert!(validate_hex_color("#fff").is_ok());
        assert!(validate_hex_color("#ffffff").is_ok());
        assert!(validate_hex_color("#ffffff80").is_ok());
    }

    #[test]
    fn test_validate_hex_color_invalid() {
        assert!(validate_hex_color("ffffff").is_err()); // Missing #
        assert!(validate_hex_color("#ff").is_err()); // Too short
        assert!(validate_hex_color("#gggggg").is_err()); // Invalid hex
    }

    #[test]
    fn test_hex_to_color32_rrggbb() {
        let color = hex_to_color32("#ffffff").unwrap();
        assert_eq!(color, egui::Color32::WHITE);
    }

    #[test]
    fn test_hex_to_color32_rrggbbaa() {
        let color = hex_to_color32("#ffffff80").unwrap();
        assert_eq!(color.r(), 255);
        assert_eq!(color.g(), 255);
        assert_eq!(color.b(), 255);
        assert_eq!(color.a(), 128);
    }

    #[test]
    fn test_hex_to_color32_rgb() {
        let color = hex_to_color32("#fff").unwrap();
        assert_eq!(color.r(), 255);
        assert_eq!(color.g(), 255);
        assert_eq!(color.b(), 255);
    }
}
