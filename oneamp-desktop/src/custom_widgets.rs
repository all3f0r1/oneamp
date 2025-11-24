use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use crate::visual_effects::VisualEffects;
use crate::theme::Theme;

/// Custom 3D button with visual effects
pub fn button_3d(
    ui: &mut Ui,
    theme: &Theme,
    text: &str,
    icon: Option<&str>,
) -> Response {
    let desired_size = Vec2::new(80.0, 32.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let visuals = ui.style().interact(&response);
        
        // Determine button color based on state
        let base_color = if response.clicked() {
            Theme::color32(&theme.colors.button_active)
        } else if response.hovered() {
            Theme::color32(&theme.colors.button_hovered)
        } else {
            Theme::color32(&theme.colors.button_normal)
        };
        
        // Drop shadow
        if !response.clicked() {
            VisualEffects::drop_shadow(
                painter,
                rect,
                4.0,
                Vec2::new(0.0, 2.0),
                4.0,
                Color32::from_black_alpha(100),
            );
        }
        
        // 3D button effect
        VisualEffects::button_3d(
            painter,
            rect,
            base_color,
            response.clicked(),
            4.0,
        );
        
        // Glow on hover
        if response.hovered() {
            VisualEffects::glow(
                painter,
                rect,
                4.0,
                6.0,
                Theme::color32(&theme.colors.display_accent).linear_multiply(0.5),
            );
        }
        
        // Text with icon
        let text_pos = if let Some(icon_str) = icon {
            // Icon + text
            let icon_pos = rect.center() - Vec2::new(20.0, 0.0);
            VisualEffects::text_with_shadow(
                painter,
                icon_pos,
                egui::Align2::CENTER_CENTER,
                icon_str,
                egui::FontId::proportional(16.0),
                visuals.text_color(),
                Color32::from_black_alpha(100),
                Vec2::new(1.0, 1.0),
            );
            rect.center() + Vec2::new(10.0, 0.0)
        } else {
            rect.center()
        };
        
        VisualEffects::text_with_shadow(
            painter,
            text_pos,
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(theme.fonts.button_size),
            visuals.text_color(),
            Color32::from_black_alpha(100),
            Vec2::new(1.0, 1.0),
        );
        
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    }
    
    response
}

/// Custom progress bar with animated shine effect
pub fn progress_bar_fancy(
    ui: &mut Ui,
    theme: &Theme,
    progress: f32,
    time: f32,
) -> Response {
    let desired_size = Vec2::new(ui.available_width(), 24.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        
        // Background track with inset shadow
        VisualEffects::drop_shadow(
            painter,
            rect.shrink(1.0),
            4.0,
            Vec2::new(0.0, 1.0),
            2.0,
            Color32::from_black_alpha(80),
        );
        
        painter.rect_filled(
            rect,
            4.0,
            Theme::color32(&theme.colors.progress_bg),
        );
        
        // Progress fill
        if progress > 0.0 {
            let fill_width = rect.width() * progress.clamp(0.0, 1.0);
            let fill_rect = Rect::from_min_size(
                rect.min,
                Vec2::new(fill_width, rect.height()),
            );
            
            // Gradient fill
            VisualEffects::gradient_rect_vertical(
                painter,
                fill_rect,
                Theme::color32(&theme.colors.progress_fill).linear_multiply(1.2),
                Theme::color32(&theme.colors.progress_fill).linear_multiply(0.8),
                4.0,
            );
            
            // Animated shine effect
            let shine_pos = (time * 0.5).fract();
            let shine_x = fill_rect.left() + fill_rect.width() * shine_pos;
            let shine_width = 20.0;
            
            if shine_x > fill_rect.left() && shine_x < fill_rect.right() {
                let shine_rect = Rect::from_min_max(
                    Pos2::new((shine_x - shine_width / 2.0).max(fill_rect.left()), fill_rect.top()),
                    Pos2::new((shine_x + shine_width / 2.0).min(fill_rect.right()), fill_rect.bottom()),
                );
                
                VisualEffects::gradient_rect_horizontal(
                    painter,
                    shine_rect,
                    Color32::from_white_alpha(0),
                    Color32::from_white_alpha(60),
                    0.0,
                );
                VisualEffects::gradient_rect_horizontal(
                    painter,
                    shine_rect,
                    Color32::from_white_alpha(60),
                    Color32::from_white_alpha(0),
                    0.0,
                );
            }
            
            // Highlight on top
            painter.line_segment(
                [fill_rect.left_top(), fill_rect.right_top()],
                Stroke::new(1.0, Color32::from_white_alpha(40)),
            );
        }
        
        // Border
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, Theme::color32(&theme.colors.border)),
        );
    }
    
    response
}

