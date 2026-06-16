//! Main application module - WSZ-only OneAmp
//!
//! This is the primary entry point for the WSZ-based player.
//! Coordinates audio, state, windows, and configuration.

mod audio_loop;
mod config_sync;
mod input;
mod menu;
mod playlist_ops;
mod skin;
mod state;

pub use skin::WSZ_PLEDIT_FONT_FAMILY;
use skin::{apply_skin_fonts, load_skin_from_config};

use crossbeam_channel::Receiver;
use eframe::egui;
use oneamp_core::{AudioCommand, RepeatMode};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::state::{AppState, PlaybackState};
use crate::audio::AudioController;
use crate::config::{AppConfig, VisualizerModeConfig};
use crate::dialog_util::DialogView;
use crate::i18n::{Lang, Strings};
use crate::platform::media_controls::MediaControlsService;
use crate::platform::menu_bar::{MacMenuBar, MenuCommand, MenuId};
use crate::platform::notifications::NotificationService;
use crate::platform::tray::TrayService;
use crate::platform::updater::UpdateChecker;
use crate::welcome::{self, SkinsDialog, Welcome, WelcomeAction};
use crate::windows::WszWindowCoordinator;
use crate::wsz_ui::main_window::VisualizerMode;
use oneamp_core::{Playlist, RecentFiles};

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "wav", "aac", "m4a", "m4b", "mp4", "alac",
];

/// Debounce window for `config.save()` — when the user keeps wiggling the
/// volume slider, we wait until they've been idle this long before
/// touching disk. Short enough that "set volume, hit Alt+F4" still
/// catches the new value before `on_exit` (`on_exit` always saves too),
/// long enough that a frantic EQ adjustment doesn't fsync 50 times.
const CONFIG_SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

/// Sleep timer fade-out duration. When the deadline hits we ramp the
/// stored volume down to zero over this many seconds, then send Pause.
/// 10 s is the Spotify / Apple Music convention and feels gentle without
/// dragging out the goodbye.
const SLEEP_TIMER_FADE_SECS: f32 = 10.0;

/// In-app transient toast. Lives at the bottom of the viewport for a
/// short duration before fading. We keep at most one alive at a time —
/// a fresh push replaces any in-flight one, which matches the Snackbar
/// convention users already know from Material apps. Used by the
/// `Stop after current` arming feedback, the drag-drop ingest count,
/// and the "Already up to date" update-check signal.
struct Toast {
    message: String,
    expires_at: std::time::Instant,
}

fn visualizer_from_config(v: VisualizerModeConfig) -> VisualizerMode {
    match v {
        VisualizerModeConfig::Spectrum => VisualizerMode::Spectrum,
        VisualizerModeConfig::Oscilloscope => VisualizerMode::Oscilloscope,
        VisualizerModeConfig::PeakMeter => VisualizerMode::PeakMeter,
        VisualizerModeConfig::Off => VisualizerMode::Off,
    }
}

fn visualizer_to_config(v: VisualizerMode) -> VisualizerModeConfig {
    match v {
        VisualizerMode::Spectrum => VisualizerModeConfig::Spectrum,
        VisualizerMode::Oscilloscope => VisualizerModeConfig::Oscilloscope,
        VisualizerMode::PeakMeter => VisualizerModeConfig::PeakMeter,
        VisualizerMode::Off => VisualizerModeConfig::Off,
    }
}

/// Resolve the auto-DPI render scale. Snaps the OS-reported
/// `native_ppp` *up* to the next integer (1.0 → 1, 1.01–1.99 → 2,
/// 2.01–2.99 → 3, …). An integer ppp is the only way the WSZ bitmap
/// atlas magnifies to uniform-size pixels under nearest-neighbour
/// sampling; snapping *up* (rather than nearest or down) also matches
/// what egui-winit hands to the compositor as a `PhysicalSize`
/// request — on a modern Wayland compositor that negotiates
/// `wp_fractional_scale_v1` (KDE Plasma ≥ 5.27, GNOME ≥ 46, wlroots
/// ≥ 0.17) the request is honoured 1:1 and the buffer ends up at
/// exactly the integer-multiple size, never going through a bilinear
/// downscale.
///
/// Trade-off on fractional-DPI displays: the player is rendered at the
/// *next* integer scale (1.5× DPI → 2× ppp), so it appears ~33 %
/// physically larger than apps that follow the OS's preferred 1.5×.
/// For a Winamp-style player where pixel-perfect sprites are the whole
/// point, that's the right trade.
///
/// On configurations where the OS scale is already integer (Mac
/// Retina, Windows 100/200/300 %, X11 default DPI, Wayland with
/// integer-only buffer scale) the snap is a no-op and behaviour is
/// strictly identical to historical builds. On Wayland fractional
/// compositors *without* `wp_fractional_scale_v1` support the
/// compositor falls back to bilinear downscale — true pixel-perfect is
/// impossible there without OS cooperation, and that's the same
/// outcome any integer-snap strategy would produce.
pub fn pick_render_scale(native_ppp: f32) -> f32 {
    native_ppp.ceil().max(1.0)
}

/// File path of the user-preset JSON. Sits alongside `config.json` and
/// `resume.json` in the OS config dir so the per-user state lives in
/// one predictable place.
fn user_presets_path() -> Option<std::path::PathBuf> {
    crate::config::AppConfig::config_path()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("eq_presets.json")))
}

/// Load the user's `PresetManager` from disk, falling back to an empty
/// one when the file doesn't exist yet or is malformed. Best-effort —
/// a corrupt JSON shouldn't block the app from launching; the user
/// just loses any presets they'd saved previously, which they can
/// re-create from the EQ dropdown.
fn load_preset_manager_or_default() -> oneamp_core::PresetManager {
    match user_presets_path() {
        Some(path) => oneamp_core::PresetManager::load_or_new(path),
        None => oneamp_core::PresetManager::new(),
    }
}

/// Build the filtered view of the playlist for the current frame.
///
/// Returns `(filtered_entries, remap, filtered_selected, filtered_current)`:
/// - `filtered_entries` is `Some(Vec<…>)` only when the filter is
///   non-empty — saves one allocation per frame on the common path.
/// - `remap[filtered_idx] = original_idx`, used after the playlist
///   window emits an action to translate the index back.
/// - `filtered_selected` mirrors the originally-selected set into
///   filtered index space, dropping items the filter excluded.
/// - `filtered_current` is the filtered index of the original current
///   track, or `None` when the playing track doesn't match the filter
///   (the row just isn't shown — the audio engine keeps playing).
///
/// Case-insensitive substring match on each entry's `display_name`.
#[allow(clippy::type_complexity)]
fn build_playlist_filter_view(
    filter: &str,
    entries: &[oneamp_core::PlaylistEntry],
    current_index: Option<usize>,
    selected: &std::collections::BTreeSet<usize>,
) -> (
    Option<Vec<oneamp_core::PlaylistEntry>>,
    Option<Vec<usize>>,
    Option<std::collections::BTreeSet<usize>>,
    Option<usize>,
) {
    let needle = filter.trim();
    if needle.is_empty() {
        return (None, None, None, None);
    }
    let needle_lower = needle.to_lowercase();
    let mut filtered: Vec<oneamp_core::PlaylistEntry> = Vec::new();
    let mut remap: Vec<usize> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if entry.display_name().to_lowercase().contains(&needle_lower) {
            filtered.push(entry.clone());
            remap.push(i);
        }
    }
    let mut filtered_sel = std::collections::BTreeSet::new();
    for (fi, &oi) in remap.iter().enumerate() {
        if selected.contains(&oi) {
            filtered_sel.insert(fi);
        }
    }
    let filtered_current = current_index.and_then(|cur| remap.iter().position(|&i| i == cur));
    (
        Some(filtered),
        Some(remap),
        Some(filtered_sel),
        filtered_current,
    )
}

