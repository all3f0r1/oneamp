use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};
use rustfft::{num_complex::Complex, FftPlanner};

/// Type of visualization
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VisualizationType {
    Oscilloscope,
    Spectrum,
}

impl VisualizationType {
    pub fn toggle(&mut self) {
        *self = match self {
            VisualizationType::Oscilloscope => VisualizationType::Spectrum,
            VisualizationType::Spectrum => VisualizationType::Oscilloscope,
        };
    }

    pub fn name(&self) -> &str {
        match self {
            VisualizationType::Oscilloscope => "Oscilloscope",
            VisualizationType::Spectrum => "Spectrum",
        }
    }
}

/// Audio visualizer
pub struct Visualizer {
    viz_type: VisualizationType,
    samples: Vec<f32>,
    spectrum: Vec<f32>,
    fft_buffer: Vec<Complex<f32>>,
    fft_planner: FftPlanner<f32>,
}

impl Visualizer {
    pub fn new() -> Self {
        Self {
            viz_type: VisualizationType::Oscilloscope,
            samples: vec![0.0; 256],
            spectrum: vec![0.0; 64],
            fft_buffer: vec![Complex::new(0.0, 0.0); 512],
            fft_planner: FftPlanner::new(),
        }
    }

    /// Toggle between visualization types
    pub fn toggle_type(&mut self) {
        self.viz_type.toggle();
    }

    /// Get current visualization type
    pub fn viz_type(&self) -> VisualizationType {
        self.viz_type
    }

    /// Get spectrum data for external rendering
    pub fn get_spectrum(&self) -> &[f32] {
        &self.spectrum
    }

    /// Update with new audio samples
    pub fn update(&mut self, audio_samples: &[f32]) {
        if audio_samples.is_empty() {
            return;
        }

        // Update oscilloscope samples (downsample if needed)
        let step = (audio_samples.len() / self.samples.len()).max(1);
        for (i, sample) in self.samples.iter_mut().enumerate() {
            let idx = i * step;
            if idx < audio_samples.len() {
                *sample = audio_samples[idx];
            }
        }

        // Compute spectrum using simple FFT approximation
        self.compute_spectrum(audio_samples);
    }

    /// Compute spectrum from audio samples using FFT
    fn compute_spectrum(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }

        // Prepare FFT buffer
        let fft_size = self.fft_buffer.len();
        for (i, buf) in self.fft_buffer.iter_mut().enumerate() {
            if i < samples.len() {
                buf.re = samples[i];
                buf.im = 0.0;
            } else {
                buf.re = 0.0;
                buf.im = 0.0;
            }
        }

        // Perform FFT
        let fft = self.fft_planner.plan_fft_forward(fft_size);
        fft.process(&mut self.fft_buffer);

        // Convert FFT output to spectrum bands
        let bins_per_band = (fft_size / 2) / self.spectrum.len();

