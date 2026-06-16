use anyhow::{Context, Result};
use oneamp_core::{RecentFiles, RepeatMode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqualizerConfig {
    /// Every field is `#[serde(default)]` so adding a future field
    /// (or migrating from an older schema that lacked one) never
    /// causes the entire config load to fail. Missing fields fall
    /// back to the `Default for EqualizerConfig` values.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_eq_gains")]
    pub gains: Vec<f32>,
    /// Master pre-amp gain in dB applied after EQ processing. Range
    /// matches the band sliders (±20 dB). Persisted across sessions.
    #[serde(default)]
    pub preamp_db: f32,
    #[serde(default)]
    pub current_preset: Option<String>,
}

fn default_eq_gains() -> Vec<f32> {
    vec![0.0; 10]
}

impl Default for EqualizerConfig {
    fn default() -> Self {
        // Delegate per-field defaults to the same `default_*` helpers
        // serde uses for missing-field recovery, so the in-memory
        // default and the deserialized-with-missing-field default can
        // never drift apart.
        Self {
            enabled: false,
            gains: default_eq_gains(),
            preamp_db: 0.0,
            current_preset: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AudioEffectsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub master_bypass: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossfadeConfigWrapper {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_crossfade_duration")]
    pub duration_secs: f32,
}

impl Default for CrossfadeConfigWrapper {
    fn default() -> Self {
        Self {
            enabled: false,
            duration_secs: default_crossfade_duration(),
        }
    }
}

fn default_crossfade_duration() -> f32 {
    3.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaplessConfigWrapper {
    #[serde(default = "default_gapless_enabled")]
    pub enabled: bool,
    #[serde(default = "default_prebuffer")]
    pub prebuffer_secs: f32,
}

impl Default for GaplessConfigWrapper {
    fn default() -> Self {
        Self {
            enabled: default_gapless_enabled(),
            prebuffer_secs: default_prebuffer(),
        }
    }
}

fn default_gapless_enabled() -> bool {
    true
}

fn default_prebuffer() -> f32 {
    2.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackConfig {
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub repeat_mode: RepeatModeConfig,
    #[serde(default)]
    pub shuffle_enabled: bool,
    /// Last stereo balance, -1.0 (full left) to 1.0 (full right). Persisted
    /// so a user who pinned playback to one ear keeps that on relaunch.
    #[serde(default)]
    pub balance: f32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            volume: default_volume(),
            muted: false,
            repeat_mode: RepeatModeConfig::Off,
            shuffle_enabled: false,
            balance: 0.0,
        }
    }
}

fn default_volume() -> f32 {
    1.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum RepeatModeConfig {
    #[default]
    Off,
    One,
    All,
}

impl From<RepeatModeConfig> for RepeatMode {
    fn from(mode: RepeatModeConfig) -> Self {
        match mode {
            RepeatModeConfig::Off => RepeatMode::Off,
            RepeatModeConfig::One => RepeatMode::One,
            RepeatModeConfig::All => RepeatMode::All,
        }
    }
}

impl From<RepeatMode> for RepeatModeConfig {
    fn from(mode: RepeatMode) -> Self {
        match mode {
            RepeatMode::Off => RepeatModeConfig::Off,
            RepeatMode::One => RepeatModeConfig::One,
            RepeatMode::All => RepeatModeConfig::All,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Every field is `#[serde(default)]` so a config file written by
    /// an older OneAmp version (missing fields added since) still
    /// loads cleanly — missing fields fall back to the type's
    /// `Default` impl rather than failing the whole load. The
    /// lenient loader in `load()` adds a second layer for the rarer
    /// "field present but mistyped" case (a future enum variant got
    /// renamed, an int became a string, …) by parsing the on-disk
    /// JSON as `Value` and deserializing field-by-field.
    #[serde(default)]
    pub equalizer: EqualizerConfig,
    #[serde(default)]
    pub audio_effects: AudioEffectsConfig,
    #[serde(default)]
    pub crossfade: CrossfadeConfigWrapper,
    #[serde(default)]
    pub gapless: GaplessConfigWrapper,
    #[serde(default)]
    pub playback: PlaybackConfig,
    #[serde(default = "default_first_run")]
    pub first_run: bool,
    /// Path to a `.wsz` Winamp skin file. `None` falls back to the built-in default skin.
    #[serde(default)]
    pub skin_path: Option<PathBuf>,
    /// Stem name of a bundled skin (e.g. `"base-2.91"`,
    /// `"Winamp5_Classified_v5.5"`) chosen from the skin picker. When
    /// `Some`, this takes precedence over `skin_path` at load time so
    /// the user can swap between embedded skins without sprinkling
    /// extracted files on disk. Cleared to `None` whenever a file-path
    /// skin is selected from the file picker.
    #[serde(default)]
    pub bundled_skin_name: Option<String>,
    /// Persisted "Always on Top" flag. Toggled from the clutterbar
    /// Options menu; applied via `egui::ViewportCommand::WindowLevel`
    /// at boot and on every change.
    #[serde(default)]
    pub always_on_top: bool,
    /// Most-recently-played files surfaced in the clutterbar Options
    /// menu. Updated on every `Play` command; persisted across sessions.
    #[serde(default)]
    pub recent_files: RecentFiles,
    /// Opt-in ReplayGain track-level gain normalization. Off by default
    /// so users without RG-tagged libraries see zero behaviour change.
    /// Toggled from the clutterbar Options menu; the audio thread reads
    /// `REPLAYGAIN_TRACK_GAIN` from each loaded track and stacks it on
    /// the user's preamp setting.
    #[serde(default)]
    pub replaygain_enabled: bool,
    /// Which ReplayGain reference the audio thread normalizes to when
    /// `replaygain_enabled` is on: per-track, per-album, or auto
    /// (album-with-track-fallback). Defaults to `Track`, matching the
    /// historical behaviour. Chosen from Audio → ReplayGain.
    #[serde(default)]
    pub replaygain_mode: oneamp_core::ReplayGainMode,
    /// Downmix stereo to mono on the fly (Audio → Mono). Off by default.
    /// Mirrors Winamp's mono toggle — useful on single-speaker setups or
    /// to check a mix's mono compatibility.
    #[serde(default)]
    pub mono_enabled: bool,
    /// Selected audio output device name, matching one of the entries
    /// returned by `oneamp_core::list_output_devices`. `None` means
    /// "use the host's default output". Persisted so a deliberate
    /// USB-DAC pick survives reboot; falls back to default silently
    /// if the named device is no longer present at next launch.
    #[serde(default)]
    pub output_device_name: Option<String>,
    /// Whether to fire an `org.freedesktop.Notifications` toast on each
    /// track change. Off by default — desktop integration users tend to
    /// already get this from the MPRIS PlaybackStatus signal via their
    /// notification daemon, and toasts every 3 minutes during a playlist
    /// can get noisy on small displays. Toggled from the Options menu.
    #[serde(default)]
    pub track_notifications_enabled: bool,
    /// Volume-dependent loudness compensation (Fletcher-Munson / ISO 226
    /// inspired). Off by default. When on, the audio thread applies
    /// low- and high-shelf boosts that scale with master attenuation —
    /// at full volume there's no effect, at quiet listening the bass /
    /// treble curve compensates for the ear's reduced sensitivity.
    /// Toggled from the Options menu.
    #[serde(default)]
    pub loudness_enabled: bool,
    /// Highest version we've already announced via the startup update
    /// check. Stored so the same release doesn't re-prompt on every
    /// launch — when the GitHub check returns this same string, we
    /// silently move on. `None` means "we've never announced any
    /// update", which is the fresh-install state.
    #[serde(default)]
    pub last_notified_update_version: Option<String>,
    /// User override for the egui `pixels_per_point`. `None` means
    /// "follow the DPI-derived auto heuristic" — `pick_render_scale`
    /// runs on every launch. When set, this value wins. Quantised to
    /// 0.25-unit steps from 1.0 to 4.0 (`1.0`, `1.25`, …, `4.0`).
    /// Fractional values introduce some sub-pixel sampling of the WSZ
    /// sprite atlas, but the player text and slider thumbs scale
    /// smoothly — the user explicitly opted into the trade-off in
    /// 1.0.6 when they asked for finer granularity than 1×/2×/3×/4×.
    /// Stored as `f32` so old configs that saved an integer (1, 2, 3,
    /// 4) deserialize cleanly (JSON numbers round-trip through `f32`).
    #[serde(default)]
    pub user_scale: Option<f32>,
    /// Last selected main-window visualizer (Spectrum / Oscilloscope /
    /// Off). Persisted so the user keeps their chosen analyser between
    /// sessions; "Off" turns the (24,43,76,16) zone blank.
    #[serde(default)]
    pub visualizer_mode: VisualizerModeConfig,
    /// Spectrum / oscilloscope render options (peak-hold, falloff speed,
    /// oscilloscope style). Persisted so the user's Winamp-style tuning
    /// survives a restart.
    #[serde(default)]
    pub visualizer_options: crate::wsz_ui::components::visualization::VisualizerOptions,
    /// Whether the player was last in shade (compact 14-px) mode. Restored
    /// at startup so a user who prefers the mini bar keeps it across
    /// sessions.
    #[serde(default)]
    pub shade_mode: bool,
    /// Whether the equalizer sub-window was open at last close. Restored
    /// at startup so the user's previous layout reappears.
    #[serde(default)]
    pub show_equalizer: bool,
    /// Whether the playlist sub-window was open at last close.
    #[serde(default)]
    pub show_playlist: bool,
    /// UI language picked on the welcome screen (or auto-detected from the
    /// OS locale on first launch). Affects the welcome screen, the Skins…
    /// dialog and any string the i18n table covers. Stored as a string
    /// (`"auto"` / `"en"` / `"fr"`) so unknown values can be added later
    /// without breaking older binaries.
    #[serde(default)]
    pub ui_lang: LangConfig,
    /// Where to look for user-provided `.wsz` skins. `None` falls back to
    /// `<config_dir>/oneamp/skins`. The Skins… dialog lets the user point
    /// this elsewhere (network share, music drive, …). Bundled and
    /// system-wide skin paths are always scanned in addition to this.
    #[serde(default)]
    pub user_skins_dir: Option<PathBuf>,
    /// Template string used to render each playlist row. Tokens are
    /// `{artist}`, `{title}`, `{album}`, `{genre}`, `{tracknumber}`,
    /// `{year}`, `{duration}`, `{filename}`. Missing fields collapse
    /// gracefully — e.g. `"{artist} - {title}"` against a tag-less file
    /// falls back to the filename. Default mirrors Winamp / iTunes.
    #[serde(default = "default_playlist_format")]
    pub playlist_display_format: String,
    /// Whether the digital time display shows remaining time (prefix
    /// `-`) instead of elapsed time. Clicking the time display toggles
    /// this — Winamp convention. Persisted so a user who prefers the
    /// remaining-time read-out keeps it across sessions.
    #[serde(default)]
    pub show_remaining: bool,
    /// Opt-in: persist playback position for files longer than ~30 min
    /// (audiobooks, long mixes, lecture recordings) so the next launch
    /// resumes where the user left off. Stored in
    /// `<config_dir>/oneamp/resume.json` keyed by absolute path. Short
    /// files are ignored regardless of this flag — restoring a 3-min
    /// pop song to 2 m 14 s would be more annoying than useful.
    #[serde(default)]
    pub resume_long_files: bool,
}

/// Default template applied when a config lacks `playlist_display_format`.
/// Mirrors Winamp's classic "Artist - Title" convention; the formatter
/// silently falls back to the filename for untagged files.
pub fn default_playlist_format() -> String {
    "{artist} - {title}".to_string()
}

/// Persisted form of `i18n::Lang`. Mirrored here so the i18n module
/// can stay free of serde.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LangConfig {
    #[default]
    Auto,
    En,
    Fr,
}

/// Persisted form of `wsz_ui::main_window::VisualizerMode`. Mirrored
/// here so the UI enum can stay private to the wsz_ui module.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum VisualizerModeConfig {
    #[default]
    Spectrum,
    Oscilloscope,
    PeakMeter,
    Off,
}

fn default_first_run() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            equalizer: EqualizerConfig::default(),
            audio_effects: AudioEffectsConfig::default(),
            crossfade: CrossfadeConfigWrapper::default(),
            gapless: GaplessConfigWrapper::default(),
            playback: PlaybackConfig::default(),
            first_run: default_first_run(),
            skin_path: None,
            bundled_skin_name: None,
            always_on_top: false,
            recent_files: RecentFiles::default(),
            replaygain_enabled: false,
            replaygain_mode: oneamp_core::ReplayGainMode::default(),
            mono_enabled: false,
            loudness_enabled: false,
            output_device_name: None,
            track_notifications_enabled: false,
            last_notified_update_version: None,
            user_scale: None,
            visualizer_mode: VisualizerModeConfig::default(),
            visualizer_options:
                crate::wsz_ui::components::visualization::VisualizerOptions::default(),
            shade_mode: false,
            show_equalizer: false,
            show_playlist: false,
            ui_lang: LangConfig::default(),
            user_skins_dir: None,
            playlist_display_format: default_playlist_format(),
            show_remaining: false,
            resume_long_files: false,
        }
    }
}

impl AppConfig {
    /// Get the config file path
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir().context("Failed to get config directory")?;
        let oneamp_dir = config_dir.join("oneamp");

        // Create directory if it doesn't exist
        if !oneamp_dir.exists() {
            fs::create_dir_all(&oneamp_dir).context("Failed to create config directory")?;
        }

        Ok(oneamp_dir.join("config.json"))
    }

    /// Load configuration from file. Returns `(config, is_first_run)`.
    ///
    /// The loader is intentionally lenient so a version upgrade never
    /// wipes user settings. Three stages, each strictly more forgiving
    /// than the previous:
    ///
    /// 1. **Strict deserialize.** The fast path — succeeds whenever
    ///    the on-disk schema matches the current `AppConfig` shape
    ///    (which thanks to `#[serde(default)]` on every field covers
    ///    "older version that lacks some new fields" too).
    /// 2. **Lenient field-by-field merge.** If strict fails (a field
    ///    type changed across versions, an enum variant got renamed,
    ///    a numeric got stringified by a previous bug, …), reparse as
    ///    `serde_json::Value` and walk the top-level fields one by
    ///    one. Anything that round-trips cleanly through serde_json
    ///    overwrites the corresponding default; anything that fails
    ///    keeps the default and the user just loses that one field.
    /// 3. **Hard reset with backup.** When even step 2's JSON parse
    ///    fails (truncated file, partial write, hand-edited and
    ///    busted), copy the broken file to `config.json.broken-<ts>`
    ///    and fall back to defaults. The user can manually salvage
    ///    fields from the backup if they want.
    ///
    /// Step 2 is the bit the user actually feels: across upgrades,
    /// any field that's still recognisable gets preserved, only the
    /// truly-incompatible ones reset.
    pub fn load() -> (Self, bool) {
        let path = match Self::config_path() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to get config path: {}", e);
                return (Self::default(), true);
            }
        };
        if !path.exists() {
            return (Self::default(), true);
        }
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to read config file: {}", e);
                return (Self::default(), true);
            }
        };

        // Stage 1: strict deserialize.
        if let Ok(mut config) = serde_json::from_str::<AppConfig>(&content) {
            let is_first = config.first_run;
            config.first_run = false;
            return (config, is_first);
        }

        // Stage 2: parse as Value, merge field-by-field into a fresh
        // default. Anything that fails to deserialize at its expected
        // type silently keeps the default — the rest survives.
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            eprintln!(
                "Config schema partially unrecognised; falling back to per-field lenient load. \
                 Fields that fail to deserialize at their current type will reset to default."
            );
            let mut config = Self::default();
            config.lenient_merge_from_value(&value);
            // Lenient-loaded means the file exists and the user has
            // already onboarded — don't re-trigger the first-run
            // welcome flow.
            config.first_run = false;
            return (config, false);
        }

        // Stage 3: even Value::deserialize failed — JSON itself is
        // corrupt. Back up the file before we overwrite it on the
        // next save, so the user can still rescue the data if they
        // want.
        let backup = path.with_extension(format!(
            "json.broken-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        ));
        if let Err(e) = fs::copy(&path, &backup) {
            eprintln!("Failed to back up corrupt config: {}", e);
        } else {
            eprintln!(
                "Config file is corrupt JSON; backed up to {} and resetting to defaults.",
                backup.display()
            );
        }
        (Self::default(), false)
    }

    /// Overwrite each field on `self` with the matching entry from
    /// `value` *if* that entry deserialises to the field's type.
    /// Fields whose JSON value is missing or type-mismatched keep
    /// the `Default::default()` value `self` was built with.
    ///
    /// This is intentionally a flat top-level walk: each AppConfig
    /// field is treated as an atom. Mid-struct partial recovery (an
    /// old `EqualizerConfig` that lost one field but kept the
    /// others) is already covered by the `#[serde(default)]`
    /// attributes on the sub-struct's fields — `serde_json::from_value`
    /// honours them.
    fn lenient_merge_from_value(&mut self, value: &serde_json::Value) {
        let serde_json::Value::Object(map) = value else {
            return;
        };
        macro_rules! pull {
            ($field:ident) => {
                if let Some(v) = map.get(stringify!($field))
                    && let Ok(parsed) = serde_json::from_value(v.clone())
                {
                    self.$field = parsed;
                }
            };
        }
        pull!(equalizer);
        pull!(audio_effects);
        pull!(crossfade);
        pull!(gapless);
        pull!(playback);
        pull!(first_run);
        pull!(skin_path);
        pull!(bundled_skin_name);
        pull!(always_on_top);
        pull!(recent_files);
        pull!(replaygain_enabled);
        pull!(replaygain_mode);
        pull!(mono_enabled);
        pull!(output_device_name);
        pull!(track_notifications_enabled);
        pull!(loudness_enabled);
        pull!(last_notified_update_version);
        pull!(user_scale);
        pull!(visualizer_mode);
        pull!(visualizer_options);
        pull!(shade_mode);
        pull!(show_equalizer);
        pull!(show_playlist);
        pull!(ui_lang);
        pull!(user_skins_dir);
        pull!(playlist_display_format);
    }

    /// Save configuration to file. Uses a write-to-tmp + rename dance so a
    /// crash mid-write never leaves a half-truncated `config.json` behind —
    /// the user always sees either the previous good state or the new one.
    /// `fsync` on the tmpfile before rename is paranoid but cheap (config
    /// is < 4 KB and we save at most once per ~750 ms of activity).
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self).context("Failed to serialize config")?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp).context("Failed to create temp config file")?;
            f.write_all(content.as_bytes())
                .context("Failed to write temp config file")?;
            f.sync_all().context("Failed to fsync temp config file")?;
        }
        fs::rename(&tmp, &path).context("Failed to rename temp config file into place")?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_equalizer_config_default() {
        let config = EqualizerConfig::default();
        assert!(!config.enabled, "Equalizer should be disabled by default");
        assert_eq!(config.gains.len(), 10, "Should have 10 bands");
        assert!(
            config.gains.iter().all(|&g| g == 0.0),
            "All gains should be 0.0"
        );
    }

    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert!(
            !config.equalizer.enabled,
            "Equalizer should be disabled by default"
        );
        assert!(config.first_run, "Should be first run by default");
    }

    #[test]
    fn test_equalizer_config_serialization() {
        let config = EqualizerConfig {
            enabled: true,
            gains: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
            preamp_db: 3.5,
            current_preset: None,
        };

        let json = serde_json::to_string(&config).expect("Should serialize");
        let deserialized: EqualizerConfig =
            serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(config.enabled, deserialized.enabled);
        assert_eq!(config.gains, deserialized.gains);
    }

    #[test]
    fn test_app_config_serialization() {
        let mut config = AppConfig::default();
        config.equalizer = EqualizerConfig {
            enabled: true,
            gains: vec![1.0; 10],
            preamp_db: 0.0,
            current_preset: None,
        };
        config.first_run = false;
        config.skin_path = Some(PathBuf::from("/skins/classic.wsz"));

        let json = serde_json::to_string(&config).expect("Should serialize");
        let deserialized: AppConfig = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(config.equalizer.enabled, deserialized.equalizer.enabled);
        assert_eq!(config.first_run, deserialized.first_run);
        assert_eq!(config.skin_path, deserialized.skin_path);
    }

    #[test]
    fn test_skin_path_default() {
        let config = AppConfig::default();
        assert!(config.skin_path.is_none());
    }

    #[test]
    fn test_skin_path_persistence() {
        let mut config = AppConfig::default();
        config.skin_path = Some(PathBuf::from("/home/u/skins/winamp5.wsz"));

        let json = serde_json::to_string(&config).expect("Should serialize");
        let deserialized: AppConfig = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(config.skin_path, deserialized.skin_path);
    }

    #[test]
    fn test_config_save_and_load() {
        // Create a test config
        let mut config = AppConfig::default();
        config.equalizer.enabled = true;
        config.equalizer.gains = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        config.first_run = false;

        // Save it
        let save_result = config.save();
        // May fail if no config directory is available, that's ok for testing
        if save_result.is_ok() {
            // Load it back
            let (loaded_config, is_first_run) = AppConfig::load();

            // Verify values
            assert_eq!(config.equalizer.enabled, loaded_config.equalizer.enabled);
            assert_eq!(config.equalizer.gains, loaded_config.equalizer.gains);
            // first_run should be false after loading
            assert!(!is_first_run, "Should not be first run after save/load");
        }
    }

    #[test]
    fn test_config_path() {
        // Test that config_path returns a valid path
        let path_result = AppConfig::config_path();
        // Should either succeed or fail gracefully
        match path_result {
            Ok(path) => {
                assert!(
                    path.ends_with("config.json"),
                    "Path should end with config.json"
                );
                assert!(
                    path.to_string_lossy().contains("oneamp"),
                    "Path should contain oneamp"
                );
            }
            Err(_) => {
                // It's ok if it fails in test environment
            }
        }
    }

    #[test]
    fn test_first_run_detection() {
        let (config, _is_first_run) = AppConfig::load();
        assert_eq!(config.equalizer.gains.len(), 10);
    }

    #[test]
    fn lenient_merge_preserves_known_fields_from_partial_object() {
        // Simulates a future on-disk schema that lacks the
        // `playlist_display_format` field and uses an older shape
        // for one sub-object. We expect every recognisable field to
        // survive, defaults to fill in the rest.
        let gains_json = vec![3.0_f32; 10];
        let raw = serde_json::json!({
            "always_on_top": true,
            "user_scale": 2.5,
            "playback": { "volume": 0.42, "muted": true },
            "equalizer": { "enabled": true, "gains": gains_json },
        });
        let mut cfg = AppConfig::default();
        cfg.lenient_merge_from_value(&raw);
        assert!(cfg.always_on_top, "explicit field should overwrite default");
        assert_eq!(cfg.user_scale, Some(2.5));
        assert!((cfg.playback.volume - 0.42).abs() < 1e-6);
        assert!(cfg.playback.muted);
        assert!(cfg.equalizer.enabled);
        assert_eq!(cfg.equalizer.gains, vec![3.0; 10]);
        // Untouched fields keep defaults.
        assert!(!cfg.replaygain_enabled);
        assert_eq!(
            cfg.playlist_display_format,
            default_playlist_format(),
            "fields the on-disk JSON doesn't mention fall back to default"
        );
    }

    #[test]
    fn lenient_merge_keeps_default_on_mistyped_field() {
        // Simulates a future where `user_scale` changed type. The
        // on-disk value won't deserialize as `Option<f32>` — the
        // lenient merge should silently keep the default, NOT panic
        // and NOT wipe the rest of the config.
        let raw = serde_json::json!({
            "always_on_top": true,
            "user_scale": { "factor": 2.0 }, // wrong shape
        });
        let mut cfg = AppConfig::default();
        cfg.lenient_merge_from_value(&raw);
        assert!(
            cfg.always_on_top,
            "good field survives next to a broken one"
        );
        assert!(
            cfg.user_scale.is_none(),
            "mistyped field falls back to default (None)"
        );
    }

    #[test]
    fn equalizer_config_deserializes_with_only_partial_fields() {
        // An older OneAmp wrote `{ "enabled": true, "gains": [...] }`
        // with no `preamp_db` or `current_preset`. The serde
        // attributes must fill those with defaults instead of failing.
        let raw = r#"{ "enabled": true, "gains": [1.0, 2.0, 3.0] }"#;
        let cfg: EqualizerConfig = serde_json::from_str(raw)
            .expect("partial EqualizerConfig should deserialize via #[serde(default)]");
        assert!(cfg.enabled);
        assert_eq!(cfg.gains, vec![1.0, 2.0, 3.0]);
        assert_eq!(cfg.preamp_db, 0.0);
        assert!(cfg.current_preset.is_none());
    }

    #[test]
    fn appconfig_deserializes_with_only_some_top_level_fields() {
        // The biggest upgrade-survival test: a config that only
        // carries volume + EQ enabled — everything else is missing —
        // must still load without error.
        let raw = r#"{
            "playback": { "volume": 0.6 },
            "equalizer": { "enabled": true }
        }"#;
        let cfg: AppConfig = serde_json::from_str(raw)
            .expect("partial AppConfig should deserialize via #[serde(default)]");
        assert!((cfg.playback.volume - 0.6).abs() < 1e-6);
        assert!(cfg.equalizer.enabled);
        assert_eq!(cfg.equalizer.gains.len(), 10);
        assert_eq!(cfg.playlist_display_format, default_playlist_format());
    }

    #[test]
    fn test_playback_config_default() {
        let config = PlaybackConfig::default();
        assert_eq!(config.volume, 1.0);
        assert!(!config.muted);
        assert_eq!(config.repeat_mode, RepeatModeConfig::Off);
    }

    #[test]
    fn test_repeat_mode_conversion() {
        use oneamp_core::RepeatMode;

        let off: RepeatMode = RepeatModeConfig::Off.into();
        assert_eq!(off, RepeatMode::Off);

        let one: RepeatMode = RepeatModeConfig::One.into();
        assert_eq!(one, RepeatMode::One);

        let all: RepeatMode = RepeatModeConfig::All.into();
        assert_eq!(all, RepeatMode::All);
    }

    #[test]
    fn test_repeat_mode_roundtrip() {
        use oneamp_core::RepeatMode;

        let modes = vec![RepeatMode::Off, RepeatMode::One, RepeatMode::All];

        for mode in modes {
            let config_mode: RepeatModeConfig = mode.into();
            let back: RepeatMode = config_mode.into();
            assert_eq!(mode, back);
        }
    }

    #[test]
    fn test_volume_bounds() {
        let mut config = PlaybackConfig::default();
        config.volume = 0.5;

        assert!(config.volume >= 0.0 && config.volume <= 1.0);
    }

    #[test]
    fn test_config_with_playback() {
        let mut config = AppConfig::default();
        config.playback = PlaybackConfig {
            volume: 0.75,
            muted: true,
            repeat_mode: RepeatModeConfig::One,
            shuffle_enabled: false,
            balance: 0.0,
        };
        config.first_run = false;

        assert_eq!(config.playback.volume, 0.75);
        assert!(config.playback.muted);
        assert_eq!(config.playback.repeat_mode, RepeatModeConfig::One);
    }
}
