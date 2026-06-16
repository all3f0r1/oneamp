//! First-launch welcome screen + Options → Skins… dialog.
//!
//! Two viewports built on the same primitives:
//!   - `Welcome::show` — first-launch dialog with language, scale, crisp
//!     toggle, default-player button, and skin picker. Reopenable via
//!     Help → Welcome screen…
//!   - `SkinsDialog::show_skins_dialog` — the same skin picker on its
//!     own, accessible from the Options menu.
//!
//! Both render skin entries as **image cards** using the bundled
//! `main.bmp` of each `.wsz` (loaded lazily via `SkinThumbnailCache`),
//! so the user picks a skin by appearance, not by filename.

use egui::{Color32, Frame, Margin, RichText, Stroke, Vec2};

use crate::config::LangConfig;
use crate::i18n::Strings;
use crate::platform::default_player;
use crate::skin_thumbnails::SkinThumbnailCache;
use crate::skins::{self, SkinEntry};
use crate::wsz_ui::skin_theme::{DialogTheme, paint_titlebar};
use oneamp_core::wsz::skin::WszSkin;

/// Action emitted by the welcome viewport on a given frame. The app
/// applies these the same way it applies `MainWindowAction`s — so the
/// skin/scale/lang plumbing already in `app/mod.rs` does the heavy
/// lifting and we don't duplicate state mutations here.
#[derive(Debug, Clone)]
pub enum WelcomeAction {
    ApplySkin(SkinEntry),
    ApplyScale(Option<f32>),
    ApplyLang(LangConfig),
    SetAsDefaultPlayer,
    /// User clicked "Get started" — commit `first_run = false` and
    /// close the welcome window.
    Done,
    /// User clicked "Skip". Same effect as `Done` but signals "I didn't
    /// touch anything intentionally".
    Skip,
}

/// Persistent state for the welcome screen across frames. Lives on
/// `OneAmpApp`.
pub struct Welcome {
    pub open: bool,
    /// Outcome of the most recent "Set as default" click — surfaced as a
    /// banner below the button. Reset on every fresh click.
    pub default_player_status: Option<DefaultPlayerStatus>,
    /// Cached skin catalog. Rebuilt when the user changes the skins
    /// folder or hits Rescan.
    pub skins: Vec<SkinEntry>,
    /// Stem of the currently-selected skin (matched against
    /// `SkinEntry::display_name`).
    pub selected_skin_name: Option<String>,
    /// Per-viewport image cache so each skin card shows its `main.bmp`.
    pub thumbs: SkinThumbnailCache,
}

#[derive(Debug, Clone)]
pub enum DefaultPlayerStatus {
    Ok,
    Failed(String),
}

impl Welcome {
    pub fn new(user_skins_dir: Option<&std::path::Path>) -> Self {
        Self {
            open: false,
            default_player_status: None,
            skins: skins::discover(user_skins_dir),
            selected_skin_name: None,
            thumbs: SkinThumbnailCache::new(),
        }
    }

    pub fn rescan(&mut self, user_skins_dir: Option<&std::path::Path>) {
        self.skins = skins::discover(user_skins_dir);
        self.thumbs.clear();
    }
}

/// Scale presets shown on the welcome screen. Kept short on purpose —
/// integer steps cover the common cases; 0.25-step values stay in the
/// titlebar menu for power users.
const WELCOME_SCALE_PRESETS: &[f32] = &[1.0, 2.0, 3.0, 4.0];

fn scale_label(s: f32) -> String {
    if (s - s.round()).abs() < 0.01 {
        format!("{:.0}×", s)
    } else {
        format!("{:.2}×", s)
    }
}

fn scale_matches(a: Option<f32>, b: f32) -> bool {
    a.is_some_and(|v| (v - b).abs() < 0.005)
}

