// Plugin Error Handling
// Defines error types and result types for the plugin system.

use std::error::Error;
use std::fmt;

/// Result type for plugin operations.
pub type PluginResult<T> = Result<T, PluginError>;

/// Error type for plugin operations.
#[derive(Debug, Clone)]
pub enum PluginError {
    /// The audio format is not supported by any plugin.
    FormatNotSupported(String),

    /// The specified file was not found.
    FileNotFound(String),

    /// An error occurred while decoding audio.
    DecodingError(String),

    /// The specified audio device was not found.
    DeviceNotFound(String),

    /// An error occurred while configuring an audio device.
    ConfigurationError(String),

    /// An error occurred while processing audio.
    ProcessingError(String),

    /// An invalid parameter was provided.
    InvalidParameter(String),

    /// The plugin is not initialized.
    NotInitialized(String),

    /// A generic error.
    Other(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::FormatNotSupported(msg) => {
                write!(f, "Format not supported: {}", msg)
            }
            PluginError::FileNotFound(msg) => {
                write!(f, "File not found: {}", msg)
            }
            PluginError::DecodingError(msg) => {
                write!(f, "Decoding error: {}", msg)
            }
            PluginError::DeviceNotFound(msg) => {
                write!(f, "Device not found: {}", msg)
            }
            PluginError::ConfigurationError(msg) => {
                write!(f, "Configuration error: {}", msg)
            }
            PluginError::ProcessingError(msg) => {
                write!(f, "Processing error: {}", msg)
            }
            PluginError::InvalidParameter(msg) => {
                write!(f, "Invalid parameter: {}", msg)
            }
            PluginError::NotInitialized(msg) => {
                write!(f, "Not initialized: {}", msg)
            }
            PluginError::Other(msg) => {
                write!(f, "Error: {}", msg)
            }
        }
    }
}

impl Error for PluginError {}

impl From<String> for PluginError {
    fn from(msg: String) -> Self {
        PluginError::Other(msg)
    }
}

impl From<&str> for PluginError {
    fn from(msg: &str) -> Self {
        PluginError::Other(msg.to_string())
    }
}

impl From<std::io::Error> for PluginError {
    fn from(err: std::io::Error) -> Self {
        PluginError::Other(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_error_display() {
        let error = PluginError::FormatNotSupported("MP4".to_string());
        assert_eq!(error.to_string(), "Format not supported: MP4");
    }

    #[test]
    fn test_plugin_error_from_string() {
        let error: PluginError = "test error".into();
        assert_eq!(error.to_string(), "Error: test error");
    }

    #[test]
    fn test_plugin_error_from_io_error() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error: PluginError = io_error.into();
        assert!(error.to_string().contains("file not found"));
    }

    #[test]
    fn test_plugin_result_ok() {
        let result: PluginResult<i32> = Ok(42);
        assert!(result.is_ok());
        if let Ok(value) = result {
            assert_eq!(value, 42);
        }
    }

    #[test]
    fn test_plugin_result_err() {
        let result: PluginResult<i32> = Err(PluginError::Other("error".to_string()));
        assert!(result.is_err());
    }
}
