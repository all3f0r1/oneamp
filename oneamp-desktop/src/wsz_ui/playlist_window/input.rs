use super::{
    DragKind, MINI_TRANSPORT, MINI_TRANSPORT_H, MINI_TRANSPORT_Y, MiniCommand, PARENTS,
    PL_MAX_HEIGHT, PL_MIN_HEIGHT, PL_WIDTH, PlaylistAction, PlaylistWindow, RESIZE_HANDLE_H,
    RESIZE_HANDLE_W, RESIZE_HANDLE_X, RESIZE_HANDLE_Y, RIGHT_W, ROW_H_SKIN, skin_rect,
};
use egui::Pos2;
use oneamp_core::{AudioCommand, AudioEngine};

impl PlaylistWindow {
    /// Refresh the visual press state of the close button and the 5
    /// parent menu buttons (ADD/REM/SEL/MISC/LIST). Run at the top of
    /// `show()` so the rendered sprites match this frame's pointer
    /// state instead of last frame's — without it the press feedback
    /// only appears after the next repaint.
    pub(super) fn update_button_press_visuals(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let Some(mouse_pos) = ui.ctx().pointer_latest_pos() else {
            self.close_pressed = false;
            self.pressed_button = None;
            return;
        };
        let is_pressed = ui.ctx().input(|i| i.pointer.primary_down());
        if !is_pressed {
            self.close_pressed = false;
            self.pressed_button = None;
            return;
        }

        let close_rect = skin_rect(&self.renderer, offset, PL_WIDTH - 11, 3, 9, 9);
        self.close_pressed = close_rect.contains(mouse_pos);

        let mut hovered = None;
        for (idx, p) in PARENTS.iter().enumerate() {
            let rect = skin_rect(&self.renderer, offset, p.skin_x, 202, 22, 18);
            if rect.contains(mouse_pos) {
                hovered = Some(idx);
                break;
            }
        }
        self.pressed_button = hovered;
    }

