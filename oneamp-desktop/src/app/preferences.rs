//! Preferences window (`Ctrl+P`): every option in one place, grouped
//! like Winamp's (General, Playback, Playlist, Equalizer, Appearance,
//! Shortcuts). Toggles that also live in menus go through the same
//! `MainWindowAction`s so both stay in sync.

use super::{OneAmpApp, keymap};
use crate::windows::MainWindowAction as A;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum PrefsTab {
    #[default]
    General,
    Playback,
    Playlist,
    Equalizer,
    Appearance,
    Shortcuts,
}

impl OneAmpApp {
    pub(super) fn show_preferences(&mut self, ctx: &egui::Context) {
        let mut actions: Vec<A> = Vec::new();
        let mut open = true;
        let mut rename: Option<(String, String)> = None;
        let mut delete: Option<String> = None;
        let mut new_profile = self.config.key_profile;
        let devices = self.refresh_output_devices().to_vec();

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("oneamp_preferences"),
            egui::ViewportBuilder::default()
                .with_title("OneAmp — Preferences")
                .with_inner_size([520.0, 420.0]),
            |vctx, _| {
                crate::dialog_util::apply_native_ppp(vctx);
                if vctx.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape))
                {
                    open = false;
                }
                egui::SidePanel::left("prefs_tabs")
                    .resizable(false)
                    .exact_width(110.0)
                    .show(vctx, |ui| {
                        for (tab, label) in [
                            (PrefsTab::General, "General"),
                            (PrefsTab::Playback, "Playback"),
                            (PrefsTab::Playlist, "Playlist"),
                            (PrefsTab::Equalizer, "Equalizer"),
                            (PrefsTab::Appearance, "Appearance"),
                            (PrefsTab::Shortcuts, "Shortcuts"),
                        ] {
                            ui.selectable_value(&mut self.prefs_tab, tab, label);
                        }
                    });
                egui::CentralPanel::default().show(vctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| match self.prefs_tab {
                        PrefsTab::General => {
                            let mut detached = self.windows.is_detached();
                            if ui
                                .checkbox(&mut detached, "Detached windows (move EQ and playlist freely)")
                                .changed()
                            {
                                actions.push(A::ToggleDetachedWindows);
                            }
                            let mut aot = self.always_on_top;
                            if ui.checkbox(&mut aot, "Always on top").changed() {
                                actions.push(A::ToggleAlwaysOnTop);
                            }
                            let mut notif = self.config.track_notifications_enabled;
                            if ui.checkbox(&mut notif, "Notify on track change").changed() {
                                actions.push(A::ToggleTrackNotifications);
                            }
                            ui.add_space(8.0);
                            if ui.button("Check for updates").clicked() {
                                actions.push(A::CheckForUpdates);
                            }
                        }
                        PrefsTab::Playback => {
                            let mut xfade = self.config.crossfade.enabled;
                            if ui.checkbox(&mut xfade, "Crossfade between tracks").changed() {
                                actions.push(A::ToggleCrossfade);
                            }
                            let mut mono = self.config.mono_enabled;
                            if ui.checkbox(&mut mono, "Mono").changed() {
                                actions.push(A::ToggleMono);
                            }
                            let mut loud = self.config.loudness_enabled;
                            if ui.checkbox(&mut loud, "Loudness compensation").changed() {
                                actions.push(A::ToggleLoudness);
                            }
                            let mut resume = self.config.resume_long_files;
                            if ui
                                .checkbox(&mut resume, "Resume long files (> 30 min) where they stopped")
                                .changed()
                            {
                                actions.push(A::ToggleResumeLongFiles);
                            }
                            let mut stop_after = self.stop_after_current;
                            if ui.checkbox(&mut stop_after, "Stop after current track").changed() {
                                actions.push(A::ToggleStopAfterCurrent);
                            }
                            ui.add_space(6.0);
                            use oneamp_core::ReplayGainMode as Rg;
                            let rg = if self.config.replaygain_enabled {
                                self.config.replaygain_mode
                            } else {
                                Rg::Off
                            };
                            egui::ComboBox::from_label("ReplayGain")
                                .selected_text(format!("{rg:?}"))
                                .show_ui(ui, |ui| {
                                    for m in [Rg::Off, Rg::Track, Rg::Album, Rg::Auto] {
                                        if ui.selectable_label(rg == m, format!("{m:?}")).clicked() {
                                            actions.push(A::SetReplayGainMode(m));
                                        }
                                    }
                                });
                            let current = self.config.output_device_name.clone();
                            egui::ComboBox::from_label("Output device")
                                .selected_text(current.clone().unwrap_or_else(|| "Default".into()))
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(current.is_none(), "Default").clicked() {
                                        actions.push(A::SelectOutputDevice(None));
                                    }
                                    for d in &devices {
                                        let on = current.as_deref() == Some(d.as_str());
                                        if ui.selectable_label(on, d).clicked() {
                                            actions.push(A::SelectOutputDevice(Some(d.clone())));
                                        }
                                    }
                                });
                        }
                        PrefsTab::Playlist => {
                            ui.label("The playlist, current track, queue and position are restored at launch.");
                            ui.label("Playback never starts on its own.");
                            ui.add_space(6.0);
                            ui.label(format!(
                                "Row format: {}",
                                self.config.playlist_display_format
                            ));
                            if ui.button("Change row format…").clicked()
                                && self.format_dialog.is_none()
                            {
                                self.format_dialog = Some(crate::format_dialog::FormatDialog::new(
                                    &self.config.playlist_display_format,
                                ));
                            }
                        }
                        PrefsTab::Equalizer => {
                            let mut auto = self.config.equalizer.auto;
                            if ui
                                .checkbox(&mut auto, "AUTO: load each track's auto-load preset")
                                .changed()
                            {
                                actions.push(A::ToggleEqAuto);
                            }
                            ui.label(format!(
                                "{} track(s) have an auto-load preset (PRESETS > Auto-load for this track).",
                                self.config.equalizer.auto_presets.len()
                            ));
                            ui.add_space(8.0);
                            ui.strong("Your presets");
                            let names: Vec<String> = self
                                .preset_manager
                                .custom_presets()
                                .iter()
                                .map(|p| p.name.clone())
                                .collect();
                            if names.is_empty() {
                                ui.label("None yet — PRESETS > Save as preset…");
                            }
                            for name in names {
                                ui.horizontal(|ui| {
                                    let editing = self.prefs_rename.as_ref().is_some_and(|(o, _)| o == &name);
                                    if editing {
                                        let (_, buf) = self.prefs_rename.as_mut().unwrap();
                                        ui.text_edit_singleline(buf);
                                        if ui.button("OK").clicked() {
                                            rename = self.prefs_rename.take();
                                        }
                                    } else {
                                        ui.label(&name);
                                        if ui.small_button("Rename").clicked() {
                                            self.prefs_rename = Some((name.clone(), name.clone()));
                                        }
                                        if ui.small_button("Delete").clicked() {
                                            delete = Some(name.clone());
                                        }
                                    }
                                });
                            }
                        }
                        PrefsTab::Appearance => {
                            if ui.button("Skins…").clicked() {
                                actions.push(A::PickSkin);
                            }
                            ui.add_space(6.0);
                            egui::ComboBox::from_label("Scale")
                                .selected_text(match self.user_scale {
                                    None => "Auto (DPI)".to_string(),
                                    Some(s) => format!("{s:.0}×"),
                                })
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(self.user_scale.is_none(), "Auto (DPI)").clicked() {
                                        actions.push(A::SetUserScale(None));
                                    }
                                    for s in [1.0_f32, 2.0, 3.0, 4.0] {
                                        let on = self.user_scale.is_some_and(|u| (u - s).abs() < 0.01);
                                        if ui.selectable_label(on, format!("{s:.0}×")).clicked() {
                                            actions.push(A::SetUserScale(Some(s)));
                                        }
                                    }
                                });
                            let mut shade = self.windows.is_shade_mode();
                            if ui.checkbox(&mut shade, "Mini mode (window shade)").changed() {
                                actions.push(A::ToggleShade);
                            }
                            if ui.button("Language and welcome screen…").clicked() {
                                actions.push(A::ShowWelcome);
                            }
                        }
                        PrefsTab::Shortcuts => {
                            ui.radio_value(
                                &mut new_profile,
                                keymap::KeyProfile::WinampClassic,
                                "Winamp Classic (Z X C V B, S shuffle, R repeat, J / F3 jump)",
                            );
                            ui.radio_value(
                                &mut new_profile,
                                keymap::KeyProfile::OneAmpLegacy,
                                "OneAmp 1.0 (N / P / S transport, letters jump in the playlist)",
                            );
                            ui.add_space(6.0);
                            if ui.button("Show all shortcuts (F1)").clicked() {
                                actions.push(A::ShowHotkeys);
                            }
                        }
                    });
                });
            },
        );

        self.preferences_open = open;
        if new_profile != self.config.key_profile {
            self.config.key_profile = new_profile;
            self.keymap = keymap::bindings(new_profile);
            self.mark_dirty();
        }
        if let Some((old, new)) = rename {
            self.rename_user_preset(&old, new.trim());
        }
        if let Some(name) = delete {
            self.preset_manager.remove_preset(&name);
            self.save_preset_store();
        }
        for a in actions {
            self.handle_main_window_action(a, ctx);
        }
    }

    fn rename_user_preset(&mut self, old: &str, new: &str) {
        if new.is_empty() || new == old {
            return;
        }
        if self.preset_manager.preset_exists(new)
            || oneamp_core::BuiltinPresets::get_by_name(new).is_some()
        {
            self.push_toast(
                format!("\"{new}\" already exists"),
                std::time::Duration::from_millis(2000),
            );
            return;
        }
        if let Some(mut p) = self.preset_manager.remove_preset(old) {
            p.name = new.to_string();
            let _ = self.preset_manager.add_preset(p);
            self.save_preset_store();
        }
    }
}
