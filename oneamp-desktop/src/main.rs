use eframe::egui;
use oneamp_core::{AudioCommand, AudioEngine, AudioEvent, TrackInfo};
use std::path::PathBuf;

mod config;
use config::AppConfig;

mod visualizer;
use visualizer::Visualizer;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 400.0])
            .with_min_inner_size([400.0, 300.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(&include_bytes!("../../architecture.png")[..])
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
    // Playlist management
    playlist: Vec<PathBuf>,
    current_track_index: Option<usize>,
    selected_track_index: Option<usize>,
    // Equalizer
    eq_enabled: bool,
    eq_gains: Vec<f32>,
    eq_frequencies: Vec<f32>,
    show_equalizer: bool,
    // Visualizer
    visualizer: Visualizer,
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
    /// Play the welcome jingle (first run only)
    fn play_jingle(&mut self) {
        // Save jingle to temp file and play it
        const JINGLE_DATA: &[u8] = include_bytes!("../../packaging/jingle.wav");
        
        if let Ok(temp_dir) = std::env::temp_dir().canonicalize() {
            let jingle_path = temp_dir.join("oneamp_jingle.wav");
            
            // Write jingle to temp file
            if std::fs::write(&jingle_path, JINGLE_DATA).is_ok() {
                // Play using the audio engine
                if let Some(engine) = &mut self.audio_engine {
                    let _ = engine.send_command(AudioCommand::Play(jingle_path));
                }
            }
        }
    }
    
    fn new() -> Self {
        let audio_engine = match AudioEngine::new() {
            Ok(engine) => Some(engine),
            Err(e) => {
                eprintln!("Failed to initialize audio engine: {}", e);
                None
            }
        };
        
        // Load configuration
        let (config, is_first_run) = AppConfig::load();
        
        let mut app = Self {
            audio_engine,
            current_track: None,
            playback_state: PlaybackState::Stopped,
            current_position: 0.0,
            total_duration: 0.0,
            error_message: None,
            playlist: Vec::new(),
            current_track_index: None,
            selected_track_index: None,
            eq_enabled: config.equalizer.enabled,
            eq_gains: config.equalizer.gains.clone(),
            eq_frequencies: vec![31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0],
            show_equalizer: false,
            visualizer: Visualizer::new(),
        };
        
        // Apply loaded equalizer settings
        if let Some(ref engine) = app.audio_engine {
            let _ = engine.send_command(AudioCommand::SetEqualizerEnabled(config.equalizer.enabled));
            let _ = engine.send_command(AudioCommand::SetEqualizerBands(config.equalizer.gains));
        }
        
        // Play jingle on first run
        if is_first_run {
            app.play_jingle();
        }
        
        app
    }
    
    fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio Files", &["mp3", "flac", "ogg", "wav"])
            .pick_file()
        {
            self.play_file(path);
        }
    }
    
    fn add_files_to_playlist(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Audio Files", &["mp3", "flac", "ogg", "wav"])
            .pick_files()
        {
            for path in paths {
                if !self.playlist.contains(&path) {
                    self.playlist.push(path);
                }
            }
        }
    }
    
    fn add_folder_to_playlist(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            if let Ok(entries) = std::fs::read_dir(folder) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(ext) = path.extension() {
                            if ["mp3", "flac", "ogg", "wav"].contains(&ext.to_str().unwrap_or(""))
                                && !self.playlist.contains(&path) {
                                self.playlist.push(path);
                            }
                        }
                    }
                }
            }
        }
    }
    
    fn remove_selected_track(&mut self) {
        if let Some(index) = self.selected_track_index {
            if index < self.playlist.len() {
                self.playlist.remove(index);
                // Adjust current track index if needed
                if let Some(current_idx) = self.current_track_index {
                    if current_idx == index {
                        self.current_track_index = None;
                    } else if current_idx > index {
                        self.current_track_index = Some(current_idx - 1);
                    }
                }
                // Adjust selected index
                if index >= self.playlist.len() && !self.playlist.is_empty() {
                    self.selected_track_index = Some(self.playlist.len() - 1);
                } else if self.playlist.is_empty() {
                    self.selected_track_index = None;
                }
            }
        }
    }
    
    fn clear_playlist(&mut self) {
        self.playlist.clear();
        self.current_track_index = None;
        self.selected_track_index = None;
    }
    
    fn sort_playlist_by_title(&mut self) {
        use oneamp_core::TrackInfo;
        
        // Create a vec of (index, title, path)
        let mut tracks: Vec<(usize, String, PathBuf)> = self.playlist
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let title = TrackInfo::from_file(path)
                    .ok()
                    .and_then(|t| t.title)
                    .unwrap_or_else(|| path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string());
                (i, title.to_lowercase(), path.clone())
            })
            .collect();
        
        // Sort by title
        tracks.sort_by(|a, b| a.1.cmp(&b.1));
        
        // Update playlist and indices
        self.playlist = tracks.iter().map(|(_, _, path)| path.clone()).collect();
        self.update_indices_after_sort(&tracks.iter().map(|(i, _, _)| *i).collect::<Vec<_>>());
    }
    
    fn sort_playlist_by_filename(&mut self) {
        let mut tracks: Vec<(usize, String, PathBuf)> = self.playlist
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let filename = path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                (i, filename, path.clone())
            })
            .collect();
        
        tracks.sort_by(|a, b| a.1.cmp(&b.1));
        self.playlist = tracks.iter().map(|(_, _, path)| path.clone()).collect();
        self.update_indices_after_sort(&tracks.iter().map(|(i, _, _)| *i).collect::<Vec<_>>());
    }
    
    fn sort_playlist_by_path(&mut self) {
        let mut tracks: Vec<(usize, PathBuf)> = self.playlist
            .iter()
            .enumerate()
            .map(|(i, path)| (i, path.clone()))
            .collect();
        
        tracks.sort_by(|a, b| a.1.cmp(&b.1));
        self.playlist = tracks.iter().map(|(_, path)| path.clone()).collect();
        self.update_indices_after_sort(&tracks.iter().map(|(i, _)| *i).collect::<Vec<_>>());
    }
    
    fn shuffle_playlist(&mut self) {
        use rand::seq::SliceRandom;
        use rand::thread_rng;
        
        let mut indices: Vec<usize> = (0..self.playlist.len()).collect();
        let mut rng = thread_rng();
        indices.shuffle(&mut rng);
        
        let mut new_playlist = Vec::with_capacity(self.playlist.len());
        for &i in &indices {
            new_playlist.push(self.playlist[i].clone());
        }
        
        self.playlist = new_playlist;
        self.update_indices_after_sort(&indices);
    }
    
    fn reverse_playlist(&mut self) {
        self.playlist.reverse();
        
        // Update current track index
        if let Some(idx) = self.current_track_index {
            self.current_track_index = Some(self.playlist.len() - 1 - idx);
        }
        
        // Update selected track index
        if let Some(idx) = self.selected_track_index {
            self.selected_track_index = Some(self.playlist.len() - 1 - idx);
        }
    }
    
    fn update_indices_after_sort(&mut self, old_to_new: &[usize]) {
        // Create a mapping from old index to new index
        let mut new_positions = vec![0; old_to_new.len()];
        for (new_idx, &old_idx) in old_to_new.iter().enumerate() {
            new_positions[old_idx] = new_idx;
        }
        
        // Update current track index
        if let Some(idx) = self.current_track_index {
            self.current_track_index = Some(new_positions[idx]);
        }
        
        // Update selected track index
        if let Some(idx) = self.selected_track_index {
            self.selected_track_index = Some(new_positions[idx]);
        }
    }
    
    fn play_track_at_index(&mut self, index: usize) {
        if index < self.playlist.len() {
            self.current_track_index = Some(index);
            let path = self.playlist[index].clone();
            self.play_file(path);
        }
    }
    
    fn play_next(&mut self) {
        if let Some(current_idx) = self.current_track_index {
            let next_idx = (current_idx + 1) % self.playlist.len();
            self.play_track_at_index(next_idx);
        } else if !self.playlist.is_empty() {
            self.play_track_at_index(0);
        }
    }
    
    fn play_previous(&mut self) {
        if let Some(current_idx) = self.current_track_index {
            let prev_idx = if current_idx == 0 {
                self.playlist.len() - 1
            } else {
                current_idx - 1
            };
            self.play_track_at_index(prev_idx);
        } else if !self.playlist.is_empty() {
            self.play_track_at_index(self.playlist.len() - 1);
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
        // Collect all events first to avoid borrow checker issues
        let mut events = Vec::new();
        if let Some(ref engine) = self.audio_engine {
            while let Some(event) = engine.try_recv_event() {
                events.push(event);
            }
        }
        
        // Process events
        for event in events {
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
                    // Auto-play next track if in playlist
                    if !self.playlist.is_empty() {
                        self.play_next();
                    }
                }
                AudioEvent::RequestNext => {
                    self.play_next();
                }
                AudioEvent::RequestPrevious => {
                    self.play_previous();
                }
                AudioEvent::EqualizerUpdated(enabled, gains) => {
                    self.eq_enabled = enabled;
                    self.eq_gains = gains;
                }
                AudioEvent::VisualizationData(samples) => {
                    self.visualizer.update(&samples);
                }
                AudioEvent::Error(msg) => {
                    self.error_message = Some(msg);
                    self.playback_state = PlaybackState::Stopped;
                }
            }
        }
    }
}

