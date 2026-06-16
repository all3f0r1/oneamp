use super::{MainWindowAction, WszMainWindow, hit_rect};
use crate::wsz_ui::components::{TitlebarAction, WinampButton};
use egui::{Context, Vec2};
use oneamp_core::{AudioCommand, AudioEngine};

/// Map a 0..1 horizontal position across the spectrum strip to the
/// closest of the 10 EQ band indices. The strip is shown on a
/// log-frequency scale spanning ≈21 Hz → ≈21 kHz (FFT bin range with
/// the 0.95 × Nyquist cap); we mirror that here so a click halfway
/// across the strip lands on the band sitting at the geometric mean
/// of the range.
fn nearest_eq_band(frac: f32) -> usize {
    // Same band centres the engine uses; duplicated to avoid pulling
    // the whole oneamp_core::equalizer_presets module just for the
    // constant. Keep this in sync with EQ_FREQUENCIES.
    const EQ_HZ: [f32; 10] = [
        31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
    ];
    let f_lo = 21.0_f32.ln();
    let f_hi = 21_000.0_f32.ln();
    let target_ln = f_lo + frac.clamp(0.0, 1.0) * (f_hi - f_lo);
    let mut best = 0usize;
    let mut best_dist = f32::INFINITY;
    for (i, &hz) in EQ_HZ.iter().enumerate() {
        let d = (hz.ln() - target_ln).abs();
        if d < best_dist {
            best_dist = d;
            best = i;
        }
    }
    best
}

impl WszMainWindow {
    /// Handle window dragging via the title bar strip of the skin. Only
    /// active when the app draws its own chrome — otherwise the OS title bar
    /// already moves the window and a second drag source fights it.
    ///
    /// `StartDrag` MUST be sent exactly once (on `drag_started`). Sending it
    /// every dragged frame floods the WM with move requests and locks the
    /// pointer in an interactive-move loop the user cannot escape.
    ///
    /// Returns `Some(action)` when a non-drag interaction (double-click)
    /// requires the app loop to do something.
    pub(super) fn handle_window_drag(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        offset: egui::Pos2,
    ) -> Option<MainWindowAction> {
        if !self.custom_chrome {
            return None;
        }

        let scale = self.renderer.get_scale();
        // Winamp title bar is the top 14 px of the 275x116 skin.
        let drag_rect = egui::Rect::from_min_size(offset, Vec2::new(275.0 * scale, 14.0 * scale));

        let response = ui.interact(
            drag_rect,
            egui::Id::new("window_drag_area"),
            egui::Sense::click_and_drag(),
        );

        if response.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        if response.double_clicked() {
            return Some(MainWindowAction::ToggleShade);
        }
        None
    }