/// Translate every index inside `action` from filtered-space back to
/// original playlist-space. The playlist window doesn't know it was
/// handed a filtered view, so any `SelectTrack`, `PlayTrack`,
/// `RemoveAt`, etc. carries an index into the filtered slice; we map
/// it through `remap` before the app handler mutates real state.
fn remap_filtered_playlist_action(
    action: crate::windows::PlaylistAction,
    remap: &[usize],
) -> crate::windows::PlaylistAction {
    use crate::windows::PlaylistAction as A;
    let lookup = |fi: usize| remap.get(fi).copied();
    match action {
        A::SelectTrack(i) => lookup(i).map(A::SelectTrack).unwrap_or(A::None),
        A::ToggleSelectTrack(i) => lookup(i).map(A::ToggleSelectTrack).unwrap_or(A::None),
        A::RangeSelectTrack(i) => lookup(i).map(A::RangeSelectTrack).unwrap_or(A::None),
        A::PlayTrack(i) => lookup(i).map(A::PlayTrack).unwrap_or(A::None),
        A::RemoveAt(i) => lookup(i).map(A::RemoveAt).unwrap_or(A::None),
        A::EditTags(i) => lookup(i).map(A::EditTags).unwrap_or(A::None),
        A::QueueTrack(i) => lookup(i).map(A::QueueTrack).unwrap_or(A::None),
        // Both endpoints of a reorder live in filtered space; remap each
        // back to a real index. If either falls outside the filter the
        // move is meaningless, so drop it.
        A::MoveTrack { from, to } => match (lookup(from), lookup(to)) {
            (Some(from), Some(to)) => A::MoveTrack { from, to },
            _ => A::None,
        },
        // No-arg actions pass through untouched. Listed exhaustively
        // so a future PlaylistAction variant carrying an index has to
        // visit this matcher.
        other @ (A::None
        | A::Close
        | A::AddFiles
        | A::AddUrl
        | A::RemoveSelected
        | A::EditPlaylistFormat
        | A::Clear
        | A::SaveM3u
        | A::LoadM3u
        | A::SelectAll
        | A::SelectNone
        | A::InvertSelection
        | A::SortByTitle) => other,
    }
}

/// How long the cached cpal output-device list stays fresh before we
/// re-enumerate. `output_devices()` on the ALSA host prints diagnostic
/// chatter to stderr (JACK probes, OSS probes, dmix slave open
/// failures, …) every call — capping enumeration at one per 30 seconds
/// keeps that noise out of journald while still picking up USB DAC
/// hotplug in roughly the time it'd take the user to walk back to the
/// keyboard.
const OUTPUT_DEVICE_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Main OneAmp application (WSZ-only)
pub struct OneAmpApp {
    /// Centralized application state
    state: AppState,

    /// Audio engine controller
    audio: AudioController,

    /// Playlist state
    playlist: Playlist,

    /// WSZ window coordinator
    windows: WszWindowCoordinator,

    /// Configuration
    config: AppConfig,

    /// Path of the track currently pre-queued via `AudioCommand::QueueNext`,
    /// kept here so we don't resend the command every frame as the current
    /// track approaches its end. Cleared when a `TrackLoaded` event arrives
    /// for it (gapless swap happened) or when the user takes a manual
    /// action that invalidates the queue (Play/Stop/Next/Previous).
    pending_next_path: Option<std::path::PathBuf>,

    /// TTF/OTF bytes the current skin asked us to register as the
    /// playlist font. Bumped on every skin load (boot + Alt+S); the
    /// `update` tick checks `fonts_dirty` and replays
    /// `egui::Context::set_fonts`. We can't do this from `new()` —
    /// `set_fonts` reaches into the Context and we don't have one until
    /// the first frame.
    skin_font_data: Option<std::sync::Arc<Vec<u8>>>,
    fonts_dirty: bool,

    /// "Always on Top" — driven by the clutterbar Options menu, persisted
    /// in `AppConfig::always_on_top`, mirrored into `WszMainWindow` each
    /// frame so the menu can render the matching checkmark.
    always_on_top: bool,
    /// Whether we've already pushed the boot-time WindowLevel through the
    /// viewport. Set on the first frame; re-applied when the user toggles
    /// the menu entry.
    always_on_top_applied: bool,

    /// Whether we've re-confirmed the egui zoom-factor matches our chosen
    /// integer render scale. `set_pixels_per_point` takes effect at the
    /// start of the next pass, so the first `update` tick still sees the
    /// OS-driven default; we re-assert on that frame so the window settles
    /// on the right physical size as early as possible.
    render_scale_applied: bool,

    /// Receiver fed by the single-instance IPC listener. Each batch is a
    /// `Vec<PathBuf>` that a secondary `oneamp <files>` invocation handed
    /// off to us. `None` when single-instance setup failed at startup —
    /// the rest of the app still works, just without handoff.
    ipc_rx: Option<Receiver<Vec<PathBuf>>>,

    /// Files received on argv at launch time (file manager's `%F` or
    /// `oneamp foo.mp3` on the command line). Drained on the first
    /// `update` tick so the audio engine is fully constructed before we
    /// try to play anything.
    pending_initial_files: Vec<PathBuf>,

    /// Persistent "recently played" list surfaced in the clutterbar
    /// Options menu. Updated on every `Play` command via
    /// `play_audio_path`; saved back to `AppConfig::recent_files` in
    /// `on_exit`.
    recent: RecentFiles,

    /// Hotkey cheat-sheet overlay toggle. F1 / `?` flips this; Escape
    /// dismisses it. Renders on top of every sub-window, including the
    /// WSZ cursor layer.
    show_hotkeys: bool,

    /// Cached cpal output device list, refreshed at most once per
    /// `OUTPUT_DEVICE_REFRESH_INTERVAL`. Enumerating devices is cheap
    /// but not free (one syscall per device for the name); we'd rather
    /// not pay that on every frame just so the Options menu has a
    /// fresh list when it does open. Cleared on app start so the first
    /// frame triggers a refresh.
    output_devices_cache: Vec<String>,
    output_devices_refreshed_at: Option<std::time::Instant>,

    /// Last character the user typed for playlist type-to-jump, and the
    /// index it landed on. Repeated presses of the same letter cycle
    /// through matches instead of always landing on the first one.
    /// Cleared whenever the playlist mutates (add / remove / clear) so
    /// stale indices never advance past the playlist tail.
    jump_last_char: Option<char>,
    jump_last_index: Option<usize>,

    /// Progress through the `N-U-L` Nullsoft easter-egg sequence (0 =
    /// idle, 1 = `n` seen, 2 = `n,u` seen). Resets after `nul_deadline`.
    nul_progress: u8,
    /// Time by which the next easter-egg key must arrive, else the
    /// sequence resets.
    nul_deadline: Option<std::time::Instant>,

    /// MPRIS2 D-Bus service. Owns the souvlaki `MediaControls` and the
    /// crossbeam receiver for inbound media-key events. `None` when
    /// the platform bus refused our registration (headless session,
    /// no D-Bus, name already taken) — the rest of the app keeps
    /// working without it.
    mpris: Option<MediaControlsService>,

    /// Native macOS menu bar (HIG-mandatory top-of-screen strip). Built
    /// once at startup; `None` on Win/Linux (which keep the bare
    /// Winamp chrome) and when muda failed to attach on macOS. Held in
    /// the struct because dropping it would tear the NSMenu down.
    #[allow(dead_code)]
    mac_menu: Option<MacMenuBar>,

    /// Cross-OS system-tray icon (Shell_NotifyIcon / NSStatusItem /
    /// StatusNotifierItem). `None` when the platform refused to host
    /// us (headless CI, no display, GTK init failed). Held to keep
    /// the icon alive — dropping it removes the tray entry.
    #[allow(dead_code)]
    tray: Option<TrayService>,

    /// Combined `MenuId → MenuCommand` map populated by the macOS
    /// menu bar and the tray. Both subsystems' clicks emerge from the
    /// same global `MenuEvent::receiver()` channel; routing through
    /// one shared map means `dispatch_menu_events` drains the channel
    /// once and never drops events meant for the other subsystem.
    menu_bindings: HashMap<MenuId, MenuCommand>,

    /// `org.freedesktop.Notifications` client. Fires a transient toast
    /// per loaded track when `config.track_notifications_enabled`. Holds
    /// its own session-bus connection independent of MPRIS; failures
    /// silently no-op so a missing daemon doesn't block playback.
    notifications: NotificationService,

    /// One-shot background update checker. Spawned at startup; the
    /// `poll_update_checker` tick drains its channel each frame and
    /// fires a single desktop notification when a newer GitHub release
    /// is published. Persists `last_notified_update_version` in
    /// config so the same release isn't re-announced on every launch.
    update_checker: UpdateChecker,

