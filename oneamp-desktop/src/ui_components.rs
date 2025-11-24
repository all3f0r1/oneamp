use eframe::egui;
use crate::theme::Theme;
use crate::track_display::TrackDisplay;
use oneamp_core::TrackInfo;

/// Render the player section (timer, track info, visualizer)
pub fn render_player_section(
    ui: &mut egui::Ui,
    theme: &Theme,
    current_track: &Option<TrackInfo>,
    current_position: f32,
    _total_duration: f32,
    visualizer_data: &[f32],
    scroll_offset: &mut usize,
) {
    let player_height = theme.layout.player_height;
    
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), player_height),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(10.0);
            
            // Timer display (large digital style)
            let timer_text = TrackDisplay::format_duration_digital(current_position);
            ui.label(
                egui::RichText::new(timer_text)
                    .size(theme.fonts.timer_size)
                    .color(Theme::color32(&theme.colors.display_text))
                    .monospace()
            );
            
            ui.add_space(8.0);
            
            // Track info (scrolling text)
            if let Some(track) = current_track {
                let title = TrackDisplay::get_title(track);
                let max_width = 40; // characters
                let scrolled = TrackDisplay::scroll_text(&title, max_width, *scroll_offset);
                
                ui.label(
                    egui::RichText::new(scrolled)
                        .size(theme.fonts.track_info_size)
                        .color(Theme::color32(&theme.colors.display_accent))
                        .monospace()
                );
                
                // Update scroll offset for animation
                *scroll_offset = (*scroll_offset + 1) % (title.len() + 3).max(1);
                
                ui.add_space(4.0);
                
                // Technical info
                let tech_info = TrackDisplay::get_technical_info(track);
                ui.label(
                    egui::RichText::new(tech_info)
                        .size(11.0)
                        .color(Theme::color32(&theme.colors.display_text).linear_multiply(0.7))
                );
            } else {
                ui.label(
                    egui::RichText::new("No track loaded")
                        .size(theme.fonts.track_info_size)
                        .color(Theme::color32(&theme.colors.display_text).linear_multiply(0.5))
                );
            }
            
            ui.add_space(8.0);
            
            // Simple visualizer
            render_visualizer(ui, theme, visualizer_data);
        },
    );
}

/// Render a simple spectrum visualizer
fn render_visualizer(ui: &mut egui::Ui, theme: &Theme, data: &[f32]) {
    let height = 40.0;
    let width = ui.available_width().min(300.0);
    
    let (response, painter) = ui.allocate_painter(
        egui::vec2(width, height),
        egui::Sense::hover(),
    );
    
    let rect = response.rect;
    let bar_count = 32.min(data.len());
    let bar_width = rect.width() / bar_count as f32;
    
    for i in 0..bar_count {
        let value = if i < data.len() {
            data[i].abs().min(1.0)
        } else {
            0.0
        };
        
        let bar_height = value * rect.height();
        let x = rect.left() + i as f32 * bar_width;
        let y = rect.bottom() - bar_height;
        
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(bar_width * 0.8, bar_height),
        );
        
        painter.rect_filled(
            bar_rect,
            2.0,
            Theme::color32(&theme.colors.display_accent),
        );
    }
}

/// Render interactive progress bar
pub fn render_progress_bar(
    ui: &mut egui::Ui,
    theme: &Theme,
    current_position: f32,
    total_duration: f32,
) -> Option<f32> {
    let mut seek_to = None;
    
    ui.horizontal(|ui| {
        // Time elapsed
        ui.label(
            egui::RichText::new(TrackDisplay::format_duration(current_position))
                .size(12.0)
                .monospace()
        );
        
        // Progress bar
        let progress = if total_duration > 0.0 {
            current_position / total_duration
        } else {
            0.0
        };
        
        let desired_width = ui.available_width() - 60.0;
        let (response, painter) = ui.allocate_painter(
            egui::vec2(desired_width, 20.0),
            egui::Sense::click_and_drag(),
        );
        
        let rect = response.rect;
        
        // Background
        painter.rect_filled(
            rect,
            4.0,
            Theme::color32(&theme.colors.progress_bg),
        );
        
        // Fill
        let fill_width = rect.width() * progress;
        let fill_rect = egui::Rect::from_min_size(
            rect.min,
            egui::vec2(fill_width, rect.height()),
        );
        painter.rect_filled(
            fill_rect,
            4.0,
            Theme::color32(&theme.colors.progress_fill),
        );
        
        // Handle click/drag to seek
        if response.clicked() || response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let x = (pos.x - rect.left()) / rect.width();
                let new_position = (x.clamp(0.0, 1.0) * total_duration).max(0.0);
                seek_to = Some(new_position);
            }
        }
        
        // Time remaining
        ui.label(
            egui::RichText::new(TrackDisplay::format_duration(total_duration))
                .size(12.0)
                .monospace()
        );
    });
    
    seek_to
}

