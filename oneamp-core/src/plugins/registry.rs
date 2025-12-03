// Plugin Registry
// Manages plugin discovery, loading, and lifecycle.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use super::traits::{InputPlugin, OutputPlugin, DSPPlugin};
use super::error::PluginResult;

/// Central registry for managing plugins.
/// Handles plugin discovery, loading, and provides access to registered plugins.
pub struct PluginRegistry {
    input_plugins: Vec<Arc<dyn InputPlugin>>,
    output_plugins: Vec<Arc<dyn OutputPlugin>>,
    dsp_plugins: Vec<Arc<dyn DSPPlugin>>,
    plugin_dir: PathBuf,
}

impl PluginRegistry {
    /// Creates a new plugin registry.
    ///
    /// # Arguments
    /// * `plugin_dir` - Directory where external plugins are located
    pub fn new(plugin_dir: PathBuf) -> Self {
        Self {
            input_plugins: Vec::new(),
            output_plugins: Vec::new(),
            dsp_plugins: Vec::new(),
            plugin_dir,
        }
    }

    /// Discovers and loads all plugins from the plugin directory.
    /// This should be called once at application startup.
    pub fn discover_plugins(&mut self) -> PluginResult<()> {
        if !self.plugin_dir.exists() {
            eprintln!(
                "Plugin directory not found: {:?}",
                self.plugin_dir
            );
            return Ok(());
        }

        // TODO: Implement dynamic plugin loading from .so/.dll files
        // For now, only built-in plugins are supported

        Ok(())
    }

    /// Registers a built-in input plugin.
    pub fn register_input_plugin(&mut self, plugin: Arc<dyn InputPlugin>) {
        self.input_plugins.push(plugin);
    }

    /// Registers a built-in output plugin.
    pub fn register_output_plugin(&mut self, plugin: Arc<dyn OutputPlugin>) {
        self.output_plugins.push(plugin);
    }

    /// Registers a built-in DSP plugin.
    pub fn register_dsp_plugin(&mut self, plugin: Arc<dyn DSPPlugin>) {
        self.dsp_plugins.push(plugin);
    }

    /// Finds an input plugin that can handle the given file.
    ///
    /// # Arguments
    /// * `path` - Path to the audio file
    ///
    /// # Returns
    /// The first plugin that can handle the file, or None if no plugin is found.
    pub fn find_input_plugin(&self, path: &Path) -> Option<Arc<dyn InputPlugin>> {
        self.input_plugins
            .iter()
            .find(|plugin| plugin.can_handle(path))
            .cloned()
    }

    /// Gets all registered input plugins.
    pub fn input_plugins(&self) -> &[Arc<dyn InputPlugin>] {
        &self.input_plugins
    }

    /// Gets all registered output plugins.
    pub fn output_plugins(&self) -> &[Arc<dyn OutputPlugin>] {
        &self.output_plugins
    }

    /// Gets all registered DSP plugins.
    pub fn dsp_plugins(&self) -> &[Arc<dyn DSPPlugin>] {
        &self.dsp_plugins
    }

    /// Returns the number of registered input plugins.
    pub fn input_plugin_count(&self) -> usize {
        self.input_plugins.len()
    }

    /// Returns the number of registered output plugins.
    pub fn output_plugin_count(&self) -> usize {
        self.output_plugins.len()
    }

    /// Returns the number of registered DSP plugins.
    pub fn dsp_plugin_count(&self) -> usize {
        self.dsp_plugins.len()
    }

    /// Gets an input plugin by index.
    pub fn get_input_plugin(&self, index: usize) -> Option<Arc<dyn InputPlugin>> {
        self.input_plugins.get(index).cloned()
    }

    /// Gets an output plugin by index.
    pub fn get_output_plugin(&self, index: usize) -> Option<Arc<dyn OutputPlugin>> {
        self.output_plugins.get(index).cloned()
    }

    /// Gets a DSP plugin by index.
    pub fn get_dsp_plugin(&self, index: usize) -> Option<Arc<dyn DSPPlugin>> {
        self.dsp_plugins.get(index).cloned()
    }

    /// Finds an input plugin by name.
    pub fn find_input_plugin_by_name(&self, name: &str) -> Option<Arc<dyn InputPlugin>> {
        self.input_plugins
            .iter()
            .find(|plugin| plugin.name() == name)
            .cloned()
    }

    /// Finds an output plugin by name.
    pub fn find_output_plugin_by_name(&self, name: &str) -> Option<Arc<dyn OutputPlugin>> {
        self.output_plugins
            .iter()
            .find(|plugin| plugin.name() == name)
            .cloned()
    }

    /// Finds a DSP plugin by name.
    pub fn find_dsp_plugin_by_name(&self, name: &str) -> Option<Arc<dyn DSPPlugin>> {
        self.dsp_plugins
            .iter()
            .find(|plugin| plugin.name() == name)
            .cloned()
    }

    /// Returns the plugin directory path.
    pub fn plugin_dir(&self) -> &Path {
        &self.plugin_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = PluginRegistry::new(PathBuf::from("/tmp/plugins"));
        assert_eq!(registry.input_plugin_count(), 0);
        assert_eq!(registry.output_plugin_count(), 0);
        assert_eq!(registry.dsp_plugin_count(), 0);
    }

    #[test]
    fn test_registry_discover_plugins() {
        let mut registry = PluginRegistry::new(PathBuf::from("/nonexistent"));
        let result = registry.discover_plugins();
        assert!(result.is_ok());
    }
}
