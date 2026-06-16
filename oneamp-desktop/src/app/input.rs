//! Input dispatch: main-window menu actions, keyboard shortcuts,
//! the F1 hotkey cheat-sheet overlay and the drag-drop ingest entry
//! point.
//!
//! All four are pure "input → app mutation" surfaces. Splitting them
//! out of `app/mod.rs` keeps the keyboard hotkey matrix and the
//! Options-menu fan-out in one file, easier to audit when adding a
//! new shortcut or menu entry.

use super::{AUDIO_EXTENSIONS, OneAmpApp};
use crate::platform::updater::UpdateChecker;
use crate::windows::{MainWindowAction, PlaylistAction};
use eframe::egui;
use oneamp_core::{AudioCommand, RepeatMode};

impl OneAmpApp {
    /// Dispatch an action emitted by the main window
    pub(super) fn handle_main_window_action(
        &mut self,
        action: MainWindowAction,
        ctx: &egui::Context,
    ) {
        match action {
            MainWindowAction::PlayCurrent => {
                let current = self.playlist.current_entry().map(|e| e.path.clone());
                if let Some(path) = current {
                    self.play_audio_path(path);
                }
            }
            MainWindowAction::OpenFile => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", AUDIO_EXTENSIONS)
                    .pick_file()
                {
                    self.playlist.add_track(path.clone());
                    self.play_audio_path(path);
                }
            }
            MainWindowAction::OpenFolder => {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    // Reuse the drag-drop ingest path: it dedupes against
                    // the playlist, kicks off playback when the engine
                    // was idle, and pulls the window forward when the
                    // user just told us "play these tracks". `force_play
                    // = false` mirrors Winamp's "Add Folder…" — appending
                    // a folder mid-playback doesn't interrupt the song.
                    self.ingest_files(&[folder], ctx, false);
                }
            }
            MainWindowAction::ToggleShade => {
                self.windows.toggle_shade();
            }
            MainWindowAction::ShowAbout => {
                // Build-info block: version + git hash (when built from a
                // checkout) + build date + the config dir the user can
                // open from their file manager to grab the JSON if a
                // bug report needs it. Bug reporters get all the
                // metadata in one place — no need to ask them
                // separately for "what version are you on?".
                let git = env!("ONEAMP_GIT_HASH");
                let version_line = if git.is_empty() {
                    format!("OneAmp v{}", env!("CARGO_PKG_VERSION"))
                } else {
                    format!("OneAmp v{} ({})", env!("CARGO_PKG_VERSION"), git)
                };
                let config_line = crate::config::AppConfig::config_path()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.display().to_string()))
                    .map(|s| format!("\nConfig: {}", s))
                    .unwrap_or_default();
                crate::dialog_util::show_info(
                    "About OneAmp",
                    &format!(
                        "{}\nBuild: {}\n\nWinamp-style audio player.\nhttps://github.com/all3f0r1/oneamp{}",
                        version_line,
                        env!("ONEAMP_BUILD_DATE"),
                        config_line,
                    ),
                );
            }
            MainWindowAction::ToggleShuffle => {
                self.state.toggle_shuffle();
                self.audio
                    .send_command(AudioCommand::SetShuffle(self.state.shuffle_enabled));
            }
            MainWindowAction::CycleRepeat => {
                let new_mode = match self.state.repeat_mode {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
                self.state.set_repeat_mode(new_mode);
                self.audio
                    .send_command(AudioCommand::SetRepeatMode(new_mode));
            }
            MainWindowAction::ToggleEqualizer => {
                self.windows.toggle_equalizer();
            }
            MainWindowAction::TogglePlaylist => {
                self.windows.toggle_playlist();
            }
            MainWindowAction::ToggleAlwaysOnTop => {
                self.always_on_top = !self.always_on_top;
                // Egui exposes the policy through ViewportCommand. Some
                // compositors (esp. Wayland) ignore the request silently —
                // there's no visible indicator beyond the menu's checkmark
                // until the next frame brings the new policy live.
                let level = if self.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
            }
            MainWindowAction::ToggleCrossfade => {
                self.config.crossfade.enabled = !self.config.crossfade.enabled;
                self.audio.send_command(AudioCommand::SetCrossfade(
                    self.config.crossfade.enabled,
                    self.config.crossfade.duration_secs,
                ));
                self.mark_dirty();
            }
            MainWindowAction::ToggleStopAfterCurrent => {
                self.stop_after_current = !self.stop_after_current;
                let msg = if self.stop_after_current {
                    "Will stop after current track"
                } else {
                    "Stop after current — cancelled"
                };
                self.push_toast(msg, std::time::Duration::from_millis(1800));
            }
            MainWindowAction::ToggleResumeLongFiles => {
                self.config.resume_long_files = !self.config.resume_long_files;
                let msg = if self.config.resume_long_files {
                    "Resume long files: ON"
                } else {
                    "Resume long files: OFF"
                };
                self.push_toast(msg, std::time::Duration::from_millis(1800));
                self.mark_dirty();
            }
            MainWindowAction::OpenEqualizerAtBand(band) => {
                // Pop the EQ open if it was closed, then flash the
                // band so the user sees which one was picked. The
                // flash auto-clears in the EQ window's paint pass
                // after ~1.5 s.
                self.windows.set_equalizer_visible(true);
                self.windows.flash_eq_band(band);
                self.mark_dirty();
            }
            MainWindowAction::OpenSavePresetDialog => {
                // Idempotent: re-clicking "Save as preset…" while the
                // modal is already up shouldn't stack a second one.
                if self.preset_name_dialog.is_none() {
                    self.preset_name_dialog =
                        Some(crate::preset_name_dialog::PresetNameDialog::new());
                }
            }
            MainWindowAction::ToggleReplayGain => {
                self.config.replaygain_enabled = !self.config.replaygain_enabled;
                self.audio.send_command(AudioCommand::SetReplayGainEnabled(
                    self.config.replaygain_enabled,
                ));
                self.mark_dirty();
            }
            MainWindowAction::SetReplayGainMode(mode) => {
                // The mode submenu is the richer control: it also drives
                // the on/off flag so the legacy `SetReplayGainEnabled`
                // path and the menu's ✓ stay consistent. Off ⇒ disabled.
                self.config.replaygain_mode = mode;
                self.config.replaygain_enabled = mode != oneamp_core::ReplayGainMode::Off;
                self.audio
                    .send_command(AudioCommand::SetReplayGainMode(mode));
                self.mark_dirty();
            }
            MainWindowAction::ToggleMono => {
                self.config.mono_enabled = !self.config.mono_enabled;
                self.audio
                    .send_command(AudioCommand::SetMono(self.config.mono_enabled));
                self.mark_dirty();
            }
            MainWindowAction::ToggleLoudness => {
                self.config.loudness_enabled = !self.config.loudness_enabled;
                self.audio.send_command(AudioCommand::SetLoudnessEnabled(
                    self.config.loudness_enabled,
                ));
                self.mark_dirty();
            }
            MainWindowAction::SelectOutputDevice(name) => {
                // Persist and push to the engine. The audio thread
                // applies it on the next track load — the live Sink
                // stays running so the user doesn't lose what they're
                // currently playing while clicking through the menu.
                self.config.output_device_name = name.clone();
                self.audio.send_command(AudioCommand::SetOutputDevice(name));
                self.mark_dirty();
            }
            MainWindowAction::ToggleTrackNotifications => {
                self.config.track_notifications_enabled = !self.config.track_notifications_enabled;
                self.mark_dirty();
            }
            MainWindowAction::PlayRecent(path) => {
                // Append the recent entry to the playlist (dedup'd by
                // path inside `add_track`) so navigation N/P keeps
                // working off it, then play. `play_audio_path` bumps
                // its own slot in RecentFiles up to the head.
                self.playlist.add_track(path.clone());
                self.play_audio_path(path);
            }
            MainWindowAction::LoadPlaylist => {
                self.handle_playlist_action(PlaylistAction::LoadM3u);
            }
            MainWindowAction::SavePlaylist => {
                self.handle_playlist_action(PlaylistAction::SaveM3u);
            }
            MainWindowAction::ClearPlaylist => {
                self.handle_playlist_action(PlaylistAction::Clear);
            }
            MainWindowAction::SetUserScale(choice) => {
                // None resets to the DPI heuristic; Some(n) pins the
                // override. Setting `scale_dirty` triggers the update
                // tick to push the new `pixels_per_point` through and
                // invalidate the coordinator's viewport cache.
                self.user_scale = choice;
                self.scale_dirty = true;
            }
            MainWindowAction::ToggleDoubleSize => {
                // Winamp's double-size toggle: 2× when off, back to the
                // DPI-auto scale when already doubled. We treat "currently
                // pinned at exactly 2×" as the doubled state; any other
                // value (auto, or a user-chosen 1×/3×/4×) doubles to 2×.
                let already_doubled = self.user_scale == Some(2.0);
                self.user_scale = if already_doubled { None } else { Some(2.0) };
                self.scale_dirty = true;
                self.mark_dirty();
            }
            MainWindowAction::SetVisualizerMode(mode) => {
                self.windows.main_window_mut().set_visualizer_mode(mode);
            }
            MainWindowAction::SetSpectrumPeakHold(on) => {
                self.config.visualizer_options.spectrum_peak_hold = on;
                self.mark_dirty();
            }
            MainWindowAction::SetSpectrumFalloff(speed) => {
                self.config.visualizer_options.spectrum_falloff = speed;
                self.mark_dirty();
            }
            MainWindowAction::SetOscilloscopeStyle(style) => {
                self.config.visualizer_options.oscilloscope_style = style;
                self.mark_dirty();
            }
            MainWindowAction::SetSleepTimer(choice) => {
                self.set_sleep_timer(choice);
            }
            MainWindowAction::CheckForUpdates => {
                // Reset the version dedup so the next poll, regardless of
                // whether it matches the last seen version, surfaces a
                // toast. Spawn a fresh checker — the original was
                // consumed at boot.
                self.config.last_notified_update_version = None;
                self.update_checker = UpdateChecker::spawn();
                // Mark this as a user-driven check so `poll_update_checker`
                // surfaces an explicit "Already up to date" toast on a
                // negative result. The startup check stays silent on no-
                // update — only a manual click expects feedback.
                self.manual_update_check_pending = Some(std::time::Instant::now());
                self.push_toast(
                    "Checking for updates…",
                    std::time::Duration::from_millis(1200),
                );
                self.mark_dirty();
            }
            MainWindowAction::PickSkin => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Winamp skin", &["wsz", "WSZ"])
                    .pick_file()
                {
                    self.apply_skin_from_file(path);
                }
            }
            MainWindowAction::ShowHotkeys => {
                self.show_hotkeys = !self.show_hotkeys;
            }
            MainWindowAction::ShowWelcome => {
                // Re-derive the discovered skin catalog so the picker
                // is fresh (the user may have dropped new .wsz files in
                // their folder since boot). `first_run` is not flipped
                // — Done / Skip on the reopened viewport still treats
                // the config as already-bootstrapped.
                self.welcome.rescan(self.config.user_skins_dir.as_deref());
                self.welcome.open = true;
            }
            MainWindowAction::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Letter keys that drive both the transport hotkeys (N/P/L/S/V/R)
    /// and playlist type-to-jump. Iterated in alphabetical order so
    /// pressing two letters in the same frame (rare but possible) lands
    /// on the alphabetically earlier one — predictable.
    pub(super) const JUMP_LETTERS: &'static [(egui::Key, char)] = &[
        (egui::Key::A, 'a'),
        (egui::Key::B, 'b'),
        (egui::Key::C, 'c'),
        (egui::Key::D, 'd'),
        (egui::Key::E, 'e'),
        (egui::Key::F, 'f'),
        (egui::Key::G, 'g'),
        (egui::Key::H, 'h'),
        (egui::Key::I, 'i'),
        (egui::Key::J, 'j'),
        (egui::Key::K, 'k'),
        (egui::Key::L, 'l'),
        (egui::Key::M, 'm'),
        (egui::Key::N, 'n'),
        (egui::Key::O, 'o'),
        (egui::Key::P, 'p'),
        (egui::Key::Q, 'q'),
        (egui::Key::R, 'r'),
        (egui::Key::S, 's'),
        (egui::Key::T, 't'),
        (egui::Key::U, 'u'),
        (egui::Key::V, 'v'),
        (egui::Key::W, 'w'),
        (egui::Key::X, 'x'),
        (egui::Key::Y, 'y'),
        (egui::Key::Z, 'z'),
    ];

    /// Handle keyboard shortcuts
    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let playlist_focused = self.windows.is_playlist_focused();
        ctx.input(|i| {
            // Space: Toggle play/pause — shared with the macOS menu
            // bar and the tray icon, so all three paths agree on what
            // "Play" means depending on current state.
            if i.key_pressed(egui::Key::Space) {
                self.toggle_playback();
            }

            // Nullsoft easter egg: typing N-U-L in quick succession (a
            // nod to Winamp's titlebar gag). Tracked across frames with a
            // short deadline. Completing it flashes a toast and swallows
            // the final `l` so it doesn't also pop the Open-file dialog.
            let now = std::time::Instant::now();
            if self.nul_deadline.map(|d| now > d).unwrap_or(true) {
                self.nul_progress = 0;
            }
            let mut nul_completed = false;
            if !i.modifiers.any() && !playlist_focused {
                let bump = std::time::Duration::from_millis(1500);
                if i.key_pressed(egui::Key::N) {
                    self.nul_progress = 1;
                    self.nul_deadline = Some(now + bump);
                } else if i.key_pressed(egui::Key::U) && self.nul_progress == 1 {
                    self.nul_progress = 2;
                    self.nul_deadline = Some(now + bump);
                } else if i.key_pressed(egui::Key::L) && self.nul_progress == 2 {
                    nul_completed = true;
                    self.nul_progress = 0;
                }
            }
            if nul_completed {
                self.push_toast("Nullsoft!", std::time::Duration::from_millis(2000));
            }

            // Ctrl+O: Open file
            if i.modifiers.ctrl
                && i.key_pressed(egui::Key::O)
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", AUDIO_EXTENSIONS)
                    .pick_file()
            {
                self.playlist.add_track(path.clone());
                self.play_audio_path(path);
            }

            // Alt+S: Open WSZ skin file (Winamp convention)
            if i.modifiers.alt
                && i.key_pressed(egui::Key::S)
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Winamp skin", &["wsz", "WSZ"])
                    .pick_file()
            {
                self.apply_skin_from_file(path);
            }

            // L: Load (open) an audio file. Mirrors Winamp's "L" shortcut and
            // matches what the Eject button does, but reachable from keyboard.
            // Suppressed when the playlist is the focused sub-window — bare
            // letters there drive type-to-jump instead.
            if i.key_pressed(egui::Key::L)
                && !i.modifiers.any()
                && !playlist_focused
                && !nul_completed
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", AUDIO_EXTENSIONS)
                    .pick_file()
            {
                self.playlist.add_track(path.clone());
                self.play_audio_path(path);
            }

            // N: Next track (suppressed when playlist focused — `n` jumps)
            if i.key_pressed(egui::Key::N) && !playlist_focused {
                let next = self.playlist.next_entry().map(|e| e.path.clone());
                if let Some(path) = next {
                    self.play_audio_path(path);
                }
            }

            // P: Previous track (suppressed when playlist focused — `p` jumps)
            if i.key_pressed(egui::Key::P) && !playlist_focused {
                let prev = self.playlist.previous_entry().map(|e| e.path.clone());
                if let Some(path) = prev {
                    self.play_audio_path(path);
                }
            }

            // S: Stop (only without modifiers, so Alt+S can be used elsewhere)
            if i.key_pressed(egui::Key::S) && !i.modifiers.any() && !playlist_focused {
                self.transport_stop();
            }

            // Z X C V B — the classic Winamp transport row, sitting under
            // the left hand for one-handed control. Z=prev, X=play,
            // C=pause, V=stop, B=next. Bare letters only, and suppressed
            // when the playlist has soft focus so they keep driving
            // type-to-jump there. These coexist with N/P/S/Space, which
            // stay wired for users who learned OneAmp's earlier layout.
            if !i.modifiers.any() && !playlist_focused {
                if i.key_pressed(egui::Key::Z) {
                    let prev = self.playlist.previous_entry().map(|e| e.path.clone());
                    if let Some(path) = prev {
                        self.play_audio_path(path);
                    }
                }
                if i.key_pressed(egui::Key::X) {
                    self.transport_play();
                }
                if i.key_pressed(egui::Key::C) {
                    self.transport_pause();
                }
                if i.key_pressed(egui::Key::V) {
                    self.transport_stop();
                }
                if i.key_pressed(egui::Key::B) {
                    let next = self.playlist.next_entry().map(|e| e.path.clone());
                    if let Some(path) = next {
                        self.play_audio_path(path);
                    }
                }
            }

            // Shift+S: Toggle "stop after current". Not gated on
            // `playlist_focused` — the bare-letter jump path explicitly
            // ignores modified key presses (see the closure below), so
            // this binding works equally well from inside the playlist.
            if i.key_pressed(egui::Key::S)
                && i.modifiers.shift
                && !i.modifiers.ctrl
                && !i.modifiers.alt
            {
                self.stop_after_current = !self.stop_after_current;
                let msg = if self.stop_after_current {
                    "Will stop after current track"
                } else {
                    "Stop after current — cancelled"
                };
                self.push_toast(msg, std::time::Duration::from_millis(1800));
            }

            // (V is the Winamp Stop hotkey — see the Z X C V B block
            // above. Shuffle stays reachable via the main-window shuffle
            // button and the Options menu.)

            // R: Toggle repeat (cycle through modes) (suppressed when playlist focused)
            if i.key_pressed(egui::Key::R) && !playlist_focused {
                let new_mode = match self.state.repeat_mode {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
                self.state.set_repeat_mode(new_mode);
                self.audio
                    .send_command(AudioCommand::SetRepeatMode(new_mode));
            }

            // M: Mute toggle (suppressed when playlist focused — bare `m`
            // jumps to the next entry whose title starts with M there).
            // Outside the playlist, this is a global hotkey.
            if i.key_pressed(egui::Key::M) && !i.modifiers.any() && !playlist_focused {
                let new_muted = !self.state.volume.muted;
                self.state.volume.muted = new_muted;
                self.audio.send_command(AudioCommand::SetMute(new_muted));
                self.mark_dirty();
            }

            // ↑ / ↓: Volume ±5 %, Shift+↑/↓: ±1 %. Always global — arrow
            // keys don't conflict with playlist type-to-jump. We optimistically
            // update local state so rapid repeats keep stepping without
            // waiting for the audio thread's echo, then let `VolumeUpdated`
            // reconcile if anything drifted.
            let vol_step = if i.modifiers.shift { 0.01 } else { 0.05 };
            if i.key_pressed(egui::Key::ArrowUp) {
                let new_vol = (self.state.volume.level + vol_step).clamp(0.0, 1.0);
                self.state.volume.level = new_vol;
                self.audio.send_command(AudioCommand::SetVolume(new_vol));
                self.mark_dirty();
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                let new_vol = (self.state.volume.level - vol_step).clamp(0.0, 1.0);
                self.state.volume.level = new_vol;
                self.audio.send_command(AudioCommand::SetVolume(new_vol));
                self.mark_dirty();
            }

            // ← / →: Seek ±5 s, Shift+←/→: ±30 s. Gated on having a
            // seekable position — a stopped/idle engine has total = 0.
            // We clamp shy of the very end so the limiter doesn't catch
            // a half-decoded frame.
            let seek_step = if i.modifiers.shift { 30.0 } else { 5.0 };
            let (cur_pos, total_pos) = self.state.position;
            if total_pos > 0.5 {
                if i.key_pressed(egui::Key::ArrowRight) {
                    let new_pos = (cur_pos + seek_step).clamp(0.0, total_pos - 0.5);
                    self.audio.send_command(AudioCommand::Seek(new_pos));
                }
                if i.key_pressed(egui::Key::ArrowLeft) {
                    let new_pos = (cur_pos - seek_step).max(0.0);
                    self.audio.send_command(AudioCommand::Seek(new_pos));
                }
            }

            // F1 / Shift+`?` (Shift+Slash on US/FR layouts via egui-winit):
            // toggle the hotkey cheat-sheet overlay. Escape also dismisses
            // it below.
            let toggle_help = i.key_pressed(egui::Key::F1)
                || (i.key_pressed(egui::Key::Slash) && i.modifiers.shift);
            if toggle_help {
                self.show_hotkeys = !self.show_hotkeys;
            }

            // Escape: dismiss the hotkey overlay. Native error dialogs
            // are owned by the OS and absorb Escape themselves, so we
            // only need to handle the in-app overlay here. Filter
            // overlay handles its own Esc inside the TextEdit closure
            // (see paint_playlist_filter_overlay) so this branch
            // doesn't pre-empt it.
            if i.key_pressed(egui::Key::Escape) && self.show_hotkeys {
                self.show_hotkeys = false;
            }
        });

        // Ctrl+F: toggle the playlist inline filter overlay. Works
        // regardless of which sub-window has soft-focus — the filter
        // only takes effect when the playlist is visible, but
        // opening / closing the overlay from anywhere matches the
        // browser/IDE convention users already know.
        let wants_filter_toggle = ctx.input(|i| {
            i.modifiers.ctrl
                && !i.modifiers.alt
                && !i.modifiers.shift
                && i.key_pressed(egui::Key::F)
        });
        if wants_filter_toggle {
            // Auto-open the playlist when the user hits Ctrl+F — a
            // filter the user can't see is pure friction. Idempotent
            // if it's already open.
            self.windows.set_playlist_visible(true);
            self.playlist_filter_open = !self.playlist_filter_open;
            self.playlist_filter_focus_pending = self.playlist_filter_open;
            if !self.playlist_filter_open {
                self.playlist_filter.clear();
            }
        }

        // Playlist type-to-jump: when the playlist sub-window has soft
        // focus, bare-letter keys move the selection to the next entry
        // whose title starts with that letter. The bare-letter transport
        // hotkeys above already short-circuit on `playlist_focused`, so
        // pressing `n` here jumps instead of moving to the next track.
        // Captured inside the closure, dispatched outside (mutates self).
        let jump_char = if playlist_focused {
            ctx.input(|i| {
                if i.modifiers.any() {
                    return None;
                }
                for (key, c) in Self::JUMP_LETTERS {
                    if i.key_pressed(*key) {
                        return Some(*c);
                    }
                }
                None
            })
        } else {
            None
        };
        if let Some(c) = jump_char {
            self.jump_in_playlist(c);
        }

        // Shift+L / Ctrl+Shift+O: Open a folder picker. Read modifier
        // state inside the closure, open the (blocking) dialog and ingest
        // outside so we can mutate `self`. The existing `L` handler
        // requires `!i.modifiers.any()`, so Shift+L doesn't double-fire it.
        let wants_folder = ctx.input(|i| {
            let shift_l = i.key_pressed(egui::Key::L)
                && i.modifiers.shift
                && !i.modifiers.ctrl
                && !i.modifiers.alt;
            let ctrl_shift_o = i.key_pressed(egui::Key::O) && i.modifiers.ctrl && i.modifiers.shift;
            shift_l || ctrl_shift_o
        });
        if wants_folder && let Some(folder) = rfd::FileDialog::new().pick_folder() {
            // Folder picker = "Add Folder…" — append without interrupting.
            self.ingest_files(&[folder], ctx, false);
        }

        // J: jump-to-file (Winamp parity). Opens the playlist filter
        // overlay focused so the user types a fragment and presses Enter
        // to play the first match. Suppressed when the playlist has soft
        // focus — bare `j` drives type-to-jump there instead.
        let wants_jump =
            !playlist_focused && ctx.input(|i| i.key_pressed(egui::Key::J) && !i.modifiers.any());
        if wants_jump {
            self.windows.set_playlist_visible(true);
            self.playlist_filter_open = true;
            self.playlist_filter_focus_pending = true;
        }

        // Ctrl+L: open the "Add URL…" dialog (Winamp parity — Ctrl+L
        // opened the location box that accepted both file paths and
        // HTTP/Shoutcast URLs). Plain L is already taken by the
        // playlist type-to-jump handler, hence the modifier gate.
        let wants_url_dialog = ctx.input(|i| {
            i.modifiers.ctrl
                && !i.modifiers.alt
                && !i.modifiers.shift
                && i.key_pressed(egui::Key::L)
        });
        if wants_url_dialog && self.url_dialog.is_none() {
            self.url_dialog = Some(crate::url_dialog::UrlDialog::new());
        }

        // Ctrl+T: Toggle always-on-top (Winamp parity). Hoisted out of
        // the `ctx.input` closure so we can call `send_viewport_cmd`
        // alongside the state flip without re-borrowing `ctx`.
        let wants_aot_toggle = ctx.input(|i| {
            i.modifiers.ctrl
                && !i.modifiers.alt
                && !i.modifiers.shift
                && i.key_pressed(egui::Key::T)
        });
        if wants_aot_toggle {
            self.always_on_top = !self.always_on_top;
            let level = if self.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
    }

    /// Render the hotkey cheat-sheet overlay. Two columns of (key,
    /// action) rows, dark panel with the skin's accent green border,
    /// centered horizontally and pinned just under the title bar. Click
    /// anywhere outside the panel to dismiss; same with Escape or
    /// pressing F1 / `?` again.
    ///
    /// `HOTKEY_ROWS` is the single source of truth: every shortcut
    /// wired up in `handle_keyboard` lands here, in roughly the order
    /// the user would discover them (transport → file → navigation
    /// → volume / seek → windows → meta). Adding a new hotkey means
    /// editing exactly one place.
    pub(super) fn paint_hotkey_overlay(&mut self, ctx: &egui::Context) {
        const ROWS: &[(&str, &str)] = &[
            ("Z X C V B", "Prev / Play / Pause / Stop / Next"),
            ("Space", "Play / Pause"),
            ("S", "Stop"),
            ("Shift+S", "Stop after current"),
            ("N / P", "Next / Previous track"),
            ("R", "Cycle repeat"),
            ("M", "Mute toggle"),
            // The skin font shipped by base-2.91 doesn't carry the
            // U+2191..U+2193 arrow glyphs — they render as tofu boxes.
            // Spell them out so the cheat-sheet stays legible on any
            // skin, including the bundled default.
            ("Up / Down", "Volume ±5 % (Shift: ±1 %)"),
            ("Left / Right", "Seek ±5 s (Shift: ±30 s)"),
            ("L / Ctrl+O", "Open file…"),
            ("Shift+L", "Open folder…"),
            ("Ctrl+L", "Open URL…"),
            ("J", "Jump to file"),
            ("Ctrl+F", "Filter playlist"),
            ("Alt+E", "Toggle playlist"),
            ("Alt+G", "Toggle equalizer"),
            ("Alt+M", "Window shade"),
            ("Alt+S", "Load .wsz skin…"),
            ("Ctrl+T", "Toggle always on top"),
            ("Drag-drop", "Add files / folders"),
            ("F1 / ?", "This help"),
            ("Esc", "Dismiss"),
        ];

        let screen = ctx.screen_rect();
        let panel_w = (screen.width() - 8.0).clamp(240.0, 270.0);
        let row_h = 12.0;
        let n = ROWS.len() as f32;
        let panel_h = row_h * n + 18.0; // top + bottom padding
        let panel_x = screen.center().x - panel_w / 2.0;
        let panel_y = screen.min.y + 14.0; // sit under the title strip
        let panel =
            egui::Rect::from_min_size(egui::pos2(panel_x, panel_y), egui::vec2(panel_w, panel_h));

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("hotkey_overlay"),
        ));
        painter.rect_filled(
            panel,
            2.0,
            egui::Color32::from_rgba_unmultiplied(15, 15, 15, 240),
        );
        painter.rect_stroke(
            panel,
            2.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 220, 100)),
        );

        // Title strip at the top of the panel.
        let title_pos = egui::pos2(panel.center().x, panel.min.y + 7.0);
        painter.text(
            title_pos,
            egui::Align2::CENTER_CENTER,
            "OneAmp keyboard shortcuts",
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgb(180, 240, 180),
        );

        // Two columns: keys on the left, actions on the right, with a
        // ratio that keeps long action labels readable. Each row is
        // painted independently so the panel survives skin-font absence.
        let key_col_x = panel.min.x + 10.0;
        let action_col_x = panel.min.x + 92.0;
        for (i, (key, action)) in ROWS.iter().enumerate() {
            let y = panel.min.y + 16.0 + i as f32 * row_h + row_h / 2.0;
            painter.text(
                egui::pos2(key_col_x, y),
                egui::Align2::LEFT_CENTER,
                *key,
                egui::FontId::proportional(9.0),
                egui::Color32::from_rgb(120, 220, 120),
            );
            painter.text(
                egui::pos2(action_col_x, y),
                egui::Align2::LEFT_CENTER,
                *action,
                egui::FontId::proportional(9.0),
                egui::Color32::from_rgb(220, 220, 220),
            );
        }

        // Click-outside-to-dismiss: detect a fresh primary-button press
        // this frame and dismiss if its position fell outside the panel.
        // Cheaper than spinning up an Area + Sense::click for the whole
        // viewport, and lets the rest of the player keep handling its
        // own clicks normally.
        let outside_click = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .map(|p| !panel.contains(p))
                    .unwrap_or(false)
        });
        if outside_click {
            self.show_hotkeys = false;
        }
    }

    /// Handle file drops. Files with an audio extension are added to the
    /// playlist; folders are walked recursively (up to `FOLDER_WALK_MAX_DEPTH`)
    /// for audio files.
    pub(super) fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            // Drag-drop = silent append in Winamp; we mirror that. Use the
            // file manager's double-click (which goes through IPC) when
            // you want the new file to actually start playing.
            self.ingest_files(&dropped, ctx, false);
        }
    }
}
