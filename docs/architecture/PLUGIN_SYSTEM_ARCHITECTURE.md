# OneAmp Plugin System Architecture

**Author:** Manus AI  
**Date:** December 3, 2025  
**Version:** 1.0

---

## 1. Overview

This document outlines the architecture for a plugin system that will allow OneAmp to be extended with custom audio codecs, output devices, and digital signal processing effects. The plugin system is designed to be flexible, safe, and maintainable while leveraging Rust's type system and trait-based design.

### Goals

- **Extensibility:** Add new audio formats, output devices, and effects without modifying core code
- **Safety:** Prevent plugin crashes from affecting the main application
- **Performance:** Minimal overhead for plugin loading and execution
- **Ease of Use:** Simple API for plugin developers to implement
- **Modularity:** Plugins can be developed, tested, and distributed independently

---

## 2. Plugin Categories

The plugin system will support three main categories of plugins:

### 2.1 Input Plugins

Input plugins are responsible for decoding audio files in various formats. They take a file path or stream as input and produce raw PCM audio data.

**Responsibilities:**
- Detect audio format from file extension or magic bytes
- Decode compressed audio (MP3, AAC, OPUS, WMA, etc.)
- Extract metadata (title, artist, duration, sample rate, channels)
- Support seeking within the file
- Handle errors gracefully

**Example Use Cases:**
- AAC decoder (for .m4a, .aac files)
- OPUS decoder (for .opus files)
- WMA decoder (for .wma files)
- FLAC decoder (alternative to symphonia)

### 2.2 Output Plugins

Output plugins handle audio playback by interfacing with the system's audio hardware and drivers.

**Responsibilities:**
- Initialize audio device
- Configure sample rate, channels, and buffer size
- Write PCM samples to audio device
- Handle device disconnection/reconnection
- Manage latency and buffering

**Example Use Cases:**
- WASAPI output (Windows)
- PulseAudio output (Linux)
- ALSA output (Linux alternative)
- CoreAudio output (macOS)
- Jack output (professional audio)

### 2.3 DSP Plugins

DSP (Digital Signal Processing) plugins apply effects or transformations to audio data.

**Responsibilities:**
- Process audio buffers in real-time
- Maintain internal state (e.g., filter coefficients)
- Handle different sample rates and channel configurations
- Provide parameter adjustment (e.g., reverb decay time)
- Support bypass/enable toggling

**Example Use Cases:**
- Reverb effect
- Compression effect
- Parametric EQ
- Limiter
- Noise gate
- Stereo widening

---

## 3. Plugin Trait Definitions

Each plugin category will have a corresponding trait that plugins must implement.

### 3.1 InputPlugin Trait

```rust
pub trait InputPlugin: Send + Sync {
    /// Returns the name of the plugin
    fn name(&self) -> &str;
    
    /// Returns the version of the plugin
    fn version(&self) -> &str;
    
    /// Returns the supported file extensions (e.g., ["aac", "m4a"])
    fn supported_formats(&self) -> Vec<&str>;
    
    /// Checks if this plugin can handle the given file
    fn can_handle(&self, path: &Path) -> bool;
    
    /// Opens and decodes an audio file
    fn open(&self, path: &Path) -> Result<Box<dyn AudioDecoder>>;
}

pub trait AudioDecoder: Send + Sync {
    /// Returns metadata about the audio file
    fn metadata(&self) -> &AudioMetadata;
    
    /// Decodes the next chunk of audio data
    fn decode_next(&mut self) -> Result<Option<AudioBuffer>>;
    
    /// Seeks to a specific position in seconds
    fn seek(&mut self, position: f32) -> Result<()>;
    
    /// Returns the current playback position in seconds
    fn position(&self) -> f32;
}

pub struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: f32,
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: Option<u32>,
}

pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}
```

### 3.2 OutputPlugin Trait

```rust
pub trait OutputPlugin: Send + Sync {
    /// Returns the name of the plugin
    fn name(&self) -> &str;
    
    /// Returns the version of the plugin
    fn version(&self) -> &str;
    
    /// Lists available audio devices
    fn list_devices(&self) -> Result<Vec<AudioDevice>>;
    
    /// Opens an audio device for playback
    fn open(&self, device: &AudioDevice, config: &AudioConfig) -> Result<Box<dyn AudioOutput>>;
}

pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub sample_rates: Vec<u32>,
}

pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: u32,
}

pub trait AudioOutput: Send + Sync {
    /// Writes audio samples to the device
    fn write(&mut self, samples: &[f32]) -> Result<()>;
    
    /// Flushes any pending audio data
    fn flush(&mut self) -> Result<()>;
    
    /// Pauses playback
    fn pause(&mut self) -> Result<()>;
    
    /// Resumes playback
    fn resume(&mut self) -> Result<()>;
    
    /// Returns the current playback latency in milliseconds
    fn latency(&self) -> u32;
}
```

