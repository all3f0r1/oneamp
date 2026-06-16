use super::region::Region;
use anyhow::{Context, Result};
use image::{DynamicImage, Rgba, RgbaImage};

pub const TRANSPARENT_MAGENTA: Rgba<u8> = Rgba([255, 0, 255, 255]);

/// True when `(r, g, b)` is the Winamp magenta colour-key within ±1 per
/// channel. The canonical key is (255, 0, 255), but Winamp tolerates a
/// one-step deviation so skins that have been round-tripped through a lossy
/// recompressor (e.g. a JPEG re-export that nudges 255→254 or 0→1) still key
/// out transparently rather than rendering an opaque magenta border. Cheap
/// enough to inline in the per-pixel transparency loop.
#[inline]
fn is_magenta_key(r: u8, g: u8, b: u8) -> bool {
    r >= 254 && g <= 1 && b >= 254
}

#[derive(Debug, Clone)]
pub struct BitmapAtlas {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl BitmapAtlas {
    pub fn from_image(img: DynamicImage) -> Self {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        Self {
            width,
            height,
            data: rgba.into_raw(),
        }
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let img = image::load_from_memory(data).context("Failed to load image from bytes")?;
        Ok(Self::from_image(img))
    }

    pub fn apply_transparency(&mut self) {
        for chunk in self.data.chunks_exact_mut(4) {
            if is_magenta_key(chunk[0], chunk[1], chunk[2]) {
                chunk[3] = 0;
            }
        }
    }

    /// Set alpha=0 on every pixel whose RGB matches `key`. Used by text.bmp
    /// loading where many skins ship a non-magenta background (default Winamp
    /// 2.x skins paint glyphs as white text on a dark blue field rather than
    /// the standard magenta).
    pub fn apply_color_key(&mut self, key: [u8; 3]) {
        for chunk in self.data.chunks_exact_mut(4) {
            if chunk[0] == key[0] && chunk[1] == key[1] && chunk[2] == key[2] {
                chunk[3] = 0;
            }
        }
    }

    /// Returns the RGB of the most common opaque pixel, or `None` if every
    /// pixel is already transparent. Skips already-keyed pixels so a follow-up
    /// `apply_color_key` doesn't pick the magenta we just made invisible.
    pub fn dominant_opaque_color(&self) -> Option<[u8; 3]> {
        use std::collections::HashMap;
        let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
        for chunk in self.data.chunks_exact(4) {
            if chunk[3] == 0 {
                continue;
            }
            *counts.entry([chunk[0], chunk[1], chunk[2]]).or_insert(0) += 1;
        }
        counts.into_iter().max_by_key(|&(_, n)| n).map(|(c, _)| c)
    }

    /// Set alpha=0 on every pixel that falls *outside* the polygon union
    /// described by `region`. Used to apply the `[Normal]` mask from a skin's
    /// `region.txt` so the main window appears with chamfered/rounded
    /// corners on a transparent viewport.
    ///
    /// No-op when the region is empty (skin doesn't override the default
    /// rectangular shape).
    pub fn apply_region_mask(&mut self, region: &Region) {
        let h = self.height;
        let w = self.width;
        self.apply_region_mask_in_rect(region, 0, 0, w, h);
    }

    /// Same as `apply_region_mask` but only zeroes alpha for pixels inside the
    /// `(x0, y0, w, h)` rectangle. Used for atlases like `eqmain.bmp` where the
    /// window region only covers the top 116 rows but the atlas continues below
    /// with sprite frames that must remain opaque.
    pub fn apply_region_mask_in_rect(&mut self, region: &Region, x0: u32, y0: u32, w: u32, h: u32) {
        if region.is_empty() {
            return;
        }

        let aw = self.width as i32;
        let x_end = (x0 + w).min(self.width) as i32;
        let y_end = (y0 + h).min(self.height) as i32;
        for y in y0 as i32..y_end {
            for x in x0 as i32..x_end {
                if !region.contains(x - x0 as i32, y - y0 as i32) {
                    let idx = ((y * aw + x) * 4) as usize + 3;
                    if let Some(alpha) = self.data.get_mut(idx) {
                        *alpha = 0;
                    }
                }
            }
        }
    }

