//! Hierarchical "main menu" popup spawned by the top-left titlebar logo
//! (skin-space (6,3,9,9), the `TitlebarButton::Menu` slot).
//!
//! ## Layout
//!
//! Top-level panel anchors to the right of the Menu button (just under
//! the titlebar). Each panel is sized to its widest row's text plus a
//! fixed gutter, so labels never get truncated and unrelated submenus
//! don't share their width. Submenus open to the right of the parent
//! row by default and **flip to the left** when the right side would
//! overflow the screen — Windows / macOS convention.
//!
//! ## Typography
//!
//! Text uses the active skin's bundled TTF (registered as
//! `WSZ_PLEDIT_FONT_FAMILY`) when available. Stock Winamp skins like
//! base-2.91 don't ship a TTF; in that case we fall back to egui's
//! proportional family. Checkmarks and submenu arrows are drawn as
//! geometric primitives (not glyphs) so they render identically across
//! bitmap and TTF fonts — drawing them as text caused tofu boxes on
//! pixel-art bitmap fonts that don't ship `✓` (U+2713) or `▸` (U+25B8).
//!
//! ## Dismissal
//!
//! Click outside the visible chain closes the menu. The very press that
//! opened the menu is masked by a one-frame `just_opened` flag — without
//! it, the press itself (which the Menu button fires on press, not
//! release) would close the menu the same frame it appeared.

use std::path::PathBuf;

use egui::{Color32, FontFamily, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::app::WSZ_PLEDIT_FONT_FAMILY;
use crate::wsz_ui::main_window::{MainWindowAction, VisualizerMode};

/// One row in the menu (top-level or inside a submenu). Leaf rows emit
/// `action` on click; parent rows show `submenu` on hover.
#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    /// `None` for parent rows whose only job is to spawn a submenu.
    pub action: Option<MainWindowAction>,
    /// Leading checkmark gutter. `None` skips the gutter entirely;
    /// `Some(b)` reserves it and paints the check primitive if `b`.
    pub checked: Option<bool>,
    /// Nested submenu. Mutually exclusive with `action`.
    pub submenu: Vec<MenuItem>,
    /// Separator line painted ABOVE this row (group divider).
    pub separator_above: bool,
}

impl MenuItem {
    pub fn action(label: impl Into<String>, action: MainWindowAction) -> Self {
        Self {
            label: label.into(),
            action: Some(action),
            checked: None,
            submenu: Vec::new(),
            separator_above: false,
        }
    }

    pub fn toggle(label: impl Into<String>, action: MainWindowAction, on: bool) -> Self {
        Self {
            label: label.into(),
            action: Some(action),
            checked: Some(on),
            submenu: Vec::new(),
            separator_above: false,
        }
    }

    pub fn parent(label: impl Into<String>, submenu: Vec<MenuItem>) -> Self {
        Self {
            label: label.into(),
            action: None,
            checked: None,
            submenu,
            separator_above: false,
        }
    }

    pub fn with_separator(mut self) -> Self {
        self.separator_above = true;
        self
    }
}

/// State held across frames — open/closed plus the path of currently
/// armed rows down the submenu chain. `armed_path[i]` is the row index
/// at level `i`; the panel at level `i+1` shows the submenu of that
/// row. An empty path means only the top-level panel is visible.
#[derive(Default)]
pub struct MainMenu {
    pub open: bool,
    armed_path: Vec<usize>,
    /// True for exactly the first render after `toggle()` opened us.
    /// The titlebar Menu button fires on press, so the press event that
    /// opened us is still live this frame — skipping the focus-loss /
    /// click-outside dismissal on that frame stops the popup viewport
    /// from closing the instant it appears.
    just_opened: bool,
    /// Timestamp of the last close so the Menu button can ignore a press
    /// that immediately follows a focus-loss-driven dismissal. Without
    /// this, clicking the open Menu button would close-then-reopen the
    /// popup in two frames (focus-loss closes; the press the user just
    /// performed on the main window then re-toggles us open).
    last_closed_at: Option<std::time::Instant>,
}