impl OneAmpApp {
    /// Handle keyboard shortcuts (Winamp-style)
    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // Playback controls
            if i.key_pressed(egui::Key::X) {
                // X - Play
                if let Some(ref engine) = self.audio_engine {
                    if self.playback_state == PlaybackState::Paused {
                        let _ = engine.send_command(AudioCommand::Resume);
                    } else if !self.playlist.is_empty() && self.current_track_index.is_none() {
                        self.play_track_at_index(0);
                    }
                }
            }
            
            if i.key_pressed(egui::Key::C) {
                // C - Pause/Unpause
                if let Some(ref engine) = self.audio_engine {
                    match self.playback_state {
                        PlaybackState::Playing => {
                            let _ = engine.send_command(AudioCommand::Pause);
                        }
                        PlaybackState::Paused => {
                            let _ = engine.send_command(AudioCommand::Resume);
                        }
                        _ => {}
                    }
                }
            }
            
            if i.key_pressed(egui::Key::V) {
                // V - Stop
                if let Some(ref engine) = self.audio_engine {
                    let _ = engine.send_command(AudioCommand::Stop);
                }
            }
            
            if i.key_pressed(egui::Key::B) {
                // B - Next track
                if let Some(ref engine) = self.audio_engine {
                    let _ = engine.send_command(AudioCommand::Next);
                }
            }
            
