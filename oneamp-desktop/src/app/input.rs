//! Input dispatch: main-window menu actions, keyboard shortcuts,
//! the F1 hotkey cheat-sheet overlay and the drag-drop ingest entry
//! point.
//!
//! All four are pure "input → app mutation" surfaces. Splitting them
//! out of `app/mod.rs` keeps the keyboard hotkey matrix and the
//! Options-menu fan-out in one file, easier to audit when adding a
//! new shortcut or menu entry.

use super::{OneAmpApp, keymap};
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
            MainWindowAction::OpenFile => self.open_files_replace(),
            MainWindowAction::OpenFolder => self.add_folder(),
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
            MainWindowAction::ToggleEqAuto => self.toggle_eq_auto(),
            MainWindowAction::ShowPreferences => self.preferences_open = true,
            MainWindowAction::JumpToFile => self.open_jump_dialog(),
            MainWindowAction::SaveEqAutoPreset => self.save_eq_auto_preset(),
            MainWindowAction::RemoveEqAutoPreset => self.remove_eq_auto_preset(),
            MainWindowAction::ToggleDetachedWindows => {
                if super::detached_windows_supported() {
                    let on = !self.windows.is_detached();
                    self.windows.set_detached(on);
                } else {
                    self.push_toast(
                        "Detached windows need X11 (Wayland can't place windows)",
                        std::time::Duration::from_millis(3000),
                    );
                }
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
            MainWindowAction::ToggleStopAfterCurrent => self.toggle_stop_after_current(),
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
            }
            MainWindowAction::ToggleDoubleSize => {
                // Winamp's double-size toggle: 2× when off, back to the
                // DPI-auto scale when already doubled. We treat "currently
                // pinned at exactly 2×" as the doubled state; any other
                // value (auto, or a user-chosen 1×/3×/4×) doubles to 2×.
                let already_doubled = self.user_scale == Some(2.0);
                self.user_scale = if already_doubled { None } else { Some(2.0) };
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

    /// Keyboard shortcuts: resolve each key press through the active
    /// profile's table (`keymap`), focused window first. Nothing fires
    /// while a text field has focus.
    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let scope = self.windows.focused_scope();
        let chords: Vec<keymap::Chord> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some(keymap::Chord {
                        key: *key,
                        ctrl: modifiers.ctrl || modifiers.mac_cmd,
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                    }),
                    _ => None,
                })
                .collect()
        });
        for chord in chords {
            let bare = !chord.ctrl && !chord.alt && !chord.shift;
            if bare && self.nullsoft_egg(chord.key) {
                continue;
            }
            // Legacy profile: bare letters in the playlist jump to the
            // next title starting with that letter.
            if bare
                && scope == keymap::Scope::Playlist
                && self.config.key_profile == keymap::KeyProfile::OneAmpLegacy
                && let Some((_, c)) = Self::JUMP_LETTERS.iter().find(|(k, _)| *k == chord.key)
            {
                self.jump_in_playlist(*c);
                continue;
            }
            if chord.key == egui::Key::Escape && (self.cancel_import() || self.show_hotkeys) {
                self.show_hotkeys = false;
                continue;
            }
            if let Some(cmd) = keymap::resolve(&self.keymap, chord, scope).cloned() {
                self.run_command(cmd, ctx);
            }
        }
    }

    /// Nullsoft easter egg: N-U-L typed quickly flashes a toast. Returns
    /// true when `key` completed it, so the final L doesn't also open a
    /// file.
    fn nullsoft_egg(&mut self, key: egui::Key) -> bool {
        let now = std::time::Instant::now();
        if self.nul_deadline.is_none_or(|d| now > d) {
            self.nul_progress = 0;
        }
        let bump = std::time::Duration::from_millis(1500);
        match (key, self.nul_progress) {
            (egui::Key::N, _) => {
                self.nul_progress = 1;
                self.nul_deadline = Some(now + bump);
            }
            (egui::Key::U, 1) => {
                self.nul_progress = 2;
                self.nul_deadline = Some(now + bump);
            }
            (egui::Key::L, 2) => {
                self.nul_progress = 0;
                self.push_toast("Nullsoft!", std::time::Duration::from_millis(2000));
                return true;
            }
            _ => self.nul_progress = 0,
        }
        false
    }

    /// Execute a keyboard command. Menu-equivalent commands go through
    /// `handle_main_window_action` so both paths stay identical.
    pub(super) fn run_command(&mut self, cmd: keymap::Command, ctx: &egui::Context) {
        use keymap::Command as C;
        match cmd {
            C::Action(a) => self.handle_main_window_action(a, ctx),
            C::Prev => {
                let prev = self.playlist.previous_entry().map(|e| e.path.clone());
                if let Some(path) = prev {
                    self.play_audio_path(path);
                }
            }
            C::Next => {
                let next = self.playlist.next_entry().map(|e| e.path.clone());
                if let Some(path) = next {
                    self.play_audio_path(path);
                }
            }
            C::Play => self.transport_play(),
            C::Pause => self.transport_pause(),
            C::PlayPause => self.toggle_playback(),
            C::Stop => self.transport_stop(),
            C::VolumeUp { fine } | C::VolumeDown { fine } => {
                let step = if fine { 0.01 } else { 0.05 };
                let step = if matches!(cmd, C::VolumeUp { .. }) {
                    step
                } else {
                    -step
                };
                // Optimistic local update so held-key repeats keep
                // stepping before the engine echoes back.
                let new_vol = (self.state.volume.level + step).clamp(0.0, oneamp_core::MAX_VOLUME);
                self.state.volume.level = new_vol;
                self.audio.send_command(AudioCommand::SetVolume(new_vol));
            }
            C::SeekForward { long } | C::SeekBack { long } => {
                let step = if long { 30.0 } else { 5.0 };
                let (cur, total) = self.state.position;
                // Idle engine reports total = 0; stay shy of the end so
                // the decoder isn't asked for a half frame.
                if total > 0.5 {
                    let new_pos = if matches!(cmd, C::SeekForward { .. }) {
                        (cur + step).min(total - 0.5)
                    } else {
                        (cur - step).max(0.0)
                    };
                    self.audio.send_command(AudioCommand::Seek(new_pos));
                }
            }
            C::Mute => {
                let muted = !self.state.volume.muted;
                self.state.volume.muted = muted;
                self.audio.send_command(AudioCommand::SetMute(muted));
            }
            C::JumpToFile => self.open_jump_dialog(),
            C::Preferences => self.preferences_open = true,
            C::OpenUrl => {
                if self.url_dialog.is_none() {
                    self.url_dialog = Some(crate::url_dialog::UrlDialog::new());
                }
            }
            C::FilterPlaylist => {
                // A filter the user can't see is pure friction.
                self.windows.set_playlist_visible(true);
                self.playlist_filter_open = !self.playlist_filter_open;
                self.playlist_filter_focus_pending = self.playlist_filter_open;
                if !self.playlist_filter_open {
                    self.playlist_filter.clear();
                }
            }
            C::ToggleTimeDisplay => {
                let main = self.windows.main_window_mut();
                let remaining = main.show_remaining();
                main.set_show_remaining(!remaining);
            }
            C::Undo => self.undo_playlist(),
            C::SelectBy { delta, extend } => {
                let n = self.playlist.len();
                if n == 0 {
                    return;
                }
                // Shift+arrows move a cursor away from the anchor.
                let from = self
                    .pl_cursor
                    .filter(|&c| c < n && extend)
                    .or(self.playlist.selected_index())
                    .or(self.playlist.current_index())
                    .unwrap_or(0);
                let to = from.saturating_add_signed(delta).min(n - 1);
                self.pl_cursor = Some(to);
                if extend {
                    self.playlist.extend_selected_to(to);
                } else {
                    self.playlist.set_selected(to);
                }
                self.windows.scroll_playlist_to(to);
            }
            C::SelectEdge { end } => {
                let n = self.playlist.len();
                if n > 0 {
                    let to = if end { n - 1 } else { 0 };
                    self.playlist.set_selected(to);
                    self.windows.scroll_playlist_to(to);
                }
            }
            C::MoveSelection { up } => self.move_selection(up),
            C::PlaySelected => {
                if let Some(idx) = self.playlist.selected_index() {
                    self.handle_playlist_action(PlaylistAction::PlayTrack(idx));
                }
            }
            C::RemoveSelected => self.handle_playlist_action(PlaylistAction::RemoveSelected),
            C::SelectAll => self.handle_playlist_action(PlaylistAction::SelectAll),
            C::QueueSelected => {
                let selected: Vec<usize> =
                    self.playlist.selected_indices().iter().copied().collect();
                for idx in selected {
                    self.playlist.toggle_queued(idx);
                }
            }
            C::EqBand { band, up } => {
                if let Some(g) = self.state.equalizer.gains.get(band).copied() {
                    let step = if up { 1.0 } else { -1.0 };
                    let g = (g + step).clamp(-oneamp_core::EQ_MAX_DB, oneamp_core::EQ_MAX_DB);
                    self.audio
                        .send_command(AudioCommand::SetEqualizerBand(band, g));
                    self.windows.set_equalizer_current_preset(None);
                }
            }
            C::EqPreamp { up } => {
                let step = if up { 1.0 } else { -1.0 };
                let db = (self.state.equalizer.preamp_db + step)
                    .clamp(-oneamp_core::EQ_MAX_DB, oneamp_core::EQ_MAX_DB);
                self.audio
                    .send_command(AudioCommand::SetEqualizerPreamp(db));
            }
            C::EqToggle => {
                self.audio.send_command(AudioCommand::SetEqualizerEnabled(
                    !self.state.equalizer.enabled,
                ));
            }
            C::EqAuto => self.toggle_eq_auto(),
        }
    }

    /// Open Winamp's Jump to file box over a snapshot of the playlist.
    pub(super) fn open_jump_dialog(&mut self) {
        let fmt = &self.config.playlist_display_format;
        let rows = self
            .playlist
            .entries()
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.format_display(fmt)))
            .collect();
        self.jump_dialog = Some(crate::jump_dialog::JumpDialog::new(
            rows,
            self.playlist.current_index(),
        ));
    }

    /// Shift every selected entry one row up or down, keeping the
    /// selection on them. Blocked at the edges so the block keeps shape.
    fn move_selection(&mut self, up: bool) {
        let selected: Vec<usize> = self.playlist.selected_indices().iter().copied().collect();
        let n = self.playlist.len();
        let blocked = if up {
            selected.first() == Some(&0)
        } else {
            selected.last() == Some(&(n.saturating_sub(1)))
        };
        if selected.is_empty() || blocked {
            return;
        }
        self.remember_for_undo();
        if up {
            for &i in &selected {
                self.playlist.move_entry(i, i - 1);
            }
        } else {
            for &i in selected.iter().rev() {
                self.playlist.move_entry(i, i + 1);
            }
        }
        if let Some(&first) = self.playlist.selected_indices().iter().next() {
            self.windows.scroll_playlist_to(first);
        }
    }

    /// Keyboard cheat-sheet, generated from the active keymap, in its
    /// own OS window so it isn't clipped by the 275×116 player.
    pub(super) fn show_hotkey_window(&mut self, ctx: &egui::Context) {
        let rows = keymap::help_rows(self.config.key_profile);
        let profile = match self.config.key_profile {
            keymap::KeyProfile::WinampClassic => "Winamp Classic",
            keymap::KeyProfile::OneAmpLegacy => "OneAmp 1.0",
        };
        let mut open = true;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("oneamp_hotkeys"),
            egui::ViewportBuilder::default()
                .with_title("OneAmp — Keyboard shortcuts")
                .with_inner_size([440.0, 520.0]),
            |vctx, _| {
                crate::dialog_util::apply_native_ppp(vctx);
                if vctx
                    .input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
                {
                    open = false;
                }
                egui::CentralPanel::default().show(vctx, |ui| {
                    ui.label(format!("Profile: {profile} (Preferences > Shortcuts)"));
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (scope, title) in [
                            (keymap::Scope::Global, "Everywhere"),
                            (keymap::Scope::Playlist, "Playlist window"),
                            (keymap::Scope::Equalizer, "Equalizer window"),
                        ] {
                            ui.add_space(6.0);
                            ui.strong(title);
                            egui::Grid::new(title).striped(true).show(ui, |ui| {
                                for (s, keys, label) in &rows {
                                    if *s == scope {
                                        ui.monospace(keys);
                                        ui.label(*label);
                                        ui.end_row();
                                    }
                                }
                            });
                        }
                    });
                });
            },
        );
        self.show_hotkeys = open;
    }

    /// Handle file drops. Files with an audio extension are added to the
    /// playlist; folders are walked recursively (up to `FOLDER_WALK_MAX_DEPTH`)
    /// for audio files.
    pub(super) fn handle_drops(&mut self, ctx: &egui::Context) {
        let mut dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        // Drops onto a detached EQ / playlist window.
        dropped.extend(self.windows.take_forwarded_drops());
        if !dropped.is_empty() {
            // Drag-drop = silent append in Winamp; we mirror that. Use the
            // file manager's double-click (which goes through IPC) when
            // you want the new file to actually start playing.
            self.start_import(dropped, false);
        }
    }
}
