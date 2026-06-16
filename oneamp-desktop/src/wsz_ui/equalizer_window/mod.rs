mod input;
mod paint;

use super::renderer::WszRenderer;
use egui::{Context, Pos2, Rect, Vec2};
use oneamp_core::wsz::skin::{SkinComponent, WszSkin};
use oneamp_core::{AudioEngine, AudioEvent};

/// Visual + audio range for the equalizer sliders, in decibels. Matches
/// Winamp's classic ±20 dB slider span (28 fill frames, 1.43 dB step).
pub const EQ_GAIN_MIN_DB: f32 = -20.0;
pub const EQ_GAIN_MAX_DB: f32 = 20.0;

/// Vertical pixel travel for the slider thumb within a 14×63 track. The thumb
/// is 11 px tall, so the thumb's top can sit anywhere in 0..(63-11)=52 px.
pub(super) const TRACK_HEIGHT: u32 = 63;
const THUMB_HEIGHT: u32 = 11;
pub(super) const SLIDER_TRAVEL: u32 = TRACK_HEIGHT - THUMB_HEIGHT;
pub(super) const SLIDER_TOP_Y: u32 = 38;

/// X coordinates of preamp + 10 band tracks (skin space). Track width 14 px.
pub(super) const TRACK_X: [u32; 11] = [21, 78, 96, 114, 132, 150, 168, 186, 204, 222, 240];

/// Skin-space x of the shade-mode volume rail (97 px wide). The rail is
/// baked into the `eq_ex.bmp` y=0..14 strip; we overlay a 3×7 thumb.
const SHADE_VOL_RAIL_X: u32 = 61;
const SHADE_VOL_RAIL_W: u32 = 97;
/// Same as `SHADE_VOL_RAIL_X` but for the balance rail (42 px wide).
const SHADE_BAL_RAIL_X: u32 = 164;
const SHADE_BAL_RAIL_W: u32 = 42;
/// Y coordinate of every shade mini-thumb (volume + balance share the row).
const SHADE_THUMB_Y: u32 = 4;
/// Half-width of the balance dead zone — values inside `±SHADE_BAL_DEADZONE`
/// pick the centered "M" sprite. Matches the spec's tolerance band so a
/// near-zero balance doesn't flicker between L and R sprites.
const SHADE_BAL_DEADZONE: f32 = 0.05;

/// Skin-space rectangle of the EQ on/off button (sprite + hot area).
pub(super) const EQ_BUTTON: SkinRect = SkinRect {
    x: 14,
    y: 18,
    w: 25,
    h: 12,
};
/// Skin-space rectangle of the AUTO button.
pub(super) const AUTO_BUTTON: SkinRect = SkinRect {
    x: 39,
    y: 18,
    w: 33,
    h: 12,
};
/// Skin-space rectangle of the PRESETS button.
pub(super) const PRESETS_BUTTON: SkinRect = SkinRect {
    x: 217,
    y: 18,
    w: 44,
    h: 12,
};

/// Action emitted by the equalizer window for the application/coordinator
/// to handle (closing, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqualizerAction {
    /// User clicked the close button on the EQ titlebar.
    Close,
    /// User picked "Save as preset…" from the PRESETS dropdown.
    /// The coordinator routes this up to the app, which opens the
    /// `PresetNameDialog` modal and (on accept) writes the current
    /// band curve + preamp into `PresetManager`.
    SaveAsUserPreset,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SkinRect {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) w: u32,
    pub(super) h: u32,
}

impl SkinRect {
    pub(super) fn screen_rect(&self, renderer: &WszRenderer, offset: Pos2) -> Rect {
        let scale = renderer.get_scale();
        let pos = renderer.skin_to_screen(self.x, self.y, offset);
        Rect::from_min_size(pos, Vec2::new(self.w as f32 * scale, self.h as f32 * scale))
    }
}

