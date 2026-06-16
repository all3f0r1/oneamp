use egui::{Color32, Pos2, Vec2};

use super::super::renderer::WszRenderer;
use super::bitmap_font::{self, GLYPH_H, GLYPH_W};

/// Inserted between every repetition of the title during scrolling. Matches
/// Winamp's classic "TITLE (M:SS) *** " loop format.
const SEPARATOR: &str = " *** ";

pub struct TitleScroller {
    position: (u32, u32),
    width: u32,
    /// One repetition of the displayed string, including trailing separator.
    /// We tile this horizontally so the scroll loops seamlessly.
    text: String,
    /// Pixels of the text already passed off-screen to the left. Grows
    /// monotonically; we modulo by `text.len() * GLYPH_W` at render time.
    scroll_position: f32,
    scroll_speed: f32,
    last_update: std::time::Instant,
}

impl TitleScroller {
    pub fn new() -> Self {
        Self {
            position: (111, 27),
            width: 153,
            text: String::new(),
            scroll_position: 0.0,
            scroll_speed: 20.0,
            last_update: std::time::Instant::now(),
        }
    }

    /// Set the title plus its duration. The displayed segment becomes
    /// `"TITLE (M:SS) *** "`, scrolled in a continuous loop.
    pub fn set_track(&mut self, title: String, duration_secs: Option<f32>) {
        let dur_str = duration_secs
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(|d| {
                let total = d as u32;
                let mins = total / 60;
                let secs = total % 60;
                format!(" ({}:{:02})", mins, secs)
            })
            .unwrap_or_default();
        self.text = format!("{}{}{}", title, dur_str, SEPARATOR);
        self.scroll_position = 0.0;
    }

    pub fn update(&mut self) {
        let elapsed = self.last_update.elapsed().as_secs_f32();
        self.last_update = std::time::Instant::now();

        if !self.text.is_empty() {
            self.scroll_position += self.scroll_speed * elapsed;
        }
    }

    pub fn render(&self, renderer: &mut WszRenderer, ui: &mut egui::Ui, offset: Pos2) {
        if self.text.is_empty() {
            return;
        }

        let scale = renderer.get_scale();
        let (x, y) = self.position;
        let pos = offset + Vec2::new(x as f32 * scale, y as f32 * scale);
        // Winamp's title region is GLYPH_H px tall — anything taller masks the
        // bitrate / khz / stereo row baked into the skin bitmap below it.
        let clip_rect = egui::Rect::from_min_size(
            pos,
            Vec2::new(self.width as f32 * scale, GLYPH_H as f32 * scale),
        );

        let segment_w_px = self.text.chars().count() as f32 * GLYPH_W as f32;
        if segment_w_px <= 0.0 {
            return;
        }

        // Modulo-wrap the scroll position so each tiled copy of the text lines
        // up exactly with its neighbour — no visible seam between repeats.
        let scroll_mod = self.scroll_position.rem_euclid(segment_w_px);

        ui.scope(|ui| {
            ui.set_clip_rect(clip_rect);

            // Draw tiled copies of the segment to fill the visible width. The
            // first copy starts at `-scroll_mod` (potentially off-screen left);
            // subsequent copies follow at `+segment_w_px` until we exceed the
            // visible window.
            let mut copy_x = -scroll_mod;
            while copy_x < self.width as f32 {
                let glyph_pos = Pos2::new(pos.x + copy_x * scale, pos.y);
                if bitmap_font::render_text(renderer, ui, &self.text, glyph_pos).is_none() {
                    // Fallback path uses egui's proportional font.
                    ui.painter().with_clip_rect(clip_rect).text(
                        glyph_pos,
                        egui::Align2::LEFT_TOP,
                        &self.text,
                        egui::FontId::proportional(8.0 * scale),
                        Color32::from_rgb(0, 255, 0),
                    );
                    break;
                }
                copy_x += segment_w_px;
            }
        });
    }
}

impl Default for TitleScroller {
    fn default() -> Self {
        Self::new()
    }
}
