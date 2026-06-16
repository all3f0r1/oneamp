//! Skin-derived theme used by sub-viewport dialogs (URL / format / tag
//! editor / preset name / welcome) and the clutterbar Options popup.
//!
//! Two responsibilities:
//!   - [`DialogTheme`] derives a palette from the active skin's `pledit.txt`
//!     colours and pushes it into the egui [`Style`] so widgets, panels and
//!     selection highlights all render in the Winamp green-on-black look
//!     (or whatever the skin overrode it with).
//!   - [`paint_titlebar`] paints a Winamp-style title bar across the top of
//!     a chrome-less sub-viewport — drag-to-move covers the whole strip
//!     except for the small close button on the right.
//!
//! Why pledit and not `titlebar.bmp` / `gen.bmp`: the classic atlases are
//! fixed-width sprites (275 px) sized for the main player and the Winamp
//! 2.9+ Media Library window. None of them stretch cleanly to the ~440–580
//! px width of our dialogs, and not every skin ships `gen.bmp` either.
//! Solid fills sampled from the pledit palette are the lowest-risk way to
//! get a coherent look across every shipped skin, and they automatically
//! pick up any custom palette the skin author defined.

use egui::{
    Color32, ColorImage, Context, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui,
    Vec2, ViewportCommand,
};
use oneamp_core::wsz::skin::{SkinComponent, WszSkin};

/// On-screen scale of the BMP title-bar strip when [`SkinnedFrame`] is
/// available. The atlas strip is 14 px tall (Winamp's native main-player
/// chrome); doubling it gives a 28 px bar that reads at native ppp.
const STRIP_SCALE: f32 = 2.0;
/// Height of the painted title bar in egui units. When the active skin
/// ships `titlebar.bmp`, the BMP strip is drawn at this exact height
/// (`14 × STRIP_SCALE`); the procedural fallback uses it too.
pub const TITLEBAR_H: f32 = 28.0;
/// Side padding for the title text inside the drag handle.
const TITLEBAR_TEXT_PAD: f32 = 8.0;
/// Edge length of the X close button (procedural fallback). The BMP
/// variant uses `9 × STRIP_SCALE = 18` instead so it matches the strip
/// scale exactly.
const CLOSE_BTN_SIZE: f32 = 18.0;
/// Right-edge gap between the close button and the window border.
const CLOSE_BTN_PAD: f32 = 6.0;
/// BMP atlas coordinates of the close-button sprite inside `titlebar.bmp`.
/// Layout from `wsz_ui/components/titlebar.rs::TitlebarButton::sprite_coords`:
/// unpressed at (18, 0), pressed at (18, 9). Both are 9 × 9.
const CLOSE_SPRITE_X: u32 = 18;
const CLOSE_SPRITE_UNPRESSED_Y: u32 = 0;
const CLOSE_SPRITE_PRESSED_Y: u32 = 9;
const CLOSE_SPRITE_SIZE: u32 = 9;
/// BMP atlas coordinates of the *strip* (the repeating decorative band)
/// inside `titlebar.bmp`. Active variant at y=0, inactive at y=15. The
/// player's main window extracts (27, 0, 275, 14) — see
/// `wsz_ui/components/titlebar.rs::TitlebarButtons::render`.
const STRIP_SRC_X: u32 = 27;
const STRIP_SRC_W: u32 = 275;
const STRIP_SRC_H: u32 = 14;

/// Skin-derived palette pushed into the dialog viewport's egui style.
#[derive(Clone, Copy, Debug)]
pub struct DialogTheme {
    /// Panel / window background fill.
    pub bg: Color32,
    /// Default body text colour.
    pub text: Color32,
    /// Highlight colour (currently-playing track in the playlist) — re-used
    /// here for the title text and hover strokes so links stand out.
    pub current: Color32,
    /// Selection fill (text selection, active widget).
    pub selection_bg: Color32,
    /// Title-bar background — a slightly lifted shade of `bg` so the strip
    /// reads as separate from the body.
    pub titlebar_bg: Color32,
    /// Idle button fill.
    pub button_fill: Color32,
    /// Hovered button fill — lighter than `button_fill` for a clear hover
    /// affordance against the typically-dark `bg`.
    pub button_hover: Color32,
    /// Border / divider stroke colour.
    pub border: Color32,
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Nudge each channel by `delta` with clamping. Used to derive the
/// title-bar / button / border shades from the pledit background.
fn shift(c: Color32, delta: i32) -> Color32 {
    let mix = |ch: u8| (ch as i32 + delta).clamp(0, 255) as u8;
    Color32::from_rgb(mix(c.r()), mix(c.g()), mix(c.b()))
}

impl DialogTheme {
    pub fn from_skin(skin: &WszSkin) -> Self {
        let p = skin.pledit.colors;
        let bg = rgb(p.normal_bg);
        Self {
            bg,
            text: rgb(p.normal),
            current: rgb(p.current),
            selection_bg: rgb(p.selected_bg),
            titlebar_bg: shift(bg, 14),
            button_fill: shift(bg, 18),
            button_hover: shift(bg, 40),
            border: shift(bg, 34),
        }
    }

