//! Render text using the WSZ `text.bmp` glyph atlas, the way Winamp does.
//!
//! Classic layout: 31 columns × 2-3 rows of 5×6 px glyphs (155 × 12 or 18+).
//! - Row 0: A-Z (cols 0-25), then `"` (26), `@` (27).
//! - Row 1: 0-9 (cols 0-9), `…` (10), `.` (11), `:` (12), `(` (13), `)`
//!   (14), `-` (15), `'` (16), `!` (17), `_` (18), `+` (19), `\` (20),
//!   `/` (21), `[` (22), `]` (23), `^` (24), `&` (25), `%` (26), `,`
//!   (27), `=` (28), `$` (29), `#` (30).
//! - Row 2 (when present and populated per-cell): a-z lowercase glyphs.
//!
//! Two glyphs need special handling because they aren't in the spec:
//! - `*` (asterisk) — the standard atlas has no asterisk slot. We
//!   always paint a hardcoded 5×6 pattern. (Earlier code tried to sniff
//!   col 11 of row 1 thinking it was the asterisk slot — but col 11 is
//!   the period per spec, so the sniff always succeeded against the
//!   period glyph and the `*** ` separator rendered as `... `.)
//! - lowercase a-z — only fully populated in some Winamp 5+ skins. The
//!   classic base-2.91 atlas, despite being 155×74 (extended), only has
//!   a handful of accented glyphs at the *start* of row 2 and leaves
//!   the rest of the row transparent. We probe each lowercase cell
//!   individually at load time and fall back to uppercase per-char when
//!   the cell is empty.

use egui::{Color32, Pos2, Rect, Vec2};
use oneamp_core::wsz::bitmap::BitmapAtlas;
use oneamp_core::wsz::skin::SkinComponent;

use super::super::renderer::WszRenderer;

pub const GLYPH_W: u32 = 5;
pub const GLYPH_H: u32 = 6;

/// Hardcoded 5×6 asterisk pattern — `1` = lit pixel, `0` = transparent.
/// Used when the skin's text.bmp doesn't ship a real `*` glyph.
const ASTERISK_PATTERN: [[u8; 5]; 6] = [
    [0, 1, 0, 1, 0],
    [0, 0, 1, 0, 0],
    [1, 1, 1, 1, 1],
    [0, 0, 1, 0, 0],
    [0, 1, 0, 1, 0],
    [0, 0, 0, 0, 0],
];

/// Map a char to its (column, row) in the atlas. Lowercase a-z map to
/// row 2 — the caller checks per-cell at render time whether row 2
/// actually has a glyph at that column and falls back to uppercase if
/// not. `*` returns `None` so the renderer takes the synthetic-pattern
/// path.
fn glyph_coord(c: char) -> Option<(u32, u32)> {
    let coord = match c {
        'A'..='Z' => ((c as u32) - ('A' as u32), 0),
        'a'..='z' => ((c as u32) - ('a' as u32), 2),
        '"' => (26, 0),
        '@' => (27, 0),
        '0'..='9' => ((c as u32) - ('0' as u32), 1),
        '…' => (10, 1),
        '.' => (11, 1),
        ':' => (12, 1),
        '(' => (13, 1),
        ')' => (14, 1),
        '-' => (15, 1),
        '\'' => (16, 1),
        '!' => (17, 1),
        '_' => (18, 1),
        '+' => (19, 1),
        '\\' => (20, 1),
        '/' => (21, 1),
        '[' => (22, 1),
        ']' => (23, 1),
        '^' => (24, 1),
        '&' => (25, 1),
        '%' => (26, 1),
        ',' => (27, 1),
        '=' => (28, 1),
        '$' => (29, 1),
        '#' => (30, 1),
        _ => return None,
    };
    Some(coord)
}

/// Per-cell populated map for row 2 of the atlas (26 lowercase columns,
/// a..z). A cell is considered populated when at least 3 pixels in its
/// 5×6 area are opaque — enough to distinguish a real glyph from a stray
/// single-pixel artifact, low enough to catch thin letters like `i`/`l`.
fn lowercase_cells(atlas: &BitmapAtlas) -> [bool; 26] {
    let mut out = [false; 26];
    let gy = 2 * GLYPH_H;
    if gy + GLYPH_H > atlas.height {
        return out;
    }
    for (i, slot) in out.iter_mut().enumerate() {
        let gx = (i as u32) * GLYPH_W;
        if gx + GLYPH_W > atlas.width {
            continue;
        }
        let mut lit = 0u32;
        'cell: for y in gy..gy + GLYPH_H {
            for x in gx..gx + GLYPH_W {
                let idx = ((y * atlas.width + x) * 4) as usize;
                if let Some(slice) = atlas.data.get(idx..idx + 4)
                    && slice[3] > 0
                {
                    lit += 1;
                    if lit >= 3 {
                        *slot = true;
                        break 'cell;
                    }
                }
            }
        }
    }
    out
}

