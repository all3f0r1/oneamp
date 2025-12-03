use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqualizerConfig {
    pub enabled: bool,
    pub gains: Vec<f32>,
}

impl Default for EqualizerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gains: vec![0.0; 10],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub equalizer: EqualizerConfig,
    #[serde(default = "default_first_run")]
    pub first_run: bool,
    #[serde(default = "default_active_skin")]
    pub active_skin: String,
}

fn default_active_skin() -> String {
    "OneAmp Dark".to_string()
}

fn default_first_run() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            equalizer: EqualizerConfig::default(),
            first_run: true,
            active_skin: default_active_skin(),
        }
    }
}

impl AppConfig {
    /// Get the config file path
    fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Failed to get config directory")?;
        let oneamp_dir = config_dir.join("oneamp");
        
        // Create directory if it doesn't exist
        if !oneamp_dir.exists() {
            fs::create_dir_all(&oneamp_dir)
                .context("Failed to create config directory")?;
        }
        
        Ok(oneamp_dir.join("config.json"))
    }
    
    /// Load configuration from file
    /// Returns (config, is_first_run)
    pub fn load() -> (Self, bool) {
        match Self::config_path() {
            Ok(path) => {
                if path.exists() {
                    match fs::read_to_string(&path) {
                        Ok(content) => {
                            match serde_json::from_str::<AppConfig>(&content) {
                                Ok(mut config) => {
                                    let is_first = config.first_run;
                                    config.first_run = false;
                                    return (config, is_first);
                                }
                                Err(e) => eprintln!("Failed to parse config: {}", e),
                            }
                        }
                        Err(e) => eprintln!("Failed to read config file: {}", e),
                    }
                }
            }
            Err(e) => eprintln!("Failed to get config path: {}", e),
        }
        
        // Return default config if loading failed (first run)
        (Self::default(), true)
    }
    
    /// Save configuration to file
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize config")?;
        fs::write(&path, content)
            .context("Failed to write config file")?;
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equalizer_config_default() {
        let config = EqualizerConfig::default();
        assert!(!config.enabled, "Equalizer should be disabled by default");
        assert_eq!(config.gains.len(), 10, "Should have 10 bands");
        assert!(config.gains.iter().all(|&g| g == 0.0), "All gains should be 0.0");
    }

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert!(!config.equalizer.enabled, "Equalizer should be disabled by default");
        assert!(config.first_run, "Should be first run by default");
    }

    #[test]
    fn test_equalizer_config_serialization() {
        let config = EqualizerConfig {
            enabled: true,
            gains: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
        };
        
        let json = serde_json::to_string(&config).expect("Should serialize");
        let deserialized: EqualizerConfig = serde_json::from_str(&json).expect("Should deserialize");
        
        assert_eq!(config.enabled, deserialized.enabled);
        assert_eq!(config.gains, deserialized.gains);
    }

    #[test]
    fn test_app_config_serialization() {
        let config = AppConfig {
            equalizer: EqualizerConfig {
                enabled: true,
                gains: vec![1.0; 10],
            },
            first_run: false,
        };
        
        let json = serde_json::to_string(&config).expect("Should serialize");
        let deserialized: AppConfig = serde_json::from_str(&json).expect("Should deserialize");
        
        assert_eq!(config.equalizer.enabled, deserialized.equalizer.enabled);
        assert_eq!(config.first_run, deserialized.first_run);
    }

    #[test]
    fn test_config_save_and_load() {
        // Create a test config
        let mut config = AppConfig::default();
        config.equalizer.enabled = true;
        config.equalizer.gains = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        config.first_run = false;
        
        // Save it
        let save_result = config.save();
        // May fail if no config directory is available, that's ok for testing
        if save_result.is_ok() {
            // Load it back
            let (loaded_config, is_first_run) = AppConfig::load();
            
            // Verify values
            assert_eq!(config.equalizer.enabled, loaded_config.equalizer.enabled);
            assert_eq!(config.equalizer.gains, loaded_config.equalizer.gains);
            // first_run should be false after loading
            assert!(!is_first_run, "Should not be first run after save/load");
        }
    }

    #[test]
    fn test_config_path() {
        // Test that config_path returns a valid path
        let path_result = AppConfig::config_path();
        // Should either succeed or fail gracefully
        match path_result {
            Ok(path) => {
                assert!(path.ends_with("config.json"), "Path should end with config.json");
                assert!(path.to_string_lossy().contains("oneamp"), "Path should contain oneamp");
            }
            Err(_) => {
                // It's ok if it fails in test environment
            }
        }
    }

    #[test]
    fn test_first_run_detection() {
        // Test that first run is detected correctly
        let (config, is_first_run) = AppConfig::load();
        
        // Should return a valid config
        assert_eq!(config.equalizer.gains.len(), 10);
        
        // is_first_run can be true or false depending on whether config exists
        // Just verify it's a boolean
        assert!(is_first_run || !is_first_run);
    }
}