    /// Push the palette into `ctx`'s egui [`Style`]. Idempotent — safe to
    /// call every frame; the underlying `Arc<Style>` reuse means a no-op
    /// when nothing changed.
    pub fn apply_to_ctx(&self, ctx: &Context) {
        let mut style = (*ctx.style()).clone();
        let v = &mut style.visuals;

        v.window_fill = self.bg;
        v.panel_fill = self.bg;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.override_text_color = Some(self.text);
        v.hyperlink_color = self.current;
        v.extreme_bg_color = shift(self.bg, -10);

        v.selection.bg_fill = self.selection_bg;
        v.selection.stroke = Stroke::new(1.0, self.current);

        // Non-interactive: labels, frames.
        v.widgets.noninteractive.bg_fill = self.bg;
        v.widgets.noninteractive.weak_bg_fill = self.bg;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text);

        // Inactive (idle) buttons / text edits.
        v.widgets.inactive.bg_fill = self.button_fill;
        v.widgets.inactive.weak_bg_fill = self.button_fill;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, self.border);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, self.text);

        // Hovered.
        v.widgets.hovered.bg_fill = self.button_hover;
        v.widgets.hovered.weak_bg_fill = self.button_hover;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, self.current);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, self.current);

        // Active (pressed / focused).
        v.widgets.active.bg_fill = self.selection_bg;
        v.widgets.active.weak_bg_fill = self.selection_bg;
        v.widgets.active.bg_stroke = Stroke::new(1.0, self.current);
        v.widgets.active.fg_stroke = Stroke::new(1.0, self.current);

        // Open (combo box dropdowns).
        v.widgets.open.bg_fill = self.selection_bg;
        v.widgets.open.weak_bg_fill = self.selection_bg;
        v.widgets.open.bg_stroke = Stroke::new(1.0, self.current);
        v.widgets.open.fg_stroke = Stroke::new(1.0, self.current);

        ctx.set_style(style);
    }
}

/// Outcome of one frame of [`paint_titlebar`].
#[derive(Default, Clone, Copy, Debug)]
pub struct TitlebarResult {
    /// User clicked the X close button this frame.
    pub close_clicked: bool,
}

/// BMP-backed dialog chrome assembled from `titlebar.bmp` pieces — the
/// same atlas the main player uses for its top strip. `None` variants
/// indicate the active skin doesn't ship that piece (e.g. a corrupt or
/// stripped-down `.wsz`); [`paint_titlebar`] then falls back to the
/// procedural look so we never end up with a blank title bar.
///
/// Textures are cached per [`Context`] via [`Context::data_mut`], keyed
/// on the skin metadata name + sprite key. A skin switch invalidates the
/// cache naturally because the name changes; stale entries from the old
/// skin sit in egui's temp data store and are GC'd when egui drops them.
pub struct SkinnedFrame {
    /// Repeating decorative strip from `titlebar.bmp` (active variant).
    /// Texture is loaded with [`TextureOptions::NEAREST_REPEAT`] so the
    /// GPU tiles it for us via the UV rect.
    strip: Option<TextureHandle>,
    /// Close-button sprite — unpressed (idle / hover).
    close_unpressed: Option<TextureHandle>,
    /// Close-button sprite — pressed (mouse down).
    close_pressed: Option<TextureHandle>,
}

