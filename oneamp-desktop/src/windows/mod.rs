//! Window management module
//!
//! Coordinates all WSZ windows (main, playlist, equalizer, shade).

use eframe::egui;
use egui::Pos2;
use oneamp_core::wsz::skin::WszSkin;

pub use super::wsz_ui::cursor::{CursorOverlay, HotArea};
pub use super::wsz_ui::main_window::MainWindowAction;
pub use super::wsz_ui::playlist_window::PlaylistAction;
pub use super::wsz_ui::{EqualizerWindow, PlaylistWindow, ShadeWindow, WszMainWindow};

/// Which sub-window currently holds soft focus. All three docked sub-windows
/// share a single OS viewport, so the OS-level focus signal is the same for
/// all of them. We layer a soft-focus model on top: whichever sub-area the
/// user clicked in last becomes "active". The titlebars use this to draw
/// their active vs inactive strip, mirroring Winamp where only the focused
/// window shows the bright title text and the others dim out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveSubWindow {
    Main,
    Equalizer,
    Playlist,
}

/// Coordinator-level window state. Layout and docking are not implemented
/// yet, so this only tracks the rendering scale and shade-mode toggle.
#[derive(Debug, Clone)]
pub struct WindowState {
    pub scale: f32,
    pub shade_mode: bool,
    pub custom_chrome: bool,
}

impl WindowState {
    pub fn new(scale: f32, custom_chrome: bool) -> Self {
        Self {
            scale,
            shade_mode: false,
            custom_chrome,
        }
    }

    pub fn toggle_shade(&mut self) {
        self.shade_mode = !self.shade_mode;
    }
}

/// Actions collected from windows in one update cycle.
pub struct WindowActions {
    pub main: Option<MainWindowAction>,
    pub playlist: PlaylistAction,
}

/// Coordinates all WSZ windows
pub struct WszWindowCoordinator {
    main_window: WszMainWindow,
    playlist_window: Option<PlaylistWindow>,
    equalizer_window: Option<EqualizerWindow>,
    shade_window: Option<ShadeWindow>,

    window_state: WindowState,
    show_playlist: bool,
    show_equalizer: bool,
    /// Soft-focus tracker. Defaults to `Main` so a fresh launch matches
    /// Winamp, where the player takes focus instead of the playlist. Updated
    /// on left-button-press based on which sub-area the pointer lands in.
    active_subwindow: ActiveSubWindow,
    /// Last viewport size we requested via `ViewportCommand::InnerSize`. Used
    /// to detect when docked sub-windows (EQ, etc.) toggle visibility so we
    /// only resize when the target actually changes.
    last_viewport_size: Option<egui::Vec2>,
    /// Owns the cached cursor textures + paints the active skin's `.cur`
    /// at the pointer position each frame. Empty when the skin ships no
    /// cursors — falls back to the OS arrow then.
    cursor_overlay: CursorOverlay,
}

impl WszWindowCoordinator {
    /// Build a coordinator with an initial equalizer state (used so the EQ
    /// window can render the persisted curve from `AppConfig` immediately).
    pub fn with_initial(
        skin: &WszSkin,
        scale: f32,
        custom_chrome: bool,
        eq_gains: &[f32],
        eq_preamp_db: f32,
        eq_enabled: bool,
    ) -> Self {
        let window_state = WindowState::new(scale, custom_chrome);

        Self {
            main_window: WszMainWindow::new(skin.clone(), scale, custom_chrome),
            playlist_window: Some(PlaylistWindow::new(skin.clone(), scale)),
            equalizer_window: Some(EqualizerWindow::with_gains(
                skin.clone(),
                scale,
                eq_gains,
                eq_preamp_db,
                eq_enabled,
            )),
            shade_window: Some(ShadeWindow::new(skin.clone(), scale)),
            window_state,
            show_playlist: false,
            show_equalizer: false,
            active_subwindow: ActiveSubWindow::Main,
            last_viewport_size: None,
            cursor_overlay: CursorOverlay::new(scale),
        }
    }