/// Render playback control buttons
pub struct ControlButtons {
    pub previous: bool,
    pub play_pause: bool,
    pub stop: bool,
    pub next: bool,
}

pub fn render_control_buttons(
    ui: &mut egui::Ui,
    is_playing: bool,
    is_paused: bool,
    has_tracks: bool,
) -> ControlButtons {
    let mut result = ControlButtons {
        previous: false,
        play_pause: false,
        stop: false,
        next: false,
    };
    
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        
        // Previous
        if ui.add_enabled(has_tracks, egui::Button::new("⏮")).clicked() {
            result.previous = true;
        }
        
        ui.add_space(8.0);
        
        // Play/Pause
        let icon = if is_playing { "⏸" } else { "▶" };
        if ui.button(icon).clicked() {
            result.play_pause = true;
        }
        
        ui.add_space(8.0);
        
        // Stop
        if ui.add_enabled(is_playing || is_paused, egui::Button::new("⏹")).clicked() {
            result.stop = true;
        }
        
        ui.add_space(8.0);
        
        // Next
        if ui.add_enabled(has_tracks, egui::Button::new("⏭")).clicked() {
            result.next = true;
        }
    });
    
    result
}

/// Render equalizer section
pub fn render_equalizer(
    ui: &mut egui::Ui,
    theme: &Theme,
    eq_enabled: &mut bool,
    eq_gains: &mut Vec<f32>,
    eq_frequencies: &[f32],
) -> bool {
    let mut changed = false;
    
    ui.horizontal(|ui| {
        ui.label("Equalizer");
        if ui.checkbox(eq_enabled, "Enabled").changed() {
            changed = true;
        }
        
        if ui.button("Reset").clicked() {
            for gain in eq_gains.iter_mut() {
                *gain = 0.0;
            }
            changed = true;
        }
    });
    
    ui.add_space(8.0);
    
    ui.horizontal(|ui| {
        let slider_width = (ui.available_width() / eq_gains.len() as f32).min(50.0);
        
        for (i, gain) in eq_gains.iter_mut().enumerate() {
            ui.vertical(|ui| {
                ui.set_width(slider_width);
                
                // Frequency label
                let freq_label = if eq_frequencies[i] >= 1000.0 {
                    format!("{}k", eq_frequencies[i] as u32 / 1000)
                } else {
                    format!("{}", eq_frequencies[i] as u32)
                };
                
                ui.label(
                    egui::RichText::new(freq_label)
                        .size(10.0)
                );
                
                // Vertical slider
                if ui.add(
                    egui::Slider::new(gain, -12.0..=12.0)
                        .vertical()
                        .show_value(false)
                ).changed() {
                    changed = true;
                }
                
                // Gain value
                ui.label(
                    egui::RichText::new(format!("{:+.1}", gain))
                        .size(9.0)
                        .monospace()
                );
            });
        }
    });
    
    changed
}

/// Render playlist section with drag-drop support
pub struct PlaylistActions {
    pub play_track: Option<usize>,
    pub select_track: Option<usize>,
}

pub fn render_playlist(
    ui: &mut egui::Ui,
    _theme: &Theme,
    playlist: &[std::path::PathBuf],
    current_track_index: Option<usize>,
    selected_track_index: Option<usize>,
) -> PlaylistActions {
    let mut actions = PlaylistActions {
        play_track: None,
        select_track: None,
    };
    
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            if playlist.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Drag and drop audio files here")
                            .size(14.0)
                            .color(Theme::color32(&theme.colors.playlist_text).linear_multiply(0.5))
                    );
                });
            } else {
                for (idx, path) in playlist.iter().enumerate() {
                    // Try to get track info for display
                    let display_text = if let Ok(track_info) = oneamp_core::TrackInfo::from_file(path) {
                        TrackDisplay::get_title(&track_info)
                    } else {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("Unknown")
                            .to_string()
                    };
                    
                    let is_current = current_track_index == Some(idx);
                    let is_selected = selected_track_index == Some(idx);
                    
                    let mut text = egui::RichText::new(display_text)
                        .size(theme.fonts.playlist_size);
                    
                    if is_current {
                        text = text.color(Theme::color32(&theme.colors.playlist_playing));
                    }
                    
                    let response = ui.selectable_label(is_selected, text);
                    
                    if response.clicked() {
                        actions.select_track = Some(idx);
                    }
                    
                    if response.double_clicked() {
                        actions.play_track = Some(idx);
                    }
                }
            }
        });
    
    actions
}