impl MainMenu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reopen cooldown — see `last_closed_at`. 200 ms is long enough to
    /// absorb the press-event-after-focus-loss race but short enough that
    /// the user clicking the Menu button "twice" intentionally re-opens
    /// the popup as expected.
    const REOPEN_COOLDOWN: std::time::Duration = std::time::Duration::from_millis(200);

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            // Guard against the focus-loss-then-press race: if we closed
            // very recently, the press that triggered this toggle is the
            // same press that just stole focus from the popup. Ignore.
            if self
                .last_closed_at
                .is_some_and(|t| t.elapsed() < Self::REOPEN_COOLDOWN)
            {
                return;
            }
            self.open = true;
            self.just_opened = true;
        }
    }

    pub fn close(&mut self) {
        if self.open {
            self.last_closed_at = Some(std::time::Instant::now());
        }
        self.open = false;
        self.armed_path.clear();
        self.just_opened = false;
    }

    /// Render the menu chain as a standalone OS sub-viewport (a popup
    /// window) and return the picked action, if any.
    ///
    /// Using a real OS sub-viewport (via `Context::show_viewport_immediate`)
    /// frees the menu from the main window's clipping rect — at `1×` the
    /// player is only 275 px wide, but the popup floats above the screen
    /// and can extend wherever it needs to. The popup is undecorated,
    /// transparent (only the panels are opaque) and `AlwaysOnTop`; click
    /// outside steals focus and the focus-loss check inside the closure
    /// closes us. Escape and OS-level close requests also dismiss.
    ///
    /// Layout: panels are first laid out in coords local to the menu
    /// button (i.e. local to the popup origin), then the popup is
    /// positioned at the menu button's screen position and sized to the
    /// chain's bounding box. The closure re-uses the same local
    /// coordinates to paint and hit-test, so a flip-left at layout time
    /// (when a submenu would walk off-monitor to the right) shows up
    /// correctly inside the popup. Supports arbitrarily deep submenus by
    /// walking `armed_path` each frame.
    pub fn render(
        &mut self,
        parent_ctx: &egui::Context,
        scale: f32,
        items: &[MenuItem],
        skin_font_available: bool,
    ) -> Option<MainWindowAction> {
        if !self.open {
            return None;
        }

        let font_id = font_id_for_menu(skin_font_available, 10.0 * scale);
        let row_h = (font_id.size + 4.0 * scale).max(14.0 * scale);

        // 1. Screen-space position of the Menu button (logical points).
        //    The popup's top-left lands here so panel 0 sits directly
        //    under it.
        let (popup_screen_pos, monitor_size) = parent_ctx.input(|i| {
            let v = i.viewport();
            let outer = v.outer_rect.map(|r| r.min).unwrap_or(Pos2::ZERO);
            let mon = v.monitor_size.unwrap_or(Vec2::splat(4096.0));
            (
                Pos2::new(outer.x + 6.0 * scale, outer.y + 13.0 * scale),
                mon,
            )
        });

        // 2. Compute the panel chain layout in popup-local coords. The
        //    chain is `armed_path.len() + 1` panels deep; each panel
        //    knows its rect and the slice of `items` it draws.
        let panels = layout_panels(
            parent_ctx,
            items,
            &self.armed_path,
            &font_id,
            scale,
            row_h,
            popup_screen_pos,
            monitor_size,
        );
        if panels.is_empty() {
            // Defensive: should never happen, but if the top-level item
            // list was empty we'd close rather than spawn an empty popup.
            self.close();
            return None;
        }

        // 3. Bounding box of all panels (in popup-local coords) drives
        //    the popup's inner size. Panels can sit at negative x when a
        //    submenu flipped left of the parent, so we offset everything
        //    so the leftmost panel is at x=0 — and shift the popup
        //    position by the same amount so the menu button stays aligned
        //    with the row 0 of panel 0.
        let min_x = panels.iter().map(|p| p.rect.min.x).fold(0.0_f32, f32::min);
        let max_x = panels.iter().map(|p| p.rect.max.x).fold(0.0_f32, f32::max);
        let max_y = panels.iter().map(|p| p.rect.max.y).fold(0.0_f32, f32::max);
        let popup_origin = Pos2::new(popup_screen_pos.x + min_x, popup_screen_pos.y);
        let popup_size = Vec2::new(max_x - min_x, max_y);
        let panel_shift = Vec2::new(-min_x, 0.0);

        // 4. Spawn the immediate viewport. The closure runs synchronously
        //    in the same pass and returns (picked, new_armed_path, close).
        let armed_path_in = self.armed_path.clone();
        let just_opened = std::mem::take(&mut self.just_opened);
        let ppp = parent_ctx.pixels_per_point();
        let viewport_id = egui::ViewportId::from_hash_of("oneamp_main_menu");

        let (picked, new_armed_path, should_close) = parent_ctx.show_viewport_immediate(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title("OneAmp menu")
                .with_inner_size(popup_size)
                .with_position(popup_origin)
                .with_decorations(false)
                .with_resizable(false)
                .with_transparent(true)
                .with_window_level(egui::WindowLevel::AlwaysOnTop)
                .with_active(true)
                .with_taskbar(false),
            |popup_ctx, _class| {
                // Keep the popup's render-scale aligned with the parent's
                // so menu text physical size matches the rest of the
                // player regardless of the OS-reported monitor ppp.
                if (popup_ctx.pixels_per_point() - ppp).abs() > 0.01 {
                    popup_ctx.set_pixels_per_point(ppp);
                }

                let mut picked: Option<MainWindowAction> = None;
                let mut next_armed: Vec<usize> = Vec::with_capacity(armed_path_in.len() + 1);

                egui::Area::new(egui::Id::new("oneamp_main_menu_area"))
                    .fixed_pos(Pos2::ZERO)
                    .order(egui::Order::Foreground)
                    .show(popup_ctx, |ui| {
                        for (level, panel) in panels.iter().enumerate() {
                            let rect = panel.rect.translate(panel_shift);
                            Self::paint_panel(ui, rect);

                            let any_with_checked = panel.items.iter().any(|i| i.checked.is_some());
                            let mut hovered_submenu: Option<usize> = None;
                            let mut hovered_any_row = false;
                            for (i, item) in panel.items.iter().enumerate() {
                                let row_top = rect.min.y + i as f32 * row_h;
                                let row_rect = Rect::from_min_size(
                                    Pos2::new(rect.min.x, row_top),
                                    Vec2::new(rect.width(), row_h),
                                );
                                let response = ui.interact(
                                    row_rect,
                                    egui::Id::new(("menu_row", level, i)),
                                    Sense::click(),
                                );
                                let is_armed = armed_path_in.get(level).copied() == Some(i);
                                let highlighted = response.hovered() || is_armed;
                                Self::paint_row(
                                    ui,
                                    row_rect,
                                    item,
                                    scale,
                                    &font_id,
                                    highlighted,
                                    any_with_checked,
                                );

                                if item.separator_above && i > 0 {
                                    let sep_y = row_top + 1.0;
                                    ui.painter().line_segment(
                                        [
                                            Pos2::new(rect.min.x + 6.0 * scale, sep_y),
                                            Pos2::new(rect.max.x - 6.0 * scale, sep_y),
                                        ],
                                        Stroke::new(1.0, Color32::from_rgb(80, 120, 60)),
                                    );
                                }

                                if response.hovered() {
                                    hovered_any_row = true;
                                    if !item.submenu.is_empty() {
                                        hovered_submenu = Some(i);
                                    }
                                }
                                if response.clicked()
                                    && let Some(action) = &item.action
                                {
                                    picked = Some(action.clone());
                                }
                            }

                            // Hover-arm policy at this level:
                            // - Cursor on a parent row in this panel →
                            //   arm it (open / switch its submenu).
                            // - Cursor on a leaf row in this panel →
                            //   disarm: the previously-open submenu must
                            //   close, otherwise hovering "Crossfade"
                            //   after passing through "Output device"
                            //   leaves the output submenu dangling.
                            // - Cursor outside every row of this panel →
                            //   keep last-frame's armed parent. This is
                            //   the moment the cursor is traversing the
                            //   gap toward the child panel, or already
                            //   inside the child panel (where row hits
                            //   at this level are false). Dropping armed
                            //   then would flicker the child closed.
                            let next_idx = if hovered_any_row {
                                hovered_submenu
                            } else {
                                armed_path_in.get(level).copied().filter(|&i| {
                                    panel.items.get(i).is_some_and(|it| !it.submenu.is_empty())
                                })
                            };
                            if let Some(idx) = next_idx {
                                next_armed.push(idx);
                            }
                        }
                    });

                // Dismissal. Focus loss is the primary mechanism — when
                // the user clicks anywhere outside the popup the OS pulls
                // focus and we read `!i.focused` next frame. We also
                // honour Escape and any OS-level close request (Alt+F4,
                // compositor dismiss). The `just_opened` guard skips the
                // focus check on the first frame because the popup
                // hasn't actually gained focus yet on some compositors.
                let close = popup_ctx.input(|i| {
                    let lost_focus = !just_opened && !i.focused;
                    i.viewport().close_requested() || i.key_pressed(egui::Key::Escape) || lost_focus
                });
                if close || picked.is_some() {
                    popup_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // Repaint while we're open so hover state stays live —
                // without this the popup only repaints on input which
                // makes submenu arming feel sluggish when the cursor
                // hovers without clicking.
                popup_ctx.request_repaint();

                (picked, next_armed, close)
            },
        );

        self.armed_path = new_armed_path;
        if picked.is_some() || should_close {
            self.close();
        }
        picked
    }

    fn paint_panel(ui: &Ui, rect: Rect) {
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(20, 20, 20));
        // Inset the stroke by half its width so the 1-px green border
        // is fully inside the panel rect. Without this, the stroke is
        // centred on the edge — half outside — and panels flush against
        // the popup viewport boundary lose those outside halves to
        // viewport clipping, which is why the top / left / right /
        // bottom edges would silently drop in v1.0.3.
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            0.0,
            Stroke::new(1.0, Color32::from_rgb(120, 200, 80)),
        );
    }

    fn paint_row(
        ui: &Ui,
        rect: Rect,
        item: &MenuItem,
        scale: f32,
        font_id: &FontId,
        highlighted: bool,
        reserve_check_gutter: bool,
    ) {
        if highlighted {
            // Inset the highlight by 1 unit on every side so it sits
            // strictly inside the panel's 1-px green border instead of
            // painting over it. The row rect spans the full panel width,
            // and rows 0 / N-1 touch the panel's top / bottom edge — so
            // without this inset the hover fill clobbers the border on
            // any edge the row is flush against, which was the symptom
            // reported in 1.0.6.
            ui.painter()
                .rect_filled(rect.shrink(1.0), 0.0, Color32::from_rgb(40, 80, 40));
        }

        // Geometry constants (all in egui-units, multiplied by scale at
        // the point of use). The check gutter only reserves space when at
        // least one row in the panel uses it — pure-leaf submenus
        // (Lecteur, …) don't waste a 14-px column on nothing.
        let check_gutter = if reserve_check_gutter { 14.0 } else { 4.0 } * scale;
        let arrow_gutter = if item.submenu.is_empty() {
            6.0 * scale
        } else {
            14.0 * scale
        };

        // Checkmark — drawn as two stroked segments, not the `✓` glyph,
        // so bitmap fonts that don't ship U+2713 render the same shape.
        if matches!(item.checked, Some(true)) {
            let cx = rect.min.x + 7.0 * scale;
            let cy = rect.center().y;
            let s = 3.0 * scale;
            let stroke = Stroke::new(1.5 * scale, Color32::from_rgb(0, 220, 80));
            ui.painter().line_segment(
                [Pos2::new(cx - s, cy), Pos2::new(cx - s * 0.3, cy + s * 0.8)],
                stroke,
            );
            ui.painter().line_segment(
                [
                    Pos2::new(cx - s * 0.3, cy + s * 0.8),
                    Pos2::new(cx + s, cy - s * 0.7),
                ],
                stroke,
            );
        }

        // Label.
        ui.painter().text(
            Pos2::new(rect.min.x + check_gutter, rect.center().y),
            egui::Align2::LEFT_CENTER,
            item.label.as_str(),
            font_id.clone(),
            Color32::from_rgb(220, 220, 220),
        );

        // Submenu arrow — filled right-pointing triangle, drawn from
        // primitives so it doesn't depend on `▸` being in the font.
        if !item.submenu.is_empty() {
            let ax = rect.max.x - arrow_gutter * 0.5;
            let ay = rect.center().y;
            let s = 3.0 * scale;
            let points = vec![
                Pos2::new(ax - s * 0.5, ay - s),
                Pos2::new(ax - s * 0.5, ay + s),
                Pos2::new(ax + s * 0.7, ay),
            ];
            ui.painter().add(egui::Shape::convex_polygon(
                points,
                Color32::from_rgb(180, 220, 180),
                Stroke::NONE,
            ));
        }
    }
}

