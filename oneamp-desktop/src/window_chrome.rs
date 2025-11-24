use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Ui, Vec2};
use crate::visual_effects::VisualEffects;
use crate::theme::Theme;

/// Custom window chrome (title bar) for frameless window
pub struct WindowChrome {
    dragging: bool,
}

impl WindowChrome {
    pub fn new() -> Self {
        Self {
            dragging: false,
        }
    }
    
    /// Render the custom title bar
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        theme: &Theme,
        title: &str,
    ) -> WindowAction {
        let mut action = WindowAction::None;
        
        egui::TopBottomPanel::top("title_bar")
            .exact_height(32.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let panel_rect = ui.max_rect();
                
                // Background with gradient
                let painter = ui.painter();
                VisualEffects::gradient_rect_vertical(
                    painter,
                    panel_rect,
                    Theme::color32(&theme.colors.panel_bg).linear_multiply(1.3),
                    Theme::color32(&theme.colors.panel_bg).linear_multiply(0.9),
                    0.0,
                );
                
                // Bottom border
                painter.line_segment(
                    [
                        Pos2::new(panel_rect.left(), panel_rect.bottom()),
                        Pos2::new(panel_rect.right(), panel_rect.bottom()),
                    ],
                    egui::Stroke::new(1.0, Theme::color32(&theme.colors.border)),
                );
                
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    
                    // App icon
                    ui.label(
                        egui::RichText::new("🎵")
                            .size(16.0)
                    );
                    
                    ui.add_space(4.0);
                    
                    // Title
                    ui.label(
                        egui::RichText::new(title)
                            .size(12.0)
                            .color(Theme::color32(&theme.colors.display_text))
                    );
                    
                    // Spacer
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Close button
                        if window_button(ui, theme, "×", WindowButtonType::Close).clicked() {
                            action = WindowAction::Close;
                        }
                        
                        // Maximize button
                        if window_button(ui, theme, "□", WindowButtonType::Maximize).clicked() {
                            action = WindowAction::ToggleMaximize;
                        }
                        
                        // Minimize button
                        if window_button(ui, theme, "−", WindowButtonType::Minimize).clicked() {
                            action = WindowAction::Minimize;
                        }
                    });
                });
                
                // Drag area (entire title bar except buttons)
                let drag_rect = Rect::from_min_max(
                    panel_rect.min,
                    Pos2::new(panel_rect.right() - 96.0, panel_rect.bottom()),
                );
                
                let drag_response = ui.interact(
                    drag_rect,
                    ui.id().with("drag_area"),
                    Sense::click_and_drag(),
                );
                
                if drag_response.dragged() {
                    action = WindowAction::StartDrag;
                }
                
                // Double-click to maximize
                if drag_response.double_clicked() {
                    action = WindowAction::ToggleMaximize;
                }
            });
        
        action
    }
}

impl Default for WindowChrome {
    fn default() -> Self {
        Self::new()
    }
}

/// Window button types
#[derive(Debug, Clone, Copy, PartialEq)]
enum WindowButtonType {
    Minimize,
    Maximize,
    Close,
}

/// Render a window control button (minimize, maximize, close)
fn window_button(
    ui: &mut Ui,
    theme: &Theme,
    text: &str,
    button_type: WindowButtonType,
) -> Response {
    let button_size = Vec2::new(32.0, 28.0);
    
    let (rect, response) = ui.allocate_exact_size(
        button_size,
        Sense::click(),
    );
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        
        // Background color
        let bg_color = if response.is_pointer_button_down_on() {
            match button_type {
                WindowButtonType::Close => Color32::from_rgb(180, 40, 40),
                _ => Theme::color32(&theme.colors.button_active),
            }
        } else if response.hovered() {
            match button_type {
                WindowButtonType::Close => Color32::from_rgb(220, 50, 50),
                _ => Theme::color32(&theme.colors.button_hovered),
            }
        } else {
            Color32::TRANSPARENT
        };
        
        // Background
        if bg_color != Color32::TRANSPARENT {
            painter.rect_filled(rect, 0.0, bg_color);
        }
        
        // Text
        let text_color = if response.hovered() {
            Color32::WHITE
        } else {
            Theme::color32(&theme.colors.display_text)
        };
        
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            text_color,
        );
        
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{:?}", button_type)));
    }
    
    response
}

/// Actions that can be triggered by window chrome
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowAction {
    None,
    Close,
    Minimize,
    ToggleMaximize,
    StartDrag,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_window_chrome_creation() {
        let chrome = WindowChrome::new();
        assert!(!chrome.dragging);
    }
    
    #[test]
    fn test_window_chrome_default() {
        let chrome = WindowChrome::default();
        assert!(!chrome.dragging);
    }
    
    #[test]
    fn test_window_action_types() {
        let actions = vec![
            WindowAction::None,
            WindowAction::Close,
            WindowAction::Minimize,
            WindowAction::ToggleMaximize,
            WindowAction::StartDrag,
        ];
        
        assert_eq!(actions.len(), 5);
    }
}
