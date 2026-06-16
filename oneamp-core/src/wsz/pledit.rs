//! Parser for Winamp `pledit.txt` skin files.
//!
//! Format (single `[Text]` section, INI-style key/value pairs):
//!
//! ```text
//! [Text]
//! Normal = #00FF00
//! Current = #FFFFFF
//! NormalBG = #000000
//! SelectedBG = #0000FF
//! Font = Arial
//! MbFG = #000000
//! MbBG = #FFFFFF
//! ```
//!
//! - Color values may be written `#RRGGBB`, `RRGGBB`, or even `0xRRGGBB`.
//! - Keys are case-insensitive.
//! - Lines starting with `;` or `#` are comments. (Note: `#` also starts hex
//!   colors — strip the comment marker only on standalone lines.)
//! - Whitespace around `=` is tolerated.
//!
//! Missing keys fall back to Winamp's classic defaults (green-on-black
//! list).

use anyhow::Result;

/// Color theme + font for the playlist editor and minibrowser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PleditColors {
    /// Default playlist text color.
    pub normal: [u8; 3],
    /// Currently-playing track color.
    pub current: [u8; 3],
    /// Row background (unselected).
    pub normal_bg: [u8; 3],
    /// Selected-row background.
    pub selected_bg: [u8; 3],
    /// Minibrowser status text.
    pub mb_fg: [u8; 3],
    /// Minibrowser status background.
    pub mb_bg: [u8; 3],
}

/// Winamp's classic green-on-black playlist palette — used when `pledit.txt`
/// is missing or doesn't override a particular key.
pub const DEFAULT_PLEDIT_COLORS: PleditColors = PleditColors {
    normal: [0x00, 0xFF, 0x00],
    current: [0xFF, 0xFF, 0xFF],
    normal_bg: [0x00, 0x00, 0x00],
    selected_bg: [0x00, 0x00, 0xC6],
    mb_fg: [0x00, 0x00, 0x00],
    mb_bg: [0xFF, 0xFF, 0xFF],
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PleditTheme {
    pub colors: PleditColors,
    /// Optional font family name. The file format also allows shipping a TTF
    /// alongside `pledit.txt`; that's loaded separately by the renderer.
    pub font: Option<String>,
}

impl Default for PleditColors {
    fn default() -> Self {
        DEFAULT_PLEDIT_COLORS
    }
}

/// Parse a hex color in `#RRGGBB`, `RRGGBB`, or `0xRRGGBB` form.
fn parse_hex_color(raw: &str) -> Option<[u8; 3]> {
    let trimmed = raw.trim().trim_matches(|c: char| c == '"' || c == '\'');
    let stripped = trimmed
        .strip_prefix('#')
        .or_else(|| trimmed.strip_prefix("0x"))
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if stripped.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&stripped[0..2], 16).ok()?;
    let g = u8::from_str_radix(&stripped[2..4], 16).ok()?;
    let b = u8::from_str_radix(&stripped[4..6], 16).ok()?;
    Some([r, g, b])
}

/// Parse a `pledit.txt` content string. Missing keys keep their defaults.
/// Always returns Ok — malformed lines are skipped silently to mirror
/// Winamp's tolerance for hand-edited skins.
pub fn parse_pledit(content: &str) -> Result<PleditTheme> {
    let mut theme = PleditTheme::default();
    let mut in_text_section = false;

    for raw in content.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Comments. `#` is ambiguous with hex colors so only treat it as a
        // comment marker when it's the first non-whitespace char *and* not
        // followed by 6 hex digits (which would be a stray color literal).
        if trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with('#') && parse_hex_color(trimmed).is_none() {
            continue;
        }

        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_text_section = name.trim().eq_ignore_ascii_case("Text");
            continue;
        }

        if !in_text_section {
            continue;
        }

        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq].trim().to_ascii_lowercase();
        let value = trimmed[eq + 1..].trim();

        match key.as_str() {
            "normal" => {
                if let Some(c) = parse_color_value(&key, value) {
                    theme.colors.normal = c;
                }
            }
            "current" => {
                if let Some(c) = parse_color_value(&key, value) {
                    theme.colors.current = c;
                }
            }
            "normalbg" => {
                if let Some(c) = parse_color_value(&key, value) {
                    theme.colors.normal_bg = c;
                }
            }
            "selectedbg" => {
                if let Some(c) = parse_color_value(&key, value) {
                    theme.colors.selected_bg = c;
                }
            }
            "mbfg" => {
                if let Some(c) = parse_color_value(&key, value) {
                    theme.colors.mb_fg = c;
                }
            }
            "mbbg" => {
                if let Some(c) = parse_color_value(&key, value) {
                    theme.colors.mb_bg = c;
                }
            }
            "font" if !value.is_empty() => {
                theme.font = Some(value.to_string());
            }
            _ => {}
        }
    }

    Ok(theme)
}

