use super::{
    AUTO_BUTTON, EQ_BUTTON, EQ_GAIN_MAX_DB, EQ_GAIN_MIN_DB, EqButton, EqualizerWindow,
    PRESETS_BUTTON, PresetRow, SHADE_BAL_DEADZONE, SHADE_BAL_RAIL_W, SHADE_BAL_RAIL_X,
    SHADE_THUMB_Y, SHADE_VOL_RAIL_W, SHADE_VOL_RAIL_X, SLIDER_TOP_Y, TRACK_HEIGHT, TRACK_X,
    auto_sprite_coords, band_fill_frame, db_to_thumb_offset, eq_sprite_coords, sample_spline,
    spline_color,
};
use egui::{Pos2, Rect, Sense, Vec2};
use oneamp_core::equalizer_presets::BuiltinPresets;
use oneamp_core::wsz::skin::SkinComponent;
use oneamp_core::{AudioCommand, AudioEngine};

// Shade-rail constants are also referenced by input.rs via the ShadeRail
// struct, but the paint side keeps using the originals to avoid the extra
// indirection in the rendering hot path.

impl EqualizerWindow {
    /// Paint the 3×7 volume + balance mini-thumbs from `eq_ex.bmp` at the
    /// position derived from `volume_value` / `balance_value`. The sprite
    /// itself is the spec's 3-state pattern (low/mid/high or L/M/R) — we
    /// pick the variant by bucketing the value, matching how Winamp draws
    /// these in shade mode.
    pub(super) fn render_shade_thumbs(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::EqEx)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // Volume rail: 97 px wide at (61,4); 3-state sprites at (1,30)
        // (low), (4,30) (mid), (7,30) (high). Thumb is 3×7. Travel = 97-3.
        let vol = self.volume_value.clamp(0.0, 1.0);
        let vol_sprite_x = if vol < 1.0 / 3.0 {
            1
        } else if vol < 2.0 / 3.0 {
            4
        } else {
            7
        };
        if let Some(thumb) = atlas.extract_region(vol_sprite_x, 30, 3, 7) {
            let thumb_skin_x = SHADE_VOL_RAIL_X + (vol * (SHADE_VOL_RAIL_W as f32 - 3.0)) as u32;
            let pos = self
                .renderer
                .skin_to_screen(thumb_skin_x, SHADE_THUMB_Y, offset);
            self.renderer.render_region(
                ui,
                &thumb,
                pos,
                &format!("eq_shade_vol_thumb_{}", vol_sprite_x),
            );
        }