pub struct EqualizerWindow {
    pub(super) renderer: WszRenderer,
    /// Per-band gain in dB, indexed 0..10 (low → high frequency)
    pub(super) eq_bands: [f32; 10],
    /// Pre-amp gain in dB. UI-only for now (no master gain in the audio
    /// engine); reflected in the spline's preamp baseline.
    pub(super) preamp: f32,
    pub(super) enabled: bool,
    pub(super) auto: bool,
    /// Index of the band currently being dragged (10 = preamp).
    pub(super) dragging: Option<usize>,
    /// Visual press state for the EQ/AUTO/PRESETS buttons.
    pub(super) pressed_button: Option<EqButton>,
    /// True when the presets dropdown is open.
    pub(super) presets_menu_open: bool,
    pub(super) mouse_was_pressed: bool,
    /// When true, the window collapses to its 14-px title strip
    /// (`eq_ex.bmp` y=0..14) — sliders/spline/buttons hidden. Toggled by
    /// double-clicking the title bar, mirroring the main window.
    pub(super) shade_mode: bool,
    /// Mirror of the audio engine's volume (0..1). The shade strip exposes
    /// a 3-state mini volume slider; we track the value here so we can
    /// pick the correct sprite + drag-update from the strip's hot area.
    /// Synced via `AudioEvent::VolumeUpdated`.
    pub(super) volume_value: f32,
    /// Mirror of the engine's balance (-1..1) — same role as
    /// `volume_value` but for the balance mini-slider.
    pub(super) balance_value: f32,
    /// Which shade-mode mini-control is currently being dragged. None when
    /// the user isn't interacting with the shade rails.
    pub(super) shade_drag: Option<ShadeDrag>,
    /// Soft-focus flag pushed in by the coordinator. Drives the active vs
    /// inactive title strip. Base-2.91 ships a blank "active" strip at
    /// y=0..14 and the title text at y=149..163, so an unfocused EQ
    /// actually shows the recognisable "WINAMP EQUALIZER" text while a
    /// focused one (where the user is interacting with the EQ) goes dark
    /// — same visual as Winamp.
    pub(super) focused: bool,
    /// Name of the last preset the user picked from the dropdown.
    /// `None` when the EQ has been edited manually since the last
    /// preset apply (or has never had one applied). Exposed via
    /// [`current_preset`] so the app can mirror it into persisted
    /// config and surface a "selected" indicator on the dropdown.
    pub(super) current_preset_name: Option<String>,
    /// `Some((band, until))` while a band-flash animation is in
    /// progress — triggered by Shift+clicking the main-window
    /// spectrum visualiser. The paint code draws a temporary glow
    /// around `band` until `until`, then the field self-clears.
    /// `band` indexes into the 10 EQ bands (0 = 31.5 Hz, 9 = 16 kHz).
    pub(super) flash_band_state: Option<(usize, std::time::Instant)>,
    /// User-defined presets loaded from `<config_dir>/oneamp/eq_presets.json`
    /// at boot and refreshed in place whenever the user adds one via
    /// "Save as preset…". Rendered in the PRESETS dropdown after the
    /// built-ins so muscle memory for stock presets stays intact.
    pub(super) user_presets: Vec<oneamp_core::equalizer_presets::EqualizerPreset>,
    /// Set to true by the dropdown's "Save as preset…" row click.
    /// The next `show()` consumes it and returns
    /// `EqualizerAction::SaveAsUserPreset` so the app can open the
    /// modal name-prompt dialog. Cleared on read.
    pub(super) pending_save_as_user_preset: bool,
}

/// Active drag inside the EQ shade strip. Volume + balance share the
/// dispatch path but the rail geometry differs, so we tag the drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShadeDrag {
    Volume,
    Balance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EqButton {
    Eq,
    Auto,
    Presets,
}

