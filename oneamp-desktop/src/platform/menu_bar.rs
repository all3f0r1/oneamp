//! Native macOS menu bar via Tauri's `muda` crate.
//!
//! macOS HIG mandates a menu bar in the top-of-screen system strip;
//! eframe / egui draws nothing there on its own, so we attach a `muda`
//! menu at app start. Win/Linux keep the bare Winamp chrome — the
//! historical Winamp aesthetic uses right-click menus instead, and a
//! hosted menu bar would jar with the custom skin chrome.
//!
//! Event flow:
//!   user clicks menu / hits ⌘O
//!     → AppKit dispatches into muda
//!     → muda posts a `MenuEvent` onto its static channel
//!     → `MacMenuBar::poll` drains that channel each frame and
//!       returns the `MenuCommand`s the app should execute
//!     → app dispatches into the existing
//!       `handle_main_window_action` / `handle_playlist_action`
//!       paths, so the menu shares one code path with the Winamp
//!       clutterbar buttons and the keyboard shortcuts.

use crate::windows::{MainWindowAction, PlaylistAction};
use oneamp_core::AudioCommand;

/// What a menu item asks the app to do. Wraps the pre-existing action
/// vocabularies so the menu doesn't introduce a third dispatcher —
/// the app reuses the same handlers the clutterbar buttons and
/// keyboard shortcuts go through.
///
/// * `MainWindow` / `Playlist` — high-level UI intents already handled
///   by `handle_main_window_action` / `handle_playlist_action`.
/// * `Audio` — raw engine command for actions that have no UI side
///   effect (Previous / Next bypass the main-window action vocabulary
///   and go straight to `audio.send_command(...)`).
/// * `TogglePlayback` — smart Play/Pause that mirrors the Space-key
///   logic in `input.rs` (Playing → Pause, Paused → Resume, Stopped →
///   start current entry). The app handles this in one place so the
///   menu and the keyboard never disagree.
// `#[allow(dead_code)]`: on non-macOS the `ShowWindow` and other tray
// variants are constructed; on macOS the `MainWindow` / `Playlist` /
// `Audio` / `TogglePlayback` ones are. Compiler can't see the full
// picture per platform, so flag the whole enum to bypass the warning.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum MenuCommand {
    MainWindow(MainWindowAction),
    Playlist(PlaylistAction),
    Audio(AudioCommand),
    /// Smart Play/Pause: toggles based on current playback state.
    /// Handled by `OneAmpApp::toggle_playback` so the menu agrees with
    /// the Space-key shortcut.
    TogglePlayback,
    /// Bring the OneAmp window to the foreground. Emitted by the tray
    /// icon (left-click and the "Show OneAmp" menu item); dispatched
    /// as `ViewportCommand::Focus` so it works regardless of whether
    /// the user minimized or sent the window behind something else.
    ShowWindow,
}

/// `MenuId` is muda's per-item handle. It's the SAME type whether it
/// comes from `muda` directly (macOS menu bar) or from `tray_icon::menu`
/// (cross-OS tray re-exports muda), so the app can keep a single
/// `HashMap<MenuId, MenuCommand>` keyed by it and look up clicks from
/// either subsystem in one drain pass — important because both fire on
/// the same global `MenuEvent::receiver()` channel.
pub use tray_icon::menu::MenuId;

#[cfg(target_os = "macos")]
pub use macos::MacMenuBar;

#[cfg(not(target_os = "macos"))]
pub struct MacMenuBar;

