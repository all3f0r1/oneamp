# OneAmp Plugin System

This directory contains example plugins for the OneAmp audio player.

## Plugin Categories

- **Input Plugins:** Decode audio files (e.g., `input-aac`)
- **DSP Plugins:** Apply audio effects (e.g., `dsp-reverb`)
- **Output Plugins:** Interface with audio hardware (not yet implemented)

## Building Plugins

To build a plugin, navigate to its directory and run:

```bash
cargo build --release
```

This will produce a shared library file (`.so`, `.dll`, `.dylib`) in the `target/release` directory.

## Installing Plugins

Copy the shared library file to the OneAmp plugins directory:

```bash
cp target/release/oneamp_input_aac.so ~/.config/oneamp/plugins/
```

## Creating a New Plugin

1. Create a new directory in `oneamp-plugins`
2. Create a `Cargo.toml` file with `crate-type = ["cdylib"]`
3. Implement the appropriate plugin trait from `oneamp-core`
4. Export a `create_<plugin_type>_plugin()` function

