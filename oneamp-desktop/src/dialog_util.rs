//! Helpers shared by every "small form" OS sub-viewport
//! (`url_dialog`, `format_dialog`, `tag_editor_dialog`).
//!
//! Each of those dialogs spawns its own OS window via
//! `Context::show_viewport_immediate`, so the OS reports an
//! initial `inner_size` chosen ahead of layout. Without a follow-up
//! resize we'd have to over-estimate that initial size — and the
//! user would see an empty band at the bottom (and sometimes the
//! right) of the dialog. `fit_viewport_height` measures the actual
//! content extent inside the central panel and re-issues
//! `ViewportCommand::InnerSize` on the first frame so the OS window
//! shrinks to exactly fit the laid-out widgets.
//!
//! Why height-only: the small-form dialogs use
//! `TextEdit::desired_width(ui.available_width())` so their text
//! fields fill whatever width the panel offers — the *content
//! width* is therefore whatever the *panel width* is, which makes
//! per-frame width auto-shrink a no-op. We pick a fixed
//! "comfortable for typing" width per dialog and let the height
//! follow the row count + optional error message + buttons.

use crate::wsz_ui::skin_theme::{DialogTheme, paint_titlebar};
use egui::{Context, Ui, Vec2, ViewportCommand};
use oneamp_core::wsz::skin::WszSkin;

/// Default inner margin around dialog content. Matches what a stock
/// `CentralPanel` would apply if we didn't override its frame.
pub const DIALOG_MARGIN: f32 = 8.0;

/// Pop a native OS error dialog with the player's name as the title
/// and `message` as the body. Routes through `rfd::MessageDialog`, so
/// the backend is `MessageBox` (Win32), `NSAlert` (AppKit) or
/// `xdg-desktop-portal` (Linux — KDE / GNOME / wlroots each render
/// their own native chrome). Blocks the calling thread until the user
/// acknowledges; this is safe to call from `eframe::App::update`
/// because the modal owns the foreground for its lifetime and egui
/// just resumes painting on the next frame once it returns.
pub fn show_error(message: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("OneAmp")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Like [`show_error`] but with the `Info` level — used for the About
/// dialog and similar non-error notices so the OS picks the right
/// icon, sound and ARIA role.
pub fn show_info(title: &str, message: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Tolerance for the "did the content height change" comparison.
/// Half a pixel is far below any visible shift, so this gates out
/// repeated `InnerSize` commands that would otherwise fire every
/// frame and trigger a resize loop on some compositors.
const RESIZE_EPSILON: f32 = 0.5;

/// Measure the laid-out content extent inside `ui` and, if it
/// differs from `last_size_sent`, push a fresh
/// `ViewportCommand::InnerSize(width, content_height + 2 * margin)`
/// to the OS so the window shrinks to fit. `last_size_sent` is
/// updated in place so the next frame is a no-op.
///
/// Call this at the *end* of the central panel's closure (after all
/// widgets have been added to `ui`) — `ui.min_rect()` only reflects
/// the final content rect after the closure has emitted everything.
pub fn fit_viewport_height(
    ctx: &Context,
    ui: &Ui,
    target_w: f32,
    margin: f32,
    last_size_sent: &mut Option<Vec2>,
) {
    let content_h = ui.min_rect().height();
    let target = Vec2::new(target_w, content_h + margin * 2.0);
    let changed = match last_size_sent {
        Some(prev) => {
            (prev.y - target.y).abs() > RESIZE_EPSILON || (prev.x - target.x).abs() > RESIZE_EPSILON
        }
        None => true,
    };
    if changed {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(target));
        *last_size_sent = Some(target);
    }
}

/// Lock the sub-viewport's `pixels_per_point` to the OS-reported
/// native value. The parent player runs at 2-3× ppp to magnify the
/// 275-px skin atlas; without this every dialog would inherit that
/// zoom and render at twice the intended size.
pub fn apply_native_ppp(ctx: &Context) {
    let native = ctx.native_pixels_per_point().unwrap_or(1.0);
    if (ctx.pixels_per_point() - native).abs() > 0.01 {
        ctx.set_pixels_per_point(native);
    }
}

/// Which button the user activated in a confirm/cancel pair, or
/// `None` if neither was clicked this frame. Returned by
/// [`confirm_buttons`] so the call-site can fold the choice into its
/// own `DialogOutcome` without re-deriving the click state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmChoice {
    #[default]
    None,
    Accept,
    Cancel,
}

