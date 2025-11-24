use eframe::egui;
use oneamp_core::{AudioCommand, AudioEngine, AudioEvent, TrackInfo};
use std::path::PathBuf;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 400.0])
            .with_min_inner_size([400.0, 300.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(&include_bytes!("../../../architecture.png")[..])
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    
    eframe::run_native(
        "OneAmp",
        options,
        Box::new(|cc| {
            // Set custom style
            setup_custom_style(&cc.egui_ctx);
            Ok(Box::new(OneAmpApp::new()))
        }),
    )
}

/// Setup custom visual style inspired by Winamp Modern theme
fn setup_custom_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    
    // Dark theme colors
    style.visuals.dark_mode = true;
    style.visuals.override_text_color = Some(egui::Color32::from_rgb(220, 220, 220));
    style.visuals.window_fill = egui::Color32::from_rgb(30, 30, 35);
    style.visuals.panel_fill = egui::Color32::from_rgb(25, 25, 30);
    
    // Button colors
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(50, 50, 60);
    style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(70, 70, 80);
    style.visuals.widgets.active.weak_bg_fill = egui::Color32::from_rgb(90, 90, 100);
    
    // Accent color (cyan-ish, inspired by Winamp)
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(0, 150, 200);
    
    ctx.set_style(style);
}

struct OneAmpApp {
    audio_engine: Option<AudioEngine>,
    current_track: Option<TrackInfo>,
    playback_state: PlaybackState,
    current_position: f32,
    total_duration: f32,
    error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

impl Default for OneAmpApp {
    fn default() -> Self {
        Self::new()
    }
}

impl OneAmpApp {
    fn new() -> Self {
        let audio_engine = match AudioEngine::new() {
            Ok(engine) => Some(engine),
            Err(e) => {
                eprintln!("Failed to initialize audio engine: {}", e);
                None
            }
        };
        
        Self {
            audio_engine,
            current_track: None,
            playback_state: PlaybackState::Stopped,
            current_position: 0.0,
            total_duration: 0.0,
            error_message: None,
        }
    }
    
    fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio Files", &["mp3", "flac"])
            .pick_file()
        {
            self.play_file(path);
        }
    }
    
    fn play_file(&mut self, path: PathBuf) {
        if let Some(ref engine) = self.audio_engine {
            if let Err(e) = engine.send_command(AudioCommand::Play(path.clone())) {
                self.error_message = Some(format!("Failed to play file: {}", e));
            }
        }
    }
    
    fn toggle_play_pause(&mut self) {
        if let Some(ref engine) = self.audio_engine {
            let cmd = match self.playback_state {
                PlaybackState::Playing => AudioCommand::Pause,
                PlaybackState::Paused => AudioCommand::Resume,
                PlaybackState::Stopped => return, // Can't resume from stopped
            };
            
            if let Err(e) = engine.send_command(cmd) {
                self.error_message = Some(format!("Failed to toggle playback: {}", e));
            }
        }
    }
    
    fn stop(&mut self) {
        if let Some(ref engine) = self.audio_engine {
            if let Err(e) = engine.send_command(AudioCommand::Stop) {
                self.error_message = Some(format!("Failed to stop playback: {}", e));
            }
        }
    }
    
    fn process_audio_events(&mut self) {
        if let Some(ref engine) = self.audio_engine {
            while let Some(event) = engine.try_recv_event() {
                match event {
                    AudioEvent::TrackLoaded(track_info) => {
                        self.current_track = Some(track_info.clone());
                        self.total_duration = track_info.duration_secs.unwrap_or(0.0);
                        self.current_position = 0.0;
                        self.error_message = None;
                    }
                    AudioEvent::Playing => {
                        self.playback_state = PlaybackState::Playing;
                    }
                    AudioEvent::Paused => {
                        self.playback_state = PlaybackState::Paused;
                    }
                    AudioEvent::Stopped => {
                        self.playback_state = PlaybackState::Stopped;
                        self.current_position = 0.0;
                    }
                    AudioEvent::Position(current, total) => {
                        self.current_position = current;
                        self.total_duration = total;
                    }
                    AudioEvent::Finished => {
                        self.playback_state = PlaybackState::Stopped;
                        self.current_position = 0.0;
                    }
                    AudioEvent::Error(msg) => {
                        self.error_message = Some(msg);
                        self.playback_state = PlaybackState::Stopped;
                    }
                }
            }
        }
    }
}

impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process audio events
        self.process_audio_events();
        
        // Request continuous repaint for smooth progress bar updates
        ctx.request_repaint();
        
        // Top panel with title
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("🎧 OneAmp");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                });
            });
            ui.add_space(4.0);
        });
        
        // Bottom panel with controls
        egui::TopBottomPanel::bottom("controls_panel").show(ctx, |ui| {
            ui.add_space(8.0);
            
            // Progress bar
            ui.horizontal(|ui| {
                ui.label(format_time(self.current_position));
                
                let progress = if self.total_duration > 0.0 {
                    self.current_position / self.total_duration
                } else {
                    0.0
                };
                
                let progress_bar = egui::ProgressBar::new(progress)
                    .show_percentage()
                    .animate(self.playback_state == PlaybackState::Playing);
                
                ui.add(progress_bar);
                ui.label(format_time(self.total_duration));
            });
            
            ui.add_space(8.0);
            
            // Control buttons
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                
                if ui.button("📂 Open File").clicked() {
                    self.open_file();
                }
                
                ui.add_space(16.0);
                
                // Play/Pause button
                let play_pause_text = match self.playback_state {
                    PlaybackState::Playing => "⏸ Pause",
                    _ => "▶ Play",
                };
                
                let play_pause_enabled = self.playback_state != PlaybackState::Stopped;
                
                if ui.add_enabled(play_pause_enabled, egui::Button::new(play_pause_text)).clicked() {
                    self.toggle_play_pause();
                }
                
                ui.add_space(8.0);
                
                // Stop button
                let stop_enabled = self.playback_state != PlaybackState::Stopped;
                if ui.add_enabled(stop_enabled, egui::Button::new("⏹ Stop")).clicked() {
                    self.stop();
                }
            });
            
            ui.add_space(8.0);
        });
        
        // Central panel with track info
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                
                if let Some(ref track) = self.current_track {
                    // Track title
                    ui.heading(track.title.as_deref().unwrap_or("Unknown Title"));
                    ui.add_space(8.0);
                    
                    // Artist
                    ui.label(
                        egui::RichText::new(track.artist.as_deref().unwrap_or("Unknown Artist"))
                            .size(16.0)
                            .color(egui::Color32::from_rgb(180, 180, 180))
                    );
                    ui.add_space(4.0);
                    
                    // Album
                    if let Some(ref album) = track.album {
                        ui.label(
                            egui::RichText::new(album)
                                .size(14.0)
                                .color(egui::Color32::from_rgb(150, 150, 150))
                        );
                    }
                    
                    ui.add_space(20.0);
                    
                    // Technical info
                    ui.group(|ui| {
                        ui.set_min_width(300.0);
                        egui::Grid::new("track_info_grid")
                            .num_columns(2)
                            .spacing([40.0, 8.0])
                            .show(ui, |ui| {
                                if let Some(sr) = track.sample_rate {
                                    ui.label("Sample Rate:");
                                    ui.label(format!("{} Hz", sr));
                                    ui.end_row();
                                }
                                
                                if let Some(ch) = track.channels {
                                    ui.label("Channels:");
                                    ui.label(format!("{}", ch));
                                    ui.end_row();
                                }
                                
                                ui.label("Format:");
                                ui.label(
                                    track.path.extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("Unknown")
                                        .to_uppercase()
                                );
                                ui.end_row();
                            });
                    });
                } else {
                    ui.add_space(60.0);
                    ui.label(
                        egui::RichText::new("No track loaded")
                            .size(18.0)
                            .color(egui::Color32::from_rgb(120, 120, 120))
                    );
                    ui.add_space(12.0);
                    ui.label("Click 'Open File' to load an audio file");
                }
                
                // Error message
                if let Some(ref error) = self.error_message {
                    ui.add_space(20.0);
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), format!("⚠ {}", error));
                }
            });
        });
    }
    
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(engine) = self.audio_engine.take() {
            let _ = engine.shutdown();
        }
    }
}

/// Format seconds as MM:SS
fn format_time(secs: f32) -> String {
    let total_secs = secs as u32;
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}", minutes, seconds)
}