impl SkinnedFrame {
    /// Try to load the BMP pieces from `skin`. Returns a `SkinnedFrame`
    /// whose individual fields are `Some` for every piece that decoded
    /// cleanly — the caller can mix-and-match (e.g. paint a BMP strip
    /// but fall back to a procedural close X if just that sprite was
    /// missing).
    pub fn load(ctx: &Context, skin: &WszSkin) -> Self {
        let atlas = skin.get_bitmap(&SkinComponent::TitleBar);
        let key_prefix = skin.metadata.name.as_str();

        let strip = atlas
            .and_then(|a| a.extract_region(STRIP_SRC_X, 0, STRIP_SRC_W, STRIP_SRC_H))
            .map(|r| {
                cache_texture(
                    ctx,
                    &format!("{key_prefix}::titlebar_strip_active"),
                    &r,
                    TextureOptions::NEAREST_REPEAT,
                )
            });

        let close_unpressed = atlas
            .and_then(|a| {
                a.extract_region(
                    CLOSE_SPRITE_X,
                    CLOSE_SPRITE_UNPRESSED_Y,
                    CLOSE_SPRITE_SIZE,
                    CLOSE_SPRITE_SIZE,
                )
            })
            .map(|r| {
                cache_texture(
                    ctx,
                    &format!("{key_prefix}::titlebar_close_unpressed"),
                    &r,
                    TextureOptions::NEAREST,
                )
            });
        let close_pressed = atlas
            .and_then(|a| {
                a.extract_region(
                    CLOSE_SPRITE_X,
                    CLOSE_SPRITE_PRESSED_Y,
                    CLOSE_SPRITE_SIZE,
                    CLOSE_SPRITE_SIZE,
                )
            })
            .map(|r| {
                cache_texture(
                    ctx,
                    &format!("{key_prefix}::titlebar_close_pressed"),
                    &r,
                    TextureOptions::NEAREST,
                )
            });

        Self {
            strip,
            close_unpressed,
            close_pressed,
        }
    }
}

/// Get-or-insert a [`TextureHandle`] in egui's temp data store. Means
/// the GPU upload happens once per `(skin × sprite)` instead of every
/// frame. Stale handles for retired skins sit in temp data until egui
/// flushes them — fine for the handful of skins a session sees.
fn cache_texture(
    ctx: &Context,
    name: &str,
    region: &oneamp_core::wsz::bitmap::BitmapRegion,
    options: TextureOptions,
) -> TextureHandle {
    let id = egui::Id::new(("wsz_dialog_tex", name));
    if let Some(h) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return h;
    }
    let img = ColorImage::from_rgba_unmultiplied(
        [region.width as usize, region.height as usize],
        &region.data,
    );
    let handle = ctx.load_texture(name, img, options);
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    handle
}

/// Paint a Winamp-style title bar across the top of `ui`. Routes through
/// the BMP-backed [`SkinnedFrame`] when the active skin ships
/// `titlebar.bmp`, and falls back to the procedural look otherwise.
/// Drag-to-move covers the whole strip outside the close button and
/// emits a single [`ViewportCommand::StartDrag`] on `drag_started` — the
/// same pattern as the main player's custom chrome (see
/// `wsz_ui/main_window/input.rs::handle_window_drag`).
pub fn paint_titlebar(
    ui: &mut Ui,
    theme: &DialogTheme,
    title: &str,
    skin: &WszSkin,
) -> TitlebarResult {
    let frame = SkinnedFrame::load(ui.ctx(), skin);
    if frame.strip.is_some() {
        paint_titlebar_skinned(ui, theme, title, &frame)
    } else {
        paint_titlebar_procedural(ui, theme, title)
    }
}

