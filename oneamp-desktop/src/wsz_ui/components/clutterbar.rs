use egui::{Pos2, Ui};
use oneamp_core::wsz::skin::SkinComponent;

use super::super::renderer::WszRenderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClutterLetter {
    /// Options menu
    O,
    /// Always-on-top
    A,
    /// ID3 editor
    I,
    /// Doublesize mode
    D,
    /// Visualization plugin selector
    V,
}

impl ClutterLetter {
    /// Sprite x-coordinate of the pressed-letter variant in titlebar.bmp.
    /// All five live on row y=44, height 43.
    fn pressed_sprite_x(&self) -> u32 {
        match self {
            Self::O => 304,
            Self::A => 312,
            Self::I => 320,
            Self::D => 328,
            Self::V => 336,
        }
    }
}

/// `O A I D V` cosmetic strip on the left of the main window.
///
/// Drawn at (10, 22) 8×43 in skin-space. Sourced from `titlebar.bmp`:
/// - default appearance: (304, 0) 8×43
/// - per-letter pressed:  (304..336 step 8, 44) 8×43
///
/// Clicking any letter swaps the *whole* strip to that letter's pressed
/// sprite for the duration of the press — matches Winamp's behaviour.
/// No actions are wired yet; the strip is visual feedback only.
pub struct Clutterbar {
    pressed: Option<ClutterLetter>,
}

impl Clutterbar {
    pub fn new() -> Self {
        Self { pressed: None }
    }

    fn position() -> (u32, u32) {
        (10, 22)
    }

    fn size() -> (u32, u32) {
        (8, 43)
    }

    /// Map a y-offset within the strip (0..43) to a letter slot. The strip
    /// holds 5 letters; we divide it into rough 8/9-px slots per letter
    /// (Winamp's font is uneven across the column but the click targets
    /// straddle the visual letter centers).
    fn letter_at_y(dy: u32) -> Option<ClutterLetter> {
        match dy {
            0..=8 => Some(ClutterLetter::O),
            9..=17 => Some(ClutterLetter::A),
            18..=26 => Some(ClutterLetter::I),
            27..=34 => Some(ClutterLetter::D),
            35..=42 => Some(ClutterLetter::V),
            _ => None,
        }
    }

    pub fn render(&self, renderer: &mut WszRenderer, ui: &mut Ui, offset: Pos2) {
        let atlas = match renderer
            .get_skin()
            .get_bitmap(&SkinComponent::TitleBar)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        let (sx, sy) = Self::size();
        let sprite_x = match self.pressed {
            None => 304,
            Some(letter) => letter.pressed_sprite_x(),
        };
        let sprite_y = if self.pressed.is_some() { 44 } else { 0 };
        if let Some(region) = atlas.extract_region(sprite_x, sprite_y, sx, sy) {
            let (dx, dy) = Self::position();
            let pos = renderer.skin_to_screen(dx, dy, offset);
            renderer.render_region(ui, &region, pos, "clutterbar");
        }
    }

    /// Track the pressed letter for visual feedback.
    ///
    /// `inside` is true if the pointer is over the clutterbar (callers
    /// suppress drag-start on the title bar above). `clicked` is the
    /// letter that just received a click, when the caller-provided
    /// `click_just_started` flag is true and the pointer is on a real
    /// letter slot — used to fire the corresponding action exactly
    /// once per press.
    pub fn handle_input(
        &mut self,
        mouse_pos: Pos2,
        offset: Pos2,
        scale: f32,
        is_pressed: bool,
        click_just_started: bool,
    ) -> ClutterInput {
        let (px, py) = Self::position();
        let (w, h) = Self::size();
        let min_x = offset.x + px as f32 * scale;
        let min_y = offset.y + py as f32 * scale;
        let max_x = min_x + w as f32 * scale;
        let max_y = min_y + h as f32 * scale;

        let inside = mouse_pos.x >= min_x
            && mouse_pos.x <= max_x
            && mouse_pos.y >= min_y
            && mouse_pos.y <= max_y;

        let mut clicked = None;
        if is_pressed && inside {
            let dy = ((mouse_pos.y - min_y) / scale) as u32;
            let letter = Self::letter_at_y(dy);
            self.pressed = letter;
            if click_just_started {
                clicked = letter;
            }
        } else {
            self.pressed = None;
        }

        ClutterInput { inside, clicked }
    }
}

/// Output of `Clutterbar::handle_input`. `inside` blocks drag-start on
/// the title bar; `clicked` carries an edge-triggered letter so the
/// caller can fire its action exactly once per press.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClutterInput {
    pub inside: bool,
    pub clicked: Option<ClutterLetter>,
}

impl Default for Clutterbar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_slot_boundaries() {
        assert_eq!(Clutterbar::letter_at_y(0), Some(ClutterLetter::O));
        assert_eq!(Clutterbar::letter_at_y(8), Some(ClutterLetter::O));
        assert_eq!(Clutterbar::letter_at_y(9), Some(ClutterLetter::A));
        assert_eq!(Clutterbar::letter_at_y(17), Some(ClutterLetter::A));
        assert_eq!(Clutterbar::letter_at_y(18), Some(ClutterLetter::I));
        assert_eq!(Clutterbar::letter_at_y(35), Some(ClutterLetter::V));
        assert_eq!(Clutterbar::letter_at_y(42), Some(ClutterLetter::V));
        assert_eq!(Clutterbar::letter_at_y(43), None);
    }
}
