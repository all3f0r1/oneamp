//! Windows `.cur` cursor parser for WSZ skins.
//!
//! Winamp ships up to 27 named cursors per skin (NORMAL, TITLEBAR, POSBAR,
//! ...) — see `WSZ_FORMAT.md` §Custom cursors. The format is the standard
//! Windows ICO/CUR container: a 6-byte header, one ICONDIRENTRY per image,
//! and the image data as a BMP DIB (no file header) or PNG.
//!
//! We extract just enough to render: a hotspot + an RGBA buffer. We decode
//! indexed-color (1/4/8 bpp) cursors via their embedded palette as well as
//! 24bpp and 32bpp; PNG-encoded frames go through the `image` crate.
//! Animated cursors (`.ani`, RIFF-wrapped multiple `.cur` frames) are
//! handled upstream in the loader by extracting their first frame and
//! feeding it to [`parse_cur`] — we do not animate.

use anyhow::{Context, Result, bail};

/// One decoded cursor: an RGBA buffer plus its hotspot offset (in cursor-
/// local pixels). The renderer paints the buffer at `pointer_pos -
/// (hotspot_x, hotspot_y)` so the click point is exactly under the
/// pointer.
#[derive(Debug, Clone)]
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    /// Tightly packed RGBA, top-down row-major. Length = width × height × 4.
    pub rgba: Vec<u8>,
}