/// Render the welcome screen in a separate OS window. Returns the
/// action(s) the app should apply this frame. `current_lang` /
/// `current_scale` / `current_skin_name` are passed in by the app so
/// the radio rows can light up the active choice.
///
/// Native `pixels_per_point` is locked on the viewport so the dialog
/// renders at the OS-natural size, regardless of the player's ppp
/// override (which is cranked up to magnify the 275-px skin).
#[allow(clippy::too_many_arguments)]
pub fn show(
    welcome: &mut Welcome,
    parent_ctx: &egui::Context,
    s: &Strings,
    current_lang_cfg: LangConfig,
    current_scale: Option<f32>,
    native_ppp: f32,
    current_skin_name: Option<&str>,
    skin: &WszSkin,
) -> Vec<WelcomeAction> {
    if !welcome.open {
        return Vec::new();
    }

    let viewport_id = egui::ViewportId::from_hash_of("oneamp_welcome");
    let inner_size = Vec2::new(480.0, 460.0);
    let mut actions: Vec<WelcomeAction> = Vec::new();
    let theme = DialogTheme::from_skin(skin);
    let title = "OneAmp — Welcome".to_string();

    parent_ctx.show_viewport_immediate(
        viewport_id,
        egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size(inner_size)
            .with_min_inner_size(Vec2::new(420.0, 380.0))
            .with_resizable(true)
            .with_decorations(false)
            .with_active(true),
        |viewport_ctx, _class| {
            apply_native_ppp(viewport_ctx);
            theme.apply_to_ctx(viewport_ctx);

            if viewport_ctx.input(|i| i.viewport().close_requested()) {
                actions.push(WelcomeAction::Skip);
                welcome.open = false;
            }

            let frame = egui::Frame::central_panel(&viewport_ctx.style())
                .inner_margin(Margin::same(0.0))
                .fill(theme.bg);
            egui::CentralPanel::default()
                .frame(frame)
                .show(viewport_ctx, |ui| {
                    let tb = paint_titlebar(ui, &theme, &title, skin);
                    if tb.close_clicked {
                        actions.push(WelcomeAction::Skip);
                        welcome.open = false;
                    }
                    egui::Frame::none()
                        .inner_margin(Margin::same(8.0))
                        .show(ui, |ui| {
                            render_welcome_body(
                                ui,
                                welcome,
                                s,
                                current_lang_cfg,
                                current_scale,
                                native_ppp,
                                current_skin_name,
                                &theme,
                                &mut actions,
                            );
                        });
                });
        },
    );

    actions
}