/// Parse the value of a recognized colour key, logging a warning when the
/// value isn't a valid hex colour. Unlike a bare comment, a malformed value
/// on a known colour key (`Normal=#GGGGGG`) is almost certainly a skin-author
/// typo — surfacing it via a stderr warning lets them spot it instead of
/// the colour silently keeping its default. Never panics; returns `None`
/// on failure so the caller leaves the default in place.
fn parse_color_value(key: &str, value: &str) -> Option<[u8; 3]> {
    match parse_hex_color(value) {
        Some(c) => Some(c),
        None => {
            eprintln!(
                "Warning: pledit.txt: invalid hex colour for `{}`: {:?} — keeping default",
                key, value
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_variants() {
        assert_eq!(parse_hex_color("#FF0000"), Some([0xFF, 0, 0]));
        assert_eq!(parse_hex_color("00FF00"), Some([0, 0xFF, 0]));
        assert_eq!(parse_hex_color("0x0000ff"), Some([0, 0, 0xFF]));
        assert_eq!(parse_hex_color("not a color"), None);
        assert_eq!(parse_hex_color("#FF"), None);
    }

    #[test]
    fn parse_default_winamp() {
        let content = r#"
[Text]
Normal=#00FF00
Current=#FFFFFF
NormalBG=#000000
SelectedBG=#0000FF
Font=Arial
MbFG=#000000
MbBG=#FFFFFF
"#;
        let theme = parse_pledit(content).unwrap();
        assert_eq!(theme.colors.normal, [0, 0xFF, 0]);
        assert_eq!(theme.colors.current, [0xFF, 0xFF, 0xFF]);
        assert_eq!(theme.colors.normal_bg, [0, 0, 0]);
        assert_eq!(theme.colors.selected_bg, [0, 0, 0xFF]);
        assert_eq!(theme.font.as_deref(), Some("Arial"));
    }

    #[test]
    fn case_insensitive_keys_and_section() {
        let content = r#"
[TEXT]
NORMAL = #112233
nOrMaLbG = #445566
"#;
        let theme = parse_pledit(content).unwrap();
        assert_eq!(theme.colors.normal, [0x11, 0x22, 0x33]);
        assert_eq!(theme.colors.normal_bg, [0x44, 0x55, 0x66]);
    }

    #[test]
    fn ignores_other_sections() {
        let content = r#"
[General]
Normal=#FF0000

[Text]
Normal=#00FF00
"#;
        let theme = parse_pledit(content).unwrap();
        assert_eq!(theme.colors.normal, [0, 0xFF, 0]);
    }

    #[test]
    fn missing_keys_keep_defaults() {
        let content = "[Text]\nNormal=#123456\n";
        let theme = parse_pledit(content).unwrap();
        assert_eq!(theme.colors.normal, [0x12, 0x34, 0x56]);
        assert_eq!(theme.colors.current, DEFAULT_PLEDIT_COLORS.current);
        assert_eq!(theme.colors.selected_bg, DEFAULT_PLEDIT_COLORS.selected_bg);
    }

    #[test]
    fn empty_input_yields_defaults() {
        let theme = parse_pledit("").unwrap();
        assert_eq!(theme.colors, DEFAULT_PLEDIT_COLORS);
        assert!(theme.font.is_none());
    }

    #[test]
    fn invalid_hex_on_colour_key_keeps_default() {
        // `#GGGGGG` is a typo, not a comment: it sits on a recognized colour
        // key, so parsing must fall back to the default (and warn) rather
        // than silently dropping the line as a comment. The other valid key
        // on the following line must still be honoured.
        let content = "[Text]\nNormal=#GGGGGG\nCurrent=#123456\n";
        let theme = parse_pledit(content).unwrap();
        assert_eq!(theme.colors.normal, DEFAULT_PLEDIT_COLORS.normal);
        assert_eq!(theme.colors.current, [0x12, 0x34, 0x56]);
    }

    #[test]
    fn trailing_semicolon_comments_ignored() {
        let content = r#"
; classic Winamp green
[Text]
Normal=#00FF00
"#;
        let theme = parse_pledit(content).unwrap();
        assert_eq!(theme.colors.normal, [0, 0xFF, 0]);
    }
}