/// Parse a `.cur` file's bytes into a `CursorImage`. The first image in the
/// archive's ICONDIR is decoded; multi-image cursors (different sizes for
/// different DPIs) are rare in classic Winamp skins.
///
/// Errors when:
/// - the header doesn't read as a CUR (type field ≠ 2),
/// - the embedded BMP uses an unsupported bit depth (only 24bpp + AND mask
///   and 32bpp are accepted),
/// - the image data overruns the slice.
pub fn parse_cur(data: &[u8]) -> Result<CursorImage> {
    if data.len() < 22 {
        bail!("CUR data too short ({} bytes, need ≥22)", data.len());
    }

    // ICONDIR header
    let reserved = u16::from_le_bytes([data[0], data[1]]);
    let kind = u16::from_le_bytes([data[2], data[3]]);
    let count = u16::from_le_bytes([data[4], data[5]]);
    if reserved != 0 {
        bail!("CUR: reserved field is {} (expected 0)", reserved);
    }
    if kind != 2 {
        bail!("CUR: type is {} (expected 2 for CUR)", kind);
    }
    if count == 0 {
        bail!("CUR: no images in archive");
    }

    // First ICONDIRENTRY at offset 6, 16 bytes long.
    let entry = &data[6..22];
    let dir_w = entry[0]; // 0 means 256
    let dir_h = entry[1]; // 0 means 256
    let hotspot_x = u16::from_le_bytes([entry[4], entry[5]]) as u32;
    let hotspot_y = u16::from_le_bytes([entry[6], entry[7]]) as u32;
    let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
    let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;

    let end = offset
        .checked_add(size)
        .context("CUR: image offset + size overflow")?;
    if end > data.len() {
        bail!(
            "CUR: image extends past file end (offset {} + size {} > {})",
            offset,
            size,
            data.len()
        );
    }
    let img = &data[offset..end];

    // PNG-encoded cursors start with the standard 8-byte PNG signature.
    // Classic Winamp skins don't ship them, but the format spec allows
    // them; decode through the `image` crate when we see one.
    if img.len() >= 8 && &img[..8] == b"\x89PNG\r\n\x1a\n" {
        let dec = image::load_from_memory_with_format(img, image::ImageFormat::Png)
            .context("CUR: PNG decode failed")?;
        let rgba = dec.to_rgba8();
        let (width, height) = rgba.dimensions();
        return Ok(CursorImage {
            width,
            height,
            hotspot_x,
            hotspot_y,
            rgba: rgba.into_raw(),
        });
    }

    // Otherwise: BMP DIB starting with BITMAPINFOHEADER (40 bytes).
    if img.len() < 40 {
        bail!("CUR: BMP DIB too short ({} bytes)", img.len());
    }
    let dib_size = u32::from_le_bytes([img[0], img[1], img[2], img[3]]);
    if dib_size < 40 {
        bail!("CUR: unsupported DIB header size {} (need ≥40)", dib_size);
    }
    let bmp_w = i32::from_le_bytes([img[4], img[5], img[6], img[7]]).unsigned_abs();
    // BMP stores height as 2× actual (XOR mask + AND mask stacked).
    let bmp_h_raw = i32::from_le_bytes([img[8], img[9], img[10], img[11]]).unsigned_abs();
    let bit_count = u16::from_le_bytes([img[14], img[15]]);
    let compression = u32::from_le_bytes([img[16], img[17], img[18], img[19]]);
    if compression != 0 {
        bail!(
            "CUR: BI_RGB only (compression {} not supported)",
            compression
        );
    }
    if !matches!(bit_count, 1 | 4 | 8 | 24 | 32) {
        bail!("CUR: bit depth {} not supported", bit_count);
    }

    let actual_h = bmp_h_raw / 2;
    if actual_h == 0 || bmp_w == 0 {
        bail!("CUR: zero-sized image");
    }
    // Cross-check against the dirent dims when populated. The dirent uses
    // 0 to mean 256 — leave that case alone.
    let dirent_w = if dir_w == 0 { 256 } else { dir_w as u32 };
    let dirent_h = if dir_h == 0 { 256 } else { dir_h as u32 };
    if (dirent_w != bmp_w || dirent_h != actual_h) && dir_w != 0 && dir_h != 0 {
        // Tolerate the mismatch — the BMP dims win — but flag it for
        // debugging weird skins.
        eprintln!(
            "CUR: dirent says {}×{} but BMP says {}×{}",
            dirent_w, dirent_h, bmp_w, actual_h
        );
    }

    // For 1/4/8 bpp the BMP carries a color palette between the header
    // and the pixel data: `biClrUsed` entries (or the bpp-implied default
    // when 0), 4 bytes each (BGR + reserved).
    let clr_used = u32::from_le_bytes([img[32], img[33], img[34], img[35]]) as usize;
    let palette_entries = if matches!(bit_count, 1 | 4 | 8) {
        if clr_used > 0 {
            clr_used
        } else {
            1usize << bit_count
        }
    } else {
        0
    };
    let palette_size = palette_entries * 4;
    let palette_off = dib_size as usize;
    let palette_end = palette_off + palette_size;
    if palette_end > img.len() {
        bail!("CUR: palette overruns image data");
    }
    let palette = &img[palette_off..palette_end];

    // Pixel data follows the palette (24/32bpp have an empty palette).
    // Stride is padded to a 4-byte boundary regardless of bpp.
    let pixel_data_offset = palette_end;
    let xor_stride = ((bmp_w as usize * bit_count as usize).div_ceil(8) + 3) & !3;
    let xor_size = xor_stride * actual_h as usize;
    let xor_end = pixel_data_offset + xor_size;
    if xor_end > img.len() {
        bail!("CUR: XOR mask overruns image data");
    }
    let xor = &img[pixel_data_offset..xor_end];

    // AND mask: 1bpp, used for transparency at every depth that lacks a
    // built-in alpha channel. Optional — some encoders omit it.
    let and_stride = (bmp_w as usize).div_ceil(32) * 4;
    let and_size = and_stride * actual_h as usize;
    let and = if pixel_data_offset + xor_size + and_size <= img.len() {
        Some(&img[xor_end..xor_end + and_size])
    } else {
        None
    };

    let mut rgba = vec![0u8; (bmp_w * actual_h * 4) as usize];
    for y in 0..actual_h {
        // Source row counted from the bottom (BMP convention); destination
        // row counted from the top (renderer convention).
        let src_row_idx = (actual_h - 1 - y) as usize;
        let dst_row_idx = y as usize;
        let row_off = src_row_idx * xor_stride;
        for x in 0..bmp_w {
            let dst_off = (dst_row_idx * bmp_w as usize + x as usize) * 4;
            let (r, g, b, a_pixel) = match bit_count {
                32 => {
                    let off = row_off + x as usize * 4;
                    (xor[off + 2], xor[off + 1], xor[off], xor[off + 3])
                }
                24 => {
                    let off = row_off + x as usize * 3;
                    (xor[off + 2], xor[off + 1], xor[off], 255)
                }
                8 => {
                    let idx = xor[row_off + x as usize] as usize;
                    palette_lookup(palette, idx)
                }
                4 => {
                    let byte = xor[row_off + x as usize / 2];
                    let nibble = if x & 1 == 0 {
                        (byte >> 4) & 0x0F
                    } else {
                        byte & 0x0F
                    };
                    palette_lookup(palette, nibble as usize)
                }
                1 => {
                    let byte = xor[row_off + x as usize / 8];
                    let bit = 7 - (x as usize % 8);
                    let idx = (byte >> bit) & 1;
                    palette_lookup(palette, idx as usize)
                }
                _ => unreachable!(),
            };
            // 32bpp ignores the AND mask in favour of its embedded alpha;
            // every other depth pulls transparency from the AND mask.
            let a = if bit_count == 32 {
                a_pixel
            } else {
                match and {
                    Some(mask) => {
                        let bit_idx = src_row_idx * and_stride * 8 + x as usize;
                        let byte = mask[bit_idx / 8];
                        let bit = 7 - (bit_idx % 8);
                        if (byte >> bit) & 1 == 1 { 0 } else { 255 }
                    }
                    None => 255,
                }
            };
            rgba[dst_off] = r;
            rgba[dst_off + 1] = g;
            rgba[dst_off + 2] = b;
            rgba[dst_off + 3] = a;
        }
    }

    // 32bpp cursors sometimes ship with all-zero alpha (the AND mask was
    // meant to drive transparency); detect and fall back to "fully
    // opaque" so the cursor isn't silently invisible.
    if bit_count == 32 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
        for chunk in rgba.chunks_exact_mut(4) {
            chunk[3] = 255;
        }
    }

    Ok(CursorImage {
        width: bmp_w,
        height: actual_h,
        hotspot_x: hotspot_x.min(bmp_w.saturating_sub(1)),
        hotspot_y: hotspot_y.min(actual_h.saturating_sub(1)),
        rgba,
    })
}