/// Render a `[Cancel] [Primary]` (or `[Primary] [Cancel]` on Windows)
/// button pair in the OS-conventional order, plus an Escape-to-cancel
/// global key binding. macOS HIG and GNOME HIG both place the primary
/// action on the **right**; Windows places it on the **left**. The
/// helper produces no leading spacer — callers should render any side
/// buttons (e.g. "Reset to default") *before* this call inside the
/// same `ui.horizontal` so the OS pair stays pinned to the trailing
/// edge.
///
/// Why Enter is **not** handled here: text fields legitimately consume
/// Enter — multiline edits insert a newline, singleline edits emit
/// `lost_focus()` only after Enter — so each dialog decides when
/// Enter == Accept based on its own focus state and field shape. The
/// `Escape == Cancel` binding is global because Escape has no
/// in-field meaning we care about.
pub fn confirm_buttons(ui: &mut Ui, accept_label: &str) -> ConfirmChoice {
    let escape = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
    let mut choice = ConfirmChoice::None;

    if cfg!(target_os = "windows") {
        if ui.button(accept_label).clicked() {
            choice = ConfirmChoice::Accept;
        }
        if ui.button("Cancel").clicked() {
            choice = ConfirmChoice::Cancel;
        }
    } else {
        if ui.button("Cancel").clicked() {
            choice = ConfirmChoice::Cancel;
        }
        if ui.button(accept_label).clicked() {
            choice = ConfirmChoice::Accept;
        }
    }

    if choice == ConfirmChoice::None && escape {
        choice = ConfirmChoice::Cancel;
    }
    choice
}

/// Result of a single modal-dialog frame. Generic over the payload
/// the dialog returns on a positive acknowledgement so each dialog
/// can ship its own (`String` for URL / format template, `usize`
/// for the tag editor's playlist row, …) without re-declaring the
/// surrounding `None` / `Cancelled` boilerplate.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum DialogOutcome<T> {
    /// No interaction this frame — keep the dialog alive.
    #[default]
    None,
    /// User closed the dialog via Cancel / window close.
    Cancelled,
    /// User confirmed (Save / Open / Submit). Carries the payload
    /// the caller needs to apply the change.
    Accepted(T),
}

/// Trait implemented by every modal sub-viewport dialog
/// (`format_dialog`, `url_dialog`, `tag_editor_dialog`). Provides
/// the default `show()` impl that wires up the viewport setup, ppp
/// lock, close-request handling, central-panel framing and the
/// post-layout auto-fit pass. Implementors only describe their
/// dialog (id / title / target width / initial height) and supply
/// the per-frame body via `render_body`.
///
/// Why a trait and not a free function: each dialog needs to carry
/// its own state (text fields, error message, last_size_sent
/// cache) across frames. A trait lets the dialog struct own that
/// state while still sharing the boilerplate via a default method.
pub trait DialogView {
    /// Payload returned to the caller on Save/Submit. The full
    /// outcome enum is `DialogOutcome<Self::Payload>`.
    type Payload;

    /// Stable hash used as the `ViewportId` for this dialog. Must
    /// be unique across all simultaneously-openable dialogs so egui
    /// doesn't collapse two of them into the same OS window.
    fn viewport_hash(&self) -> &'static str;

    /// Title displayed in the OS title bar. Allocated as a `String`
    /// so the tag editor can interpolate the current filename.
    fn title(&self) -> String;

