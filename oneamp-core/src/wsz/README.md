# WSZ Skin Loader

This module provides support for loading Winamp 2.x/5.x skin files (`.wsz` format) in OneAmp.

## Overview

The WSZ format is a ZIP archive containing BMP images and optional configuration files that define the visual appearance of the Winamp player interface.

## Usage

### Basic Loading

```rust
use oneamp_core::wsz::WszLoader;

let skin = WszLoader::load_from_file("path/to/skin.wsz")?;

println!("Loaded skin: {}", skin.metadata.name);
println!("Components: {}", skin.bitmaps.len());
```

### Loading from Memory

```rust
use oneamp_core::wsz::WszLoader;

let wsz_data: Vec<u8> = std::fs::read("skin.wsz")?;
let skin = WszLoader::load_from_bytes(&wsz_data)?;
```

### Accessing Skin Components

```rust
use oneamp_core::wsz::{WszLoader, SkinComponent};

let skin = WszLoader::load_from_file("skin.wsz")?;

if let Some(main_bitmap) = skin.get_bitmap(&SkinComponent::Main) {
    println!("Main window size: {}x{}", main_bitmap.width, main_bitmap.height);
}

if let Some(buttons) = skin.get_bitmap(&SkinComponent::CButtons) {
    let play_button = buttons.extract_region(0, 0, 23, 18)?;
}
```

### Extracting Bitmap Regions

```rust
use oneamp_core::wsz::{WszLoader, SkinComponent};

let skin = WszLoader::load_from_file("skin.wsz")?;
let main = skin.get_bitmap(&SkinComponent::Main).unwrap();

let title_region = main.extract_region(0, 0, 275, 14)?;
```

### Working with Regions

```rust
use oneamp_core::wsz::WszLoader;

let skin = WszLoader::load_from_file("skin.wsz")?;

for region in &skin.regions {
    println!("Region: {}", region.name);
    println!("Points: {}", region.points.len());

    if region.is_point_inside(100, 50) {
        println!("Point (100, 50) is inside region {}", region.name);
    }
}
```

## Architecture

### Modules

- **`loader.rs`**: Main WSZ file loading and extraction
- **`skin.rs`**: Skin data structures and metadata
- **`bitmap.rs`**: Bitmap processing and region extraction
- **`region.rs`**: Region.txt parser for custom window shapes

### Key Structures

#### `WszSkin`

Main structure representing a loaded Winamp skin.

```rust
pub struct WszSkin {
    pub metadata: SkinMetadata,
    pub bitmaps: HashMap<SkinComponent, BitmapAtlas>,
    pub regions: Vec<Region>,
}
```

#### `SkinComponent`

Enum representing standard Winamp skin components:

- `Main` - Main window background (275×116)
- `CButtons` - Control buttons (Previous, Play, Pause, Stop, Next, Eject)
- `MonoSter` - VU-meter display
- `Numbers` - Digital display characters
- `PlayPaus` - Play/Pause indicator
- `PosBar` - Position slider
- `TitleBar` - Window title bar
- `Volume` - Volume slider
- `Balance` - Balance slider
- `Pledit` - Playlist editor window
- `EqMain` - Equalizer window

#### `BitmapAtlas`

Stores raw RGBA pixel data from a BMP image.

```rust
pub struct BitmapAtlas {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,  // RGBA format, 4 bytes per pixel
}
```

Key features:
- Automatic transparency handling (magenta RGB(255,0,255) → transparent)
- Region extraction for sprites and button states
- Conversion to RGBA image format

#### `Region`

Defines custom clickable/drawable regions with polygon hit testing.

```rust
pub struct Region {
    pub name: String,
    pub points: Vec<RegionPoint>,
}
```

Supports:
- Point-in-polygon testing
- Custom window shapes
- Non-rectangular clickable areas

## Standard Winamp Skin Layout

### Main Window (275×116 pixels)

```
┌─────────────────────────────────────────┐
│ Title Bar (14px height)                 │  ← titlebar.bmp
├─────────────────────────────────────────┤
│ [Visual] [Time] [Info]                  │
│                                          │
│ [━━━━━━━━━━━━━━━━━━] Position          │  ← posbar.bmp
│                                          │
│ [◀][▶][■][⏸][⏭][⏮] Controls            │  ← cbuttons.bmp
│                                          │
│ Volume │ Balance │ EQ │ PL │ [○] [≡]   │  ← volume.bmp, balance.bmp
└─────────────────────────────────────────┘
```

### Component Dimensions

| Component | Width | Height | Frames | Notes |
|-----------|-------|--------|--------|-------|
| Main | 275 | 116 | 1 | Background |
| CButtons | 23 | 18 | 5×2 | 5 buttons × 2 states |
| Numbers | 9 | 13 | 11 | 0-9, colon, minus |
| PosBar | 248 | 10 | 29 | Position slider frames |
| Volume | 68 | 13 | 28 | Volume slider |
| Balance | 38 | 13 | 28 | Balance slider |

## Transparency

Winamp skins use magenta (RGB 255, 0, 255) as the transparency color. The loader automatically converts this to alpha transparency:

```rust
let mut atlas = BitmapAtlas::from_bytes(&bmp_data)?;
atlas.apply_transparency();  // Magenta → transparent
```

## Region.txt Format

The `region.txt` file defines custom window shapes using polygon coordinates:

```ini
; Comments start with semicolon or hash
[WindowName]
x1, y1
x2, y2
x3, y3
...

[AnotherWindow]
x1 y1
x2 y2
...
```

Example:

```ini
[MainWindow]
0, 0
275, 0
275, 116
0, 116

[PlayButton]
16, 88
39, 88
39, 106
16, 106
```

## Error Handling

The loader includes comprehensive error handling:

- Missing main.bmp → Error (required component)
- Missing optional components → Warning, continues loading
- Invalid BMP data → Warning, skips component
- Corrupted region.txt → Warning, ignores regions
- Invalid ZIP → Error

## Testing

The module includes extensive unit tests. Run them with:

```bash
cargo test --package oneamp-core wsz
```

Test coverage:
- ✓ Component identification from filenames
- ✓ BMP loading and transparency
- ✓ Region extraction from bitmaps
- ✓ Region.txt parsing
- ✓ Point-in-polygon testing
- ✓ WSZ archive loading
- ✓ Metadata extraction

## Performance Considerations

- **Lazy loading**: Only requested bitmap regions are extracted
- **Memory efficiency**: Bitmaps stored in compressed RGBA format
- **Caching**: Skin components cached in HashMap for O(1) access
- **Transparency**: Processed once during load, not on every render

## Future Enhancements

- [ ] Cursors support (`.cur` files)
- [ ] Animated components (multiple frames)
- [ ] Color themes (user-adjustable hues)
- [ ] Modern skin format (Winamp 3+ XML-based)
- [ ] Skin preview thumbnails
- [ ] Hot-reloading for development

## Examples

See `/tests/integration_test.rs` for more examples of loading and using WSZ skins.

## Resources

- [Winamp Skinning Guide](http://wiki.winamp.com/wiki/Skinning)
- [WSZ Format Specification](http://wiki.winamp.com/wiki/WSZ_Format)
- [Classic Skin Archive](https://skins.webamp.org/)