/// Decode the FIRST frame of an animated cursor (`.ani`) into a
/// `CursorImage`. We don't animate — showing the first frame is enough for
/// the cursor to render something instead of nothing.
///
/// `.ani` is a RIFF container with the `ACON` form type. Its frames are
/// embedded `.cur`/`.ico` blobs stored as `icon` chunks, usually inside a
/// `LIST` chunk tagged `fram`. We walk the RIFF chunk list, find the first
/// `icon` chunk wherever it lives, and hand its bytes to [`parse_cur`] (CUR
/// and ICO share the same ICONDIR layout; the type field differs but the
/// pixel decode is identical).
///
/// Errors when the bytes aren't a well-formed `RIFF`/`ACON` container or no
/// `icon` chunk is found. Callers should treat an error as "skip this
/// cursor" and keep loading — never panic on malformed skin data.
pub fn parse_ani_first_frame(data: &[u8]) -> Result<CursorImage> {
    // RIFF header: "RIFF" <u32 size> "ACON"
    if data.len() < 12 {
        bail!("ANI: data too short ({} bytes, need ≥12)", data.len());
    }
    if &data[0..4] != b"RIFF" {
        bail!("ANI: missing RIFF magic");
    }
    if &data[8..12] != b"ACON" {
        bail!("ANI: form type is not ACON");
    }

    let icon = find_first_icon_chunk(&data[12..])
        .context("ANI: no 'icon' frame chunk found in container")?;
    parse_cur(icon).context("ANI: first frame failed to decode as CUR/ICO")
}