    /// Timestamp of the last `mark_dirty` call. `Some(t)` means we have
    /// pending changes; once `t.elapsed() >= CONFIG_SAVE_DEBOUNCE` the
    /// update loop flushes to disk and resets this to `None`. Survives
    /// crashes far better than the previous on_exit-only save, while
    /// staying off the hot path during slider drags.
    config_dirty_since: Option<std::time::Instant>,

    /// User-driven scale override in 0.25 steps from 1.0 to 4.0 — `None`
    /// means follow the DPI heuristic. Mirrored from
    /// `AppConfig::user_scale` at boot and updated by
    /// `MainWindowAction::SetUserScale`. The update tick re-asserts
    /// `pixels_per_point` whenever this changes; switching back to `None`
    /// (the "Auto (DPI)" menu entry) re-applies whatever
    /// `pick_render_scale(native_ppp)` returns for the current display.
    user_scale: Option<f32>,
    /// Whether we've pushed the user-scale override through this session.
    /// Bumped on every change so the next update tick re-emits
    /// `set_pixels_per_point` and `invalidate_viewport_cache`.
    scale_dirty: bool,

    /// Active sleep-timer deadline, if any. `Some(t)` means we'll fade the
    /// volume out over `SLEEP_TIMER_FADE_SECS` ending at `t`, then send
    /// `Pause`. Cleared by user actions (Stop, Pause, choose Off in menu)
    /// and by the deadline firing. Ephemeral — not persisted across
    /// sessions, by design: a sleep timer set today shouldn't fire
    /// tomorrow.
    sleep_timer_deadline: Option<std::time::Instant>,
    /// Volume captured at the moment the sleep timer was armed, so we can
    /// restore it after Pause and so the fade ramp interpolates from the
    /// right starting point regardless of where the user moves the slider
    /// mid-fade.
    sleep_timer_pre_volume: f32,
    /// The duration choice (15/30/60/90 min) that was picked from the
    /// menu. Drives the radio-style checkmark; cleared when the timer
    /// fires or the user picks "Off".
    sleep_timer_choice: Option<u32>,

    /// First-launch welcome screen state. `open` flips true on the very
    /// first launch (when `AppConfig::load` returned `is_first_run = true`)
    /// and stays true until the user clicks Skip or Done in the welcome
    /// viewport. After that, `config.first_run = false` is persisted and
    /// the welcome screen never reopens.
    welcome: Welcome,

    /// Options → Skins… dialog state. Open/closed flag + cached skin
    /// catalog + text-edit buffer for the user's skins folder. Shared
    /// rendering helpers with `welcome` so the picker UI doesn't get
    /// duplicated.
    skins_dialog: SkinsDialog,

    /// Cached UI strings keyed by the resolved language. Rebuilt whenever
    /// the welcome screen reports a `ApplyLang` action.
    strings: Strings,

    /// Modal tag editor opened from the playlist context menu. `None`
    /// when no dialog is visible; `Some` while the user is editing tags
    /// on a specific playlist entry. Drained after the user clicks
    /// Save or Cancel.
    tag_editor: Option<crate::tag_editor_dialog::TagEditorDialog>,

    /// Modal "Open URL" dialog. `None` when closed; `Some` while the
    /// user is typing an HTTP(S) URL to add to the playlist.
    url_dialog: Option<crate::url_dialog::UrlDialog>,

    /// Modal "Edit playlist display format" dialog. `None` when closed;
    /// `Some` while the user is editing the format template.
    format_dialog: Option<crate::format_dialog::FormatDialog>,

    /// "Stop after current track" one-shot. When true, the next
    /// `AudioEvent::Finished` is intercepted and turned into a Stop
    /// command instead of the default playlist auto-advance. Any
    /// user-initiated playback action (Play, Next, Previous, Stop)
    /// silently disarms it — the user clearly wants to keep going.
    /// Session-only: not persisted, so a flag set today never fires
    /// tomorrow.
    stop_after_current: bool,

    /// Single in-flight toast. A new push replaces any active one;
    /// painted as an unobtrusive chip at the bottom-center of the
    /// viewport until `expires_at`. Cleared on paint when expired.
    toast: Option<Toast>,

    /// `Some(t)` when the user explicitly triggered "Check for updates"
    /// from the menu at instant `t`. The poll loop watches for either
    /// the update-info channel firing (newer release found → toast it)
    /// OR the channel disconnecting without a hit (no update → toast
    /// "You're up to date"). Reset to `None` once one of those fires
    /// or after a long timeout so a stuck request doesn't sit pending
    /// across many launches.
    manual_update_check_pending: Option<std::time::Instant>,

    /// Live substring filter applied to the playlist before the
    /// playlist window paints its rows. Empty string = no filter
    /// (the playlist renders unchanged). Updated in real-time from
    /// the overlay's `TextEdit`. Case-insensitive match against the
    /// formatted display title.
    playlist_filter: String,
    /// Whether the inline filter overlay is visible. Toggled by
    /// `Ctrl+F`; Esc closes + clears the filter.
    playlist_filter_open: bool,
    /// True for one frame after the overlay opened so we can
    /// `request_focus()` on the TextEdit. Auto-cleared once the
    /// initial focus has been delivered.
    playlist_filter_focus_pending: bool,

    /// User-defined EQ presets, persisted at
    /// `<config_dir>/oneamp/eq_presets.json`. Loaded at boot and
    /// pushed into the EQ window each frame so the PRESETS dropdown
    /// renders them after the built-ins. Saving a new one through the
    /// modal name dialog appends here and re-flushes the JSON.
    preset_manager: oneamp_core::PresetManager,

    /// Modal "Save EQ preset…" name-prompt dialog. `Some` while the
    /// user is typing; drained when they hit Save or Cancel.
    preset_name_dialog: Option<crate::preset_name_dialog::PresetNameDialog>,

    /// Per-file playback resume state, opt-in via
    /// `AppConfig::resume_long_files`. Loaded from
    /// `<config_dir>/oneamp/resume.json` at boot and saved on exit
    /// (plus throttled writes mid-playback so a crash doesn't lose
    /// hours of progress). See `crate::resume_store` for the on-disk
    /// shape and gating thresholds.
    resume: crate::resume_store::ResumeStore,
    /// Last time we wrote the resume store mid-playback. `None` means
    /// "due now"; subsequent ticks compare against
    /// `resume_store::SAVE_INTERVAL_SECS` to throttle.
    last_resume_save_at: Option<std::time::Instant>,
    /// Set to the path the audio thread just loaded, alongside the
    /// position we want to seek to once playback is actually rolling.
    /// Cleared on the first `Position` event that confirms the load
    /// reached the audio pipeline. We don't seek directly from
    /// `TrackLoaded`: the audio thread is still finishing the load,
    /// and a Seek issued in that window is sometimes silently dropped
    /// (especially on container-seekable codecs that re-probe on
    /// open).
    pending_resume: Option<(std::path::PathBuf, f32)>,
}