### 3.3 DSPPlugin Trait

```rust
pub trait DSPPlugin: Send + Sync {
    /// Returns the name of the plugin
    fn name(&self) -> &str;
    
    /// Returns the version of the plugin
    fn version(&self) -> &str;
    
    /// Returns the category of the effect (e.g., "Reverb", "Compression")
    fn category(&self) -> &str;
    
    /// Creates a new instance of the DSP processor
    fn create_processor(&self) -> Result<Box<dyn DSPProcessor>>;
}

pub trait DSPProcessor: Send + Sync {
    /// Processes an audio buffer
    fn process(&mut self, buffer: &mut AudioBuffer) -> Result<()>;
    
    /// Sets a parameter value (e.g., "decay_time" = 0.5)
    fn set_parameter(&mut self, name: &str, value: f32) -> Result<()>;
    
    /// Gets a parameter value
    fn get_parameter(&self, name: &str) -> Result<f32>;
    
    /// Returns a list of available parameters
    fn parameters(&self) -> Vec<ParameterInfo>;
    
    /// Enables or disables the effect
    fn set_enabled(&mut self, enabled: bool);
    
    /// Resets the internal state of the processor
    fn reset(&mut self) -> Result<()>;
}

pub struct ParameterInfo {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: String,
}
```

---

## 4. Plugin Registry and Management

The `PluginRegistry` is the central component that manages plugin discovery, loading, and lifecycle.

### 4.1 PluginRegistry Structure

```rust
pub struct PluginRegistry {
    input_plugins: Vec<Arc<dyn InputPlugin>>,
    output_plugins: Vec<Arc<dyn OutputPlugin>>,
    dsp_plugins: Vec<Arc<dyn DSPPlugin>>,
    plugin_dir: PathBuf,
}

impl PluginRegistry {
    /// Creates a new plugin registry
    pub fn new(plugin_dir: PathBuf) -> Self { ... }
    
    /// Discovers and loads all plugins from the plugin directory
    pub fn discover_plugins(&mut self) -> Result<()> { ... }
    
    /// Registers a built-in plugin
    pub fn register_input_plugin(&mut self, plugin: Arc<dyn InputPlugin>) { ... }
    pub fn register_output_plugin(&mut self, plugin: Arc<dyn OutputPlugin>) { ... }
    pub fn register_dsp_plugin(&mut self, plugin: Arc<dyn DSPPlugin>) { ... }
    
    /// Finds a plugin that can handle a specific file
    pub fn find_input_plugin(&self, path: &Path) -> Option<Arc<dyn InputPlugin>> { ... }
    
    /// Gets all available output plugins
    pub fn output_plugins(&self) -> &[Arc<dyn OutputPlugin>] { ... }
    
    /// Gets all available DSP plugins
    pub fn dsp_plugins(&self) -> &[Arc<dyn DSPPlugin>] { ... }
}
```

### 4.2 Plugin Discovery

Plugins can be discovered in two ways:

1. **Built-in Plugins:** Compiled directly into OneAmp (e.g., symphonia-based decoders)
2. **External Plugins:** Loaded from dynamic libraries (.so, .dll, .dylib files)

For external plugins, the registry will:
1. Scan the `~/.config/oneamp/plugins/` directory
2. Load each `.so`/`.dll`/`.dylib` file
3. Call a standardized entry point function to retrieve the plugin
4. Validate the plugin implements the correct trait

---

## 5. Plugin Loading Mechanism

External plugins will use Rust's `libloading` crate to dynamically load shared libraries.

### 5.1 Plugin Entry Point

Each external plugin must export a C-compatible function:

```rust
#[no_mangle]
pub extern "C" fn create_plugin() -> *mut dyn InputPlugin {
    let plugin = MyAACPlugin::new();
    let boxed: Box<dyn InputPlugin> = Box::new(plugin);
    Box::into_raw(boxed)
}
```

### 5.2 Loading Process