/// Recursively scan a sequence of RIFF chunks for the first `icon` chunk.
/// `LIST` chunks (which is where `.ani` keeps its `fram` frame list) are
/// descended into. Returns a slice of the icon chunk's payload, or `None`.
///
/// Defensive: bounds-checks every chunk header and size so a truncated or
/// hostile container can't read out of bounds — it just stops scanning.
fn find_first_icon_chunk(mut buf: &[u8]) -> Option<&[u8]> {
    while buf.len() >= 8 {
        let id = &buf[0..4];
        let size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        let body_start: usize = 8;
        let body_end = body_start.checked_add(size)?;
        if body_end > buf.len() {
            return None;
        }
        let body = &buf[body_start..body_end];

        if id == b"icon" {
            return Some(body);
        }
        if id == b"LIST" {
            // A LIST body starts with a 4-byte list type (e.g. "fram"),
            // followed by nested chunks.
            if body.len() >= 4
                && let Some(found) = find_first_icon_chunk(&body[4..])
            {
                return Some(found);
            }
        }

        // Chunks are word-aligned: an odd size is followed by a pad byte.
        let advance = body_end + (size & 1);
        if advance <= buf.len() {
            buf = &buf[advance..];
        } else {
            break;
        }
    }
    None
}

/// Look up a palette entry at `idx`. BMP palettes are stored BGR + a
/// reserved byte (we ignore the reserved byte here). Out-of-range indices
/// fall back to opaque black so a malformed cursor doesn't read OOB.
fn palette_lookup(palette: &[u8], idx: usize) -> (u8, u8, u8, u8) {
    let off = idx * 4;
    if off + 3 < palette.len() {
        (palette[off + 2], palette[off + 1], palette[off], 255)
    } else {
        (0, 0, 0, 255)
    }
}

/// Map a Winamp cursor filename (case-insensitive, with or without
/// extension) to its `CursorKind`. Returns `None` for filenames Winamp
/// doesn't recognise — those are silently ignored.
pub fn kind_from_filename(name: &str) -> Option<CursorKind> {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    match stem.to_ascii_uppercase().as_str() {
        "NORMAL" => Some(CursorKind::Normal),
        "MAINMENU" => Some(CursorKind::MainMenu),
        "MIN" => Some(CursorKind::Min),
        "CLOSE" => Some(CursorKind::Close),
        "WINBUT" => Some(CursorKind::WinBut),
        "TITLEBAR" => Some(CursorKind::TitleBar),
        "SONGNAME" => Some(CursorKind::SongName),
        "POSBAR" => Some(CursorKind::PosBar),
        "VOLBAR" => Some(CursorKind::VolBar),
        "VOLBAL" => Some(CursorKind::VolBal),
        "EQNORMAL" => Some(CursorKind::EqNormal),
        "EQTITLE" => Some(CursorKind::EqTitle),
        "EQSLID" => Some(CursorKind::EqSlid),
        "EQCLOSE" => Some(CursorKind::EqClose),
        "PNORMAL" => Some(CursorKind::PNormal),
        "PTBAR" => Some(CursorKind::PTBar),
        "PCLOSE" => Some(CursorKind::PClose),
        "PWINBUT" => Some(CursorKind::PWinBut),
        "PVSCROLL" => Some(CursorKind::PVScroll),
        "PSIZE" => Some(CursorKind::PSize),
        "WSNORMAL" => Some(CursorKind::WsNormal),
        "WSCLOSE" => Some(CursorKind::WsClose),
        "WSMIN" => Some(CursorKind::WsMin),
        "WSPOSBAR" => Some(CursorKind::WsPosBar),
        "WSWINBUT" => Some(CursorKind::WsWinBut),
        "PWSNORM" => Some(CursorKind::PWsNorm),
        "PWSSIZE" => Some(CursorKind::PWsSize),
        _ => None,
    }
}