/// Pick the FontId for the menu text. Mirrors `pledit_font_id` in
/// `playlist_window` — uses the skin's bundled TTF when present so the
/// menu visually matches the rest of the player.
fn font_id_for_menu(skin_font_available: bool, size: f32) -> FontId {
    if skin_font_available {
        FontId::new(size, FontFamily::Name(WSZ_PLEDIT_FONT_FAMILY.into()))
    } else {
        FontId::proportional(size)
    }
}

/// Width of a single panel: max label width across every row plus the
/// check / arrow gutters reserved for that panel. Rounded up so the
/// rightmost label has a small margin from the panel border. Takes a
/// `&Context` so it can be called both inside an `Ui` (legacy) and
/// outside of one (popup viewport layout pre-pass).
fn panel_width(ctx: &egui::Context, items: &[MenuItem], font_id: &FontId, scale: f32) -> f32 {
    let any_with_checked = items.iter().any(|i| i.checked.is_some());
    let any_with_submenu = items.iter().any(|i| !i.submenu.is_empty());
    let left_gutter = if any_with_checked { 14.0 } else { 4.0 } * scale;
    let right_gutter = if any_with_submenu { 16.0 } else { 8.0 } * scale;

    let mut max_text = 0.0_f32;
    for item in items {
        let galley_w = ctx.fonts(|f| {
            let galley = f.layout_no_wrap(item.label.clone(), font_id.clone(), Color32::WHITE);
            galley.size().x
        });
        if galley_w > max_text {
            max_text = galley_w;
        }
    }
    (max_text + left_gutter + right_gutter).ceil()
}

