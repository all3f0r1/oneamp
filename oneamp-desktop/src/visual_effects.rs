use eframe::egui::{self, Color32, Painter, Pos2, Rect, Rounding, Shape, Stroke, Vec2};

/// Visual effects utilities for advanced UI rendering
pub struct VisualEffects;

impl VisualEffects {
    /// Draw a drop shadow behind a rectangle
    pub fn drop_shadow(
        painter: &Painter,
        rect: Rect,
        rounding: impl Into<Rounding>,
        offset: Vec2,
        blur_radius: f32,
        color: Color32,
    ) {
        let rounding = rounding.into();
        let steps = (blur_radius / 2.0).ceil() as usize;
        
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let expansion = blur_radius * (1.0 - t);
            let alpha = (color.a() as f32 * (1.0 - t)) as u8;
            
            let shadow_rect = rect
                .translate(offset)
                .expand(expansion);
            
            painter.rect_filled(
                shadow_rect,
                rounding,
                Color32::from_rgba_premultiplied(
                    color.r(),
                    color.g(),
                    color.b(),
                    alpha,
                ),
            );
        }
    }
    
    /// Draw a glow effect around a rectangle
    pub fn glow(
        painter: &Painter,
        rect: Rect,
        rounding: impl Into<Rounding>,
        glow_size: f32,
        color: Color32,
    ) {
        let rounding = rounding.into();
        let steps = (glow_size / 2.0).ceil() as usize;
        
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let expansion = glow_size * t;
            let alpha = (color.a() as f32 * (1.0 - t * t)) as u8; // Quadratic falloff
            
            painter.rect_stroke(
                rect.expand(expansion),
                rounding,
                Stroke::new(
                    1.0,
                    Color32::from_rgba_premultiplied(
                        color.r(),
                        color.g(),
                        color.b(),
                        alpha,
                    ),
                ),
            );
        }
    }
    
    /// Draw a vertical gradient rectangle
    pub fn gradient_rect_vertical(
        painter: &Painter,
        rect: Rect,
        top_color: Color32,
        bottom_color: Color32,
        rounding: impl Into<Rounding>,
    ) {
        let rounding = rounding.into();
        
        // Create mesh for gradient
        let mut mesh = egui::Mesh::default();
        
        let top_left = rect.left_top();
        let top_right = rect.right_top();
        let bottom_left = rect.left_bottom();
        let bottom_right = rect.right_bottom();
        
        // Add vertices with colors
        mesh.colored_vertex(top_left, top_color);
        mesh.colored_vertex(top_right, top_color);
        mesh.colored_vertex(bottom_right, bottom_color);
        mesh.colored_vertex(bottom_left, bottom_color);
        
        // Add triangles
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        
        painter.add(Shape::mesh(mesh));
        
        // Add border if rounded
        if rounding != Rounding::ZERO {
            painter.rect_stroke(
                rect,
                rounding,
                Stroke::new(1.0, top_color.linear_multiply(0.5)),
            );
        }
    }
    
    /// Draw a horizontal gradient rectangle
    pub fn gradient_rect_horizontal(
        painter: &Painter,
        rect: Rect,
        left_color: Color32,
        right_color: Color32,
        rounding: impl Into<Rounding>,
    ) {
        let rounding = rounding.into();
        
        let mut mesh = egui::Mesh::default();
        
        let top_left = rect.left_top();
        let top_right = rect.right_top();
        let bottom_left = rect.left_bottom();
        let bottom_right = rect.right_bottom();
        
        mesh.colored_vertex(top_left, left_color);
        mesh.colored_vertex(top_right, right_color);
        mesh.colored_vertex(bottom_right, right_color);
        mesh.colored_vertex(bottom_left, left_color);
        
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        
        painter.add(Shape::mesh(mesh));
        
        if rounding != Rounding::ZERO {
            painter.rect_stroke(
                rect,
                rounding,
                Stroke::new(1.0, left_color.linear_multiply(0.5)),
            );
        }
    }
    
    /// Draw a 3D button with bevel effect
    pub fn button_3d(
        painter: &Painter,
        rect: Rect,
        base_color: Color32,
        pressed: bool,
        rounding: impl Into<Rounding>,
    ) {
        let rounding = rounding.into();
        
        if pressed {
            // Pressed state: darker, shadow inside
            let dark_color = base_color.linear_multiply(0.7);
            painter.rect_filled(rect, rounding, dark_color);
            
            // Inner shadow (top-left)
            painter.line_segment(
                [rect.left_top(), rect.right_top()],
                Stroke::new(1.0, Color32::from_black_alpha(80)),
            );
            painter.line_segment(
                [rect.left_top(), rect.left_bottom()],
                Stroke::new(1.0, Color32::from_black_alpha(80)),
            );
        } else {
            // Normal state: gradient with highlight
            let top_color = base_color.linear_multiply(1.2);
            let bottom_color = base_color.linear_multiply(0.8);
            
            Self::gradient_rect_vertical(painter, rect, top_color, bottom_color, rounding);
            
            // Highlight on top edge
            painter.line_segment(
                [rect.left_top(), rect.right_top()],
                Stroke::new(1.0, Color32::from_white_alpha(60)),
            );
            
            // Shadow on bottom edge
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(1.0, Color32::from_black_alpha(60)),
            );
        }
    }
    
    /// Draw text with shadow for better readability
    pub fn text_with_shadow(
        painter: &Painter,
        pos: Pos2,
        anchor: egui::Align2,
        text: impl Into<String>,
        font_id: egui::FontId,
        text_color: Color32,
        shadow_color: Color32,
        shadow_offset: Vec2,
    ) {
        let text = text.into();
        
        // Draw shadow
        painter.text(
            pos + shadow_offset,
            anchor,
            &text,
            font_id.clone(),
            shadow_color,
        );
        
        // Draw text
        painter.text(pos, anchor, text, font_id, text_color);
    }
    
    /// Draw LCD-style text with glow effect
    pub fn lcd_text(
        painter: &Painter,
        pos: Pos2,
        anchor: egui::Align2,
        text: impl Into<String>,
        font_id: egui::FontId,
        color: Color32,
    ) {
        let text = text.into();
        
        // Glow layers
        for i in 0..3 {
            let offset = Vec2::new(i as f32 * 0.3, 0.0);
            let alpha = (color.a() as f32 * 0.3) as u8;
            painter.text(
                pos + offset,
                anchor,
                &text,
                font_id.clone(),
                Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), alpha),
            );
        }
        
        // Main text
        painter.text(pos, anchor, text, font_id, color);
    }
    
    /// Draw a metallic panel with reflections
    pub fn metallic_panel(
        painter: &Painter,
        rect: Rect,
        base_color: Color32,
        rounding: impl Into<Rounding>,
    ) {
        let rounding = rounding.into();
        
        // Base gradient (dark -> light -> dark)
        let height = rect.height();
        let top_third = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x, rect.min.y + height / 3.0),
        );
        let middle_third = Rect::from_min_max(
            Pos2::new(rect.min.x, rect.min.y + height / 3.0),
            Pos2::new(rect.max.x, rect.min.y + 2.0 * height / 3.0),
        );
        let bottom_third = Rect::from_min_max(
            Pos2::new(rect.min.x, rect.min.y + 2.0 * height / 3.0),
            rect.max,
        );
        
        // Top: dark to light
        Self::gradient_rect_vertical(
            painter,
            top_third,
            base_color.linear_multiply(0.7),
            base_color.linear_multiply(1.3),
            Rounding::ZERO,
        );
        
        // Middle: light
        painter.rect_filled(middle_third, Rounding::ZERO, base_color.linear_multiply(1.3));
        
        // Bottom: light to dark
        Self::gradient_rect_vertical(
            painter,
            bottom_third,
            base_color.linear_multiply(1.3),
            base_color.linear_multiply(0.7),
            Rounding::ZERO,
        );
        
        // Border
        painter.rect_stroke(rect, rounding, Stroke::new(1.0, base_color.linear_multiply(0.5)));
    }
    
    /// Draw a glass/acrylic panel
    pub fn glass_panel(
        painter: &Painter,
        rect: Rect,
        base_color: Color32,
        rounding: impl Into<Rounding>,
    ) {
        let rounding = rounding.into();
        
        // Semi-transparent background
        painter.rect_filled(rect, rounding, base_color);
        
        // Top highlight
        let highlight_height = rect.height() * 0.3;
        let highlight_rect = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x, rect.min.y + highlight_height),
        );
        
        Self::gradient_rect_vertical(
            painter,
            highlight_rect,
            Color32::from_white_alpha(30),
            Color32::from_white_alpha(0),
            Rounding::ZERO,
        );
        
        // Border
        painter.rect_stroke(
            rect,
            rounding,
            Stroke::new(1.0, Color32::from_white_alpha(40)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_visual_effects_module_exists() {
        // Basic smoke test
        let _effects = VisualEffects;
    }
}
