//! Custom-cursor rendering for WSZ skins.
//!
//! We hand the skin's `.cur` bitmap straight to egui-winit (via our
//! local fork of egui that exposes `ctx.set_cursor_image(...)`), which
//! creates a real `winit::CustomCursor` and asks the OS to display it.
//! This avoids the clipping problem of in-window painted cursors — the
//! sprite is now drawn by the compositor and can extend past the
//! window edge like any native cursor.
//!
//! Each kind's RGBA buffer is wrapped in an `Arc<[u8]>` the first time
//! it is requested. egui-winit dedupes by `Arc::as_ptr`, so reusing the
//! same Arc across frames avoids re-uploading the bitmap to the OS.
//! The cache also keeps the Arcs alive for the lifetime of the skin —
//! `clear()` drops them when the skin changes.
//!
//! The hot-area picker (`pick_kind`) keeps the layout knowledge in one
//! place rather than threading per-cursor decisions through every window.
//! It's a coarse approximation: the bounding rects are pulled from
//! `WSZ_FORMAT.md` and don't account for skin-specific region masks.

use egui::{Context, CursorIcon, CustomCursorImage};
use oneamp_core::wsz::cursor::{CursorImage, CursorKind};
use oneamp_core::wsz::skin::WszSkin;
use std::collections::HashMap;
use std::sync::Arc;

/// Where the pointer currently is from the cursor picker's perspective.
/// The picker maps a screen-space pointer to one of these regions; the
/// renderer turns the region into a `CursorKind` (with the appropriate
/// fallback chain when the skin doesn't ship that exact slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotArea {
    /// Default — pointer is over content with no special cursor.
    Normal,
    /// Pointer is over the main window's title bar drag area.
    TitleBar,
    /// Pointer is over the menu (Options) button — clutterbar `O`.
    MainMenu,
    /// Minimize button on the main titlebar.
    Min,
    /// Close button on the main titlebar.
    Close,
    /// Windowshade button on the main titlebar.
    WinBut,
    /// Title scroller hot area.
    SongName,
    /// Position slider hot area.
    PosBar,
    /// Volume slider hot area.
    VolBar,
    /// Balance slider hot area.
    VolBal,
    /// Equalizer window: title bar.
    EqTitle,
    /// Equalizer window: any of the 11 vertical sliders.
    EqSlid,
    /// Equalizer window: close button.
    EqClose,
    /// Equalizer window: anywhere else.
    EqNormal,
    /// Playlist window: title bar.
    PTBar,
    /// Playlist window: close button.
    PClose,
    /// Playlist window: scrollbar thumb / groove.
    PVScroll,
    /// Playlist window: resize handle.
    PSize,
    /// Playlist window: anywhere else.
    PNormal,
}

impl HotArea {
    /// Pick the best `CursorKind` for this hot area, with falls back to
    /// the closest available cursor if the skin doesn't ship the exact
    /// slot. Order matters: a more specific cursor wins, but if it's
    /// missing we degrade gracefully.
    pub fn cursor_kind(self, skin: &WszSkin) -> Option<CursorKind> {
        let candidates: &[CursorKind] = match self {
            HotArea::Normal => &[CursorKind::Normal],
            HotArea::TitleBar => &[CursorKind::TitleBar, CursorKind::Normal],
            HotArea::MainMenu => &[CursorKind::MainMenu, CursorKind::Normal],
            HotArea::Min => &[CursorKind::Min, CursorKind::Normal],
            HotArea::Close => &[CursorKind::Close, CursorKind::Normal],
            HotArea::WinBut => &[CursorKind::WinBut, CursorKind::Normal],
            HotArea::SongName => &[CursorKind::SongName, CursorKind::Normal],
            HotArea::PosBar => &[CursorKind::PosBar, CursorKind::Normal],
            HotArea::VolBar => &[CursorKind::VolBar, CursorKind::Normal],
            HotArea::VolBal => &[CursorKind::VolBal, CursorKind::Normal],
            HotArea::EqTitle => &[
                CursorKind::EqTitle,
                CursorKind::EqNormal,
                CursorKind::Normal,
            ],
            HotArea::EqSlid => &[CursorKind::EqSlid, CursorKind::EqNormal, CursorKind::Normal],
            HotArea::EqClose => &[
                CursorKind::EqClose,
                CursorKind::EqNormal,
                CursorKind::Normal,
            ],
            HotArea::EqNormal => &[CursorKind::EqNormal, CursorKind::Normal],
            HotArea::PTBar => &[CursorKind::PTBar, CursorKind::PNormal, CursorKind::Normal],
            HotArea::PClose => &[CursorKind::PClose, CursorKind::PNormal, CursorKind::Normal],
            HotArea::PVScroll => &[
                CursorKind::PVScroll,
                CursorKind::PNormal,
                CursorKind::Normal,
            ],
            HotArea::PSize => &[CursorKind::PSize, CursorKind::PNormal, CursorKind::Normal],
            HotArea::PNormal => &[CursorKind::PNormal, CursorKind::Normal],
        };
        candidates
            .iter()
            .copied()
            .find(|k| skin.cursors.contains_key(k))
    }
}

