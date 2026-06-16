use super::{
    AUTO_BUTTON, EQ_BUTTON, EQ_GAIN_MAX_DB, EQ_GAIN_MIN_DB, EqualizerAction, EqualizerWindow,
    PRESETS_BUTTON, SHADE_RAIL, SLIDER_TOP_Y, ShadeDrag, SkinRect, TRACK_HEIGHT, TRACK_X,
};
use egui::{Pos2, Rect};
use oneamp_core::eqf;
use oneamp_core::{AudioCommand, AudioEngine};

impl EqualizerWindow {
    /// Handle pointer interaction with the shade-mode mini volume/balance
    /// rails. Starts a drag on click-inside, continues every frame until
    /// release, and dispatches `SetVolume` / `SetBalance` on each move so
    /// the engine and the strip sprite stay in sync.
    ///
    /// Returns `None` for now — the close button is handled by the caller
    /// because it shares the strip-wide click response.
    pub(super) fn handle_shade_input(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        audio_engine: Option<&AudioEngine>,
    ) -> Option<EqualizerAction> {
        let mouse_pos = ui.ctx().pointer_latest_pos()?;
        let is_pressed = ui.ctx().input(|i| i.pointer.primary_down());

        if !is_pressed {
            self.shade_drag = None;
            return None;
        }

        let vol_rect = SkinRect {
            x: SHADE_RAIL.vol_x,
            y: SHADE_RAIL.y,
            w: SHADE_RAIL.vol_w,
            h: 7,
        }
        .screen_rect(&self.renderer, offset);
        let bal_rect = SkinRect {
            x: SHADE_RAIL.bal_x,
            y: SHADE_RAIL.y,
            w: SHADE_RAIL.bal_w,
            h: 7,
        }
        .screen_rect(&self.renderer, offset);

        // Pick a drag target on click-just-started. Once latched, stay on
        // that rail even if the pointer drifts off — matches the rest of
        // the codebase's slider behaviour.
        if self.shade_drag.is_none() {
            if vol_rect.contains(mouse_pos) {
                self.shade_drag = Some(ShadeDrag::Volume);
            } else if bal_rect.contains(mouse_pos) {
                self.shade_drag = Some(ShadeDrag::Balance);
            }
        }

        match self.shade_drag {
            Some(ShadeDrag::Volume) => {
                let normalized =
                    ((mouse_pos.x - vol_rect.min.x) / vol_rect.width()).clamp(0.0, 1.0);
                self.volume_value = normalized;
                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::SetVolume(normalized));
                }
            }
            Some(ShadeDrag::Balance) => {
                let normalized =
                    ((mouse_pos.x - bal_rect.min.x) / bal_rect.width()).clamp(0.0, 1.0);
                let balance = normalized * 2.0 - 1.0;
                self.balance_value = balance;
                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::SetBalance(balance));
                }
            }
            None => {}
        }

        None
    }

    /// Refresh the visual press state of the EQ/AUTO/PRESETS overlays.
    /// Runs at the top of `show()` so the rendered sprite matches the
    /// current pointer state in the SAME frame; without this, the press
    /// feedback would lag by one repaint.
    ///
    /// PRESETS gets a special case: while the dropdown is open, keep the
    /// button visually pressed regardless of where the pointer is — this
    /// matches Winamp, where the PRESETS button stays "down" for the
    /// entire lifetime of its menu.
    pub(super) fn update_button_press_visuals(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let menu_open_pressed = if self.presets_menu_open {
            Some(super::EqButton::Presets)
        } else {
            None
        };
        let Some(mouse_pos) = ui.ctx().pointer_latest_pos() else {
            self.pressed_button = menu_open_pressed;
            return;
        };
        let is_pressed = ui.ctx().input(|i| i.pointer.primary_down());
        if !is_pressed {
            self.pressed_button = menu_open_pressed;
            return;
        }
        let eq_rect = EQ_BUTTON.screen_rect(&self.renderer, offset);
        let auto_rect = AUTO_BUTTON.screen_rect(&self.renderer, offset);
        let presets_rect = PRESETS_BUTTON.screen_rect(&self.renderer, offset);
        self.pressed_button = if eq_rect.contains(mouse_pos) {
            Some(super::EqButton::Eq)
        } else if auto_rect.contains(mouse_pos) {
            Some(super::EqButton::Auto)
        } else if presets_rect.contains(mouse_pos) {
            Some(super::EqButton::Presets)
        } else {
            menu_open_pressed
        };
    }

    pub(super) fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        audio_engine: Option<&AudioEngine>,
    ) -> Option<EqualizerAction> {
        let mut action = None;

        let Some(mouse_pos) = ui.ctx().pointer_latest_pos() else {
            self.dragging = None;
            self.pressed_button = None;
            self.mouse_was_pressed = false;
            return None;
        };

        let is_pressed = ui.ctx().input(|i| i.pointer.primary_down());
        let click_just_started = is_pressed && !self.mouse_was_pressed;
        let click_released = !is_pressed && self.mouse_was_pressed;

        // The EQ window is docked inside the main viewport — it shares the
        // OS window with the main player, so no per-window drag handler.
        // Dragging the title strip would just no-op or fight the main
        // window's drag handler.

        // Close button — Winamp paints it inside the title strip itself
        // (already in the bg). Hot area: top-left close icon (top-right area
        // of the title strip ≈ x=264..273, y=3..12 in skin space — reusing
        // the same offset Winamp uses for the main window).
        let close_rect = SkinRect {
            x: 264,
            y: 3,
            w: 9,
            h: 9,
        }
        .screen_rect(&self.renderer, offset);
        if click_just_started && close_rect.contains(mouse_pos) {
            action = Some(EqualizerAction::Close);
            self.mouse_was_pressed = is_pressed;
            return action;
        }

        // EQ / AUTO / PRESETS hot areas. Visual press state has already
        // been refreshed by `update_button_press_visuals` at the top of
        // `show()`; we just need the rects here for the click logic
        // below.
        let eq_rect = EQ_BUTTON.screen_rect(&self.renderer, offset);
        let auto_rect = AUTO_BUTTON.screen_rect(&self.renderer, offset);
        let presets_rect = PRESETS_BUTTON.screen_rect(&self.renderer, offset);

        if click_just_started {
            if eq_rect.contains(mouse_pos) {
                self.enabled = !self.enabled;
                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::SetEqualizerEnabled(self.enabled));
                }
            } else if auto_rect.contains(mouse_pos) {
                self.auto = !self.auto;
            } else if presets_rect.contains(mouse_pos) {
                self.presets_menu_open = !self.presets_menu_open;
            } else if !self.presets_menu_open {
                for slider_idx in 0..TRACK_X.len() {
                    if self.slider_hit_rect(slider_idx, offset).contains(mouse_pos) {
                        self.dragging = Some(slider_idx);
                        self.apply_drag(slider_idx, mouse_pos, offset, audio_engine);
                        break;
                    }
                }
            }
        }

        if is_pressed {
            if let Some(idx) = self.dragging {
                self.apply_drag(idx, mouse_pos, offset, audio_engine);
            }
        } else if click_released {
            self.dragging = None;
        }

        self.mouse_was_pressed = is_pressed;
        action
    }

    fn slider_hit_rect(&self, slider_idx: usize, offset: Pos2) -> Rect {
        SkinRect {
            x: TRACK_X[slider_idx],
            y: SLIDER_TOP_Y,
            w: 14,
            h: TRACK_HEIGHT,
        }
        .screen_rect(&self.renderer, offset)
    }

    fn apply_drag(
        &mut self,
        slider_idx: usize,
        mouse_pos: Pos2,
        offset: Pos2,
        audio_engine: Option<&AudioEngine>,
    ) {
        let scale = self.renderer.get_scale();
        let track_top_screen =
            self.renderer
                .skin_to_screen(TRACK_X[slider_idx], SLIDER_TOP_Y, offset);
        let local_y = (mouse_pos.y - track_top_screen.y).clamp(0.0, TRACK_HEIGHT as f32 * scale);
        let normalized = local_y / (TRACK_HEIGHT as f32 * scale);
        let gain = EQ_GAIN_MAX_DB - normalized * (EQ_GAIN_MAX_DB - EQ_GAIN_MIN_DB);

        if slider_idx == 0 {
            self.preamp = gain.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
            if let Some(engine) = audio_engine {
                let _ = engine.send_command(AudioCommand::SetEqualizerPreamp(self.preamp));
            }
        } else {
            let band = slider_idx - 1;
            self.eq_bands[band] = gain.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
            // Manual band drag invalidates the "current preset" mark
            // — the curve no longer matches a known preset, so the
            // dropdown shouldn't claim it does. Preamp drags don't
            // clear the mark: preamp is an independent baseline.
            self.current_preset_name = None;
            if let Some(engine) = audio_engine {
                let _ =
                    engine.send_command(AudioCommand::SetEqualizerBand(band, self.eq_bands[band]));
            }
        }
    }

    /// File-dialog → parse `.eqf` → push bands + preamp through to the
    /// engine. Errors are swallowed silently for now — wiring an error
    /// surface for the EQ window would require a new `EqualizerAction`
    /// variant; deferred until the user actually reports a bad file.
    pub(super) fn handle_load_eqf(&mut self, audio_engine: Option<&AudioEngine>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Winamp EQ preset", &["eqf", "EQF"])
            .pick_file()
        else {
            return;
        };
        let Ok(file) = std::fs::File::open(&path) else {
            return;
        };
        let mut reader = std::io::BufReader::new(file);
        let preset = match eqf::read_eqf(&mut reader) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to load .eqf {}: {}", path.display(), e);
                return;
            }
        };

        for (slot, &g) in self.eq_bands.iter_mut().zip(preset.bands.iter()) {
            *slot = g.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
        }
        self.preamp = preset.preamp_db.clamp(EQ_GAIN_MIN_DB, EQ_GAIN_MAX_DB);
        // A user-loaded .eqf isn't one of our built-in presets, but
        // we still want the dropdown to display *something* — use
        // the .eqf's embedded preset name (or the filename stem when
        // the file doesn't carry one).
        let display_name = if preset.name.trim().is_empty() {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        } else {
            Some(preset.name.clone())
        };
        self.current_preset_name = display_name;

        if let Some(engine) = audio_engine {
            let _ = engine.send_command(AudioCommand::SetEqualizerBands(self.eq_bands.to_vec()));
            let _ = engine.send_command(AudioCommand::SetEqualizerPreamp(self.preamp));
        }
    }

    /// File-dialog → write `.eqf` with the current bands + preamp under a
    /// stock preset name. The user can rename the file from the save
    /// dialog itself.
    pub(super) fn handle_save_eqf(&self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Winamp EQ preset", &["eqf"])
            .set_file_name("OneAmp.eqf")
            .save_file()
        else {
            return;
        };
        let preset = eqf::EqfPreset {
            // The preset name slot is metadata Winamp surfaces in its own
            // EQ browser. Using the file stem keeps the in-archive name
            // matching the on-disk filename without forcing the user to
            // type it twice.
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("OneAmp")
                .to_string(),
            preamp_db: self.preamp,
            bands: self.eq_bands,
        };
        let Ok(file) = std::fs::File::create(&path) else {
            return;
        };
        let mut writer = std::io::BufWriter::new(file);
        if let Err(e) = eqf::write_eqf(&mut writer, &preset) {
            eprintln!("Failed to save .eqf {}: {}", path.display(), e);
        }
    }
}
