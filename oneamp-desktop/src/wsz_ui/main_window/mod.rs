mod input;
mod paint;

use super::components::{
    BalanceSlider, ButtonManager, Clutterbar, DigitalDisplay, PlayState, PlayStateIndicator,
    PositionSlider, TitlebarButtons, VolumeSlider, WinampButton,
};
use super::components::{
    BitrateDisplay, ChannelState, MainMenu, MenuContext, MonoStereoDisplay, Oscilloscope,
    SpectrumAnalyzer, TitleScroller, build_menu_items,
};
use super::renderer::WszRenderer;
use egui::{Context, Pos2, Vec2};
use oneamp_core::wsz::skin::WszSkin;
use oneamp_core::{AudioEngine, AudioEvent};
use std::path::PathBuf;

/// Which visualizer is drawn in the main window's (24,43) 76×16 zone.
/// Cycled by clicking the zone — same behaviour as classic Winamp 2.x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerMode {
    Spectrum,
    Oscilloscope,
    /// Stereo peak / RMS bars. Reads `AudioEngine::latest_meter`.
    PeakMeter,
    Off,
}

impl VisualizerMode {
    fn next(self) -> Self {
        match self {
            Self::Spectrum => Self::Oscilloscope,
            Self::Oscilloscope => Self::PeakMeter,
            Self::PeakMeter => Self::Off,
            Self::Off => Self::Spectrum,
        }
    }
}