    pub fn extract_region(&self, x: u32, y: u32, width: u32, height: u32) -> Option<BitmapRegion> {
        if x + width > self.width || y + height > self.height {
            return None;
        }

        let mut region_data = Vec::with_capacity((width * height * 4) as usize);

        for row in y..(y + height) {
            let start_idx = ((row * self.width + x) * 4) as usize;
            let end_idx = start_idx + (width * 4) as usize;
            region_data.extend_from_slice(&self.data[start_idx..end_idx]);
        }

        Some(BitmapRegion {
            x,
            y,
            width,
            height,
            data: region_data,
        })
    }

    pub fn to_rgba_image(&self) -> RgbaImage {
        RgbaImage::from_raw(self.width, self.height, self.data.clone())
            .expect("Invalid image dimensions")
    }
}

#[derive(Debug, Clone)]
pub struct BitmapRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl BitmapRegion {
    pub fn to_rgba_image(&self) -> RgbaImage {
        RgbaImage::from_raw(self.width, self.height, self.data.clone())
            .expect("Invalid region dimensions")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Pressed,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct ButtonFrames {
    pub normal: BitmapRegion,
    pub pressed: Option<BitmapRegion>,
    pub disabled: Option<BitmapRegion>,
}

impl ButtonFrames {
    pub fn get_frame(&self, state: ButtonState) -> &BitmapRegion {
        match state {
            ButtonState::Normal => &self.normal,
            ButtonState::Pressed => self.pressed.as_ref().unwrap_or(&self.normal),
            ButtonState::Disabled => self.disabled.as_ref().unwrap_or(&self.normal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitmap_atlas_creation() {
        let img = DynamicImage::new_rgba8(100, 100);
        let atlas = BitmapAtlas::from_image(img);

        assert_eq!(atlas.width, 100);
        assert_eq!(atlas.height, 100);
        assert_eq!(atlas.data.len(), 100 * 100 * 4);
    }

    #[test]
    fn test_transparency_application() {
        let mut img_data = vec![0u8; 4 * 4];
        img_data[0] = 255;
        img_data[1] = 0;
        img_data[2] = 255;
        img_data[3] = 255;

        let mut atlas = BitmapAtlas {
            width: 2,
            height: 2,
            data: img_data,
        };

        atlas.apply_transparency();
        assert_eq!(atlas.data[3], 0);
    }

    #[test]
    fn test_magenta_tolerance_keys_within_one_step() {
        // Three opaque pixels: exact magenta, ±1 magenta, and a near-but-not
        // colour that must stay opaque.
        let mut data = Vec::new();
        data.extend_from_slice(&[255, 0, 255, 255]); // exact key -> transparent
        data.extend_from_slice(&[254, 1, 254, 255]); // within ±1 -> transparent
        data.extend_from_slice(&[250, 0, 255, 255]); // too far on R -> opaque

        let mut atlas = BitmapAtlas {
            width: 3,
            height: 1,
            data,
        };

        atlas.apply_transparency();

        assert_eq!(atlas.data[3], 0, "(255,0,255) should key out");
        assert_eq!(atlas.data[7], 0, "(254,1,254) should key out");
        assert_eq!(atlas.data[11], 255, "(250,0,255) should stay opaque");
    }

    #[test]
    fn test_is_magenta_key_boundaries() {
        assert!(is_magenta_key(255, 0, 255));
        assert!(is_magenta_key(254, 1, 254));
        assert!(!is_magenta_key(250, 0, 255));
        assert!(!is_magenta_key(255, 2, 255)); // G out of ±1
        assert!(!is_magenta_key(253, 0, 255)); // R out of ±1
    }

    #[test]
    fn test_extract_region() {
        let img = DynamicImage::new_rgba8(100, 100);
        let atlas = BitmapAtlas::from_image(img);

        let region = atlas.extract_region(10, 10, 20, 20);
        assert!(region.is_some());

        let region = region.unwrap();
        assert_eq!(region.width, 20);
        assert_eq!(region.height, 20);
        assert_eq!(region.x, 10);
        assert_eq!(region.y, 10);
    }

    #[test]
    fn test_extract_region_out_of_bounds() {
        let img = DynamicImage::new_rgba8(100, 100);
        let atlas = BitmapAtlas::from_image(img);

        let region = atlas.extract_region(90, 90, 20, 20);
        assert!(region.is_none());
    }
}
