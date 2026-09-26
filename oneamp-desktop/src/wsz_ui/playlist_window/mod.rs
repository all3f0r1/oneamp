mod input;
mod paint;

use super::renderer::WszRenderer;
use crate::app::WSZ_PLEDIT_FONT_FAMILY;
use egui::{Color32, Context, Pos2, Rect, Vec2};
use oneamp_core::PlaylistEntry;
use oneamp_core::wsz::skin::WszSkin;
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
    /// ADD → DIR: pick a folder and append its audio files.
    AddDir,
    /// Mini-transport eject: Winamp's "Play file" (replace + play),
    /// same as the main window's eject.
    OpenFile,
    /// Mini-transport play: Winamp `X` semantics (start / resume / restart).
    TransportPlay,
    /// Mini-transport pause: toggles pause, no-op when stopped.
    TransportPause,
    /// User picked "Add URL…" from a menu (clutterbar or playlist
    /// context menu). The app spawns a small dialog that asks for an
    /// HTTP(S) URL and either appends the stream to the playlist or
    /// starts playing it directly.
    AddUrl,
    RemoveSelected,
    /// REM → CROP: remove every entry that is *not* selected.
    Crop,
    /// REM → MISC: remove entries whose file no longer exists.
    RemoveDead,
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
/// Winamp resizes the playlist in steps of the 29-px side tile, so the
/// side fillers never end on a clipped tile.
pub(super) const PL_HEIGHT_STEP: u32 = 29;
pub(super) const PL_MAX_HEIGHT: u32 = PL_MIN_HEIGHT + 23 * PL_HEIGHT_STEP;

/// Clamp `h` to the allowed range and round it to the nearest step.
pub(super) fn snap_height(h: i32) -> u32 {
    let steps = ((h - PL_MIN_HEIGHT as i32) as f32 / PL_HEIGHT_STEP as f32).round();
    (PL_MIN_HEIGHT as i32 + steps as i32 * PL_HEIGHT_STEP as i32)
        .clamp(PL_MIN_HEIGHT as i32, PL_MAX_HEIGHT as i32) as u32
}

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

/// Submenu row height, and the atlas y of each row's sprite (19-px pitch:
/// 18 px of sprite + 1 px gap).
pub(super) const SUB_H: u32 = 18;
pub(super) const SUB_ATLAS_Y: [u32; 4] = [111, 130, 149, 168];

/// A parent button (ADD/REM/SEL/MISC/LIST) and the popup it unfolds.
///
/// Winamp layout: the popup's bottom row covers the parent button itself
/// and the rows stack upwards; a 3-px bar sits on the left, over the
/// button's left bevel. Normal sprites live at `atlas_x`, hovered ones at
/// `atlas_x + 23`, the bar at `bar_x` (height = rows × 18).
pub(super) struct ParentSpec {
    /// Skin-space x of the button body (y follows the bottom bar, see
    /// `buttons_y`).
    pub(super) skin_x: u32,
    pub(super) atlas_x: u32,
    pub(super) bar_x: u32,
    /// One action per row, top → bottom. `None` rows render but do nothing.
    pub(super) actions: &'static [Option<PlaylistAction>],
}