impl OneAmpApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        use_custom_chrome: bool,
        initial_files: Vec<PathBuf>,
        ipc_rx: Option<Receiver<Vec<PathBuf>>>,
    ) -> Self {
        // Pick an integer egui pixels_per_point based on the OS-reported
        // DPI so the WSZ sprite atlas is rasterized at a clean integer
        // multiple. `set_pixels_per_point` translates internally to a
        // zoom_factor change, so 1 egui-unit always maps to render_scale
        // physical pixels — the renderer keeps painting in source-pixel
        // space (`scale=1.0` on `WszRenderer`) and egui does the
        // magnification at draw time. See `pick_render_scale` for the
        // heuristic and why fractional scales would blur the skin.
        // Load configuration first so the user-scale override (if any)
        // wins over the DPI auto-pick on this very first frame — the
        // viewport then comes up at the size the user explicitly chose
        // last session instead of flashing the OS default.
        let (config, is_first_run) = AppConfig::load();

        let native_ppp = cc.egui_ctx.native_pixels_per_point().unwrap_or(1.0);
        let render_scale = config
            .user_scale
            .unwrap_or_else(|| pick_render_scale(native_ppp));
        cc.egui_ctx.set_pixels_per_point(render_scale);

        // Initialize audio controller
        let audio = AudioController::new();

        // Initialize state from config — including persisted balance and
        // shuffle, which the previous build allocated config fields for
        // but never actually applied at boot.
        let state = AppState {
            playback: crate::app::state::PlaybackState::Stopped,
            current_track: None,
            position: (0.0, 0.0),
            volume: crate::app::state::VolumeState {
                level: config.playback.volume,
                muted: config.playback.muted,
                balance: config.playback.balance,
            },
            equalizer: crate::app::state::EqualizerState {
                enabled: config.equalizer.enabled,
                gains: config.equalizer.gains.clone(),
                preamp_db: config.equalizer.preamp_db,
                current_preset: config.equalizer.current_preset.clone(),
            },
            repeat_mode: config.playback.repeat_mode.into(),
            shuffle_enabled: config.playback.shuffle_enabled,
        };

        // The playlist is intentionally session-only: every launch starts
        // empty. Any files the user wants from the previous session come
        // back via "Add files…", drag-drop, or "Open with OneAmp".
        let playlist = Playlist::default();

        let skin = load_skin_from_config(
            config.skin_path.as_deref(),
            config.bundled_skin_name.as_deref(),
        );
        let skin_font_data = skin.font_data.clone();
        let mut windows = WszWindowCoordinator::with_initial(
            &skin,
            1.0,
            use_custom_chrome,
            &config.equalizer.gains,
            config.equalizer.preamp_db,
            config.equalizer.enabled,
        );
        // Restore last session's docked layout. The coordinator's resize
        // pass on the first frame folds these into `target_viewport_size`,
        // so the OS viewport comes up sized for both the player and any
        // re-opened sub-windows in one go.
        windows.set_equalizer_visible(config.show_equalizer);
        windows.set_playlist_visible(config.show_playlist);
        windows.set_shade_mode(config.shade_mode);
        windows
            .main_window_mut()
            .set_visualizer_mode(visualizer_from_config(config.visualizer_mode));
        windows
            .main_window_mut()
            .set_show_remaining(config.show_remaining);
        // Restore last session's picked preset name so the EQ
        // dropdown remembers the selection. The gains themselves
        // were already applied through the engine's
        // SetEqualizerBands; this is just the name marker.
        windows.set_equalizer_current_preset(config.equalizer.current_preset.clone());

        // Apply persisted audio state to the audio thread so playback starts
        // with the user's saved equalizer curve, volume, balance, shuffle, …
        audio.send_command(AudioCommand::SetVolume(config.playback.volume));
        if config.playback.muted {
            audio.send_command(AudioCommand::SetMute(true));
        }
        audio.send_command(AudioCommand::SetBalance(config.playback.balance));
        audio.send_command(AudioCommand::SetShuffle(config.playback.shuffle_enabled));
        audio.send_command(AudioCommand::SetRepeatMode(
            config.playback.repeat_mode.into(),
        ));
        audio.send_command(AudioCommand::SetEqualizerEnabled(config.equalizer.enabled));
        audio.send_command(AudioCommand::SetEqualizerBands(
            config.equalizer.gains.clone(),
        ));
        audio.send_command(AudioCommand::SetEqualizerPreamp(config.equalizer.preamp_db));
        audio.send_command(AudioCommand::SetCrossfade(
            config.crossfade.enabled,
            config.crossfade.duration_secs,
        ));
        audio.send_command(AudioCommand::SetReplayGainEnabled(
            config.replaygain_enabled,
        ));
        // Push the persisted RG reference (track/album/auto) and mono
        // downmix so the engine matches the menu's ✓ on the first frame.
        audio.send_command(AudioCommand::SetReplayGainMode(config.replaygain_mode));
        audio.send_command(AudioCommand::SetMono(config.mono_enabled));
        audio.send_command(AudioCommand::SetLoudnessEnabled(config.loudness_enabled));
        audio.send_command(AudioCommand::SetOutputDevice(
            config.output_device_name.clone(),
        ));

        let always_on_top = config.always_on_top;
        // Migration: OneAmp ≤ 1.0.5 exposed 0.25-step scale values in
        // the View → Scale menu. A user who set 1.5× back then would
        // come up at 1.5× on the new build, sub-pixel-sampling the WSZ
        // sprite atlas — wrong for a pixel-perfect skin player. Snap
        // any persisted fractional value *up* to the next integer
        // (same policy as `pick_render_scale` for the auto path) so
        // first launch after the upgrade settles on a clean N× zoom.
        // `1.0` and friends pass through unchanged.
        let user_scale = config.user_scale.map(|s| s.ceil().max(1.0));
        let recent = config.recent_files.clone();
        let strings = Strings::new(Lang::resolve(config.ui_lang));
        let user_skins_dir_snapshot = config.user_skins_dir.clone();
        let mut welcome_state = Welcome::new(user_skins_dir_snapshot.as_deref());
        welcome_state.open = is_first_run;
        // Seed the welcome picker's "currently selected" indicator from
        // the persisted skin choice so the bundled-skin entry it would
        // boot to is highlighted even before the user clicks.
        welcome_state.selected_skin_name = config.bundled_skin_name.clone().or_else(|| {
            config
                .skin_path
                .as_ref()
                .and_then(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
        });
        // MPRIS bus registration happens once at startup. A failure here
        // never blocks the app — multimedia keys / desktop integration
        // just go silent for this session.
        let mpris = MediaControlsService::new(cc);
        // Both subsystems contribute their click-id → command pairs to
        // a single shared map so the app can drain
        // `MenuEvent::receiver()` once per frame and dispatch without
        // worrying about who registered which item.
        let mut menu_bindings: HashMap<MenuId, MenuCommand> = HashMap::new();
        let mac_menu = MacMenuBar::install(&mut menu_bindings);
        let tray = TrayService::install(&mut menu_bindings);
        Self {
            state,
            audio,
            playlist,
            windows,
            config,
            pending_next_path: None,
            mac_menu,
            tray,
            menu_bindings,
            skin_font_data,
            fonts_dirty: true,
            always_on_top,
            always_on_top_applied: false,
            render_scale_applied: false,
            ipc_rx,
            pending_initial_files: initial_files,
            recent,
            mpris,
            notifications: NotificationService::new(),
            // Fire the GitHub release check immediately so the user
            // sees a toast within seconds of launch (request races
            // splash / skin paint; network failure is silent).
            update_checker: UpdateChecker::spawn(),
            show_hotkeys: false,
            jump_last_char: None,
            jump_last_index: None,
            nul_progress: 0,
            nul_deadline: None,
            output_devices_cache: Vec::new(),
            output_devices_refreshed_at: None,
            config_dirty_since: None,
            user_scale,
            scale_dirty: user_scale.is_some(),
            sleep_timer_deadline: None,
            sleep_timer_pre_volume: 0.0,
            sleep_timer_choice: None,
            welcome: welcome_state,
            skins_dialog: SkinsDialog::new(user_skins_dir_snapshot.as_deref()),
            strings,
            tag_editor: None,
            url_dialog: None,
            format_dialog: None,
            stop_after_current: false,
            toast: None,
            manual_update_check_pending: None,
            playlist_filter: String::new(),
            playlist_filter_open: false,
            playlist_filter_focus_pending: false,
            preset_manager: load_preset_manager_or_default(),
            preset_name_dialog: None,
            resume: crate::resume_store::default_path()
                .map(|p| crate::resume_store::ResumeStore::load(&p))
                .unwrap_or_default(),
            last_resume_save_at: None,
            pending_resume: None,
        }
    }

    /// Snapshot the live EQ curve under `name` as a new user preset
    /// and persist the store. Rejects empty names and built-in name
    /// collisions; on either case we surface an in-app toast and
    /// leave the dialog re-openable so the user can pick a different
    /// name without losing their typing. On success the EQ window
    /// renders the new entry on the next frame via the per-frame
    /// `user_eq_presets` push in `collect_actions`.
    fn commit_user_preset(&mut self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if oneamp_core::BuiltinPresets::get_by_name(&name).is_some() {
            self.push_toast(
                format!("\"{}\" is a built-in preset — pick another name.", name),
                std::time::Duration::from_millis(2400),
            );
            return;
        }
        let preset = oneamp_core::EqualizerPreset {
            name: name.clone(),
            gains: self.state.equalizer.gains.clone(),
            description: None,
            preamp_db: self.state.equalizer.preamp_db,
        };
        if let Err(e) = self.preset_manager.add_preset(preset) {
            self.push_toast(
                format!("Couldn't save preset: {}", e),
                std::time::Duration::from_millis(2400),
            );
            return;
        }
        if let Some(path) = user_presets_path()
            && let Err(e) = self.preset_manager.save(&path)
        {
            eprintln!("Failed to save preset store: {}", e);
        }
        self.push_toast(
            format!("Saved preset \"{}\"", name),
            std::time::Duration::from_millis(1800),
        );
    }

    /// Push an in-app toast. Replaces any active one — Snackbar
    /// convention. `duration` is how long the toast stays fully opaque;
    /// `paint_toast` adds a 250 ms fade-out on top of that automatically.
    pub(super) fn push_toast(&mut self, msg: impl Into<String>, duration: std::time::Duration) {
        self.toast = Some(Toast {
            message: msg.into(),
            expires_at: std::time::Instant::now() + duration,
        });
    }

    /// Render the inline playlist filter overlay (Ctrl+F). A small
    /// chip with a TextEdit and a hint about Esc, anchored near the
    /// top of the viewport so it sits above the playlist's first
    /// row when the user has scrolled it back to the top. Auto-
    /// focuses the field the first frame after opening so the user
    /// can type without an extra click. Esc inside the field clears
    /// the filter and closes the overlay; clearing the text alone
    /// also drops the row-filtering until the user types again.
    pub(super) fn paint_playlist_filter_overlay(&mut self, ctx: &egui::Context) {
        if !self.playlist_filter_open {
            return;
        }
        // Drain Escape before the TextEdit gets a chance to render — when
        // the field has focus, egui's text widget calls `consume_key` on
        // Escape (to bail out of editing) and the event vanishes from
        // raw input. Reading it here, ahead of any widget, sees the press
        // straight from the frame's raw events. Close + clear in one
        // shot so the playlist returns to its unfiltered view.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.playlist_filter.clear();
            self.playlist_filter_open = false;
            return;
        }
        // Enter turns the filter into Winamp's jump-to-file: play the
        // first entry matching the typed fragment, then dismiss. Read
        // ahead of the TextEdit for the same reason as Escape — a
        // focused singleline field swallows Enter otherwise.
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            let needle = self.playlist_filter.trim().to_lowercase();
            if !needle.is_empty() {
                let hit = self
                    .playlist
                    .entries()
                    .iter()
                    .position(|e| e.display_name().to_lowercase().contains(&needle));
                if let Some(idx) = hit {
                    self.playlist.set_current(idx);
                    if let Some(path) = self.playlist.current_entry().map(|e| e.path.clone()) {
                        self.play_audio_path(path);
                    }
                }
            }
            self.playlist_filter.clear();
            self.playlist_filter_open = false;
            return;
        }
        let viewport_id = egui::Id::new("playlist_filter_overlay");
        let screen = ctx.screen_rect();
        // Anchor at the top-center of the viewport. 240×24 chip is
        // enough room for the field + label without crowding the
        // skin's title strip. Vertical inset of 4 keeps it just
        // under the close-button row of the WSZ main window.
        let chip_w = 260.0_f32.min(screen.width() - 16.0);
        let chip_h = 26.0;
        let chip_x = screen.center().x - chip_w / 2.0;
        let chip_y = screen.min.y + 4.0;
        // Use an Area so the TextEdit gets real focus / input
        // routing — a `layer_painter` alone wouldn't accept text
        // entry.
        egui::Area::new(viewport_id)
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(chip_x, chip_y))
            .show(ctx, |ui| {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(chip_x, chip_y),
                    egui::vec2(chip_w, chip_h),
                );
                ui.painter().rect_filled(
                    rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(15, 15, 15, 240),
                );
                ui.painter().rect_stroke(
                    rect,
                    4.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 220, 100)),
                );
                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(8.0, 4.0))),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Filter:")
                                    .color(egui::Color32::from_rgb(120, 220, 120))
                                    .size(11.0),
                            );
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut self.playlist_filter)
                                    .desired_width(ui.available_width() - 50.0)
                                    .hint_text("artist / title"),
                            );
                            if self.playlist_filter_focus_pending {
                                edit.request_focus();
                                self.playlist_filter_focus_pending = false;
                            }
                            ui.label(
                                egui::RichText::new("Enter / Esc")
                                    .color(egui::Color32::GRAY)
                                    .size(9.0),
                            );
                        });
                    },
                );
            });
    }

    /// Render the sleep-timer badge in the top-right of the viewport
    /// while the timer is armed. Hidden otherwise. Format is `Zz Xm`
    /// for ≥1 min remaining, `Zz X s` for the last minute (countdown
    /// every second). The chip flashes between dim/bright once per
    /// second in the last minute so the imminent fade-out is obvious
    /// at a glance. Painted before `paint_toast` so a transient toast
    /// at the bottom doesn't overlap and so the badge stays visible
    /// continuously while armed.
    pub(super) fn paint_sleep_badge(&self, ctx: &egui::Context) {
        let Some(deadline) = self.sleep_timer_deadline else {
            return;
        };
        let now = std::time::Instant::now();
        if now >= deadline {
            return;
        }
        let remaining = deadline - now;
        let total_secs = remaining.as_secs();
        let (label, urgent) = if total_secs >= 60 {
            // Round up so a "30-minute" timer with 29 m 58 s left still
            // reads "30m" for two seconds — matches user expectations.
            let mins = total_secs.div_ceil(60);
            (format!("Zz {}m", mins), false)
        } else {
            (format!("Zz {}s", total_secs.max(1)), true)
        };
        // Flash: in the last minute, alternate between two brightness
        // tiers every ~500 ms so the chip blinks twice per second.
        let bright = if urgent {
            (remaining.as_millis() / 500).is_multiple_of(2)
        } else {
            true
        };

        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("sleep_badge"),
        ));
        let accent = if urgent {
            egui::Color32::from_rgb(255, 140, 60) // amber when imminent
        } else {
            egui::Color32::from_rgb(60, 220, 100)
        };
        let text_color = if bright {
            egui::Color32::from_rgb(240, 240, 240)
        } else {
            egui::Color32::from_rgb(120, 120, 120)
        };
        let font = egui::FontId::proportional(10.0);
        let galley = painter.layout_no_wrap(label, font, text_color);
        let pad_x = 6.0;
        let pad_y = 2.0;
        let chip_w = galley.size().x + pad_x * 2.0;
        let chip_h = galley.size().y + pad_y * 2.0;
        // Anchor to the top-right of the viewport with a small inset so
        // the chip doesn't kiss the window edge.
        let chip = egui::Rect::from_min_size(
            egui::pos2(screen.max.x - chip_w - 6.0, screen.min.y + 6.0),
            egui::vec2(chip_w, chip_h),
        );
        painter.rect_filled(
            chip,
            3.0,
            egui::Color32::from_rgba_unmultiplied(15, 15, 15, 220),
        );
        painter.rect_stroke(chip, 3.0, egui::Stroke::new(1.0, accent));
        painter.galley(
            egui::pos2(chip.min.x + pad_x, chip.min.y + pad_y),
            galley,
            egui::Color32::WHITE,
        );
    }

    /// Render the active toast, if any. Painted as a small dark chip
    /// with the skin's accent green border at the bottom-center of the
    /// viewport. Fades to fully transparent during the last 250 ms so
    /// the dismissal isn't an abrupt pop. Self-clears on expiry.
    pub(super) fn paint_toast(&mut self, ctx: &egui::Context) {
        const FADE_MS: u64 = 250;
        let Some(toast) = self.toast.as_ref() else {
            return;
        };
        let now = std::time::Instant::now();
        if now >= toast.expires_at {
            self.toast = None;
            return;
        }
        let remaining = toast.expires_at - now;
        // Linear alpha ramp over the last FADE_MS; full opacity before that.
        let alpha = if remaining.as_millis() as u64 > FADE_MS {
            240u8
        } else {
            (240.0 * remaining.as_millis() as f32 / FADE_MS as f32) as u8
        };

        let screen = ctx.screen_rect();
        // Sized to the message so a short "Track added" doesn't sit on a
        // huge plate, and a longer one still fits. Cap the width so we
        // don't drag the chip past the player viewport edges.
        let font = egui::FontId::proportional(10.0);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("toast_overlay"),
        ));
        let galley = painter.layout_no_wrap(
            toast.message.clone(),
            font.clone(),
            egui::Color32::from_rgba_unmultiplied(230, 230, 230, alpha),
        );
        let pad_x = 10.0;
        let pad_y = 4.0;
        let chip_w = (galley.size().x + pad_x * 2.0).min(screen.width() - 16.0);
        let chip_h = galley.size().y + pad_y * 2.0;
        let chip = egui::Rect::from_min_size(
            egui::pos2(
                screen.center().x - chip_w / 2.0,
                screen.max.y - chip_h - 6.0,
            ),
            egui::vec2(chip_w, chip_h),
        );
        painter.rect_filled(
            chip,
            3.0,
            egui::Color32::from_rgba_unmultiplied(15, 15, 15, alpha),
        );
        painter.rect_stroke(
            chip,
            3.0,
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(60, 220, 100, alpha),
            ),
        );
        painter.galley(
            egui::pos2(chip.min.x + pad_x, chip.min.y + pad_y),
            galley,
            egui::Color32::WHITE,
        );
    }

    /// Currently-armed sleep-timer choice (15/30/60/90 min) so the
    /// menu can render the radio-style checkmark. None when the timer
    /// isn't armed.
    fn active_sleep_timer_minutes(&self) -> Option<u32> {
        self.sleep_timer_choice
    }

    /// Sleep-timer tick. Off when `sleep_timer_deadline` is `None`. While
    /// armed, the last `SLEEP_TIMER_FADE_SECS` are spent linearly ramping
    /// volume from `sleep_timer_pre_volume` down to zero so the user
    /// doesn't get a hard cut to silence. At the deadline we send Pause
    /// and restore the original volume (so the next Play comes back at
    /// the level the user actually wanted).
    fn tick_sleep_timer(&mut self) {
        let Some(deadline) = self.sleep_timer_deadline else {
            return;
        };
        let now = std::time::Instant::now();
        if now >= deadline {
            self.audio.send_command(oneamp_core::AudioCommand::Pause);
            // Restore the captured volume so the next Play resumes at it.
            self.audio
                .send_command(oneamp_core::AudioCommand::SetVolume(
                    self.sleep_timer_pre_volume,
                ));
            self.state.volume.level = self.sleep_timer_pre_volume;
            self.sleep_timer_deadline = None;
            return;
        }
        let remaining = (deadline - now).as_secs_f32();
        if remaining < SLEEP_TIMER_FADE_SECS {
            let progress = remaining / SLEEP_TIMER_FADE_SECS;
            let target = self.sleep_timer_pre_volume * progress.clamp(0.0, 1.0);
            self.audio
                .send_command(oneamp_core::AudioCommand::SetVolume(target));
            self.state.volume.level = target;
        }
    }

    /// Dispatch a `WelcomeAction` emitted by the welcome viewport. The
    /// per-action work mirrors what `MainWindowAction` handlers do, so
    /// switching scale / skin from the welcome screen leaves the app in
    /// the same state as picking the same option from the menu later.
    fn handle_welcome_action(&mut self, action: WelcomeAction) {
        match action {
            WelcomeAction::ApplySkin(entry) => {
                self.apply_skin_entry(entry);
            }
            WelcomeAction::ApplyScale(choice) => {
                self.user_scale = choice;
                self.scale_dirty = true;
                self.mark_dirty();
            }
            WelcomeAction::ApplyLang(lang_cfg) => {
                self.config.ui_lang = lang_cfg;
                self.strings = Strings::new(Lang::resolve(lang_cfg));
                self.mark_dirty();
            }
            WelcomeAction::SetAsDefaultPlayer => {
                self.welcome.default_player_status = Some(welcome::invoke_set_as_default());
            }
            WelcomeAction::Done | WelcomeAction::Skip => {
                self.config.first_run = false;
                self.mark_dirty();
            }
        }
    }

    /// Arm or cancel the sleep timer. `None` clears any pending timer
    /// and restores the captured volume so a half-faded ramp doesn't
    /// leave the user at 30 % the next time they hit Play.
    fn set_sleep_timer(&mut self, choice: Option<u32>) {
        match choice {
            None => {
                // Restore the captured volume if a fade was already in
                // progress — otherwise the user picked "Off" before any
                // fade started and the live volume is fine.
                if self.sleep_timer_deadline.is_some() {
                    self.state.volume.level = self.sleep_timer_pre_volume;
                    self.audio
                        .send_command(oneamp_core::AudioCommand::SetVolume(
                            self.sleep_timer_pre_volume,
                        ));
                }
                self.sleep_timer_deadline = None;
                self.sleep_timer_choice = None;
            }
            Some(minutes) => {
                let duration = std::time::Duration::from_secs(u64::from(minutes) * 60);
                self.sleep_timer_deadline = Some(std::time::Instant::now() + duration);
                self.sleep_timer_pre_volume = self.state.volume.level;
                self.sleep_timer_choice = Some(minutes);
            }
        }
    }

    /// Refresh the cached output-device list if `OUTPUT_DEVICE_REFRESH_INTERVAL`
    /// has elapsed since the last enumeration. Cheap to call every
    /// frame — the elapsed check short-circuits when the cache is hot.
    /// Returns the current snapshot for callers that want to thread it
    /// into the UI without an extra borrow.
    fn refresh_output_devices(&mut self) -> &[String] {
        let stale = self
            .output_devices_refreshed_at
            .map(|t| t.elapsed() >= OUTPUT_DEVICE_REFRESH_INTERVAL)
            .unwrap_or(true);
        if stale {
            self.output_devices_cache = oneamp_core::list_output_devices();
            self.output_devices_refreshed_at = Some(std::time::Instant::now());
        }
        &self.output_devices_cache
    }
}

