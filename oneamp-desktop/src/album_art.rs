use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Ui};
use eframe::egui::{Color32, Painter, Pos2, Rect, Vec2};
use crate::visual_effects::VisualEffects;
use crate::theme::Theme;
use std::path::Path;
use std::sync::Arc;

/// Album art display with reflection effect
pub struct AlbumArtDisplay {
    texture: Option<TextureHandle>,
    image_data: Option<Arc<ColorImage>>,
    last_track_path: Option<String>,
}

impl AlbumArtDisplay {
    pub fn new() -> Self {
        Self {
            texture: None,
            image_data: None,
            last_track_path: None,
        }
    }
    
    /// Load album art from a track file
    pub fn load_from_track(&mut self, track_path: &Path, ctx: &egui::Context) {
        let path_str = track_path.to_string_lossy().to_string();
        
        // Skip if already loaded for this track
        if self.last_track_path.as_ref() == Some(&path_str) {
            return;
        }
        
        self.last_track_path = Some(path_str);
        
        // Try to extract album art using lofty
        match extract_album_art(track_path) {
            Some(image_data) => {
                self.image_data = Some(Arc::new(image_data.clone()));
                self.texture = Some(ctx.load_texture(
                    "album_art",
                    image_data,
                    TextureOptions::LINEAR,
                ));
            }
            None => {
                // No album art found, clear texture
                self.texture = None;
                self.image_data = None;
            }
        }
    }
    
    /// Render the album art with reflection effect
    pub fn render(&self, ui: &mut Ui, theme: &Theme, size: f32) {
        if let Some(texture) = &self.texture {
            let (rect, _response) = ui.allocate_exact_size(
                Vec2::new(size, size * 1.3), // Extra space for reflection
                egui::Sense::hover(),
            );
            
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                
                // Main album art
                let art_rect = Rect::from_min_size(
                    rect.min,
                    Vec2::splat(size),
                );
                
                // Shadow
                VisualEffects::drop_shadow(
                    painter,
                    art_rect,
                    8.0,
                    Vec2::new(0.0, 4.0),
                    4.0,
                    Color32::from_black_alpha(150),
                );
                
                // Album art image
                ui.painter().image(
                    texture.id(),
                    art_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                
                // Border
                painter.rect_stroke(
                    art_rect,
                    4.0,
                    egui::Stroke::new(1.0, Theme::color32(&theme.colors.border)),
                );
                
                // Reflection effect
                let reflection_height = size * 0.3;
                let reflection_rect = Rect::from_min_size(
                    Pos2::new(rect.min.x, rect.min.y + size),
                    Vec2::new(size, reflection_height),
                );
                
                // Draw reflection (flipped vertically with fade)
                draw_reflection(painter, texture, reflection_rect);
            }
        } else {
            // No album art, show placeholder
            render_placeholder(ui, theme, size);
        }
    }
    
    /// Check if album art is available
    pub fn has_art(&self) -> bool {
        self.texture.is_some()
    }
}

impl Default for AlbumArtDisplay {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract album art from audio file using lofty
fn extract_album_art(path: &Path) -> Option<ColorImage> {
    use lofty::probe::Probe;
    use lofty::picture::Picture;
    use lofty::file::TaggedFileExt;
    
    // Read the audio file
    let tagged_file = Probe::open(path)
        .ok()?
        .read()
        .ok()?;
    
    // Try to get the first picture
    let picture: &Picture = tagged_file
        .primary_tag()
        .and_then(|tag| tag.pictures().first())
        .or_else(|| {
            tagged_file
                .tags()
                .iter()
                .find_map(|tag| tag.pictures().first())
        })?;
    
    // Decode image data
    let image_data = picture.data();
    
    // Try to decode with image crate
    let img = image::load_from_memory(image_data).ok()?;
    let rgba = img.to_rgba8();
    
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels: Vec<Color32> = rgba
        .pixels()
        .map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    
    Some(ColorImage { size, pixels })
}

/// Draw reflection effect for album art
fn draw_reflection(painter: &Painter, texture: &TextureHandle, rect: Rect) {
    let steps = 10;
    let step_height = rect.height() / steps as f32;
    
    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let alpha = (1.0 - t) * 0.4; // Fade from 40% to 0%
        
        let step_rect = Rect::from_min_size(
            Pos2::new(rect.min.x, rect.min.y + i as f32 * step_height),
            Vec2::new(rect.width(), step_height),
        );
        
        // UV coordinates (flipped vertically)
        let uv_top = t;
        let uv_bottom = ((i + 1) as f32 / steps as f32).min(1.0);
        
        painter.image(
            texture.id(),
            step_rect,
            egui::Rect::from_min_max(
                egui::pos2(0.0, uv_top),
                egui::pos2(1.0, uv_bottom),
            ),
            Color32::from_white_alpha((alpha * 255.0) as u8),
        );
    }
}

/// Render placeholder when no album art is available
fn render_placeholder(ui: &mut Ui, theme: &Theme, size: f32) {
    let (rect, _response) = ui.allocate_exact_size(
        Vec2::splat(size),
        egui::Sense::hover(),
    );
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        
        // Background with gradient
        VisualEffects::gradient_rect_vertical(
            painter,
            rect,
            Theme::color32(&theme.colors.panel_bg).linear_multiply(1.2),
            Theme::color32(&theme.colors.panel_bg).linear_multiply(0.8),
            4.0,
        );
        
        // Border
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, Theme::color32(&theme.colors.border)),
        );
        
        // Music note icon (simple)
        let center = rect.center();
        let icon_size = size * 0.4;
        
        // Draw a simple music note
        let note_color = Theme::color32(&theme.colors.display_text).linear_multiply(0.3);
        
        // Note stem
        painter.line_segment(
            [
                Pos2::new(center.x, center.y + icon_size * 0.3),
                Pos2::new(center.x, center.y - icon_size * 0.3),
            ],
            egui::Stroke::new(3.0, note_color),
        );
        
        // Note head (circle)
        painter.circle_filled(
            Pos2::new(center.x, center.y + icon_size * 0.3),
            icon_size * 0.15,
            note_color,
        );
        
        // Note flag
        painter.line_segment(
            [
                Pos2::new(center.x, center.y - icon_size * 0.3),
                Pos2::new(center.x + icon_size * 0.2, center.y - icon_size * 0.1),
            ],
            egui::Stroke::new(3.0, note_color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_album_art_display_creation() {
        let display = AlbumArtDisplay::new();
        assert!(!display.has_art());
        assert!(display.last_track_path.is_none());
    }
    
    #[test]
    fn test_album_art_display_default() {
        let display = AlbumArtDisplay::default();
        assert!(!display.has_art());
    }
}
