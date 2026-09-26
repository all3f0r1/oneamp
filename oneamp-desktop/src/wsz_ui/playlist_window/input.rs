use super::{
    DragKind, MINI_TRANSPORT, MINI_TRANSPORT_H, MINI_TRANSPORT_Y, MiniCommand, PARENTS, PL_WIDTH,
    PlaylistAction, PlaylistWindow, RESIZE_HANDLE_H, RESIZE_HANDLE_W, RESIZE_HANDLE_X,
    RESIZE_HANDLE_Y, RIGHT_W, ROW_H_SKIN, skin_rect, snap_height,
};
use egui::Pos2;
use oneamp_core::{AudioCommand, AudioEngine};

impl PlaylistWindow {
    /// Refresh the close button's press visual at the top of `show()` so
    /// the sprite matches this frame's pointer state instead of last
    /// frame's.
    pub(super) fn update_button_press_visuals(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let pressed = ui.ctx().input(|i| i.pointer.primary_down());
        let close_rect = skin_rect(&self.renderer, offset, PL_WIDTH - 11, 3, 9, 9);
        self.close_pressed = pressed
            && ui
                .ctx()
                .pointer_latest_pos()
                .is_some_and(|p| close_rect.contains(p));
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

        // Parent buttons + their popup (highest priority — the popup
        // floats over the rest of the playlist body).
        let released = !is_pressed && self.mouse_was_pressed;
        if let Some(act) = self.submenu_input(mouse_pos, offset, click_just_started, released) {
            self.mouse_was_pressed = is_pressed;
            return act;
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
                    // Play/Pause/Open go through the app so they share the
                    // main window's playback-state semantics.
                    let send = |cmd| {
                        if let Some(engine) = audio_engine {
                            let _ = engine.send_command(cmd);
                        }
                    };
                    match mt.cmd {
                        MiniCommand::Previous => send(AudioCommand::Previous),
                        MiniCommand::Play => action = PlaylistAction::TransportPlay,
                        MiniCommand::Pause => action = PlaylistAction::TransportPause,
                        MiniCommand::Stop => send(AudioCommand::Stop),
                        MiniCommand::Next => send(AudioCommand::Next),
                        MiniCommand::Open => action = PlaylistAction::OpenFile,
                    }
                    self.mouse_was_pressed = is_pressed;
                    return action;
                }
            }
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
                self.height_skin = snap_height(start_height_skin as i32 + dy_skin);
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

    /// Winamp popup menus: pressing a parent button unfolds its popup (the
    /// bottom row covers the button); releasing over a row fires it. The
    /// release that ends the opening press is ignored while still on the
    /// bottom row, so a plain click leaves the popup open for a second
    /// click, and press-drag-release works in one gesture. Returns
    /// `Some` when the event was consumed.
    fn submenu_input(
        &mut self,
        pos: Pos2,
        offset: Pos2,
        pressed_now: bool,
        released: bool,
    ) -> Option<PlaylistAction> {
        if pressed_now {
            if self.submenu_row_at(pos, offset).is_some() {
                self.submenu_armed = true;
                return Some(PlaylistAction::None);
            }
            let by = self.buttons_y();
            if let Some(idx) = PARENTS
                .iter()
                .position(|p| skin_rect(&self.renderer, offset, p.skin_x, by, 22, 18).contains(pos))
            {
                self.open_submenu = Some(idx);
                self.submenu_armed = false;
                return Some(PlaylistAction::None);
            }
            self.open_submenu = None;
            return None;
        }
        let idx = self.open_submenu.filter(|_| released)?;
        let bottom = PARENTS[idx].actions.len() - 1;
        match self.submenu_row_at(pos, offset) {
            Some(row) if row == bottom && !self.submenu_armed => {
                self.submenu_armed = true;
                Some(PlaylistAction::None)
            }
            Some(row) => {
                self.open_submenu = None;
                Some(
                    PARENTS[idx].actions[row]
                        .clone()
                        .unwrap_or(PlaylistAction::None),
                )
            }
            None => {
                self.open_submenu = None;
                None
            }
        }
    }
}