    /// Bundle of actions emitted in a single `update` cycle.
    #[allow(clippy::too_many_arguments)]
    pub fn collect_actions(
        &mut self,
        ctx: &egui::Context,
        audio_engine: Option<&oneamp_core::AudioEngine>,
        spectrum_data: &[f32],
        waveform_data: &oneamp_core::WaveformSnapshot,
        meter_data: &oneamp_core::MeterSnapshot,
        events: &[oneamp_core::AudioEvent],
        playlist_entries: &[oneamp_core::PlaylistEntry],
        playlist_current: Option<usize>,
        playlist_selected: &std::collections::BTreeSet<usize>,
        playlist_queued: &[Option<usize>],
        shuffle_on: bool,
        repeat_on: bool,
        always_on_top: bool,
        crossfade_enabled: bool,
        replaygain_enabled: bool,
        replaygain_mode: oneamp_core::ReplayGainMode,
        mono_enabled: bool,
        loudness_enabled: bool,
        track_notifications_enabled: bool,
        output_devices: Vec<String>,
        current_output_device: Option<String>,
        recent_paths: Vec<std::path::PathBuf>,
        user_scale: Option<f32>,
        sleep_timer_minutes: Option<u32>,
        stop_after_current: bool,
        resume_long_files: bool,
        user_eq_presets: Vec<oneamp_core::equalizer_presets::EqualizerPreset>,
        playlist_display_format: &str,
        visualizer_options: crate::wsz_ui::components::visualization::VisualizerOptions,
    ) -> WindowActions {
        // Forward audio events to windows that need playback state
        if !events.is_empty() {
            self.main_window.update(events);
            if let Some(ref mut shade) = self.shade_window {
                shade.update(events);
            }
            if let Some(ref mut eq) = self.equalizer_window {
                eq.update(events);
            }
            if let Some(ref mut pl) = self.playlist_window {
                pl.update(events);
            }
        }

        // Soft-focus model: pick whichever sub-area the user just clicked
        // in as the active sub-window. All three sub-windows share one OS
        // viewport, so the OS focus signal alone can't tell them apart —
        // we track click position instead. Default stays Main, matching
        // Winamp where the player owns focus until the user interacts
        // with another sub-window.
        self.update_active_subwindow(ctx);
        // Soft-focus is PURELY position-based, never gated by the OS
        // focus signal. The comment in `main_window/mod.rs::show` already
        // documents why: undecorated/transparent viewports on Wayland
        // (and some X11 setups) report `i.focused == false` permanently,
        // and AND-ing that bit here would force every sub-window's
        // titlebar to stay inactive forever even though the click-routing
        // model is already telling us which area the user is interacting
        // with. We want the active-strip feedback to track which sub-area
        // the user last clicked, full stop.
        let main_focused = self.active_subwindow == ActiveSubWindow::Main;
        let eq_focused = self.active_subwindow == ActiveSubWindow::Equalizer;
        let playlist_focused = self.active_subwindow == ActiveSubWindow::Playlist;
        self.main_window.set_soft_focused(main_focused);
        if let Some(ref mut eq) = self.equalizer_window {
            eq.set_focused(eq_focused);
        }
        if let Some(ref mut pl) = self.playlist_window {
            pl.set_focused(playlist_focused);
        }

        // Push the latest user-preset list into the EQ window once
        // per frame so a "Save as preset…" the user just submitted
        // shows up in the dropdown immediately on the next paint.
        if let Some(eq) = self.equalizer_window.as_mut() {
            eq.set_user_presets(user_eq_presets);
        }

        // Mirror app-level toggles into the main window's toggle button states
        // every frame — cheap, and keeps shuffle/repeat/EQ/PL highlight in sync
        // regardless of where they were toggled from (button click, keyboard).
        self.main_window.set_shuffle(shuffle_on);
        self.main_window.set_repeat_on(repeat_on);
        self.main_window.set_equalizer_visible(self.show_equalizer);
        self.main_window.set_playlist_visible(self.show_playlist);
        self.main_window.set_always_on_top(always_on_top);
        self.main_window.set_crossfade_enabled(crossfade_enabled);
        self.main_window.set_replaygain_enabled(replaygain_enabled);
        self.main_window.set_loudness_enabled(loudness_enabled);
        self.main_window
            .set_track_notifications_enabled(track_notifications_enabled);
        self.main_window.set_visualizer_options(visualizer_options);
        self.main_window
            .set_output_device_state(output_devices.clone(), current_output_device.clone());
        self.main_window.set_recent_paths(recent_paths.clone());

        // Snapshot every menu-driven flag in one shot so the titlebar
        // menu's `build_menu_items` reads consistent state for the frame
        // it paints.
        // Whether the active skin's bundled TTF is loaded — the menu
        // uses it for label rendering when present, mirroring the
        // playlist row font choice. Computed from the main window's
        // skin so the menu visually matches the rest of the player.
        let skin_font_available = self.main_window.skin().font_data.is_some();
        self.main_window
            .set_menu_context(crate::wsz_ui::components::MenuContext {
                always_on_top,
                crossfade_enabled,
                replaygain_mode,
                mono_enabled,
                loudness_enabled,
                track_notifications_enabled,
                shade_mode: self.window_state.shade_mode,
                eq_visible: self.show_equalizer,
                playlist_visible: self.show_playlist,
                visualizer_mode: self.main_window.visualizer_mode(),
                visualizer_options,
                user_scale,
                output_devices,
                current_output_device,
                recent_paths,
                sleep_timer_minutes,
                stop_after_current,
                resume_long_files,
                skin_font_available,
            });

        // Always show main window
        let mut main_action =
            self.main_window
                .show(ctx, audio_engine, spectrum_data, waveform_data, meter_data);
        let mut playlist_action = PlaylistAction::None;

        // Show shade window if in shade mode
        if self.window_state.shade_mode {
            if let Some(ref mut shade) = self.shade_window {
                shade.show(ctx, audio_engine);
            }
        } else {
            // Sub-windows dock under the main player in the order EQ → Playlist.
            // Each one shifts the next dock_y down by its own height (which
            // shrinks to 14 px when shaded), so a shaded EQ doesn't leave a
            // dead band before the playlist.
            let mut next_dock_y = 116u32;
            if self.show_equalizer
                && let Some(ref mut win) = self.equalizer_window
            {
                match win.show(ctx, audio_engine, next_dock_y) {
                    Some(super::wsz_ui::equalizer_window::EqualizerAction::Close) => {
                        self.show_equalizer = false;
                    }
                    Some(super::wsz_ui::equalizer_window::EqualizerAction::SaveAsUserPreset) => {
                        // Surface the EQ-side intent up the action
                        // pipe. The app handles `OpenSavePresetDialog`
                        // by popping the modal name prompt and (on
                        // accept) writing into `PresetManager`. We
                        // prefer overwriting any main_action set this
                        // frame because the user's most recent
                        // explicit click was inside the EQ dropdown.
                        main_action = Some(
                            super::wsz_ui::main_window::MainWindowAction::OpenSavePresetDialog,
                        );
                    }
                    None => {}
                }
                next_dock_y += win.height_skin();
            }
            if self.show_playlist
                && let Some(ref mut win) = self.playlist_window
            {
                playlist_action = win.show(
                    ctx,
                    next_dock_y,
                    audio_engine,
                    playlist_entries,
                    playlist_current,
                    playlist_selected,
                    playlist_queued,
                    playlist_display_format,
                );
                if matches!(playlist_action, PlaylistAction::Close) {
                    self.show_playlist = false;
                    // Don't propagate Close to the app — it's handled here.
                    playlist_action = PlaylistAction::None;
                }
            }
        }

        // Resize the OS viewport to fit whatever sub-windows are docked
        // below the main player. Done once per frame so both opening and
        // closing the EQ are reflected immediately. We send the cmd only
        // when the target changes — Wayland in particular is sensitive to
        // redundant resize requests.
        //
        // We must keep `min ≤ max` at every intermediate state. The window
        // starts non-resizable (main.rs pins min=max=275×116), so opening
        // the playlist would push min above the still-pinned max and trip
        // wl_surface protocol error 4 ("Invalid min/max size"). Order:
        // when growing, raise max first; when shrinking, drop min first.
        let target_size = self.target_viewport_size();
        if self.last_viewport_size != Some(target_size) {
            let growing = match self.last_viewport_size {
                Some(prev) => target_size.x > prev.x || target_size.y > prev.y,
                None => true,
            };
            if growing {
                ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(target_size));
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(target_size));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(target_size));
                ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(target_size));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target_size));
            self.last_viewport_size = Some(target_size);
        }

        WindowActions {
            main: main_action,
            playlist: playlist_action,
        }
    }

    /// Watch for a press-just-started this frame and route it to the
    /// sub-window the pointer lands in. Reads from `egui`'s pointer state
    /// before any sub-window's `show()` runs, so the focus update lands on
    /// the same frame the user clicks. Clicks outside any sub-area leave
    /// the active sub-window unchanged so dragging the OS title bar or
    /// clicking the desktop doesn't bounce focus around.
    fn update_active_subwindow(&mut self, ctx: &egui::Context) {
        let just_pressed = ctx.input(|i| i.pointer.primary_pressed());
        if !just_pressed {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let scale = self.window_state.scale;
        let main_h = if self.window_state.shade_mode {
            14.0
        } else {
            116.0
        };
        let window_w = 275.0 * scale;
        if pos.x < 0.0 || pos.x > window_w {
            return;
        }

        // Main window y range starts at 0.
        let main_y_end = main_h * scale;
        if pos.y < main_y_end {
            self.active_subwindow = ActiveSubWindow::Main;
            return;
        }

        // Sub-windows are docked under main in the order EQ → Playlist.
        if !self.window_state.shade_mode {
            let mut next_top = main_y_end;
            if self.show_equalizer {
                let eq_h = self
                    .equalizer_window
                    .as_ref()
                    .map(|w| w.height_skin())
                    .unwrap_or(116);
                let eq_y_end = next_top + (eq_h as f32) * scale;
                if pos.y < eq_y_end {
                    self.active_subwindow = ActiveSubWindow::Equalizer;
                    return;
                }
                next_top = eq_y_end;
            }
            if self.show_playlist {
                let pl_h = self
                    .playlist_window
                    .as_ref()
                    .map(|w| w.height_skin())
                    .unwrap_or(232);
                let pl_y_end = next_top + (pl_h as f32) * scale;
                if pos.y < pl_y_end {
                    self.active_subwindow = ActiveSubWindow::Playlist;
                }
            }
        }
    }

    /// Compute the OS viewport size needed to fit the main window plus any
    /// docked sub-windows. All sizes are in egui logical points.
    fn target_viewport_size(&self) -> egui::Vec2 {
        let scale = self.window_state.scale;
        let main_h = if self.window_state.shade_mode {
            14.0
        } else {
            116.0
        };
        let mut extra_h = 0.0;
        // Tracks how far below the EQ window the preset dropdown
        // would draw when open. Reported in skin units relative to
        // the EQ window's bottom edge.
        let mut eq_menu_overflow = 0u32;
        if !self.window_state.shade_mode {
            if self.show_equalizer {
                let eq_h = self
                    .equalizer_window
                    .as_ref()
                    .map(|w| w.height_skin())
                    .unwrap_or(116);
                extra_h += eq_h as f32;
                eq_menu_overflow = self
                    .equalizer_window
                    .as_ref()
                    .map(|w| w.preset_menu_overlay_extra_skin())
                    .unwrap_or(0);
            }
            if self.show_playlist {
                let pl_h = self
                    .playlist_window
                    .as_ref()
                    .map(|w| w.height_skin())
                    .unwrap_or(232);
                extra_h += pl_h as f32;
            }
        }
        // When the EQ preset dropdown is open, the viewport needs to
        // be tall enough to contain it. The dropdown overflows the
        // EQ floor by `eq_menu_overflow` skin units; any pixels the
        // playlist (or other docked sub-windows) already provide
        // count toward that — we only enlarge if the existing total
        // isn't already enough.
        let nominal_total = main_h + extra_h;
        let menu_required_total = if eq_menu_overflow > 0 {
            // EQ docks at y = main_h. Its floor sits at main_h + eq_h.
            // The menu extends `eq_menu_overflow` past that floor, so
            // the OS viewport needs to be at least menu_bottom tall.
            let eq_h = self
                .equalizer_window
                .as_ref()
                .map(|w| w.height_skin() as f32)
                .unwrap_or(116.0);
            main_h + eq_h + eq_menu_overflow as f32
        } else {
            0.0
        };
        let total = nominal_total.max(menu_required_total);
        egui::Vec2::new(275.0 * scale, total * scale)
    }

    /// Toggle playlist window visibility
    pub fn toggle_playlist(&mut self) {
        self.show_playlist = !self.show_playlist;
    }

    /// Current visibility of the docked playlist sub-window. Mirrors
    /// `show_playlist` so the app can persist it without poking internals.
    pub fn is_playlist_visible(&self) -> bool {
        self.show_playlist
    }

    /// Force the playlist visibility — used at boot to restore the
    /// previous session's docked layout.
    pub fn set_playlist_visible(&mut self, on: bool) {
        self.show_playlist = on;
    }

    /// Current visibility of the docked equalizer sub-window.
    pub fn is_equalizer_visible(&self) -> bool {
        self.show_equalizer
    }

    /// Force the equalizer visibility — used at boot to restore the
    /// previous session's docked layout.
    pub fn set_equalizer_visible(&mut self, on: bool) {
        self.show_equalizer = on;
    }

    /// Briefly highlight the EQ band the user picked by Shift+clicking
    /// the spectrum visualiser. The paint code reads
    /// `flash_band` and renders a glow around that slider for ~1.5 s.
    /// Idempotent — calling it again resets the timer.
    pub fn flash_eq_band(&mut self, band: usize) {
        if let Some(eq) = self.equalizer_window.as_mut() {
            eq.flash_band(band);
        }
    }

    /// True when the main window is in compact (14-px) shade mode. Read
    /// at exit so the next launch can reopen in the same mode.
    pub fn is_shade_mode(&self) -> bool {
        self.window_state.shade_mode
    }

    /// Force shade mode at boot.
    pub fn set_shade_mode(&mut self, on: bool) {
        self.window_state.shade_mode = on;
    }

    /// Borrow the main window so the app can poke at its visualiser mode
    /// for persistence. Kept narrow on purpose — most app-level concerns
    /// go through `MainWindowAction` instead.
    pub fn main_window_mut(&mut self) -> &mut WszMainWindow {
        &mut self.main_window
    }

    /// Borrow the active skin. Used by sub-viewport dialogs to derive
    /// their painted title-bar palette and body theme without cloning
    /// the skin or duplicating ownership on `OneAmpApp`.
    pub fn active_skin(&self) -> &WszSkin {
        self.main_window.skin()
    }

    /// Snapshot of the EQ window's currently-applied preset name, if
    /// any. Returns `None` when the EQ has been edited manually since
    /// the last preset apply, when no EQ window exists (shouldn't
    /// happen post-init), or when the user hasn't picked a preset
    /// this session. Polled by the app each frame so the persisted
    /// `EqualizerConfig::current_preset` field stays in sync.
    pub fn equalizer_current_preset(&self) -> Option<&str> {
        self.equalizer_window
            .as_ref()
            .and_then(|w| w.current_preset())
    }

    /// Push the persisted preset name back into the EQ window at
    /// boot. Mirrors what was saved last session so the dropdown's
    /// "currently selected" indicator survives a relaunch.
    pub fn set_equalizer_current_preset(&mut self, name: Option<String>) {
        if let Some(w) = self.equalizer_window.as_mut() {
            w.set_current_preset(name);
        }
    }

    /// Whether the playlist sub-window is currently the soft-focus target
    /// AND visible — the only situation where type-to-jump should
    /// hijack bare-letter keys. Used by `app::OneAmpApp` to switch
    /// bare-letter handling between transport hotkeys (main focused)
    /// and playlist navigation (playlist focused).
    pub fn is_playlist_focused(&self) -> bool {
        self.show_playlist && self.active_subwindow == ActiveSubWindow::Playlist
    }

    /// Drop the `last_viewport_size` cache so the next `collect_actions`
    /// pass re-sends `Min/Max/InnerSize` even if the egui-unit target
    /// hasn't changed. Used by the app when `pixels_per_point` flips:
    /// the physical size depends on ppp × egui-units, so a ppp change
    /// requires a fresh resize round even though the egui-unit target
    /// is unchanged.
    pub fn invalidate_viewport_cache(&mut self) {
        self.last_viewport_size = None;
    }

    /// Toggle equalizer window visibility
    pub fn toggle_equalizer(&mut self) {
        self.show_equalizer = !self.show_equalizer;
    }

    /// Toggle shade mode
    pub fn toggle_shade(&mut self) {
        self.window_state.toggle_shade();
    }

    /// Update skin for all windows
    pub fn update_skin(&mut self, skin: &WszSkin) {
        let scale = self.window_state.scale;
        let custom_chrome = self.window_state.custom_chrome;
        self.main_window = WszMainWindow::new(skin.clone(), scale, custom_chrome);
        self.playlist_window = Some(PlaylistWindow::new(skin.clone(), scale));
        self.equalizer_window = Some(EqualizerWindow::new(skin.clone(), scale));
        self.shade_window = Some(ShadeWindow::new(skin.clone(), scale));
        // Cursor textures captured a previous skin's bitmaps — drop them
        // so the next paint rebuilds from the new skin's `.cur` files.
        self.cursor_overlay.clear();
    }

    /// Map a screen-space pointer position to the matching `HotArea` based
    /// on the current window layout. Coarse — bounding rects from
    /// `WSZ_FORMAT.md`, no per-pixel region masks. Areas not covered fall
    /// through to `HotArea::Normal` so the cursor stays sensible by
    /// default.
    pub fn pick_hot_area(&self, pointer: Pos2) -> HotArea {
        let scale = self.window_state.scale;
        let to_skin = |abs: Pos2, top_y_skin: u32| -> Option<(i32, i32)> {
            let local_x = (abs.x / scale).round() as i32;
            let local_y = ((abs.y / scale) - top_y_skin as f32).round() as i32;
            Some((local_x, local_y))
        };

        // Main window occupies skin-y 0..main_h.
        let main_h = if self.window_state.shade_mode {
            14
        } else {
            116
        };
        if let Some((x, y)) = to_skin(pointer, 0)
            && (0..275).contains(&x)
            && (0..main_h).contains(&y)
        {
            return main_window_hot_area(x, y, self.window_state.shade_mode);
        }

        // Sub-windows are docked below the main window in the order
        // EQ → Playlist (matching `collect_actions`). When shaded, each
        // shrinks to its 14-px strip.
        let mut next_top = main_h as u32;
        if !self.window_state.shade_mode && self.show_equalizer {
            let eq_h = self
                .equalizer_window
                .as_ref()
                .map(|w| w.height_skin())
                .unwrap_or(116);
            if let Some((x, y)) = to_skin(pointer, next_top)
                && (0..275).contains(&x)
                && (0..eq_h as i32).contains(&y)
            {
                return eq_window_hot_area(x, y, eq_h == 14);
            }
            next_top += eq_h;
        }
        if !self.window_state.shade_mode && self.show_playlist {
            let pl_h = self
                .playlist_window
                .as_ref()
                .map(|w| w.height_skin())
                .unwrap_or(232);
            if let Some((x, y)) = to_skin(pointer, next_top)
                && (0..275).contains(&x)
                && (0..pl_h as i32).contains(&y)
            {
                return playlist_window_hot_area(x, y, pl_h, pl_h == 14);
            }
        }

        HotArea::Normal
    }

    /// Hide the system cursor (when the skin ships any `.cur`) and paint
    /// the matching sprite at the pointer position. Called once per
    /// frame, after `collect_actions`, so the cursor sits on top of every
    /// window's content. No-op when the active skin has no cursors —
    /// the user keeps their OS arrow.
    pub fn paint_cursor(&mut self, ctx: &egui::Context) {
        if self.main_window.skin().cursors.is_empty() {
            return;
        }
        let pointer = match ctx.input(|i| i.pointer.latest_pos()) {
            Some(p) => p,
            None => return,
        };
        let area = self.pick_hot_area(pointer);
        let skin = self.main_window.skin();
        self.cursor_overlay.paint(ctx, skin, area);
    }

    /// Handle keyboard shortcuts for windows
    pub fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // Alt+E: Toggle playlist
            if i.modifiers.alt && i.key_pressed(egui::Key::E) {
                self.toggle_playlist();
            }

            // Alt+G: Toggle equalizer
            if i.modifiers.alt && i.key_pressed(egui::Key::G) {
                self.toggle_equalizer();
            }

            // Alt+M: Toggle shade mode
            if i.modifiers.alt && i.key_pressed(egui::Key::M) {
                self.toggle_shade();
            }
        });
    }
}