/// Lock the dialog's ppp to the OS-reported native value so the welcome
/// content renders at native size, decoupled from the player's
/// magnified ppp. Same dance the Skins… dialog uses.
fn apply_native_ppp(ctx: &egui::Context) {
    let native = ctx.native_pixels_per_point().unwrap_or(1.0);
    if (ctx.pixels_per_point() - native).abs() > 0.01 {
        ctx.set_pixels_per_point(native);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_welcome_body(
    ui: &mut egui::Ui,
    welcome: &mut Welcome,
    s: &Strings,
    current_lang_cfg: LangConfig,
    current_scale: Option<f32>,
    native_ppp: f32,
    current_skin_name: Option<&str>,
    theme: &DialogTheme,
    actions: &mut Vec<WelcomeAction>,
) {
    let accent = theme.current;

    // ── Header ─────────────────────────────────────────────────────
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(RichText::new(s.welcome_title()).size(18.0).strong());
    });
    ui.label(
        RichText::new(s.welcome_subtitle())
            .color(Color32::GRAY)
            .small(),
    );
    ui.add_space(4.0);
    horizontal_rule(ui, accent);
    ui.add_space(6.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // ── Language + Scale ─────────────────────────────────
            section_frame(ui, |ui| {
                ui.label(RichText::new(s.language_section()).strong());
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(current_lang_cfg == LangConfig::Auto, s.lang_auto())
                        .clicked()
                    {
                        actions.push(WelcomeAction::ApplyLang(LangConfig::Auto));
                    }
                    if ui
                        .selectable_label(current_lang_cfg == LangConfig::En, s.lang_english())
                        .clicked()
                    {
                        actions.push(WelcomeAction::ApplyLang(LangConfig::En));
                    }
                    if ui
                        .selectable_label(current_lang_cfg == LangConfig::Fr, s.lang_french())
                        .clicked()
                    {
                        actions.push(WelcomeAction::ApplyLang(LangConfig::Fr));
                    }
                });
                ui.add_space(8.0);

                ui.label(RichText::new(s.scale_section()).strong());
                ui.horizontal_wrapped(|ui| {
                    // "Auto" shows what `pick_render_scale` resolves
                    // the OS-reported ppp to — e.g. on a 1.5× compositor
                    // it reads "Auto (2×)" because we always snap to
                    // the nearest integer.
                    let resolved_auto = crate::app::pick_render_scale(native_ppp);
                    let auto_label = format!("{} ({})", s.scale_auto(), scale_label(resolved_auto));
                    if ui
                        .selectable_label(current_scale.is_none(), auto_label)
                        .clicked()
                    {
                        actions.push(WelcomeAction::ApplyScale(None));
                    }
                    for &preset in WELCOME_SCALE_PRESETS {
                        if ui
                            .selectable_label(
                                scale_matches(current_scale, preset),
                                scale_label(preset),
                            )
                            .clicked()
                        {
                            actions.push(WelcomeAction::ApplyScale(Some(preset)));
                        }
                    }
                });
            });

            ui.add_space(6.0);

            // ── Default player ───────────────────────────────────
            section_frame(ui, |ui| {
                ui.label(RichText::new(s.default_player_section()).strong());
                ui.horizontal(|ui| {
                    if ui.button(s.default_player_button()).clicked() {
                        actions.push(WelcomeAction::SetAsDefaultPlayer);
                    }
                });
                match &welcome.default_player_status {
                    Some(DefaultPlayerStatus::Ok) => {
                        ui.label(RichText::new(s.default_player_done()).color(accent).small());
                    }
                    Some(DefaultPlayerStatus::Failed(detail)) => {
                        ui.label(
                            RichText::new(format!("{}\n{}", s.default_player_failed(), detail))
                                .color(Color32::from_rgb(220, 80, 80))
                                .small(),
                        );
                    }
                    None => {}
                }
            });

            ui.add_space(6.0);

            // ── Skin picker ──────────────────────────────────────
            section_frame(ui, |ui| {
                ui.label(RichText::new(s.skins_section()).strong());
                ui.add_space(4.0);
                render_skin_grid(
                    ui,
                    &welcome.skins,
                    &mut welcome.thumbs,
                    &mut welcome.selected_skin_name,
                    current_skin_name,
                    theme,
                    actions,
                );
                ui.add_space(6.0);
                if ui.button(s.skins_browse()).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Winamp skin", &["wsz", "WSZ"])
                        .pick_file()
                {
                    let entry = SkinEntry {
                        name: path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Custom")
                            .to_string(),
                        source: crate::skins::SkinSource::UserDir,
                        payload: crate::skins::SkinPayload::File(path),
                    };
                    welcome.selected_skin_name = Some(entry.display_name().to_string());
                    actions.push(WelcomeAction::ApplySkin(entry));
                }
            });
            ui.add_space(8.0);
        });

    // ── Footer ─────────────────────────────────────────────────────
    horizontal_rule(ui, accent);
    ui.add_space(4.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .add(egui::Button::new(
                RichText::new(s.welcome_done()).size(13.0),
            ))
            .clicked()
        {
            actions.push(WelcomeAction::Done);
            welcome.open = false;
        }
        if ui.button(s.welcome_skip()).clicked() {
            actions.push(WelcomeAction::Skip);
            welcome.open = false;
        }
    });
}

