use super::texture_cache::TextureCache;
use egui::{Image, Pos2, Rect, Sense, Ui, Vec2};
use oneamp_core::wsz::{bitmap::BitmapRegion, skin::SkinComponent, skin::WszSkin};
use std::sync::Arc;

pub struct WszRenderer {
    texture_cache: TextureCache,
    skin: Arc<WszSkin>,
    scale: f32,
}

impl WszRenderer {
    pub fn new(skin: WszSkin, scale: f32) -> Self {
        Self {
            texture_cache: TextureCache::new(),
            skin: Arc::new(skin),
            scale,
        }
    }

    pub fn get_scale(&self) -> f32 {
        self.scale
    }

    pub fn render_component(
        &mut self,
        ui: &mut Ui,
        component: &SkinComponent,
        pos: Pos2,
    ) -> Option<Rect> {
        let atlas = self.skin.get_bitmap(component)?;
        let key = format!("{:?}", component);
        let texture = self.texture_cache.get_or_create(ui.ctx(), &key, atlas);

        let size = Vec2::new(
            atlas.width as f32 * self.scale,
            atlas.height as f32 * self.scale,
        );

        let rect = Rect::from_min_size(pos, size);

        let image = Image::from_texture(texture)
            .fit_to_exact_size(size)
            .sense(Sense::hover());

        ui.put(rect, image);

        Some(rect)
    }

    pub fn render_region(
        &mut self,
        ui: &mut Ui,
        region: &BitmapRegion,
        pos: Pos2,
        key_suffix: &str,
    ) -> Rect {
        let key = format!("region_{}_{}", key_suffix, region.x);
        let texture = self.texture_cache.get_or_create_region(
            ui.ctx(),
            &key,
            &region.data,
            region.width,
            region.height,
        );

        let size = Vec2::new(
            region.width as f32 * self.scale,
            region.height as f32 * self.scale,
        );

        let rect = Rect::from_min_size(pos, size);

        let image = Image::from_texture(texture)
            .fit_to_exact_size(size)
            .sense(Sense::hover());

        ui.put(rect, image);

        rect
    }

    pub fn skin_to_screen(&self, skin_x: u32, skin_y: u32, offset: Pos2) -> Pos2 {
        Pos2::new(
            offset.x + skin_x as f32 * self.scale,
            offset.y + skin_y as f32 * self.scale,
        )
    }

    pub fn get_skin(&self) -> &WszSkin {
        &self.skin
    }
}
