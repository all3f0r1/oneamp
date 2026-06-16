//! Minimal i18n layer for the welcome screen and skins dialog.
//!
//! Translation table is compile-time: each string lives as a method on
//! `Strings` returning a `&'static str` selected by the active language.
//! No HashMap, no key strings, no missing-key footguns — the compiler
//! refuses to build if a method's `match` arm is missing.
//!
//! Auto-detection looks at `$LANG` / `$LC_ALL` / `$LC_MESSAGES` on Linux
//! and macOS. On Windows it reads `GetUserDefaultLocaleName`. Anything
//! that starts with `fr` resolves to French; everything else (including
//! parse failures) falls back to English.

use crate::config::LangConfig;

/// Resolved UI language. `Auto` collapses to either `En` or `Fr` via
/// `Lang::resolve_auto` at app boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// Resolve a `LangConfig` (which can be `Auto`) into a concrete `Lang`.
    /// `Auto` peeks at the OS locale and picks French when it starts with
    /// `fr`, English otherwise.
    pub fn resolve(cfg: LangConfig) -> Self {
        match cfg {
            LangConfig::En => Self::En,
            LangConfig::Fr => Self::Fr,
            LangConfig::Auto => Self::detect_from_os(),
        }
    }

    fn detect_from_os() -> Self {
        if let Some(loc) = detect_locale()
            && loc.to_lowercase().starts_with("fr")
        {
            return Self::Fr;
        }
        Self::En
    }
}

#[cfg(not(target_os = "windows"))]
fn detect_locale() -> Option<String> {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var)
            && !v.is_empty()
        {
            return Some(v);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_locale() -> Option<String> {
    use std::os::raw::c_int;
    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(lpLocaleName: *mut u16, cchLocaleName: c_int) -> c_int;
    }
    let mut buf = [0u16; LOCALE_NAME_MAX_LENGTH];
    let written = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as c_int) };
    if written <= 1 {
        return None;
    }
    let slice = &buf[..(written as usize - 1)];
    String::from_utf16(slice).ok()
}

/// All UI strings exposed by the welcome screen and the skins dialog.
/// Strings that are reused elsewhere in the app should grow new methods
/// here over time; right now we only translate the new surfaces.
pub struct Strings {
    pub lang: Lang,
}

impl Strings {
    pub fn new(lang: Lang) -> Self {
        Self { lang }
    }

    pub fn welcome_title(&self) -> &'static str {
        match self.lang {
            Lang::En => "Welcome to OneAmp",
            Lang::Fr => "Bienvenue dans OneAmp",
        }
    }

    pub fn welcome_subtitle(&self) -> &'static str {
        match self.lang {
            Lang::En => {
                "Set up the essentials. You can change everything later from the Options menu."
            }
            Lang::Fr => "Réglez l'essentiel. Tout reste modifiable plus tard dans le menu Options.",
        }
    }

    pub fn language_section(&self) -> &'static str {
        match self.lang {
            Lang::En => "Language",
            Lang::Fr => "Langue",
        }
    }

    pub fn lang_auto(&self) -> &'static str {
        match self.lang {
            Lang::En => "Auto (system)",
            Lang::Fr => "Auto (système)",
        }
    }

    pub fn lang_english(&self) -> &'static str {
        match self.lang {
            Lang::En => "English",
            Lang::Fr => "Anglais",
        }
    }

    pub fn lang_french(&self) -> &'static str {
        match self.lang {
            Lang::En => "French",
            Lang::Fr => "Français",
        }
    }

    pub fn scale_section(&self) -> &'static str {
        match self.lang {
            Lang::En => "Display scale",
            Lang::Fr => "Échelle d'affichage",
        }
    }

    pub fn scale_auto(&self) -> &'static str {
        match self.lang {
            Lang::En => "Auto (DPI)",
            Lang::Fr => "Auto (DPI)",
        }
    }

    pub fn default_player_section(&self) -> &'static str {
        match self.lang {
            Lang::En => "Default audio player",
            Lang::Fr => "Lecteur audio par défaut",
        }
    }

    pub fn default_player_button(&self) -> &'static str {
        match self.lang {
            Lang::En => "Make OneAmp my default audio player",
            Lang::Fr => "Définir OneAmp comme lecteur audio par défaut",
        }
    }

    pub fn default_player_done(&self) -> &'static str {
        match self.lang {
            Lang::En => "OneAmp is now your default audio player.",
            Lang::Fr => "OneAmp est désormais votre lecteur audio par défaut.",
        }
    }

    pub fn default_player_failed(&self) -> &'static str {
        match self.lang {
            Lang::En => "Failed to set OneAmp as default. See log for details.",
            Lang::Fr => "Impossible de définir OneAmp par défaut. Voir les logs.",
        }
    }

    pub fn skins_section(&self) -> &'static str {
        match self.lang {
            Lang::En => "Skin",
            Lang::Fr => "Skin",
        }
    }

    pub fn skins_browse(&self) -> &'static str {
        match self.lang {
            Lang::En => "Browse…",
            Lang::Fr => "Parcourir…",
        }
    }

    pub fn skins_dialog_title(&self) -> &'static str {
        match self.lang {
            Lang::En => "Skins",
            Lang::Fr => "Skins",
        }
    }

    pub fn welcome_skip(&self) -> &'static str {
        match self.lang {
            Lang::En => "Skip",
            Lang::Fr => "Plus tard",
        }
    }

    pub fn welcome_done(&self) -> &'static str {
        match self.lang {
            Lang::En => "Get started",
            Lang::Fr => "Commencer",
        }
    }

    pub fn close(&self) -> &'static str {
        match self.lang {
            Lang::En => "Close",
            Lang::Fr => "Fermer",
        }
    }
}