impl eframe::App for OneAmpApp {
    /// Clear the framebuffer with full transparency so the alpha mask baked
    /// into `main.bmp` (from the skin's `region.txt` `[Normal]` polygon)
    /// shows through to the desktop. Without this, eframe's default opaque
    /// clear would draw a solid rectangle in the masked-out corners.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Re-assert the integer render scale on the first frame. The
        // `set_pixels_per_point` call in `new` queues for the next pass
        // — by frame one egui has settled, and from here the
        // `WszWindowCoordinator`'s per-frame `target_viewport_size` lock
        // takes over. `egui-winit`'s `InnerSize` handler converts our
        // 275×116 egui-unit target to render_scale × 275 physical pixels.
        if !self.render_scale_applied
            && let Some(native_ppp) = ctx.native_pixels_per_point()
        {
            let render_scale = self
                .user_scale
                .unwrap_or_else(|| pick_render_scale(native_ppp));
            let live_ppp = ctx.pixels_per_point();
            if (live_ppp - render_scale).abs() > 0.01 {
                // ppp not active yet — keep requesting until it
                // matches. `set_pixels_per_point` queues for the
                // next pass; the coordinator's resize loop uses
                // the LIVE ppp inside egui-winit, so until ppp
                // settles, any `InnerSize` would be miscomputed
                // (1.5 × 275 = 413 px instead of 2 × 275 = 550).
                ctx.set_pixels_per_point(render_scale);
            } else {
                // ppp is live. Force the coordinator to re-emit
                // its `InnerSize` — its cache keys on egui units
                // (275×116), which didn't change when we flipped
                // ppp, so without invalidation it would silently
                // keep the boot-time 413×174 physical window.
                self.windows.invalidate_viewport_cache();
                self.render_scale_applied = true;
            }
        }

