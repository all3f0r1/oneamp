use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use crate::visual_effects::VisualEffects;
use crate::theme::Theme;

/// Advanced equalizer display with 3D sliders and level indicators
pub struct EqualizerDisplay {
    peak_values: Vec<f32>,
    peak_hold_time: Vec<f32>,
    peak_decay_speed: f32,
    last_update: std::time::Instant,
}

impl EqualizerDisplay {
    pub fn new(band_count: usize) -> Self {
        Self {
            peak_values: vec![0.0; band_count],
            peak_hold_time: vec![0.0; band_count],
            peak_decay_speed: 2.0, // dB per second
            last_update: std::time::Instant::now(),
        }
    }
    
    /// Update peak indicators
    pub fn update(&mut self, eq_gains: &[f32]) {
        let dt = self.last_update.elapsed().as_secs_f32();
        self.last_update = std::time::Instant::now();
        
        for (i, &gain) in eq_gains.iter().enumerate() {
            if i < self.peak_values.len() {
                // Update peak if current value is higher
                if gain.abs() > self.peak_values[i] {
                    self.peak_values[i] = gain.abs();
                    self.peak_hold_time[i] = 1.0; // Hold for 1 second
                } else if self.peak_hold_time[i] > 0.0 {
                    // Hold peak
                    self.peak_hold_time[i] -= dt;
                } else {
                    // Decay peak
                    self.peak_values[i] = (self.peak_values[i] - self.peak_decay_speed * dt).max(0.0);
                }
            }
        }
    }
    
    /// Render the advanced equalizer display
    pub fn render(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        eq_enabled: &mut bool,
        eq_gains: &mut Vec<f32>,
        eq_frequencies: &[f32],
    ) -> bool {
        let mut changed = false;
        
        // Update peaks
        self.update(eq_gains);
        
        // Glass panel background
        let panel_rect = ui.available_rect_before_wrap();
        if ui.is_rect_visible(panel_rect) {
            let painter = ui.painter();
            VisualEffects::glass_panel(
                painter,
                panel_rect,
                Theme::color32(&theme.colors.panel_bg).linear_multiply(0.8),
                6.0,
            );
        }
        
        ui.vertical(|ui| {
            ui.add_space(8.0);
            
            // Header with enable checkbox and reset button
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                
                ui.label(
                    egui::RichText::new("Equalizer")
                        .size(16.0)
                        .color(Theme::color32(&theme.colors.display_text))
                );
                
                if ui.checkbox(eq_enabled, "Enabled").changed() {
                    changed = true;
                }
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    
                    if ui.button("Reset").clicked() {
                        for gain in eq_gains.iter_mut() {
                            *gain = 0.0;
                        }
                        changed = true;
                    }
                });
            });
            
            ui.add_space(12.0);
            
            // Equalizer bands
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                
                let available_width = ui.available_width() - 16.0;
                let band_width = (available_width / eq_gains.len() as f32).min(80.0);
                
                for (i, gain) in eq_gains.iter_mut().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(band_width);
                        
                        // Frequency label
                        let freq_label = if eq_frequencies[i] >= 1000.0 {
                            format!("{}k", eq_frequencies[i] as u32 / 1000)
                        } else {
                            format!("{}", eq_frequencies[i] as u32)
                        };
                        
                        ui.label(
                            egui::RichText::new(freq_label)
                                .size(10.0)
                                .color(Theme::color32(&theme.colors.display_text).linear_multiply(0.7))
                        );
                        
                        ui.add_space(4.0);
                        
                        // 3D Slider with level indicator
                        if render_eq_slider_3d(
                            ui,
                            theme,
                            gain,
                            self.peak_values.get(i).copied().unwrap_or(0.0),
                        ) {
                            changed = true;
                        }
                        
                        ui.add_space(4.0);
                        
                        // Gain value
                        ui.label(
                            egui::RichText::new(format!("{:+.1}", gain))
                                .size(9.0)
                                .monospace()
                                .color(Theme::color32(&theme.colors.display_text))
                        );
                    });
                }
                
                ui.add_space(8.0);
            });
            
            ui.add_space(8.0);
        });
        
        changed
    }
}

