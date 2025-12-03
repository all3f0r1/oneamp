// Plugin Loader
// Handles dynamic loading of plugins from shared libraries.

use std::path::Path;
use super::traits::InputPlugin;
use super::error::{PluginError, PluginResult};

/// Plugin loader for dynamically loading plugins from shared libraries.
/// 
/// This is a placeholder for future implementation using libloading.
/// For now, only built-in plugins are supported.
pub struct PluginLoader;

impl PluginLoader {
    /// Loads a plugin from a shared library file.
    ///
    /// # Arguments
    /// * `path` - Path to the plugin shared library (.so, .dll, .dylib)
    ///
    /// # Returns
    /// A boxed InputPlugin trait object, or an error if loading fails.
    ///
    /// # Note
    /// This is a placeholder for future implementation.
    /// The actual implementation will use libloading to dynamically load
    /// shared libraries and call the plugin entry point function.
    pub fn load_input_plugin(_path: &Path) -> PluginResult<Box<dyn InputPlugin>> {
        Err(PluginError::Other(
            "Dynamic plugin loading is not yet implemented".to_string(),
        ))
    }

    /// Validates a plugin file before loading.
    ///
    /// # Arguments
    /// * `path` - Path to the plugin file
    ///
    /// # Returns
    /// Ok if the file appears to be a valid plugin, Err otherwise.
    pub fn validate_plugin_file(path: &Path) -> PluginResult<()> {
        // Check that file exists
        if !path.exists() {
            return Err(PluginError::FileNotFound(
                format!("{:?}", path),
            ));
        }

        // Check that file has a valid extension
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        match extension {
            "so" | "dll" | "dylib" => Ok(()),
            _ => Err(PluginError::Other(
                format!("Invalid plugin file extension: {}", extension),
            )),
        }
    }

    /// Lists all valid plugin files in a directory.
    ///
    /// # Arguments
    /// * `dir` - Directory to scan for plugins
    ///
    /// # Returns
    /// A vector of paths to valid plugin files.
    pub fn list_plugins(dir: &Path) -> PluginResult<Vec<std::path::PathBuf>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut plugins = Vec::new();

        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if Self::validate_plugin_file(&path).is_ok() {
                        plugins.push(path);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to read plugin directory: {}", e);
            }
        }

        Ok(plugins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validate_plugin_file_invalid_extension() {
        let path = PathBuf::from("plugin.txt");
        assert!(PluginLoader::validate_plugin_file(&path).is_err());
    }

    #[test]
    fn test_validate_plugin_file_nonexistent() {
        let path = PathBuf::from("/nonexistent/plugin.so");
        assert!(PluginLoader::validate_plugin_file(&path).is_err());
    }

    #[test]
    fn test_list_plugins_nonexistent_dir() {
        let result = PluginLoader::list_plugins(Path::new("/nonexistent"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