        // Replay set_fonts when the active skin changed (boot is just the
        // first dirty edge). set_fonts is heavy — don't run it every frame.
        if self.fonts_dirty {
            apply_skin_fonts(ctx, self.skin_font_data.as_ref());
            self.fonts_dirty = false;
        }

        // Push the persisted WindowLevel once at startup. Sending it from
        // `new()` would have no effect — the viewport doesn't exist yet.
        if !self.always_on_top_applied {
            let level = if self.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
            self.always_on_top_applied = true;
        }

        // Process files received on argv at boot (drained once on the
        // first tick so audio + windows are fully wired by then) and any
        // files handed off by a secondary `oneamp <files>` invocation
        // via the IPC socket since the last frame.
        if !self.pending_initial_files.is_empty() {
            let paths = std::mem::take(&mut self.pending_initial_files);
            // argv files come from a fresh launch via "Open with OneAmp"
            // (or `oneamp foo.mp3` on the CLI) — the user asked us to play
            // these tracks, so honour that even if something is mid-play
            // (this can only happen on a restore-state launch in practice).
            self.ingest_files(&paths, ctx, true);
        }
        // Drain via local collection so the borrow of `self.ipc_rx`
        // releases before `ingest_files` reborrows `self` mutably.
        let mut batches: Vec<Vec<PathBuf>> = Vec::new();
        if let Some(rx) = self.ipc_rx.as_ref() {
            while let Ok(batch) = rx.try_recv() {
                batches.push(batch);
            }
        }
        for batch in batches {
            // IPC handoff = a secondary `oneamp <files>` invocation, almost
            // always triggered by the user double-clicking a file in their
            // OS file manager. Force playback of the first new track so the
            // double-click feels like "play this now", not "queue this".
            self.ingest_files(&batch, ctx, true);
        }

