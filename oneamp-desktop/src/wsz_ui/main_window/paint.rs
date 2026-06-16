use super::{MainWindowAction, VisualizerMode, WszMainWindow};
use egui::{Pos2, Vec2};
use oneamp_core::wsz::skin::SkinComponent;
use std::sync::OnceLock;

/// Cached once per process: was OneAmp launched under a Wayland session?
/// Read from `WAYLAND_DISPLAY` at first use. The Options menu uses it to
/// label the "Always on top" entry "(X11 only)" since winit's
/// `set_window_level` is a no-op on the Wayland backend.
fn is_wayland_session() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        std::env::var("WAYLAND_DISPLAY")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    })
}

impl WszMainWindow {
    pub(super) fn render_background(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        self.renderer
            .render_component(ui, &SkinComponent::Main, offset);
    }

    pub(super) fn render_visualization(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        spectrum: &[f32],
        waveform: &oneamp_core::WaveformSnapshot,
        meter: &oneamp_core::MeterSnapshot,
        _delta_time: f32,
    ) {
        let scale = self.renderer.get_scale();

        if !spectrum.is_empty() {
            self.spectrum.update(spectrum);
        }
        if !waveform.is_empty() {
            self.oscilloscope.update(waveform);
        }
        self.peak_meter.update(meter);

        // Per WSZ_FORMAT.md main.bmp note: when stopped, the visualization
        // area is not drawn — the underlying main.bmp background shows
        // through. mono_stereo is its own thing and stays visible.
        let vis_colors = self.renderer.get_skin().vis_colors;
        let active = self.is_playing || self.is_paused;
        if active {
            match self.visualizer_mode {
                VisualizerMode::Spectrum => {
                    self.spectrum.render(ui, offset, scale, &vis_colors);
                }
                VisualizerMode::Oscilloscope => {
                    self.oscilloscope.render(ui, offset, scale, &vis_colors);
                }
                VisualizerMode::PeakMeter => {
                    self.peak_meter.render(ui, offset, scale, &vis_colors);
                }
                VisualizerMode::Off => {}
            }
        }
        self.mono_stereo.render(&mut self.renderer, ui, offset);
    }

    pub(super) fn render_info_displays(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        self.bitrate_display.render(&mut self.renderer, ui, offset);
        self.title_scroller.render(&mut self.renderer, ui, offset);
    }

    pub(super) fn render_play_state(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        self.play_state.render(&mut self.renderer, ui, offset);
    }

    /// Render the four MM:SS digits from `nums.bmp`. The atlas holds 11
    /// 9×13 sprites laid out horizontally: `0`-`9` then a minus sign at
    /// index 10. Winamp does NOT include a colon glyph in `nums.bmp` — the
    /// player paints two small dots in the gap between minutes and seconds
    /// itself, which we reproduce below.
    pub(super) fn render_display(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let scale = self.renderer.get_scale();
        let (display_x, display_y) = self.display.position();
        let (digit_w, digit_h) = self.display.digit_size();
        let time = self.display.time_digits();

        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Numbers)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // Spec WSZ §main.bmp: minute tens at x=48, minute ones at x=60,
        // second tens at x=78, second ones at x=90 → offsets [0,12,30,42]
        // relative to display_x=48. The 9-wide colon gap sits between
        // slots 1 and 2 (between x=60+9=69 and x=78).
        const SLOT_OFFSETS: [u32; 4] = [0, 12, 30, 42];
        const MINUS_SPRITE_INDEX: u32 = 10;

        for (i, &d) in time.digits.iter().enumerate() {
            let sprite_index = if i == 0 && time.minus {
                MINUS_SPRITE_INDEX
            } else {
                u32::from(d)
            };
            let sprite_x = sprite_index * digit_w;
            if let Some(region) = atlas.extract_region(sprite_x, 0, digit_w, digit_h) {
                let abs_x = display_x + SLOT_OFFSETS[i];
                let pos = self.renderer.skin_to_screen(abs_x, display_y, offset);
                self.renderer
                    .render_region(ui, &region, pos, &format!("digit_{}", i));
            }
        }