/// One row inside the PRESETS dropdown. Borrows the preset out of the
/// vector that lives on the stack while the menu is open — avoids
/// cloning each name to drive the UI.
pub(super) enum PresetRow<'a> {
    /// "Load .eqf…" — opens an `rfd` file picker and applies the parsed
    /// preset on success.
    LoadEqf,
    /// "Save as .eqf…" — opens an `rfd` save dialog and writes the
    /// current bands + preamp.
    SaveEqf,
    /// "Save as preset…" — opens the in-app name dialog, then writes
    /// the current bands + preamp into the user preset store.
    SaveAsUserPreset,
    /// Built-in preset row (Rock, Pop, Jazz, …).
    Builtin(&'a oneamp_core::equalizer_presets::EqualizerPreset),
    /// User-defined preset (saved earlier via "Save as preset…").
    User(&'a oneamp_core::equalizer_presets::EqualizerPreset),
}

impl EqualizerWindow {
    pub fn new(skin: WszSkin, scale: f32) -> Self {
        Self::with_gains(skin, scale, &[0.0; 10], 0.0, false)
    }

    pub fn with_gains(
        skin: WszSkin,
        scale: f32,
        gains: &[f32],
        preamp_db: f32,
        enabled: bool,
    ) -> Self {
        let mut eq_bands = [0.0; 10];
        for (i, slot) in eq_bands.iter_mut().enumerate() {
            *slot = gains
                .get(i)
                .copied()
                .unwrap_or(0.0)
                .clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
        }

        Self {
            renderer: WszRenderer::new(skin, scale),
            eq_bands,
            preamp: preamp_db.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB),
            enabled,
            auto: false,
            dragging: None,
            pressed_button: None,
            presets_menu_open: false,
            mouse_was_pressed: false,
            shade_mode: false,
            // Default to a mid-volume / centered balance until the engine
            // emits its first `Updated` events. Picking 0.5 / 0.0 also
            // stops the shade-mode thumbs from snapping to the rail edges
            // on cold open.
            volume_value: 0.5,
            balance_value: 0.0,
            shade_drag: None,
            focused: false,
            current_preset_name: None,
            flash_band_state: None,
            user_presets: Vec::new(),
            pending_save_as_user_preset: false,
        }
    }

    /// Push the latest list of user-defined presets into the window so
    /// the PRESETS dropdown can render them after the built-ins. Called
    /// by the app at boot (when the JSON store loads) and after every
    /// successful "Save as preset…" submission.
    pub fn set_user_presets(
        &mut self,
        presets: Vec<oneamp_core::equalizer_presets::EqualizerPreset>,
    ) {
        self.user_presets = presets;
    }

    /// Trigger a temporary highlight on `band` for ~1.5 s. Used by the
    /// app when the user Shift+clicks a frequency on the main-window
    /// spectrum visualiser — surfacing which band the engine actually
    /// considers nearest. Re-calling resets the timer.
    pub fn flash_band(&mut self, band: usize) {
        if band >= 10 {
            return;
        }
        self.flash_band_state = Some((
            band,
            std::time::Instant::now() + std::time::Duration::from_millis(1500),
        ));
    }

    /// Snapshot of the last preset name the user applied via the
    /// dropdown. `None` when the bands have been edited manually
    /// since (or the EQ was never preset-driven this session).
    /// Polled by the app each frame so the persisted
    /// `EqualizerConfig::current_preset` stays in sync.
    pub fn current_preset(&self) -> Option<&str> {
        self.current_preset_name.as_deref()
    }

    /// Push the persisted preset name back into the window on boot
    /// so the dropdown remembers which entry was last applied.
    pub fn set_current_preset(&mut self, name: Option<String>) {
        self.current_preset_name = name;
    }

    /// Soft-focus setter driven by the coordinator. `true` means the EQ is
    /// the currently active sub-window — its titlebar will render the
    /// active variant. `false` defers to the inactive title strip variant
    /// (the one carrying "WINAMP EQUALIZER" text in stock skins).
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Apply audio events to the window so it reflects the engine's state.
    pub fn update(&mut self, events: &[AudioEvent]) {
        for event in events {
            match event {
                AudioEvent::EqualizerPreampUpdated(db) => {
                    self.preamp = db.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
                }
                AudioEvent::EqualizerUpdated(enabled, gains) => {
                    self.enabled = *enabled;
                    for (i, slot) in self.eq_bands.iter_mut().enumerate() {
                        if let Some(g) = gains.get(i).copied() {
                            *slot = g.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
                        }
                    }
                }
                // Skip volume/balance echoes while the user is dragging
                // the shade strip's rail of the same axis — otherwise
                // the in-flight drag value fights with the engine's
                // last-frame value and the thumb snaps backwards.
                AudioEvent::VolumeUpdated(vol, _)
                    if !matches!(self.shade_drag, Some(ShadeDrag::Volume)) =>
                {
                    self.volume_value = vol.clamp(0.0, 1.0);
                }
                AudioEvent::BalanceUpdated(bal)
                    if !matches!(self.shade_drag, Some(ShadeDrag::Balance)) =>
                {
                    self.balance_value = bal.clamp(-1.0, 1.0);
                }
                _ => {}
            }
        }
    }

    /// Skin-space height the EQ window currently occupies. 116 in normal
    /// mode, 14 in shade mode. The coordinator uses this to size the OS
    /// viewport and to dock subsequent sub-windows.
    pub fn height_skin(&self) -> u32 {
        if self.shade_mode { 14 } else { 116 }
    }

    /// Skin-space pixels of vertical overflow caused by the open
    /// preset dropdown. Returns `0` when the dropdown is closed.
    /// Used by the coordinator to grow the OS viewport so the menu
    /// isn't cropped by the bottom edge when no playlist is docked
    /// below the EQ. The dock position of the next sub-window does
    /// *not* shift — the menu floats on a top-order Area instead.
    pub fn preset_menu_overlay_extra_skin(&self) -> u32 {
        if !self.presets_menu_open || self.shade_mode {
            return 0;
        }
        // PRESETS button bottom edge in skin units (relative to the
        // EQ window's top). Matches the layout constant the menu
        // renderer computes from `PRESETS_BUTTON`.
        let menu_top_in_eq = PRESETS_BUTTON.y + PRESETS_BUTTON.h;
        // 14 px per row × (3 I/O rows + 16 built-ins + N user presets).
        // Counted dynamically so adding / saving a user preset grows
        // the viewport overflow on the next frame without manual sync.
        let row_count: u32 = 3 + 16 + self.user_presets.len() as u32;
        let menu_h = 14 * row_count;
        let menu_bottom_in_eq = menu_top_in_eq + menu_h;
        // Only the overflow *past* the EQ window's own floor counts;
        // anything that fits inside the existing 116 px doesn't
        // demand a taller viewport.
        menu_bottom_in_eq.saturating_sub(self.height_skin())
    }

    /// Render the equalizer as a docked area inside the main viewport at
    /// `dock_y_skin` (skin-space y; typically 116 to sit right below the
    /// main window). Drawn in the parent ctx so the OS sees a single window
    /// and the user gets one taskbar entry.
    pub fn show(
        &mut self,
        ctx: &Context,
        audio_engine: Option<&AudioEngine>,
        dock_y_skin: u32,
    ) -> Option<EqualizerAction> {
        if self.shade_mode {
            return self.show_shade(ctx, dock_y_skin, audio_engine);
        }

        let scale = self.renderer.get_scale();
        let window_size = Vec2::new(275.0 * scale, 116.0 * scale);

        let inner = egui::Area::new(egui::Id::new("wsz_equalizer_window"))
            .fixed_pos(Pos2::new(0.0, dock_y_skin as f32 * scale))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_min_size(window_size);
                ui.set_max_size(window_size);

                let area_rect = ui.max_rect();
                let offset = area_rect.min;

                // Title-bar area (top 14 px). Shade mode is disabled for
                // sub-windows since 1.0.5 — only the main window can be
                // shaded ("Mini mode" in the titlebar menu). The
                // double-click handler that used to set
                // `self.shade_mode = true` is gone; we still register the
                // interact rect so the drag-start logic (handled
                // elsewhere) has a clear hit area but it no longer
                // collapses the EQ.
                let title_rect =
                    Rect::from_min_size(offset, Vec2::new(275.0 * scale, 14.0 * scale));
                let _title_response = ui.interact(
                    title_rect,
                    egui::Id::new("eq_title_drag"),
                    egui::Sense::click(),
                );

                // Pre-pass: compute press visuals BEFORE rendering. handle_input
                // runs at the end of this closure; without this pre-pass, the
                // EQ/AUTO/PRESETS overlays render with last frame's state and
                // the press feedback only appears on the next repaint.
                self.update_button_press_visuals(ui, offset);

                self.render_background(ui, offset);
                self.render_band_fills(ui, offset);
                self.render_thumbs(ui, offset);
                self.render_button_overlays(ui, offset);
                self.render_spline(ui, offset);
                self.render_band_flash(ui, offset);
                self.render_presets_menu(ui, offset, audio_engine);
                let input_action = self.handle_input(ui, offset, audio_engine);
                // "Save as preset…" click in the dropdown sets the
                // pending flag during render_presets_menu. Drain it
                // before returning so the modal name-prompt dialog
                // opens exactly once per click.
                if self.pending_save_as_user_preset {
                    self.pending_save_as_user_preset = false;
                    Some(EqualizerAction::SaveAsUserPreset)
                } else {
                    input_action
                }
            });

        inner.inner
    }

    /// Shade-mode rendering: paint the 14-px title strip from `eq_ex.bmp`,
    /// plus the spec's mini volume/balance sliders (3-state thumbs at
    /// y=30) overlaid on top of the strip's baked rails. Close button hot
    /// area + double-click-to-unshade are honored. Skin without
    /// `eq_ex.bmp` falls back to a flat filler so the strip is at least
    /// visible.
    ///
    /// The shade-strip rails are visually part of the strip extract — we
    /// only need to overdraw the 3×7 thumbs at the position derived from
    /// `volume_value` / `balance_value`, and forward the click+drag to
    /// the audio engine via `SetVolume` / `SetBalance`.
    fn show_shade(
        &mut self,
        ctx: &Context,
        dock_y_skin: u32,
        audio_engine: Option<&AudioEngine>,
    ) -> Option<EqualizerAction> {
        let scale = self.renderer.get_scale();
        let strip_size = Vec2::new(275.0 * scale, 14.0 * scale);

        let inner = egui::Area::new(egui::Id::new("wsz_equalizer_window"))
            .fixed_pos(Pos2::new(0.0, dock_y_skin as f32 * scale))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_min_size(strip_size);
                ui.set_max_size(strip_size);
                let area_rect = ui.max_rect();
                let offset = area_rect.min;

                // Background — eq_ex.bmp top strip if present.
                if let Some(atlas) = self
                    .renderer
                    .get_skin()
                    .get_bitmap(&SkinComponent::EqEx)
                    .cloned()
                {
                    if let Some(bg) = atlas.extract_region(0, 0, 275, 14) {
                        self.renderer
                            .render_region(ui, &bg, offset, "eq_shade_strip");
                    }
                } else {
                    ui.painter().rect_filled(
                        Rect::from_min_size(offset, strip_size),
                        0.0,
                        egui::Color32::from_rgb(0x18, 0x1a, 0x1f),
                    );
                }

                // Mini volume + balance thumbs overlaid on top of the rails
                // baked into the strip extract above.
                self.render_shade_thumbs(ui, offset);

                // Strip-wide click sense covers double-click and click
                // (close button). Drag handling for the rails uses the
                // pointer state directly so we don't need an interact area
                // for it.
                let response = ui.interact(
                    Rect::from_min_size(offset, strip_size),
                    egui::Id::new("eq_shade_strip_click"),
                    egui::Sense::click(),
                );
                if response.double_clicked() {
                    self.shade_mode = false;
                }

                let action = self.handle_shade_input(ui, offset, audio_engine);

                if action.is_some() {
                    return action;
                }

                // Close button — same skin coords as full mode (top-right
                // of the strip), tested via single click. Only fires when
                // we're not in the middle of a rail drag.
                if response.clicked()
                    && self.shade_drag.is_none()
                    && let Some(pos) = ui.ctx().pointer_latest_pos()
                {
                    let close_rect = SkinRect {
                        x: 264,
                        y: 3,
                        w: 9,
                        h: 9,
                    }
                    .screen_rect(&self.renderer, offset);
                    if close_rect.contains(pos) {
                        return Some(EqualizerAction::Close);
                    }
                }
                None
            });

        inner.inner
    }

    /// Slider value (preamp at index 0, then 10 bands).
    pub(super) fn value_for(&self, slider_idx: usize) -> f32 {
        if slider_idx == 0 {
            self.preamp
        } else {
            self.eq_bands[slider_idx - 1]
        }
    }
}

