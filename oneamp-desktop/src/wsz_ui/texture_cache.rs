use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use oneamp_core::wsz::bitmap::BitmapAtlas;
use std::collections::HashMap;

pub struct TextureCache {
    textures: HashMap<String, TextureHandle>,
}

impl TextureCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
        }
    }

    pub fn get_or_create(
        &mut self,
        ctx: &Context,
        key: &str,
        atlas: &BitmapAtlas,
    ) -> &TextureHandle {
        self.textures.entry(key.to_string()).or_insert_with(|| {
            let color_image = Self::bitmap_to_color_image(atlas);
            // NEAREST so the upscaled skin stays crisp like the original
            // Winamp; LINEAR would blur pixel art on hi-DPI displays.
            ctx.load_texture(key, color_image, TextureOptions::NEAREST)
        })
    }

    pub fn get_or_create_region(
        &mut self,
        ctx: &Context,
        key: &str,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> &TextureHandle {
        self.textures.entry(key.to_string()).or_insert_with(|| {
            let color_image =
                ColorImage::from_rgba_unmultiplied([width as usize, height as usize], data);
            // NEAREST so the upscaled skin stays crisp like the original
            // Winamp; LINEAR would blur pixel art on hi-DPI displays.
            ctx.load_texture(key, color_image, TextureOptions::NEAREST)
        })
    }

    fn bitmap_to_color_image(atlas: &BitmapAtlas) -> ColorImage {
        ColorImage::from_rgba_unmultiplied(
            [atlas.width as usize, atlas.height as usize],
            &atlas.data,
        )
    }
}

impl Default for TextureCache {
    fn default() -> Self {
        Self::new()
    }
}