/// Lazily-resolved slice of items at one panel of the chain, plus the
/// rect (in popup-local coords) where that panel paints. Built by
/// `layout_panels` so both sizing the popup viewport and rendering its
/// contents read from the same source of truth.
struct PanelLayout<'a> {
    items: &'a [MenuItem],
    rect: Rect,
}

/// Walk the armed-path chain and produce a panel layout: one
/// `PanelLayout` per visible level (top-level + each open submenu).
/// Coordinates are local to the *menu button* (i.e. the popup origin),
/// so panel 0 starts at (0, 0). Submenus default-anchor to the right
/// edge of the parent row, flipping to the left when the right side
/// would push past the monitor's right edge.
#[allow(clippy::too_many_arguments)]
fn layout_panels<'a>(
    ctx: &egui::Context,
    root_items: &'a [MenuItem],
    armed_path: &[usize],
    font_id: &FontId,
    scale: f32,
    row_h: f32,
    popup_screen_pos: Pos2,
    monitor_size: Vec2,
) -> Vec<PanelLayout<'a>> {
    let mut out: Vec<PanelLayout<'a>> = Vec::with_capacity(armed_path.len() + 1);
    let mut current: &'a [MenuItem] = root_items;
    let mut anchor = Pos2::ZERO;

    // Monitor bound for the flip-left heuristic, expressed in popup-local
    // coords (subtracting popup origin from monitor edge). Negative means
    // we have already overflowed — clamp anyway and let the popup span
    // off-screen rather than refuse to paint.
    let monitor_max_local = monitor_size.x - popup_screen_pos.x;

    let mut level: usize = 0;
    loop {
        let w = panel_width(ctx, current, font_id, scale);
        let h = row_h * current.len() as f32;
        let rect = Rect::from_min_size(anchor, Vec2::new(w, h));
        out.push(PanelLayout {
            items: current,
            rect,
        });

        let Some(&idx) = armed_path.get(level) else {
            break;
        };
        let Some(parent_item) = current.get(idx) else {
            break;
        };
        if parent_item.submenu.is_empty() {
            break;
        }
        let child = &parent_item.submenu;
        let child_w = panel_width(ctx, child, font_id, scale);
        let prefer_right = anchor.x + w + child_w <= monitor_max_local;
        let child_x = if prefer_right {
            anchor.x + w
        } else {
            anchor.x - child_w
        };
        let child_y = anchor.y + idx as f32 * row_h;
        anchor = Pos2::new(child_x, child_y);
        current = child;
        level += 1;
    }
    out
}