/// Shared skin grid renderer. Each entry is an `ImageButton` showing the
/// skin's `main.bmp` thumbnail (loaded lazily via `thumbs`). Falls back
/// to a text-only label when the `.wsz` failed to parse. Click → push
/// `WelcomeAction::ApplySkin(entry)`.
fn render_skin_grid(
    ui: &mut egui::Ui,
    entries: &[SkinEntry],
    thumbs: &mut SkinThumbnailCache,
    selected_local: &mut Option<String>,
    current_skin_name: Option<&str>,
    theme: &DialogTheme,
    actions: &mut Vec<WelcomeAction>,
) {
    if entries.is_empty() {
        ui.label(RichText::new("No skins discovered.").color(theme.text));
        return;
    }
    // Card sized so the 275×116 main.bmp fits at ~½ scale with a small
    // caption strip beneath. 140×72 image + ~20 px caption = 92 total
    // card height. Width 150 leaves comfortable padding around the
    // 140-px image.
    let card_size = Vec2::new(150.0, 96.0);
    let img_size = Vec2::new(140.0, 60.0); // 275:116 ≈ 2.37, → 140:59
    let avail_w = ui.available_width();
    let cols = ((avail_w / (card_size.x + 8.0)).floor() as usize).max(1);
    egui::Grid::new("oneamp_skin_grid")
        .num_columns(cols)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (i, entry) in entries.iter().enumerate() {
                let selected = current_skin_name == Some(entry.display_name())
                    || selected_local.as_deref() == Some(entry.display_name());
                let clicked =
                    render_skin_card(ui, entry, thumbs, card_size, img_size, selected, theme);
                if clicked {
                    *selected_local = Some(entry.display_name().to_string());
                    actions.push(WelcomeAction::ApplySkin(entry.clone()));
                }
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
}

/// Render one skin card. Lays out a Frame containing the thumbnail
/// image on top and the entry name (file stem) underneath. The Frame's
/// stroke colour reflects the selected state.
fn render_skin_card(
    ui: &mut egui::Ui,
    entry: &SkinEntry,
    thumbs: &mut SkinThumbnailCache,
    card_size: Vec2,
    img_size: Vec2,
    selected: bool,
    theme: &DialogTheme,
) -> bool {
    let accent = theme.current;
    let stroke = if selected {
        Stroke::new(2.0, accent)
    } else {
        Stroke::new(1.0, theme.border)
    };
    let frame = Frame::group(ui.style())
        .stroke(stroke)
        .inner_margin(Margin::same(4.0));
    let response = frame
        .show(ui, |ui| {
            ui.set_min_size(card_size);
            ui.vertical_centered(|ui| {
                let ctx = ui.ctx().clone();
                if let Some(tex) = thumbs.get_or_load(&ctx, entry) {
                    ui.add(
                        egui::Image::new(tex)
                            .fit_to_exact_size(img_size)
                            .rounding(2.0),
                    );
                } else {
                    // Skin failed to parse — placeholder rect.
                    let (rect, _) = ui.allocate_exact_size(img_size, egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, Color32::from_gray(40));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "—",
                        egui::FontId::default(),
                        Color32::DARK_GRAY,
                    );
                }
                ui.label(
                    RichText::new(entry.display_name())
                        .small()
                        .color(if selected {
                            accent
                        } else {
                            Color32::LIGHT_GRAY
                        }),
                );
            });
        })
        .response;
    let response = response.interact(egui::Sense::click());
    response.clicked()
}

fn horizontal_rule(ui: &mut egui::Ui, color: Color32) {
    let rect = ui.available_rect_before_wrap();
    let y = ui.cursor().top();
    ui.painter().line_segment(
        [
            egui::Pos2::new(rect.left() + 4.0, y),
            egui::Pos2::new(rect.right() - 4.0, y),
        ],
        Stroke::new(1.0, color),
    );
    ui.add_space(1.0);
}

fn section_frame<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::group(ui.style())
        .stroke(Stroke::new(1.0, Color32::from_gray(60)))
        .inner_margin(Margin::same(8.0))
        .show(ui, content)
        .inner
}

/// Reusable "set as default" call shared between the welcome screen and
/// the (future) options dialog. Returns the status that the welcome
/// screen renders below its button.
pub fn invoke_set_as_default() -> DefaultPlayerStatus {
    match default_player::set_as_default() {
        Ok(()) => DefaultPlayerStatus::Ok,
        Err(e) => DefaultPlayerStatus::Failed(e.to_string()),
    }
}

// ─── Skins… dialog ─────────────────────────────────────────────────

