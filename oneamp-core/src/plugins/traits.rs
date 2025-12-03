// Plugin Trait Definitions
// Defines the interfaces that all plugins must implement.

use std::path::Path;
use super::error::PluginResult;

/// Metadata about an audio file.
#[derive(Debug, Clone)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: f32,
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: Option<u32>,
}

impl Default for AudioMetadata {
    fn default() -> Self {
        Self {
            title: None,
            artist: None,
            album: None,
            duration: 0.0,
            sample_rate: 44100,
            channels: 2,
            bitrate: None,
        }
    }
}

/// Raw audio data buffer.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioBuffer {
    /// Creates a new audio buffer.
    pub fn new(sample_rate: u32, channels: u16, capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            sample_rate,
            channels,
        }
    }

    /// Returns the number of frames in the buffer.
    pub fn frame_count(&self) -> usize {
        if self.channels > 0 {
            self.samples.len() / self.channels as usize
        } else {
            0
        }
    }

    /// Clears the buffer.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Trait for audio decoders (input plugins).
/// Implementations handle decoding compressed audio formats into PCM samples.
pub trait AudioDecoder: Send + Sync {
    /// Returns metadata about the audio file.
    fn metadata(&self) -> &AudioMetadata;

    /// Decodes the next chunk of audio data.
    /// Returns None when the end of file is reached.
    fn decode_next(&mut self) -> PluginResult<Option<AudioBuffer>>;

    /// Seeks to a specific position in seconds.
    fn seek(&mut self, position: f32) -> PluginResult<()>;

    /// Returns the current playback position in seconds.
    fn position(&self) -> f32;
}

/// Trait for input plugins (audio decoders).
/// Implementations handle opening and decoding audio files in various formats.
pub trait InputPlugin: Send + Sync {
    /// Returns the name of the plugin.
    fn name(&self) -> &str;

    /// Returns the version of the plugin.
    fn version(&self) -> &str;

    /// Returns the supported file extensions (e.g., ["aac", "m4a"]).
    fn supported_formats(&self) -> Vec<&str>;

    /// Checks if this plugin can handle the given file.
    fn can_handle(&self, path: &Path) -> bool;

    /// Opens and prepares an audio file for decoding.
    fn open(&self, path: &Path) -> PluginResult<Box<dyn AudioDecoder>>;
}

/// Information about an audio device.
#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub sample_rates: Vec<u32>,
}

/// Audio configuration for output devices.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            channels: 2,
            buffer_size: 2048,
        }
    }
}

/// Trait for audio output streams.
/// Implementations handle writing PCM samples to audio devices.
pub trait AudioOutput: Send + Sync {
    /// Writes audio samples to the device.
    fn write(&mut self, samples: &[f32]) -> PluginResult<()>;

    /// Flushes any pending audio data.
    fn flush(&mut self) -> PluginResult<()>;

    /// Pauses playback.
    fn pause(&mut self) -> PluginResult<()>;

    /// Resumes playback.
    fn resume(&mut self) -> PluginResult<()>;

    /// Returns the current playback latency in milliseconds.
    fn latency(&self) -> u32;
}

/// Trait for output plugins (audio devices).
/// Implementations handle interfacing with system audio hardware.
pub trait OutputPlugin: Send + Sync {
    /// Returns the name of the plugin.
    fn name(&self) -> &str;

    /// Returns the version of the plugin.
    fn version(&self) -> &str;

    /// Lists available audio devices.
    fn list_devices(&self) -> PluginResult<Vec<AudioDevice>>;

    /// Opens an audio device for playback.
    fn open(&self, device: &AudioDevice, config: &AudioConfig) -> PluginResult<Box<dyn AudioOutput>>;
}

/// Information about a DSP plugin parameter.
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: String,
}

/// Trait for DSP audio processors.
/// Implementations apply effects or transformations to audio data.
pub trait DSPProcessor: Send + Sync {
    /// Processes an audio buffer in-place.
    fn process(&mut self, buffer: &mut AudioBuffer) -> PluginResult<()>;

    /// Sets a parameter value (e.g., "decay_time" = 0.5).
    fn set_parameter(&mut self, name: &str, value: f32) -> PluginResult<()>;

    /// Gets a parameter value.
    fn get_parameter(&self, name: &str) -> PluginResult<f32>;

    /// Returns a list of available parameters.
    fn parameters(&self) -> Vec<ParameterInfo>;

    /// Enables or disables the effect.
    fn set_enabled(&mut self, enabled: bool);

    /// Resets the internal state of the processor.
    fn reset(&mut self) -> PluginResult<()>;
}

/// Trait for DSP plugins (effects).
/// Implementations provide audio processing effects like reverb, compression, etc.
pub trait DSPPlugin: Send + Sync {
    /// Returns the name of the plugin.
    fn name(&self) -> &str;

    /// Returns the version of the plugin.
    fn version(&self) -> &str;

    /// Returns the category of the effect (e.g., "Reverb", "Compression").
    fn category(&self) -> &str;

    /// Creates a new instance of the DSP processor.
    fn create_processor(&self) -> PluginResult<Box<dyn DSPProcessor>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_metadata_default() {
        let metadata = AudioMetadata::default();
        assert_eq!(metadata.sample_rate, 44100);
        assert_eq!(metadata.channels, 2);
        assert_eq!(metadata.duration, 0.0);
    }

    #[test]
    fn test_audio_buffer_creation() {
        let buffer = AudioBuffer::new(44100, 2, 1024);
        assert_eq!(buffer.sample_rate, 44100);
        assert_eq!(buffer.channels, 2);
        assert_eq!(buffer.frame_count(), 0);
    }

    #[test]
    fn test_audio_buffer_frame_count() {
        let mut buffer = AudioBuffer::new(44100, 2, 1024);
        buffer.samples = vec![0.0; 4410]; // 2205 frames at 2 channels
        assert_eq!(buffer.frame_count(), 2205);
    }

    #[test]
    fn test_audio_config_default() {
        let config = AudioConfig::default();
        assert_eq!(config.sample_rate, 44100);
        assert_eq!(config.channels, 2);
        assert_eq!(config.buffer_size, 2048);
    }

    #[test]
    fn test_parameter_info_creation() {
        let param = ParameterInfo {
            name: "decay_time".to_string(),
            min: 0.0,
            max: 10.0,
            default: 2.0,
            unit: "seconds".to_string(),
        };
        assert_eq!(param.name, "decay_time");
        assert_eq!(param.max, 10.0);
    }
}
