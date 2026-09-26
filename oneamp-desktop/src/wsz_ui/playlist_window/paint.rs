use super::{
    BOTTOM_H, DragKind, LEFT_W, PARENTS, PL_WIDTH, PlaylistAction, PlaylistWindow, RIGHT_W,
    ROW_H_SKIN, SUB_ATLAS_Y, SUB_H, TITLE_H, pledit_font_id,
};
use crate::wsz_ui::components::bitmap_font;
use egui::{Pos2, Rect, Sense, Vec2};
use oneamp_core::PlaylistEntry;
use oneamp_core::wsz::skin::SkinComponent;

impl PlaylistWindow {
    /// Render the pledit.bmp skin frame (title bar + side fillers + bottom
    /// control bars). Each piece is extracted from the atlas and stamped at
    /// its destination position; tile-able pieces are stamped repeatedly.
    pub(super) fn render_frame(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Pledit)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // ---- Title bar (y=0..20 active, y=21..41 inactive) -------------
        // Active strip carries the bright "WINAMP PLAYLIST" text; inactive
        // dims it down. The coordinator drives `self.focused`, defaulting
        // to false so the playlist matches Winamp's "main player owns the
        // focus" baseline.
        let title_src_y: u32 = if self.focused { 0 } else { 21 };
        let title_tag: &str = if self.focused { "active" } else { "inactive" };
        if let Some(piece) = atlas.extract_region(0, title_src_y, 25, TITLE_H) {
            let pos = self.renderer.skin_to_screen(0, 0, offset);
            self.renderer
                .render_region(ui, &piece, pos, &format!("pl_tl_left_{title_tag}"));
        }
        if let Some(piece) = atlas.extract_region(153, title_src_y, 25, TITLE_H) {
            let pos = self.renderer.skin_to_screen(PL_WIDTH - 25, 0, offset);
            self.renderer
                .render_region(ui, &piece, pos, &format!("pl_tl_right_{title_tag}"));
        }
        let title_strip_x = (PL_WIDTH - 100) / 2;
        if let Some(piece) = atlas.extract_region(26, title_src_y, 100, TITLE_H) {
            let pos = self.renderer.skin_to_screen(title_strip_x, 0, offset);
            self.renderer
                .render_region(ui, &piece, pos, &format!("pl_tl_center_{title_tag}"));
        }
        self.tile_horizontal(
            ui,
            &atlas,
            offset,
            &format!("pl_tl_fill_l_{title_tag}"),
            127,
            title_src_y,
            25,
            TITLE_H,
            25,
            title_strip_x,
            0,
        );
        self.tile_horizontal(
            ui,
            &atlas,
            offset,
            &format!("pl_tl_fill_r_{title_tag}"),
            127,
            title_src_y,
            25,
            TITLE_H,
            title_strip_x + 100,
            PL_WIDTH - 25,
            0,
        );

        // ---- Side fillers (vertical tile between title and bottom) ------
        let body_top = self.body_top();
        let body_bot = self.body_bot();
        self.tile_vertical(
            ui,
            &atlas,
            offset,
            "pl_left_fill",
            0,
            42,
            LEFT_W,
            29,
            0,
            body_top,
            body_bot,
        );
        let right_x = PL_WIDTH - RIGHT_W;
        self.tile_vertical(
            ui,
            &atlas,
            offset,
            "pl_right_left_bar",
            31,
            42,
            5,
            29,
            right_x,
            body_top,
            body_bot,
        );
        self.tile_vertical(
            ui,
            &atlas,
            offset,
            "pl_right_groove",
            36,
            42,
            8,
            29,
            right_x + 5,
            body_top,
            body_bot,
        );
        self.tile_vertical(
            ui,
            &atlas,
            offset,
            "pl_right_right_bar",
            44,
            42,
            7,
            29,
            right_x + 13,
            body_top,
            body_bot,
        );

        // ---- Bottom control bars ----------------------------------------
        if let Some(bar) = atlas.extract_region(0, 72, 125, BOTTOM_H) {
            let pos = self.renderer.skin_to_screen(0, body_bot, offset);
            self.renderer.render_region(ui, &bar, pos, "pl_bottom_left");
        }
        if let Some(bar) = atlas.extract_region(126, 72, 150, BOTTOM_H) {
            let pos = self.renderer.skin_to_screen(125, body_bot, offset);
            self.renderer
                .render_region(ui, &bar, pos, "pl_bottom_right");
        }