/// Open state for the Options → Skins… dialog. Lives separately from
/// `Welcome` so the user can pop the dialog up over a running player
/// without involving any first-launch state.
pub struct SkinsDialog {
    pub open: bool,
    pub skins: Vec<SkinEntry>,
    pub selected_skin_name: Option<String>,
    pub thumbs: SkinThumbnailCache,
}

impl SkinsDialog {
    pub fn new(user_skins_dir: Option<&std::path::Path>) -> Self {
        Self {
            open: false,
            skins: skins::discover(user_skins_dir),
            selected_skin_name: None,
            thumbs: SkinThumbnailCache::new(),
        }
    }

    /// Currently unused — the `MainWindowAction::OpenSkinsDialog` entry
    /// that called this was removed along with the Affichage → Skins…
    /// menu item. Kept so a future re-introduction of a "Skins…"
    /// surface can re-route through the same rescan sequence.
    #[allow(dead_code)]
    pub fn rescan(&mut self, user_skins_dir: Option<&std::path::Path>) {
        self.skins = skins::discover(user_skins_dir);
        self.thumbs.clear();
    }
}

/// Render the Options → Skins… dialog in a separate OS window. Returns
/// the per-frame action list; closing the window flips `dialog.open` to
/// false so the caller can react.
pub fn show_skins_dialog(
    dialog: &mut SkinsDialog,
    parent_ctx: &egui::Context,
    s: &Strings,
    current_skin_name: Option<&str>,
    skin: &WszSkin,
) -> Vec<WelcomeAction> {
    if !dialog.open {
        return Vec::new();
    }
    let viewport_id = egui::ViewportId::from_hash_of("oneamp_skins_dialog");
    let inner_size = Vec2::new(520.0, 420.0);
    let mut actions: Vec<WelcomeAction> = Vec::new();
    let theme = DialogTheme::from_skin(skin);
    let title = s.skins_dialog_title().to_string();

    parent_ctx.show_viewport_immediate(
        viewport_id,
        egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size(inner_size)
            .with_min_inner_size(Vec2::new(420.0, 320.0))
            .with_resizable(true)
            .with_decorations(false)
            .with_active(true),
        |viewport_ctx, _class| {
            apply_native_ppp(viewport_ctx);
            theme.apply_to_ctx(viewport_ctx);
            if viewport_ctx.input(|i| i.viewport().close_requested()) {
                dialog.open = false;
            }
            let frame = egui::Frame::central_panel(&viewport_ctx.style())
                .inner_margin(Margin::same(0.0))
                .fill(theme.bg);
            egui::CentralPanel::default()
                .frame(frame)
                .show(viewport_ctx, |ui| {
                    let tb = paint_titlebar(ui, &theme, &title, skin);
                    if tb.close_clicked {
                        dialog.open = false;
                    }
                    egui::Frame::none()
                        .inner_margin(Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.heading(&title);
                            ui.add_space(6.0);
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    render_skin_grid(
                                        ui,
                                        &dialog.skins,
                                        &mut dialog.thumbs,
                                        &mut dialog.selected_skin_name,
                                        current_skin_name,
                                        &theme,
                                        &mut actions,
                                    );
                                    ui.add_space(8.0);
                                    if ui.button(s.skins_browse()).clicked()
                                        && let Some(path) = rfd::FileDialog::new()
                                            .add_filter("Winamp skin", &["wsz", "WSZ"])
                                            .pick_file()
                                    {
                                        let entry = SkinEntry {
                                            name: path
                                                .file_stem()
                                                .and_then(|s| s.to_str())
                                                .unwrap_or("Custom")
                                                .to_string(),
                                            source: crate::skins::SkinSource::UserDir,
                                            payload: crate::skins::SkinPayload::File(path),
                                        };
                                        dialog.selected_skin_name =
                                            Some(entry.display_name().to_string());
                                        actions.push(WelcomeAction::ApplySkin(entry));
                                    }
                                });
                            ui.add_space(6.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button(s.close()).clicked() {
                                        dialog.open = false;
                                    }
                                },
                            );
                        });
                });
        },
    );
    actions
}