        for (i, band) in self.spectrum.iter_mut().enumerate() {
            let start = i * bins_per_band;
            let end = ((i + 1) * bins_per_band).min(fft_size / 2);

            // Calculate magnitude for this band
            let mut magnitude = 0.0;
            for bin in start..end {
                let complex = self.fft_buffer[bin];
                magnitude += (complex.re * complex.re + complex.im * complex.im).sqrt();
            }
            magnitude /= (end - start) as f32;

            // Normalize and apply smoothing
            magnitude = (magnitude / 100.0).min(1.0); // Normalize
            *band = *band * 0.7 + magnitude * 0.3; // Smooth
        }
    }

    /// Draw the visualizer
    pub fn draw(&self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter();

        // Draw background
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 20, 25));

        match self.viz_type {
            VisualizationType::Oscilloscope => self.draw_oscilloscope(painter, rect),
            VisualizationType::Spectrum => self.draw_spectrum(painter, rect),
        }
    }

    /// Draw oscilloscope (waveform)
    fn draw_oscilloscope(&self, painter: &Painter, rect: Rect) {
        let width = rect.width();
        let height = rect.height();
        let center_y = rect.center().y;

        // Draw center line
        painter.line_segment(
            [
                Pos2::new(rect.left(), center_y),
                Pos2::new(rect.right(), center_y),
            ],
            Stroke::new(1.0, Color32::from_rgb(40, 40, 50)),
        );

        // Draw waveform
        let mut points = Vec::with_capacity(self.samples.len());
        for (i, &sample) in self.samples.iter().enumerate() {
            let x = rect.left() + (i as f32 / self.samples.len() as f32) * width;
            let y = center_y - sample * (height * 0.4);
            points.push(Pos2::new(x, y));
        }

        if points.len() > 1 {
            painter.add(egui::Shape::line(
                points,
                Stroke::new(2.0, Color32::from_rgb(0, 200, 255)),
            ));
        }
    }

    /// Draw spectrum analyzer (frequency bars)
    fn draw_spectrum(&self, painter: &Painter, rect: Rect) {
        let width = rect.width();
        let height = rect.height();
        let bar_width = width / self.spectrum.len() as f32;
        let bar_spacing = 2.0;

        for (i, &energy) in self.spectrum.iter().enumerate() {
            let x = rect.left() + i as f32 * bar_width;
            let bar_height = energy * height * 2.0; // Scale up for visibility
            let bar_height = bar_height.min(height); // Clamp to max height

            if bar_height > 1.0 {
                // Gradient color based on height
                let color = if bar_height > height * 0.7 {
                    Color32::from_rgb(255, 100, 100) // Red for high
                } else if bar_height > height * 0.4 {
                    Color32::from_rgb(255, 200, 0) // Yellow for medium
                } else {
                    Color32::from_rgb(0, 200, 255) // Cyan for low
                };

                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(x + bar_spacing / 2.0, rect.bottom() - bar_height),
                        Vec2::new(bar_width - bar_spacing, bar_height),
                    ),
                    2.0,
                    color,
                );
            }
        }
    }
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visualizer {
    /// Render oscilloscope with fancy effects
    pub fn render_oscilloscope_fancy(
        &self,
        painter: &eframe::egui::Painter,
        rect: eframe::egui::Rect,
        _time: f32,
    ) {
        use eframe::egui::{Color32, Pos2, Stroke};

        if self.samples.is_empty() {
            return;
        }

        let width = rect.width();
        let height = rect.height();
        let center_y = rect.center().y;

        // Build waveform points
        let mut points = Vec::new();
        for (i, &sample) in self.samples.iter().enumerate() {
            let x = rect.left() + (i as f32 / self.samples.len() as f32) * width;
            let y = center_y + sample * height * 0.4;
            points.push(Pos2::new(x, y));
        }

        // Glow layers (3 layers for depth)
        for layer in 0..3 {
            let alpha = (80 - layer * 25).max(10) as u8;
            let thickness = 2.0 + layer as f32 * 1.5;
            painter.add(eframe::egui::Shape::line(
                points.clone(),
                Stroke::new(
                    thickness,
                    Color32::from_rgba_premultiplied(100, 180, 255, alpha),
                ),
            ));
        }

        // Main waveform
        painter.add(eframe::egui::Shape::line(
            points,
            Stroke::new(2.0, Color32::from_rgb(100, 180, 255)),
        ));

        // Center line
        painter.line_segment(
            [
                Pos2::new(rect.left(), center_y),
                Pos2::new(rect.right(), center_y),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(30)),
        );
    }

    /// Render spectrum analyzer with fancy effects
    pub fn render_spectrum_fancy(
        &self,
        painter: &eframe::egui::Painter,
        rect: eframe::egui::Rect,
        _time: f32,
    ) {
        use crate::visual_effects::VisualEffects;
        use eframe::egui::{Color32, Pos2, Rect as EguiRect, Vec2};

        if self.spectrum.is_empty() {
            return;
        }

        let bar_count = self.spectrum.len();
        let bar_width = (rect.width() / bar_count as f32) * 0.8;
        let spacing = (rect.width() / bar_count as f32) * 0.2;

        for (i, &magnitude) in self.spectrum.iter().enumerate() {
            let x = rect.left() + i as f32 * (bar_width + spacing);
            let bar_height = magnitude * rect.height();

            if bar_height < 1.0 {
                continue;
            }

            let bar_rect = EguiRect::from_min_size(
                Pos2::new(x, rect.bottom() - bar_height),
                Vec2::new(bar_width, bar_height),
            );

            // Gradient color based on height (green -> yellow -> red)
            let color = if magnitude > 0.8 {
                Color32::from_rgb(255, 50, 50) // Red
            } else if magnitude > 0.5 {
                Color32::from_rgb(255, 200, 50) // Yellow
            } else {
                Color32::from_rgb(50, 255, 100) // Green
            };

            // Glow effect for high bars
            if magnitude > 0.6 {
                VisualEffects::glow(painter, bar_rect, 2.0, 4.0, color.linear_multiply(0.5));
            }

            // Bar with gradient
            VisualEffects::gradient_rect_vertical(
                painter,
                bar_rect,
                color.linear_multiply(1.2),
                color.linear_multiply(0.7),
                2.0,
            );

            // Reflection at bottom (subtle)
            let reflection_height = (bar_height * 0.3).min(20.0);
            let reflection_rect = EguiRect::from_min_size(
                Pos2::new(x, rect.bottom()),
                Vec2::new(bar_width, reflection_height),
            );

            VisualEffects::gradient_rect_vertical(
                painter,
                reflection_rect,
                color.linear_multiply(0.3),
                Color32::from_black_alpha(0),
                2.0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visualization_type_toggle() {
        let mut viz_type = VisualizationType::Oscilloscope;
        viz_type.toggle();
        assert_eq!(viz_type, VisualizationType::Spectrum);

        viz_type.toggle();
        assert_eq!(viz_type, VisualizationType::Oscilloscope);
    }

    #[test]
    fn test_visualization_type_name() {
        assert_eq!(VisualizationType::Oscilloscope.name(), "Oscilloscope");
        assert_eq!(VisualizationType::Spectrum.name(), "Spectrum");
    }

    #[test]
    fn test_visualizer_creation() {
        let visualizer = Visualizer::new();
        assert_eq!(visualizer.viz_type(), VisualizationType::Oscilloscope);
        assert_eq!(visualizer.samples.len(), 256);
        assert_eq!(visualizer.spectrum.len(), 64);
    }

    #[test]
    fn test_visualizer_default() {
        let visualizer = Visualizer::default();
        assert_eq!(visualizer.viz_type(), VisualizationType::Oscilloscope);
    }

    #[test]
    fn test_visualizer_toggle() {
        let mut visualizer = Visualizer::new();
        assert_eq!(visualizer.viz_type(), VisualizationType::Oscilloscope);

        visualizer.toggle_type();
        assert_eq!(visualizer.viz_type(), VisualizationType::Spectrum);

        visualizer.toggle_type();
        assert_eq!(visualizer.viz_type(), VisualizationType::Oscilloscope);
    }

    #[test]
    fn test_visualizer_update_empty() {
        let mut visualizer = Visualizer::new();

        // Update with empty samples (should not panic)
        visualizer.update(&[]);

        // Samples should still be initialized
        assert_eq!(visualizer.samples.len(), 256);
        assert_eq!(visualizer.spectrum.len(), 64);
    }

    #[test]
    fn test_visualizer_update_with_data() {
        let mut visualizer = Visualizer::new();

        // Create test audio samples (sine wave)
        let samples: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();

        visualizer.update(&samples);

        // Verify that samples were updated
        assert_eq!(visualizer.samples.len(), 256);

        // At least some samples should be non-zero
        let non_zero_count = visualizer.samples.iter().filter(|&&s| s != 0.0).count();
        assert!(non_zero_count > 0, "Should have some non-zero samples");
    }

    #[test]
    fn test_spectrum_computation() {
        let mut visualizer = Visualizer::new();

        // Create test audio samples with known frequency content
        let samples: Vec<f32> = (0..512).map(|i| (i as f32 * 0.05).sin()).collect();

        visualizer.update(&samples);

        // Spectrum should be updated
        assert_eq!(visualizer.spectrum.len(), 64);

        // At least some spectrum bands should have energy
        let non_zero_bands = visualizer.spectrum.iter().filter(|&&s| s > 0.0).count();
        assert!(
            non_zero_bands > 0,
            "Should have some non-zero spectrum bands"
        );
    }

    #[test]
    fn test_spectrum_normalization() {
        let mut visualizer = Visualizer::new();

        // Create very loud samples
        let samples: Vec<f32> = vec![1.0; 512];

        visualizer.update(&samples);

        // All spectrum values should be normalized (between 0 and 1)
        for &magnitude in &visualizer.spectrum {
            assert!(
                magnitude >= 0.0 && magnitude <= 1.0,
                "Spectrum magnitude should be normalized: {}",
                magnitude
            );
        }
    }

    #[test]
    fn test_fft_buffer_size() {
        let visualizer = Visualizer::new();
        assert_eq!(
            visualizer.fft_buffer.len(),
            512,
            "FFT buffer should be 512 samples"
        );
    }

    #[test]
    fn test_multiple_updates() {
        let mut visualizer = Visualizer::new();

        // Update multiple times
        for _ in 0..10 {
            let samples: Vec<f32> = (0..256).map(|i| (i as f32 * 0.1).sin()).collect();
            visualizer.update(&samples);
        }

        // Should still be valid
        assert_eq!(visualizer.samples.len(), 256);
        assert_eq!(visualizer.spectrum.len(), 64);
    }

    #[test]
    fn test_get_spectrum() {
        let mut visualizer = Visualizer::new();

        // Create test samples
        let samples: Vec<f32> = (0..512).map(|i| (i as f32 * 0.1).sin()).collect();
        visualizer.update(&samples);

        // Get spectrum
        let spectrum = visualizer.get_spectrum();

        // Should return the spectrum slice
        assert_eq!(spectrum.len(), 64);

        // Spectrum values should be normalized
        for &val in spectrum {
            assert!(
                val >= 0.0 && val <= 1.0,
                "Spectrum value out of range: {}",
                val
            );
        }
    }
}