        // ---- Pressed close button overlay -------------------------------
        if self.close_pressed
            && let Some(reg) = atlas.extract_region(52, 42, 9, 9)
        {
            let pos = self.renderer.skin_to_screen(PL_WIDTH - 11, 3, offset);
            self.renderer
                .render_region(ui, &reg, pos, "pl_close_pressed");
        }
    }

    /// Stamp `region` repeatedly along the X axis from `x0` to `x1`. The
    /// last tile is clipped horizontally if it would overshoot.
    #[allow(clippy::too_many_arguments)]
    fn tile_horizontal(
        &mut self,
        ui: &mut egui::Ui,
        atlas: &oneamp_core::wsz::bitmap::BitmapAtlas,
        offset: Pos2,
        key_prefix: &str,
        atlas_x: u32,
        atlas_y: u32,
        tile_w: u32,
        tile_h: u32,
        x0: u32,
        x1: u32,
        dst_y: u32,
    ) {
        if x1 <= x0 {
            return;
        }
        let mut x = x0;
        let mut i = 0;
        while x < x1 {
            let remaining = x1 - x;
            let w = tile_w.min(remaining);
            if let Some(region) = atlas.extract_region(atlas_x, atlas_y, w, tile_h) {
                let pos = self.renderer.skin_to_screen(x, dst_y, offset);
                self.renderer
                    .render_region(ui, &region, pos, &format!("{}_{}", key_prefix, i));
            }
            x += w;
            i += 1;
        }
    }

    /// Stamp `region` repeatedly along the Y axis from `y0` to `y1`. The
    /// last tile is clipped vertically if it would overshoot — caches a
    /// dedicated key so different heights don't collide in the texture cache.
    #[allow(clippy::too_many_arguments)]
    fn tile_vertical(
        &mut self,
        ui: &mut egui::Ui,
        atlas: &oneamp_core::wsz::bitmap::BitmapAtlas,
        offset: Pos2,
        key_prefix: &str,
        atlas_x: u32,
        atlas_y: u32,
        tile_w: u32,
        tile_h: u32,
        dst_x: u32,
        y0: u32,
        y1: u32,
    ) {
        if y1 <= y0 {
            return;
        }
        let mut y = y0;
        let mut i = 0;
        while y < y1 {
            let remaining = y1 - y;
            let h = tile_h.min(remaining);
            if let Some(region) = atlas.extract_region(atlas_x, atlas_y, tile_w, h) {
                let pos = self.renderer.skin_to_screen(dst_x, y, offset);
                self.renderer.render_region(
                    ui,
                    &region,
                    pos,
                    &format!("{}_{}_{}", key_prefix, i, h),
                );
            }
            y += h;
            i += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_rows(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        entries: &[PlaylistEntry],
        current_index: Option<usize>,
        selected: &std::collections::BTreeSet<usize>,
        queued: &[Option<usize>],
        display_format: &str,
    ) -> PlaylistAction {
        let scale = self.renderer.get_scale();
        let list_x = LEFT_W;
        let list_y = TITLE_H;
        let list_w = PL_WIDTH - LEFT_W - RIGHT_W;
        let list_h = self.height_skin - TITLE_H - BOTTOM_H;
        let row_h_screen = ROW_H_SKIN as f32 * scale;

        let list_rect = Rect::from_min_size(
            self.renderer.skin_to_screen(list_x, list_y, offset),
            Vec2::new(list_w as f32 * scale, list_h as f32 * scale),
        );

        let normal_color = self.pledit_color(|c| c.normal);
        let current_color = self.pledit_color(|c| c.current);
        let normal_bg = self.pledit_color(|c| c.normal_bg);
        let selected_bg = self.pledit_color(|c| c.selected_bg);

        ui.painter().rect_filled(list_rect, 0.0, normal_bg);

        // Clamp scroll_offset against current content size every frame —
        // otherwise removing rows or resizing the window down can leave us
        // scrolled past the end with a sudden empty view.
        let total_h = entries.len() as f32 * row_h_screen;
        let max_offset = (total_h - list_rect.height()).max(0.0);
        if let Some(idx) = self.ensure_visible.take() {
            let top = idx as f32 * row_h_screen;
            let bottom = top + row_h_screen;
            if top < self.scroll_offset {
                self.scroll_offset = top;
            } else if bottom > self.scroll_offset + list_rect.height() {
                self.scroll_offset = bottom - list_rect.height();
            }
        }
        self.scroll_offset = self.scroll_offset.clamp(0.0, max_offset);

        let visible_rows = (list_rect.height() / row_h_screen).ceil() as usize + 1;
        let start_index = (self.scroll_offset / row_h_screen) as usize;
        let end_index = (start_index + visible_rows).min(entries.len());

        let mut action = PlaylistAction::None;
        // Rows under the open popup must not react to its clicks.
        let over_menu = ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|p| self.submenu_row_at(p, offset).is_some());

        for (i, entry) in entries[start_index..end_index].iter().enumerate() {
            let actual_idx = start_index + i;
            let row_top =
                list_rect.min.y + i as f32 * row_h_screen - (self.scroll_offset % row_h_screen);
            if row_top + row_h_screen < list_rect.min.y || row_top > list_rect.max.y {
                continue;
            }
            let row_rect = Rect::from_min_size(
                Pos2::new(list_rect.min.x, row_top),
                Vec2::new(list_rect.width(), row_h_screen),
            );

            let is_current = Some(actual_idx) == current_index;
            let is_selected = selected.contains(&actual_idx);
            if is_selected {
                ui.painter().rect_filled(row_rect, 0.0, selected_bg);
            }

            // Queue badge: Winamp shows a bracketed play-order number on
            // entries that are queued to play next. `queued[idx]` is the
            // 1-based queue position, or None.
            let queue_badge = queued
                .get(actual_idx)
                .copied()
                .flatten()
                .map(|pos| format!("[{}] ", pos))
                .unwrap_or_default();
            let title_text = format!(
                "{}{}. {}",
                queue_badge,
                actual_idx + 1,
                entry.format_display(display_format)
            );
            let duration_text = entry.duration.map(|d| {
                let mins = (d / 60.0) as u32;
                let secs = (d % 60.0) as u32;
                format!("{}:{:02}", mins, secs)
            });
            let row_color = if is_current {
                current_color
            } else {
                normal_color
            };
            let row_color = if entry.unavailable {
                row_color.gamma_multiply(0.45)
            } else {
                row_color
            };
            let row_font = pledit_font_id(self.renderer.get_skin(), 9.0 * scale);
            let row_left = row_rect.min + Vec2::new(4.0 * scale, 1.0 * scale);
            let row_right = Pos2::new(row_rect.max.x - 4.0 * scale, row_rect.min.y + 1.0 * scale);

            // Right-align the duration first so we know exactly how much
            // horizontal room is left for the title. Winamp puts the time
            // flush against the right edge and truncates the title with `…`
            // in front of it; without the right-align, long titles push the
            // duration off-screen and the row looks unfinished.
            let dur_width = match &duration_text {
                Some(s) => {
                    let r = ui.painter().text(
                        row_right,
                        egui::Align2::RIGHT_TOP,
                        s,
                        row_font.clone(),
                        row_color,
                    );
                    r.width() + 6.0 * scale // gap between title and duration
                }
                None => 0.0,
            };

            let title_max_w = (row_right.x - row_left.x) - dur_width;
            if title_max_w > 0.0 {
                let mut job = egui::text::LayoutJob::single_section(
                    title_text,
                    egui::TextFormat::simple(row_font, row_color),
                );
                job.wrap = egui::text::TextWrapping {
                    max_width: title_max_w,
                    max_rows: 1,
                    break_anywhere: true,
                    overflow_character: Some('…'),
                };
                let galley = ui.fonts(|f| f.layout_job(job));
                ui.painter().galley(row_left, galley, row_color);
            }

            let sense = if over_menu {
                Sense::hover()
            } else {
                Sense::click_and_drag()
            };
            let response = ui.interact(row_rect, egui::Id::new(("pl_row", actual_idx)), sense);
            if response.drag_started() {
                self.row_drag = Some(actual_idx);
            }
            if response.double_clicked() {
                action = PlaylistAction::PlayTrack(actual_idx);
            } else if response.clicked() {
                // Modifier keys reach us through egui's input state. Ctrl
                // toggles a single row, Shift extends from the anchor,
                // plain click replaces the selection.
                let modifiers = ui.ctx().input(|i| i.modifiers);
                action = if modifiers.ctrl {
                    PlaylistAction::ToggleSelectTrack(actual_idx)
                } else if modifiers.shift {
                    PlaylistAction::RangeSelectTrack(actual_idx)
                } else {
                    PlaylistAction::SelectTrack(actual_idx)
                };
            }
            // Drop: the drag is owned by the row it started on, so this
            // fires on the source row. Resolve the target slot from the
            // pointer's current y and emit a reorder.
            if response.drag_stopped()
                && let Some(from) = self.row_drag.take()
                && let Some(p) = ui.ctx().pointer_latest_pos()
            {
                let rel = ((p.y - list_rect.min.y + (self.scroll_offset % row_h_screen))
                    / row_h_screen)
                    .floor()
                    .max(0.0);
                let to = (start_index + rel as usize).min(entries.len().saturating_sub(1));
                if to != from {
                    action = PlaylistAction::MoveTrack { from, to };
                }
            }

            // Right-click context menu on the row — Play / Edit tags /
            // Remove / Add URL / Format. We bind the menu to *this* row
            // so `Edit tags` always references the row the user clicked
            // on, even if a different row holds the multi-selection.
            response.context_menu(|ui| {
                if ui.button("Play").clicked() {
                    action = PlaylistAction::PlayTrack(actual_idx);
                    ui.close_menu();
                }
                let is_queued = queued.get(actual_idx).copied().flatten().is_some();
                let queue_label = if is_queued {
                    "Remove from queue"
                } else {
                    "Play next (queue)"
                };
                if ui.button(queue_label).clicked() {
                    action = PlaylistAction::QueueTrack(actual_idx);
                    ui.close_menu();
                }
                if ui.button("Edit tags…").clicked() {
                    action = PlaylistAction::EditTags(actual_idx);
                    ui.close_menu();
                }
                if ui.button("Remove from playlist").clicked() {
                    action = PlaylistAction::RemoveAt(actual_idx);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Add URL…").clicked() {
                    action = PlaylistAction::AddUrl;
                    ui.close_menu();
                }
                if ui.button("Edit display format…").clicked() {
                    action = PlaylistAction::EditPlaylistFormat;
                    ui.close_menu();
                }
            });
        }

        // Mouse wheel scroll inside the list rect.
        if list_rect.contains(ui.ctx().pointer_latest_pos().unwrap_or(Pos2::ZERO)) {
            let scroll_delta = ui.ctx().input(|i| {
                if i.smooth_scroll_delta.y != 0.0 {
                    Some(i.smooth_scroll_delta.y)
                } else if i.raw_scroll_delta.y != 0.0 {
                    Some(i.raw_scroll_delta.y * 20.0)
                } else {
                    None
                }
            });
            if let Some(delta) = scroll_delta {
                self.scroll_offset = (self.scroll_offset - delta).clamp(0.0, max_offset);
            }
        }

        action
    }

    pub(super) fn render_scrollbar_thumb(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        entry_count: usize,
    ) {
        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Pledit)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };
        let scale = self.renderer.get_scale();
        let body_top = self.body_top();
        let body_bot = self.body_bot();
        let groove_h_skin = body_bot - body_top;
        let thumb_h_skin = 18u32;
        if groove_h_skin <= thumb_h_skin {
            return;
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

        let pressed = matches!(self.drag, Some(DragKind::Scrollbar { .. }));
        let sprite_x = if pressed { 61 } else { 52 };
        if let Some(thumb) = atlas.extract_region(sprite_x, 53, 8, thumb_h_skin) {
            let pos = self
                .renderer
                .skin_to_screen(PL_WIDTH - RIGHT_W + 5, thumb_y_skin, offset);
            self.renderer
                .render_region(ui, &thumb, pos, &format!("pl_thumb_{}", pressed));
        }
    }

    /// Paint the two time fields of the bottom-right bar with the WSZ
    /// `text.bmp` glyphs, like Winamp:
    /// - upper field (x=133, bar+10): running time of the selection / of
    ///   the whole list, `+` appended when some lengths are unknown;
    /// - lower field (bar+23): elapsed time of the current track, minutes
    ///   right-aligned before and seconds after the colon baked into
    ///   `pledit.bmp` at x=208. Blank when stopped.
    ///
    /// Falls back to the playlist font when the atlas is missing.
    pub(super) fn render_mini_time(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        entries: &[PlaylistEntry],
        selected: &std::collections::BTreeSet<usize>,
    ) {
        let (mut sel, mut total, mut unknown) = (0.0f32, 0.0f32, false);
        for (i, e) in entries.iter().enumerate() {
            match e.duration {
                Some(d) => {
                    total += d;
                    if selected.contains(&i) {
                        sel += d;
                    }
                }
                None => unknown = true,
            }
        }
        let plus = if unknown { "+" } else { "" };
        let running = format!("{}/{}{plus}", fmt_hms(sel), fmt_hms(total));
        let bar = self.body_bot();
        self.paint_text(ui, offset, &running, 133, bar + 10);

        if !self.stopped {
            let s = self.current_time_secs.max(0.0) as u32;
            let mins = (s / 60).min(99).to_string();
            let x = 208 - 5 * mins.len() as u32;
            self.paint_text(ui, offset, &mins, x, bar + 23);
            self.paint_text(ui, offset, &format!("{:02}", s % 60), 211, bar + 23);
        }
    }

    fn paint_text(&mut self, ui: &mut egui::Ui, offset: Pos2, text: &str, x: u32, y: u32) {
        let pos = self.renderer.skin_to_screen(x, y, offset);
        if bitmap_font::render_text(&mut self.renderer, ui, text, pos).is_none() {
            ui.painter().text(
                pos,
                egui::Align2::LEFT_TOP,
                text,
                pledit_font_id(self.renderer.get_skin(), 7.0 * self.renderer.get_scale()),
                self.pledit_color(|c| c.normal),
            );
        }
    }

    /// Render the open submenu: rows stacked upwards from the parent button
    /// (bottom row covers it), hovered row in its pressed sprite, 3-px bar
    /// on the left over the button's bevel.
    pub(super) fn render_submenu(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let Some(idx) = self.open_submenu else {
            return;
        };
        let Some(atlas) = self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Pledit)
            .cloned()
        else {
            return;
        };
        let parent = &PARENTS[idx];
        let rows = parent.actions.len();
        let hovered = ui
            .ctx()
            .pointer_latest_pos()
            .and_then(|p| self.submenu_row_at(p, offset));

        for (row, atlas_y) in SUB_ATLAS_Y.iter().take(rows).enumerate() {
            let hot = hovered == Some(row);
            let atlas_x = parent.atlas_x + if hot { 23 } else { 0 };
            if let Some(region) = atlas.extract_region(atlas_x, *atlas_y, 22, SUB_H) {
                let pos = self.submenu_row_rect(offset, idx, row).min;
                self.renderer
                    .render_region(ui, &region, pos, &format!("pl_sub_{idx}_{row}_{hot}"));
            }
        }

        if let Some(region) = atlas.extract_region(parent.bar_x, 111, 3, rows as u32 * SUB_H) {
            let top = self.submenu_row_rect(offset, idx, 0).min;
            let pos = Pos2::new(top.x - 3.0 * self.renderer.get_scale(), top.y);
            self.renderer
                .render_region(ui, &region, pos, &format!("pl_sub_bar_{idx}"));
        }
    }
}

/// `M:SS`, or `H:MM:SS` past an hour.
fn fmt_hms(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fmt_hms_formats() {
        assert_eq!(super::fmt_hms(0.0), "0:00");
        assert_eq!(super::fmt_hms(125.9), "2:05");
        assert_eq!(super::fmt_hms(3725.0), "1:02:05");
    }

    #[test]
    fn snap_height_steps() {
        use super::super::{PL_MAX_HEIGHT, snap_height};
        assert_eq!(snap_height(0), 116);
        assert_eq!(snap_height(232), 232);
        assert_eq!(snap_height(245), 232);
        assert_eq!(snap_height(247), 261);
        assert_eq!(snap_height(10_000), PL_MAX_HEIGHT);
    }
}
