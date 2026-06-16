mod input;
mod paint;

use super::renderer::WszRenderer;
use crate::app::WSZ_PLEDIT_FONT_FAMILY;
use egui::{Color32, Context, Pos2, Rect, Vec2};
use oneamp_core::PlaylistEntry;
use oneamp_core::wsz::skin::{SkinComponent, WszSkin};
use oneamp_core::{AudioEngine, AudioEvent};

/// Pick the FontId for playlist text. When the skin shipped a TTF in its
/// archive (registered as `WSZ_PLEDIT_FONT_FAMILY` at load time, see
/// `app::apply_skin_fonts`), use it. Otherwise fall back to egui's
/// proportional family — base-2.91 (and most stock Winamp skins) request
/// `Font=Arial` in `pledit.txt` and don't ship a TTF, so the proportional
/// family gets us closest to Winamp's playlist look. Monospace was the
/// old fallback and made rows look like a terminal pane instead of the
/// Arial-rendered playlist users expect.
pub(super) fn pledit_font_id(skin: &WszSkin, size: f32) -> egui::FontId {
    if skin.font_data.is_some() {
        egui::FontId::new(size, egui::FontFamily::Name(WSZ_PLEDIT_FONT_FAMILY.into()))
    } else {
        egui::FontId::proportional(size)
    }
}

/// Action emitted by the playlist window for the application to handle.
/// The window itself does not own the playlist state — it renders a slice
/// passed in by the caller and reports back what the user did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistAction {
    None,
    /// Plain click on a row: replace the whole selection with this index.
    SelectTrack(usize),
    /// Ctrl+click: flip whether `idx` is selected, leave the others.
    ToggleSelectTrack(usize),
    /// Shift+click: extend the selection from the previous anchor to
    /// `idx` inclusive.
    RangeSelectTrack(usize),
    PlayTrack(usize),
    AddFiles,
    /// User picked "Add URL…" from a menu (clutterbar or playlist
    /// context menu). The app spawns a small dialog that asks for an
    /// HTTP(S) URL and either appends the stream to the playlist or
    /// starts playing it directly.
    AddUrl,
    RemoveSelected,
    /// Remove exactly one row, regardless of the current selection
    /// state. Emitted by the right-click "Remove from playlist" entry
    /// so the user doesn't have to also click the row first.
    RemoveAt(usize),
    /// User picked "Edit tags…" from the right-click context menu on a
    /// row. The app opens the tag editor dialog scoped to this entry's
    /// file path.
    EditTags(usize),
    /// User picked "Edit playlist format…" from the right-click
    /// context menu. Opens a small dialog where the user can edit the
    /// `playlist_display_format` template.
    EditPlaylistFormat,
    Clear,
    SaveM3u,
    LoadM3u,
    /// SEL submenu — select every entry.
    SelectAll,
    /// SEL submenu — clear the selection.
    SelectNone,
    /// SEL submenu — invert the selection (selected ↔ unselected).
    InvertSelection,
    /// MISC submenu — sort entries by title (case-insensitive).
    SortByTitle,
    /// Drag-reorder: move the entry at `from` to slot `to` (both in the
    /// view's index space — the app remaps them through the active
    /// filter before mutating the real playlist).
    MoveTrack {
        from: usize,
        to: usize,
    },
    /// Toggle whether the entry at `idx` is in the "play next" queue
    /// (right-click → Play next / Remove from queue).
    QueueTrack(usize),
    /// User clicked the close button on the playlist titlebar.
    Close,
}

/// Default playlist window dimensions in skin space.
pub(super) const PL_WIDTH: u32 = 275;
pub const PL_DEFAULT_HEIGHT: u32 = 232;
pub(super) const PL_MIN_HEIGHT: u32 = 116;
pub(super) const PL_MAX_HEIGHT: u32 = 800;

/// Title bar height (cornerpieces + tile).
pub(super) const TITLE_H: u32 = 20;
/// Bottom control area height (control bars).
pub(super) const BOTTOM_H: u32 = 38;
/// Left side filler width.
pub(super) const LEFT_W: u32 = 12;
/// Right side filler width (5 + 8 + 7 — left bar + scroll groove + right bar).
pub(super) const RIGHT_W: u32 = 20;
/// Skin-space height of one playlist row.
pub(super) const ROW_H_SKIN: u32 = 11;

