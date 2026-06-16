//! Integration smoke test: the bundled default skin's `.cur` files must
//! parse so the cursor overlay has something to render at runtime. If
//! this test breaks, OneAmp falls back to the OS cursor silently — the
//! test makes the regression loud.

use oneamp_core::wsz::WszLoader;
use oneamp_core::wsz::cursor::CursorKind;

const BUNDLED_SKIN: &[u8] = include_bytes!("../../skins/base-2.91.wsz");

#[test]
fn bundled_skin_cursors_parse() {
    let skin = WszLoader::load_from_bytes(BUNDLED_SKIN).expect("bundled skin loads");
    // The 2.91 base skin ships 8 cursors (CLOSE/EQSLID/EQTITLE/MAINMENU/
    // POSBAR/PSIZE/TITLEBAR/VOLBAL). Other skins may vary; this test is
    // strictly about the bundled one.
    assert!(
        skin.cursors.len() >= 4,
        "expected ≥4 cursors in default skin, got {}",
        skin.cursors.len()
    );
    // Every parsed cursor must have a non-empty RGBA buffer that matches
    // its declared dims. A zero-area buffer means the parser silently
    // produced a useless image.
    for (kind, image) in &skin.cursors {
        let expected = (image.width * image.height * 4) as usize;
        assert_eq!(
            image.rgba.len(),
            expected,
            "{:?}: rgba len {} ≠ {}×{}×4 = {}",
            kind,
            image.rgba.len(),
            image.width,
            image.height,
            expected
        );
        assert!(
            image.width > 0 && image.height > 0,
            "{:?}: zero-sized",
            kind
        );
    }
    // Spot-check a cursor we know is in the archive.
    assert!(
        skin.cursors.contains_key(&CursorKind::Close),
        "CLOSE.CUR missing from cursors"
    );
}
