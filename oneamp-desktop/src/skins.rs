//! Skin discovery and bundling.
//!
//! `SkinSource` enumerates the three places a `.wsz` can come from:
//!   - `Bundled`: compiled into the binary by `build.rs` (BUNDLED_SKINS).
//!   - `UserDir`: a per-user folder (default `<config>/oneamp/skins`),
//!     pointed at by `AppConfig::user_skins_dir`.
//!   - `SystemDir`: distro-installed shared paths (`/usr/share/oneamp/
//!     skins`, the macOS app bundle, `%ProgramFiles%/OneAmp/skins`).
//!
//! `discover` returns a deduped, sorted `Vec<SkinEntry>` ready to render
//! in a picker. Dedup is by file-stem (case-insensitive) so a user copy
//! of a bundled skin in their folder takes precedence over the embedded
//! version — same behaviour `WszLoader::load_from_file` would have if
//! both paths existed.

use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/bundled_skins.rs"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkinSource {
    Bundled,
    UserDir,
    SystemDir,
}

#[derive(Debug, Clone)]
pub enum SkinPayload {
    /// Bundled skin — bytes are linked into the binary at compile time.
    Embedded(&'static [u8]),
    /// On-disk skin discovered in a user or system folder.
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct SkinEntry {
    pub name: String,
    /// Where the skin came from. Currently only used at dedup time
    /// (precedence: user → system → bundled) and as informational
    /// metadata for future surfaces; the welcome / Skins… picker
    /// shows skins as image cards without source badges.
    #[allow(dead_code)]
    pub source: SkinSource,
    pub payload: SkinPayload,
}

impl SkinEntry {
    /// Display name (file stem). Bundled skins keep the name set by
    /// `build.rs`; on-disk skins use the file stem of their path.
    pub fn display_name(&self) -> &str {
        &self.name
    }

    /// Stable identity used for dedup and for "is this the currently
    /// selected skin?" checks. Case-insensitive on the stem; matches the
    /// behaviour of the rest of the player (Windows skin folders pulled
    /// in from Winamp often mix case).
    fn dedup_key(&self) -> String {
        self.name.to_lowercase()
    }
}

/// Default user-skins folder when `AppConfig::user_skins_dir` is `None`.
/// `<config_dir>/oneamp/skins` so it sits next to `config.json`.
pub fn default_user_skins_dir() -> Option<PathBuf> {
    let base = dirs::config_dir()?;
    Some(base.join("oneamp").join("skins"))
}

/// Per-OS list of system-wide skin search paths. None of these are
/// guaranteed to exist — `discover` silently skips missing ones.
fn system_skin_dirs() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();

    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("/usr/share/oneamp/skins"));
        paths.push(PathBuf::from("/usr/local/share/oneamp/skins"));
        if let Ok(xdg) = std::env::var("XDG_DATA_DIRS") {
            for dir in xdg.split(':') {
                if dir.is_empty() {
                    continue;
                }
                paths.push(PathBuf::from(dir).join("oneamp/skins"));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // App-bundle resources path. macOS .app structure is
        // `OneAmp.app/Contents/Resources/skins`.
        if let Ok(exe) = std::env::current_exe()
            && let Some(macos_dir) = exe.parent()
            && let Some(contents) = macos_dir.parent()
        {
            paths.push(contents.join("Resources").join("skins"));
        }
        paths.push(PathBuf::from("/Library/Application Support/OneAmp/skins"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            paths.push(PathBuf::from(pf).join("OneAmp").join("skins"));
        }
        if let Ok(pf) = std::env::var("ProgramFiles(x86)") {
            paths.push(PathBuf::from(pf).join("OneAmp").join("skins"));
        }
    }

    paths
}

/// Walk `dir` (one level, non-recursive) and return every `.wsz` file
/// found. Skips dotfiles and anything we can't stat. Quiet on missing
/// or unreadable directories — a fresh install has no user folder yet.
fn scan_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("wsz"))
                    .unwrap_or(false)
        })
        .collect();
    out.sort();
    out
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

/// Build the deduped skin catalog. `user_dir` is the override from
/// `AppConfig::user_skins_dir`; pass `None` to use the default location.
///
/// Order of precedence (first wins on dedup): user folder, system
/// folders, bundled. So a user copy of `base-2.91.wsz` overrides the
/// embedded version — exactly what you'd want when iterating on a skin.
pub fn discover(user_dir_override: Option<&Path>) -> Vec<SkinEntry> {
    let mut entries: Vec<SkinEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let user_dir = user_dir_override
        .map(|p| p.to_path_buf())
        .or_else(default_user_skins_dir);
    if let Some(dir) = user_dir.as_deref() {
        for path in scan_dir(dir) {
            let name = file_stem(&path);
            let key = name.to_lowercase();
            if seen.insert(key) {
                entries.push(SkinEntry {
                    name,
                    source: SkinSource::UserDir,
                    payload: SkinPayload::File(path),
                });
            }
        }
    }

    for dir in system_skin_dirs() {
        for path in scan_dir(&dir) {
            let name = file_stem(&path);
            let key = name.to_lowercase();
            if seen.insert(key) {
                entries.push(SkinEntry {
                    name,
                    source: SkinSource::SystemDir,
                    payload: SkinPayload::File(path),
                });
            }
        }
    }

    for skin in BUNDLED_SKINS {
        let key = skin.name.to_lowercase();
        if seen.insert(key) {
            entries.push(SkinEntry {
                name: skin.name.to_string(),
                source: SkinSource::Bundled,
                payload: SkinPayload::Embedded(skin.bytes),
            });
        }
    }

    entries.sort_by_key(|a| a.dedup_key());
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_includes_at_least_one_bundled_skin() {
        // BUNDLED_SKINS is populated by build.rs from skins/. The
        // repo ships base-2.91.wsz at minimum, so the discover() result
        // must include it (unless the user folder happens to have a
        // skin with the same stem, which still satisfies the assertion).
        let entries = discover(None);
        let has_base = entries
            .iter()
            .any(|e| e.name.to_lowercase().contains("base"));
        assert!(
            has_base,
            "discover() should include at least the bundled base skin"
        );
    }

    #[test]
    fn dedup_key_lowercases_name() {
        let e = SkinEntry {
            name: "MySkin".to_string(),
            source: SkinSource::Bundled,
            payload: SkinPayload::Embedded(&[]),
        };
        assert_eq!(e.dedup_key(), "myskin");
    }
}