/// Shade-mode rail layout constants — exposed to the input module so it
/// can build the same hit rectangles the paint side draws into.
pub(super) const SHADE_RAIL: ShadeRail = ShadeRail {
    vol_x: SHADE_VOL_RAIL_X,
    vol_w: SHADE_VOL_RAIL_W,
    bal_x: SHADE_BAL_RAIL_X,
    bal_w: SHADE_BAL_RAIL_W,
    y: SHADE_THUMB_Y,
};

pub(super) struct ShadeRail {
    pub(super) vol_x: u32,
    pub(super) vol_w: u32,
    pub(super) bal_x: u32,
    pub(super) bal_w: u32,
    pub(super) y: u32,
}

/// Map a dB gain to the thumb's vertical offset within the 14×63 track.
/// 0 = top (+20 dB), 52 = bottom (−20 dB).
pub(super) fn db_to_thumb_offset(gain_db: f32) -> u32 {
    let normalized =
        ((gain_db - EQ_GAIN_MIN_DB) / (EQ_GAIN_MAX_DB - EQ_GAIN_MIN_DB)).clamp(0.0, 1.0);
    ((1.0 - normalized) * SLIDER_TRAVEL as f32).round() as u32
}

/// Map a dB gain to the eqmain.bmp band-fill atlas frame coords. Returns
/// `None` for gains in the deadband around 0 dB (no fill needed). Negative
/// gains live at y=164, positive at y=229; both have 14 frames spaced
/// 15 px apart starting at x=13.
///
/// Per WSZ_FORMAT.md §eqmain.bmp the frame ordering is least → most
/// extreme: frame 0 (x=13) is ±1.42 dB and frame 13 (x=208) is ±20 dB.
/// The previous version of this function inverted that mapping, which
/// painted the +15 dB color sprite for a +5 dB gain — making small
/// boosts look maxed-out and the per-band color palette feel scrambled.
pub(super) fn band_fill_frame(gain_db: f32) -> Option<(u32, u32)> {
    let normalized = (gain_db / EQ_GAIN_MAX_DB).clamp(-1.0, 1.0);
    let abs_idx = (normalized.abs() * 14.0).round() as i32;
    if abs_idx == 0 {
        return None;
    }
    let frame_idx = (abs_idx - 1).clamp(0, 13) as u32;
    let x = 13 + frame_idx * 15;
    let y = if gain_db < 0.0 { 164 } else { 229 };
    Some((x, y))
}