```rust
pub fn load_plugin_from_file(path: &Path) -> Result<Arc<dyn InputPlugin>> {
    unsafe {
        let lib = libloading::Library::new(path)?;
        let constructor: libloading::Symbol<unsafe extern "C" fn() -> *mut dyn InputPlugin> =
            lib.get(b"create_plugin")?;
        let raw_plugin = constructor();
        let plugin = Box::from_raw(raw_plugin);
        Ok(Arc::new(*plugin))
    }
}
```

---

## 6. Error Handling

Plugins should use a standardized error type:

```rust
pub type PluginResult<T> = Result<T, PluginError>;

#[derive(Debug)]
pub enum PluginError {
    FormatNotSupported(String),
    FileNotFound(String),
    DecodingError(String),
    DeviceNotFound(String),
    ConfigurationError(String),
    ProcessingError(String),
    Other(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::FormatNotSupported(msg) => write!(f, "Format not supported: {}", msg),
            PluginError::FileNotFound(msg) => write!(f, "File not found: {}", msg),
            // ... other variants
        }
    }
}

impl std::error::Error for PluginError {}
```

---

## 7. Integration with OneAmp

### 7.1 Audio Engine Integration

The `AudioEngine` will be modified to use the plugin registry:

```rust
pub struct AudioEngine {
    plugin_registry: PluginRegistry,
    current_input_plugin: Option<Arc<dyn InputPlugin>>,
    current_output_plugin: Option<Arc<dyn OutputPlugin>>,
    dsp_chain: Vec<Box<dyn DSPProcessor>>,
    // ... other fields
}

impl AudioEngine {
    pub fn new(plugin_dir: PathBuf) -> Result<Self> {
        let mut registry = PluginRegistry::new(plugin_dir);
        registry.discover_plugins()?;
        
        // Register built-in plugins
        registry.register_input_plugin(Arc::new(SymphoniaInputPlugin::new()));
        
        Ok(Self {
            plugin_registry: registry,
            // ...
        })
    }
    
    pub fn load_file(&mut self, path: &Path) -> Result<TrackInfo> {
        let plugin = self.plugin_registry
            .find_input_plugin(path)
            .ok_or("No plugin found for this file")?;
        
        let decoder = plugin.open(path)?;
        // ... rest of loading logic
    }
}
```

### 7.2 DSP Chain

The DSP chain will be managed as a vector of processors:

```rust
impl AudioEngine {
    pub fn add_dsp_effect(&mut self, plugin: Arc<dyn DSPPlugin>) -> Result<()> {
        let processor = plugin.create_processor()?;
        self.dsp_chain.push(processor);
        Ok(())
    }
    
    pub fn remove_dsp_effect(&mut self, index: usize) {
        if index < self.dsp_chain.len() {
            self.dsp_chain.remove(index);
        }
    }
    
    pub fn process_audio(&mut self, buffer: &mut AudioBuffer) -> Result<()> {
        for processor in &mut self.dsp_chain {
            processor.process(buffer)?;
        }
        Ok(())
    }
}
```

---

## 8. Directory Structure

After implementation, the plugin system will have the following structure:

```
oneamp/
├── oneamp-core/
│   └── src/
│       ├── plugins/
│       │   ├── mod.rs
│       │   ├── registry.rs
│       │   ├── traits.rs
│       │   ├── error.rs
│       │   └── loader.rs
│       └── lib.rs
├── oneamp-plugins/
│   ├── input-aac/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── output-pulseaudio/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── dsp-reverb/
│       ├── Cargo.toml
│       └── src/lib.rs
└── plugins/
    └── (external plugin .so/.dll files)
```

---

## 9. Dependencies

The plugin system will require:

```toml
[dependencies]
libloading = "0.8"  # For dynamic library loading
```

---

## 10. Security Considerations

- **Sandboxing:** Plugins run in the same process as OneAmp. Consider using `seccomp` or similar mechanisms for sandboxing in the future.
- **Signature Verification:** Optionally verify plugin signatures to prevent malicious code.
- **Resource Limits:** Monitor plugin resource usage (CPU, memory) and kill runaway plugins.
- **API Versioning:** Version the plugin API to maintain compatibility across OneAmp versions.

---

## 11. Future Enhancements

- **Plugin Marketplace:** A central repository for discovering and downloading plugins
- **Plugin Configuration UI:** Built-in UI for configuring plugin parameters
- **Plugin Metrics:** Monitor plugin performance and resource usage
- **Hot Reloading:** Reload plugins without restarting the application
- **Plugin Sandboxing:** Run plugins in isolated processes for safety