/// Description of a parent button: where it sits, and whether/how its
/// submenu unfolds.
pub(super) struct ParentSpec {
    /// Skin-space x of the button (y is fixed at 202).
    pub(super) skin_x: u32,
    /// Direct action when no submenu is configured (kept for parity with the
    /// pre-submenu wiring: SEL/MISC have no submenu yet, so a plain click
    /// still does something useful).
    pub(super) fallback: Option<PlaylistAction>,
    /// Atlas x of the submenu's column of sub-button sprites (each 22×18,
    /// stacked at atlas y=111, 130, 149).
    pub(super) submenu_atlas_x: Option<u32>,
    /// Atlas x of the column's 3×54 decoration bar (cosmetic vertical strip
    /// painted to the right of the unfolded sub-buttons). The bar at
    /// atlas y=111 spans all three sub-rows and visually links them into a
    /// single popup. Per `WSZ_FORMAT.md` §pledit.bmp.
    pub(super) decoration_atlas_x: Option<u32>,
    /// Actions to dispatch when the user clicks each row of the submenu
    /// (top → bottom). `None` slots are no-ops.
    pub(super) submenu_actions: [Option<PlaylistAction>; 3],
}

/// Static layout for the 5 parent buttons (ADD/REM/SEL/MISC/LIST) along the
/// bottom row. Positions match `pledit.bmp`'s baked-in sprites at atlas
/// y=80; SEL and MISC have no submenu wired yet so they don't unfold.
pub(super) const PARENTS: [ParentSpec; 5] = [
    ParentSpec {
        skin_x: 14,
        fallback: None,
        submenu_atlas_x: Some(0),
        decoration_atlas_x: Some(48),
        submenu_actions: [
            // URL → DIR → FILE. None of these distinguish in the v1 file
            // dialog, so all map to AddFiles. URL/DIR support comes later.
            Some(PlaylistAction::AddFiles),
            Some(PlaylistAction::AddFiles),
            Some(PlaylistAction::AddFiles),
        ],
    },
    ParentSpec {
        skin_x: 43,
        fallback: None,
        submenu_atlas_x: Some(54),
        decoration_atlas_x: Some(100),
        submenu_actions: [
            // All (Remove all) → CROP (Remove all but selected — not impl)
            // → FILE (Remove selected).
            Some(PlaylistAction::Clear),
            None,
            Some(PlaylistAction::RemoveSelected),
        ],
    },
    ParentSpec {
        skin_x: 72,
        fallback: None,
        // SEL → INV / NONE / ALL.
        submenu_atlas_x: Some(104),
        decoration_atlas_x: Some(150),
        submenu_actions: [
            Some(PlaylistAction::InvertSelection),
            Some(PlaylistAction::SelectNone),
            Some(PlaylistAction::SelectAll),
        ],
    },
    ParentSpec {
        skin_x: 101,
        fallback: None,
        // MISC → SORT / FILE INF / MISC OPTS. Only SORT is wired —
        // FILE INF + MISC OPTS leave the click as a no-op until a
        // dialog surface is built.
        submenu_atlas_x: Some(154),
        decoration_atlas_x: Some(200),
        submenu_actions: [Some(PlaylistAction::SortByTitle), None, None],
    },
    ParentSpec {
        skin_x: 232,
        fallback: None,
        submenu_atlas_x: Some(204),
        decoration_atlas_x: Some(250),
        submenu_actions: [
            // NEW (clear list) → SAVE → LOAD.
            Some(PlaylistAction::Clear),
            Some(PlaylistAction::SaveM3u),
            Some(PlaylistAction::LoadM3u),
        ],
    },
];

