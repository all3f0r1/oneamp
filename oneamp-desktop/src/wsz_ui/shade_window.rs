use super::components::buttons::WinampButton;
use super::components::display::DigitalDisplay;
use super::components::sliders::VolumeSlider;
use super::renderer::WszRenderer;
use egui::{Context, Pos2, Vec2};
use oneamp_core::wsz::skin::{SkinComponent, WszSkin};
use oneamp_core::{AudioCommand, AudioEngine, AudioEvent};

pub struct ShadeWindow {
    renderer: WszRenderer,
    volume_slider: VolumeSlider,
    display: DigitalDisplay,
    is_playing: bool,
    is_paused: bool,
    mouse_pressed: bool,
}

impl ShadeWindow {
    pub fn new(skin: WszSkin, scale: f32) -> Self {
        Self {
            renderer: WszRenderer::new(skin, scale),
            volume_slider: VolumeSlider::new(),
            display: DigitalDisplay::new(),
            is_playing: false,
            is_paused: false,
            mouse_pressed: false,
        }
    }

    pub fn show(&mut self, ctx: &Context, audio_engine: Option<&AudioEngine>) {
        let scale = self.renderer.get_scale();
        let window_size = Vec2::new(275.0 * scale, 14.0 * scale);

        egui::Window::new("OneAmp - Shade")
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .fixed_size(window_size)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let window_rect = ui.available_rect_before_wrap();
                let window_offset = window_rect.min;

                self.render_background(ui, window_offset);
                self.render_display(ui, window_offset);
                self.render_buttons(ui, window_offset);
                self.render_volume(ui, window_offset);
                self.handle_input(ui, window_offset, audio_engine);
            });
    }

    fn render_background(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        // Faithful path: blit the 275×14 WindowShade band that Winamp
        // bakes into titlebar.bmp at skin-space y=29. The band already
        // carries the title-text recess, the mini position groove and
        // the right-hand titlebar buttons, so a real skin's shade strip
        // looks exactly like Winamp's instead of a flat grey bar.
        //
        // The `region.txt [WindowShade]` mask that Winamp also applies
        // is *window-shaping* metadata — it only matters once the shade
        // window is its own OS viewport (the floating-windows case).
        // Embedded in the single viewport
        // it's a no-op, so we don't apply it here yet.
        let band = self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::TitleBar)
            .and_then(|atlas| atlas.extract_region(0, 29, 275, 14));
        if let Some(region) = band {
            let pos = self.renderer.skin_to_screen(0, 0, offset);
            self.renderer.render_region(ui, &region, pos, "shade_band");
        } else {
            // Synthetic / truncated skins lack the band — fall back to
            // the historical flat strip so shade mode still renders.
            let scale = self.renderer.get_scale();
            ui.painter().rect_filled(
                egui::Rect::from_min_size(offset, Vec2::new(275.0 * scale, 14.0 * scale)),
                0.0,
                egui::Color32::from_rgb(40, 40, 40),
            );
        }
    }

    fn render_display(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let scale = self.renderer.get_scale();
        // Mini time readout, drawn from nums.bmp (4 digits + optional
        // leading minus glyph at sprite index 10) so it matches the
        // main window's LCD instead of a proportional font. Falls back
        // to proportional text when the skin has no numbers atlas.
        let display_x = 4u32;
        let display_y = 3u32;
        let digit_w = 9u32;
        let digit_h = 13u32;
        let time = self.display.time_digits();

        let atlas = self
            .renderer
            .get_skin()
            .get_bitmap(&SkinComponent::Numbers)
            .cloned();
        let Some(atlas) = atlas else {
            ui.painter().text(
                offset + Vec2::new(display_x as f32 * scale, display_y as f32 * scale),
                egui::Align2::LEFT_TOP,
                self.display.format_time(),
                egui::FontId::proportional(8.0 * scale),
                egui::Color32::from_rgb(0, 255, 0),
            );
            return;
        };

        const SLOT_OFFSETS: [u32; 4] = [0, 9, 21, 30];
        const MINUS_SPRITE_INDEX: u32 = 10;
        for (i, &d) in time.digits.iter().enumerate() {
            let sprite_index = if i == 0 && time.minus {
                MINUS_SPRITE_INDEX
            } else {
                u32::from(d)
            };
            if let Some(region) = atlas.extract_region(sprite_index * digit_w, 0, digit_w, digit_h)
            {
                let pos =
                    self.renderer
                        .skin_to_screen(display_x + SLOT_OFFSETS[i], display_y, offset);
                self.renderer
                    .render_region(ui, &region, pos, &format!("shade_digit_{}", i));
            }
        }
    }

    fn render_buttons(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let scale = self.renderer.get_scale();
        let button_positions = [
            (80.0, 2.0, "◀◀"),
            (95.0, 2.0, "▶"),
            (110.0, 2.0, "⏸"),
            (125.0, 2.0, "■"),
            (140.0, 2.0, "▶▶"),
        ];

        for (x, y, symbol) in button_positions.iter() {
            let pos = offset + Vec2::new(x * scale, y * scale);
            ui.painter().text(
                pos,
                egui::Align2::LEFT_TOP,
                symbol,
                egui::FontId::proportional(10.0 * scale),
                egui::Color32::WHITE,
            );
        }
    }

    fn render_volume(&mut self, ui: &mut egui::Ui, offset: Pos2) {
        let scale = self.renderer.get_scale();
        let vol_x = 200.0;
        let vol_y = 5.0;

        let bar_width = 50.0 * scale;
        let bar_height = 4.0 * scale;
        let filled_width = bar_width * self.volume_slider.value;

        let pos = offset + Vec2::new(vol_x * scale, vol_y * scale);

        ui.painter().rect_filled(
            egui::Rect::from_min_size(pos, Vec2::new(bar_width, bar_height)),
            0.0,
            egui::Color32::from_rgb(60, 60, 60),
        );

        ui.painter().rect_filled(
            egui::Rect::from_min_size(pos, Vec2::new(filled_width, bar_height)),
            0.0,
            egui::Color32::from_rgb(0, 255, 0),
        );
    }

    fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        offset: Pos2,
        audio_engine: Option<&AudioEngine>,
    ) {
        let scale = self.renderer.get_scale();

        if let Some(mouse_pos) = ui.ctx().pointer_latest_pos() {
            let is_pressed = ui.ctx().input(|i| i.pointer.primary_down());

            if is_pressed && !self.mouse_pressed {
                let button_rects = [
                    (
                        egui::Rect::from_min_size(
                            offset + Vec2::new(80.0 * scale, 2.0 * scale),
                            Vec2::new(12.0 * scale, 10.0 * scale),
                        ),
                        WinampButton::Previous,
                    ),
                    (
                        egui::Rect::from_min_size(
                            offset + Vec2::new(95.0 * scale, 2.0 * scale),
                            Vec2::new(12.0 * scale, 10.0 * scale),
                        ),
                        WinampButton::Play,
                    ),
                    (
                        egui::Rect::from_min_size(
                            offset + Vec2::new(110.0 * scale, 2.0 * scale),
                            Vec2::new(12.0 * scale, 10.0 * scale),
                        ),
                        WinampButton::Pause,
                    ),
                    (
                        egui::Rect::from_min_size(
                            offset + Vec2::new(125.0 * scale, 2.0 * scale),
                            Vec2::new(12.0 * scale, 10.0 * scale),
                        ),
                        WinampButton::Stop,
                    ),
                    (
                        egui::Rect::from_min_size(
                            offset + Vec2::new(140.0 * scale, 2.0 * scale),
                            Vec2::new(12.0 * scale, 10.0 * scale),
                        ),
                        WinampButton::Next,
                    ),
                ];

                for (rect, button) in button_rects.iter() {
                    if rect.contains(mouse_pos) {
                        self.handle_button_click(*button, audio_engine);
                        break;
                    }
                }

                let vol_rect = egui::Rect::from_min_size(
                    offset + Vec2::new(200.0 * scale, 5.0 * scale),
                    Vec2::new(50.0 * scale, 4.0 * scale),
                );

                if vol_rect.contains(mouse_pos) {
                    self.volume_slider.is_dragging = true;
                }
            }

            if !is_pressed {
                self.volume_slider.is_dragging = false;
            }

            if self.volume_slider.is_dragging {
                let vol_rect = egui::Rect::from_min_size(
                    offset + Vec2::new(200.0 * scale, 5.0 * scale),
                    Vec2::new(50.0 * scale, 4.0 * scale),
                );

                let local_x = (mouse_pos.x - vol_rect.min.x).clamp(0.0, vol_rect.width());
                self.volume_slider.value = local_x / vol_rect.width();

                if let Some(engine) = audio_engine {
                    let _ = engine.send_command(AudioCommand::SetVolume(self.volume_slider.value));
                }
            }

            self.mouse_pressed = is_pressed;
        }
    }

    fn handle_button_click(&mut self, button: WinampButton, audio_engine: Option<&AudioEngine>) {
        let Some(engine) = audio_engine else {
            return;
        };

        let _ = match button {
            WinampButton::Play => {
                if self.is_paused {
                    engine.send_command(AudioCommand::Resume)
                } else {
                    Ok(())
                }
            }
            WinampButton::Pause => engine.send_command(AudioCommand::Pause),
            WinampButton::Stop => engine.send_command(AudioCommand::Stop),
            WinampButton::Next => engine.send_command(AudioCommand::Next),
            WinampButton::Previous => engine.send_command(AudioCommand::Previous),
            _ => Ok(()),
        };
    }

    pub fn update(&mut self, events: &[AudioEvent]) {
        for event in events {
            match event {
                AudioEvent::Playing => {
                    self.is_playing = true;
                    self.is_paused = false;
                }
                AudioEvent::Paused => {
                    self.is_paused = true;
                }
                AudioEvent::Stopped => {
                    self.is_playing = false;
                    self.is_paused = false;
                }
                AudioEvent::Position(current, total) => {
                    self.display.set_time(*current, *total);
                }
                AudioEvent::VolumeUpdated(vol, _) if !self.volume_slider.is_dragging => {
                    self.volume_slider.set_value(*vol);
                }
                _ => {}
            }
        }
    }
}
