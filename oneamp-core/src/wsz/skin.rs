use super::bitmap::BitmapAtlas;
use super::cursor::{CursorImage, CursorKind};
use super::pledit::{DEFAULT_PLEDIT_COLORS, PleditTheme};
use super::region::Region;
use super::viscolor::{DEFAULT_VIS_COLORS, VisColors};
use std::collections::HashMap;

/// Build a solid-color RGBA atlas of the given dimensions.
fn solid_atlas(width: u32, height: u32, rgba: [u8; 4]) -> BitmapAtlas {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        data.extend_from_slice(&rgba);
    }
    BitmapAtlas {
        width,
        height,
        data,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkinComponent {
    Main,
    CButtons,
    MonoSter,
    Numbers,
    PlayPaus,
    PosBar,
    TitleBar,
    Volume,
    Balance,
    Shufrep,
    Text,
    Pledit,
    EqMain,
    EqEx,
    /// `mb.bmp` — minibrowser frame (pre-2.9, almost never used).
    Mb,
    /// `avs.bmp` — AVS visualization window frame (2.x+).
    Avs,
    /// `gen.bmp` — Winamp 2.9+ general-purpose window frame (Media Library).
    Gen,
    /// `genex.bmp` — Winamp 2.9+ buttons/sliders/palette atlas.
    GenEx,
    /// `nums_ex.bmp` — Winamp 2.9+ extended numbers (adds minus glyph for
    /// time-left mode).
    NumsEx,
    /// `video.bmp` — Winamp 2.9+ video window frame.
    Video,
    Custom(String),
}

impl SkinComponent {
    pub fn from_filename(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        match lower.as_str() {
            "main.bmp" => Some(Self::Main),
            "cbuttons.bmp" => Some(Self::CButtons),
            "monoster.bmp" => Some(Self::MonoSter),
            "numbers.bmp" => Some(Self::Numbers),
            "playpaus.bmp" => Some(Self::PlayPaus),
            "posbar.bmp" => Some(Self::PosBar),
            "titlebar.bmp" => Some(Self::TitleBar),
            "volume.bmp" => Some(Self::Volume),
            "balance.bmp" => Some(Self::Balance),
            "shufrep.bmp" => Some(Self::Shufrep),
            "text.bmp" => Some(Self::Text),
            "pledit.bmp" => Some(Self::Pledit),
            "eqmain.bmp" => Some(Self::EqMain),
            "eq_ex.bmp" => Some(Self::EqEx),
            "mb.bmp" => Some(Self::Mb),
            "avs.bmp" => Some(Self::Avs),
            "gen.bmp" => Some(Self::Gen),
            "genex.bmp" => Some(Self::GenEx),
            "nums_ex.bmp" => Some(Self::NumsEx),
            "video.bmp" => Some(Self::Video),
            _ if lower.ends_with(".bmp") => Some(Self::Custom(name.to_string())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkinMetadata {
    pub name: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub readme: Option<String>,
}

impl Default for SkinMetadata {
    fn default() -> Self {
        Self {
            name: "Unknown Skin".to_string(),
            author: None,
            version: None,
            readme: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WszSkin {
    pub metadata: SkinMetadata,
    pub bitmaps: HashMap<SkinComponent, BitmapAtlas>,
    pub regions: Vec<Region>,
    pub vis_colors: VisColors,
    pub pledit: PleditTheme,
    /// Raw bytes of a TTF/OTF font shipped inside the WSZ archive. Per the
    /// `pledit.txt` spec, Winamp loads this for the playlist editor and
    /// minibrowser instead of the named OS font. `None` means the skin
    /// didn't ship one; the renderer should fall back to its default
    /// family. Wrapped in `Arc` so registering it with egui's
    /// `FontDefinitions` is a refcount bump rather than a clone.
    pub font_data: Option<std::sync::Arc<Vec<u8>>>,
    /// Decoded `.cur` cursor sprites keyed by their Winamp slot. Empty
    /// when the skin ships no cursors; the renderer should fall back to
    /// the OS arrow in that case.
    pub cursors: HashMap<CursorKind, CursorImage>,
}

impl WszSkin {
    pub fn new() -> Self {
        Self {
            metadata: SkinMetadata::default(),
            bitmaps: HashMap::new(),
            regions: Vec::new(),
            vis_colors: DEFAULT_VIS_COLORS,
            pledit: PleditTheme {
                colors: DEFAULT_PLEDIT_COLORS,
                font: None,
            },
            font_data: None,
            cursors: HashMap::new(),
        }
    }

    /// Find the `[Normal]` region — the polygon mask for the main window's
    /// non-shade state. Comparison is case-insensitive because skin authors
    /// don't agree on capitalization.
    pub fn normal_region(&self) -> Option<&Region> {
        self.region_by_name("Normal")
    }

    /// Case-insensitive lookup for a named region. Returns the first non-empty
    /// region matching `name` — empty regions are treated as "no mask".
    pub fn region_by_name(&self, name: &str) -> Option<&Region> {
        self.regions
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(name) && !r.is_empty())
    }

    /// Built-in fallback skin used when no `.wsz` file is configured.
    /// Produces solid-color bitmap atlases at the standard Winamp dimensions
    /// so the renderer has something to extract regions from. The result is
    /// a flat dark UI — placeholder until a real default skin is shipped.
    pub fn synthetic_default() -> Self {
        let mut skin = Self::new();
        skin.metadata.name = "OneAmp Default".to_string();
        skin.metadata.author = Some("OneAmp".to_string());

        // Color palette: slate gray UI with classic Winamp-green accents.
        const BG: [u8; 4] = [0x2a, 0x2c, 0x33, 0xff];
        const PANEL: [u8; 4] = [0x18, 0x1a, 0x1f, 0xff];
        const BUTTON: [u8; 4] = [0x40, 0x44, 0x4a, 0xff];
        const ACCENT: [u8; 4] = [0x00, 0xc8, 0x4a, 0xff];
        const DIGIT_BG: [u8; 4] = [0x10, 0x10, 0x10, 0xff];

        // Standard Winamp atlas sizes — keep enough room for the renderer's
        // extract_region calls (some encode frame index as an x-offset, e.g.
        // posbar uses frame*8 + 248 width). Oversized atlases are harmless.
        skin.bitmaps
            .insert(SkinComponent::Main, solid_atlas(275, 116, BG));
        skin.bitmaps
            .insert(SkinComponent::TitleBar, solid_atlas(275, 28, PANEL));
        skin.bitmaps
            .insert(SkinComponent::CButtons, solid_atlas(138, 36, BUTTON));
        skin.bitmaps
            .insert(SkinComponent::Numbers, solid_atlas(99, 13, DIGIT_BG));
        skin.bitmaps
            .insert(SkinComponent::Text, solid_atlas(287, 18, PANEL));
        skin.bitmaps
            .insert(SkinComponent::PosBar, solid_atlas(500, 10, PANEL));
        skin.bitmaps
            .insert(SkinComponent::Volume, solid_atlas(68, 422, ACCENT));
        skin.bitmaps
            .insert(SkinComponent::Balance, solid_atlas(38, 422, ACCENT));
        skin.bitmaps
            .insert(SkinComponent::MonoSter, solid_atlas(58, 24, PANEL));
        skin.bitmaps
            .insert(SkinComponent::Shufrep, solid_atlas(275, 43, PANEL));
        skin.bitmaps
            .insert(SkinComponent::Pledit, solid_atlas(275, 232, PANEL));
        skin.bitmaps
            .insert(SkinComponent::EqMain, solid_atlas(275, 313, BG));
        skin.bitmaps
            .insert(SkinComponent::EqEx, solid_atlas(275, 56, BG));
        skin.bitmaps
            .insert(SkinComponent::PlayPaus, solid_atlas(42, 9, ACCENT));

        skin
    }

    pub fn get_bitmap(&self, component: &SkinComponent) -> Option<&BitmapAtlas> {
        self.bitmaps.get(component)
    }

    pub fn has_component(&self, component: &SkinComponent) -> bool {
        self.bitmaps.contains_key(component)
    }

    pub fn list_components(&self) -> Vec<&SkinComponent> {
        self.bitmaps.keys().collect()
    }
}

impl Default for WszSkin {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_component_from_filename() {
        assert_eq!(
            SkinComponent::from_filename("main.bmp"),
            Some(SkinComponent::Main)
        );
        assert_eq!(
            SkinComponent::from_filename("MAIN.BMP"),
            Some(SkinComponent::Main)
        );
        assert_eq!(
            SkinComponent::from_filename("cbuttons.bmp"),
            Some(SkinComponent::CButtons)
        );
        assert!(matches!(
            SkinComponent::from_filename("custom.bmp"),
            Some(SkinComponent::Custom(_))
        ));
        assert_eq!(SkinComponent::from_filename("notabmp.txt"), None);
    }

    #[test]
    fn test_wsz_skin_creation() {
        let skin = WszSkin::new();
        assert_eq!(skin.bitmaps.len(), 0);
        assert_eq!(skin.regions.len(), 0);
        assert_eq!(skin.metadata.name, "Unknown Skin");
    }

    #[test]
    fn test_synthetic_default_has_required_components() {
        let skin = WszSkin::synthetic_default();
        // The renderer requires at minimum a Main bitmap; the validate step
        // in WszLoader rejects skins without it. Make sure the synthetic
        // fallback satisfies that contract and provides every component the
        // main window references.
        for required in [
            SkinComponent::Main,
            SkinComponent::CButtons,
            SkinComponent::PosBar,
            SkinComponent::Volume,
            SkinComponent::Balance,
            SkinComponent::Numbers,
            SkinComponent::EqMain,
            SkinComponent::Pledit,
        ] {
            assert!(
                skin.has_component(&required),
                "synthetic_default missing {:?}",
                required
            );
        }
        // PosBar's renderer encodes the slider frame as an x-offset of
        // up to 28*8 + 248 = 472 px, so the atlas must be at least that wide.
        let posbar = skin.get_bitmap(&SkinComponent::PosBar).unwrap();
        assert!(posbar.width >= 472);
    }
}