/// Mini-transport buttons in the bottom-right control bar.
#[derive(Debug, Clone, Copy)]
pub(super) struct MiniTransport {
    pub(super) skin_x: u32,
    pub(super) w: u32,
    pub(super) cmd: MiniCommand,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum MiniCommand {
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Open,
}

pub(super) const MINI_TRANSPORT: [MiniTransport; 6] = [
    MiniTransport {
        skin_x: 132,
        w: 7,
        cmd: MiniCommand::Previous,
    },
    MiniTransport {
        skin_x: 140,
        w: 8,
        cmd: MiniCommand::Play,
    },
    MiniTransport {
        skin_x: 149,
        w: 9,
        cmd: MiniCommand::Pause,
    },
    MiniTransport {
        skin_x: 159,
        w: 9,
        cmd: MiniCommand::Stop,
    },
    MiniTransport {
        skin_x: 169,
        w: 7,
        cmd: MiniCommand::Next,
    },
    MiniTransport {
        skin_x: 177,
        w: 9,
        cmd: MiniCommand::Open,
    },
];

pub(super) const MINI_TRANSPORT_Y: u32 = 94;
pub(super) const MINI_TRANSPORT_H: u32 = 8;

/// Resize handle hot area inside the bottom-right control bar.
pub(super) const RESIZE_HANDLE_X: u32 = 257;
pub(super) const RESIZE_HANDLE_Y: u32 = 89;
pub(super) const RESIZE_HANDLE_W: u32 = 19;
pub(super) const RESIZE_HANDLE_H: u32 = 21;

pub struct PlaylistWindow {
    pub(super) renderer: WszRenderer,
    /// Vertical scroll offset in pixels (screen space, not skin).
    pub(super) scroll_offset: f32,
    /// Pressed-state index of the parent button under the pointer (visual).
    pub(super) pressed_button: Option<usize>,
    /// Currently unfolded submenu (None = closed).
    pub(super) open_submenu: Option<usize>,
    /// True while the user holds the close button.
    pub(super) close_pressed: bool,
    /// Currently dragged sub-state. Shared so we don't double-process drags.
    pub(super) drag: Option<DragKind>,
    /// Source view-index of an in-progress row drag-reorder, set on
    /// `drag_started` over a row and consumed on `drag_stopped`. Kept
    /// separate from `drag` (scrollbar/resize) because row drags are
    /// driven by per-row egui responses inside `render_rows`, not the
    /// window-level pointer routing in `handle_input`.
    pub(super) row_drag: Option<usize>,
    /// Dynamic playlist height in skin space. Updated by the resize handle;
    /// queried by the coordinator to size the OS viewport.
    pub(super) height_skin: u32,
    /// When true, the window collapses to its 14-px shade strip drawn from
    /// `pledit.bmp` y=42 cornerpieces. Toggled by double-clicking the
    /// title bar — same gesture the main window uses.
    pub(super) shade_mode: bool,
    /// Latest playback time (seconds) — fed by `update(events)`. Drives the
    /// mini-transport time digits.
    pub(super) current_time_secs: f32,
    /// Total length of the loaded track. `None` when no track is loaded or
    /// the duration is unknown (e.g., streaming source); the mini-transport
    /// degrades to elapsed-only in that case.
    pub(super) current_total_secs: Option<f32>,
    pub(super) mouse_was_pressed: bool,
    /// Soft-focus flag pushed in by the coordinator. Drives the active vs
    /// inactive cornerpiece/title-tile extracts from `pledit.bmp`.
    pub(super) focused: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DragKind {
    /// Dragging the scrollbar thumb. Stores the screen-space offset of the
    /// pointer from the top of the thumb, so the thumb doesn't snap to the
    /// pointer's exact y at drag start.
    Scrollbar { grab_offset: f32 },
    /// Dragging the resize handle. Stores the initial pointer y and the
    /// initial height_skin so we can apply a stable delta each frame.
    Resize {
        start_pointer_y: f32,
        start_height_skin: u32,
    },
}

impl PlaylistWindow {
    pub fn new(skin: WszSkin, scale: f32) -> Self {
        Self {
            renderer: WszRenderer::new(skin, scale),
            scroll_offset: 0.0,
            pressed_button: None,
            open_submenu: None,
            close_pressed: false,
            drag: None,
            row_drag: None,
            height_skin: PL_DEFAULT_HEIGHT,
            shade_mode: false,
            current_time_secs: 0.0,
            current_total_secs: None,
            mouse_was_pressed: false,
            focused: false,
        }
    }