/// Snapshot of every flag/value the main menu reads. Pushed in by the
/// app once per frame.
pub struct MenuContext {
    pub always_on_top: bool,
    pub crossfade_enabled: bool,
    pub replaygain_mode: oneamp_core::ReplayGainMode,
    pub mono_enabled: bool,
    pub loudness_enabled: bool,
    pub track_notifications_enabled: bool,
    pub shade_mode: bool,
    pub eq_visible: bool,
    pub playlist_visible: bool,
    pub visualizer_mode: VisualizerMode,
    pub visualizer_options: super::visualization::VisualizerOptions,
    pub user_scale: Option<f32>,
    pub output_devices: Vec<String>,
    pub current_output_device: Option<String>,
    pub recent_paths: Vec<PathBuf>,
    pub sleep_timer_minutes: Option<u32>,
    /// Whether "Stop after current track" is currently armed. Drives the
    /// matching ✓ in the Player menu. Session-only on the app side, so
    /// this is purely a mirror — not persisted.
    pub stop_after_current: bool,
    /// Whether the "Resume long files" preference is on. Drives the
    /// ✓ in the Audio menu. Persisted via `AppConfig::resume_long_files`.
    pub resume_long_files: bool,
    /// Whether the active skin shipped a TTF — drives font selection for
    /// the menu text so the popup matches the rest of the player.
    pub skin_font_available: bool,
}

