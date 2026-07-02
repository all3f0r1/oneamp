use super::bitmap::BitmapAtlas;
use super::cursor::{kind_from_filename, parse_ani_first_frame, parse_cur};
use super::pledit::parse_pledit;
use super::region::parse_region_file;
use super::skin::{SkinComponent, WszSkin};
use super::viscolor::parse_viscolor;
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

/// Bounds against a hostile `.wsz` (skins are downloaded from the web, so
/// treat the archive as untrusted input). A genuine Winamp skin has a
/// couple dozen small BMPs/text files; these caps are generous multiples
/// of that while still ruling out a zip-bomb-style blowup.
const MAX_ZIP_ENTRIES: usize = 4096;
const MAX_ENTRY_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB / file
const MAX_TOTAL_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB / archive

/// Read `file` fully, bounded by both a per-entry cap and a running
/// archive-wide total — independent of whatever uncompressed size the ZIP's
/// central directory *claims*, since that header is attacker-controlled.
/// Reads one byte past the per-entry cap to detect an overrun without
/// buffering the whole bomb first.
fn read_bounded<R: Read>(file: &mut R, total_decompressed: &mut u64) -> Result<Vec<u8>> {
    let remaining_total = MAX_TOTAL_DECOMPRESSED_BYTES.saturating_sub(*total_decompressed);
    let cap = MAX_ENTRY_DECOMPRESSED_BYTES.min(remaining_total);
    let mut buffer = Vec::new();
    let read = file.take(cap + 1).read_to_end(&mut buffer)?;
    if read as u64 > cap {
        bail!(
            "entry exceeds the {}-byte decompressed size limit (archive total cap {} MiB)",
            cap,
            MAX_TOTAL_DECOMPRESSED_BYTES / (1024 * 1024)
        );
    }
    *total_decompressed += read as u64;
    Ok(buffer)
}

pub struct WszLoader;

impl WszLoader {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<WszSkin> {
        let file = File::open(path.as_ref()).context("Failed to open WSZ file")?;

        Self::load_from_reader(file)
    }

    pub fn load_from_bytes(data: &[u8]) -> Result<WszSkin> {
        let cursor = Cursor::new(data);
        Self::load_from_reader(cursor)
    }

    fn load_from_reader<R: Read + std::io::Seek>(reader: R) -> Result<WszSkin> {
        let mut archive = ZipArchive::new(reader).context("Failed to read ZIP archive")?;

        if archive.len() > MAX_ZIP_ENTRIES {
            bail!(
                "WSZ archive has {} entries, exceeding the {} limit",
                archive.len(),
                MAX_ZIP_ENTRIES
            );
        }

        let mut skin = WszSkin::new();
        let mut total_decompressed: u64 = 0;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .context("Failed to read file from archive")?;

            let filename = file.name().to_string();
            let filename_lower = filename.to_lowercase();

            if filename_lower.ends_with(".bmp") {
                let buffer = match read_bounded(&mut file, &mut total_decompressed) {
                    Ok(buffer) => buffer,
                    Err(e) => {
                        eprintln!("Warning: Skipping oversized bitmap {}: {}", filename, e);
                        continue;
                    }
                };

                if let Some(component) = SkinComponent::from_filename(&filename) {
                    match BitmapAtlas::from_bytes(&buffer) {
                        Ok(mut atlas) => {
                            atlas.apply_transparency();
                            // text.bmp ships a non-magenta background in many
                            // classic Winamp skins (default.wsz uses dark blue
                            // 32,66,129 for example). Magenta keying alone
                            // leaves the entire atlas opaque, which means the
                            // glyph's background color paints over everything
                            // it overlaps. Detect the dominant remaining color
                            // and key it out as well so the title scroller
                            // shows the lit pixels only.
                            if matches!(component, SkinComponent::Text)
                                && let Some(bg) = atlas.dominant_opaque_color()
                            {
                                atlas.apply_color_key(bg);
                            }
                            skin.bitmaps.insert(component, atlas);
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to load bitmap {}: {}", filename, e);
                        }
                    }
                }
            } else if filename_lower == "region.txt" {
                let Ok(buffer) = read_bounded(&mut file, &mut total_decompressed) else {
                    continue;
                };
                let content = String::from_utf8_lossy(&buffer);
                match parse_region_file(&content) {
                    Ok(regions) => skin.regions = regions,
                    Err(e) => {
                        eprintln!("Warning: Failed to parse region.txt: {}", e);
                    }
                }
            } else if filename_lower == "viscolor.txt" {
                let Ok(buffer) = read_bounded(&mut file, &mut total_decompressed) else {
                    continue;
                };
                skin.vis_colors = parse_viscolor(&String::from_utf8_lossy(&buffer));
            } else if filename_lower == "pledit.txt" {
                let Ok(buffer) = read_bounded(&mut file, &mut total_decompressed) else {
                    continue;
                };
                let content = String::from_utf8_lossy(&buffer);
                match parse_pledit(&content) {
                    Ok(theme) => skin.pledit = theme,
                    Err(e) => {
                        eprintln!("Warning: Failed to parse pledit.txt: {}", e);
                    }
                }
            } else if filename_lower == "readme.txt" || filename_lower == "read_me.txt" {
                if let Ok(buffer) = read_bounded(&mut file, &mut total_decompressed) {
                    skin.metadata.readme = Some(String::from_utf8_lossy(&buffer).into_owned());
                }
            } else if filename_lower.ends_with(".cur") || filename_lower.ends_with(".ani") {
                let stem = filename.rsplit('/').next().unwrap_or(filename.as_str());
                let Some(kind) = kind_from_filename(stem) else {
                    continue;
                };
                let Ok(buffer) = read_bounded(&mut file, &mut total_decompressed) else {
                    continue;
                };
                // `.ani` is a RIFF wrapper around `.cur`/`.ico` frames. We
                // don't animate; we decode the FIRST frame so the cursor
                // shows something instead of nothing. If the container is
                // malformed, fall back to skipping it (never break loading).
                let parsed = if filename_lower.ends_with(".ani") {
                    parse_ani_first_frame(&buffer)
                } else {
                    parse_cur(&buffer)
                };
                match parsed {
                    Ok(image) => {
                        skin.cursors.insert(kind, image);
                    }
                    Err(e) => {
                        eprintln!("Skipping cursor {} (parse failed): {}", filename, e);
                    }
                }
            } else if (filename_lower.ends_with(".ttf") || filename_lower.ends_with(".otf"))
                && skin.font_data.is_none()
            {
                // First TTF/OTF wins — most skins ship at most one. Per the
                // pledit.txt spec, this font is meant for the playlist
                // editor and minibrowser. We hand the raw bytes to the
                // renderer; egui will register them as a custom family.
                if let Ok(buffer) = read_bounded(&mut file, &mut total_decompressed)
                    && !buffer.is_empty()
                {
                    skin.font_data = Some(std::sync::Arc::new(buffer));
                }
            }
        }

