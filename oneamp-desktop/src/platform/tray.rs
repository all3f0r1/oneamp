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
///   this side just keeps an empty marker so the field exists on all
///   platforms. The process exiting collapses the GTK thread.
pub struct TrayService {
    #[cfg(not(target_os = "linux"))]
    _icon: tray_icon::TrayIcon,
}

impl TrayService {
    pub fn install(bindings: &mut HashMap<MenuId, MenuCommand>) -> Option<Self> {
        install_impl(bindings)
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
fn install_impl(bindings: &mut HashMap<MenuId, MenuCommand>) -> Option<TrayService> {
    use crossbeam_channel::bounded;
    use std::thread;
    use std::time::Duration;
    use tray_icon::TrayIconBuilder;

    // One-shot channel: the GTK thread builds the menu + tray, ships
    // the freshly-minted bindings map back to us, then enters
    // `gtk::main()` for the rest of the process lifetime.
    let (tx, rx) = bounded::<Option<HashMap<MenuId, MenuCommand>>>(1);

    thread::Builder::new()
        .name("oneamp-tray-gtk".to_string())
        .spawn(move || {
            if gtk::init().is_err() {
                let _ = tx.send(None);
                return;
            }
            let icon = match build_icon() {
                Some(i) => i,
                None => {
                    let _ = tx.send(None);
                    return;
                }
            };
            let mut local_bindings: HashMap<MenuId, MenuCommand> = HashMap::new();
            let menu = build_menu(&mut local_bindings);
            let _tray = match TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_icon(icon)
                .with_tooltip("OneAmp")
                .build()
            {
                Ok(t) => t,
                Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
            };
            // Hand the bindings to the main thread BEFORE blocking in
            // `gtk::main`. After this point the GTK thread is just an
            // event pump — all menu clicks land on the global
            // `MenuEvent::receiver()` queue the main thread drains.
            let _ = tx.send(Some(local_bindings));
            gtk::main();
        })
        .ok()?;

    // Block startup briefly while the GTK thread initialises. Two
    // seconds is generous — gtk::init() + tray-icon build usually
    // completes in <100 ms on a healthy Linux session. If we time out,
    // assume the tray is not available (headless / no display) and
    // fall back to no-tray; the rest of the app keeps working.
    let received = rx.recv_timeout(Duration::from_secs(2)).ok()??;
    bindings.extend(received);
    Some(TrayService {})
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