/// One of Winamp's 27 named cursor slots. Each maps to a specific UI
/// region — see `WSZ_FORMAT.md` §Custom cursors for the canonical list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorKind {
    /// Default arrow over the main window content.
    Normal,
    /// Hover on the menu (Options) button.
    MainMenu,
    /// Hover on the main window minimize button.
    Min,
    /// Hover on the main window close button.
    Close,
    /// Hover on the windowshade toggle.
    WinBut,
    /// Hover on the main window title bar (drag area).
    TitleBar,
    /// Hover on the song-name scroll area.
    SongName,
    /// Hover on the position slider.
    PosBar,
    /// Hover on the volume slider.
    VolBar,
    /// Hover on the balance slider.
    VolBal,
    /// Equalizer window default cursor.
    EqNormal,
    /// Hover on the equalizer title bar.
    EqTitle,
    /// Hover on an equalizer vertical slider.
    EqSlid,
    /// Hover on the equalizer close button.
    EqClose,
    /// Playlist window default cursor.
    PNormal,
    /// Hover on the playlist title bar.
    PTBar,
    /// Hover on the playlist close button.
    PClose,
    /// Hover on the playlist windowshade toggle.
    PWinBut,
    /// Hover on the playlist vertical scrollbar.
    PVScroll,
    /// Hover on the playlist resize handle.
    PSize,
    /// Windowshade-mode default cursor (main window).
    WsNormal,
    /// Hover on the shade close button.
    WsClose,
    /// Hover on the shade minimize button.
    WsMin,
    /// Hover on the shade position slider.
    WsPosBar,
    /// Hover on the shade exit-windowshade button.
    WsWinBut,
    /// Hover on the playlist shade-mode default area.
    PWsNorm,
    /// Hover on the playlist shade-mode resize handle.
    PWsSize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_from_filename_handles_extension_and_case() {
        assert_eq!(kind_from_filename("NORMAL.CUR"), Some(CursorKind::Normal));
        assert_eq!(kind_from_filename("normal.cur"), Some(CursorKind::Normal));
        assert_eq!(kind_from_filename("PSIZE"), Some(CursorKind::PSize));
        assert_eq!(kind_from_filename("unknown.cur"), None);
    }

    #[test]
    fn parse_cur_rejects_short_input() {
        assert!(parse_cur(&[0; 4]).is_err());
    }

    #[test]
    fn parse_cur_accepts_minimal_4bpp_indexed() {
        // Hand-roll a 2×1 indexed cursor: pixel[0] uses palette index 1
        // (red), pixel[1] uses index 2 (green).
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
        buf.extend_from_slice(&2u16.to_le_bytes()); // type = CUR
        buf.extend_from_slice(&1u16.to_le_bytes()); // count
        buf.push(2); // width
        buf.push(1); // height
        buf.push(16); // colors
        buf.push(0); // reserved
        buf.extend_from_slice(&0u16.to_le_bytes()); // hotspot_x
        buf.extend_from_slice(&0u16.to_le_bytes()); // hotspot_y
        let size_off = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // size placeholder
        buf.extend_from_slice(&22u32.to_le_bytes()); // offset
        let img_start = buf.len();
        // BITMAPINFOHEADER (4bpp, 16 palette entries)
        buf.extend_from_slice(&40u32.to_le_bytes());
        buf.extend_from_slice(&2i32.to_le_bytes()); // width
        buf.extend_from_slice(&2i32.to_le_bytes()); // height (×2)
        buf.extend_from_slice(&1u16.to_le_bytes()); // planes
        buf.extend_from_slice(&4u16.to_le_bytes()); // bit count = 4
        buf.extend_from_slice(&0u32.to_le_bytes()); // compression
        buf.extend_from_slice(&0u32.to_le_bytes()); // image size
        buf.extend_from_slice(&0i32.to_le_bytes()); // x ppm
        buf.extend_from_slice(&0i32.to_le_bytes()); // y ppm
        buf.extend_from_slice(&16u32.to_le_bytes()); // colors used
        buf.extend_from_slice(&0u32.to_le_bytes()); // important colors
        // 16 palette entries, BGR + reserved. Index 0 = black, 1 = red,
        // 2 = green, rest = black.
        for i in 0..16 {
            match i {
                1 => buf.extend_from_slice(&[0x00, 0x00, 0xFF, 0x00]), // red
                2 => buf.extend_from_slice(&[0x00, 0xFF, 0x00, 0x00]), // green
                _ => buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]),
            }
        }
        // XOR pixel row: 1 byte (high nibble = px0 = 1, low nibble = px1
        // = 2), padded to 4 bytes.
        buf.extend_from_slice(&[0x12, 0x00, 0x00, 0x00]);
        // AND mask: 1 row of 4 bytes (32-bit aligned).
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        let total = buf.len() - img_start;
        buf[size_off..size_off + 4].copy_from_slice(&(total as u32).to_le_bytes());

        let img = parse_cur(&buf).expect("4bpp indexed cursor should parse");
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 1);
        // Pixel 0 → red, pixel 1 → green.
        assert_eq!(&img.rgba[..4], &[0xFF, 0x00, 0x00, 0xFF]);
        assert_eq!(&img.rgba[4..8], &[0x00, 0xFF, 0x00, 0xFF]);
    }

    #[test]
    fn parse_cur_accepts_minimal_24bpp() {
        // Hand-roll a 1×1 black 24bpp cursor with hotspot (0,0).
        let mut buf = Vec::new();
        // ICONDIR
        buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
        buf.extend_from_slice(&2u16.to_le_bytes()); // type = CUR
        buf.extend_from_slice(&1u16.to_le_bytes()); // count
        // ICONDIRENTRY
        buf.push(1); // width
        buf.push(1); // height
        buf.push(0); // colors
        buf.push(0); // reserved
        buf.extend_from_slice(&0u16.to_le_bytes()); // hotspot_x
        buf.extend_from_slice(&0u16.to_le_bytes()); // hotspot_y
        // size + offset filled in below
        let size_off = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&22u32.to_le_bytes()); // offset = 22
        let img_start = buf.len();
        // BITMAPINFOHEADER
        buf.extend_from_slice(&40u32.to_le_bytes()); // size
        buf.extend_from_slice(&1i32.to_le_bytes()); // width
        buf.extend_from_slice(&2i32.to_le_bytes()); // height (× 2)
        buf.extend_from_slice(&1u16.to_le_bytes()); // planes
        buf.extend_from_slice(&24u16.to_le_bytes()); // bit count
        buf.extend_from_slice(&0u32.to_le_bytes()); // compression
        buf.extend_from_slice(&0u32.to_le_bytes()); // image size
        buf.extend_from_slice(&0i32.to_le_bytes()); // x ppm
        buf.extend_from_slice(&0i32.to_le_bytes()); // y ppm
        buf.extend_from_slice(&0u32.to_le_bytes()); // colors used
        buf.extend_from_slice(&0u32.to_le_bytes()); // important colors
        // XOR row: 1 px BGR + 1 byte padding (stride aligned to 4)
        buf.extend_from_slice(&[0x10, 0x20, 0x30, 0x00]);
        // AND row: 1 byte (stride aligned to 4 bytes = 4 bytes total)
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        let total_img = buf.len() - img_start;
        buf[size_off..size_off + 4].copy_from_slice(&(total_img as u32).to_le_bytes());

        let img = parse_cur(&buf).expect("minimal 1×1 24bpp cursor should parse");
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 1);
        // BMP order BGR -> our order RGB.
        assert_eq!(&img.rgba[..3], &[0x30, 0x20, 0x10]);
        assert_eq!(img.rgba[3], 255);
    }

    /// Build a minimal 1×1 24bpp `.cur` blob (the same bytes the 24bpp test
    /// hand-rolls) for reuse in the `.ani` test.
    fn minimal_cur_24bpp() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
        buf.extend_from_slice(&2u16.to_le_bytes()); // type = CUR
        buf.extend_from_slice(&1u16.to_le_bytes()); // count
        buf.push(1); // width
        buf.push(1); // height
        buf.push(0); // colors
        buf.push(0); // reserved
        buf.extend_from_slice(&0u16.to_le_bytes()); // hotspot_x
        buf.extend_from_slice(&0u16.to_le_bytes()); // hotspot_y
        let size_off = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // size placeholder
        buf.extend_from_slice(&22u32.to_le_bytes()); // offset = 22
        let img_start = buf.len();
        buf.extend_from_slice(&40u32.to_le_bytes()); // DIB size
        buf.extend_from_slice(&1i32.to_le_bytes()); // width
        buf.extend_from_slice(&2i32.to_le_bytes()); // height (× 2)
        buf.extend_from_slice(&1u16.to_le_bytes()); // planes
        buf.extend_from_slice(&24u16.to_le_bytes()); // bit count
        buf.extend_from_slice(&0u32.to_le_bytes()); // compression
        buf.extend_from_slice(&0u32.to_le_bytes()); // image size
        buf.extend_from_slice(&0i32.to_le_bytes()); // x ppm
        buf.extend_from_slice(&0i32.to_le_bytes()); // y ppm
        buf.extend_from_slice(&0u32.to_le_bytes()); // colors used
        buf.extend_from_slice(&0u32.to_le_bytes()); // important colors
        buf.extend_from_slice(&[0x10, 0x20, 0x30, 0x00]); // XOR row
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // AND row
        let total_img = buf.len() - img_start;
        buf[size_off..size_off + 4].copy_from_slice(&(total_img as u32).to_le_bytes());
        buf
    }

    #[test]
    fn parse_ani_extracts_first_frame() {
        // Wrap one CUR frame inside RIFF/ACON: a LIST 'fram' containing a
        // single 'icon' chunk.
        let frame = minimal_cur_24bpp();

        // Inner: 'icon' <size> <frame> (+ pad if odd)
        let mut icon_chunk = Vec::new();
        icon_chunk.extend_from_slice(b"icon");
        icon_chunk.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        icon_chunk.extend_from_slice(&frame);
        if frame.len() & 1 == 1 {
            icon_chunk.push(0);
        }

        // LIST 'fram' <icon_chunk>
        let mut list_body = Vec::new();
        list_body.extend_from_slice(b"fram");
        list_body.extend_from_slice(&icon_chunk);
        let mut list_chunk = Vec::new();
        list_chunk.extend_from_slice(b"LIST");
        list_chunk.extend_from_slice(&(list_body.len() as u32).to_le_bytes());
        list_chunk.extend_from_slice(&list_body);

        // RIFF <size> 'ACON' <list_chunk>
        let mut riff_body = Vec::new();
        riff_body.extend_from_slice(b"ACON");
        riff_body.extend_from_slice(&list_chunk);
        let mut ani = Vec::new();
        ani.extend_from_slice(b"RIFF");
        ani.extend_from_slice(&(riff_body.len() as u32).to_le_bytes());
        ani.extend_from_slice(&riff_body);

        let img = parse_ani_first_frame(&ani).expect(".ani first frame should decode");
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 1);
        assert_eq!(&img.rgba[..3], &[0x30, 0x20, 0x10]);
    }

    #[test]
    fn parse_ani_rejects_non_riff() {
        assert!(parse_ani_first_frame(b"not a riff file at all").is_err());
    }

    #[test]
    fn parse_ani_rejects_missing_icon() {
        // Valid RIFF/ACON but no icon chunk.
        let mut riff_body = Vec::new();
        riff_body.extend_from_slice(b"ACON");
        // anih chunk with 4 bytes of junk
        riff_body.extend_from_slice(b"anih");
        riff_body.extend_from_slice(&4u32.to_le_bytes());
        riff_body.extend_from_slice(&[0, 0, 0, 0]);
        let mut ani = Vec::new();
        ani.extend_from_slice(b"RIFF");
        ani.extend_from_slice(&(riff_body.len() as u32).to_le_bytes());
        ani.extend_from_slice(&riff_body);

        assert!(parse_ani_first_frame(&ani).is_err());
    }
}
