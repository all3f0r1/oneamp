//! Per-skin thumbnail cache for the welcome screen and Skins… dialog.
//!
//! For each `SkinEntry`, we lazy-load the underlying `.wsz` via
//! `WszLoader`, extract the 275×116 `main.bmp` bitmap atlas, and upload
//! it to the requesting `egui::Context` with NEAREST filtering. The
//! resulting `TextureHandle` is cached so each skin is only parsed once
//! per session.
//!
//! Textures in egui are bound to the context they were uploaded to, so
//! two viewports (welcome viewport vs. Skins… dialog viewport) need
//! independent caches — instances of this struct live on `Welcome` and
//! `SkinsDialog` respectively. Cheap: a cache miss is one ZIP unpack +
//! one texture upload, ~5–20 ms per skin.

use crate::skins::{SkinEntry, SkinPayload};
use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use oneamp_core::wsz::{loader::WszLoader, skin::SkinComponent};
use std::collections::HashMap;

#[derive(Default)]
pub struct SkinThumbnailCache {
    /// `None` for entries whose `.wsz` failed to parse — keeps us from
    /// retrying a broken file on every frame. Re-attempting only
    /// happens after `clear()` (rescan).
    textures: HashMap<String, Option<TextureHandle>>,
}

impl SkinThumbnailCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop every cached texture — used when the user rescans their
    /// skins folder so newly-added entries get re-parsed (and
    /// previously-cached entries get refreshed in case the file
    /// changed on disk).
    pub fn clear(&mut self) {
        self.textures.clear();
    }

    /// Get the thumbnail texture for `entry`, loading it on first
    /// request. Returns `None` when the underlying `.wsz` could not be
    /// parsed; the caller should fall back to a text-only label.
    pub fn get_or_load(&mut self, ctx: &Context, entry: &SkinEntry) -> Option<&TextureHandle> {
        let key = entry.name.clone();
        if !self.textures.contains_key(&key) {
            let tex = load_main_thumbnail(ctx, &key, &entry.payload);
            self.textures.insert(key.clone(), tex);
        }
        self.textures.get(&key).and_then(|t| t.as_ref())
    }
}

fn load_main_thumbnail(ctx: &Context, name: &str, payload: &SkinPayload) -> Option<TextureHandle> {
    let skin = match payload {
        SkinPayload::Embedded(bytes) => WszLoader::load_from_bytes(bytes).ok()?,
        SkinPayload::File(path) => WszLoader::load_from_file(path).ok()?,
    };
    let atlas = skin.get_bitmap(&SkinComponent::Main)?;
    let img = ColorImage::from_rgba_unmultiplied(
        [atlas.width as usize, atlas.height as usize],
        &atlas.data,
    );
    Some(ctx.load_texture(format!("skin_thumb_{name}"), img, TextureOptions::NEAREST))
}
