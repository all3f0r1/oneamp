//! Unified menu-event dispatch shared by the macOS menu bar and the
//! cross-OS system tray.
//!
//! Both subsystems register their items into `self.menu_bindings`
//! (built in `OneAmpApp::new`) and clicks from both emerge on the
//! same global `tray_icon::menu::MenuEvent::receiver()` channel
//! (muda re-exports through tray_icon). Draining the channel in one
//! place keeps either subsystem from accidentally consuming the
//! other's events and dropping them.
//!
//! In addition to menu clicks we also poll `TrayIconEvent::receiver()`
//! here, so a left-click on the tray icon itself focuses the OneAmp
//! window — a convention users expect from media players.

use super::OneAmpApp;
use crate::platform::menu_bar::MenuCommand;
use eframe::egui;

impl OneAmpApp {
    /// Drain every pending menu / tray event and route it through the
    /// existing app handlers. Called once per frame from the update
    /// loop, after the main-window / playlist actions have already
    /// been processed (so menu state reflects the current frame's
    /// outcome).
    pub(super) fn dispatch_menu_events(&mut self, ctx: &egui::Context) {
        // Menu items (macOS menu bar AND tray context menu).
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if let Some(cmd) = self.menu_bindings.get(&event.id).cloned() {
                self.dispatch_menu_command(cmd, ctx);
            }
        }

        // Direct tray-icon clicks (left-click on the icon itself, not
        // a menu item). Right-clicks are handled by the OS — they pop
        // up the menu we registered with the icon, no work here.
        while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if let tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Down,
                ..
            } = event
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }

    fn dispatch_menu_command(&mut self, cmd: MenuCommand, ctx: &egui::Context) {
        match cmd {
            MenuCommand::MainWindow(a) => self.handle_main_window_action(a, ctx),
            MenuCommand::Playlist(a) => self.handle_playlist_action(a),
            MenuCommand::Audio(c) => self.audio.send_command(c),
            MenuCommand::TogglePlayback => self.toggle_playback(),
            MenuCommand::ShowWindow => {
                // Focus brings the window to the foreground even when
                // it's minimised or behind another window — what the
                // user expects from "Show OneAmp" on the tray menu.
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }
}
