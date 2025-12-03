// OneAmp Plugin System
// Provides a flexible architecture for extending OneAmp with custom audio codecs,
// output devices, and digital signal processing effects.

pub mod error;
pub mod loader;
pub mod registry;
pub mod traits;

pub use error::{PluginError, PluginResult};
pub use loader::PluginLoader;
pub use registry::PluginRegistry;
pub use traits::{
    AudioBuffer, AudioConfig, AudioDecoder, AudioDevice, AudioMetadata, AudioOutput, DSPPlugin,
    DSPProcessor, InputPlugin, OutputPlugin, ParameterInfo,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_error_display() {
        let error = PluginError::FormatNotSupported("MP4".to_string());
        assert_eq!(error.to_string(), "Format not supported: MP4");
    }
}