/// BMP-backed title bar: tile the `titlebar.bmp` strip horizontally via
/// `TextureOptions::NEAREST_REPEAT` (the GPU does the tiling for us), paint
/// the close sprite at the right edge with idle/pressed variants, and overlay
/// the title text using the dialog font.
fn paint_titlebar_skinned(
    ui: &mut Ui,
    theme: &DialogTheme,
    title: &str,
    frame: &SkinnedFrame,
) -> TitlebarResult {
    let mut result = TitlebarResult::default();
    let avail_w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, TITLEBAR_H), Sense::hover());

    // Tile the strip horizontally. UV width = how many strip-widths fit in
    // the dialog; the wrap=Repeat sampler does the loop. Height fixed at 1
    // because the strip exactly covers TITLEBAR_H thanks to `STRIP_SCALE`.
    if let Some(strip) = frame.strip.as_ref() {
        let tile_w_on_screen = STRIP_SRC_W as f32 * STRIP_SCALE;
        let uv = Rect::from_min_max(
            Pos2::new(0.0, 0.0),
            Pos2::new(rect.width() / tile_w_on_screen, 1.0),
        );
        ui.painter().image(strip.id(), rect, uv, Color32::WHITE);
    }

    // Close button — interact first so the click is captured before the
    // drag handle sees it.
    let close_size = CLOSE_SPRITE_SIZE as f32 * STRIP_SCALE;
    let close_rect = Rect::from_min_size(
        Pos2::new(
            rect.right() - CLOSE_BTN_PAD - close_size,
            rect.center().y - close_size / 2.0,
        ),
        Vec2::splat(close_size),
    );
    let close_resp = ui.interact(close_rect, ui.id().with("dialog_close_btn"), Sense::click());
    let pressed = close_resp.is_pointer_button_down_on();
    let sprite = if pressed {
        frame
            .close_pressed
            .as_ref()
            .or(frame.close_unpressed.as_ref())
    } else {
        frame.close_unpressed.as_ref()
    };
    if let Some(tex) = sprite {
        let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
        ui.painter().image(tex.id(), close_rect, uv, Color32::WHITE);
    }
    if close_resp.clicked() {
        result.close_clicked = true;
    }

    // Drag handle covers everything left of the close button. `StartDrag`
    // MUST fire exactly once per gesture — re-sending it each dragged
    // frame floods the WM with move requests (same trap documented in
    // `main_window/input.rs::handle_window_drag`).
    let drag_rect = Rect::from_min_max(rect.min, Pos2::new(close_rect.left() - 2.0, rect.bottom()));
    let drag_resp = ui.interact(
        drag_rect,
        ui.id().with("dialog_drag_area"),
        Sense::click_and_drag(),
    );
    if drag_resp.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    // Title text — left-aligned in the drag handle. Use the skin's pledit
    // `current` colour so the text reads over whatever strip background
    // the skin author drew.
    let font = egui::FontId::proportional(12.0);
    ui.painter().text(
        Pos2::new(drag_rect.left() + TITLEBAR_TEXT_PAD, drag_rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        font,
        theme.current,
    );

    result
}

/// Solid-fill title bar used when the active skin doesn't ship
/// `titlebar.bmp` (corrupt `.wsz`, custom skin with only `pledit.bmp`,
/// etc.). Same drag + close behaviour as [`paint_titlebar_skinned`].
fn paint_titlebar_procedural(ui: &mut Ui, theme: &DialogTheme, title: &str) -> TitlebarResult {
    let mut result = TitlebarResult::default();
    let avail_w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, TITLEBAR_H), Sense::hover());

    ui.painter().rect_filled(rect, 0.0, theme.titlebar_bg);
    ui.painter().line_segment(
        [
            Pos2::new(rect.left(), rect.bottom() - 1.0),
            Pos2::new(rect.right(), rect.bottom() - 1.0),
        ],
        Stroke::new(1.0, theme.border),
    );

    let close_rect = Rect::from_min_size(
        Pos2::new(
            rect.right() - CLOSE_BTN_PAD - CLOSE_BTN_SIZE,
            rect.center().y - CLOSE_BTN_SIZE / 2.0,
        ),
        Vec2::splat(CLOSE_BTN_SIZE),
    );
    let close_resp = ui.interact(close_rect, ui.id().with("dialog_close_btn"), Sense::click());
    let close_fill = if close_resp.hovered() {
        theme.button_hover
    } else {
        theme.button_fill
    };
    ui.painter().rect_filled(close_rect, 2.0, close_fill);
    ui.painter()
        .rect_stroke(close_rect, 2.0, Stroke::new(1.0, theme.border));
    let pad = 3.0;
    let stroke = Stroke::new(1.5, theme.text);
    ui.painter().line_segment(
        [
            close_rect.min + Vec2::splat(pad),
            close_rect.max - Vec2::splat(pad),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            Pos2::new(close_rect.max.x - pad, close_rect.min.y + pad),
            Pos2::new(close_rect.min.x + pad, close_rect.max.y - pad),
        ],
        stroke,
    );
    if close_resp.clicked() {
        result.close_clicked = true;
    }

    let drag_rect = Rect::from_min_max(rect.min, Pos2::new(close_rect.left() - 2.0, rect.bottom()));
    let drag_resp = ui.interact(
        drag_rect,
        ui.id().with("dialog_drag_area"),
        Sense::click_and_drag(),
    );
    if drag_resp.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    let font = egui::FontId::proportional(12.0);
    ui.painter().text(
        Pos2::new(drag_rect.left() + TITLEBAR_TEXT_PAD, drag_rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        font,
        theme.current,
    );

    result
}
