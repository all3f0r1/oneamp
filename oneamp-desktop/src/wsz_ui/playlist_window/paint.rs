use super::{
    BOTTOM_H, DragKind, LEFT_W, PARENTS, PL_WIDTH, PlaylistAction, PlaylistWindow, RIGHT_W,
    ROW_H_SKIN, TITLE_H, pledit_font_id,
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

        // ---- Parent button pressed-state overlay -------------------------
        // pledit.bmp ships ADD/REM/SEL/MISC/LIST as unpressed-only sprites
        // baked into the bottom-bar extracts above. Real Winamp paints a
        // darker tint on top of the pressed button so the user gets click
        // feedback; we reproduce that with a translucent dark rectangle
        // since there's no dedicated pressed sprite to overlay. Also
        // visually anchors the button while its submenu is unfolded.
        //
        // The baked button artwork includes a 3-px left bevel that sits
        // *outside* the spec's `(skin_x, w=22)` body rect. A 22-wide
        // overlay would leave that bevel un-darkened and create a stray
        // bright column at the button's left edge while it's pressed —
        // widen the overlay to 25 px starting at `skin_x - 3` so the
        // bevel dims with the body.
        let pressed_idx = self.pressed_button.or(self.open_submenu);
        if let Some(idx) = pressed_idx {
            let parent = &PARENTS[idx];
            let overlay_x = parent.skin_x.saturating_sub(3);
            let rect = super::skin_rect(&self.renderer, offset, overlay_x, 202, 25, 18);
            ui.painter()
                .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(80));
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
        self.scroll_offset = self.scroll_offset.clamp(0.0, max_offset);

        let visible_rows = (list_rect.height() / row_h_screen).ceil() as usize + 1;
        let start_index = (self.scroll_offset / row_h_screen) as usize;
        let end_index = (start_index + visible_rows).min(entries.len());

        let mut action = PlaylistAction::None;

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

            let response = ui.interact(
                row_rect,
                egui::Id::new(("pl_row", actual_idx)),
                Sense::click_and_drag(),
            );
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
        let sprite_x = if pressed { 62 } else { 52 };
        if let Some(thumb) = atlas.extract_region(sprite_x, 53, 8, thumb_h_skin) {
            let pos = self
                .renderer
                .skin_to_screen(PL_WIDTH - RIGHT_W + 5, thumb_y_skin, offset);
            self.renderer
                .render_region(ui, &thumb, pos, &format!("pl_thumb_{}", pressed));
        }
    }

    /// Paint the playlist's "Time status display" (90 px wide, atlas
    /// y=82), rendered through the WSZ `text.bmp` glyph atlas — same
    /// typography as the title scroller and KBPS readout. Format follows
    /// Winamp: `M:SS/M:SS` (elapsed/total, no leading zero on minutes)
    /// when the track length is known, otherwise just `M:SS` elapsed.
    ///
    /// Falls back to the playlist font when the atlas is missing or
    /// undersized; the bitmap path silently no-ops there rather than
    /// painting nothing.
    ///
    /// We deliberately render in the y=82 status zone, NOT the mini-
    /// transport's y=95 digit slot. Both are real fields in pledit.bmp,
    /// but Winamp itself uses y=82 for the active time readout — that's
    /// the visible "0:00/9:24" in screenshots; y=95 is a separate, much
    /// smaller pair of slots that stay blank in stock skins.
    pub(super) fn render_mini_time(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let scale = self.renderer.get_scale();
        let fmt = |secs: f32| -> String {
            let s = secs.max(0.0) as u32;
            let mm = (s / 60).min(999);
            let ss = s % 60;
            format!("{}:{:02}", mm, ss)
        };
        let elapsed = fmt(self.current_time_secs);
        let text = match self.current_total_secs {
            Some(total) => format!("{}/{}", elapsed, fmt(total)),
            None => elapsed,
        };
        let pos = self
            .renderer
            .skin_to_screen(133, self.body_bot() + (82 - 72), offset);
        if bitmap_font::render_text(&mut self.renderer, ui, &text, pos).is_none() {
            ui.painter().text(
                pos,
                egui::Align2::LEFT_TOP,
                text,
                pledit_font_id(self.renderer.get_skin(), 7.0 * scale),
                self.pledit_color(|c| c.normal),
            );
        }
    }

    /// Render the open submenu (3 stacked sub-buttons above the parent).
    pub(super) fn render_submenu(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let Some(idx) = self.open_submenu else {
            return;
        };
        let parent = &PARENTS[idx];
        let Some(atlas_x) = parent.submenu_atlas_x else {
            return;
        };

        let atlas = match self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Pledit)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        let parent_y: u32 = 202;
        let sub_h: u32 = 18;
        let dst_top = parent_y - 3 * sub_h;
        // The parent button is at (skin_x, parent_y); 3 sub-buttons stack
        // *above* it (indices 0..3 going down), so the topmost sub-button
        // sits at parent_y - 3 * sub_h.
        //
        // The parent button artwork baked into the bottom-bar extract
        // carries a 3-px left bevel that sits *outside* the spec's
        // `(skin_x, w=22)` body rect. Sub-menu sprites at (atlas_x, …) are
        // 22 px wide and have no equivalent bevel: drawn at `parent.skin_x`
        // they expose the parent's bevel as a stray column to their left
        // and look shifted right relative to the parent. Shifting the dst
        // by `-SUBMENU_BEVEL_X_SHIFT` overdraws the bevel so the column
        // disappears.
        const SUBMENU_BEVEL_X_SHIFT: u32 = 3;
        let dst_x = parent.skin_x.saturating_sub(SUBMENU_BEVEL_X_SHIFT);

        let atlas_ys = [111u32, 130u32, 149u32];
        for (row, atlas_y) in atlas_ys.iter().enumerate() {
            let dst_y = parent_y - (3 - row as u32) * sub_h;
            // Rows whose sub-action is unwired still render the sprite so
            // the popup looks like Winamp's; the click path returns None
            // for them, which means the button is visible but inert.
            if let Some(region) = atlas.extract_region(atlas_x, *atlas_y, 22, sub_h) {
                let pos = self.renderer.skin_to_screen(dst_x, dst_y, offset);
                self.renderer
                    .render_region(ui, &region, pos, &format!("pl_sub_{}_{}", idx, row));
            }
        }

        // Decoration bar: 3×54 cosmetic strip painted at the right edge of
        // the unfolded column. Sourced from the column's slot in pledit.bmp
        // at y=111 (the bar continues across all three sub-rows in the
        // atlas, so a single 3×54 extract covers the whole popup height).
        // Without this the unfolded popup looks like floating loose buttons
        // instead of the framed widget Winamp shows. Anchored to the
        // shifted sub-button column so the popup keeps a single visual
        // alignment line.
        if let Some(deco_x) = parent.decoration_atlas_x
            && let Some(region) = atlas.extract_region(deco_x, 111, 3, 54)
        {
            let pos = self.renderer.skin_to_screen(dst_x + 22, dst_top, offset);
            self.renderer
                .render_region(ui, &region, pos, &format!("pl_sub_deco_{}", idx));
        }
    }
}