/// Actions emitted by the main window that the application must handle
/// (because they require state outside the window itself, like the playlist).
#[derive(Debug, Clone)]
pub enum MainWindowAction {
    /// Play the currently selected playlist entry (Play button while stopped).
    PlayCurrent,
    /// Open a file dialog to add and play a track (Eject button).
    OpenFile,
    /// Open a folder picker and append every audio file found inside
    /// (recursive, depth-capped) to the playlist. Triggered from the
    /// clutterbar Options menu or from `Shift+L` / `Ctrl+Shift+O`.
    OpenFolder,
    /// Toggle main window shade mode (Windowshade button or double-click title).
    ToggleShade,
    /// Show the About dialog (clicking the About hot area).
    ShowAbout,
    /// Flip shuffle on/off (Shuffle toggle).
    ToggleShuffle,
    /// Cycle repeat mode (Off → All → One → Off).
    CycleRepeat,
    /// Toggle equalizer window (EQ toggle).
    ToggleEqualizer,
    /// Toggle playlist window (PL toggle).
    TogglePlaylist,
    /// Toggle "Always on Top" — the app sends the matching
    /// `ViewportCommand::WindowLevel` and persists the new state.
    ToggleAlwaysOnTop,
    /// Toggle equal-power crossfade between same-format tracks. The app
    /// flips the persisted flag and pushes `SetCrossfade(enabled,
    /// duration)` through to the audio engine.
    ToggleCrossfade,
    /// Arm or disarm "stop after current". When armed, the next
    /// `AudioEvent::Finished` is intercepted before the playlist auto-
    /// advance and the engine is sent `Stop` instead. Any user-initiated
    /// playback action (Play, Next, Previous, double-click in the
    /// playlist, …) silently disarms the flag — the user clearly wants
    /// to keep playing.
    ToggleStopAfterCurrent,
    /// Enable / disable "resume long files" — the app stashes a
    /// playback position for any file longer than
    /// `RESUME_MIN_DURATION_SECS` and seeks back to it the next time
    /// the file loads. Persisted via `AppConfig::resume_long_files`.
    ToggleResumeLongFiles,
    /// Shift+click on the spectrum visualiser opens the EQ window
    /// pre-focused on the band whose centre frequency is closest to
    /// the clicked x position. The band is in `0..10` (EQ_FREQUENCIES
    /// order: 31.5 Hz → 16 kHz). The window's `flash_band` paints a
    /// brief glow on that band so the user sees which one was picked.
    OpenEqualizerAtBand(usize),
    /// User clicked "Save as preset…" in the EQ window's PRESETS
    /// dropdown. The app opens the `PresetNameDialog`; on accept it
    /// pushes the new preset into `PresetManager` and refreshes the
    /// EQ window's user-preset list.
    OpenSavePresetDialog,
    /// Toggle ReplayGain track-level gain normalization. The app flips
    /// the persisted flag and pushes `SetReplayGainEnabled` through.
    ToggleReplayGain,
    /// Pick the ReplayGain reference (Off / Track / Album / Auto) from
    /// the Audio → ReplayGain submenu. The app persists the mode, keeps
    /// `replaygain_enabled` in sync (Off ⇒ disabled), and pushes
    /// `SetReplayGainMode` to the engine.
    SetReplayGainMode(oneamp_core::ReplayGainMode),
    /// Toggle stereo→mono downmix (Audio → Mono). The app flips the
    /// persisted flag and pushes `SetMono` to the engine.
    ToggleMono,
    /// Toggle volume-dependent loudness compensation. The app flips
    /// the persisted flag and pushes `SetLoudnessEnabled` through to
    /// the audio engine.
    ToggleLoudness,
    /// Switch the cpal audio output device. `None` means "host default";
    /// `Some(name)` matches a row from `oneamp_core::list_output_devices`.
    /// Takes effect on the next track load (no live-switch in v1).
    SelectOutputDevice(Option<String>),
    /// Toggle the per-track `org.freedesktop.Notifications` toast. The app
    /// flips the persisted flag; no engine command — the notifier reads
    /// the flag directly when an `AudioEvent::TrackLoaded` arrives.
    ToggleTrackNotifications,
    /// User picked an entry from the "Recent files" section of the
    /// Options menu — open and play that file. The path is the absolute
    /// path the file was originally played from.
    PlayRecent(PathBuf),
    /// Load an M3U playlist from disk (Fichier menu).
    LoadPlaylist,
    /// Save the current playlist to an M3U on disk (Fichier menu).
    SavePlaylist,
    /// Empty the current playlist and stop playback.
    ClearPlaylist,
    /// User picked one of the integer UI-scale presets from the Affichage
    /// → Échelle submenu. `None` falls back to the DPI-derived auto
    /// heuristic; `Some(n)` forces a specific `pixels_per_point` value.
    SetUserScale(Option<f32>),
    /// Toggle Winamp's "double size" mode (clutterbar `D`). Flips the
    /// player between its native 1× pixel scale and 2×, reusing the
    /// existing `user_scale` override. A second press returns to the
    /// DPI-derived auto scale.
    ToggleDoubleSize,
    /// User picked a visualizer mode from the Affichage → Visualiseur
    /// submenu. Same enum as the click-cycle on the visualizer zone.
    SetVisualizerMode(VisualizerMode),
    /// Toggle the spectrum analyzer's floating peak-hold marker.
    SetSpectrumPeakHold(bool),
    /// Set the spectrum analyzer's peak-fall speed.
    SetSpectrumFalloff(crate::wsz_ui::components::visualization::FalloffSpeed),
    /// Set the oscilloscope render style (Lines / Dots / Solid).
    SetOscilloscopeStyle(crate::wsz_ui::components::visualization::OscilloscopeStyle),
    /// User picked a sleep-timer duration from the Audio → Sleep timer
    /// submenu. `None` cancels the active timer; `Some(min)` arms a
    /// fresh one ending `min` minutes from now.
    SetSleepTimer(Option<u32>),
    /// Manual update check from the Aide menu. Same path as the
    /// startup `UpdateChecker` but ignores the dedup so the user can
    /// re-verify after they explicitly asked.
    CheckForUpdates,
    /// Open the WSZ skin picker dialog (Affichage menu). Same UX as
    /// pressing `Alt+S`.
    PickSkin,
    /// Toggle the hotkey cheat-sheet overlay (Aide menu, F1 parity).
    ShowHotkeys,
    /// Reopen the first-launch welcome screen on demand. Originally a
    /// one-shot first-run-only viewport; surfaced under Help once we
    /// learned that users with existing configs had no way to access
    /// the centralised lang / scale / skin / default-player setup
    /// without manually flipping `first_run` in the config JSON.
    ShowWelcome,
    /// Close the app from the Lecteur menu — `ViewportCommand::Close`.
    Quit,
}