/// Render a 3D equalizer slider with level indicator and peak
fn render_eq_slider_3d(
    ui: &mut Ui,
    theme: &Theme,
    value: &mut f32,
    peak_value: f32,
) -> bool {
    let mut changed = false;
    
    let slider_height = 120.0;
    let slider_width = 40.0;
    
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(slider_width, slider_height),
        Sense::click_and_drag(),
    );
    
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        
        // Track background (metallic)
        let track_width = 8.0;
        let track_rect = Rect::from_center_size(
            rect.center(),
            Vec2::new(track_width, slider_height),
        );
        
        VisualEffects::metallic_panel(
            painter,
            track_rect,
            Theme::color32(&theme.colors.eq_slider).linear_multiply(0.6),
            4.0,
        );
        
        // Center line (0 dB)
        let center_y = rect.center().y;
        painter.line_segment(
            [
                Pos2::new(track_rect.left(), center_y),
                Pos2::new(track_rect.right(), center_y),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(80)),
        );
        
        // Level indicator (gradient fill)
        let normalized_value = (*value + 12.0) / 24.0; // -12 to +12 -> 0 to 1
        let fill_height = slider_height * normalized_value.clamp(0.0, 1.0);
        
        if fill_height > 1.0 {
            let fill_rect = Rect::from_min_size(
                Pos2::new(track_rect.left(), track_rect.bottom() - fill_height),
                Vec2::new(track_width, fill_height),
            );
            
            // Gradient color based on value
            let color = if *value > 6.0 {
                Color32::from_rgb(255, 50, 50) // Red
            } else if *value > 0.0 {
                Color32::from_rgb(255, 200, 50) // Yellow
            } else if *value > -6.0 {
                Color32::from_rgb(50, 255, 100) // Green
            } else {
                Color32::from_rgb(50, 150, 255) // Blue
            };
            
            VisualEffects::gradient_rect_vertical(
                painter,
                fill_rect,
                color.linear_multiply(1.2),
                color.linear_multiply(0.7),
                4.0,
            );
        }
        
        // Peak indicator (small horizontal line)
        if peak_value > 0.0 {
            let normalized_peak = (peak_value + 12.0) / 24.0;
            let peak_y = track_rect.bottom() - slider_height * normalized_peak.clamp(0.0, 1.0);
            
            let peak_color = if peak_value > 6.0 {
                Color32::from_rgb(255, 100, 100)
            } else {
                Color32::from_rgb(100, 255, 150)
            };
            
            // Glow for peak
            for i in 0..3 {
                let alpha = (100 - i * 30).max(20) as u8;
                painter.line_segment(
                    [
                        Pos2::new(track_rect.left() - 2.0, peak_y + i as f32 * 0.5),
                        Pos2::new(track_rect.right() + 2.0, peak_y + i as f32 * 0.5),
                    ],
                    Stroke::new(2.0, Color32::from_rgba_premultiplied(
                        peak_color.r(),
                        peak_color.g(),
                        peak_color.b(),
                        alpha,
                    )),
                );
            }
            
            // Main peak line
            painter.line_segment(
                [
                    Pos2::new(track_rect.left() - 2.0, peak_y),
                    Pos2::new(track_rect.right() + 2.0, peak_y),
                ],
                Stroke::new(2.0, peak_color),
            );
        }
        
        // Handle drag
        if response.dragged() {
            if let Some(pointer_pos) = response.interact_pointer_pos() {
                let normalized = 1.0 - ((pointer_pos.y - rect.top()) / slider_height).clamp(0.0, 1.0);
                *value = (normalized * 24.0 - 12.0).clamp(-12.0, 12.0);
                changed = true;
                response.mark_changed();
            }
        }
        
        // Thumb (draggable knob)
        let thumb_y = rect.bottom() - slider_height * normalized_value.clamp(0.0, 1.0);
        let thumb_center = Pos2::new(rect.center().x, thumb_y);
        let thumb_size = Vec2::new(slider_width, 12.0);
        let thumb_rect = Rect::from_center_size(thumb_center, thumb_size);
        
        // Thumb shadow
        VisualEffects::drop_shadow(
            painter,
            thumb_rect,
            4.0,
            Vec2::new(0.0, 2.0),
            3.0,
            Color32::from_black_alpha(120),
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
            4.0,
        );
        
        response.widget_info(|| egui::WidgetInfo::slider(true, *value as f64, ""));
    }
    
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_equalizer_display_creation() {
        let eq = EqualizerDisplay::new(10);
        assert_eq!(eq.peak_values.len(), 10);
        assert_eq!(eq.peak_hold_time.len(), 10);
    }
    
    #[test]
    fn test_peak_update() {
        let mut eq = EqualizerDisplay::new(3);
        let gains = vec![5.0, -3.0, 8.0];
        
        eq.update(&gains);
        
        assert!(eq.peak_values[0] > 0.0);
        assert!(eq.peak_values[2] > 0.0);
    }
}
