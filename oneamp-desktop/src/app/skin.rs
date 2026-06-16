//! Skin loading & font registration.
//!
//! Bundled default-skin bytes, the persist-resolution helper, the
//! per-skin egui font registration pass and the two `OneAmpApp`
//! methods that apply a new skin live here so `app/mod.rs` doesn't
//! have to know about WSZ internals.
//!
//! `WSZ_PLEDIT_FONT_FAMILY` is re-exported from `app/mod.rs` to keep
//! the public path `crate::app::WSZ_PLEDIT_FONT_FAMILY` stable for
//! the wsz_ui modules that key playlist row rendering off it.

use super::OneAmpApp;
use crate::skins::{self, SkinEntry, SkinPayload};
use oneamp_core::wsz::loader::WszLoader;
use oneamp_core::wsz::skin::WszSkin;
use std::path::{Path, PathBuf};

/// Bytes for the bundled default Winamp skin. Embedded so a fresh
/// install can always render *some* skin without depending on an
/// install path. Loaded when `config.skin_path` is `None` or when
/// loading the user-selected skin fails.
const DEFAULT_SKIN_BYTES: &[u8] = include_bytes!("../../../skins/base-2.91.wsz");

pub(super) fn load_bundled_default_skin() -> WszSkin {
    match WszLoader::load_from_bytes(DEFAULT_SKIN_BYTES) {
        Ok(skin) => skin,
        Err(e) => {
            eprintln!("Failed to load bundled default skin: {}", e);
            WszSkin::synthetic_default()
        }
    }
}

/// Resolve the persisted "active skin" choice to a live `WszSkin`. The
/// bundled-name override wins over the file path so the user can pick a
/// non-default bundled skin (e.g. `Winamp5_Classified_v5.5`) on the
/// welcome screen and have it survive relaunch without sprinkling
/// extracted .wsz files on disk.
pub(super) fn load_skin_from_config(
    skin_path: Option<&Path>,
    bundled_skin_name: Option<&str>,
) -> WszSkin {
    if let Some(name) = bundled_skin_name {
        for s in skins::BUNDLED_SKINS {
            if s.name.eq_ignore_ascii_case(name) {
                match WszLoader::load_from_bytes(s.bytes) {
                    Ok(skin) => return skin,
                    Err(e) => {
                        eprintln!("Failed to load bundled skin '{}': {}", name, e);
                        break;
                    }
                }
            }
        }
    }
    if let Some(path) = skin_path {
        match WszLoader::load_from_file(path) {
            Ok(skin) => return skin,
            Err(e) => eprintln!("Failed to load skin {}: {}", path.display(), e),
        }
    }
    load_bundled_default_skin()
}

/// Family name under which a skin's bundled TTF/OTF is registered with
/// egui. The playlist row renderer keys off this name; absence falls back
/// to `FontFamily::Monospace` (egui's built-in).
pub const WSZ_PLEDIT_FONT_FAMILY: &str = "wsz_pledit";

/// Re-register egui's font set so the WSZ skin's bundled TTF (if any) is
/// available under `WSZ_PLEDIT_FONT_FAMILY`. Call this on every skin
/// change. With `font_data == None` the family is dropped, and the
/// playlist falls back to monospace — matches Winamp's "no TTF" path.
pub(super) fn apply_skin_fonts(ctx: &egui::Context, font_data: Option<&std::sync::Arc<Vec<u8>>>) {
    let mut defs = egui::FontDefinitions::default();

    if let Some(data) = font_data {
        // egui's FontDefinitions stores Arc<FontData>. Clone the bytes
        // out of our Arc<Vec<u8>> — egui keeps its own copy, and we're
        // only doing this on skin changes (rare).
        defs.font_data.insert(
            WSZ_PLEDIT_FONT_FAMILY.to_string(),
            std::sync::Arc::new(egui::FontData::from_owned((**data).clone())),
        );
        defs.families.insert(
            egui::FontFamily::Name(WSZ_PLEDIT_FONT_FAMILY.into()),
            vec![WSZ_PLEDIT_FONT_FAMILY.to_string()],
        );
    }

    ctx.set_fonts(defs);
}

impl OneAmpApp {
    /// Load and apply a `.wsz` from disk. Used by both Alt+S and the
    /// `PickSkin` menu entry — keeps the five-step "load, swap fonts,
    /// rebuild windows, persist, mark dirty" sequence in one place.
    pub(super) fn apply_skin_from_file(&mut self, path: PathBuf) {
        match WszLoader::load_from_file(&path) {
            Ok(skin) => {
                self.skin_font_data = skin.font_data.clone();
                self.fonts_dirty = true;
                self.windows.update_skin(&skin);
                self.config.skin_path = Some(path);
                self.config.bundled_skin_name = None;
                self.mark_dirty();
            }
            Err(e) => {
                crate::dialog_util::show_error(&format!("Failed to load skin: {}", e));
            }
        }
    }

    /// Apply a `SkinEntry` (bundled or on-disk) from the welcome screen
    /// or the Skins… dialog. Embedded payloads persist as
    /// `bundled_skin_name`; file payloads persist as `skin_path`.
    pub(super) fn apply_skin_entry(&mut self, entry: SkinEntry) {
        match entry.payload {
            SkinPayload::Embedded(bytes) => match WszLoader::load_from_bytes(bytes) {
                Ok(skin) => {
                    self.skin_font_data = skin.font_data.clone();
                    self.fonts_dirty = true;
                    self.windows.update_skin(&skin);
                    self.config.skin_path = None;
                    self.config.bundled_skin_name = Some(entry.name);
                    self.mark_dirty();
                }
                Err(e) => {
                    crate::dialog_util::show_error(&format!("Failed to load bundled skin: {}", e));
                }
            },
            SkinPayload::File(path) => {
                self.apply_skin_from_file(path);
            }
        }
    }
}