        Self::validate_skin(&skin)?;
        Self::fill_missing_critical_sheets(&mut skin);
        Self::extract_metadata(&mut skin);
        Self::apply_region_masks(&mut skin);

        Ok(skin)
    }

    /// The sheets the main/EQ/playlist renderer hard-depends on. A skin
    /// missing any of these (because the author omitted it, or it failed to
    /// parse and never landed in `skin.bitmaps`) renders broken — buttons
    /// vanish, the position bar disappears, etc. Winamp falls back to the
    /// base skin SHEET-BY-SHEET in this situation; we mirror that by
    /// substituting the matching sheet from `WszSkin::synthetic_default()`
    /// (see `fill_missing_critical_sheets`).
    ///
    /// `text` is intentionally treated as critical too: without it the title
    /// scroller can't render. `EqEx`/2.9+ sheets are not in this list — they
    /// are optional and the renderer already tolerates their absence.
    const CRITICAL_SHEETS: &'static [SkinComponent] = &[
        SkinComponent::Main,
        SkinComponent::CButtons,
        SkinComponent::TitleBar,
        SkinComponent::Numbers,
        SkinComponent::Text,
        SkinComponent::Volume,
        SkinComponent::Balance,
        SkinComponent::MonoSter,
        SkinComponent::PosBar,
        SkinComponent::PlayPaus,
        SkinComponent::EqMain,
        SkinComponent::Pledit,
        SkinComponent::Shufrep,
    ];

    /// For every critical sheet the loaded skin is missing, copy in the
    /// corresponding sheet from the built-in synthetic default. This is the
    /// per-sheet fallback Winamp performs against its base skin: a skin that
    /// ships `main.bmp` but forgot `cbuttons.bmp` still renders, with the
    /// synthetic cbuttons standing in for the missing one instead of leaving
    /// a hole (or crashing the renderer's `extract_region` calls).
    ///
    /// The fallback source is `WszSkin::synthetic_default()`, which is
    /// reachable from inside `oneamp-core` and guarantees every critical
    /// sheet at the standard Winamp dimensions. We only synthesize the
    /// default skin if at least one sheet is actually missing, so the common
    /// case (complete skin) pays nothing.
    fn fill_missing_critical_sheets(skin: &mut WszSkin) {
        let any_missing = Self::CRITICAL_SHEETS
            .iter()
            .any(|c| !skin.bitmaps.contains_key(c));
        if !any_missing {
            return;
        }

        let fallback = WszSkin::synthetic_default();
        for component in Self::CRITICAL_SHEETS {
            if skin.bitmaps.contains_key(component) {
                continue;
            }
            if let Some(sheet) = fallback.bitmaps.get(component) {
                eprintln!(
                    "Warning: WSZ skin missing critical sheet {:?}; substituting synthetic fallback",
                    component
                );
                skin.bitmaps.insert(component.clone(), sheet.clone());
            }
        }
    }

    /// Apply each `region.txt` polygon section as an alpha mask on the
    /// matching atlas. This is what gives Winamp its chamfered corners —
    /// pixels outside the polygon become transparent so the OS desktop shows
    /// through. No-op for sections missing from `region.txt`.
    ///
    /// Sections per Winamp spec:
    /// - `[Normal]` → main.bmp (full atlas)
    /// - `[WindowShade]` → titlebar.bmp shade strip (y=0..14)
    /// - `[Equalizer]` → eqmain.bmp window (y=0..116)
    /// - `[EqualizerWS]` → eq_ex.bmp window (y=0..14)
    fn apply_region_masks(skin: &mut WszSkin) {
        if let Some(region) = skin.region_by_name("Normal").cloned()
            && let Some(main) = skin.bitmaps.get_mut(&SkinComponent::Main)
        {
            main.apply_region_mask(&region);
        }
        if let Some(region) = skin.region_by_name("Equalizer").cloned()
            && let Some(eq) = skin.bitmaps.get_mut(&SkinComponent::EqMain)
        {
            // eqmain.bmp is 275×~313 — only the top 116 rows are the
            // visible window, the rest holds sprite frames that must
            // remain opaque.
            eq.apply_region_mask_in_rect(&region, 0, 0, 275, 116);
        }
        if let Some(region) = skin.region_by_name("WindowShade").cloned()
            && let Some(tb) = skin.bitmaps.get_mut(&SkinComponent::TitleBar)
        {
            // The shade-mode window uses the active strip at (27, 29)
            // 275×14, but the spec defines the polygon in 0..275 × 0..14
            // local coordinates. Mask only that strip.
            tb.apply_region_mask_in_rect(&region, 27, 29, 275, 14);
        }
        if let Some(region) = skin.region_by_name("EqualizerWS").cloned()
            && let Some(eq) = skin.bitmaps.get_mut(&SkinComponent::EqEx)
        {
            eq.apply_region_mask_in_rect(&region, 0, 0, 275, 14);
        }
    }

    /// Minimum-viability gate, run BEFORE per-sheet fallback.
    ///
    /// Policy: we require `main.bmp` to be present. Everything else can be
    /// filled in sheet-by-sheet from the synthetic default
    /// (`fill_missing_critical_sheets`), but `main.bmp` is the anchor — it
    /// defines the window geometry and proves the archive is actually a
    /// Winamp skin and not, say, an empty zip or a zip of unrelated files.
    /// Rejecting here (rather than silently synthesizing a whole skin)
    /// keeps a totally empty/garbage archive from loading as a blank
    /// "OneAmp Default" with no indication anything went wrong.
    fn validate_skin(skin: &WszSkin) -> Result<()> {
        if !skin.has_component(&SkinComponent::Main) {
            bail!("Invalid skin: missing main.bmp");
        }

        Ok(())
    }

    fn extract_metadata(skin: &mut WszSkin) {
        if let Some(readme) = &skin.metadata.readme {
            let lines: Vec<&str> = readme.lines().take(10).collect();

            for line in lines {
                let line = line.trim();

                if skin.metadata.name == "Unknown Skin"
                    && !line.is_empty()
                    && !line.starts_with('#')
                {
                    skin.metadata.name = line.to_string();
                }

                if line.to_lowercase().contains("author:") {
                    let parts: Vec<&str> = line.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        skin.metadata.author = Some(parts[1].trim().to_string());
                    }
                }

                if line.to_lowercase().contains("version:") {
                    let parts: Vec<&str> = line.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        skin.metadata.version = Some(parts[1].trim().to_string());
                    }
                }
            }
        }
    }

    pub fn list_components(skin: &WszSkin) -> Vec<String> {
        let mut components: Vec<String> = skin.bitmaps.keys().map(|c| format!("{:?}", c)).collect();

        components.sort();
        components
    }

    pub fn get_skin_info(skin: &WszSkin) -> String {
        let mut info = String::new();
        info.push_str(&format!("Name: {}\n", skin.metadata.name));

        if let Some(author) = &skin.metadata.author {
            info.push_str(&format!("Author: {}\n", author));
        }

        if let Some(version) = &skin.metadata.version {
            info.push_str(&format!("Version: {}\n", version));
        }

        info.push_str(&format!("Components: {}\n", skin.bitmaps.len()));
        info.push_str(&format!("Regions: {}\n", skin.regions.len()));

        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::{FileOptions, ZipWriter};

    fn create_test_wsz() -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
            let options: FileOptions<'_, ()> = FileOptions::default();

            let bmp_header: Vec<u8> = vec![
                0x42, 0x4D, 0x36, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x00, 0x00, 0x00,
                0x28, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00,
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF,
                0xFF, 0x00,
            ];

            zip.start_file("main.bmp", options).unwrap();
            zip.write_all(&bmp_header).unwrap();

            zip.start_file("readme.txt", options).unwrap();
            zip.write_all(b"Test Skin\nAuthor: Test\nVersion: 1.0")
                .unwrap();

            zip.finish().unwrap();
        }
        buffer
    }

    #[test]
    fn test_load_wsz_from_bytes() {
        let wsz_data = create_test_wsz();
        let result = WszLoader::load_from_bytes(&wsz_data);

        assert!(result.is_ok());
        let skin = result.unwrap();
        assert!(skin.has_component(&SkinComponent::Main));
    }

    #[test]
    fn test_validate_skin_missing_main() {
        let skin = WszSkin::new();
        let result = WszLoader::validate_skin(&skin);
        assert!(result.is_err());
    }

    #[test]
    fn test_loads_ttf_font_when_present() {
        let mut buffer = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
            let options: FileOptions<'_, ()> = FileOptions::default();

            // Minimum-viable main.bmp so validation passes
            let bmp_header: Vec<u8> = vec![
                0x42, 0x4D, 0x36, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x00, 0x00, 0x00,
                0x28, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00,
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF,
                0xFF, 0x00,
            ];
            zip.start_file("main.bmp", options).unwrap();
            zip.write_all(&bmp_header).unwrap();

            // Synthetic TTF — egui won't actually parse it, but the loader
            // doesn't validate; it only stores bytes. The integration with
            // egui's font registration is exercised at runtime.
            zip.start_file("ARIAL.TTF", options).unwrap();
            zip.write_all(b"\x00\x01\x00\x00\x00fake-ttf").unwrap();

            zip.finish().unwrap();
        }
        let skin = WszLoader::load_from_bytes(&buffer).unwrap();
        assert!(
            skin.font_data.is_some(),
            "TTF in archive should land in skin.font_data"
        );
        assert_eq!(
            skin.font_data.as_ref().unwrap().as_slice(),
            b"\x00\x01\x00\x00\x00fake-ttf"
        );
    }

    #[test]
    fn test_no_ttf_means_no_font_data() {
        let wsz_data = create_test_wsz();
        let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();
        assert!(skin.font_data.is_none());
    }

    #[test]
    fn test_missing_critical_sheets_filled_from_synthetic() {
        // The test WSZ ships only main.bmp (+ readme). Every other critical
        // sheet must be substituted from the synthetic default so the
        // renderer doesn't break on a partial skin.
        let wsz_data = create_test_wsz();
        let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

        for component in WszLoader::CRITICAL_SHEETS {
            assert!(
                skin.has_component(component),
                "critical sheet {:?} should be present after per-sheet fallback",
                component
            );
        }

        // The fallback cbuttons must be a usable atlas (non-zero size) so
        // the renderer's extract_region calls succeed.
        let cbuttons = skin.get_bitmap(&SkinComponent::CButtons).unwrap();
        assert!(cbuttons.width > 0 && cbuttons.height > 0);
    }

    #[test]
    fn test_complete_skin_keeps_own_main_sheet() {
        // When main.bmp is present in the archive, the loader must keep the
        // archive's own sheet — fallback only fills sheets that are absent.
        let wsz_data = create_test_wsz();
        let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();
        let main = skin.get_bitmap(&SkinComponent::Main).unwrap();
        // The test main.bmp is 1×1; the synthetic Main is 275×116. Proving
        // we kept the 1×1 archive sheet confirms no clobbering.
        assert_eq!(main.width, 1);
        assert_eq!(main.height, 1);
    }

    #[test]
    fn test_get_skin_info() {
        let mut skin = WszSkin::new();
        skin.metadata.name = "Test Skin".to_string();
        skin.metadata.author = Some("Test Author".to_string());

        let info = WszLoader::get_skin_info(&skin);
        assert!(info.contains("Test Skin"));
        assert!(info.contains("Test Author"));
    }
}