/// Caches one `CustomCursorImage` per `CursorKind` so the same `Arc`
/// is handed to egui-winit on every frame — that lets the integration
/// skip re-registering the bitmap with the OS (it dedupes by Arc ptr).
pub struct CursorOverlay {
    images: HashMap<CursorKind, CustomCursorImage>,
}

impl CursorOverlay {
    pub fn new(_scale: f32) -> Self {
        Self {
            images: HashMap::new(),
        }
    }

    /// Drop every cached image. Call when the active skin changes —
    /// otherwise stale Arcs from the previous skin's cursors stay in
    /// the cache and would mismatch the new skin's bitmaps.
    pub fn clear(&mut self) {
        self.images.clear();
    }

    /// Push the bitmap matching `area` to egui as the OS cursor for the
    /// next frame. No-op when the skin ships no cursors at all or the
    /// pointer is off the surface — in those cases we leave egui's
    /// `cursor_icon` alone so the user keeps their normal arrow.
    ///
    /// Whenever the skin has at least one cursor and the pointer is on
    /// the surface, we always push *some* image (falling back to
    /// `Normal` or the first available kind), so the OS arrow never
    /// sneaks back in over the app.
    pub fn paint(&mut self, ctx: &Context, skin: &WszSkin, area: HotArea) {
        if skin.cursors.is_empty() {
            return;
        }
        // `latest_pos` returns `None` after `PointerGone` (cursor left
        // the surface), so we use it as the in-viewport gate.
        if ctx.input(|i| i.pointer.latest_pos()).is_none() {
            ctx.set_cursor_image(None);
            return;
        }

        let kind = area.cursor_kind(skin).or_else(|| {
            if skin.cursors.contains_key(&CursorKind::Normal) {
                Some(CursorKind::Normal)
            } else {
                skin.cursors.keys().copied().next()
            }
        });
        let Some(kind) = kind else {
            return;
        };
        let Some(image) = skin.cursors.get(&kind) else {
            return;
        };

        let entry = self
            .images
            .entry(kind)
            .or_insert_with(|| build_cursor_image(image));

        // Reset CursorIcon::None just in case some widget set a different
        // icon — the cursor_image path overrides the icon anyway in our
        // patched egui-winit, but staying explicit makes intent obvious
        // and protects against integrations that ignore cursor_image.
        ctx.set_cursor_icon(CursorIcon::None);
        ctx.set_cursor_image(Some(entry.clone()));
    }
}

fn build_cursor_image(image: &CursorImage) -> CustomCursorImage {
    CustomCursorImage {
        rgba: Arc::from(image.rgba.as_slice()),
        size: [image.width as u16, image.height as u16],
        hotspot: [image.hotspot_x as u16, image.hotspot_y as u16],
    }
}