        // Balance rail: 42 px wide at (164,4); sprites at (11/14/17, 30).
        // Travel = 42-3.
        let bal = self.balance_value.clamp(-1.0, 1.0);
        let bal_sprite_x = if bal < -SHADE_BAL_DEADZONE {
            11
        } else if bal > SHADE_BAL_DEADZONE {
            17
        } else {
            14
        };
        if let Some(thumb) = atlas.extract_region(bal_sprite_x, 30, 3, 7) {
            let normalized = (bal + 1.0) / 2.0;
            let thumb_skin_x =
                SHADE_BAL_RAIL_X + (normalized * (SHADE_BAL_RAIL_W as f32 - 3.0)) as u32;
            let pos = self
                .renderer
                .skin_to_screen(thumb_skin_x, SHADE_THUMB_Y, offset);
            self.renderer.render_region(
                ui,
                &thumb,
                pos,
                &format!("eq_shade_bal_thumb_{}", bal_sprite_x),
            );
        }
    }

    pub(super) fn render_background(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::EqMain)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // The visible window is the top 275×116 of eqmain.bmp. Anything below
        // that is sprite frames we composite ourselves.
        if let Some(bg) = atlas.extract_region(0, 0, 275, 116) {
            self.renderer
                .render_region(ui, &bg, offset, "eq_background");
        }

        // Title strip overlay. The base-2.91 skin (and most stock Winamp
        // skins) ships a blank "active" strip at y=0..14 — the actual
        // "WINAMP EQUALIZER" artwork lives in the lower extracts:
        //   * y=134..148 — bright variant (active state)
        //   * y=149..163 — dimmer variant (inactive state)
        // The spec mislabels both as "inactive"; sampling the pixels
        // shows y=134 is the brighter of the two. Always overlay the
        // appropriate strip so the user sees the title text, brightening
        // it when the EQ is focused and dimming it when it isn't —
        // visually identical to Winamp's active/inactive transition.
        let strip_src_y: u32 = if self.focused { 134 } else { 149 };
        let strip_tag: &str = if self.focused { "active" } else { "inactive" };
        if let Some(strip) = atlas.extract_region(0, strip_src_y, 275, 14) {
            let pos = self.renderer.skin_to_screen(0, 0, offset);
            self.renderer
                .render_region(ui, &strip, pos, &format!("eq_title_{strip_tag}"));
        }
    }

    pub(super) fn render_band_fills(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::EqMain)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // Track guide for every slider (preamp + 10 bands). The band fill
        // sprites at y=164/229 ARE the visible colored bar — at exactly 0 dB
        // `band_fill_frame` returns None and nothing draws, so a slider at
        // rest would otherwise show just an isolated thumb on an empty
        // strip. Painting a thin vertical guide down the middle of every
        // track keeps all 11 sliders visually consistent regardless of
        // gain. Color is sampled from the band fill sprite so it matches
        // the active skin's palette.
        //
        // Width is 2 skin-pixels so it stays visible at every scale step
        // — a 1-px logical line vanishes on hi-DPI displays once egui
        // rounds it down to the nearest physical pixel.
        let scale = self.renderer.get_scale();
        let track_color = self.slider_track_color(&atlas);
        let track_width_skin: f32 = 2.0;
        let track_top_px = SLIDER_TOP_Y as f32 * scale;
        let track_height_px = TRACK_HEIGHT as f32 * scale;
        let track_width_px = track_width_skin * scale;
        for &track_x in TRACK_X.iter() {
            let track_x_center = (track_x + 14 / 2) as f32 * scale;
            ui.painter().rect_filled(
                Rect::from_min_size(
                    Pos2::new(
                        offset.x + track_x_center - track_width_px / 2.0,
                        offset.y + track_top_px,
                    ),
                    Vec2::new(track_width_px, track_height_px),
                ),
                0.0,
                track_color,
            );
        }

        // 11 sliders (preamp + 10 bands). All use the same fill atlas; the
        // skin's neg-dB and pos-dB strips composite onto the track from the
        // center outward.
        for (slider_idx, &track_x) in TRACK_X.iter().enumerate() {
            let gain = self.value_for(slider_idx);
            let Some((frame_x, frame_y)) = band_fill_frame(gain) else {
                continue;
            };

            if let Some(fill) = atlas.extract_region(frame_x, frame_y, 14, 63) {
                let pos = self.renderer.skin_to_screen(track_x, SLIDER_TOP_Y, offset);
                self.renderer.render_region(
                    ui,
                    &fill,
                    pos,
                    &format!("eq_fill_{}_{}", slider_idx, frame_x),
                );
            }
        }
    }

    pub(super) fn render_thumbs(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::EqMain)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        for (slider_idx, &track_x) in TRACK_X.iter().enumerate() {
            let gain = self.value_for(slider_idx);
            let pressed = self.dragging == Some(slider_idx);
            let thumb_y = if pressed { 176 } else { 164 };
            let Some(thumb) = atlas.extract_region(0, thumb_y, 11, 11) else {
                continue;
            };

            // Thumb is 11 px wide on a 14 px track — center it horizontally.
            let thumb_skin_x = track_x + (14 - 11) / 2;
            let thumb_skin_y = SLIDER_TOP_Y + db_to_thumb_offset(gain);
            let pos = self
                .renderer
                .skin_to_screen(thumb_skin_x, thumb_skin_y, offset);
            self.renderer.render_region(
                ui,
                &thumb,
                pos,
                &format!("eq_thumb_{}_{}", slider_idx, pressed),
            );
        }
    }

    pub(super) fn render_button_overlays(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::EqMain)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // EQ / AUTO buttons live on row y=119 of eqmain.bmp as 4 sprites each
        // (off-rel, off-pr, on-rel, on-pr). The background extract at (14,18)
        // and (39,18) carries the *default* off-rel state for skins that ship
        // no alternate sprites — but real skins ship them, and overlaying
        // only when the state changes leaves the off-state showing through
        // any magenta-keyed alternate sprite. Always overdraw the active
        // state so the button visibly switches between off/on/pressed.
        let eq_pressed = self.pressed_button == Some(EqButton::Eq);
        let (sx, sy) = eq_sprite_coords(self.enabled, eq_pressed);
        if let Some(region) = atlas.extract_region(sx, sy, EQ_BUTTON.w, EQ_BUTTON.h) {
            let pos = self
                .renderer
                .skin_to_screen(EQ_BUTTON.x, EQ_BUTTON.y, offset);
            self.renderer.render_region(
                ui,
                &region,
                pos,
                &format!("eq_btn_eq_{}_{}", self.enabled, eq_pressed),
            );
        }

        let auto_pressed = self.pressed_button == Some(EqButton::Auto);
        let (sx, sy) = auto_sprite_coords(self.auto, auto_pressed);
        if let Some(region) = atlas.extract_region(sx, sy, AUTO_BUTTON.w, AUTO_BUTTON.h) {
            let pos = self
                .renderer
                .skin_to_screen(AUTO_BUTTON.x, AUTO_BUTTON.y, offset);
            self.renderer.render_region(
                ui,
                &region,
                pos,
                &format!("eq_btn_auto_{}_{}", self.auto, auto_pressed),
            );
        }

        // PRESETS button. Per WSZ_FORMAT.md §eqmain.bmp the unpressed sprite
        // lives at the duplicate atlas location (224,164) and the pressed
        // variant at (224,176). Some skins (incl. base-2.91) ship a blank
        // strip at the baked (217,18) zone, so the unpressed label only
        // shows up when we overlay it ourselves — same approach as the
        // EQ/AUTO overlays above.
        let presets_pressed = self.pressed_button == Some(EqButton::Presets);
        let presets_sprite_y = if presets_pressed { 176 } else { 164 };
        if let Some(region) =
            atlas.extract_region(224, presets_sprite_y, PRESETS_BUTTON.w, PRESETS_BUTTON.h)
        {
            let pos = self
                .renderer
                .skin_to_screen(PRESETS_BUTTON.x, PRESETS_BUTTON.y, offset);
            self.renderer.render_region(
                ui,
                &region,
                pos,
                &format!("eq_btn_presets_{}", presets_pressed),
            );
        }
    }

    /// Render the 113×20 minidisplay spline. Background already painted by
    /// `render_background`; we only need to overdraw the curve.
    pub(super) fn render_spline(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::EqMain)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };
        let scale = self.renderer.get_scale();

        const SPLINE_X: u32 = 86;
        const SPLINE_Y: u32 = 17;
        const SPLINE_W: u32 = 113;
        const SPLINE_H: u32 = 19;

        // Sample the per-band gains across SPLINE_W x positions via cubic
        // interpolation (smoother than linear; matches the visual feel of
        // the Winamp curve). Y-axis: top of strip = +20 dB, bottom = -20 dB.
        let stripe = atlas.extract_region(115, 294, 1, SPLINE_H);
        if let Some(stripe) = stripe {
            for px in 0..SPLINE_W {
                let t = px as f32 / (SPLINE_W - 1) as f32;
                let gain = sample_spline(&self.eq_bands, t);
                let normalized =
                    ((gain - EQ_GAIN_MIN_DB) / (EQ_GAIN_MAX_DB - EQ_GAIN_MIN_DB)).clamp(0.0, 1.0);
                let stripe_offset = ((1.0 - normalized) * (SPLINE_H - 1) as f32).round() as i32;
                // The stripe is 19 px tall; we shift it vertically so the
                // stripe's center sits at the computed y (1-px wide vertical
                // line emerges from the stripe's centermost pixel).
                let abs_x = SPLINE_X + px;
                let abs_y_center = SPLINE_Y + (SPLINE_H / 2);
                let y_target =
                    (abs_y_center as i32 + stripe_offset - (SPLINE_H as i32 / 2)).max(0) as u32;
                let pos = self.renderer.skin_to_screen(abs_x, y_target, offset);
                // Paint a single 1×1 px dot from the brightest stripe color
                // to avoid bloating the texture cache with 113 stripe variants.
                ui.painter().rect_filled(
                    Rect::from_min_size(pos, Vec2::new(scale, scale)),
                    0.0,
                    spline_color(&stripe),
                );
            }
        }

        // Preamp baseline: a horizontal line at the preamp's normalized
        // position — drawn whenever the preamp is non-zero.
        if self.preamp.abs() > 0.01 {
            let normalized = ((self.preamp - EQ_GAIN_MIN_DB) / (EQ_GAIN_MAX_DB - EQ_GAIN_MIN_DB))
                .clamp(0.0, 1.0);
            let line_skin_y = SPLINE_Y + ((1.0 - normalized) * (SPLINE_H - 1) as f32) as u32;
            if let Some(line) = atlas.extract_region(0, 314, SPLINE_W, 1) {
                let pos = self.renderer.skin_to_screen(SPLINE_X, line_skin_y, offset);
                self.renderer
                    .render_region(ui, &line, pos, "eq_preamp_line");
            }
        }
    }

    /// Paint a temporary glow around the band selected by Shift-
    /// clicking the main-window spectrum visualiser. The state is set
    /// by [`EqualizerWindow::flash_band`]; we draw a green rounded
    /// outline whose alpha ramps down over the remaining lifetime so
    /// the indicator fades instead of popping out. Cleared once the
    /// deadline passes.
    pub(super) fn render_band_flash(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let Some((band, until)) = self.flash_band_state else {
            return;
        };
        let now = std::time::Instant::now();
        if now >= until {
            self.flash_band_state = None;
            return;
        }
        // band ∈ 0..10 maps to TRACK_X[1..11] (slot 0 is the preamp).
        let track_x = match TRACK_X.get(band + 1) {
            Some(&x) => x,
            None => return,
        };
        let scale = self.renderer.get_scale();
        let top_left = self.renderer.skin_to_screen(track_x, SLIDER_TOP_Y, offset);
        let rect = Rect::from_min_size(
            top_left,
            Vec2::new(14.0 * scale, TRACK_HEIGHT as f32 * scale),
        );
        // Linear alpha ramp over the lifetime, scaled by a 1.5×
        // brightness peak at the start so the first frame is loud
        // before settling. Clamp at 255 because we multiply.
        let remaining_ms = (until - now).as_millis() as f32;
        let alpha = (remaining_ms / 1500.0).clamp(0.0, 1.0);
        let accent_a = (255.0 * alpha) as u8;
        let glow = egui::Color32::from_rgba_unmultiplied(120, 240, 120, accent_a);
        // Layer the outline twice (1 px + 3 px) for a soft halo look
        // without needing actual blur — egui's CPU painter doesn't
        // ship one.
        ui.painter()
            .rect_stroke(rect.expand(2.0 * scale), 3.0, egui::Stroke::new(3.0, glow));
        ui.painter()
            .rect_stroke(rect, 2.0, egui::Stroke::new(1.5, glow));
    }

    pub(super) fn render_presets_menu(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        audio_engine: Option<&AudioEngine>,
    ) {
        if !self.presets_menu_open {
            return;
        }
        let scale = self.renderer.get_scale();
        // Right-align the menu to the PRESETS button's right edge so it
        // doesn't overflow the 275-wide EQ window. Winamp does the same
        // when the menu would otherwise clip on the right.
        let menu_w_scaled = 130.0 * scale;
        let presets_right_x = (PRESETS_BUTTON.x + PRESETS_BUTTON.w) as f32 * scale + offset.x;
        let menu_x = presets_right_x - menu_w_scaled;
        let menu_y = (PRESETS_BUTTON.y + PRESETS_BUTTON.h) as f32 * scale + offset.y;

        // Build the row list: I/O entries first, then built-ins, then
        // user-defined presets at the bottom. User presets render in a
        // softer cyan so they're visually distinct from the green
        // built-ins without breaking the dark skin palette.
        let presets = BuiltinPresets::all();
        let user_presets = self.user_presets.clone();
        let mut rows: Vec<PresetRow> = Vec::with_capacity(presets.len() + user_presets.len() + 3);
        rows.push(PresetRow::LoadEqf);
        rows.push(PresetRow::SaveEqf);
        rows.push(PresetRow::SaveAsUserPreset);
        for p in presets.iter() {
            rows.push(PresetRow::Builtin(p));
        }
        for p in user_presets.iter() {
            rows.push(PresetRow::User(p));
        }

        let row_h = 14.0 * scale;
        let menu_w = 130.0 * scale;
        let menu_h = row_h * rows.len() as f32;
        let menu_rect = Rect::from_min_size(Pos2::new(menu_x, menu_y), Vec2::new(menu_w, menu_h));

        // Render the dropdown inside its own top-order `Area`. The
        // EQ's parent Area is sized 275×116 in skin units and its
        // painter clips to that rect — without a separate Area, any
        // rows past y=116 would be silently cropped (especially
        // visible when the playlist sub-window isn't docked below,
        // so the OS viewport doesn't even *try* to draw past the EQ
        // floor). Anchoring to `menu_rect.min` with
        // `Order::Foreground` puts the menu on top of every other
        // sub-window's pixels and gives it its own clip rect sized
        // to fit the full row stack.
        //
        // The OS viewport still needs to be tall enough to hold the
        // overflow — that's handled in the coordinator via
        // `EqualizerWindow::preset_menu_overlay_extra_skin`.
        let mut hover_close = false;
        let menu_ctx = ui.ctx().clone();
        let active_preset = self.current_preset_name.clone();
        let mut click_action: Option<usize> = None;
        let mut hovered_pos: Option<egui::Pos2> = None;
        egui::Area::new(egui::Id::new("wsz_equalizer_presets_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(menu_rect.min)
            .show(&menu_ctx, |area_ui| {
                area_ui.set_min_size(menu_rect.size());
                area_ui.set_max_size(menu_rect.size());

                area_ui
                    .painter()
                    .rect_filled(menu_rect, 0.0, egui::Color32::from_rgb(20, 20, 20));
                area_ui.painter().rect_stroke(
                    menu_rect,
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 200, 80)),
                );

                for (i, row) in rows.iter().enumerate() {
                    let row_rect = Rect::from_min_size(
                        Pos2::new(menu_x, menu_y + i as f32 * row_h),
                        Vec2::new(menu_w, row_h),
                    );
                    let response = area_ui.interact(
                        row_rect,
                        egui::Id::new(("eq_preset_row", i)),
                        Sense::click(),
                    );
                    let label = match row {
                        PresetRow::LoadEqf => "Load .eqf…",
                        PresetRow::SaveEqf => "Save as .eqf…",
                        PresetRow::SaveAsUserPreset => "Save as preset…",
                        PresetRow::Builtin(p) => p.name.as_str(),
                        PresetRow::User(p) => p.name.as_str(),
                    };
                    // Highlight whichever preset row matches the
                    // currently-applied name so the user can see at
                    // a glance which one is active.
                    let is_active = matches!(row, PresetRow::Builtin(_) | PresetRow::User(_))
                        && active_preset.as_deref() == Some(label);
                    let bg = if response.hovered() {
                        egui::Color32::from_rgb(40, 80, 40)
                    } else if is_active {
                        egui::Color32::from_rgb(28, 50, 28)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    area_ui.painter().rect_filled(row_rect, 0.0, bg);
                    // Colour code: green for built-ins, cyan for user
                    // presets (so the home-grown ones are easy to spot
                    // in a long list), white for I/O / save actions.
                    let color = match row {
                        PresetRow::Builtin(_) => egui::Color32::from_rgb(0, 220, 80),
                        PresetRow::User(_) => egui::Color32::from_rgb(120, 200, 230),
                        _ => egui::Color32::from_rgb(220, 220, 220),
                    };
                    area_ui.painter().text(
                        row_rect.left_center() + Vec2::new(6.0 * scale, 0.0),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(10.0 * scale),
                        color,
                    );
                    if response.clicked() {
                        click_action = Some(i);
                    }
                    if response.hovered() {
                        hovered_pos = Some(row_rect.center());
                    }
                }
            });
        let _ = hovered_pos; // (kept for parity with the previous body)

        if let Some(i) = click_action {
            let row = &rows[i];
            // `apply_preset` runs the same band+preamp swap for both
            // built-in and user-defined presets. Defined inline because
            // it captures `audio_engine` and `self.eq_bands` /
            // `self.preamp` / `self.current_preset_name` mutably.
            let apply_preset =
                |this: &mut Self, preset: &oneamp_core::equalizer_presets::EqualizerPreset| {
                    let mut padded = [0.0; 10];
                    for (j, slot) in padded.iter_mut().enumerate() {
                        *slot = preset
                            .gains
                            .get(j)
                            .copied()
                            .unwrap_or(0.0)
                            .clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
                    }
                    this.eq_bands = padded;
                    let new_preamp = preset.preamp_db.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
                    this.preamp = new_preamp;
                    this.current_preset_name = Some(preset.name.clone());
                    if let Some(engine) = audio_engine {
                        let _ =
                            engine.send_command(AudioCommand::SetEqualizerBands(padded.to_vec()));
                        let _ = engine.send_command(AudioCommand::SetEqualizerPreamp(new_preamp));
                    }
                };
            match row {
                PresetRow::LoadEqf => {
                    self.handle_load_eqf(audio_engine);
                }
                PresetRow::SaveEqf => {
                    self.handle_save_eqf();
                }
                PresetRow::SaveAsUserPreset => {
                    // Defer the actual save to the app — it owns the
                    // PresetManager + modal name dialog. The flag is
                    // drained from `show()` and turned into
                    // `EqualizerAction::SaveAsUserPreset`.
                    self.pending_save_as_user_preset = true;
                }
                PresetRow::Builtin(preset) => {
                    apply_preset(self, preset);
                }
                PresetRow::User(preset) => {
                    // Clone the user preset locally — the borrow into
                    // `self.user_presets` collides with the mutable
                    // borrows `apply_preset` needs.
                    let cloned = (*preset).clone();
                    apply_preset(self, &cloned);
                }
            }
            hover_close = true;
        }

        // Click outside the menu closes it. We also close after applying a
        // preset so the user sees their selection take effect immediately.
        //
        // The outside-click check fires on mouse-down (primary_pressed),
        // NOT mouse-up (primary_clicked). The click that OPENED the menu
        // ends in a mouse-up on the PRESETS button — which sits above the
        // menu rect, so a `primary_clicked()` check would treat that
        // release as a click outside and immediately close the menu the
        // same frame it first renders. Using primary_pressed dodges that:
        // the opening press happens before render_presets_menu sees
        // `presets_menu_open == true`, and only a fresh press elsewhere
        // closes the menu.
        if hover_close {
            self.presets_menu_open = false;
        } else {
            // "Outside" excludes both the menu AND the PRESETS button —
            // clicks on the button itself are handled by handle_input's
            // toggle, which would otherwise race this check (close fires
            // first → toggle re-opens → menu stays open on every click).
            let presets_rect = PRESETS_BUTTON.screen_rect(&self.renderer, offset);
            let click_outside = ui.ctx().input(|i| {
                i.pointer.primary_pressed()
                    && i.pointer
                        .interact_pos()
                        .map(|p| !menu_rect.contains(p) && !presets_rect.contains(p))
                        .unwrap_or(false)
            });
            if click_outside {
                self.presets_menu_open = false;
            }
        }
    }

    /// Sample a color from the most-saturated negative-dB band fill sprite
    /// (`(13, 164)`, the −20 dB frame). The pixel at the center of the bar
    /// is the brightest opaque pixel in the column, which we use as the
    /// procedural slider track color so it matches whatever palette the
    /// active skin chose. Falls back to a Winamp-flavored green when the
    /// sample point is transparent or out of bounds.
    fn slider_track_color(&self, atlas: &oneamp_core::wsz::bitmap::BitmapAtlas) -> egui::Color32 {
        // Center of the 14-wide −20 dB fill sprite, midway down the 63 px
        // height — a row that's reliably inside the colored bar regardless
        // of how the skin shaped the gradient.
        let sample_x = 13 + 14 / 2;
        let sample_y = 164 + TRACK_HEIGHT / 2;
        if sample_x >= atlas.width || sample_y >= atlas.height {
            return egui::Color32::from_rgb(0, 200, 80);
        }
        let idx = ((sample_y * atlas.width + sample_x) * 4) as usize;
        match atlas.data.get(idx..idx + 4) {
            Some(p) if p[3] > 0 => egui::Color32::from_rgb(p[0], p[1], p[2]),
            _ => egui::Color32::from_rgb(0, 200, 80),
        }
    }
}