/// Decide whether row 2 of the atlas is a genuine `a-z` lowercase row,
/// as opposed to a few stray accented glyphs that happen to live at the
/// start of an extended atlas.
///
/// base-2.91 is the canonical false-positive case: 155×74 atlas with
/// glyphs that look like `Š Ø Å ? +` (or similar) in cells 0..5 of row 2
/// and transparency from col 5 onwards. Per-cell probing alone would
/// trust those 5 cells as `a-e` glyphs and render `+` everywhere there's
/// an `e` in the title. A real `a-z` row has content in *most* of the
/// 26 cells (full alphabet), so requiring ≥ 20 populated cells catches
/// genuine Winamp 5+ lowercase atlases and rejects sparse decoration.
fn row2_is_lowercase_alphabet(cells: &[bool; 26]) -> bool {
    cells.iter().filter(|&&b| b).count() >= 20
}

/// Pick the bright text color from the atlas by sampling the `'A'` glyph.
/// Falls back to Winamp green if the sample is transparent. Used to colour
/// the synthetic asterisk so it matches the rest of the line.
fn pick_text_color(atlas: &BitmapAtlas) -> Color32 {
    for y in 0..GLYPH_H {
        for x in 0..GLYPH_W {
            let idx = ((y * atlas.width + x) * 4) as usize;
            if let Some(slice) = atlas.data.get(idx..idx + 4)
                && slice[3] > 0
            {
                return Color32::from_rgba_unmultiplied(slice[0], slice[1], slice[2], slice[3]);
            }
        }
    }
    Color32::from_rgb(0, 255, 0)
}

/// Paint a synthetic 5×6 asterisk at `top_left` using `color`. Pixel size is
/// `scale` (matches the renderer's logical→physical conversion).
fn paint_asterisk(ui: &mut egui::Ui, top_left: Pos2, scale: f32, color: Color32) {
    let painter = ui.painter();
    for (row, cells) in ASTERISK_PATTERN.iter().enumerate() {
        for (col, &lit) in cells.iter().enumerate() {
            if lit == 0 {
                continue;
            }
            let px = top_left.x + col as f32 * scale;
            let py = top_left.y + row as f32 * scale;
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(px, py), Vec2::splat(scale)),
                0.0,
                color,
            );
        }
    }
}

/// Render `text` left-aligned starting at `screen_pos`, using glyphs from the
/// WSZ Text atlas. Returns `None` (and paints nothing) if the skin's text
/// atlas is too small to contain the standard layout — callers should pick a
/// fallback path in that case.
pub fn render_text(
    renderer: &mut WszRenderer,
    ui: &mut egui::Ui,
    text: &str,
    screen_pos: Pos2,
) -> Option<f32> {
    let scale = renderer.get_scale();
    let atlas = renderer
        .get_skin()
        .get_bitmap(&SkinComponent::Text)
        .cloned()?;
    if atlas.width < 31 * GLYPH_W || atlas.height < 2 * GLYPH_H {
        return None;
    }
    let lowercase = lowercase_cells(&atlas);
    let use_row2 = row2_is_lowercase_alphabet(&lowercase);
    let asterisk_color = pick_text_color(&atlas);

    let advance = (GLYPH_W as f32) * scale;
    let mut x = screen_pos.x;
    for (i, c) in text.chars().enumerate() {
        if c == '*' {
            // No standard atlas slot for `*` — always synthesise. The
            // `*** ` separator only appears as part of the title scroller
            // loop, so a hardcoded 5×6 cross is enough.
            paint_asterisk(ui, Pos2::new(x, screen_pos.y), scale, asterisk_color);
            x += advance;
            continue;
        }

        // Lowercase routing: when row 2 is a real a-z alphabet, use it
        // per-cell (any column that's empty falls back to uppercase).
        // Otherwise force-uppercase all lowercase chars — base-2.91 has
        // only a few non-Latin glyphs in row 2 cells 0..5, and trusting
        // those as `a-e` would render `+` for every `e` in titles.
        let coord = match glyph_coord(c) {
            Some((col, 2)) => {
                if use_row2 && lowercase[col as usize] {
                    Some((col, 2))
                } else {
                    glyph_coord(c.to_ascii_uppercase())
                }
            }
            other => other,
        };
        if let Some((col, row)) = coord {
            let gx = col * GLYPH_W;
            let gy = row * GLYPH_H;
            if let Some(region) = atlas.extract_region(gx, gy, GLYPH_W, GLYPH_H) {
                renderer.render_region(
                    ui,
                    &region,
                    Pos2::new(x, screen_pos.y),
                    &format!("font_{}_{}", c as u32, i),
                );
            }
        }
        x += advance;
    }
    Some(x - screen_pos.x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coord_lookup_basic() {
        assert_eq!(glyph_coord('A'), Some((0, 0)));
        assert_eq!(glyph_coord('Z'), Some((25, 0)));
        assert_eq!(glyph_coord('a'), Some((0, 2)));
        assert_eq!(glyph_coord('z'), Some((25, 2)));
        assert_eq!(glyph_coord('0'), Some((0, 1)));
        assert_eq!(glyph_coord('9'), Some((9, 1)));
        // Spec: col 10 of row 1 is the ellipsis `…`, col 11 is the period.
        assert_eq!(glyph_coord('…'), Some((10, 1)));
        assert_eq!(glyph_coord('.'), Some((11, 1)));
        assert_eq!(glyph_coord(':'), Some((12, 1)));
        assert_eq!(glyph_coord('*'), None); // hardcoded path
        assert_eq!(glyph_coord(' '), None);
        assert_eq!(glyph_coord('é'), None);
    }
}