    /// Soft-focus setter driven by the coordinator. Drives the active vs
    /// inactive title-bar variants. `false` matches Winamp's screenshot
    /// where the playlist titlebar dims to its inactive sprite while the
    /// main player holds focus.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Apply audio events so the mini-transport time display stays in sync.
    pub fn update(&mut self, events: &[AudioEvent]) {
        for event in events {
            match event {
                AudioEvent::Position(current, total) => {
                    self.current_time_secs = *current;
                    // `total` is sent as 0.0 when unknown (streams, ICY, …);
                    // fold that to None so the mini-transport falls back to
                    // elapsed-only instead of printing "/0:00".
                    self.current_total_secs = if total.is_finite() && *total > 0.0 {
                        Some(*total)
                    } else {
                        None
                    };
                }
                AudioEvent::TrackLoaded(track) => {
                    self.current_total_secs = track.duration_secs;
                }
                _ => {}
            }
        }
    }

    /// Skin-space height the window currently occupies. Coordinator queries
    /// this each frame to compute the OS viewport size. In shade mode the
    /// window collapses to its 14-px title strip regardless of the
    /// resize-handle setting.
    pub fn height_skin(&self) -> u32 {
        if self.shade_mode {
            14
        } else {
            self.height_skin
        }
    }

    /// Render the playlist as a docked area inside the main viewport at
    /// `dock_y_skin` (skin-space y; 116 below main alone, 232 below main+EQ).
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &Context,
        dock_y_skin: u32,
        audio_engine: Option<&AudioEngine>,
        entries: &[PlaylistEntry],
        current_index: Option<usize>,
        selected: &std::collections::BTreeSet<usize>,
        queued: &[Option<usize>],
        display_format: &str,
    ) -> PlaylistAction {
        if self.shade_mode {
            return self.show_shade(ctx, dock_y_skin);
        }

        let scale = self.renderer.get_scale();
        let window_size = Vec2::new(PL_WIDTH as f32 * scale, self.height_skin as f32 * scale);

        let mut action = PlaylistAction::None;

        egui::Area::new(egui::Id::new("wsz_playlist_window"))
            .fixed_pos(Pos2::new(0.0, dock_y_skin as f32 * scale))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_min_size(window_size);
                ui.set_max_size(window_size);

                let area_rect = ui.max_rect();
                let offset = area_rect.min;

                // Pre-pass: refresh close + parent-button press visuals
                // so the rendered sprites match this frame's pointer
                // state. handle_input later runs its full click logic;
                // it can rely on these flags being set already.
                self.update_button_press_visuals(ui, offset);

                // Background fill (pledit's NormalBG) — keeps the list area
                // dark when the skin's frame doesn't fully cover it.
                let bg_color = self.pledit_color(|c| c.normal_bg);
                ui.painter()
                    .rect_filled(Rect::from_min_size(offset, window_size), 0.0, bg_color);

                // Title strip — interaction registered so the strip
                // pixels stay hit-testable (drag handling lives
                // elsewhere); render_frame paints over the unobstructed
                // area. Shade mode is disabled for sub-windows since
                // 1.0.5 — only the main window can be shaded. The
                // double-click → `shade_mode = true` handler that lived
                // here is gone.
                let title_rect =
                    Rect::from_min_size(offset, Vec2::new(PL_WIDTH as f32 * scale, 20.0 * scale));
                let _title_response = ui.interact(
                    title_rect,
                    egui::Id::new("pl_title_drag"),
                    egui::Sense::click(),
                );

                self.render_frame(ui, offset);
                let row_action = self.render_rows(
                    ui,
                    offset,
                    entries,
                    current_index,
                    selected,
                    queued,
                    display_format,
                );
                if row_action != PlaylistAction::None {
                    action = row_action;
                }
                self.render_scrollbar_thumb(ui, offset, entries.len());
                self.render_mini_time(ui, offset);
                self.render_submenu(ui, offset);

                let btn_action = self.handle_input(ui, offset, audio_engine, entries.len());
                if btn_action != PlaylistAction::None {
                    action = btn_action;
                }
            });

