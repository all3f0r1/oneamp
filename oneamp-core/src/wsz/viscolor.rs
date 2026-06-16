//! Parser for Winamp `viscolor.txt` — the 24 RGB colors used by the
//! visualization area.
//!
//! Standard layout (one entry per line, `R,G,B [// comment]`):
//!
//! ```text
//! 0       background
//! 1       grid / dot color
//! 2..=17  spectrum analyzer (16 colors, top → bottom)
//! 18..=22 oscilloscope (5 colors, troughs → crests)
//! 23      analyzer peak / "last value" marker
//! ```
//!
//! Skins that ship a malformed or short file fall back to a hard-coded
//! Winamp-default palette so the visualizer always has 24 colors.

/// 24 RGB triples. Indices match the `viscolor.txt` line numbering above.
pub type VisColors = [[u8; 3]; 24];

/// The palette baked into stock Winamp. Used as a fallback when a skin
/// omits `viscolor.txt` or ships an unparseable one.
pub const DEFAULT_VIS_COLORS: VisColors = [
    [0, 0, 0],       // 0  background
    [24, 33, 41],    // 1  dot grid
    [239, 49, 16],   // 2  spec top
    [206, 41, 16],   // 3
    [214, 90, 0],    // 4
    [214, 102, 0],   // 5
    [214, 115, 0],   // 6
    [198, 123, 8],   // 7
    [222, 165, 24],  // 8
    [214, 181, 33],  // 9
    [189, 222, 41],  // 10
    [148, 222, 33],  // 11
    [41, 206, 16],   // 12
    [50, 190, 16],   // 13
    [57, 181, 16],   // 14
    [49, 156, 8],    // 15
    [41, 148, 0],    // 16
    [24, 132, 8],    // 17 spec bottom
    [255, 255, 255], // 18 osc 1
    [214, 214, 222], // 19 osc 2
    [181, 189, 189], // 20 osc 3
    [160, 170, 175], // 21 osc 4
    [148, 156, 165], // 22 osc 5
    [150, 150, 150], // 23 last peak
];

/// Strip an end-of-line comment (`//` or `;`) and trim whitespace.
fn strip_comment(line: &str) -> &str {
    let mut cut = line.len();
    if let Some(idx) = line.find("//") {
        cut = cut.min(idx);
    }
    if let Some(idx) = line.find(';') {
        cut = cut.min(idx);
    }
    line[..cut].trim()
}

/// Parse a `viscolor.txt` content string into 24 RGB triples. Lines that
/// fail to parse are skipped; if fewer than 24 valid entries are present,
/// the remaining slots are filled from `DEFAULT_VIS_COLORS`.
pub fn parse_viscolor(content: &str) -> VisColors {
    let mut out = DEFAULT_VIS_COLORS;
    let mut i = 0usize;

    for line in content.lines() {
        if i >= out.len() {
            break;
        }
        let trimmed = strip_comment(line);
        if trimmed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = trimmed.split(',').collect();
        if parts.len() < 3 {
            continue;
        }
        let r = parts[0].trim().parse::<u8>();
        let g = parts[1].trim().parse::<u8>();
        let b = parts[2].trim().parse::<u8>();
        if let (Ok(r), Ok(g), Ok(b)) = (r, g, b) {
            out[i] = [r, g, b];
            i += 1;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_has_24_entries() {
        assert_eq!(DEFAULT_VIS_COLORS.len(), 24);
    }

    #[test]
    fn parse_winamp_default_file() {
        // The `viscolor.txt` shipped in Winamp's base skin — `// comment`
        // style on each line. The first 18 entries are the analyzer band
        // gradient; 18..=22 are the oscilloscope levels; 23 is last peak.
        let content = r#"
0,0,0,         // color 0 = black
24,33,41,      // color 1 = grey for dots
239,49,16,     // color 2 = top of spec
206,41,16,     // 3
214,90,0,      // 4
214,102,0,     // 5
214,115,0,     // 6
198,123,8,     // 7
222,165,24,    // 8
214,181,33,    // 9
189,222,41,    // 10
148,222,33,    // 11
41,206,16,     // 12
50,190,16,     // 13
57,181,16,     // 14
49,156,8,      // 15
41,148,0,      // 16
24,132,8,      // 17 = bottom of spec
255,255,255,   // 18 = osc 1
214,214,222,   // 19
181,189,189,   // 20
160,170,175,   // 21
148,156,165,   // 22
150,150,150,   // 23 = analyzer peak
"#;
        let colors = parse_viscolor(content);
        assert_eq!(colors[0], [0, 0, 0]);
        assert_eq!(colors[2], [239, 49, 16]);
        assert_eq!(colors[17], [24, 132, 8]);
        assert_eq!(colors[18], [255, 255, 255]);
        assert_eq!(colors[23], [150, 150, 150]);
    }

    #[test]
    fn parse_short_file_pads_with_defaults() {
        // Only 3 valid entries — slots 3..=23 should keep the default palette.
        let content = "1,2,3\n4,5,6\n7,8,9\n";
        let colors = parse_viscolor(content);
        assert_eq!(colors[0], [1, 2, 3]);
        assert_eq!(colors[1], [4, 5, 6]);
        assert_eq!(colors[2], [7, 8, 9]);
        assert_eq!(colors[3], DEFAULT_VIS_COLORS[3]);
        assert_eq!(colors[23], DEFAULT_VIS_COLORS[23]);
    }

    #[test]
    fn parse_skips_bad_lines_without_breaking_alignment() {
        // Bad line in the middle: the next valid line should land in the
        // skipped slot, NOT at slot+1. Otherwise indices shift and every
        // visualizer color is wrong.
        let content = "1,2,3\nnot,a,number\n4,5,6\n";
        let colors = parse_viscolor(content);
        assert_eq!(colors[0], [1, 2, 3]);
        assert_eq!(colors[1], [4, 5, 6]);
    }

    #[test]
    fn parse_empty_input_returns_default() {
        let colors = parse_viscolor("");
        assert_eq!(colors, DEFAULT_VIS_COLORS);
    }
}
