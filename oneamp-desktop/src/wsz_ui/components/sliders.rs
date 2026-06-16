use egui::Pos2;

pub struct PositionSlider {
    position: (u32, u32),
    width: u32,
    pub progress: f32,
    pub is_dragging: bool,
    /// Set to `Some(progress)` for one frame after the user releases the
    /// thumb so the caller can fire a single seek command. Sending Seek
    /// on every drag tick (60×/s) overwhelms the audio thread and clears
    /// the output buffer faster than playback can refill it — playback
    /// stalls until the user lets go. Drag-end semantics fix that.
    pending_seek: Option<f32>,
}

impl PositionSlider {
    pub fn new() -> Self {
        Self {
            position: (16, 72),
            width: 248,
            progress: 0.0,
            is_dragging: false,
            pending_seek: None,
        }
    }

    pub fn position(&self) -> (u32, u32) {
        self.position
    }

    pub fn height(&self) -> u32 {
        10
    }

    pub fn set_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    pub fn is_hovered(&self, mouse_pos: Pos2, window_offset: Pos2, scale: f32) -> bool {
        let slider_x = window_offset.x + self.position.0 as f32 * scale;
        let slider_y = window_offset.y + self.position.1 as f32 * scale;
        let slider_w = self.width as f32 * scale;
        let slider_h = self.height() as f32 * scale;

        mouse_pos.x >= slider_x
            && mouse_pos.x <= slider_x + slider_w
            && mouse_pos.y >= slider_y
            && mouse_pos.y <= slider_y + slider_h
    }

    /// Update visual progress while dragging. Does NOT emit a seek command —
    /// call `take_pending_seek` after `set_dragging(false)` to get the final
    /// position once the user releases.
    pub fn handle_drag(&mut self, mouse_pos: Pos2, window_offset: Pos2, scale: f32) {
        if !self.is_dragging {
            return;
        }

        let slider_x = window_offset.x + self.position.0 as f32 * scale;
        let slider_w = self.width as f32 * scale;

        let local_x = (mouse_pos.x - slider_x).clamp(0.0, slider_w);
        self.progress = local_x / slider_w;
    }

    /// Mark the end of a drag so `take_pending_seek` returns the new
    /// progress on the next call. No-op if no drag was in progress.
    pub fn end_drag(&mut self) {
        if self.is_dragging {
            self.is_dragging = false;
            self.pending_seek = Some(self.progress);
        }
    }

    pub fn take_pending_seek(&mut self) -> Option<f32> {
        self.pending_seek.take()
    }
}

impl Default for PositionSlider {
    fn default() -> Self {
        Self::new()
    }
}

pub struct VolumeSlider {
    position: (u32, u32),
    pub value: f32,
    pub is_dragging: bool,
}

impl VolumeSlider {
    pub fn new() -> Self {
        Self {
            position: (107, 57),
            value: 1.0,
            is_dragging: false,
        }
    }

    pub fn position(&self) -> (u32, u32) {
        self.position
    }

    pub fn size(&self) -> (u32, u32) {
        (68, 13)
    }

    pub fn set_value(&mut self, value: f32) {
        self.value = value.clamp(0.0, 1.0);
    }

    pub fn is_hovered(&self, mouse_pos: Pos2, window_offset: Pos2, scale: f32) -> bool {
        let (w, h) = self.size();
        let slider_x = window_offset.x + self.position.0 as f32 * scale;
        let slider_y = window_offset.y + self.position.1 as f32 * scale;

        mouse_pos.x >= slider_x
            && mouse_pos.x <= slider_x + w as f32 * scale
            && mouse_pos.y >= slider_y
            && mouse_pos.y <= slider_y + h as f32 * scale
    }

    pub fn handle_drag(&mut self, mouse_pos: Pos2, window_offset: Pos2, scale: f32) -> Option<f32> {
        if !self.is_dragging {
            return None;
        }

        let (w, _) = self.size();
        let slider_x = window_offset.x + self.position.0 as f32 * scale;
        let slider_w = w as f32 * scale;

        let local_x = (mouse_pos.x - slider_x).clamp(0.0, slider_w);
        self.value = local_x / slider_w;

        Some(self.value)
    }

    pub fn get_frame(&self) -> u32 {
        (self.value * 27.0).round() as u32
    }
}

impl Default for VolumeSlider {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BalanceSlider {
    position: (u32, u32),
    pub value: f32,
    pub is_dragging: bool,
}

impl BalanceSlider {
    pub fn new() -> Self {
        Self {
            position: (177, 57),
            value: 0.0,
            is_dragging: false,
        }
    }

    pub fn position(&self) -> (u32, u32) {
        self.position
    }

    pub fn size(&self) -> (u32, u32) {
        (38, 13)
    }

    pub fn set_value(&mut self, value: f32) {
        self.value = value.clamp(-1.0, 1.0);
    }

    pub fn is_hovered(&self, mouse_pos: Pos2, window_offset: Pos2, scale: f32) -> bool {
        let (w, h) = self.size();
        let slider_x = window_offset.x + self.position.0 as f32 * scale;
        let slider_y = window_offset.y + self.position.1 as f32 * scale;

        mouse_pos.x >= slider_x
            && mouse_pos.x <= slider_x + w as f32 * scale
            && mouse_pos.y >= slider_y
            && mouse_pos.y <= slider_y + h as f32 * scale
    }

    pub fn handle_drag(&mut self, mouse_pos: Pos2, window_offset: Pos2, scale: f32) -> Option<f32> {
        if !self.is_dragging {
            return None;
        }

        let (w, _) = self.size();
        let slider_x = window_offset.x + self.position.0 as f32 * scale;
        let slider_w = w as f32 * scale;

        let local_x = (mouse_pos.x - slider_x).clamp(0.0, slider_w);
        let normalized = local_x / slider_w;
        self.value = (normalized - 0.5) * 2.0;

        Some(self.value)
    }

    pub fn get_frame(&self) -> u32 {
        let normalized = (self.value + 1.0) / 2.0;
        (normalized * 27.0).round() as u32
    }
}

impl Default for BalanceSlider {
    fn default() -> Self {
        Self::new()
    }
}
