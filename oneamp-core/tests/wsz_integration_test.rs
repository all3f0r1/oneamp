use oneamp_core::wsz::{BitmapAtlas, SkinComponent, WszLoader};
use std::io::Cursor;
use std::io::Write;
use zip::write::{FileOptions, ZipWriter};

fn create_minimal_bmp(width: u32, height: u32) -> Vec<u8> {
    let row_size = (width * 3).div_ceil(4) * 4;
    let pixel_data_size = row_size * height;
    let file_size = 54 + pixel_data_size;

    let mut bmp = Vec::new();

    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());

    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    bmp.extend_from_slice(&height.to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&pixel_data_size.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());

    for _y in 0..height {
        for _x in 0..width {
            bmp.push(128);
            bmp.push(128);
            bmp.push(128);
        }
        while (bmp.len() - 54) % row_size as usize != 0 {
            bmp.push(0);
        }
    }

    bmp
}

fn create_test_wsz_complete() -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let options: FileOptions<'_, ()> = FileOptions::default();

        zip.start_file("main.bmp", options).unwrap();
        zip.write_all(&create_minimal_bmp(275, 116)).unwrap();

        zip.start_file("cbuttons.bmp", options).unwrap();
        zip.write_all(&create_minimal_bmp(115, 36)).unwrap();

        zip.start_file("numbers.bmp", options).unwrap();
        zip.write_all(&create_minimal_bmp(99, 13)).unwrap();

        zip.start_file("volume.bmp", options).unwrap();
        zip.write_all(&create_minimal_bmp(68, 13)).unwrap();

        zip.start_file("readme.txt", options).unwrap();
        zip.write_all(b"Test Skin for OneAmp\nAuthor: Integration Test\nVersion: 1.0.0")
            .unwrap();

        zip.start_file("region.txt", options).unwrap();
        // Real Winamp region.txt syntax: each section's polygons share one
        // NumPoints/PointList line.
        let region_content = r#"
; Main window region
[MainWindow]
NumPoints=4
PointList=0,0 275,0 275,116 0,116

; Play button region
[PlayButton]
NumPoints=4
PointList=16,88 39,88 39,106 16,106
"#;
        zip.write_all(region_content.as_bytes()).unwrap();

        zip.finish().unwrap();
    }
    buffer
}

#[test]
fn test_load_complete_skin() {
    let wsz_data = create_test_wsz_complete();
    let result = WszLoader::load_from_bytes(&wsz_data);

    assert!(result.is_ok(), "Should successfully load complete skin");
    let skin = result.unwrap();

    assert_eq!(skin.metadata.name, "Test Skin for OneAmp");
    assert_eq!(skin.metadata.author, Some("Integration Test".to_string()));
    assert_eq!(skin.metadata.version, Some("1.0.0".to_string()));
}

#[test]
fn test_skin_components_loaded() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    assert!(skin.has_component(&SkinComponent::Main));
    assert!(skin.has_component(&SkinComponent::CButtons));
    assert!(skin.has_component(&SkinComponent::Numbers));
    assert!(skin.has_component(&SkinComponent::Volume));

    // Per-sheet fallback (Winamp parity): even though this test archive
    // ships no pledit.bmp, the loader now substitutes the synthetic
    // sheet so the renderer never hits a missing critical component.
    assert!(skin.has_component(&SkinComponent::Pledit));
}

#[test]
fn test_bitmap_dimensions() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    let main = skin.get_bitmap(&SkinComponent::Main).unwrap();
    assert_eq!(main.width, 275);
    assert_eq!(main.height, 116);

    let volume = skin.get_bitmap(&SkinComponent::Volume).unwrap();
    assert_eq!(volume.width, 68);
    assert_eq!(volume.height, 13);
}

#[test]
fn test_regions_loaded() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    assert_eq!(skin.regions.len(), 2);

    let main_window = skin
        .regions
        .iter()
        .find(|r| r.name == "MainWindow")
        .expect("Should have MainWindow region");

    assert_eq!(main_window.polygons.len(), 1);
    assert_eq!(main_window.polygons[0].points.len(), 4);

    let play_button = skin
        .regions
        .iter()
        .find(|r| r.name == "PlayButton")
        .expect("Should have PlayButton region");

    assert_eq!(play_button.polygons.len(), 1);
    assert_eq!(play_button.polygons[0].points.len(), 4);
}

#[test]
fn test_region_hit_testing() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    let play_button = skin
        .regions
        .iter()
        .find(|r| r.name == "PlayButton")
        .expect("Should have PlayButton region");

    assert!(play_button.contains(27, 97));
    assert!(!play_button.contains(100, 100));
    assert!(!play_button.contains(0, 0));
}

#[test]
fn test_bitmap_region_extraction() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    let main = skin.get_bitmap(&SkinComponent::Main).unwrap();

    let region = main.extract_region(0, 0, 100, 50);
    assert!(region.is_some());

    let region = region.unwrap();
    assert_eq!(region.width, 100);
    assert_eq!(region.height, 50);
    assert_eq!(region.x, 0);
    assert_eq!(region.y, 0);
}

#[test]
fn test_transparency_magenta() {
    let width: u32 = 10;
    let height: u32 = 10;
    let mut bmp_data = create_minimal_bmp(width, height);

    // BMP pixel data is BGR, bottom-up; the first row in the file ends up at
    // the bottom of the decoded RGBA image after the BMP→RGBA flip.
    let header_size = 54;
    bmp_data[header_size] = 255;
    bmp_data[header_size + 1] = 0;
    bmp_data[header_size + 2] = 255;

    let mut atlas = BitmapAtlas::from_bytes(&bmp_data).unwrap();
    atlas.apply_transparency();

    let bottom_left_alpha = ((height - 1) * width * 4 + 3) as usize;
    assert_eq!(
        atlas.data[bottom_left_alpha], 0,
        "Magenta pixel should be transparent"
    );
}

#[test]
fn test_missing_main_bmp() {
    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let options: FileOptions<'_, ()> = FileOptions::default();

        zip.start_file("cbuttons.bmp", options).unwrap();
        zip.write_all(&create_minimal_bmp(115, 36)).unwrap();

        zip.finish().unwrap();
    }

    let result = WszLoader::load_from_bytes(&buffer);
    assert!(result.is_err(), "Should fail without main.bmp");
}

#[test]
fn test_skin_info_formatting() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    let info = WszLoader::get_skin_info(&skin);

    assert!(info.contains("Test Skin for OneAmp"));
    assert!(info.contains("Integration Test"));
    assert!(info.contains("1.0.0"));
    assert!(info.contains("Components:"));
    assert!(info.contains("Regions:"));
}

#[test]
fn test_list_components() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    let components = WszLoader::list_components(&skin);

    assert!(!components.is_empty());
    assert!(components.iter().any(|c| c.contains("Main")));
}

#[test]
fn test_bitmap_to_rgba_image() {
    let wsz_data = create_test_wsz_complete();
    let skin = WszLoader::load_from_bytes(&wsz_data).unwrap();

    let main = skin.get_bitmap(&SkinComponent::Main).unwrap();
    let rgba_image = main.to_rgba_image();

    assert_eq!(rgba_image.width(), 275);
    assert_eq!(rgba_image.height(), 116);
}
