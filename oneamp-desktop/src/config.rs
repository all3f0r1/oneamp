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
}

fn default_first_run() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            equalizer: EqualizerConfig::default(),
            first_run: true,
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