/// Pick a `HotArea` for a pointer at skin-space `(x, y)` inside the main
/// window. `shade` is true when the main window is in 14-px shade mode —
/// the slider/visualisation areas are gone, only title bar + shade
/// controls remain.
fn main_window_hot_area(x: i32, y: i32, shade: bool) -> HotArea {
    if shade {
        // Spec WSZ §titlebar.bmp: shade-mode close at (264,3,9,9), shade
        // toggle at (254,3,9,9), minimize at (244,3,9,9). Position bar in
        // shade mode lives at (16,3,210,7); tracker uses `WsPosBar`.
        if hit(x, y, 244, 3, 9, 9) {
            return HotArea::Min;
        }
        if hit(x, y, 254, 3, 9, 9) {
            return HotArea::WinBut;
        }
        if hit(x, y, 264, 3, 9, 9) {
            return HotArea::Close;
        }
        return HotArea::Normal;
    }

    // Title bar buttons (top 14 px). Reading order keeps the buttons
    // ahead of the catch-all "anywhere on the title bar" rule below.
    if (0..14).contains(&y) {
        if hit(x, y, 6, 3, 9, 9) {
            return HotArea::MainMenu;
        }
        if hit(x, y, 244, 3, 9, 9) {
            return HotArea::Min;
        }
        if hit(x, y, 254, 3, 9, 9) {
            return HotArea::WinBut;
        }
        if hit(x, y, 264, 3, 9, 9) {
            return HotArea::Close;
        }
        return HotArea::TitleBar;
    }

    // Clutterbar `O` (Options). The strip is at (10,22,8,43) but only
    // the topmost 8×8 slot is the menu trigger; the rest of the strip
    // stays Normal until the other letters are wired up.
    if hit(x, y, 10, 22, 8, 8) {
        return HotArea::MainMenu;
    }

    if hit(x, y, 16, 72, 248, 10) {
        return HotArea::PosBar;
    }
    if hit(x, y, 107, 57, 68, 13) {
        return HotArea::VolBar;
    }
    if hit(x, y, 177, 57, 38, 13) {
        return HotArea::VolBal;
    }
    if hit(x, y, 112, 27, 154, 6) {
        return HotArea::SongName;
    }

    HotArea::Normal
}

