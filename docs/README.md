# OneAmp Documentation

Welcome to the OneAmp documentation. This directory contains comprehensive guides, architecture documentation, and API references for developers.

## Quick Navigation

### Architecture & Design

- **[Skin System Architecture](./architecture/SKIN_SYSTEM_ARCHITECTURE.md)** - Complete design of the TOML-based skinning system
- **[Plugin System Architecture](./architecture/PLUGIN_SYSTEM_ARCHITECTURE.md)** - Extensible plugin framework for audio processing

### Integration Guides

- **[Skin Integration Guide](./guides/SKIN_INTEGRATION_GUIDE.md)** - Step-by-step integration of the skin system into OneAmp
- **[Plugin Integration Guide](./guides/PLUGIN_INTEGRATION_GUIDE.md)** - How to integrate plugins into the audio engine

### API References

- **[Skins API](./api/SKINS_API.md)** - Complete API reference for the skin system
- **[Plugins API](./api/PLUGINS_API.md)** - Plugin development guide and API reference

### Examples

The `examples/` directory contains sample code and templates for creating custom skins and plugins.

## Project Structure

```
docs/
├── architecture/     # System design and architecture documents
├── guides/          # Integration and implementation guides
├── api/             # API references and technical documentation
├── examples/        # Example code and templates
└── README.md        # This file
```

## Getting Started

### For Users

If you want to customize OneAmp's appearance, start with the [Skin Integration Guide](./guides/SKIN_INTEGRATION_GUIDE.md).

### For Developers

If you want to extend OneAmp's functionality with plugins, start with the [Plugin System Architecture](./architecture/PLUGIN_SYSTEM_ARCHITECTURE.md) and then follow the [Plugin Integration Guide](./guides/PLUGIN_INTEGRATION_GUIDE.md).

## Key Features

### Skin System

OneAmp includes a flexible, TOML-based skinning system that allows complete customization of the application's appearance. The system includes:

- **Default Skins:** OneAmp Dark (modern) and Winamp5 Classified (classic)
- **Easy Customization:** Simple TOML format for creating custom skins
- **Dynamic Loading:** Skins are discovered and loaded at runtime
- **Extensible:** Support for custom fonts, colors, and metrics

### Plugin System

The plugin system provides extensibility for audio processing:

- **Input Plugins:** Decode audio files in various formats
- **Output Plugins:** Interface with audio hardware
- **DSP Plugins:** Apply audio effects and processing

## Version Information

This documentation covers OneAmp v0.14.0 and later.

## Contributing

When contributing to OneAmp, please ensure that:

1. All code follows Rust best practices (clippy, rustfmt)
2. Documentation is updated alongside code changes
3. New features include comprehensive documentation
4. Examples are provided for complex features

## License

OneAmp is licensed under the MIT License. See the LICENSE file in the root directory for details.

