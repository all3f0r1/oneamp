//! Cross-platform system-tray / status-bar icon via Tauri's
//! `tray-icon` crate.
//!
//! Backends per OS:
//!
//! | OS       | Native API                                         | Event-loop requirement |
//! |----------|----------------------------------------------------|------------------------|
//! | Windows  | `Shell_NotifyIcon`                                 | Main thread (HWND msg pump shared with eframe). |
//! | macOS    | `NSStatusBar` / `NSStatusItem`                     | Main thread (NSApp loop shared with eframe). |
//! | Linux    | `StatusNotifierItem` via `libayatana-appindicator` | Dedicated GTK thread (eframe owns winit, GTK gets its own). |
//!
//! Menu items registered through this module merge their IDs into a
//! caller-owned `HashMap<MenuId, MenuCommand>` that the app drains
//! each frame off the single global `MenuEvent::receiver()` channel
//! (shared with `platform::menu_bar`). Routing both subsystems
//! through one receiver avoids the race where each would `try_recv`
//! the other's events and drop them.
//!
//! On Linux we spawn a dedicated GTK thread because:
//!   1. eframe drives winit on the main thread, which has no GTK
//!      integration; libappindicator needs a running `gtk::main()`
//!      to fire its callbacks.
//!   2. `muda::Menu` and `tray_icon::TrayIcon` are not `Send` on
//!      Linux (their gtk-rs backing types are thread-bound). Both
//!      must be created *inside* the GTK thread.
//!   3. The bindings map IS `Send`, so we ship it back to the main
//!      thread over a one-shot channel after the menu is built.

use crate::platform::menu_bar::{MenuCommand, MenuId};
use crate::windows::MainWindowAction;
use oneamp_core::AudioCommand;
use std::collections::HashMap;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

/// Keeps the tray icon alive for the lifetime of the app.
///
/// * Win / macOS — directly owns the `TrayIcon` handle. Dropping
///   removes the icon from the system tray.
/// * Linux — the icon and the GTK loop live on a detached thread;
///   this side only holds the channel the thread ships its menu
///   bindings back on. The process exiting collapses the GTK thread.
pub struct TrayService {
    #[cfg(not(target_os = "linux"))]
    _icon: tray_icon::TrayIcon,
    #[cfg(target_os = "linux")]
    pending: Option<crossbeam_channel::Receiver<HashMap<MenuId, MenuCommand>>>,
}

impl TrayService {
    pub fn install(bindings: &mut HashMap<MenuId, MenuCommand>) -> Option<Self> {
        install_impl(bindings)
    }

    /// Merge the tray menu's bindings into `bindings` once the GTK
    /// thread has built the icon. Called every frame; a no-op after the
    /// first delivery and on platforms where `install` is synchronous.
    pub fn poll_bindings(&mut self, bindings: &mut HashMap<MenuId, MenuCommand>) {
        #[cfg(target_os = "linux")]
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(received) => {
                    bindings.extend(received);
                    self.pending = None;
                }
                // Thread gave up (no display / no tray host): stop polling.
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.pending = None,
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = bindings;
    }
}

#[cfg(not(target_os = "linux"))]
fn install_impl(bindings: &mut HashMap<MenuId, MenuCommand>) -> Option<TrayService> {
    use tray_icon::TrayIconBuilder;

    let icon = build_icon()?;
    let menu = build_menu(bindings);
    let _icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_tooltip("OneAmp")
        .build()
        .ok()?;
    Some(TrayService { _icon })
}

#[cfg(target_os = "linux")]
fn install_impl(_bindings: &mut HashMap<MenuId, MenuCommand>) -> Option<TrayService> {
    use crossbeam_channel::bounded;
    use std::thread;
    use tray_icon::TrayIconBuilder;

    // One-shot channel: the GTK thread builds the menu + tray, ships
    // the freshly-minted bindings map back to us, then enters
    // `gtk::main()` for the rest of the process lifetime. On failure it
    // just drops the sender.
    let (tx, rx) = bounded::<HashMap<MenuId, MenuCommand>>(1);

    thread::Builder::new()
        .name("oneamp-tray-gtk".to_string())
        .spawn(move || {
            if gtk::init().is_err() {
                return;
            }
            let Some(icon) = build_icon() else {
                return;
            };
            let mut local_bindings: HashMap<MenuId, MenuCommand> = HashMap::new();
            let menu = build_menu(&mut local_bindings);
            let Ok(_tray) = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_icon(icon)
                .with_tooltip("OneAmp")
                .build()
            else {
                return;
            };
            // Hand the bindings to the main thread BEFORE blocking in
            // `gtk::main`. After this point the GTK thread is just an
            // event pump — all menu clicks land on the global
            // `MenuEvent::receiver()` queue the main thread drains.
            let _ = tx.send(local_bindings);
            gtk::main();
        })
        .ok()?;

    // Don't wait for the GTK thread here: gtk::init() + tray-icon build
    // takes ~80 ms, and the window isn't shown until the first frame.
    // No bindings are needed until the icon exists; `poll_bindings`
    // picks them up from the update loop.
    Some(TrayService { pending: Some(rx) })
}

/// Build the tray menu and register each item's id in `bindings`.
/// Kept identical across OSes — the menu is short (six items) so we
/// don't need OS-conditional layouts here.
fn build_menu(bindings: &mut HashMap<MenuId, MenuCommand>) -> Menu {
    let menu = Menu::new();

    let play_pause = MenuItem::new("Play / Pause", true, None);
    bindings.insert(play_pause.id().clone(), MenuCommand::TogglePlayback);
    let previous = MenuItem::new("Previous", true, None);
    bindings.insert(
        previous.id().clone(),
        MenuCommand::Audio(AudioCommand::Previous),
    );
    let next = MenuItem::new("Next", true, None);
    bindings.insert(next.id().clone(), MenuCommand::Audio(AudioCommand::Next));

    let show = MenuItem::new("Show OneAmp", true, None);
    bindings.insert(show.id().clone(), MenuCommand::ShowWindow);

    let quit = MenuItem::new("Quit OneAmp", true, None);
    bindings.insert(
        quit.id().clone(),
        MenuCommand::MainWindow(MainWindowAction::Quit),
    );

    // Layout intent: playback controls first, separator, window
    // management, separator, destructive action last. Matches the
    // muscle memory of Spotify / VLC tray menus.
    let _ = menu.append_items(&[
        &play_pause,
        &previous,
        &next,
        &PredefinedMenuItem::separator(),
        &show,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);
    menu
}

/// Decode the bundled PNG icon into an `Icon` the OS's tray API can
/// blit. We pass through eframe's helper (already used for the main
/// window icon in `main.rs`) so the PNG decoder dependency stays
/// shared and we don't pin two `image` versions in the tree.
fn build_icon() -> Option<tray_icon::Icon> {
    let icon_data =
        eframe::icon_data::from_png_bytes(&include_bytes!("../../../icon_256.png")[..]).ok()?;
    tray_icon::Icon::from_rgba(icon_data.rgba, icon_data.width, icon_data.height).ok()
}
