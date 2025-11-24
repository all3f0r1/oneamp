use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};
use std::f32::consts::PI;

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
}

impl Visualizer {
    pub fn new() -> Self {
        Self {
            viz_type: VisualizationType::Oscilloscope,
            samples: vec![0.0; 256],
            spectrum: vec![0.0; 64],
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
    
    /// Compute spectrum from audio samples (simplified FFT)
    fn compute_spectrum(&mut self, samples: &[f32]) {
        // Simple energy-based spectrum analyzer
        // Divide audio into frequency bands
        let samples_per_band = samples.len() / self.spectrum.len();
        
        for (i, band) in self.spectrum.iter_mut().enumerate() {
            let start = i * samples_per_band;
            let end = ((i + 1) * samples_per_band).min(samples.len());
            
            if start >= samples.len() {
                *band = 0.0;
                continue;
            }
            
            // Calculate RMS energy for this band
            let mut energy = 0.0;
            let mut count = 0;
            for &sample in &samples[start..end] {
                energy += sample * sample;
                count += 1;
            }
            
            if count > 0 {
                energy = (energy / count as f32).sqrt();
                // Apply some smoothing
                *band = *band * 0.7 + energy * 0.3;
            }
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