pub struct WszMainWindow {
    pub(super) renderer: WszRenderer,
    pub(super) buttons: ButtonManager,
    pub(super) position_slider: PositionSlider,
    pub(super) volume_slider: VolumeSlider,
    pub(super) balance_slider: BalanceSlider,
    pub(super) display: DigitalDisplay,
    pub(super) play_state: PlayStateIndicator,
    pub(super) spectrum: SpectrumAnalyzer,
    pub(super) oscilloscope: Oscilloscope,
    pub(super) peak_meter: crate::wsz_ui::components::visualization::PeakMeter,
    pub(super) mono_stereo: MonoStereoDisplay,
    pub(super) bitrate_display: BitrateDisplay,
    pub(super) title_scroller: TitleScroller,
    pub(super) titlebar: TitlebarButtons,
    pub(super) clutterbar: Clutterbar,
    /// Active visualizer in the (24,43) 76×16 zone. Cycled by clicking the
    /// zone (Winamp behaviour). Off hides the spectrum sprites.
    pub(super) visualizer_mode: VisualizerMode,
    /// Open state of the Options popup that the clutterbar `O` letter
    /// triggers. Closed by clicking outside or selecting an entry.
    pub(super) options_menu_open: bool,
    /// Mirror of the app's "always on top" config, used to draw the
    /// checkmark on the Options menu. Synced via `set_always_on_top`.
    pub(super) always_on_top: bool,
    /// Mirror of the app's "crossfade" config — drives the matching
    /// checkmark in the Options menu. Synced via `set_crossfade_enabled`.
    pub(super) crossfade_enabled: bool,
    /// Mirror of the app's "ReplayGain" config — drives the matching
    /// checkmark in the Options menu. Synced via `set_replaygain_enabled`.
    pub(super) replaygain_enabled: bool,
    /// Mirror of the app's "Loudness" config — drives the matching
    /// checkmark. Synced via `set_loudness_enabled`.
    pub(super) loudness_enabled: bool,
    /// Mirror of the app's "Track notifications" config — drives the
    /// matching checkmark. Synced via `set_track_notifications_enabled`.
    pub(super) track_notifications_enabled: bool,
    /// Enumerated cpal output devices (typically PulseAudio sinks /
    /// PipeWire nodes). Refreshed each frame the Options menu is open;
    /// rendered as radio-button rows in the menu's "Audio output" group.
    pub(super) output_devices: Vec<String>,
    /// Currently-selected output device name; `None` = host default.
    /// Used to render the ✓ on the matching device row.
    pub(super) current_output_device: Option<String>,
    /// Recently-played file paths surfaced as clickable rows inside the
    /// Options menu. Synced from `AppConfig::recent_files` once per
    /// frame via `set_recent_paths`.
    pub(super) recent_paths: Vec<PathBuf>,
    /// Hierarchical popup spawned by the top-left logo (titlebar menu
    /// button). Owns its open/closed state across frames; rendering
    /// runs after every other paint so the submenu chain sits on top
    /// of the rest of the player.
    pub(super) main_menu: MainMenu,
    /// Snapshot of every toggle/selection the menu reads — pushed in
    /// once per frame by the coordinator via `set_menu_context`.
    /// Lives here so `render_main_menu` can borrow it without the
    /// coordinator passing a long arg list through the show() chain.
    pub(super) menu_context: Option<MenuContext>,