        // Process audio events and forward them to windows
        let events = self.process_audio_events();

        // Drain inbound MPRIS events (multimedia keys, desktop widget
        // clicks, `playerctl …`) and translate them into AudioCommands
        // / viewport commands. Done after `process_audio_events` so any
        // state change we react to is published outward first.
        if let Some(mpris) = self.mpris.as_mut() {
            mpris.poll_events(&self.audio, ctx);
        }

        // Background update check: usually `None` for the first
        // hundred-ish frames while the HTTP request is in flight,
        // then `Some(info)` exactly once, then `None` forever after.
        self.poll_update_checker();

        // Handle user input
        self.handle_keyboard(ctx);
        self.handle_drops(ctx);
        self.windows.handle_shortcuts(ctx);

        let spectrum = self.audio.get_spectrum_data();
        let waveform = self.audio.get_waveform_data();
        let meter = self.audio.get_meter_data();
        let recent_paths: Vec<PathBuf> =
            self.recent.files().iter().map(|r| r.path.clone()).collect();
        let output_devices = self.refresh_output_devices().to_vec();
        let current_output_device = self.config.output_device_name.clone();
        let user_scale = self.user_scale;
        let sleep_timer_minutes = self.active_sleep_timer_minutes();

        // Apply the inline filter (Ctrl+F) by computing a filtered view
        // of the playlist before handing it to the coordinator. The
        // remap table converts filtered indices the playlist window
        // returns in `PlaylistAction` back to the original indices the
        // real `Playlist` uses. When the filter is empty all the
        // `filtered_*` locals are `None` and the playlist UI sees the
        // unmodified playlist (zero overhead on the hot path).
        let (filtered_entries, filter_remap, filtered_selected, filtered_current) =
            build_playlist_filter_view(
                &self.playlist_filter,
                self.playlist.entries(),
                self.playlist.current_index(),
                self.playlist.selected_indices(),
            );
        let entries_view: &[oneamp_core::PlaylistEntry] = filtered_entries
            .as_deref()
            .unwrap_or_else(|| self.playlist.entries());
        let current_view = if filter_remap.is_some() {
            filtered_current
        } else {
            self.playlist.current_index()
        };
        let selected_view: &std::collections::BTreeSet<usize> = filtered_selected
            .as_ref()
            .unwrap_or_else(|| self.playlist.selected_indices());
        // Queue badges, aligned to whichever view (filtered or full) the
        // playlist window will paint. Values are 1-based queue positions
        // in real playlist space, resolved through the filter remap.
        let queued_view: Vec<Option<usize>> = match filter_remap.as_ref() {
            Some(remap) => remap
                .iter()
                .map(|&real_idx| self.playlist.queued_position(real_idx))
                .collect(),
            None => (0..entries_view.len())
                .map(|i| self.playlist.queued_position(i))
                .collect(),
        };

        let actions = self.windows.collect_actions(
            ctx,
            self.audio.engine(),
            &spectrum[..],
            waveform.as_ref(),
            meter.as_ref(),
            &events,
            entries_view,
            current_view,
            selected_view,
            &queued_view,
            self.state.shuffle_enabled,
            !matches!(self.state.repeat_mode, RepeatMode::Off),
            self.always_on_top,
            self.config.crossfade.enabled,
            self.config.replaygain_enabled,
            self.config.replaygain_mode,
            self.config.mono_enabled,
            self.config.loudness_enabled,
            self.config.track_notifications_enabled,
            output_devices,
            current_output_device,
            recent_paths,
            user_scale,
            sleep_timer_minutes,
            self.stop_after_current,
            self.config.resume_long_files,
            self.preset_manager
                .custom_presets()
                .into_iter()
                .cloned()
                .collect(),
            &self.config.playlist_display_format,
            self.config.visualizer_options,
        );

        if let Some(action) = actions.main {
            self.handle_main_window_action(action, ctx);
        }
        // Remap any filtered-space index inside the emitted
        // PlaylistAction before the app sees it — otherwise actions
        // would mutate the wrong row when the filter is active.
        let remapped_playlist_action = match filter_remap.as_ref() {
            Some(remap) => remap_filtered_playlist_action(actions.playlist, remap),
            None => actions.playlist,
        };
        self.handle_playlist_action(remapped_playlist_action);

        self.dispatch_menu_events(ctx);