    pub(super) fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        audio_engine: Option<&AudioEngine>,
        entry_count: usize,
    ) -> PlaylistAction {
        let mut action = PlaylistAction::None;

        let Some(mouse_pos) = ui.ctx().pointer_latest_pos() else {
            self.pressed_button = None;
            self.close_pressed = false;
            self.drag = None;
            self.mouse_was_pressed = false;
            return action;
        };

        let is_pressed = ui.ctx().input(|i| i.pointer.primary_down());
        let click_just_started = is_pressed && !self.mouse_was_pressed;

        // Continue or end any active drag first.
        if let Some(drag) = self.drag {
            if !is_pressed {
                self.drag = None;
            } else {
                self.continue_drag(drag, mouse_pos, offset, entry_count);
                self.mouse_was_pressed = is_pressed;
                return action;
            }
        }

        // Close button at top-right cornerpiece.
        let close_rect = skin_rect(&self.renderer, offset, PL_WIDTH - 11, 3, 9, 9);
        self.close_pressed = is_pressed && close_rect.contains(mouse_pos);
        if click_just_started && close_rect.contains(mouse_pos) {
            action = PlaylistAction::Close;
            self.mouse_was_pressed = is_pressed;
            return action;
        }

        // Submenu sub-button clicks (highest priority — submenu floats over
        // the rest of the playlist body).
        if click_just_started && let Some(act) = self.submenu_click(mouse_pos, offset) {
            action = act;
            self.open_submenu = None;
            self.mouse_was_pressed = is_pressed;
            return action;
        }

        // Resize handle. Skin rect is inside the bottom-right control bar.
        let resize_skin_y = self.body_bot() + (RESIZE_HANDLE_Y - 72);
        let resize_rect = skin_rect(
            &self.renderer,
            offset,
            RESIZE_HANDLE_X,
            resize_skin_y,
            RESIZE_HANDLE_W,
            RESIZE_HANDLE_H,
        );
        if click_just_started && resize_rect.contains(mouse_pos) {
            self.drag = Some(DragKind::Resize {
                start_pointer_y: mouse_pos.y,
                start_height_skin: self.height_skin,
            });
            self.mouse_was_pressed = is_pressed;
            return action;
        }

        // Scrollbar thumb hit area.
        if click_just_started
            && let Some(grab_offset) =
                self.scrollbar_thumb_grab_offset(mouse_pos, offset, entry_count)
        {
            self.drag = Some(DragKind::Scrollbar { grab_offset });
            self.mouse_was_pressed = is_pressed;
            return action;
        }

        // Mini-transport buttons.
        if click_just_started {
            for mt in &MINI_TRANSPORT {
                let mt_y = self.body_bot() + (MINI_TRANSPORT_Y - 72);
                let rect = skin_rect(
                    &self.renderer,
                    offset,
                    mt.skin_x,
                    mt_y,
                    mt.w,
                    MINI_TRANSPORT_H,
                );
                if rect.contains(mouse_pos) {
                    if let Some(engine) = audio_engine {
                        match mt.cmd {
                            MiniCommand::Previous => {
                                let _ = engine.send_command(AudioCommand::Previous);
                            }
                            MiniCommand::Play => {
                                let _ = engine.send_command(AudioCommand::Resume);
                            }
                            MiniCommand::Pause => {
                                let _ = engine.send_command(AudioCommand::Pause);
                            }
                            MiniCommand::Stop => {
                                let _ = engine.send_command(AudioCommand::Stop);
                            }
                            MiniCommand::Next => {
                                let _ = engine.send_command(AudioCommand::Next);
                            }
                            MiniCommand::Open => {
                                action = PlaylistAction::AddFiles;
                            }
                        }
                    } else if matches!(mt.cmd, MiniCommand::Open) {
                        action = PlaylistAction::AddFiles;
                    }
                    self.mouse_was_pressed = is_pressed;
                    return action;
                }
            }
        }

        // Parent buttons: visual press feedback + submenu toggle / fallback.
        let mut hovered = None;
        for (idx, p) in PARENTS.iter().enumerate() {
            let rect = skin_rect(&self.renderer, offset, p.skin_x, 202, 22, 18);
            if rect.contains(mouse_pos) {
                hovered = Some(idx);
                if click_just_started {
                    if p.submenu_atlas_x.is_some() {
                        // Toggle: if same submenu open, close it; else open new.
                        self.open_submenu = if self.open_submenu == Some(idx) {
                            None
                        } else {
                            Some(idx)
                        };
                    } else if let Some(fallback) = p.fallback.clone() {
                        action = fallback;
                        self.open_submenu = None;
                    } else {
                        // SEL/MISC parents: no action wired yet, just close
                        // any open submenu so the click feels responsive.
                        self.open_submenu = None;
                    }
                }
                break;
            }
        }
        self.pressed_button = if is_pressed { hovered } else { None };

        // Click outside any submenu/parent closes the open submenu.
        if click_just_started
            && hovered.is_none()
            && self.open_submenu.is_some()
            && self.submenu_click(mouse_pos, offset).is_none()
        {
            self.open_submenu = None;
        }

        self.mouse_was_pressed = is_pressed;
        action
    }

    fn continue_drag(&mut self, drag: DragKind, mouse_pos: Pos2, offset: Pos2, entry_count: usize) {
        match drag {
            DragKind::Scrollbar { grab_offset } => {
                let scale = self.renderer.get_scale();
                let body_top_screen = self.renderer.skin_to_screen(0, self.body_top(), offset).y;
                let body_h_screen = (self.body_bot() - self.body_top()) as f32 * scale;
                let thumb_h_screen = 18.0 * scale;
                let travel = (body_h_screen - thumb_h_screen).max(1.0);
                let thumb_top =
                    (mouse_pos.y - grab_offset).clamp(body_top_screen, body_top_screen + travel);
                let normalized = (thumb_top - body_top_screen) / travel;
                let row_h_screen = ROW_H_SKIN as f32 * scale;
                let total_h_screen = entry_count as f32 * row_h_screen;
                let max_offset = (total_h_screen - body_h_screen).max(0.0);
                self.scroll_offset = (normalized * max_offset).clamp(0.0, max_offset);
            }
            DragKind::Resize {
                start_pointer_y,
                start_height_skin,
            } => {
                let scale = self.renderer.get_scale();
                let dy_screen = mouse_pos.y - start_pointer_y;
                let dy_skin = (dy_screen / scale).round() as i32;
                let new_h = (start_height_skin as i32 + dy_skin)
                    .clamp(PL_MIN_HEIGHT as i32, PL_MAX_HEIGHT as i32)
                    as u32;
                self.height_skin = new_h;
            }
        }
    }

    /// Returns the screen-space pointer offset above the thumb's top edge
    /// when the click hits the thumb, else `None`. Used to start a drag
    /// with the existing grab position so the thumb doesn't snap.
    fn scrollbar_thumb_grab_offset(
        &self,
        mouse_pos: Pos2,
        offset: Pos2,
        entry_count: usize,
    ) -> Option<f32> {
        let scale = self.renderer.get_scale();
        let body_top = self.body_top();
        let body_bot = self.body_bot();
        let groove_h_skin = body_bot - body_top;
        let thumb_h_skin = 18u32;
        if groove_h_skin <= thumb_h_skin {
            return None;
        }
        let row_h_screen = ROW_H_SKIN as f32 * scale;
        let list_h_screen = (body_bot - body_top) as f32 * scale;
        let total_h_screen = entry_count as f32 * row_h_screen;
        let max_offset = (total_h_screen - list_h_screen).max(0.0);
        let normalized = if max_offset > 0.0 {
            self.scroll_offset / max_offset
        } else {
            0.0
        };
        let travel = groove_h_skin - thumb_h_skin;
        let thumb_y_skin = body_top + (normalized * travel as f32) as u32;
        let thumb_rect = skin_rect(
            &self.renderer,
            offset,
            PL_WIDTH - RIGHT_W + 5,
            thumb_y_skin,
            8,
            thumb_h_skin,
        );
        if thumb_rect.contains(mouse_pos) {
            Some(mouse_pos.y - thumb_rect.min.y)
        } else {
            None
        }
    }

    /// Hit-test against the currently open submenu's sub-buttons. Returns
    /// the action to dispatch when a button is hit. Rows whose action is
    /// `None` are rendered (Winamp parity) but consume the click silently.
    /// Hot-area dst_x mirrors the `-3 px` shift applied by `render_submenu`
    /// so the click target lines up with the visible sprite.
    fn submenu_click(&self, mouse_pos: Pos2, offset: Pos2) -> Option<PlaylistAction> {
        let idx = self.open_submenu?;
        let parent = &PARENTS[idx];
        parent.submenu_atlas_x?;
        let parent_y: u32 = 202;
        let sub_h: u32 = 18;
        let dst_x = parent.skin_x.saturating_sub(3);
        for (row, action) in parent.submenu_actions.iter().enumerate() {
            let dst_y = parent_y - (3 - row as u32) * sub_h;
            let rect = skin_rect(&self.renderer, offset, dst_x, dst_y, 22, sub_h);
            if rect.contains(mouse_pos) {
                // Unwired rows still swallow the click so it doesn't fall
                // through to a parent button below — `PlaylistAction::None`
                // is a no-op upstream and closes the submenu via the
                // caller's standard dismiss path.
                return Some(action.clone().unwrap_or(PlaylistAction::None));
            }
        }
        None
    }
}