/// Sprite coords for the EQ on/off button. Layout per WSZ_FORMAT.md:
/// off-rel (10,119), off-pr (128,119), on-rel (69,119), on-pr (187,119).
pub(super) fn eq_sprite_coords(on: bool, pressed: bool) -> (u32, u32) {
    let x = match (on, pressed) {
        (false, false) => 10,
        (false, true) => 128,
        (true, false) => 69,
        (true, true) => 187,
    };
    (x, 119)
}

/// Sprite coords for the AUTO button: off-rel (35,119), off-pr (153,119),
/// on-rel (94,119), on-pr (212,119).
pub(super) fn auto_sprite_coords(on: bool, pressed: bool) -> (u32, u32) {
    let x = match (on, pressed) {
        (false, false) => 35,
        (false, true) => 153,
        (true, false) => 94,
        (true, true) => 212,
    };
    (x, 119)
}

/// Centripetal Catmull-Rom-ish interpolation across the 10 band gains.
/// `t` ∈ [0, 1] maps to band-space [0, 9]. Output in dB.
pub(super) fn sample_spline(bands: &[f32; 10], t: f32) -> f32 {
    let pos = t.clamp(0.0, 1.0) * 9.0;
    let i = pos.floor() as isize;
    let frac = pos - pos.floor();

    let p = |idx: isize| -> f32 {
        let idx = idx.clamp(0, 9) as usize;
        bands[idx]
    };
    let p0 = p(i - 1);
    let p1 = p(i);
    let p2 = p(i + 1);
    let p3 = p(i + 2);

    // Catmull-Rom basis matrix.
    let t2 = frac * frac;
    let t3 = t2 * frac;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * frac
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// Pull the dominant opaque color from the spline stripe sprite for use as
/// the curve color. Falls back to Winamp's classic green when the stripe is
/// missing or fully transparent.
pub(super) fn spline_color(stripe: &oneamp_core::wsz::bitmap::BitmapRegion) -> egui::Color32 {
    for chunk in stripe.data.chunks_exact(4) {
        if chunk[3] > 0 {
            return egui::Color32::from_rgb(chunk[0], chunk[1], chunk[2]);
        }
    }
    egui::Color32::from_rgb(0, 200, 80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_to_thumb_offset_extremes() {
        assert_eq!(db_to_thumb_offset(EQ_GAIN_MAX_DB), 0);
        assert_eq!(db_to_thumb_offset(EQ_GAIN_MIN_DB), SLIDER_TRAVEL);
        // 0 dB sits in the middle.
        let mid = db_to_thumb_offset(0.0);
        assert!((mid as i32 - (SLIDER_TRAVEL as i32 / 2)).abs() <= 1);
    }

    #[test]
    fn band_fill_frame_boundaries() {
        assert!(band_fill_frame(0.0).is_none());
        // Max negative → frame 13 of neg row at (208, 164) per WSZ spec.
        let (x, y) = band_fill_frame(-20.0).unwrap();
        assert_eq!(y, 164);
        assert_eq!(x, 208);
        // Max positive → frame 13 of pos row at (208, 229).
        let (x, y) = band_fill_frame(20.0).unwrap();
        assert_eq!(y, 229);
        assert_eq!(x, 208);
        // Smallest non-zero positive gain (~+1.42 dB) → frame 0 at x=13.
        let (x, _) = band_fill_frame(1.42).unwrap();
        assert_eq!(x, 13);
    }

    #[test]
    fn spline_passes_through_endpoints() {
        let bands = [3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -3.0];
        assert!((sample_spline(&bands, 0.0) - 3.0).abs() < 0.01);
        assert!((sample_spline(&bands, 1.0) - (-3.0)).abs() < 0.01);
    }
}
