use egui::Pos2;

use super::super::renderer::WszRenderer;
use oneamp_core::wsz::skin::SkinComponent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Stopped,
    Playing,
    Paused,
}

/// Tiny play / pause / stop indicator drawn at the left of the time digits.
/// Sourced from `playpaus.bmp` (42×9 atlas). Layout per WSZ_FORMAT.md:
/// - (1,0) 8×9  play       (col 0 reserved as filler)
/// - (9,0) 9×9  pause
/// - (18,0) 9×9 stop
/// - (27,0) 2×9 filler (drawn under play to cover the pause/stop edge)
/// - (36,0) 3×9 work indicator on  (seek/buffer)
/// - (39,0) 3×9 work indicator off
pub struct PlayStateIndicator {
    position: (u32, u32),
    state: PlayState,
}

impl PlayStateIndicator {
    pub fn new() -> Self {
        Self {
            position: (24, 28),
            state: PlayState::Stopped,
        }
    }

    pub fn set_state(&mut self, state: PlayState) {
        self.state = state;
    }

    pub fn render(&self, renderer: &mut WszRenderer, ui: &mut egui::Ui, offset: Pos2) {
        let atlas = match renderer
            .get_skin()
            .get_bitmap(&SkinComponent::PlayPaus)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };
        let (dx, dy) = self.position;

        match self.state {
            PlayState::Stopped => {
                if let Some(r) = atlas.extract_region(18, 0, 9, 9) {
                    let pos = renderer.skin_to_screen(dx, dy, offset);
                    renderer.render_region(ui, &r, pos, "playpaus_stop");
                }
            }
            PlayState::Paused => {
                if let Some(r) = atlas.extract_region(9, 0, 9, 9) {
                    let pos = renderer.skin_to_screen(dx, dy, offset);
                    renderer.render_region(ui, &r, pos, "playpaus_pause");
                }
            }
            PlayState::Playing => {
                // Filler 2×9 covers the leftmost columns, then play 8×9 sits
                // to its right. Together they span 10 px of the 11-px slot.
                if let Some(r) = atlas.extract_region(27, 0, 2, 9) {
                    let pos = renderer.skin_to_screen(dx, dy, offset);
                    renderer.render_region(ui, &r, pos, "playpaus_filler");
                }
                if let Some(r) = atlas.extract_region(1, 0, 8, 9) {
                    let pos = renderer.skin_to_screen(dx + 2, dy, offset);
                    renderer.render_region(ui, &r, pos, "playpaus_play");
                }
            }
        }
    }
}

impl Default for PlayStateIndicator {
    fn default() -> Self {
        Self::new()
    }
}
