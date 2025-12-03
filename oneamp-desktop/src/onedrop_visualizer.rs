use anyhow::Result;
use onedrop_engine::{EngineConfig, MilkEngine};
use onedrop_renderer::RenderConfig;
use std::path::{Path, PathBuf};

/// OneDrop (Milkdrop) visualizer wrapper for OneAmp
pub struct OneDropVisualizer {
    engine: MilkEngine,
    presets: Vec<PathBuf>,
    current_index: usize,
    enabled: bool,
    width: u32,
    height: u32,
}

impl OneDropVisualizer {
    /// Create a new OneDrop visualizer
    pub async fn new(width: u32, height: u32) -> Result<Self> {
        let config = EngineConfig {
            render_config: RenderConfig {
                width,
                height,
                ..Default::default()
            },
            sample_rate: 44100.0,
            enable_per_frame: true,
            enable_per_pixel: false, // Disabled for performance
        };

        let engine = MilkEngine::new(config).await?;

        Ok(Self {
            engine,
            presets: Vec::new(),
            current_index: 0,
            enabled: false,
            width,
            height,
        })
    }

    /// Load presets from a directory
    pub fn load_presets<P: AsRef<Path>>(&mut self, preset_dir: P) -> Result<()> {
        let preset_dir = preset_dir.as_ref();

        if !preset_dir.exists() {
            // Preset directory does not exist
            return Ok(());
        }

        self.presets.clear();

        // Scan directory for .milk files
        for entry in std::fs::read_dir(preset_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "milk") {
                self.presets.push(path);
            }
        }

        // Sort presets by name
        self.presets.sort();

        // Loaded presets

        // Load first preset if available
        if !self.presets.is_empty() {
            self.load_current_preset()?;
        }

        Ok(())
    }

    /// Load the current preset
    fn load_current_preset(&mut self) -> Result<()> {
        if self.presets.is_empty() {
            return Ok(());
        }

        let preset_path = &self.presets[self.current_index];
        // Loading preset

        self.engine.load_preset(preset_path)?;

        Ok(())
    }

    /// Update visualizer with audio samples
    pub fn update(&mut self, audio_samples: &[f32], delta_time: f32) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        self.engine.update(audio_samples, delta_time)?;

        Ok(())
    }

    /// Get the render texture size
    pub fn render_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Check if visualizer is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the render texture for display
    pub fn render_texture(&self) -> &wgpu::Texture {
        self.engine.render_texture()
    }

    /// Navigate to next preset
    pub fn next_preset(&mut self) -> Result<()> {
        if self.presets.is_empty() {
            return Ok(());
        }

        self.current_index = (self.current_index + 1) % self.presets.len();
        self.load_current_preset()?;

        Ok(())
    }

    /// Navigate to previous preset
    pub fn previous_preset(&mut self) -> Result<()> {
        if self.presets.is_empty() {
            return Ok(());
        }

        self.current_index = if self.current_index == 0 {
            self.presets.len() - 1
        } else {
            self.current_index - 1
        };

        self.load_current_preset()?;

        Ok(())
    }

    /// Get current preset name
    pub fn current_preset_name(&self) -> Option<String> {
        if self.presets.is_empty() {
            return None;
        }

        self.presets[self.current_index]
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    }

    /// Get preset count
    pub fn preset_count(&self) -> usize {
        self.presets.len()
    }

    /// Get current preset index (1-based for display)
    pub fn current_preset_index(&self) -> usize {
        if self.presets.is_empty() {
            0
        } else {
            self.current_index + 1
        }
    }

    /// Enable/disable visualizer
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if presets are loaded
    pub fn has_presets(&self) -> bool {
        !self.presets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_onedrop_visualizer_creation() {
        let visualizer = pollster::block_on(async { OneDropVisualizer::new(800, 600).await });

        assert!(visualizer.is_ok());
    }

    #[test]
    fn test_preset_loading() {
        let mut visualizer =
            pollster::block_on(async { OneDropVisualizer::new(800, 600).await.unwrap() });

        // Try to load from onedrop test-presets directory
        let preset_dir = PathBuf::from("../../onedrop/test-presets");
        if preset_dir.exists() {
            let result = visualizer.load_presets(&preset_dir);
            assert!(result.is_ok());

            if visualizer.has_presets() {
                assert!(visualizer.current_preset_name().is_some());
                assert!(visualizer.preset_count() > 0);
            }
        }
    }

    #[test]
    fn test_preset_navigation() {
        let mut visualizer =
            pollster::block_on(async { OneDropVisualizer::new(800, 600).await.unwrap() });

        let preset_dir = PathBuf::from("../../onedrop/test-presets");
        if preset_dir.exists() {
            let _ = visualizer.load_presets(&preset_dir);

            if visualizer.preset_count() > 1 {
                let first_name = visualizer.current_preset_name();

                visualizer.next_preset().unwrap();
                let second_name = visualizer.current_preset_name();

                assert_ne!(first_name, second_name);

                visualizer.previous_preset().unwrap();
                let back_to_first = visualizer.current_preset_name();

                assert_eq!(first_name, back_to_first);
            }
        }
    }
}
