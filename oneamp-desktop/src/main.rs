//! OneAmp - Desktop Application
//! Lecteur audio WSZ-only (Winamp skins)

use eframe::egui;

mod app;
mod audio;
mod config;
mod dialog_util;
mod format_dialog;
mod i18n;
mod platform;
mod platform_detection;
mod preset_name_dialog;
mod resume_store;
mod skin_thumbnails;
mod skins;
mod tag_editor_dialog;
mod url_dialog;
mod welcome;
mod windows;
mod wsz_ui;

use platform::ipc;

use app::OneAmpApp;

/// Read `ONEAMP_CUSTOM_CHROME` (`1`/`0`/`true`/`false`/`yes`/`no`, case
/// insensitive) and return the user's explicit choice. `None` when the var
/// is unset or unparseable, in which case platform auto-detection wins.
fn custom_chrome_env_override() -> Option<bool> {
    let raw = std::env::var("ONEAMP_CUSTOM_CHROME").ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn main() -> eframe::Result {
    // Collect paths passed on argv (file manager %F expansion or
    // `oneamp foo.mp3` on the command line). If another instance is
    // already running, hand them off via the Unix socket and exit —
    // a second window would clutter the desktop and split the playlist.
    let arg_paths = ipc::collect_arg_paths();
    if ipc::try_forward(&arg_paths) {
        return Ok(());
    }
    // No primary reachable: we become it. Hold the listener guard for
    // the rest of main so the socket file is cleaned up on graceful exit.
    let (ipc_rx, _ipc_guard) = match ipc::bind_primary() {
        Ok(pair) => (Some(pair.0), Some(pair.1)),
        Err(e) => {
            // Socket bind failure is not fatal — the player still works,
            // just without single-instance handoff (subsequent invocations
            // will each spawn their own window). Most likely cause: no
            // write access to $XDG_RUNTIME_DIR / /tmp.
            eprintln!("oneamp: single-instance disabled ({e})");
            (None, None)
        }
    };

    let platform_info = platform_detection::PlatformInfo::detect();
    let (use_custom_chrome, chrome_source) = match custom_chrome_env_override() {
        Some(v) => (v, "ONEAMP_CUSTOM_CHROME"),
        None => (platform_info.should_use_custom_chrome(), "auto-detected"),
    };

    println!("OneAmp - WSZ-Only Player");
    println!("Platform: {}", platform_info.description());
    println!(
        "Custom window chrome: {} ({})",
        if use_custom_chrome {
            "enabled"
        } else {
            "disabled"
        },
        chrome_source,
    );

    // Match the WSZ main window pixel-for-pixel — any extra space appears as
    // letterboxing around the skin, which kills the "this is Winamp" look.
    const SKIN_W: f32 = 275.0;
    const SKIN_H: f32 = 116.0;

    // Probe both Vulkan and GL so a host with a broken Vulkan ICD (or no
    // ICD at all) can still fall back to GLES via libEGL. The libstdc++
    // shadowing that broke libEGL/Vulkan ICDs in v0.17.1 (issue #8) is
    // now fixed at the bundling layer — release.yml strips the runner's
    // libstdc++.so.6 / libgcc_s.so.1 from the AppDir, so the host's C++
    // runtime is used and both backends initialise normally. Override
    // with `WGPU_BACKEND=vulkan` or `WGPU_BACKEND=gl` to pin one.
    let supported_backends = eframe::wgpu::util::backend_bits_from_env()
        .unwrap_or(eframe::wgpu::Backends::PRIMARY | eframe::wgpu::Backends::GL);
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew {
        supported_backends: backends,
        ..
    } = &mut wgpu_options.wgpu_setup
    {
        *backends = supported_backends;
    }

    let options = eframe::NativeOptions {
        // `transparent` lets the skin's `region.txt` polygon show through —
        // pixels outside the `[Normal]` mask have alpha=0 (applied at load
        // time on `main.bmp`) and the compositor blends them with the
        // desktop. On systems without a compositor the corners fall back to
        // black, which is acceptable.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([SKIN_W, SKIN_H])
            .with_min_inner_size([SKIN_W, SKIN_H])
            .with_resizable(false)
            .with_decorations(!use_custom_chrome)
            .with_transparent(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(&include_bytes!("../../icon_256.png")[..])
                    .unwrap_or_default(),
            ),
        wgpu_options,
        ..Default::default()
    };

    eframe::run_native(
        "OneAmp",
        options,
        Box::new(move |cc| {
            Ok(Box::new(OneAmpApp::new(
                cc,
                use_custom_chrome,
                arg_paths,
                ipc_rx,
            )))
        }),
    )
}
