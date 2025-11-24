use std::f32::consts::PI;

/// Biquad filter implementation for audio equalization
/// Based on Robert Bristow-Johnson's Audio EQ Cookbook
#[derive(Debug, Clone)]
pub struct BiquadFilter {
    // Filter coefficients
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    
    // State variables for left and right channels
    x1_l: f32,
    x2_l: f32,
    y1_l: f32,
    y2_l: f32,
    
    x1_r: f32,
    x2_r: f32,
    y1_r: f32,
    y2_r: f32,
}

impl BiquadFilter {
    /// Create a new biquad filter with neutral coefficients (pass-through)
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1_l: 0.0,
            x2_l: 0.0,
            y1_l: 0.0,
            y2_l: 0.0,
            x1_r: 0.0,
            x2_r: 0.0,
            y1_r: 0.0,
            y2_r: 0.0,
        }
    }
    
    /// Configure as a peaking EQ filter
    /// 
    /// # Arguments
    /// * `sample_rate` - Sample rate in Hz
    /// * `frequency` - Center frequency in Hz
    /// * `gain_db` - Gain in decibels (positive = boost, negative = cut)
    /// * `q` - Q factor (bandwidth), typically 0.5 to 2.0
    pub fn set_peaking_eq(&mut self, sample_rate: f32, frequency: f32, gain_db: f32, q: f32) {
        let a = 10_f32.powf(gain_db / 40.0);
        let omega = 2.0 * PI * frequency / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q);
        
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_omega;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha / a;
        
        // Normalize coefficients
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
    
    /// Process a stereo sample pair
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        // Process left channel
        let left_out = self.b0 * left + self.b1 * self.x1_l + self.b2 * self.x2_l
                       - self.a1 * self.y1_l - self.a2 * self.y2_l;
        self.x2_l = self.x1_l;
        self.x1_l = left;
        self.y2_l = self.y1_l;
        self.y1_l = left_out;
        
        // Process right channel
        let right_out = self.b0 * right + self.b1 * self.x1_r + self.b2 * self.x2_r
                        - self.a1 * self.y1_r - self.a2 * self.y2_r;
        self.x2_r = self.x1_r;
        self.x1_r = right;
        self.y2_r = self.y1_r;
        self.y1_r = right_out;
        
        (left_out, right_out)
    }
    
    /// Reset filter state (useful when changing tracks)
    pub fn reset(&mut self) {
        self.x1_l = 0.0;
        self.x2_l = 0.0;
        self.y1_l = 0.0;
        self.y2_l = 0.0;
        self.x1_r = 0.0;
        self.x2_r = 0.0;
        self.y1_r = 0.0;
        self.y2_r = 0.0;
    }
}

impl Default for BiquadFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// 10-band graphic equalizer
#[derive(Debug, Clone)]
pub struct Equalizer {
    /// Individual band filters
    bands: Vec<BiquadFilter>,
    /// Band frequencies in Hz
    frequencies: Vec<f32>,
    /// Band gains in dB (-12 to +12)
    gains: Vec<f32>,
    /// Current sample rate
    sample_rate: f32,
    /// Whether the equalizer is enabled
    enabled: bool,
}

impl Equalizer {
    /// Create a new 10-band equalizer
    pub fn new(sample_rate: f32) -> Self {
        // Standard 10-band equalizer frequencies
        let frequencies = vec![
            31.25,   // Sub-bass
            62.5,    // Bass
            125.0,   // Bass
            250.0,   // Low midrange
            500.0,   // Midrange
            1000.0,  // Midrange
            2000.0,  // Upper midrange
            4000.0,  // Presence
            8000.0,  // Brilliance
            16000.0, // Air
        ];
        
        let mut eq = Self {
            bands: vec![BiquadFilter::new(); 10],
            frequencies: frequencies.clone(),
            gains: vec![0.0; 10],
            sample_rate,
            enabled: false,
        };
        
        // Initialize all filters with 0 dB gain
        eq.update_filters();
        
        eq
    }
    
    /// Enable or disable the equalizer
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            // Reset all filters when disabling
            for band in &mut self.bands {
                band.reset();
            }
        }
    }
    
    /// Check if equalizer is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    /// Set gain for a specific band (0-9)
    /// 
    /// # Arguments
    /// * `band_index` - Band index (0-9)
    /// * `gain_db` - Gain in decibels (-12 to +12)
    pub fn set_band_gain(&mut self, band_index: usize, gain_db: f32) {
        if band_index < self.gains.len() {
            self.gains[band_index] = gain_db.clamp(-12.0, 12.0);
            self.update_filter(band_index);
        }
    }
    
    /// Get gain for a specific band
    pub fn get_band_gain(&self, band_index: usize) -> f32 {
        self.gains.get(band_index).copied().unwrap_or(0.0)
    }
    
    /// Get all band gains
    pub fn get_all_gains(&self) -> &[f32] {
        &self.gains
    }
    
    /// Set all band gains at once
    pub fn set_all_gains(&mut self, gains: &[f32]) {
        for (i, &gain) in gains.iter().enumerate().take(self.gains.len()) {
            self.gains[i] = gain.clamp(-12.0, 12.0);
        }
        self.update_filters();
    }
    
    /// Reset all bands to 0 dB (flat response)
    pub fn reset_all_bands(&mut self) {
        for gain in &mut self.gains {
            *gain = 0.0;
        }
        self.update_filters();
    }
    
    /// Get band frequencies
    pub fn get_frequencies(&self) -> &[f32] {
        &self.frequencies
    }
    
    /// Update a single filter's coefficients
    fn update_filter(&mut self, band_index: usize) {
        if band_index < self.bands.len() {
            let q = 1.0; // Q factor for graphic EQ
            self.bands[band_index].set_peaking_eq(
                self.sample_rate,
                self.frequencies[band_index],
                self.gains[band_index],
                q,
            );
        }
    }
    
    /// Update all filter coefficients
    fn update_filters(&mut self) {
        for i in 0..self.bands.len() {
            self.update_filter(i);
        }
    }
    
    /// Process a stereo sample through all bands
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        if !self.enabled {
            return (left, right);
        }
        
        let mut l = left;
        let mut r = right;
        
        // Process through all bands in series
        for band in &mut self.bands {
            let (l_out, r_out) = band.process_stereo(l, r);
            l = l_out;
            r = r_out;
        }
        
        (l, r)
    }
    
    /// Update sample rate (call when track changes)
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() > 0.1 {
            self.sample_rate = sample_rate;
            self.update_filters();
        }
    }
}

impl Default for Equalizer {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_biquad_passthrough() {
        let mut filter = BiquadFilter::new();
        let (l, r) = filter.process_stereo(1.0, -1.0);
        assert!((l - 1.0).abs() < 0.001);
        assert!((r + 1.0).abs() < 0.001);
    }
    
    #[test]
    fn test_equalizer_disabled() {
        let mut eq = Equalizer::new(44100.0);
        eq.set_band_gain(0, 6.0);
        let (l, r) = eq.process_stereo(1.0, 1.0);
        // Should pass through unchanged when disabled
        assert!((l - 1.0).abs() < 0.001);
        assert!((r - 1.0).abs() < 0.001);
    }
    
    #[test]
    fn test_equalizer_gain_clamping() {
        let mut eq = Equalizer::new(44100.0);
        eq.set_band_gain(0, 20.0); // Should clamp to 12.0
        assert_eq!(eq.get_band_gain(0), 12.0);
        eq.set_band_gain(1, -20.0); // Should clamp to -12.0
        assert_eq!(eq.get_band_gain(1), -12.0);
    }
}