    pub(super) fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        offset: egui::Pos2,
        audio_engine: Option<&AudioEngine>,
    ) -> Option<MainWindowAction> {
        let scale = self.renderer.get_scale();
        let mut action = None;

        if let Some(mouse_pos) = ui.ctx().pointer_latest_pos() {
            let is_pressed = ui.ctx().input(|i| i.pointer.primary_down());
            let click_just_started = is_pressed && !self.mouse_pressed;
            let mut titlebar_consumed_click = false;

            // Update clutterbar visual state every frame; consumes the click
            // when the pointer is over the strip so it doesn't fall through
            // to slider drag-start logic below.
            let clutter = self.clutterbar.handle_input(
                mouse_pos,
                offset,
                scale,
                is_pressed,
                click_just_started,
            );
            match clutter.clicked {
                Some(crate::wsz_ui::components::clutterbar::ClutterLetter::O) => {
                    // The post-1.0 menu lives in the top-left titlebar
                    // logo. Keep the clutterbar `O` working as a familiar
                    // alias so long-time Winamp users (who muscle-memory'd
                    // the strip) still land on something useful — it just
                    // opens the hierarchical menu now instead of the
                    // legacy flat popup.
                    self.toggle_main_menu();
                }
                Some(crate::wsz_ui::components::clutterbar::ClutterLetter::D) => {
                    // `D` = Winamp double-size. The app toggles the 1×/2×
                    // user-scale override.
                    action = Some(MainWindowAction::ToggleDoubleSize);
                }
                // A (always-on-top), I (ID3 editor) and V (vis plugin)
                // stay cosmetic until they earn dedicated actions.
                _ => {}
            }

            if self.custom_chrome
                && let Some(tb_action) = self.titlebar.check_click(
                    ui,
                    mouse_pos,
                    offset,
                    scale,
                    is_pressed,
                    click_just_started,
                )
            {
                titlebar_consumed_click = true;
                action = self.handle_titlebar_action(tb_action, ctx);
            }

            if click_just_started && !titlebar_consumed_click && !clutter.inside {
                if hit_rect(mouse_pos, offset, scale, 24, 43, 76, 16) {
                    // Shift+click on the spectrum opens the EQ pinned
                    // on the band closest to the clicked frequency.
                    // Plain click keeps Winamp's classic cycle.
                    let shift_held = ctx.input(|i| i.modifiers.shift);
                    if shift_held {
                        let zone_x = 24.0 * scale + offset.x;
                        let local_x = (mouse_pos.x - zone_x).clamp(0.0, 76.0 * scale);
                        let frac = local_x / (76.0 * scale);
                        let band = nearest_eq_band(frac);
                        action = Some(MainWindowAction::OpenEqualizerAtBand(band));
                    } else {
                        self.visualizer_mode = self.visualizer_mode.next();
                    }
                } else if hit_rect(mouse_pos, offset, scale, 48, 26, 51, 13) {
                    self.display.show_remaining = !self.display.show_remaining;
                } else if hit_rect(mouse_pos, offset, scale, 253, 91, 13, 15) {
                    action = Some(MainWindowAction::ShowAbout);
                } else if let Some(button) = self.buttons.check_click(mouse_pos, offset, scale) {
                    action = self.handle_button_click(button, audio_engine);
                }

                if self.position_slider.is_hovered(mouse_pos, offset, scale) {
                    self.position_slider.is_dragging = true;
                }

                if self.volume_slider.is_hovered(mouse_pos, offset, scale) {
                    self.volume_slider.is_dragging = true;
                }

                if self.balance_slider.is_hovered(mouse_pos, offset, scale) {
                    self.balance_slider.is_dragging = true;
                }
            }

            if !is_pressed {
                // end_drag() captures the final position into pending_seek so
                // we can fire a single Seek command on release rather than
                // flooding the audio thread every frame.
                self.position_slider.end_drag();
                self.volume_slider.is_dragging = false;
                self.balance_slider.is_dragging = false;
            }

            // Update visual progress every frame while dragging — no seek yet.
            self.position_slider.handle_drag(mouse_pos, offset, scale);

            if let Some(progress) = self.position_slider.take_pending_seek()
                && let Some(engine) = audio_engine
                && self.display.total_time > 0.0
            {
                let seek_time = progress * self.display.total_time;
                let _ = engine.send_command(AudioCommand::Seek(seek_time));
            }

            if let Some(volume) = self.volume_slider.handle_drag(mouse_pos, offset, scale)
                && let Some(engine) = audio_engine
            {
                let _ = engine.send_command(AudioCommand::SetVolume(volume));
            }

            if let Some(balance) = self.balance_slider.handle_drag(mouse_pos, offset, scale)
                && let Some(engine) = audio_engine
            {
                let _ = engine.send_command(AudioCommand::SetBalance(balance));
            }

            // Mouse-wheel seek when the pointer is over the position
            // bar — 5 s per notch (matches the ←/→ hotkey step), one
            // shot per scroll event. Scroll up = forward in time.
            // Gated on the position slider being hovered so wheeling
            // over the rest of the player still routes to the right
            // widget (volume / balance / etc.) without surprise seeks.
            if self.position_slider.is_hovered(mouse_pos, offset, scale) {
                let scroll_y = ctx.input(|i| i.raw_scroll_delta.y);
                if let Some(new_pos) = self.try_wheel_seek(scroll_y, 5.0)
                    && let Some(engine) = audio_engine
                {
                    let _ = engine.send_command(AudioCommand::Seek(new_pos));
                }
            }

            self.buttons
                .update_all(Some(mouse_pos), is_pressed, offset, scale);
            self.mouse_pressed = is_pressed;
        } else {
            self.buttons.update_all(None, false, offset, scale);
        }

        action
    }

    fn handle_button_click(
        &mut self,
        button: WinampButton,
        audio_engine: Option<&AudioEngine>,
    ) -> Option<MainWindowAction> {
        match button {
            WinampButton::Play => {
                if self.is_paused {
                    if let Some(engine) = audio_engine {
                        let _ = engine.send_command(AudioCommand::Resume);
                    }
                    None
                } else if !self.is_playing {
                    // Stopped: ask the app to start the current playlist track
                    Some(MainWindowAction::PlayCurrent)
                } else {
                    None
                }
            }
            WinampButton::Pause => {
                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::Pause);
                }
                None
            }
            WinampButton::Stop => {
                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::Stop);
                }
                None
            }
            WinampButton::Next => {
                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::Next);
                }
                None
            }
            WinampButton::Previous => {
                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::Previous);
                }
                None
            }
            WinampButton::Eject => Some(MainWindowAction::OpenFile),
            WinampButton::Shuffle => Some(MainWindowAction::ToggleShuffle),
            WinampButton::Repeat => Some(MainWindowAction::CycleRepeat),
            WinampButton::EqToggle => Some(MainWindowAction::ToggleEqualizer),
            WinampButton::PlToggle => Some(MainWindowAction::TogglePlaylist),
        }
    }

    fn handle_titlebar_action(
        &mut self,
        action: TitlebarAction,
        ctx: &Context,
    ) -> Option<MainWindowAction> {
        match action {
            TitlebarAction::OpenMenu => {
                // Top-left logo opens (or closes) the hierarchical
                // titlebar menu. Toggling is enough — `MainMenu::render`
                // handles click-outside dismissal on its own. Returns
                // None because the menu's own picks come through as a
                // separate action via `render_main_menu` later in the
                // paint pipeline.
                self.toggle_main_menu();
                None
            }
            TitlebarAction::Minimize => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                None
            }
            TitlebarAction::ToggleShade => Some(MainWindowAction::ToggleShade),
            TitlebarAction::Close => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                None
            }
        }
    }
}
