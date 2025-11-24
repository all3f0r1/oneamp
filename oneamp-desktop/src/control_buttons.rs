use eframe::egui::{self, Color32, Painter, Pos2, Response, Sense, Shape, Stroke, Ui, Vec2};
use crate::visual_effects::VisualEffects;
use crate::theme::Theme;
use std::f32::consts::PI;

/// Button icons for media controls
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ButtonIcon {
    Play,
    Pause,
    Stop,
    Previous,
    Next,
}

impl ButtonIcon {
    /// Draw the icon at the specified center position
    pub fn draw(&self, painter: &Painter, center: Pos2, size: f32, color: Color32) {
        match self {
            ButtonIcon::Play => {
                // Triangle pointing right
                let offset = size * 0.1; // Slight offset to center visually
                let points = vec![
                    center + Vec2::new(-size/3.0 + offset, -size/2.0),
                    center + Vec2::new(-size/3.0 + offset, size/2.0),
                    center + Vec2::new(size/2.0 + offset, 0.0),
                ];
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
            }
            ButtonIcon::Pause => {
                // Two vertical bars
                let bar_width = size / 5.0;
                let bar_height = size * 0.8;
                let spacing = size / 6.0;
                
                // Left bar
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        center + Vec2::new(-spacing, 0.0),
                        Vec2::new(bar_width, bar_height),
                    ),
                    2.0,
                    color,
                );
                
                // Right bar
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        center + Vec2::new(spacing, 0.0),
                        Vec2::new(bar_width, bar_height),
                    ),
                    2.0,
                    color,
                );
            }
            ButtonIcon::Stop => {
                // Square
                let square_size = size * 0.6;
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        center,
                        Vec2::splat(square_size),
                    ),
                    2.0,
                    color,
                );
            }
            ButtonIcon::Previous => {
                // Bar + triangle pointing left
                let bar_width = size / 8.0;
                let bar_height = size * 0.7;
                
                // Bar on left
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        center + Vec2::new(-size/3.0, 0.0),
                        Vec2::new(bar_width, bar_height),
                    ),
                    1.0,
                    color,
                );
                
                // Triangle
                let points = vec![
                    center + Vec2::new(size/3.0, -size/2.5),
                    center + Vec2::new(size/3.0, size/2.5),
                    center + Vec2::new(-size/6.0, 0.0),
                ];
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
            }
            ButtonIcon::Next => {
                // Triangle + bar pointing right
                let bar_width = size / 8.0;
                let bar_height = size * 0.7;
                
                // Triangle
                let points = vec![
                    center + Vec2::new(-size/3.0, -size/2.5),
                    center + Vec2::new(-size/3.0, size/2.5),
                    center + Vec2::new(size/6.0, 0.0),
                ];
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
                
                // Bar on right
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        center + Vec2::new(size/3.0, 0.0),
                        Vec2::new(bar_width, bar_height),
                    ),
                    1.0,
                    color,
                );
            }
        }
    }
}

/// Render a circular control button with 3D effect
pub fn control_button(
    ui: &mut Ui,
    theme: &Theme,
    icon: ButtonIcon,
    active: bool,
    size: f32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(size),
        Sense::click(),
    );
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let center = rect.center();
        let radius = size / 2.0 - 2.0;
        
        // Shadow (unless pressed)
        if !response.clicked() {
            for i in 0..3 {
                let shadow_radius = radius + 1.0;
                let shadow_alpha = (60 - i * 15).max(10) as u8;
                painter.circle_filled(
                    center + Vec2::new(1.0 + i as f32 * 0.5, 2.0 + i as f32 * 0.5),
                    shadow_radius,
                    Color32::from_black_alpha(shadow_alpha),
                );
            }
        }
        
        // Button body color
        let button_color = if response.is_pointer_button_down_on() {
            Theme::color32(&theme.colors.button_active)
        } else if response.hovered() {
            Theme::color32(&theme.colors.button_hovered)
        } else {
            Theme::color32(&theme.colors.button_normal)
        };
        
        // Circular gradient (top lighter, bottom darker)
        draw_circular_gradient(
            painter,
            center,
            radius,
            button_color.linear_multiply(1.3),
            button_color.linear_multiply(0.7),
        );
        
        // Glow if active or hovered
        if active || response.hovered() {
            let glow_color = Theme::color32(&theme.colors.display_accent);
            for i in 0..4 {
                let glow_radius = radius + 2.0 + i as f32 * 2.0;
                let alpha = (80 - i * 20).max(10) as u8;
                painter.circle_stroke(
                    center,
                    glow_radius,
                    Stroke::new(
                        2.0,
                        Color32::from_rgba_premultiplied(
                            glow_color.r(),
                            glow_color.g(),
                            glow_color.b(),
                            alpha,
                        ),
                    ),
                );
            }
        }
        
        // Highlight on top
        painter.circle_filled(
            center + Vec2::new(0.0, -radius * 0.3),
            radius * 0.4,
            Color32::from_white_alpha(40),
        );
        
        // Icon
        let icon_color = if active {
            Theme::color32(&theme.colors.display_accent)
        } else {
            Theme::color32(&theme.colors.display_text)
        };
        
        icon.draw(painter, center, size * 0.35, icon_color);
        
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{:?}", icon)));
    }
    
    response
}

