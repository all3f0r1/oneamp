use egui::{Pos2, Rect, Sense, Ui, Vec2};
use oneamp_core::wsz::skin::SkinComponent;

use super::super::renderer::WszRenderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TitlebarButton {
    Menu,
    Minimize,
    Shade,
    Close,
}

impl TitlebarButton {
    /// Destination position on main.bmp (titlebar origin == main window origin).
    pub fn position(&self) -> (u32, u32) {
        match self {
            Self::Menu => (6, 3),
            Self::Minimize => (244, 3),
            Self::Shade => (254, 3),
            Self::Close => (264, 3),
        }
    }

    /// All four window-control buttons are 9×9.
    pub fn size(&self) -> (u32, u32) {
        (9, 9)
    }

    /// Sprite coords inside titlebar.bmp. Layout per WSZ_FORMAT.md:
    /// - rows 0..18 hold Menu/Min/Close pairs (unpressed at y=0, pressed y=9).
    /// - row 18 holds Windowshade unpressed (x=0) / pressed (x=9).
    ///   (Maximize/Unshade lives at row 27 — used only by shade mode.)
    pub fn sprite_coords(&self, pressed: bool) -> (u32, u32) {
        let p = if pressed { 9 } else { 0 };
        match self {
            Self::Menu => (0, p),
            Self::Minimize => (9, p),
            Self::Close => (18, p),
            Self::Shade => (p, 18),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitlebarAction {
    OpenMenu,
    Minimize,
    ToggleShade,
    Close,
}

pub struct TitlebarButtons {
    /// Whether the window currently has focus — drives the active vs inactive
    /// title bar strip.
    pub active: bool,
    /// True when the audio engine failed to start. Selects the alternate
    /// strip variant (y=57/72) — Winamp surfaces a visibly different bar
    /// so the user knows playback is dead before they hit Play.
    pub audio_failed: bool,
    pressed: Option<TitlebarButton>,
}

impl TitlebarButtons {
    pub fn new() -> Self {
        Self {
            active: true,
            audio_failed: false,
            pressed: None,
        }
    }

    /// Render the titlebar strip on top of main.bmp's top 14 px and the 4
    /// control-button sprites at their fixed positions. No-op if titlebar.bmp
    /// is missing.
    pub fn render(&mut self, renderer: &mut WszRenderer, ui: &mut Ui, offset: Pos2) {
        let atlas = match renderer
            .get_skin()
            .get_bitmap(&SkinComponent::TitleBar)
            .cloned()
        {
            Some(a) => a,
            None => return,
        };

        // Strip variants per WSZ_FORMAT (y, all at x=27, 275×14):
        //   y=0   active            y=15  inactive
        //   y=29  audio-init failed active (with EQ visual)  unused here
        //   y=57  audio-init failed active     y=72  audio-init failed inactive
        let strip_y = match (self.audio_failed, self.active) {
            (true, true) => 57,
            (true, false) => 72,
            (false, true) => 0,
            (false, false) => 15,
        };
        if let Some(strip) = atlas.extract_region(27, strip_y, 275, 14) {
            let pos = renderer.skin_to_screen(0, 0, offset);
            renderer.render_region(ui, &strip, pos, "titlebar_strip");
        }

        // Buttons. We don't draw a Menu sprite when there's no titlebar.bmp —
        // already handled by the early return above.
        for &btn in &[
            TitlebarButton::Menu,
            TitlebarButton::Minimize,
            TitlebarButton::Shade,
            TitlebarButton::Close,
        ] {
            let pressed = self.pressed == Some(btn);
            let (sx, sy) = btn.sprite_coords(pressed);
            let (w, h) = btn.size();
            if let Some(region) = atlas.extract_region(sx, sy, w, h) {
                let (dx, dy) = btn.position();
                let pos = renderer.skin_to_screen(dx, dy, offset);
                renderer.render_region(ui, &region, pos, &format!("titlebar_{:?}", btn));
            }
        }
    }

    /// Process a click in the titlebar area. Returns the action to dispatch.
    /// `mouse_pos` is in screen coordinates; `offset` is the main window origin.
    pub fn check_click(
        &mut self,
        ui: &Ui,
        mouse_pos: Pos2,
        offset: Pos2,
        scale: f32,
        is_pressed: bool,
        click_just_started: bool,
    ) -> Option<TitlebarAction> {
        let buttons = [
            TitlebarButton::Menu,
            TitlebarButton::Minimize,
            TitlebarButton::Shade,
            TitlebarButton::Close,
        ];

        // Visual feedback: highlight the button under the pointer while held.
        self.pressed = if is_pressed {
            buttons
                .into_iter()
                .find(|b| hit_test(*b, mouse_pos, offset, scale))
        } else {
            None
        };

        if !click_just_started {
            return None;
        }

        let hit = buttons
            .into_iter()
            .find(|b| hit_test(*b, mouse_pos, offset, scale))?;

        // Reserve a hit area in egui so the underlying drag handler does not
        // also consume this click as a drag start.
        let (dx, dy) = hit.position();
        let (w, h) = hit.size();
        let rect = Rect::from_min_size(
            Pos2::new(offset.x + dx as f32 * scale, offset.y + dy as f32 * scale),
            Vec2::new(w as f32 * scale, h as f32 * scale),
        );
        ui.interact(rect, egui::Id::new(("titlebar_btn", hit)), Sense::click());

        Some(match hit {
            TitlebarButton::Menu => TitlebarAction::OpenMenu,
            TitlebarButton::Minimize => TitlebarAction::Minimize,
            TitlebarButton::Shade => TitlebarAction::ToggleShade,
            TitlebarButton::Close => TitlebarAction::Close,
        })
    }
}

fn hit_test(btn: TitlebarButton, mouse_pos: Pos2, offset: Pos2, scale: f32) -> bool {
    let (dx, dy) = btn.position();
    let (w, h) = btn.size();
    let min_x = offset.x + dx as f32 * scale;
    let min_y = offset.y + dy as f32 * scale;
    let max_x = min_x + w as f32 * scale;
    let max_y = min_y + h as f32 * scale;
    mouse_pos.x >= min_x && mouse_pos.x <= max_x && mouse_pos.y >= min_y && mouse_pos.y <= max_y
}

impl Default for TitlebarButtons {
    fn default() -> Self {
        Self::new()
    }
}