    pub(super) is_playing: bool,
    pub(super) is_paused: bool,
    /// True once a track has been loaded — drives the visibility of the time
    /// digits, KBPS/KHZ readouts, and play-state LED. Winamp keeps these
    /// slots blank before any track is loaded; we do the same so a fresh
    /// launch matches.
    pub(super) has_track: bool,
    /// Soft-focus flag pushed in by the coordinator. Combined with the OS
    /// focus signal to decide whether to draw the titlebar's active or
    /// inactive strip — the OS only knows about the shared viewport, so
    /// this is how main goes "inactive" when the user clicks into the EQ
    /// or playlist sub-area.
    pub(super) soft_focused: bool,
    pub(super) mouse_pressed: bool,
    /// When false, the OS provides window chrome — the in-skin drag handler
    /// must stay off, otherwise its `StartDrag` fights the compositor and
    /// traps the pointer in a perpetual move loop.
    pub(super) custom_chrome: bool,
}

impl WszMainWindow {
    pub fn new(skin: WszSkin, scale: f32, custom_chrome: bool) -> Self {
        Self {
            renderer: WszRenderer::new(skin, scale),
            buttons: ButtonManager::new(),
            position_slider: PositionSlider::new(),
            volume_slider: VolumeSlider::new(),
            balance_slider: BalanceSlider::new(),
            display: DigitalDisplay::new(),
            play_state: PlayStateIndicator::new(),
            spectrum: SpectrumAnalyzer::new(),
            oscilloscope: Oscilloscope::new(),
            peak_meter: crate::wsz_ui::components::visualization::PeakMeter::new(),
            mono_stereo: MonoStereoDisplay::new(),
            bitrate_display: BitrateDisplay::new(),
            title_scroller: TitleScroller::new(),
            titlebar: TitlebarButtons::new(),
            clutterbar: Clutterbar::new(),
            visualizer_mode: VisualizerMode::Spectrum,
            options_menu_open: false,
            always_on_top: false,
            crossfade_enabled: false,
            replaygain_enabled: false,
            loudness_enabled: false,
            track_notifications_enabled: false,
            output_devices: Vec::new(),
            current_output_device: None,
            recent_paths: Vec::new(),
            main_menu: MainMenu::new(),
            menu_context: None,
            is_playing: false,
            is_paused: false,
            has_track: false,
            // Start focused — fresh launch should show the active titlebar
            // until the user clicks into a docked sub-window.
            soft_focused: true,
            mouse_pressed: false,
            custom_chrome,
        }
    }