/// Build the full menu tree from `ctx`. All labels are in English to
/// match the rest of the player UI.
pub fn build_menu_items(ctx: &MenuContext) -> Vec<MenuItem> {
    use MainWindowAction as A;

    let mut recent_items: Vec<MenuItem> = ctx
        .recent_paths
        .iter()
        .take(10)
        .map(|p| {
            let label = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown")
                .to_string();
            MenuItem::action(label, A::PlayRecent(p.clone()))
        })
        .collect();
    if recent_items.is_empty() {
        recent_items.push(MenuItem {
            label: "(empty)".to_string(),
            action: None,
            checked: None,
            submenu: Vec::new(),
            separator_above: false,
        });
    }

    let player_menu = vec![
        MenuItem::action("Open file…", A::OpenFile),
        MenuItem::action("Open folder…", A::OpenFolder),
        MenuItem::action("Load playlist…", A::LoadPlaylist),
        MenuItem::action("Save playlist…", A::SavePlaylist).with_separator(),
        MenuItem::toggle(
            "Stop after current",
            A::ToggleStopAfterCurrent,
            ctx.stop_after_current,
        ),
        MenuItem::action("Clear playlist", A::ClearPlaylist),
        MenuItem::parent("Recent", recent_items).with_separator(),
        MenuItem::action("Quit", A::Quit).with_separator(),
    ];

    // Integer-only scale presets. Fractional scales (1.25, 1.5, …)
    // sub-pixel-sample the WSZ sprite atlas — every Winamp pixel ends
    // up as some pixels-of-irregular-width on screen, the visible
    // "soft and uneven" look that's wrong for a pixel-perfect skin
    // player. We deliberately don't offer them in the UI; the boot
    // path also clamps any legacy fractional value persisted from
    // an older OneAmp up to the next integer (see
    // `OneAmpApp::new`).
    fn scale_matches(a: Option<f32>, b: f32) -> bool {
        a.is_some_and(|v| (v - b).abs() < 0.005)
    }
    let scale_presets: &[f32] = &[1.0, 2.0, 3.0, 4.0];
    let mut scale_menu = Vec::with_capacity(scale_presets.len() + 1);
    scale_menu.push(
        MenuItem::toggle(
            "Auto (DPI)",
            A::SetUserScale(None),
            ctx.user_scale.is_none(),
        )
        .with_separator(),
    );
    for &s in scale_presets.iter() {
        let label = format!("{:.0}×", s);
        scale_menu.push(MenuItem::toggle(
            label,
            A::SetUserScale(Some(s)),
            scale_matches(ctx.user_scale, s),
        ));
    }

    use super::visualization::{FalloffSpeed as Fs, OscilloscopeStyle as Os};
    let vopts = ctx.visualizer_options;
    let falloff_menu = vec![
        MenuItem::toggle(
            "Slow",
            A::SetSpectrumFalloff(Fs::Slow),
            vopts.spectrum_falloff == Fs::Slow,
        ),
        MenuItem::toggle(
            "Medium",
            A::SetSpectrumFalloff(Fs::Medium),
            vopts.spectrum_falloff == Fs::Medium,
        ),
        MenuItem::toggle(
            "Fast",
            A::SetSpectrumFalloff(Fs::Fast),
            vopts.spectrum_falloff == Fs::Fast,
        ),
    ];
    let osc_style_menu = vec![
        MenuItem::toggle(
            "Lines",
            A::SetOscilloscopeStyle(Os::Lines),
            vopts.oscilloscope_style == Os::Lines,
        ),
        MenuItem::toggle(
            "Dots",
            A::SetOscilloscopeStyle(Os::Dots),
            vopts.oscilloscope_style == Os::Dots,
        ),
        MenuItem::toggle(
            "Solid",
            A::SetOscilloscopeStyle(Os::Solid),
            vopts.oscilloscope_style == Os::Solid,
        ),
    ];
    let visualizer_menu = vec![
        MenuItem::toggle(
            "Spectrum",
            A::SetVisualizerMode(VisualizerMode::Spectrum),
            ctx.visualizer_mode == VisualizerMode::Spectrum,
        ),
        MenuItem::toggle(
            "Oscilloscope",
            A::SetVisualizerMode(VisualizerMode::Oscilloscope),
            ctx.visualizer_mode == VisualizerMode::Oscilloscope,
        ),
        MenuItem::toggle(
            "Off",
            A::SetVisualizerMode(VisualizerMode::Off),
            ctx.visualizer_mode == VisualizerMode::Off,
        )
        .with_separator(),
        MenuItem::toggle(
            "Spectrum peak hold",
            A::SetSpectrumPeakHold(!vopts.spectrum_peak_hold),
            vopts.spectrum_peak_hold,
        ),
        MenuItem::parent("Spectrum falloff", falloff_menu),
        MenuItem::parent("Oscilloscope style", osc_style_menu),
    ];

    let display_menu = vec![
        MenuItem::parent("Scale", scale_menu),
        MenuItem::parent("Visualizer", visualizer_menu),
        MenuItem::toggle("Mini mode (shade)", A::ToggleShade, ctx.shade_mode).with_separator(),
        MenuItem::toggle("Equalizer", A::ToggleEqualizer, ctx.eq_visible),
        MenuItem::toggle("Playlist", A::TogglePlaylist, ctx.playlist_visible),
        MenuItem::toggle("Always on top", A::ToggleAlwaysOnTop, ctx.always_on_top).with_separator(),
        MenuItem::action("Change skin…", A::PickSkin),
    ];

    let mut output_items = vec![MenuItem::toggle(
        "Default device",
        A::SelectOutputDevice(None),
        ctx.current_output_device.is_none(),
    )];
    for name in ctx.output_devices.iter().take(12) {
        let label = if name.len() > 28 {
            format!("{}…", &name[..27])
        } else {
            name.clone()
        };
        let on = ctx.current_output_device.as_deref() == Some(name.as_str());
        output_items.push(MenuItem::toggle(
            label,
            A::SelectOutputDevice(Some(name.clone())),
            on,
        ));
    }

    let sleep_menu = vec![
        MenuItem::toggle(
            "Off",
            A::SetSleepTimer(None),
            ctx.sleep_timer_minutes.is_none(),
        ),
        MenuItem::toggle(
            "15 minutes",
            A::SetSleepTimer(Some(15)),
            ctx.sleep_timer_minutes == Some(15),
        )
        .with_separator(),
        MenuItem::toggle(
            "30 minutes",
            A::SetSleepTimer(Some(30)),
            ctx.sleep_timer_minutes == Some(30),
        ),
        MenuItem::toggle(
            "60 minutes",
            A::SetSleepTimer(Some(60)),
            ctx.sleep_timer_minutes == Some(60),
        ),
        MenuItem::toggle(
            "90 minutes",
            A::SetSleepTimer(Some(90)),
            ctx.sleep_timer_minutes == Some(90),
        ),
    ];

    use oneamp_core::ReplayGainMode as Rg;
    let replaygain_menu = vec![
        MenuItem::toggle(
            "Off",
            A::SetReplayGainMode(Rg::Off),
            ctx.replaygain_mode == Rg::Off,
        ),
        MenuItem::toggle(
            "Track",
            A::SetReplayGainMode(Rg::Track),
            ctx.replaygain_mode == Rg::Track,
        ),
        MenuItem::toggle(
            "Album",
            A::SetReplayGainMode(Rg::Album),
            ctx.replaygain_mode == Rg::Album,
        ),
        MenuItem::toggle(
            "Auto (album → track)",
            A::SetReplayGainMode(Rg::Auto),
            ctx.replaygain_mode == Rg::Auto,
        ),
    ];

    let audio_menu = vec![
        MenuItem::toggle("Crossfade", A::ToggleCrossfade, ctx.crossfade_enabled),
        MenuItem::parent("ReplayGain", replaygain_menu),
        MenuItem::toggle("Mono", A::ToggleMono, ctx.mono_enabled),
        MenuItem::toggle("Loudness", A::ToggleLoudness, ctx.loudness_enabled),
        MenuItem::toggle(
            "Track notifications",
            A::ToggleTrackNotifications,
            ctx.track_notifications_enabled,
        ),
        MenuItem::toggle(
            "Resume long files",
            A::ToggleResumeLongFiles,
            ctx.resume_long_files,
        )
        .with_separator(),
        MenuItem::parent("Output device", output_items),
        MenuItem::parent("Sleep timer", sleep_menu),
    ];

    let help_menu = vec![
        MenuItem::action("Keyboard shortcuts", A::ShowHotkeys),
        MenuItem::action("Welcome screen…", A::ShowWelcome).with_separator(),
        MenuItem::action("Check for updates", A::CheckForUpdates).with_separator(),
        MenuItem::action("About OneAmp", A::ShowAbout),
    ];

    vec![
        MenuItem::parent("Player", player_menu),
        MenuItem::parent("View", display_menu),
        MenuItem::parent("Audio", audio_menu),
        MenuItem::parent("Help", help_menu),
    ]
}
