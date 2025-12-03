// OneAmp Plugin System
// Provides a flexible architecture for extending OneAmp with custom audio codecs,
// output devices, and digital signal processing effects.

pub mod traits;
pub mod error;
pub mod registry;
pub mod loader;

pub use traits::{
    InputPlugin, AudioDecoder, AudioMetadata, AudioBuffer,
    OutputPlugin, AudioDevice, AudioConfig, AudioOutput,
    DSPPlugin, DSPProcessor, ParameterInfo,
};
pub use error::{PluginError, PluginResult};
pub use registry::PluginRegistry;
pub use loader::PluginLoader;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_error_display() {
        let error = PluginError::FormatNotSupported("MP4".to_string());
        assert_eq!(
            error.to_string(),
            "Format not supported: MP4"
        );
    }
}
