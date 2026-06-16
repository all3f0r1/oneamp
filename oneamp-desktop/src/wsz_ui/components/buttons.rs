use egui::Pos2;
use oneamp_core::wsz::bitmap::ButtonState;
use oneamp_core::wsz::skin::SkinComponent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WinampButton {
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
    Shuffle,
    Repeat,
    EqToggle,
    PlToggle,
}

impl WinampButton {
    pub fn position(&self) -> (u32, u32) {
        match self {
            Self::Previous => (16, 88),
            Self::Play => (39, 88),
            Self::Pause => (62, 88),
            Self::Stop => (85, 88),
            Self::Next => (108, 88),
            Self::Eject => (136, 89),
            Self::Shuffle => (164, 89),
            Self::Repeat => (211, 89),
            Self::EqToggle => (219, 58),
            Self::PlToggle => (242, 58),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        match self {
            Self::Previous | Self::Play | Self::Pause | Self::Stop => (23, 18),
            Self::Next => (22, 18),
            Self::Eject => (22, 16),
            Self::Shuffle => (47, 15),
            Self::Repeat => (28, 15),
            Self::EqToggle | Self::PlToggle => (23, 12),
        }
    }

    /// Which atlas the sprite lives in. cbuttons.bmp for transport, shufrep.bmp
    /// for the four toggle buttons.
    pub fn source_atlas(&self) -> SkinComponent {
        match self {
            Self::Shuffle | Self::Repeat | Self::EqToggle | Self::PlToggle => {
                SkinComponent::Shufrep
            }
            _ => SkinComponent::CButtons,
        }
    }

    /// Sprite coords for the current state. `is_on` only matters for toggle
    /// buttons (Shuffle/Repeat/EqToggle/PlToggle); ignored otherwise.
    pub fn sprite_coords(&self, state: ButtonState, is_on: bool) -> (u32, u32) {
        match self {
            Self::Previous | Self::Play | Self::Pause | Self::Stop | Self::Next | Self::Eject => {
                let base_x = match self {
                    Self::Previous => 0,
                    Self::Play => 23,
                    Self::Pause => 46,
                    Self::Stop => 69,
                    Self::Next => 92,
                    Self::Eject => 114,
                    _ => unreachable!(),
                };
                let y_offset = match state {
                    ButtonState::Pressed => 18,
                    _ => 0,
                };
                (base_x, y_offset)
            }
            // Repeat: 4 sprites stacked at x=0, y=0/15/30/45 (off-rel, off-pr, on-rel, on-pr).
            Self::Repeat => (0, sprite_row_y(state, is_on)),
            // Shuffle: same Y offsets but x=28.
            Self::Shuffle => (28, sprite_row_y(state, is_on)),
            // EqToggle: off-released (0,61), off-pressed (46,61), on-released (0,73), on-pressed (46,73).
            Self::EqToggle => toggle_12px_coords(0, 46, state, is_on),
            // PlToggle: off-released (23,61), off-pressed (69,61), on-released (23,73), on-pressed (69,73).
            Self::PlToggle => toggle_12px_coords(23, 69, state, is_on),
        }
    }
}

/// y-offset for the 4-row shufrep layout (off-rel/off-pr/on-rel/on-pr stacked).
fn sprite_row_y(state: ButtonState, is_on: bool) -> u32 {
    let pressed = matches!(state, ButtonState::Pressed);
    match (is_on, pressed) {
        (false, false) => 0,
        (false, true) => 15,
        (true, false) => 30,
        (true, true) => 45,
    }
}

/// (x, y) for the 12 px toggle buttons (EQ/PL): two columns (off/on x = released_x
/// vs pressed_x) and two rows (released y=61, pressed y=73).
fn toggle_12px_coords(
    released_x: u32,
    pressed_x: u32,
    state: ButtonState,
    is_on: bool,
) -> (u32, u32) {
    let pressed = matches!(state, ButtonState::Pressed);
    let x = if pressed { pressed_x } else { released_x };
    let y = if is_on { 73 } else { 61 };
    (x, y)
}

pub struct ButtonComponent {
    pub button_type: WinampButton,
    pub state: ButtonState,
    pub enabled: bool,
    /// Only meaningful for toggle buttons (Shuffle/Repeat/EqToggle/PlToggle).
    pub is_on: bool,
}

impl ButtonComponent {
    pub fn new(button_type: WinampButton) -> Self {
        Self {
            button_type,
            state: ButtonState::Normal,
            enabled: true,
            is_on: false,
        }
    }