    /// Fixed content width in egui units. The dialog is OS-resizable
    /// but the auto-fit pass always snaps width back to this value
    /// (see `fit_viewport_height`).
    fn target_width(&self) -> f32;

    /// Starting height for the first frame, before the auto-fit pass
    /// measures the real content extent. A generous overshoot is
    /// fine — `fit_viewport_height` shrinks it on the very next
    /// command flush.
    fn initial_height(&self) -> f32;

    /// Mutable handle to the cached last-requested `InnerSize`. The
    /// cache lives on the dialog struct so the auto-fit pass can
    /// short-circuit redundant `InnerSize` commands.
    fn last_size_sent(&mut self) -> &mut Option<Vec2>;

    /// Per-frame body rendering. Called inside the central panel
    /// after the close-request check. Implementors set
    /// `*outcome = DialogOutcome::Accepted(...)` on Save / Submit.
    /// Cancel is wired up by the default `show()` impl.
    fn render_body(&mut self, ui: &mut Ui, outcome: &mut DialogOutcome<Self::Payload>);

    /// Spawn the OS sub-viewport, drive one frame and return what
    /// the user did this frame. The default impl factors out the
    /// 30-odd lines of viewport setup that used to be copy-pasted
    /// across every dialog.
    ///
    /// `skin` drives the painted Winamp-style chrome (title bar palette,
    /// body widget colours). The OS decorations are disabled so the
    /// painted title bar isn't a second draggable strip below the real
    /// one — drag-to-move is wired up via [`ViewportCommand::StartDrag`]
    /// from inside [`paint_titlebar`].
    fn show(&mut self, parent_ctx: &Context, skin: &WszSkin) -> DialogOutcome<Self::Payload> {
        let mut outcome: DialogOutcome<Self::Payload> = DialogOutcome::None;

        let target_w = self.target_width();
        let initial_h = self.initial_height();
        let title = self.title();
        let viewport_id = egui::ViewportId::from_hash_of(self.viewport_hash());
        let initial = self
            .last_size_sent()
            .unwrap_or(Vec2::new(target_w, initial_h));
        let theme = DialogTheme::from_skin(skin);

        parent_ctx.show_viewport_immediate(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title(title.clone())
                .with_inner_size(initial)
                .with_resizable(true)
                .with_decorations(false)
                .with_active(true),
            |viewport_ctx, _class| {
                apply_native_ppp(viewport_ctx);
                theme.apply_to_ctx(viewport_ctx);

                if viewport_ctx.input(|i| i.viewport().close_requested()) {
                    outcome = DialogOutcome::Cancelled;
                }

                // Drop the central panel's default margin: the title bar
                // wants to paint flush against the window edges, then
                // `inner_panel_frame` re-introduces the body margin.
                let frame = egui::Frame::central_panel(&viewport_ctx.style())
                    .inner_margin(egui::Margin::same(0.0))
                    .fill(theme.bg);
                egui::CentralPanel::default()
                    .frame(frame)
                    .show(viewport_ctx, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let tb = paint_titlebar(ui, &theme, &title, skin);
                        if tb.close_clicked {
                            outcome = DialogOutcome::Cancelled;
                        }

                        // Body lives in an inset child UI so widgets get
                        // the comfortable DIALOG_MARGIN gutter that the
                        // outer frame no longer provides.
                        let body_frame =
                            egui::Frame::none().inner_margin(egui::Margin::same(DIALOG_MARGIN));
                        body_frame.show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            self.render_body(ui, &mut outcome);
                        });

                        // `margin = 0` — the outer central panel has no
                        // inner margin (so the titlebar paints flush) and
                        // the body's own `Frame::none().inner_margin(...)`
                        // already lives *inside* `ui.min_rect()`. Adding
                        // another `2 × DIALOG_MARGIN` here would
                        // double-count the bottom padding and leave a
                        // visible empty band under the buttons row.
                        fit_viewport_height(viewport_ctx, ui, target_w, 0.0, self.last_size_sent());
                    });
            },
        );

        outcome
    }
}