        // Colon dots between slots 1 and 2. Approximate LCD green is good
        // enough for stock skins; a future pass can sample the lit color
        // from the atlas to match custom skins.
        let colon_color = egui::Color32::from_rgb(0, 255, 0);
        let dot = 2.0 * scale;
        // Colon sits in the 9-px gap between digit 1 (ends at +21) and digit 2
        // (starts at +30). +24 centers the 2-px dot in that gap.
        for dy in [3u32, 8u32] {
            let abs = self
                .renderer
                .skin_to_screen(display_x + 24, display_y + dy, offset);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(abs, egui::Vec2::splat(dot)),
                0.0,
                colon_color,
            );
        }
    }

    pub(super) fn render_position_slider(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        // Per WSZ_FORMAT.md: when stopped, the position bar is not drawn —
        // the underlying main.bmp background shows through.
        if !self.is_playing && !self.is_paused {
            return;
        }
        // posbar.bmp is a single 248×10 track + two 29×10 thumb sprites
        // (unpressed at x=248, pressed at x=278). The thumb is composited on
        // top of the static track at `progress * (248 - 29)`.
        let posbar = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::PosBar)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };
        let (slider_x, slider_y) = self.position_slider.position();
        let track_h = posbar.height.min(10);

        if let Some(track) = posbar.extract_region(0, 0, 248, track_h) {
            let pos = self.renderer.skin_to_screen(slider_x, slider_y, offset);
            self.renderer.render_region(ui, &track, pos, "posbar_track");
        }

        if posbar.width >= 277 {
            let thumb_x_in_atlas = if self.position_slider.is_dragging {
                278
            } else {
                248
            };
            if let Some(thumb) = posbar.extract_region(thumb_x_in_atlas, 0, 29, track_h) {
                let progress = self.position_slider.progress.clamp(0.0, 1.0);
                let thumb_skin_x = slider_x + (progress * (248.0 - 29.0)) as u32;
                let pos = self.renderer.skin_to_screen(thumb_skin_x, slider_y, offset);
                self.renderer.render_region(ui, &thumb, pos, "posbar_thumb");
            }
        }
    }

    pub(super) fn render_buttons(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        // Each button declares its source atlas (cbuttons.bmp or shufrep.bmp).
        // We extract per-button so the two atlases co-exist without aliasing.
        //
        // The cache key MUST include the sprite atlas coordinates: pressed
        // and unpressed sprites of the same button share the same render
        // position, and a single `button_{:?}` key would cause the texture
        // cache to lock onto whichever variant got rendered first — the
        // press feedback would then never appear.
        let button_regions: Vec<_> = self
            .buttons
            .buttons()
            .iter()
            .filter_map(|button| {
                let (btn_w, btn_h) = button.button_type.size();
                let (sprite_x, sprite_y) = button.get_current_sprite_coords();
                let atlas = self
                    .renderer
                    .get_skin()
                    .get_bitmap(&button.button_type.source_atlas())?;
                atlas
                    .extract_region(sprite_x, sprite_y, btn_w, btn_h)
                    .map(|region| {
                        (
                            region,
                            button.button_type.position(),
                            format!("button_{:?}_{}_{}", button.button_type, sprite_x, sprite_y),
                        )
                    })
            })
            .collect();

        for (button_region, (btn_x, btn_y), name) in button_regions {
            let pos = self.renderer.skin_to_screen(btn_x, btn_y, offset);
            self.renderer.render_region(ui, &button_region, pos, &name);
        }
    }

    pub(super) fn render_volume_slider(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        // volume.bmp: 28 rail frames stacked at y=0,15,30,...,405 (68×13 each)
        // + thumb at (15,422) unpressed / (0,422) pressed (14×11). Frame index
        // = round(value × 27).
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Volume)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };
        if atlas.width < 68 || atlas.height < 13 {
            return;
        }

        let (slider_x, slider_y) = self.volume_slider.position();
        let frame = self.volume_slider.get_frame();

        if let Some(rail) = atlas.extract_region(0, frame * 15, 68, 13) {
            let pos = self.renderer.skin_to_screen(slider_x, slider_y, offset);
            self.renderer.render_region(ui, &rail, pos, "volume_rail");
        }

        if atlas.height >= 433 {
            let thumb_x_in_atlas = if self.volume_slider.is_dragging {
                0
            } else {
                15
            };
            if let Some(thumb) = atlas.extract_region(thumb_x_in_atlas, 422, 14, 11) {
                let value = self.volume_slider.value.clamp(0.0, 1.0);
                let thumb_skin_x = slider_x + (value * (68.0 - 14.0)) as u32;
                // Thumb is 11 px tall vs the 13-px rail — center it vertically (+1).
                let pos = self
                    .renderer
                    .skin_to_screen(thumb_skin_x, slider_y + 1, offset);
                self.renderer.render_region(ui, &thumb, pos, "volume_thumb");
            }
        }
    }

    pub(super) fn render_balance_slider(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        // balance.bmp: same vertical stripe layout as volume but the visible
        // portion is the central 38 px (cols 9..47). Thumb at (15,422) /
        // (0,422). Frame index from |balance| (centered = filler).
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Balance)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };
        if atlas.width < 47 || atlas.height < 13 {
            return;
        }

        let (slider_x, slider_y) = self.balance_slider.position();
        let frame = self.balance_slider.get_frame();

        if let Some(rail) = atlas.extract_region(9, frame * 15, 38, 13) {
            let pos = self.renderer.skin_to_screen(slider_x, slider_y, offset);
            self.renderer.render_region(ui, &rail, pos, "balance_rail");
        }

        if atlas.height >= 433 {
            let thumb_x_in_atlas = if self.balance_slider.is_dragging {
                0
            } else {
                15
            };
            if let Some(thumb) = atlas.extract_region(thumb_x_in_atlas, 422, 14, 11) {
                let normalized = (self.balance_slider.value + 1.0) / 2.0;
                let thumb_skin_x = slider_x + (normalized.clamp(0.0, 1.0) * (38.0 - 14.0)) as u32;
                let pos = self
                    .renderer
                    .skin_to_screen(thumb_skin_x, slider_y + 1, offset);
                self.renderer
                    .render_region(ui, &thumb, pos, "balance_thumb");
            }
        }
    }

    /// Draw the Options popup spawned by the clutterbar `O` letter.
    /// Returns an action when the user picks one of the entries; the
    /// menu also closes when the user clicks anywhere outside it.
    pub(super) fn render_options_menu(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
    ) -> Option<MainWindowAction> {
        if !self.options_menu_open {
            return None;
        }
        let scale = self.renderer.get_scale();

        // Anchor right of the clutterbar's `O` slot.
        let menu_x = offset.x + 20.0 * scale;
        let menu_y = offset.y + 22.0 * scale;
        let menu_w = 160.0 * scale;
        let row_h = 14.0 * scale;

        // Recent-files entries follow the main toggles, capped at 5 so the
        // popup doesn't grow taller than the player. Display name is the
        // file stem — full paths would overflow the 160 px popup.
        const RECENT_CAP: usize = 5;
        let recent: Vec<(String, MainWindowAction)> = self
            .recent_paths
            .iter()
            .take(RECENT_CAP)
            .map(|p| {
                let label = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                (label, MainWindowAction::PlayRecent(p.clone()))
            })
            .collect();

        // Row vector: each entry is (label, action, checked_opt).
        // `checked_opt == None` skips the ✓ column entirely (used for
        // non-toggleable rows like "About" and the Recent entries).
        let mut rows: Vec<(String, MainWindowAction, Option<bool>)> = vec![
            (
                "Open folder…".to_string(),
                MainWindowAction::OpenFolder,
                None,
            ),
            (
                // Suffix "(X11 only)" when we detect a Wayland session
                // — winit's `set_window_level` is a no-op on Wayland
                // (no `_NET_WM_STATE_ABOVE` equivalent in the core
                // protocol), so the toggle wouldn't visibly raise the
                // player. We let the user keep the persisted flag (in
                // case they hop back to X11) but make the limitation
                // legible from the menu itself.
                if is_wayland_session() {
                    "Always on top (X11 only)".to_string()
                } else {
                    "Always on top".to_string()
                },
                MainWindowAction::ToggleAlwaysOnTop,
                Some(self.always_on_top),
            ),
            (
                "Crossfade".to_string(),
                MainWindowAction::ToggleCrossfade,
                Some(self.crossfade_enabled),
            ),
            (
                "ReplayGain".to_string(),
                MainWindowAction::ToggleReplayGain,
                Some(self.replaygain_enabled),
            ),
            (
                "Track notifications".to_string(),
                MainWindowAction::ToggleTrackNotifications,
                Some(self.track_notifications_enabled),
            ),
            (
                "About OneAmp".to_string(),
                MainWindowAction::ShowAbout,
                None,
            ),
        ];
        // Output-device group: one radio-style row per cpal device, with
        // a leading "Default device" row that maps to None. Skipped when
        // no devices are enumerable (headless / cpal init failed) so the
        // menu doesn't show a dead section. Capped at 10 to keep the
        // popup from outgrowing the screen on systems with many monitors
        // (PulseAudio sinks + monitors can easily reach 20).
        let device_separator_idx = if !self.output_devices.is_empty() {
            let idx = rows.len();
            rows.push((
                "Default device".to_string(),
                MainWindowAction::SelectOutputDevice(None),
                Some(self.current_output_device.is_none()),
            ));
            for name in self.output_devices.iter().take(10) {
                let label = if name.len() > 24 {
                    format!("{}…", &name[..23])
                } else {
                    name.clone()
                };
                let checked = self.current_output_device.as_deref() == Some(name.as_str());
                rows.push((
                    label,
                    MainWindowAction::SelectOutputDevice(Some(name.clone())),
                    Some(checked),
                ));
            }
            Some(idx)
        } else {
            None
        };

        let separator_idx = if !recent.is_empty() {
            let idx = rows.len();
            for (label, action) in recent {
                rows.push((label, action, None));
            }
            Some(idx)
        } else {
            None
        };

        let menu_h = row_h * rows.len() as f32;
        let menu_rect =
            egui::Rect::from_min_size(Pos2::new(menu_x, menu_y), Vec2::new(menu_w, menu_h));

        // Drive the popup palette off the active skin's `pledit.txt` so
        // it tracks whatever the user picked. Falls back to Winamp's
        // classic green-on-black when the skin didn't override pledit
        // (see `DEFAULT_PLEDIT_COLORS`).
        let theme = crate::wsz_ui::skin_theme::DialogTheme::from_skin(self.renderer.get_skin());

        ui.painter().rect_filled(menu_rect, 0.0, theme.bg);
        ui.painter()
            .rect_stroke(menu_rect, 0.0, egui::Stroke::new(1.0, theme.border));

        let mut picked: Option<MainWindowAction> = None;
        for (i, (label, action, checked)) in rows.iter().enumerate() {
            let row_rect = egui::Rect::from_min_size(
                Pos2::new(menu_x, menu_y + i as f32 * row_h),
                Vec2::new(menu_w, row_h),
            );
            let response = ui.interact(
                row_rect,
                egui::Id::new(("options_menu_row", i)),
                egui::Sense::click(),
            );
            if response.hovered() {
                ui.painter().rect_filled(row_rect, 0.0, theme.selection_bg);
            }
            // The leading checkmark column is reserved for togglable rows
            // (Some(bool)); informational/recent rows pass None and the
            // text starts a little further left so the section still reads.
            if let Some(c) = checked {
                let mark = if *c { "✓" } else { " " };
                ui.painter().text(
                    row_rect.left_center() + Vec2::new(6.0 * scale, 0.0),
                    egui::Align2::LEFT_CENTER,
                    mark,
                    egui::FontId::proportional(10.0 * scale),
                    theme.current,
                );
            }
            ui.painter().text(
                row_rect.left_center() + Vec2::new(20.0 * scale, 0.0),
                egui::Align2::LEFT_CENTER,
                label.as_str(),
                egui::FontId::proportional(10.0 * scale),
                theme.text,
            );
            if response.clicked() {
                picked = Some(action.clone());
            }
        }

        // Thin separator lines above each grouped section so the
        // toggle / device / recent groups read distinctly.
        for sep in [device_separator_idx, separator_idx].into_iter().flatten() {
            let y = menu_y + sep as f32 * row_h;
            ui.painter().line_segment(
                [
                    Pos2::new(menu_x + 4.0 * scale, y),
                    Pos2::new(menu_x + menu_w - 4.0 * scale, y),
                ],
                egui::Stroke::new(1.0, theme.border),
            );
        }

        // Click outside the menu closes it. We use primary_pressed (mouse
        // down) rather than primary_clicked (mouse up) so the release of
        // the clutterbar `O` click that OPENED the menu doesn't also
        // close it — that release lands on the `O` slot, which is outside
        // the menu rect, and a click-released check would treat it as a
        // dismiss the same frame the menu first paints.
        let click_outside = ui.ctx().input(|i| {
            i.pointer.primary_pressed()
                && i.pointer
                    .interact_pos()
                    .map(|p| !menu_rect.contains(p))
                    .unwrap_or(false)
        });
        if picked.is_some() || click_outside {
            self.options_menu_open = false;
        }
        picked
    }
}
