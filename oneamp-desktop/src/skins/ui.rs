// Skin Selection and Management UI
// Provides UI components for selecting and managing skins.

use super::SkinManager;
use egui::{RichText, Ui};

/// Renders a skin selector menu in the UI.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `skin_manager` - The skin manager instance
///
/// # Returns
/// `true` if a skin was selected, `false` otherwise
pub fn skin_selector_menu(ui: &mut Ui, skin_manager: &mut SkinManager) -> bool {
    // Collect skin data before the menu to avoid borrow checker issues
    let skins_data: Vec<_> = skin_manager
        .available_skins
        .iter()
        .enumerate()
        .map(|(index, skin)| {
            let is_active = index == skin_manager.active_skin_index;
            (
                index,
                skin.metadata.name.clone(),
                skin.metadata.description.clone(),
                is_active,
            )
        })
        .collect();

    let mut skin_changed = false;

    ui.menu_button("🎨 Skins", |ui| {
        for (index, name, description, is_active) in skins_data {
            let label = if is_active {
                RichText::new(format!("✓ {}", name)).color(egui::Color32::from_rgb(0, 212, 255))
            } else {
                RichText::new(&name)
            };

            if ui.selectable_label(is_active, label).clicked() {
                if skin_manager.set_active_skin(index) {
                    skin_changed = true;
                }
            }

            // Show tooltip with skin description
            ui.label(
                RichText::new(&description)
                    .small()
                    .color(egui::Color32::GRAY),
            );
        }
    });

    skin_changed
}

/// Renders a compact skin selector button.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `skin_manager` - The skin manager instance
///
/// # Returns
/// `true` if the skin selector was clicked, `false` otherwise
pub fn skin_selector_button(ui: &mut Ui, skin_manager: &SkinManager) -> bool {
    let current_skin = skin_manager.get_active_skin();
    let button_text = format!("Skin: {}", current_skin.metadata.name);

    ui.button(button_text).clicked()
}

/// Renders a detailed skin info panel.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `skin_manager` - The skin manager instance
pub fn skin_info_panel(ui: &mut Ui, skin_manager: &SkinManager) {
    let skin = skin_manager.get_active_skin();

    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.heading("Current Skin");
            ui.separator();

            ui.label(RichText::new(&skin.metadata.name).strong());
            ui.label(format!("Author: {}", skin.metadata.author));
            ui.label(format!("Version: {}", skin.metadata.version));
            ui.label(format!("Description: {}", skin.metadata.description));

            ui.separator();
            ui.label("Colors:");
            ui.horizontal(|ui| {
                // Show a color swatch for the background
                let bg_color =
                    parse_hex_color(&skin.colors.background).unwrap_or(egui::Color32::GRAY);
                ui.colored_label(bg_color, "■ Background");

                let text_color = parse_hex_color(&skin.colors.text).unwrap_or(egui::Color32::WHITE);
                ui.colored_label(text_color, "■ Text");

                let accent_color =
                    parse_hex_color(&skin.colors.accent).unwrap_or(egui::Color32::LIGHT_BLUE);
                ui.colored_label(accent_color, "■ Accent");
            });

            ui.label(format!("Font: {}", skin.fonts.proportional));
        });
    });
}

/// Renders a skin selector dialog.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `skin_manager` - The skin manager instance
///
/// # Returns
/// `true` if a skin was selected, `false` otherwise
pub fn skin_selector_dialog(ui: &mut Ui, skin_manager: &mut SkinManager) -> bool {
    let mut skin_changed = false;

    // Collect skin data before rendering to avoid borrow checker issues
    let skins_data: Vec<_> = skin_manager
        .available_skins
        .iter()
        .enumerate()
        .map(|(index, skin)| {
            (
                index,
                skin.metadata.name.clone(),
                skin.metadata.description.clone(),
                skin.metadata.author.clone(),
                index == skin_manager.active_skin_index,
            )
        })
        .collect();

    ui.label("Available Skins:");
    ui.separator();

    for (index, name, description, author, is_active) in skins_data {
        ui.group(|ui| {
            ui.vertical(|ui| {
                let label = if is_active {
                    RichText::new(&name)
                        .strong()
                        .color(egui::Color32::from_rgb(0, 212, 255))
                } else {
                    RichText::new(&name).strong()
                };

                ui.label(label);
                ui.label(
                    RichText::new(&description)
                        .small()
                        .color(egui::Color32::GRAY),
                );
                ui.label(
                    RichText::new(format!("by {}", author))
                        .small()
                        .color(egui::Color32::DARK_GRAY),
                );

                ui.horizontal(|ui| {
                    if !is_active && ui.button("Select").clicked() {
                        if skin_manager.set_active_skin(index) {
                            skin_changed = true;
                        }
                    }

                    if is_active {
                        ui.label(
                            RichText::new("✓ Active").color(egui::Color32::from_rgb(0, 212, 255)),
                        );
                    }
                });
            });
        });
    }

    skin_changed
}

/// Helper function to parse hex color strings.
fn parse_hex_color(hex: &str) -> Option<egui::Color32> {
    if !hex.starts_with('#') {
        return None;
    }

    let hex_part = &hex[1..];
    let (r, g, b) = match hex_part.len() {
        6 => {
            let r = u8::from_str_radix(&hex_part[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex_part[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex_part[4..6], 16).ok()?;
            (r, g, b)
        }
        _ => return None,
    };

    Some(egui::Color32::from_rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color_valid() {
        let color = parse_hex_color("#ffffff");
        assert_eq!(color, Some(egui::Color32::WHITE));
    }

    #[test]
    fn test_parse_hex_color_invalid() {
        assert_eq!(parse_hex_color("ffffff"), None);
        assert_eq!(parse_hex_color("#gg"), None);
    }
}