            if i.key_pressed(egui::Key::Z) {
                // Z - Previous track
                if let Some(ref engine) = self.audio_engine {
                    let _ = engine.send_command(AudioCommand::Previous);
                }
            }
            
            // Volume control (not implemented in audio engine yet, but reserved)
            // if i.key_pressed(egui::Key::ArrowUp) {
            //     // Volume up
            // }
            // if i.key_pressed(egui::Key::ArrowDown) {
            //     // Volume down
            // }
            
            // Playlist navigation
            if i.key_pressed(egui::Key::Home) {
                // Jump to first track
                if !self.playlist.is_empty() {
                    self.play_track_at_index(0);
                }
            }
            
            if i.key_pressed(egui::Key::End) {
                // Jump to last track
                if !self.playlist.is_empty() {
                    let last_index = self.playlist.len() - 1;
                    self.play_track_at_index(last_index);
                }
            }
            
            // File operations
            if i.modifiers.ctrl && i.key_pressed(egui::Key::L) {
                // Ctrl+L - Open file
                self.open_file();
            }
            
            // Equalizer toggle
            if i.modifiers.alt && i.key_pressed(egui::Key::G) {
                // Alt+G - Toggle equalizer
                self.show_equalizer = !self.show_equalizer;
            }
        });
    }
    
    /// Apply custom modern theme
    fn apply_custom_theme(ctx: &egui::Context) {
        use egui::{Color32, Rounding, Stroke, Vec2};
        
        let mut style = (*ctx.style()).clone();
        
        // Modern dark theme colors
        let bg_color = Color32::from_rgb(25, 25, 30);           // Darker background
        let panel_bg = Color32::from_rgb(30, 30, 35);           // Panel background
        let widget_bg = Color32::from_rgb(40, 40, 45);          // Widget background
        let accent_color = Color32::from_rgb(0, 180, 220);      // Brighter cyan accent
        let text_color = Color32::from_rgb(220, 220, 225);      // Slightly brighter text
        
        // Background colors
        style.visuals.window_fill = bg_color;
        style.visuals.panel_fill = panel_bg;
        style.visuals.extreme_bg_color = widget_bg;
        
        // Widget colors
        style.visuals.widgets.noninteractive.bg_fill = widget_bg;
        style.visuals.widgets.inactive.bg_fill = widget_bg;
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(50, 50, 55);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(60, 60, 65);
        
        // Text colors
        style.visuals.override_text_color = Some(text_color);
        style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text_color);
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text_color);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, accent_color);
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, accent_color);
        
        // Accent colors
        style.visuals.selection.bg_fill = accent_color.linear_multiply(0.3);
        style.visuals.selection.stroke = Stroke::new(1.0, accent_color);
        style.visuals.hyperlink_color = accent_color;
        
        // Rounded corners
        style.visuals.widgets.noninteractive.rounding = Rounding::same(6.0);
        style.visuals.widgets.inactive.rounding = Rounding::same(6.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(6.0);
        style.visuals.widgets.active.rounding = Rounding::same(6.0);
        style.visuals.window_rounding = Rounding::same(8.0);
        
        // Spacing
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(10.0);
        
        ctx.set_style(style);
    }
}

impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply custom theme
        Self::apply_custom_theme(ctx);
        
        // Handle keyboard shortcuts
        self.handle_keyboard_shortcuts(ctx);
        
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
                    ui.add_space(8.0);
                    if ui.button("🎵 Equalizer").clicked() {
                        self.show_equalizer = !self.show_equalizer;
                    }
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
                
                // Previous button
                let prev_enabled = !self.playlist.is_empty();
                if ui.add_enabled(prev_enabled, egui::Button::new("⏮ Previous")).clicked() {
                    self.play_previous();
                }
                
                ui.add_space(8.0);
                
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
                
                ui.add_space(8.0);
                
                // Next button
                let next_enabled = !self.playlist.is_empty();
                if ui.add_enabled(next_enabled, egui::Button::new("⏭ Next")).clicked() {
                    self.play_next();
                }
            });
            
            ui.add_space(8.0);
        });
        
        // Left panel with playlist
        egui::SidePanel::left("playlist_panel")
            .default_width(250.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Playlist");
                ui.separator();
                
                // Playlist controls
                ui.horizontal(|ui| {
                    if ui.button("➕ Add Files").clicked() {
                        self.add_files_to_playlist();
                    }
                    if ui.button("📁 Add Folder").clicked() {
                        self.add_folder_to_playlist();
                    }
                });
                
                ui.horizontal(|ui| {
                    let remove_enabled = self.selected_track_index.is_some();
                    if ui.add_enabled(remove_enabled, egui::Button::new("➖ Remove")).clicked() {
                        self.remove_selected_track();
                    }
                    if ui.button("🗑 Clear All").clicked() {
                        self.clear_playlist();
                    }
                });
                
                // Sort controls
                ui.horizontal(|ui| {
                    ui.menu_button("⬍ Sort", |ui| {
                        if ui.button("By Title").clicked() {
                            self.sort_playlist_by_title();
                            ui.close_menu();
                        }
                        if ui.button("By Filename").clicked() {
                            self.sort_playlist_by_filename();
                            ui.close_menu();
                        }
                        if ui.button("By Path").clicked() {
                            self.sort_playlist_by_path();
                            ui.close_menu();
                        }
                        if ui.button("Shuffle").clicked() {
                            self.shuffle_playlist();
                            ui.close_menu();
                        }
                        if ui.button("Reverse").clicked() {
                            self.reverse_playlist();
                            ui.close_menu();
                        }
                    });
                });
                
                ui.separator();
                
                // Playlist items
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        if self.playlist.is_empty() {
                            ui.add_space(20.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new("No tracks in playlist")
                                        .color(egui::Color32::from_rgb(120, 120, 120))
                                );
                            });
                        } else {
                            let mut track_to_play = None;
                            
                            for (idx, path) in self.playlist.iter().enumerate() {
                                let file_name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("Unknown");
                                
                                let is_current = self.current_track_index == Some(idx);
                                let is_selected = self.selected_track_index == Some(idx);
                                
                                let mut text = egui::RichText::new(file_name);
                                
                                if is_current {
                                    text = text.color(egui::Color32::from_rgb(0, 200, 255));
                                }
                                
                                let response = ui.selectable_label(is_selected, text);
                                
                                if response.clicked() {
                                    self.selected_track_index = Some(idx);
                                }
                                
                                if response.double_clicked() {
                                    track_to_play = Some(idx);
                                }
                            }
                            
                            // Play track after the loop to avoid borrow checker issues
                            if let Some(idx) = track_to_play {
                                self.play_track_at_index(idx);
                            }
                        }
                    });
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
                    
                    ui.add_space(20.0);
                    
                    // Visualizer
                    let viz_height = 80.0;
                    let viz_rect = ui.allocate_space(egui::Vec2::new(ui.available_width().min(400.0), viz_height)).1;
                    
                    // Make visualizer clickable to toggle type
                    let response = ui.allocate_rect(viz_rect, egui::Sense::click());
                    if response.clicked() {
                        self.visualizer.toggle_type();
                    }
                    
                    // Draw visualizer
                    self.visualizer.draw(ui, viz_rect);
                    
                    // Show current viz type
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("📊 {} (click to switch)", self.visualizer.viz_type().name()))
                            .size(12.0)
                            .color(egui::Color32::from_rgb(120, 120, 120))
                    );
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
        
        // Equalizer window
        if self.show_equalizer {
            egui::Window::new("🎵 Equalizer")
                .default_width(400.0)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    
                    // Enable/Disable checkbox
                    let mut enabled = self.eq_enabled;
                    if ui.checkbox(&mut enabled, "Enable Equalizer").changed() {
                        self.eq_enabled = enabled;
                        if let Some(ref engine) = self.audio_engine {
                            let _ = engine.send_command(AudioCommand::SetEqualizerEnabled(enabled));
                        }
                    }
                    
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    
                    // Reset button
                    ui.horizontal(|ui| {
                        if ui.button("🔄 Reset All").clicked() {
                            if let Some(ref engine) = self.audio_engine {
                                let _ = engine.send_command(AudioCommand::ResetEqualizer);
                            }
                        }
                    });
                    
                    ui.add_space(12.0);
                    
                    // Equalizer sliders
                    ui.vertical(|ui| {
                        for i in 0..10 {
                            ui.horizontal(|ui| {
                                let freq = self.eq_frequencies[i];
                                let freq_label = if freq < 1000.0 {
                                    format!("{:.0} Hz", freq)
                                } else {
                                    format!("{:.1} kHz", freq / 1000.0)
                                };
                                
                                ui.label(egui::RichText::new(freq_label).size(14.0).monospace());
                                ui.add_space(8.0);
                                
                                let mut gain = self.eq_gains[i];
                                let slider = egui::Slider::new(&mut gain, -12.0..=12.0)
                                    .suffix(" dB")
                                    .step_by(0.5)
                                    .show_value(true);
                                
                                if ui.add(slider).changed() {
                                    self.eq_gains[i] = gain;
                                    if let Some(ref engine) = self.audio_engine {
                                        let _ = engine.send_command(AudioCommand::SetEqualizerBand(i, gain));
                                    }
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
                    
                    ui.add_space(8.0);
                });
        }
    }
    
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save configuration
        let config = AppConfig {
            equalizer: config::EqualizerConfig {
                enabled: self.eq_enabled,
                gains: self.eq_gains.clone(),
            },
            first_run: false,
        };
        if let Err(e) = config.save() {
            eprintln!("Failed to save configuration: {}", e);
        }
        
        // Shutdown audio engine
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