/// Custom slider with 3D thumb
pub fn slider_3d(
    ui: &mut Ui,
    theme: &Theme,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> Response {
    let desired_size = Vec2::new(ui.available_width().min(200.0), 24.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        
        // Track
        let track_rect = Rect::from_center_size(
            rect.center(),
            Vec2::new(rect.width(), 6.0),
        );
        
        painter.rect_filled(
            track_rect,
            3.0,
            Theme::color32(&theme.colors.progress_bg),
        );
        
        // Handle drag
        if response.dragged() {
            if let Some(pointer_pos) = response.interact_pointer_pos() {
                let normalized = ((pointer_pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                *value = range.start() + normalized * (range.end() - range.start());
                response.mark_changed();
            }
        }
        
        // Thumb position
        let normalized_value = (*value - range.start()) / (range.end() - range.start());
        let thumb_x = rect.left() + rect.width() * normalized_value.clamp(0.0, 1.0);
        let thumb_center = Pos2::new(thumb_x, rect.center().y);
        let thumb_radius = 10.0;
        let thumb_rect = Rect::from_center_size(
            thumb_center,
            Vec2::splat(thumb_radius * 2.0),
        );
        
        // Thumb shadow
        VisualEffects::drop_shadow(
            painter,
            thumb_rect,
            thumb_radius,
            Vec2::new(0.0, 2.0),
            4.0,
            Color32::from_black_alpha(100),
        );
        
        // Thumb 3D effect
        let thumb_color = if response.dragged() {
            Theme::color32(&theme.colors.button_active)
        } else if response.hovered() {
            Theme::color32(&theme.colors.button_hovered)
        } else {
            Theme::color32(&theme.colors.button_normal)
        };
        
        VisualEffects::button_3d(
            painter,
            thumb_rect,
            thumb_color,
            response.dragged(),
            thumb_radius,
        );
        
        response.widget_info(|| egui::WidgetInfo::slider(true, *value as f64, ""));
    }
    
    response
}

/// LCD-style digital display
pub fn lcd_display(
    ui: &mut Ui,
    theme: &Theme,
    text: &str,
    large: bool,
) -> Response {
    let font_size = if large { theme.fonts.timer_size } else { theme.fonts.track_info_size };
    let desired_size = Vec2::new(
        ui.available_width().min(300.0),
        font_size + 16.0,
    );
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        
        // LCD background
        painter.rect_filled(
            rect,
            4.0,
            Theme::color32(&theme.colors.display_bg),
        );
        
        // Inset shadow
        painter.rect_stroke(
            rect.shrink(1.0),
            4.0,
            Stroke::new(1.0, Color32::from_black_alpha(100)),
        );
        
        // LCD text with glow
        VisualEffects::lcd_text(
            painter,
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::monospace(font_size),
            Theme::color32(&theme.colors.display_text),
        );
    }
    
    response
}

/// Metallic panel container
pub fn metallic_panel(
    ui: &mut Ui,
    theme: &Theme,
    content: impl FnOnce(&mut Ui),
) {
    let available = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(available, Sense::hover());
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        
        VisualEffects::metallic_panel(
            painter,
            rect,
            Theme::color32(&theme.colors.panel_bg),
            6.0,
        );
    }
    
    let inner_rect = rect.shrink(8.0);
    ui.allocate_ui_at_rect(inner_rect, content);
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_custom_widgets_module_exists() {
        // Smoke test
    }
}