        // Welcome screen: first launch shows it as a separate OS window.
        // Closing via Done/Skip flips `welcome.open` false; we won't
        // reopen it until `config.first_run` flips back true (which we
        // never do).
        let current_skin_name: Option<String> =
            self.config.bundled_skin_name.clone().or_else(|| {
                self.config
                    .skin_path
                    .as_ref()
                    .and_then(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
            });
        let native_ppp_now = ctx.native_pixels_per_point().unwrap_or(1.0);
        let welcome_actions = welcome::show(
            &mut self.welcome,
            ctx,
            &self.strings,
            self.config.ui_lang,
            self.user_scale,
            native_ppp_now,
            current_skin_name.as_deref(),
            self.windows.active_skin(),
        );
        for action in welcome_actions {
            self.handle_welcome_action(action);
        }
        // Skins… dialog (separate window from welcome). Same action
        // vocabulary so we route through `handle_welcome_action` and
        // share all the persistence + skin-swap plumbing.
        let dialog_actions = welcome::show_skins_dialog(
            &mut self.skins_dialog,
            ctx,
            &self.strings,
            current_skin_name.as_deref(),
            self.windows.active_skin(),
        );
        for action in dialog_actions {
            self.handle_welcome_action(action);
        }

        // Paint the WSZ-skinned cursor on top of every window. Falls
        // through to the OS arrow when the active skin shipped no
        // `.cur` files or when `pick_hot_area` doesn't match the
        // pointer to a known region.
        self.windows.paint_cursor(ctx);

        // Drop-target hint: when the OS reports files being dragged
        // anywhere over the viewport, outline the whole window in the
        // skin's accent green for one frame so the user knows OneAmp
        // will accept the drop. Cheap (one stroke), zero new state,
        // disappears the moment the drag leaves or completes.
        let dragging = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if dragging {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_hint"),
            ));
            let rect = ctx.screen_rect();
            painter.rect_stroke(
                rect.shrink(1.5),
                0.0,
                egui::Stroke::new(3.0, egui::Color32::from_rgb(60, 220, 100)),
            );
        }

        if self.show_hotkeys {
            self.paint_hotkey_overlay(ctx);
        }

        // Playlist inline filter overlay (Ctrl+F). Painted before
        // the badge / toast so they can stack above it without
        // overlapping its text field. The overlay also handles its
        // own Esc-to-close and clears the filter on dismiss.
        self.paint_playlist_filter_overlay(ctx);

        // Sleep-timer badge (top-right corner of the viewport, only
        // while armed). Painted before the toast so a transient toast
        // can land at the bottom without overlapping it.
        self.paint_sleep_badge(ctx);

        // In-app toast (Snackbar). Single-slot — `push_toast` replaces
        // any in-flight one. Renders at the bottom of the viewport,
        // fades out over its last 250 ms, self-clears on expiry.
        self.paint_toast(ctx);

        // Tag editor / URL / display-format dialogs spawned by the
        // playlist context menu. Each one drains on Cancel or Save —
        // matching the `Outcome::*` vocabulary of the dialog module.
        if let Some(mut dlg) = self.tag_editor.take() {
            match dlg.show(ctx, self.windows.active_skin()) {
                crate::tag_editor_dialog::Outcome::None => {
                    self.tag_editor = Some(dlg);
                }
                crate::tag_editor_dialog::Outcome::Cancelled => {}
                crate::tag_editor_dialog::Outcome::Accepted(idx) => {
                    // Reload the entry's metadata from disk so the
                    // updated tags are reflected in the playlist row
                    // without restarting. `refresh_entry_metadata`
                    // re-reads via `TrackInfo::from_file`.
                    self.playlist.refresh_entry_metadata(idx);
                }
            }
        }
        if let Some(mut dlg) = self.url_dialog.take() {
            match dlg.show(ctx, self.windows.active_skin()) {
                crate::url_dialog::Outcome::None => {
                    self.url_dialog = Some(dlg);
                }
                crate::url_dialog::Outcome::Cancelled => {}
                crate::url_dialog::Outcome::Accepted(url) => {
                    // Reject obviously-unsupported schemes up front so
                    // the audio thread doesn't waste a connect attempt
                    // on `file://` / `ftp://`. The actual GET still
                    // runs on the audio thread.
                    match oneamp_core::http_stream::validate_stream_url(&url) {
                        Ok(()) => {
                            self.audio.send_command(AudioCommand::PlayUrl(url.clone()));
                            // Append the URL to the playlist so the
                            // user can re-open it without re-typing.
                            // `add_track` would try to read tags from
                            // disk; we instead push a synthetic entry
                            // whose path is the URL itself.
                            let entry = oneamp_core::PlaylistEntry::with_metadata(
                                std::path::PathBuf::from(&url),
                                Some(url.clone()),
                                None,
                                None,
                                None,
                            );
                            self.playlist.add_entry(entry);
                        }
                        Err(e) => {
                            crate::dialog_util::show_error(&format!("{}", e));
                        }
                    }
                }
            }
        }
        if let Some(mut dlg) = self.format_dialog.take() {
            match dlg.show(ctx, self.windows.active_skin()) {
                crate::format_dialog::Outcome::None => {
                    self.format_dialog = Some(dlg);
                }
                crate::format_dialog::Outcome::Cancelled => {}
                crate::format_dialog::Outcome::Accepted(template) => {
                    self.config.playlist_display_format = template;
                    self.mark_dirty();
                }
            }
        }

        // Preset name dialog: drains on Save / Cancel. On Save we
        // snapshot the live EQ band curve + preamp into a new
        // `EqualizerPreset`, push it through `PresetManager`, and
        // flush the JSON. The EQ window picks it up on the next
        // frame via the per-frame `user_eq_presets` arg into
        // `collect_actions`.
        if let Some(mut dlg) = self.preset_name_dialog.take() {
            match dlg.show(ctx, self.windows.active_skin()) {
                crate::preset_name_dialog::Outcome::None => {
                    self.preset_name_dialog = Some(dlg);
                }
                crate::preset_name_dialog::Outcome::Cancelled => {}
                crate::preset_name_dialog::Outcome::Accepted(name) => {
                    self.commit_user_preset(name);
                }
            }
        }

        // Watch for any persistable state drifting away from what we last
        // wrote out — this catches volume slider drags, balance moves,
        // MPRIS-driven mutations and EQ tweaks in one place, without
        // needing every mutation site to remember to call `mark_dirty()`.
        // Cheap: ~12 scalar compares + one slice compare on EQ gains.
        self.check_persistable_drift();

        // Flush the dirty config once the user has been idle for
        // CONFIG_SAVE_DEBOUNCE. The flush itself is atomic (tmpfile +
        // rename) and < 4 KB; the debounce is so we don't fsync on
        // every frame of a slider drag.
        if let Some(t) = self.config_dirty_since
            && t.elapsed() >= CONFIG_SAVE_DEBOUNCE
        {
            self.flush_config();
        }

        // Apply user-driven scale override. Same dance as the boot-time
        // render_scale_applied path: queue `set_pixels_per_point`, wait
        // for egui to settle, then invalidate the coordinator's viewport
        // cache so the next resize round picks up the new physical size.
        //
        // When `user_scale == None` ("Auto (DPI)" menu entry), we don't
        // freeze pixels_per_point at whatever value the user last picked
        // — instead we re-derive the integer scale from the OS DPI right
        // here and push it through, so clicking Auto actually flips back
        // to the auto-detected value instead of doing nothing visible.
        if self.scale_dirty {
            let native = ctx.native_pixels_per_point().unwrap_or(1.0);
            let target = self.user_scale.unwrap_or_else(|| pick_render_scale(native));
            let live_ppp = ctx.pixels_per_point();
            if (live_ppp - target).abs() > 0.01 {
                ctx.set_pixels_per_point(target);
            } else {
                self.windows.invalidate_viewport_cache();
                self.scale_dirty = false;
            }
        }

        // Sleep-timer ramp + fire. Runs every frame so the volume fade is
        // smooth; once the deadline is reached we Pause and clear the
        // armed state so a subsequent Play doesn't re-trigger the timer.
        self.tick_sleep_timer();

        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Drain any in-flight audio events so the final flush sees
        // the user's most recent volume / balance / EQ-band echoes,
        // not the stale snapshot from before the last drag release.
        // Without this, a slider-then-close-immediately sequence
        // saves the pre-drag values: the slider dispatched SetVolume
        // straight to the audio thread, the echo VolumeUpdated event
        // arrived in the channel — but `state.volume.level` only
        // catches up when `process_audio_events` drains it, which
        // hadn't happened yet.
        //
        // The 30 ms sleep is sized against the audio thread's 5 ms
        // recv_timeout: in the worst case the command sits ~5 ms in
        // the channel, the thread takes < 1 ms to process and emit,
        // and the echo lands within another 5 ms. 30 ms gives a 5×
        // safety margin without making shutdown feel sluggish.
        std::thread::sleep(std::time::Duration::from_millis(30));
        let _ = self.process_audio_events();
        // Final flush regardless of debounce state — anything the user
        // touched in the last 300 ms still hits disk. Reuses the same
        // mirroring + atomic-write path the debounced ticks use.
        self.flush_config();
        // Resume store: one final write on shutdown so the in-memory
        // upserts since the last throttled save reach disk. Best-effort
        // — a write failure here just means the user's last 0-15 s of
        // progress doesn't carry over (the throttled mid-playback
        // writes already covered the rest).
        if let Some(path) = crate::resume_store::default_path()
            && let Err(e) = self.resume.save(&path)
        {
            eprintln!("Failed to save resume store: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pick_render_scale;

    #[test]
    fn pick_render_scale_snaps_up_to_next_integer() {
        // Integer ppp is returned unchanged (ceil of an integer is
        // itself), so Mac Retina / Windows 100-200-300 % users see the
        // same scale they always have.
        assert_eq!(pick_render_scale(1.0), 1.0);
        assert_eq!(pick_render_scale(2.0), 2.0);
        assert_eq!(pick_render_scale(3.0), 3.0);

        // Anything fractional rounds up so the WSZ atlas always lands
        // on a uniform N×N nearest-neighbour magnification.
        assert_eq!(pick_render_scale(1.01), 2.0);
        assert_eq!(pick_render_scale(1.25), 2.0);
        assert_eq!(pick_render_scale(1.5), 2.0);
        assert_eq!(pick_render_scale(1.99), 2.0);
        assert_eq!(pick_render_scale(2.01), 3.0);
        assert_eq!(pick_render_scale(2.5), 3.0);
        assert_eq!(pick_render_scale(2.99), 3.0);

        // Sub-1 ppp (or a bogus DPI report) clamps to 1.0 so the
        // player never shrinks into a sub-pixel footprint.
        assert_eq!(pick_render_scale(0.0), 1.0);
        assert_eq!(pick_render_scale(0.5), 1.0);
        assert_eq!(pick_render_scale(0.99), 1.0);
        assert_eq!(pick_render_scale(-1.0), 1.0);
    }
}