    /// Borrow the active skin so the coordinator can pass it to the cursor
    /// overlay (and any other consumer that needs it without owning a
    /// separate copy).
    pub fn skin(&self) -> &WszSkin {
        self.renderer.get_skin()
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        audio_engine: Option<&AudioEngine>,
        spectrum_data: &[f32],
        waveform_data: &oneamp_core::WaveformSnapshot,
        meter_data: &oneamp_core::MeterSnapshot,
    ) -> Option<MainWindowAction> {
        let scale = self.renderer.get_scale();
        let window_size = Vec2::new(275.0 * scale, 116.0 * scale);

        let delta_time = ctx.input(|i| i.unstable_dt);
        self.title_scroller.update();

        // Direct painting - no window wrapper. The Area closure returns the
        // action it produced; we extract it via InnerResponse so the borrow
        // checker doesn't complain about moving an outer `let mut action`
        // through an FnOnce.
        let inner = egui::Area::new(egui::Id::new("wsz_main_window"))
            .fixed_pos(Pos2::ZERO)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_min_size(window_size);
                ui.set_max_size(window_size);

                let area_rect = ui.max_rect();
                let offset = area_rect.min;

                // Refresh button press visuals BEFORE we render them.
                // `handle_input` runs at the end of this closure and would
                // otherwise leave the rendered sprite one frame behind the
                // pointer state — the press feedback wouldn't show until
                // the *next* repaint, which on a static window means it
                // doesn't show at all until the user moves the mouse.
                let scale = self.renderer.get_scale();
                let (mouse_pos_pre, is_pressed_pre) =
                    ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
                self.buttons
                    .update_all(mouse_pos_pre, is_pressed_pre, offset, scale);

                self.render_background(ui, offset);
                if self.custom_chrome {
                    // `self.soft_focused` is the single source of truth for
                    // main-titlebar state — the coordinator already folds
                    // the OS focus signal into it (`os_focused &&
                    // active_subwindow == Main`), so consulting `i.focused`
                    // here too would just AND the same bit twice and, on
                    // compositors that report `i.focused == false` for our
                    // undecorated/transparent viewport, would force the
                    // titlebar permanently inactive. The EQ and playlist
                    // windows already follow this pattern with their own
                    // `self.focused`.
                    self.titlebar.active = self.soft_focused;
                    // None means `AudioEngine::new()` failed in
                    // AudioController::new(). Surface the spec's
                    // alternate y=57/72 strip so the user notices.
                    self.titlebar.audio_failed = audio_engine.is_none();
                    self.titlebar.render(&mut self.renderer, ui, offset);
                }
                self.clutterbar.render(&mut self.renderer, ui, offset);
                self.render_visualization(
                    ui,
                    offset,
                    spectrum_data,
                    waveform_data,
                    meter_data,
                    delta_time,
                );
                if self.has_track {
                    self.render_play_state(ui, offset);
                    self.render_display(ui, offset);
                    self.render_info_displays(ui, offset);
                }
                self.render_position_slider(ui, offset);
                self.render_buttons(ui, offset);
                self.render_volume_slider(ui, offset);
                self.render_balance_slider(ui, offset);
                let action = self.handle_input(ui, ctx, offset, audio_engine);
                let drag_action = self.handle_window_drag(ui, ctx, offset);
                let options_action = self.render_options_menu(ui, offset);
                // Drive the titlebar menu LAST. The popup itself paints
                // in its own OS sub-viewport (see `MainMenu::render`), so
                // this call doesn't touch the parent `ui` — it just
                // spawns / refreshes the popup and reads back the
                // user's pick. Z-ordering against the parent player is
                // handled by the OS (the popup is `AlwaysOnTop`).
                let menu_action = self.drive_main_menu(ctx);
                action.or(drag_action).or(options_action).or(menu_action)
            });

        inner.inner
    }

    /// Sync toggle button on/off state with the rest of the app. Called from
    /// the app loop after state updates.
    pub fn set_shuffle(&mut self, on: bool) {
        self.buttons.set_toggle(WinampButton::Shuffle, on);
    }

    pub fn set_repeat_on(&mut self, on: bool) {
        self.buttons.set_toggle(WinampButton::Repeat, on);
    }

    pub fn set_equalizer_visible(&mut self, on: bool) {
        self.buttons.set_toggle(WinampButton::EqToggle, on);
    }

    pub fn set_playlist_visible(&mut self, on: bool) {
        self.buttons.set_toggle(WinampButton::PlToggle, on);
    }

    /// Mirror the app-level Always-on-Top flag so the Options menu can
    /// render the matching checkmark.
    pub fn set_always_on_top(&mut self, on: bool) {
        self.always_on_top = on;
    }

    /// Soft-focus setter pushed in by the coordinator. The titlebar's
    /// `active` flag combines this with the OS focus signal so the main
    /// player goes inactive when the user clicks into a docked sub-window
    /// even though the OS still considers our viewport focused.
    pub fn set_soft_focused(&mut self, focused: bool) {
        self.soft_focused = focused;
    }

    /// Mirror the app-level crossfade flag so the Options menu can render
    /// the matching checkmark.
    pub fn set_crossfade_enabled(&mut self, on: bool) {
        self.crossfade_enabled = on;
    }

    /// Mirror the app-level ReplayGain flag so the Options menu can render
    /// the matching checkmark.
    pub fn set_replaygain_enabled(&mut self, on: bool) {
        self.replaygain_enabled = on;
    }

    /// Mirror the app-level loudness flag so the Options menu can render
    /// the matching checkmark.
    pub fn set_loudness_enabled(&mut self, on: bool) {
        self.loudness_enabled = on;
    }

    /// Mirror the app-level track-notifications flag so the Options
    /// menu can render the matching checkmark.
    pub fn set_track_notifications_enabled(&mut self, on: bool) {
        self.track_notifications_enabled = on;
    }

    /// Push the live cpal-output-device list + current selection in so
    /// the Options menu renders matching radio rows. Called once per
    /// frame from the coordinator; cheap to re-set even when unchanged.
    pub fn set_output_device_state(&mut self, devices: Vec<String>, current: Option<String>) {
        self.output_devices = devices;
        self.current_output_device = current;
    }

    /// Push the most-recent paths into the window so the Options menu can
    /// render them as clickable rows. Cap at the visible-row budget — the
    /// menu only shows ~5 entries to keep the popup compact.
    pub fn set_recent_paths(&mut self, paths: Vec<PathBuf>) {
        self.recent_paths = paths;
    }

    /// Current visualiser mode, exposed so the app can persist it on save
    /// and survive between sessions.
    pub fn visualizer_mode(&self) -> VisualizerMode {
        self.visualizer_mode
    }

    /// Force the visualiser mode at boot from the persisted config.
    pub fn set_visualizer_mode(&mut self, mode: VisualizerMode) {
        self.visualizer_mode = mode;
    }

    /// Push the user's analyzer / oscilloscope options into the
    /// visualization components. Called once per frame from the
    /// coordinator so a menu change takes effect immediately.
    pub fn set_visualizer_options(
        &mut self,
        opts: crate::wsz_ui::components::visualization::VisualizerOptions,
    ) {
        self.spectrum
            .set_options(opts.spectrum_peak_hold, opts.spectrum_falloff);
        self.oscilloscope.set_style(opts.oscilloscope_style);
    }

    /// Whether the digital time display is showing remaining time
    /// (prefix `-`) instead of elapsed time. Flipped by clicking the
    /// time display (Winamp convention) and mirrored into `AppConfig`
    /// so the preference survives across sessions.
    pub fn show_remaining(&self) -> bool {
        self.display.show_remaining
    }

    /// Restore the persisted "show remaining" preference on boot.
    pub fn set_show_remaining(&mut self, remaining: bool) {
        self.display.show_remaining = remaining;
    }

    /// One-shot scroll-wheel seek on the position bar. Called from the
    /// app's update loop when the pointer hovers over the slider rect
    /// and a scroll delta is reported by egui. Returns the absolute
    /// position in seconds the caller should send via `AudioCommand::Seek`,
    /// or `None` if the engine has no seekable position yet (idle /
    /// streaming radio).
    ///
    /// `step_secs` is the seek delta per "click" of the wheel — egui
    /// reports `scroll_delta.y` in pixels with one notch being ≈50 px
    /// on most mice; we threshold against that. Positive y = scroll up
    /// = seek forward, matching the volume slider's convention where
    /// up = louder.
    pub fn try_wheel_seek(&self, scroll_y: f32, step_secs: f32) -> Option<f32> {
        if self.display.total_time <= 0.5 {
            return None;
        }
        if scroll_y.abs() < 1.0 {
            return None;
        }
        // egui pre-multiplies scroll_delta by ~50 per notch. One notch
        // → one step; anything bigger scales linearly so a fast flick
        // covers more ground.
        let notches = (scroll_y / 50.0).round();
        if notches.abs() < 0.5 {
            return None;
        }
        let new_pos = (self.display.current_time + notches * step_secs)
            .clamp(0.0, self.display.total_time - 0.5);
        Some(new_pos)
    }

    /// Push the live snapshot of every menu-driven toggle/selection into
    /// the window. The titlebar menu rebuilds its `MenuItem` tree from
    /// this snapshot every time it opens, so toggles always reflect the
    /// current state without explicit invalidation. Called once per
    /// frame from the coordinator.
    pub fn set_menu_context(&mut self, ctx: MenuContext) {
        self.menu_context = Some(ctx);
    }

    /// Open / close the titlebar menu. Used by the titlebar input
    /// handler when the user clicks the (6,3) Menu button.
    pub fn toggle_main_menu(&mut self) {
        self.main_menu.toggle();
    }

    /// Drive the titlebar menu (no-op while closed) and return the
    /// action the user picked, if any. The menu lives in its own OS
    /// sub-viewport (popup window) since 1.0.3, so it doesn't paint into
    /// the parent `Ui` and doesn't need the main window's offset — it
    /// computes its own screen position from the parent context's
    /// viewport info.
    pub(super) fn drive_main_menu(
        &mut self,
        parent_ctx: &egui::Context,
    ) -> Option<MainWindowAction> {
        if !self.main_menu.open {
            return None;
        }
        let scale = self.renderer.get_scale();
        let ctx = self.menu_context.as_ref()?;
        let items = build_menu_items(ctx);
        let skin_font_available = ctx.skin_font_available;
        self.main_menu
            .render(parent_ctx, scale, &items, skin_font_available)
    }

    pub fn update(&mut self, events: &[AudioEvent]) {
        for event in events {
            match event {
                AudioEvent::Playing => {
                    self.is_playing = true;
                    self.is_paused = false;
                    self.play_state.set_state(PlayState::Playing);
                }
                AudioEvent::Paused => {
                    self.is_paused = true;
                    self.play_state.set_state(PlayState::Paused);
                }
                AudioEvent::Stopped => {
                    self.is_playing = false;
                    self.is_paused = false;
                    // Stop = "no track loaded" in Winamp terms: the digits +
                    // KBPS/KHZ slots go blank again. Resetting `has_track`
                    // here keeps the display in sync with the engine.
                    self.has_track = false;
                    self.play_state.set_state(PlayState::Stopped);
                    self.mono_stereo.set_state(ChannelState::None);
                }
                AudioEvent::Position(current, total) => {
                    self.display.set_time(*current, *total);
                    if !self.position_slider.is_dragging {
                        let progress = if *total > 0.0 { current / total } else { 0.0 };
                        self.position_slider.set_progress(progress);
                    }
                }
                AudioEvent::VolumeUpdated(vol, _) if !self.volume_slider.is_dragging => {
                    self.volume_slider.set_value(*vol);
                }
                AudioEvent::BalanceUpdated(balance) if !self.balance_slider.is_dragging => {
                    self.balance_slider.set_value(*balance);
                }
                AudioEvent::TrackLoaded(track) => {
                    self.has_track = true;
                    let title = track.title.clone().unwrap_or_else(|| {
                        track
                            .path
                            .file_name()
                            .and_then(|n: &std::ffi::OsStr| n.to_str())
                            .unwrap_or("Unknown")
                            .to_string()
                    });
                    self.title_scroller.set_track(title, track.duration_secs);

                    // TrackInfo.bitrate is in bits/sec; the display takes kbps.
                    let bitrate_kbps = track.bitrate.map(|b| b / 1000).unwrap_or(0);
                    let sample_rate_hz = track.sample_rate.unwrap_or(0);
                    self.bitrate_display.set_info(bitrate_kbps, sample_rate_hz);

                    if let Some(channels) = track.channels {
                        self.mono_stereo.set_state(if channels >= 2 {
                            ChannelState::Stereo
                        } else {
                            ChannelState::Mono
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

/// True if the screen-space `mouse_pos` falls inside the skin-space rect
/// `(skin_x, skin_y, skin_w, skin_h)` translated by `offset` and scaled.
pub(super) fn hit_rect(
    mouse_pos: Pos2,
    offset: Pos2,
    scale: f32,
    skin_x: u32,
    skin_y: u32,
    skin_w: u32,
    skin_h: u32,
) -> bool {
    let min_x = offset.x + skin_x as f32 * scale;
    let min_y = offset.y + skin_y as f32 * scale;
    let max_x = min_x + skin_w as f32 * scale;
    let max_y = min_y + skin_h as f32 * scale;
    mouse_pos.x >= min_x && mouse_pos.x <= max_x && mouse_pos.y >= min_y && mouse_pos.y <= max_y
}