        action
    }

    /// Shade-mode rendering: 14-px title strip drawn from `pledit.bmp`
    /// y=42 cornerpieces (left + right) plus a tiled middle band. Other
    /// chrome is hidden. Double-click anywhere on the strip toggles back
    /// to full mode.
    fn show_shade(&mut self, ctx: &Context, dock_y_skin: u32) -> PlaylistAction {
        let scale = self.renderer.get_scale();
        let strip_size = Vec2::new(PL_WIDTH as f32 * scale, 14.0 * scale);

        let mut action = PlaylistAction::None;

        egui::Area::new(egui::Id::new("wsz_playlist_window"))
            .fixed_pos(Pos2::new(0.0, dock_y_skin as f32 * scale))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_min_size(strip_size);
                ui.set_max_size(strip_size);
                let area_rect = ui.max_rect();
                let offset = area_rect.min;

                // Background fill so the strip is visible even on skins
                // missing the y=42 cornerpieces.
                ui.painter().rect_filled(
                    Rect::from_min_size(offset, strip_size),
                    0.0,
                    self.pledit_color(|c| c.normal_bg),
                );

                if let Some(atlas) = self
                    .renderer
                    .get_skin()
                    .get_bitmap(&SkinComponent::Pledit)
                    .cloned()
                {
                    // Left cornerpiece 25×14 from y=42
                    if let Some(piece) = atlas.extract_region(0, 42, 25, 14) {
                        let pos = self.renderer.skin_to_screen(0, 0, offset);
                        self.renderer.render_region(ui, &piece, pos, "pl_shade_tl");
                    }
                    // Right cornerpiece 25×14 from y=42
                    if let Some(piece) = atlas.extract_region(153, 42, 25, 14) {
                        let pos = self.renderer.skin_to_screen(PL_WIDTH - 25, 0, offset);
                        self.renderer.render_region(ui, &piece, pos, "pl_shade_tr");
                    }
                    // Middle tile — 100×14 segment from (26,42), tiled
                    // across the gap between the two cornerpieces. The
                    // last copy is clipped so we don't overshoot.
                    if let Some(tile) = atlas.extract_region(26, 42, 100, 14) {
                        let mut x = 25u32;
                        while x < PL_WIDTH - 25 {
                            let span = (PL_WIDTH - 25 - x).min(100);
                            let region = if span == 100 {
                                tile.clone()
                            } else {
                                match atlas.extract_region(26, 42, span, 14) {
                                    Some(r) => r,
                                    None => break,
                                }
                            };
                            let pos = self.renderer.skin_to_screen(x, 0, offset);
                            self.renderer.render_region(
                                ui,
                                &region,
                                pos,
                                &format!("pl_shade_mid_{x}"),
                            );
                            x += 100;
                        }
                    }
                }

                let response = ui.interact(
                    Rect::from_min_size(offset, strip_size),
                    egui::Id::new("pl_shade_strip_click"),
                    egui::Sense::click(),
                );
                if response.double_clicked() {
                    self.shade_mode = false;
                }
                if response.clicked()
                    && let Some(pos) = ui.ctx().pointer_latest_pos()
                {
                    // Close button at (264, 3, 9, 9) skin space —
                    // same coords as full-mode close.
                    let close_pos = self.renderer.skin_to_screen(264, 3, offset);
                    let close_rect =
                        Rect::from_min_size(close_pos, Vec2::new(9.0 * scale, 9.0 * scale));
                    if close_rect.contains(pos) {
                        action = PlaylistAction::Close;
                    }
                }
            });

        action
    }

    pub(super) fn pledit_color(
        &self,
        pick: impl Fn(&oneamp_core::wsz::pledit::PleditColors) -> [u8; 3],
    ) -> Color32 {
        let rgb = pick(&self.renderer.get_skin().pledit.colors);
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    pub(super) fn body_top(&self) -> u32 {
        TITLE_H
    }
    pub(super) fn body_bot(&self) -> u32 {
        self.height_skin - BOTTOM_H
    }
}

pub(super) fn skin_rect(
    renderer: &WszRenderer,
    offset: Pos2,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Rect {
    let scale = renderer.get_scale();
    let pos = renderer.skin_to_screen(x, y, offset);
    Rect::from_min_size(pos, Vec2::new(w as f32 * scale, h as f32 * scale))
}