pub(super) const PARENTS: [ParentSpec; 5] = [
    ParentSpec {
        skin_x: 14,
        atlas_x: 0,
        bar_x: 48,
        // URL / DIR / FILE
        actions: &[
            Some(PlaylistAction::AddUrl),
            Some(PlaylistAction::AddDir),
            Some(PlaylistAction::AddFiles),
        ],
    },
    ParentSpec {
        skin_x: 43,
        atlas_x: 54,
        bar_x: 100,
        // ALL / CROP / SEL / MISC (Winamp's MISC menu → remove dead files)
        actions: &[
            Some(PlaylistAction::Clear),
            Some(PlaylistAction::Crop),
            Some(PlaylistAction::RemoveSelected),
            Some(PlaylistAction::RemoveDead),
        ],
    },
    ParentSpec {
        skin_x: 72,
        atlas_x: 104,
        bar_x: 150,
        // INV / ZERO / ALL
        actions: &[
            Some(PlaylistAction::InvertSelection),
            Some(PlaylistAction::SelectNone),
            Some(PlaylistAction::SelectAll),
        ],
    },
    ParentSpec {
        skin_x: 101,
        atlas_x: 154,
        bar_x: 200,
        // SORT / FILE INF / MISC OPTS — the last two have no dialog yet.
        actions: &[Some(PlaylistAction::SortByTitle), None, None],
    },
    ParentSpec {
        skin_x: 231,
        atlas_x: 204,
        bar_x: 250,
        // NEW / SAVE / LOAD
        actions: &[
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
    /// Currently unfolded submenu (None = closed).
    pub(super) open_submenu: Option<usize>,
    /// False while the press that opened the submenu is still held: its
    /// release over the bottom row (= the parent button) keeps the popup
    /// open instead of firing that row, so a plain click just unfolds it.
    pub(super) submenu_armed: bool,
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
    /// Latest playback time (seconds) — fed by `update(events)`. Drives the
    /// mini-transport time digits.
    pub(super) current_time_secs: f32,
    /// Total length of the loaded track. `None` when no track is loaded or
    /// the duration is unknown (e.g., streaming source); the mini-transport
    /// degrades to elapsed-only in that case.
    pub(super) current_total_secs: Option<f32>,
    /// Nothing playing: the mini time field stays blank, like Winamp.
    pub(super) stopped: bool,
    pub(super) mouse_was_pressed: bool,
    /// Soft-focus flag pushed in by the coordinator. Drives the active vs
    /// inactive cornerpiece/title-tile extracts from `pledit.bmp`.
    pub(super) focused: bool,
    /// Row to scroll into view on the next paint (keyboard navigation).
    pub ensure_visible: Option<usize>,
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
    /// Points per skin pixel.
    pub fn set_scale(&mut self, scale: f32) {
        self.renderer.set_scale(scale);
    }

    pub fn new(skin: WszSkin, scale: f32) -> Self {
        Self {
            renderer: WszRenderer::new(skin, scale),
            scroll_offset: 0.0,
            open_submenu: None,
            submenu_armed: false,
            close_pressed: false,
            drag: None,
            row_drag: None,
            height_skin: PL_DEFAULT_HEIGHT,
            current_time_secs: 0.0,
            current_total_secs: None,
            stopped: true,
            mouse_was_pressed: false,
            focused: false,
            ensure_visible: None,
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
                    self.stopped = false;
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
                AudioEvent::Stopped => self.stopped = true,
                _ => {}
            }
        }
    }

    /// Skin-space height the window currently occupies. Coordinator queries
    /// this each frame to compute the OS viewport size.
    pub fn height_skin(&self) -> u32 {
        self.height_skin
    }

    /// Resize-handle height. Persisted across launches.
    pub fn full_height_skin(&self) -> u32 {
        self.height_skin
    }

    pub fn set_full_height_skin(&mut self, h: u32) {
        self.height_skin = snap_height(h as i32);
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
                self.render_mini_time(ui, offset, entries, selected);
                self.render_submenu(ui, offset);

                let btn_action = self.handle_input(ui, offset, audio_engine, entries.len());
                if btn_action != PlaylistAction::None {
                    action = btn_action;
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
    /// Skin rect of submenu row `row` of the open popup.
    pub(super) fn submenu_row_rect(&self, offset: Pos2, idx: usize, row: usize) -> Rect {
        let n = PARENTS[idx].actions.len() as u32;
        let y = self.buttons_y() + SUB_H - (n - row as u32) * SUB_H;
        skin_rect(&self.renderer, offset, PARENTS[idx].skin_x, y, 22, SUB_H)
    }

    /// Row of the open submenu under `pos`, if any.
    pub(super) fn submenu_row_at(&self, pos: Pos2, offset: Pos2) -> Option<usize> {
        let idx = self.open_submenu?;
        (0..PARENTS[idx].actions.len())
            .find(|&r| self.submenu_row_rect(offset, idx, r).contains(pos))
    }

    /// Skin-space y of the ADD/REM/SEL/MISC/LIST buttons: 8 px into the
    /// bottom bar, so they follow the bar when the window is resized.
    pub(super) fn buttons_y(&self) -> u32 {
        self.body_bot() + 8
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
