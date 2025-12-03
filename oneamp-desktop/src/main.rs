use eframe::egui;
use oneamp_core::{AudioCommand, AudioEngine, AudioEvent, TrackInfo};
use std::path::PathBuf;

mod config;
use config::AppConfig;

mod visualizer;
use visualizer::Visualizer;

mod theme;
use theme::Theme;

mod track_display;

mod ui_components;

mod visual_effects;

mod custom_widgets;

mod animations;
use animations::AnimationTimer;

mod equalizer_display;
use equalizer_display::EqualizerDisplay;

mod control_buttons;
use control_buttons::{ControlAction, control_button_row};

mod album_art;
use album_art::AlbumArtDisplay;

mod window_chrome;
use window_chrome::{WindowChrome, WindowAction};

mod onedrop_visualizer;
use onedrop_visualizer::OneDropVisualizer;

mod platform_detection;
use platform_detection::PlatformInfo;

mod skins;
use skins::SkinManager;

fn main() -> eframe::Result {
    let theme = Theme::default();
    
    // Smart platform detection for window chrome
    // Detects OS, desktop environment, and display server
    let platform_info = PlatformInfo::detect();
    let use_custom_chrome = platform_info.should_use_custom_chrome();
    
    println!("Platform: {}", platform_info.description());
    println!("Custom window chrome: {}", if use_custom_chrome { "enabled" } else { "disabled" });
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([theme.layout.window_min_width, theme.layout.window_min_height])
            .with_min_inner_size([theme.layout.window_min_width, theme.layout.window_min_height])
            .with_decorations(!use_custom_chrome) // Custom chrome = no decorations
            .with_icon(
                eframe::icon_data::from_png_bytes(&include_bytes!("../../icon_256.png")[..])
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    
    eframe::run_native(
        "OneAmp",
        options,
        Box::new(move |cc| {
            Ok(Box::new(OneAmpApp::new(cc, use_custom_chrome)))
        }),
    )
}

struct OneAmpApp {
    audio_engine: Option<AudioEngine>,
    current_track: Option<TrackInfo>,
    playback_state: PlaybackState,
    current_position: f32,
    total_duration: f32,
    error_message: Option<String>,
    
    // Playlist
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
    
    // Theme
    theme: Theme,
    
    // Skin system
    skin_manager: SkinManager,
    show_skin_selector: bool,
    
    // UI state
    scroll_offset: usize,
    last_scroll_update: std::time::Instant,
    
    // Animation
    animation_timer: AnimationTimer,
    
    // New UI components
    equalizer_display: EqualizerDisplay,
    album_art: AlbumArtDisplay,
    window_chrome: WindowChrome,
    
    // OneDrop visualizer
    onedrop: Option<OneDropVisualizer>,
    
    // Platform-specific window chrome
    use_custom_chrome: bool,
    use_onedrop: bool,
    onedrop_texture_id: Option<egui::TextureId>,
    visualizer_fullscreen: bool,
    
    // Performance monitoring
    frame_times: std::collections::VecDeque<f32>,
    show_fps: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

impl OneAmpApp {
    fn new(cc: &eframe::CreationContext<'_>, use_custom_chrome: bool) -> Self {
        let theme = Theme::default();
        theme.apply_to_egui(&cc.egui_ctx);
        
        let audio_engine = match AudioEngine::new() {
            Ok(engine) => Some(engine),
            Err(e) => {
                eprintln!("Failed to initialize audio engine: {}", e);
                None
            }
        };
        
        let (config, is_first_run) = AppConfig::load();
        
        // Initialize skin manager
        let skins_dir = dirs::config_dir()
            .map(|d| d.join("oneamp").join("skins"))
            .unwrap_or_else(|| PathBuf::from("./skins"));
        
        let mut skin_manager = SkinManager::discover_and_load(&skins_dir);
        
        // Load the active skin from config
        if let Some(index) = skin_manager.find_skin_by_name(&config.active_skin) {
            skin_manager.set_active_skin(index);
        }
        
        // Apply the active skin
        skin_manager.apply_skin(&cc.egui_ctx);
        
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
            theme,
            skin_manager,
            show_skin_selector: false,
            scroll_offset: 0,
            last_scroll_update: std::time::Instant::now(),
            animation_timer: AnimationTimer::new(),
            equalizer_display: EqualizerDisplay::new(10),
            album_art: AlbumArtDisplay::new(),
            window_chrome: WindowChrome::new(),
            onedrop: None,  // Will be initialized asynchronously
            use_custom_chrome,
            use_onedrop: false,
            onedrop_texture_id: None,
            visualizer_fullscreen: false,
            frame_times: std::collections::VecDeque::with_capacity(60),
            show_fps: false,
        };
        
        // Initialize OneDrop visualizer asynchronously
        app.onedrop = pollster::block_on(async {
            match OneDropVisualizer::new(800, 600).await {
                Ok(mut visualizer) => {
                    // Try to load presets from onedrop directory
                    let preset_dir = std::path::PathBuf::from("../../onedrop/test-presets");
                    if preset_dir.exists() {
                        let _ = visualizer.load_presets(&preset_dir);
                    }
                    Some(visualizer)
                }
                Err(e) => {
                    eprintln!("Failed to initialize OneDrop visualizer: {}", e);
                    None
                }
            }
        });
        
        if let Some(ref engine) = app.audio_engine {
            let _ = engine.send_command(AudioCommand::SetEqualizerEnabled(config.equalizer.enabled));
            let _ = engine.send_command(AudioCommand::SetEqualizerBands(config.equalizer.gains));
        }
        
        if is_first_run {
            app.play_jingle();
        }
        
        app
    }
    
    fn play_jingle(&mut self) {
        const JINGLE_DATA: &[u8] = include_bytes!("../../packaging/jingle.wav");
        
        if let Ok(temp_dir) = std::env::temp_dir().canonicalize() {
            let jingle_path = temp_dir.join("oneamp_jingle.wav");
            
            if std::fs::write(&jingle_path, JINGLE_DATA).is_ok() {
                if let Some(engine) = &mut self.audio_engine {
                    let _ = engine.send_command(AudioCommand::Play(jingle_path));
                }
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
                    self.current_track = Some(track_info);
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
    
    fn play_file(&mut self, path: PathBuf) {
        if let Some(ref engine) = self.audio_engine {
            let _ = engine.send_command(AudioCommand::Play(path));
        }
    }
    
    fn play_track_at_index(&mut self, index: usize) {
        if index < self.playlist.len() {
            self.current_track_index = Some(index);
            self.play_file(self.playlist[index].clone());
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
    
    fn toggle_play_pause(&mut self) {
        if let Some(ref engine) = self.audio_engine {
            match self.playback_state {
                PlaybackState::Playing => {
                    let _ = engine.send_command(AudioCommand::Pause);
                }
                PlaybackState::Paused => {
                    let _ = engine.send_command(AudioCommand::Resume);
                }
                PlaybackState::Stopped => {
                    if !self.playlist.is_empty() {
                        self.play_track_at_index(self.current_track_index.unwrap_or(0));
                    }
                }
            }
        }
    }
    
    fn stop(&mut self) {
        if let Some(ref engine) = self.audio_engine {
            let _ = engine.send_command(AudioCommand::Stop);
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
                if let Some(current_idx) = self.current_track_index {
                    if current_idx == index {
                        self.current_track_index = None;
                    } else if current_idx > index {
                        self.current_track_index = Some(current_idx - 1);
                    }
                }
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
    
    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                self.toggle_play_pause();
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::O) {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio Files", &["mp3", "flac", "ogg", "wav"])
                    .pick_file()
                {
                    self.play_file(path);
                }
            }
        });
    }
    
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                for file in &i.raw.dropped_files {
                    if let Some(path) = &file.path {
                        if path.is_file() {
                            if let Some(ext) = path.extension() {
                                if ["mp3", "flac", "ogg", "wav"].contains(&ext.to_str().unwrap_or("")) {
                                    if !self.playlist.contains(path) {
                                        self.playlist.push(path.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

impl eframe::App for OneAmpApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Apply the active skin at the beginning of each frame
        self.skin_manager.apply_skin(ctx);
        
        self.theme.apply_to_egui(ctx);
        
        // Custom window chrome (platform-specific)
        // Enabled on Windows/macOS, disabled on Linux (system freeze issues)
        if self.use_custom_chrome {
            let window_action = self.window_chrome.render(ctx, &self.theme, "OneAmp");
            match window_action {
                WindowAction::Close => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                WindowAction::Minimize => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                WindowAction::ToggleMaximize => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                }
                WindowAction::StartDrag => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                WindowAction::None => {}
            }
        }
        
        self.handle_keyboard_shortcuts(ctx);
        self.handle_dropped_files(ctx);
        self.process_audio_events();
        
        // Update FPS counter
        let delta_time = ctx.input(|i| i.unstable_dt);
        self.frame_times.push_back(delta_time);
        if self.frame_times.len() > 60 {
            self.frame_times.pop_front();
        }
        
        // Update OneDrop visualizer with audio samples
        if self.use_onedrop {
            if let Some(ref mut onedrop) = self.onedrop {
                // Get audio samples from visualizer
                let audio_samples = self.visualizer.get_spectrum();
                // Convert spectrum to audio samples (simplified)
                let samples: Vec<f32> = audio_samples.iter()
                    .flat_map(|&v| vec![v, v])  // Duplicate for stereo
                    .collect();
                
                let _ = onedrop.update(&samples, delta_time);
            }
        }
        
        // Update scroll animation
        if self.last_scroll_update.elapsed().as_millis() > 200 {
            self.last_scroll_update = std::time::Instant::now();
        }
        
        ctx.request_repaint();
        
        // Main vertical layout: Player -> Equalizer -> Playlist
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical(|ui| {
                // PLAYER SECTION
                ui_components::render_player_section(
                    ui,
                    &self.theme,
                    &self.current_track,
                    self.current_position,
                    self.total_duration,
                    self.visualizer.get_spectrum(),
                    &mut self.scroll_offset,
                );
                
                ui.add_space(8.0);
                
                // PROGRESS BAR
                if let Some(seek_pos) = ui_components::render_progress_bar(
                    ui,
                    &self.theme,
                    self.current_position,
                    self.total_duration,
                ) {
                    if let Some(ref engine) = self.audio_engine {
                        let _ = engine.send_command(AudioCommand::Seek(seek_pos));
                    }
                }
                
                ui.add_space(8.0);
                
                // CONTROL BUTTONS (new 3D buttons)
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    
                    // Album art on the left
                    if let Some(ref track) = self.current_track {
                        self.album_art.load_from_track(&track.path, ctx);
                    }
                    
                    if self.album_art.has_art() {
                        self.album_art.render(ui, &self.theme, 120.0);
                        ui.add_space(16.0);
                    }
                    
                    // Control buttons
                    ui.vertical(|ui| {
                        ui.add_space(20.0);
                        
                        let action = control_button_row(
                            ui,
                            &self.theme,
                            self.playback_state == PlaybackState::Playing,
                            self.playback_state == PlaybackState::Paused,
                        );
                        
                        match action {
                            ControlAction::Previous => self.play_previous(),
                            ControlAction::Play => self.toggle_play_pause(),
                            ControlAction::Pause => self.toggle_play_pause(),
                            ControlAction::Stop => self.stop(),
                            ControlAction::Next => self.play_next(),
                            ControlAction::None => {}
                        }
                    });
                });
                
                ui.add_space(8.0);
                ui.separator();
                
                // VISUALIZER TOGGLE
                ui.horizontal(|ui| {
                    ui.label("Visualizer:");
                    
                    if ui.selectable_label(!self.use_onedrop, "Spectrum").clicked() {
                        self.use_onedrop = false;
                        if let Some(ref mut onedrop) = self.onedrop {
                            onedrop.set_enabled(false);
                        }
                    }
                    
                    let has_presets = self.onedrop.as_ref().map_or(false, |od| od.has_presets());
                    
                    if has_presets {
                        if ui.selectable_label(self.use_onedrop, "Milkdrop").clicked() {
                            self.use_onedrop = true;
                            if let Some(ref mut onedrop) = self.onedrop {
                                onedrop.set_enabled(true);
                            }
                        }
                        
                        if self.use_onedrop {
                            ui.separator();
                            
                            let mut action = None;
                            
                            if ui.button("◄").clicked() {
                                action = Some("prev");
                            }
                            
                            // Get preset info without holding borrow
                            let preset_info = self.onedrop.as_ref().map(|od| {
                                (od.current_preset_index(), od.preset_count(), od.current_preset_name())
                            });
                            
                            if let Some((idx, count, name)) = preset_info {
                                if let Some(preset_name) = name {
                                    ui.label(format!("[{}/{}] {}", idx, count, preset_name));
                                }
                            }
                            
                            if ui.button("►").clicked() {
                                action = Some("next");
                            }
                            
                            // Execute action after releasing borrow
                            if let Some(action) = action {
                                if let Some(ref mut onedrop) = self.onedrop {
                                    match action {
                                        "prev" => { let _ = onedrop.previous_preset(); }
                                        "next" => { let _ = onedrop.next_preset(); }
                                        _ => {}
                                    }
                                }
                            }
                                
                            ui.separator();
                            
                            // Fullscreen toggle
                            if ui.button("🕲 Fullscreen").clicked() {
                                self.visualizer_fullscreen = !self.visualizer_fullscreen;
                            }
                            
                            // FPS toggle
                            if ui.button(if self.show_fps { "Hide FPS" } else { "Show FPS" }).clicked() {
                                self.show_fps = !self.show_fps;
                            }
                            
                            if self.show_fps {
                                let fps = if !self.frame_times.is_empty() {
                                    let avg_time: f32 = self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32;
                                    if avg_time > 0.0 { 1.0 / avg_time } else { 0.0 }
                                } else { 0.0 };
                                ui.label(format!("FPS: {:.1}", fps));
                            }
                        }
                    }
                });
                
                // OneDrop visualization rendering
                if self.use_onedrop && !self.visualizer_fullscreen {
                    if let Some(ref onedrop) = self.onedrop {
                        if onedrop.is_enabled() {
                            ui.add_space(8.0);
                            ui.label("Milkdrop Visualization:");
                            
                            // Get texture from OneDrop
                            let texture = onedrop.render_texture();
                            let (width, height) = onedrop.render_size();
                            
                            // Register texture with egui if not already done
                            if self.onedrop_texture_id.is_none() {
                                if let Some(render_state) = frame.wgpu_render_state() {
                                    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                                    let texture_id = render_state.renderer.write().register_native_texture(
                                        &render_state.device,
                                        &texture_view,
                                        wgpu::FilterMode::Linear,
                                    );
                                    self.onedrop_texture_id = Some(texture_id);
                                }
                            }
                            
                            // Display the texture
                            if let Some(texture_id) = self.onedrop_texture_id {
                                let size = egui::vec2(width as f32, height as f32);
                                ui.image(egui::load::SizedTexture::new(texture_id, size));
                            }
                        }
                    }
                }
                
                ui.add_space(8.0);
                ui.separator();
                
                // EQUALIZER SECTION (new advanced display)
                ui.horizontal(|ui| {
                    ui.heading("🎚 Equalizer");
                    if ui.button(if self.show_equalizer { "▼" } else { "▶" }).clicked() {
                        self.show_equalizer = !self.show_equalizer;
                    }
                });
                
                if self.show_equalizer {
                    ui.add_space(8.0);
                    if self.equalizer_display.render(
                        ui,
                        &self.theme,
                        &mut self.eq_enabled,
                        &mut self.eq_gains,
                        &self.eq_frequencies,
                    ) {
                        if let Some(ref engine) = self.audio_engine {
                            let _ = engine.send_command(AudioCommand::SetEqualizerEnabled(self.eq_enabled));
                            let _ = engine.send_command(AudioCommand::SetEqualizerBands(self.eq_gains.clone()));
                        }
                    }
                }
                
                ui.add_space(8.0);
                ui.separator();
                
                // PLAYLIST SECTION
                ui.horizontal(|ui| {
                    ui.heading("🎵 Playlist");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("➕ Add Files").clicked() {
                            self.add_files_to_playlist();
                        }
                        if ui.button("📁 Add Folder").clicked() {
                            self.add_folder_to_playlist();
                        }
                        if ui.add_enabled(self.selected_track_index.is_some(), egui::Button::new("➖ Remove")).clicked() {
                            self.remove_selected_track();
                        }
                        if ui.button("🗑 Clear").clicked() {
                            self.clear_playlist();
                        }
                    });
                });
                
                ui.add_space(4.0);
                
                let actions = ui_components::render_playlist(
                    ui,
                    &self.theme,
                    &self.playlist,
                    self.current_track_index,
                    self.selected_track_index,
                );
                
                if let Some(idx) = actions.play_track {
                    self.play_track_at_index(idx);
                }
                if let Some(idx) = actions.select_track {
                    self.selected_track_index = Some(idx);
                }
            });
        });
        
        // Error message toast
        let mut clear_error = false;
        if let Some(ref msg) = self.error_message {
            let msg_clone = msg.clone();
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(&msg_clone);
                    if ui.button("OK").clicked() {
                        clear_error = true;
                    }
                });
        }
        if clear_error {
            self.error_message = None;
        }
        
        // Fullscreen visualizer mode
        if self.visualizer_fullscreen && self.use_onedrop {
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(ref onedrop) = self.onedrop {
                    if onedrop.is_enabled() {
                        // Get texture from OneDrop
                        let texture = onedrop.render_texture();
                        
                        // Register texture if not already done
                        if self.onedrop_texture_id.is_none() {
                            if let Some(render_state) = frame.wgpu_render_state() {
                                let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                                let texture_id = render_state.renderer.write().register_native_texture(
                                    &render_state.device,
                                    &texture_view,
                                    wgpu::FilterMode::Linear,
                                );
                                self.onedrop_texture_id = Some(texture_id);
                            }
                        }
                        
                        // Display fullscreen texture
                        if let Some(texture_id) = self.onedrop_texture_id {
                            let available_size = ui.available_size();
                            ui.image(egui::load::SizedTexture::new(texture_id, available_size));
                        }
                    }
                }
                
                // Close button overlay
                ui.allocate_ui_at_rect(egui::Rect::from_min_size(
                    egui::pos2(10.0, 10.0),
                    egui::vec2(150.0, 30.0)
                ), |ui| {
                    if ui.button("✕ Close Fullscreen").clicked() {
                        self.visualizer_fullscreen = false;
                    }
                });
            });
        }
    }
}
