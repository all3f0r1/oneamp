#![allow(dead_code, unused_imports, unused_variables)]

use std::path::Path;
use oneamp_core::plugins::traits::{InputPlugin, AudioDecoder, AudioMetadata, AudioBuffer};
use oneamp_core::plugins::error::{PluginError, PluginResult};

pub struct AACInputPlugin;

impl InputPlugin for AACInputPlugin {
    fn name(&self) -> &str {
        "AAC Input"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn supported_formats(&self) -> Vec<&str> {
        vec!["aac", "m4a"]
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| matches!(ext.to_lowercase().as_str(), "aac" | "m4a"))
            .unwrap_or(false)
    }

    fn open(&self, path: &Path) -> PluginResult<Box<dyn AudioDecoder>> {
        Err(PluginError::Other("AAC decoding not yet implemented".to_string()))
    }
}

#[no_mangle]
pub extern "C" fn create_input_plugin() -> *mut dyn InputPlugin {
    let plugin = AACInputPlugin;
    let boxed: Box<dyn InputPlugin> = Box::new(plugin);
    Box::into_raw(boxed)
}