#[cfg(not(target_os = "macos"))]
impl MacMenuBar {
    pub fn install(_bindings: &mut std::collections::HashMap<MenuId, MenuCommand>) -> Option<Self> {
        None
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{AudioCommand, MainWindowAction, MenuCommand, MenuId, PlaylistAction};
    use muda::{
        Menu, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    };
    use std::collections::HashMap;

    /// Installs the macOS menu bar. Item-id → command bindings are
    /// merged into the caller-owned `bindings` map so the app can
    /// drain `MenuEvent::receiver()` once per frame and dispatch
    /// regardless of which subsystem (menu bar / tray) emitted the
    /// event. We hold the `Menu` only to keep it alive — muda owns
    /// the underlying NSMenu via a global strong ref but dropping our
    /// side decrements the count and tears it down.
    pub struct MacMenuBar {
        _menu: Menu,
    }

    impl MacMenuBar {
        /// Build the menu, register every clickable item in `bindings`,
        /// and install the result as the macOS app menu bar. Returns
        /// `None` if muda failed to talk to AppKit (e.g. headless CI)
        /// — the rest of the app keeps working unchanged.
        pub fn install(bindings: &mut HashMap<MenuId, MenuCommand>) -> Option<Self> {
            let menu = Menu::new();

            // === Application menu (always first on macOS) ===
            // Convention: "About / Hide / Hide Others / Show All / Quit"
            // — the predefined items below pull localised AppKit strings
            // automatically so the menu reads correctly in any system
            // language.
            let app_menu = Submenu::new("OneAmp", true);
            let about = MenuItem::new("About OneAmp", true, None);
            bindings.insert(
                about.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::ShowAbout),
            );
            app_menu
                .append_items(&[
                    &about,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(None),
                ])
                .ok()?;

            // === File ===
            let file_menu = Submenu::new("File", true);
            let open_file = MenuItem::new(
                "Open File…",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyO)),
            );
            bindings.insert(
                open_file.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::OpenFile),
            );
            let open_folder = MenuItem::new(
                "Open Folder…",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::SUPER | Modifiers::SHIFT),
                    Code::KeyO,
                )),
            );
            bindings.insert(
                open_folder.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::OpenFolder),
            );
            let open_url = MenuItem::new(
                "Open URL…",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyL)),
            );
            bindings.insert(
                open_url.id().clone(),
                MenuCommand::Playlist(PlaylistAction::AddUrl),
            );
            let load_playlist = MenuItem::new("Load Playlist…", true, None);
            bindings.insert(
                load_playlist.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::LoadPlaylist),
            );
            let save_playlist = MenuItem::new(
                "Save Playlist…",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyS)),
            );
            bindings.insert(
                save_playlist.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::SavePlaylist),
            );
            let clear_playlist = MenuItem::new("Clear Playlist", true, None);
            bindings.insert(
                clear_playlist.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::ClearPlaylist),
            );
            file_menu
                .append_items(&[
                    &open_file,
                    &open_folder,
                    &open_url,
                    &PredefinedMenuItem::separator(),
                    &load_playlist,
                    &save_playlist,
                    &clear_playlist,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::close_window(None),
                ])
                .ok()?;

            // === Playback ===
            // No Space accelerator on Play/Pause: Space is needed for
            // text input in the welcome screen and sub-viewport
            // dialogs, and AppKit would consume it before egui sees
            // it. Egui already binds Space globally in `input.rs`, so
            // the menu only offers the click path.
            let playback_menu = Submenu::new("Playback", true);
            let play_pause = MenuItem::new("Play / Pause", true, None);
            bindings.insert(play_pause.id().clone(), MenuCommand::TogglePlayback);
            let previous = MenuItem::new(
                "Previous",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::ArrowLeft)),
            );
            bindings.insert(
                previous.id().clone(),
                MenuCommand::Audio(AudioCommand::Previous),
            );
            let next = MenuItem::new(
                "Next",
                true,
                Some(Accelerator::new(Some(Modifiers::SUPER), Code::ArrowRight)),
            );
            bindings.insert(next.id().clone(), MenuCommand::Audio(AudioCommand::Next));
            let shuffle = MenuItem::new("Shuffle", true, None);
            bindings.insert(
                shuffle.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::ToggleShuffle),
            );
            let repeat = MenuItem::new("Cycle Repeat", true, None);
            bindings.insert(
                repeat.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::CycleRepeat),
            );
            playback_menu
                .append_items(&[
                    &play_pause,
                    &previous,
                    &next,
                    &PredefinedMenuItem::separator(),
                    &shuffle,
                    &repeat,
                ])
                .ok()?;

            // === View ===
            let view_menu = Submenu::new("View", true);
            let toggle_eq = MenuItem::new("Equalizer", true, None);
            bindings.insert(
                toggle_eq.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::ToggleEqualizer),
            );
            let toggle_pl = MenuItem::new("Playlist", true, None);
            bindings.insert(
                toggle_pl.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::TogglePlaylist),
            );
            let always_on_top = MenuItem::new("Always on Top", true, None);
            bindings.insert(
                always_on_top.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::ToggleAlwaysOnTop),
            );
            let pick_skin = MenuItem::new(
                "Choose Skin…",
                true,
                Some(Accelerator::new(Some(Modifiers::ALT), Code::KeyS)),
            );
            bindings.insert(
                pick_skin.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::PickSkin),
            );
            view_menu
                .append_items(&[
                    &toggle_eq,
                    &toggle_pl,
                    &PredefinedMenuItem::separator(),
                    &always_on_top,
                    &pick_skin,
                ])
                .ok()?;

            // === Help ===
            let help_menu = Submenu::new("Help", true);
            let hotkeys = MenuItem::new(
                "Keyboard Shortcuts",
                true,
                Some(Accelerator::new(None, Code::F1)),
            );
            bindings.insert(
                hotkeys.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::ShowHotkeys),
            );
            let check_updates = MenuItem::new("Check for Updates…", true, None);
            bindings.insert(
                check_updates.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::CheckForUpdates),
            );
            let show_welcome = MenuItem::new("Show Welcome…", true, None);
            bindings.insert(
                show_welcome.id().clone(),
                MenuCommand::MainWindow(MainWindowAction::ShowWelcome),
            );
            help_menu
                .append_items(&[&hotkeys, &check_updates, &show_welcome])
                .ok()?;

            menu.append_items(&[
                &app_menu,
                &file_menu,
                &playback_menu,
                &view_menu,
                &help_menu,
            ])
            .ok()?;

            // `init_for_nsapp` installs the menu as the app menu bar.
            // Must run on the main thread (AppKit requirement) — eframe
            // guarantees we are on the main thread inside `App::new`.
            menu.init_for_nsapp();

            Some(Self { _menu: menu })
        }
    }
}