/// Pick a `HotArea` for a pointer at sub-window-local `(x, y)` inside the
/// equalizer window. `shade` collapses every region except the title
/// strip's close button.
fn eq_window_hot_area(x: i32, y: i32, shade: bool) -> HotArea {
    // Close button shares the same skin coords in normal + shade mode.
    if hit(x, y, 264, 3, 9, 9) {
        return HotArea::EqClose;
    }
    if shade {
        return HotArea::EqNormal;
    }
    // Title bar drag area.
    if (0..14).contains(&y) {
        return HotArea::EqTitle;
    }
    // 11 vertical sliders (preamp + 10 bands) at y=38..101, 14×63 each.
    let track_x: [u32; 11] = [21, 78, 96, 114, 132, 150, 168, 186, 204, 222, 240];
    if (38..101).contains(&y) {
        for &tx in track_x.iter() {
            if hit(x, y, tx, 38, 14, 63) {
                return HotArea::EqSlid;
            }
        }
    }
    HotArea::EqNormal
}

/// Pick a `HotArea` for a pointer at sub-window-local `(x, y)` inside the
/// playlist window. `pl_h` is the current dynamic height in skin space —
/// the resize handle sits at (`pl_h - 27`, 257) once the user pulls the
/// window taller. `shade` collapses everything except the close button.
fn playlist_window_hot_area(x: i32, y: i32, pl_h: u32, shade: bool) -> HotArea {
    // Close button hot area is `(PL_WIDTH-11, 3, 9, 9)` — shared between
    // normal and shade rendering.
    if hit(x, y, 275 - 11, 3, 9, 9) {
        return HotArea::PClose;
    }
    if shade {
        return HotArea::PNormal;
    }
    // Title bar (top 20 px including the cornerpieces).
    if (0..20).contains(&y) {
        return HotArea::PTBar;
    }
    let body_bot = pl_h.saturating_sub(38) as i32;
    let resize_y = body_bot + (89 - 72);
    if hit(x, y, 257, resize_y as u32, 19, 21) {
        return HotArea::PSize;
    }
    // Scroll groove on the right side, between title and bottom bars.
    if hit(x, y, 260, 20, 8, pl_h.saturating_sub(58)) {
        return HotArea::PVScroll;
    }
    HotArea::PNormal
}

/// True if `(x, y)` falls inside the rect `(rx..rx+w, ry..ry+h)`.
fn hit(x: i32, y: i32, rx: u32, ry: u32, w: u32, h: u32) -> bool {
    let rx = rx as i32;
    let ry = ry as i32;
    x >= rx && x < rx + w as i32 && y >= ry && y < ry + h as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_state_creation() {
        let state = WindowState::new(2.0, false);
        assert_eq!(state.scale, 2.0);
        assert!(!state.shade_mode);
        assert!(!state.custom_chrome);
    }

    #[test]
    fn test_shade_toggle() {
        let mut state = WindowState::new(1.0, false);
        assert!(!state.shade_mode);
        state.toggle_shade();
        assert!(state.shade_mode);
    }
}