    pub fn is_hovered(&self, mouse_pos: Pos2, window_offset: Pos2, scale: f32) -> bool {
        if !self.enabled {
            return false;
        }

        let (btn_x, btn_y) = self.button_type.position();
        let (btn_w, btn_h) = self.button_type.size();

        let btn_rect_min_x = window_offset.x + btn_x as f32 * scale;
        let btn_rect_min_y = window_offset.y + btn_y as f32 * scale;
        let btn_rect_max_x = btn_rect_min_x + btn_w as f32 * scale;
        let btn_rect_max_y = btn_rect_min_y + btn_h as f32 * scale;

        mouse_pos.x >= btn_rect_min_x
            && mouse_pos.x <= btn_rect_max_x
            && mouse_pos.y >= btn_rect_min_y
            && mouse_pos.y <= btn_rect_max_y
    }

    pub fn update_state(&mut self, is_hovered: bool, is_pressed: bool) {
        if !self.enabled {
            self.state = ButtonState::Disabled;
            return;
        }

        self.state = if is_pressed && is_hovered {
            ButtonState::Pressed
        } else {
            ButtonState::Normal
        };
    }

    pub fn get_current_sprite_coords(&self) -> (u32, u32) {
        self.button_type.sprite_coords(self.state, self.is_on)
    }
}

pub struct ButtonManager {
    buttons: Vec<ButtonComponent>,
}

impl ButtonManager {
    pub fn new() -> Self {
        Self {
            buttons: vec![
                ButtonComponent::new(WinampButton::Previous),
                ButtonComponent::new(WinampButton::Play),
                ButtonComponent::new(WinampButton::Pause),
                ButtonComponent::new(WinampButton::Stop),
                ButtonComponent::new(WinampButton::Next),
                ButtonComponent::new(WinampButton::Eject),
                ButtonComponent::new(WinampButton::Shuffle),
                ButtonComponent::new(WinampButton::Repeat),
                ButtonComponent::new(WinampButton::EqToggle),
                ButtonComponent::new(WinampButton::PlToggle),
            ],
        }
    }

    pub fn buttons(&self) -> &[ButtonComponent] {
        &self.buttons
    }

    pub fn set_toggle(&mut self, button: WinampButton, on: bool) {
        if let Some(b) = self.buttons.iter_mut().find(|b| b.button_type == button) {
            b.is_on = on;
        }
    }

    pub fn update_all(
        &mut self,
        mouse_pos: Option<Pos2>,
        is_pressed: bool,
        window_offset: Pos2,
        scale: f32,
    ) {
        for button in &mut self.buttons {
            if let Some(pos) = mouse_pos {
                let is_hovered = button.is_hovered(pos, window_offset, scale);
                button.update_state(is_hovered, is_pressed);
            } else {
                button.update_state(false, false);
            }
        }
    }

    pub fn check_click(
        &self,
        mouse_pos: Pos2,
        window_offset: Pos2,
        scale: f32,
    ) -> Option<WinampButton> {
        for button in &self.buttons {
            if button.is_hovered(mouse_pos, window_offset, scale) {
                return Some(button.button_type);
            }
        }
        None
    }
}

impl Default for ButtonManager {
    fn default() -> Self {
        Self::new()
    }
}