/// Draw a circular gradient from top to bottom
fn draw_circular_gradient(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    top_color: Color32,
    bottom_color: Color32,
) {
    // Draw multiple horizontal slices to simulate gradient
    let slices = 32;
    for i in 0..slices {
        let t = i as f32 / slices as f32;
        let next_t = (i + 1) as f32 / slices as f32;
        
        // Interpolate color
        let color = Color32::from_rgb(
            (top_color.r() as f32 * (1.0 - t) + bottom_color.r() as f32 * t) as u8,
            (top_color.g() as f32 * (1.0 - t) + bottom_color.g() as f32 * t) as u8,
            (top_color.b() as f32 * (1.0 - t) + bottom_color.b() as f32 * t) as u8,
        );
        
        // Calculate y positions
        let y1 = center.y - radius + t * radius * 2.0;
        let y2 = center.y - radius + next_t * radius * 2.0;
        
        // Calculate width at this height (circle equation)
        let dy1 = (y1 - center.y).abs();
        let dy2 = (y2 - center.y).abs();
        
        if dy1 < radius && dy2 < radius {
            let width1 = (radius * radius - dy1 * dy1).sqrt() * 2.0;
            let width2 = (radius * radius - dy2 * dy2).sqrt() * 2.0;
            
            // Draw trapezoid
            let points = vec![
                Pos2::new(center.x - width1 / 2.0, y1),
                Pos2::new(center.x + width1 / 2.0, y1),
                Pos2::new(center.x + width2 / 2.0, y2),
                Pos2::new(center.x - width2 / 2.0, y2),
            ];
            
            painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
        }
    }
    
    // Outer circle stroke for clean edge
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(1.0, Color32::from_black_alpha(100)),
    );
}

/// Render a row of control buttons
pub fn control_button_row(
    ui: &mut Ui,
    theme: &Theme,
    is_playing: bool,
    is_paused: bool,
) -> ControlAction {
    let mut action = ControlAction::None;
    
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        
        // Previous
        if control_button(ui, theme, ButtonIcon::Previous, false, 40.0).clicked() {
            action = ControlAction::Previous;
        }
        
        ui.add_space(4.0);
        
        // Play/Pause
        let play_pause_icon = if is_playing && !is_paused {
            ButtonIcon::Pause
        } else {
            ButtonIcon::Play
        };
        
        if control_button(ui, theme, play_pause_icon, is_playing, 48.0).clicked() {
            action = if is_playing && !is_paused {
                ControlAction::Pause
            } else {
                ControlAction::Play
            };
        }
        
        ui.add_space(4.0);
        
        // Stop
        if control_button(ui, theme, ButtonIcon::Stop, false, 40.0).clicked() {
            action = ControlAction::Stop;
        }
        
        ui.add_space(4.0);
        
        // Next
        if control_button(ui, theme, ButtonIcon::Next, false, 40.0).clicked() {
            action = ControlAction::Next;
        }
        
        ui.add_space(8.0);
    });
    
    action
}

/// Actions that can be triggered by control buttons
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlAction {
    None,
    Play,
    Pause,
    Stop,
    Previous,
    Next,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_button_icon_types() {
        let icons = vec![
            ButtonIcon::Play,
            ButtonIcon::Pause,
            ButtonIcon::Stop,
            ButtonIcon::Previous,
            ButtonIcon::Next,
        ];
        
        assert_eq!(icons.len(), 5);
    }
    
    #[test]
    fn test_control_action() {
        let action = ControlAction::Play;
        assert_eq!(action, ControlAction::Play);
        assert_ne!(action, ControlAction::Pause);
    }
}
