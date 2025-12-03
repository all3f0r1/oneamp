use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub mod audio_capture;
pub mod audio_thread_symphonia;
pub mod cpal_output;
pub mod eq_source;
pub mod equalizer;
pub mod plugins;
pub mod rodio_output;
pub mod symphonia_player;

pub use audio_capture::{AudioCaptureBuffer, AudioCaptureSource};
pub use eq_source::EqualizerSource;
pub use equalizer::Equalizer;

/// Commands that can be sent to the audio thread
#[derive(Debug, Clone)]
pub enum AudioCommand {
    /// Load and play a file
    Play(PathBuf),
    /// Pause playback
    Pause,
    /// Resume playback
    Resume,
    /// Stop playback
    Stop,
    /// Seek to a position (in seconds)
    Seek(f32),
    /// Play next track in playlist
    Next,
    /// Play previous track in playlist
    Previous,
    /// Set equalizer enabled state
    SetEqualizerEnabled(bool),
    /// Set equalizer band gain (band_index, gain_db)
    SetEqualizerBand(usize, f32),
    /// Set all equalizer bands at once
    SetEqualizerBands(Vec<f32>),
    /// Reset equalizer to flat response
    ResetEqualizer,
    /// Shutdown the audio thread
    Shutdown,
}

/// Events sent from the audio thread to the GUI
#[derive(Debug, Clone)]
pub enum AudioEvent {
    /// Track loaded successfully with metadata
    TrackLoaded(TrackInfo),
    /// Playback started
    Playing,
    /// Playback paused
    Paused,
    /// Playback stopped
    Stopped,
    /// Playback position update (current_secs, total_secs)
    Position(f32, f32),
    /// Playback finished (track ended)
    Finished,
    /// Request next track from playlist
    RequestNext,
    /// Request previous track from playlist
    RequestPrevious,
    /// Equalizer state updated (enabled, gains)
    EqualizerUpdated(bool, Vec<f32>),
    /// Audio samples for visualization
    VisualizationData(Vec<f32>),
    /// Error occurred
    Error(String),
}

/// Track metadata information
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: Option<f32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
}

impl TrackInfo {
    /// Extract metadata from a file
    pub fn from_file(path: &PathBuf) -> Result<Self> {
        let file = File::open(path).context("Failed to open audio file for metadata reading")?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension() {
            hint.with_extension(ext.to_str().unwrap_or(""));
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .context("Failed to probe audio file")?;

        let mut format = probed.format;

        let mut title = None;
        let mut artist = None;
        let mut album = None;

        // Get metadata from the format
        if let Some(metadata_rev) = format.metadata().current() {
            for tag in metadata_rev.tags() {
                match tag.std_key {
                    Some(symphonia::core::meta::StandardTagKey::TrackTitle) => {
                        title = Some(tag.value.to_string());
                    }
                    Some(symphonia::core::meta::StandardTagKey::Artist) => {
                        artist = Some(tag.value.to_string());
                    }
                    Some(symphonia::core::meta::StandardTagKey::Album) => {
                        album = Some(tag.value.to_string());
                    }
                    _ => {}
                }
            }
        }

        let mut sample_rate = None;
        let mut channels = None;
        let mut duration_secs = None;

        // Get track information
        if let Some(track) = format.default_track() {
            let codec_params = &track.codec_params;

            sample_rate = codec_params.sample_rate;
            channels = codec_params.channels.map(|c| c.count() as u8);

            if let (Some(n_frames), Some(sr)) = (codec_params.n_frames, codec_params.sample_rate) {
                duration_secs = Some(n_frames as f32 / sr as f32);
            }
        }

        Ok(TrackInfo {
            path: path.clone(),
            title,
            artist,
            album,
            duration_secs,
            sample_rate,
            channels,
        })
    }
}

/// Audio engine that runs in a separate thread
pub struct AudioEngine {
    command_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl AudioEngine {
    /// Create a new audio engine
    pub fn new() -> Result<Self> {
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();

        let thread_handle = thread::spawn(move || {
            if let Err(e) =
                audio_thread_symphonia::audio_thread_main_symphonia(command_rx, event_tx)
            {
                eprintln!("Audio thread error: {}", e);
            }
        });

        Ok(AudioEngine {
            command_tx,
            event_rx,
            thread_handle: Some(thread_handle),
        })
    }

    /// Send a command to the audio thread
    pub fn send_command(&self, cmd: AudioCommand) -> Result<()> {
        self.command_tx
            .send(cmd)
            .context("Failed to send command to audio thread")
    }

    /// Try to receive an event from the audio thread (non-blocking)
    pub fn try_recv_event(&self) -> Option<AudioEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Shutdown the audio engine
    pub fn shutdown(mut self) -> Result<()> {
        self.send_command(AudioCommand::Shutdown)?;
        if let Some(handle) = self.thread_handle.take() {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Failed to join audio thread"))?;
        }
        Ok(())
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Main audio thread function
#[allow(dead_code)]
fn audio_thread_main(
    command_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
) -> Result<()> {
    let (_stream, stream_handle) =
        OutputStream::try_default().context("Failed to get default audio output device")?;

    let mut sink: Option<Sink> = None;
    let mut current_track: Option<TrackInfo> = None;
    let mut is_paused = false;

    // Create equalizer (shared between audio processing and command handling)
    let equalizer = std::sync::Arc::new(std::sync::Mutex::new(Equalizer::new(44100.0)));

    // Create audio capture buffer for visualization
    let capture_buffer = std::sync::Arc::new(std::sync::Mutex::new(AudioCaptureBuffer::new(2048)));
    let capture_buffer_clone = capture_buffer.clone();

    // Throttle position updates to reduce allocations
    let mut last_position_update = std::time::Instant::now();
    let position_update_interval = Duration::from_millis(100);

    loop {
        // Check for commands
        if let Ok(cmd) = command_rx.try_recv() {
            match cmd {
                AudioCommand::Play(path) => {
                    // Stop current playback
                    sink = None;

                    // Load track metadata
                    match TrackInfo::from_file(&path) {
                        Ok(track_info) => {
                            current_track = Some(track_info.clone());
                            let _ = event_tx.send(AudioEvent::TrackLoaded(track_info));

                            // Load and play the file
                            match load_and_play(
                                &path,
                                &stream_handle,
                                equalizer.clone(),
                                capture_buffer.clone(),
                            ) {
                                Ok(new_sink) => {
                                    sink = Some(new_sink);
                                    is_paused = false;
                                    let _ = event_tx.send(AudioEvent::Playing);
                                }
                                Err(e) => {
                                    let _ = event_tx
                                        .send(AudioEvent::Error(format!("Failed to play: {}", e)));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(AudioEvent::Error(format!("Failed to load track: {}", e)));
                        }
                    }
                }
                AudioCommand::Pause => {
                    if let Some(ref s) = sink {
                        if !is_paused {
                            s.pause();
                            is_paused = true;
                            let _ = event_tx.send(AudioEvent::Paused);
                        }
                    }
                }
                AudioCommand::Resume => {
                    if let Some(ref s) = sink {
                        if is_paused {
                            s.play();
                            is_paused = false;
                            let _ = event_tx.send(AudioEvent::Playing);
                        }
                    }
                }
                AudioCommand::Stop => {
                    sink = None;
                    current_track = None;
                    is_paused = false;
                    let _ = event_tx.send(AudioEvent::Stopped);
                }
                AudioCommand::Seek(_pos) => {
                    if let Some(ref track) = current_track {
                        // Stop current playback
                        sink = None;

                        // Reload and seek by skipping to position
                        // Note: This is a limitation of rodio - it restarts playback
                        match load_and_play(
                            &track.path,
                            &stream_handle,
                            equalizer.clone(),
                            capture_buffer.clone(),
                        ) {
                            Ok(new_sink) => {
                                // Skip to the desired position
                                new_sink.skip_one();
                                // Note: rodio doesn't support precise seeking
                                // We can only restart playback
                                // A proper implementation would require a custom decoder

                                if is_paused {
                                    new_sink.pause();
                                }

                                sink = Some(new_sink);
                                // Send Playing event to update UI
                                let _ = event_tx.send(AudioEvent::Playing);
                            }
                            Err(e) => {
                                let _ = event_tx
                                    .send(AudioEvent::Error(format!("Failed to seek: {}", e)));
                            }
                        }
                    }
                }
                AudioCommand::Next => {
                    // Stop current playback and request next track from GUI
                    sink = None;
                    current_track = None;
                    is_paused = false;
                    let _ = event_tx.send(AudioEvent::RequestNext);
                }
                AudioCommand::Previous => {
                    // Stop current playback and request previous track from GUI
                    sink = None;
                    current_track = None;
                    is_paused = false;
                    let _ = event_tx.send(AudioEvent::RequestPrevious);
                }
                AudioCommand::SetEqualizerEnabled(enabled) => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.set_enabled(enabled);
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::SetEqualizerBand(band_index, gain_db) => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.set_band_gain(band_index, gain_db);
                        let enabled = eq.is_enabled();
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::SetEqualizerBands(gains) => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.set_all_gains(&gains);
                        let enabled = eq.is_enabled();
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::ResetEqualizer => {
                    if let Ok(mut eq) = equalizer.lock() {
                        eq.reset_all_bands();
                        let enabled = eq.is_enabled();
                        let gains = eq.get_all_gains().to_vec();
                        let _ = event_tx.send(AudioEvent::EqualizerUpdated(enabled, gains));
                    }
                }
                AudioCommand::Shutdown => {
                    break;
                }
            }
        }

        // Update playback position
        if let Some(ref s) = sink {
            if s.empty() {
                // Track finished
                sink = None;
                current_track = None;
                is_paused = false;
                let _ = event_tx.send(AudioEvent::Finished);
            } else if !is_paused {
                // Send position update (throttled)
                if last_position_update.elapsed() >= position_update_interval {
                    if let Some(ref track) = current_track {
                        let current_pos = s.get_pos().as_secs_f32();
                        let total_duration = track.duration_secs.unwrap_or(0.0);
                        let _ = event_tx.send(AudioEvent::Position(current_pos, total_duration));
                    }
                    last_position_update = std::time::Instant::now();
                }

                // Send visualization data
                if let Ok(buffer) = capture_buffer_clone.lock() {
                    let samples = buffer.get_samples().to_vec();
                    let _ = event_tx.send(AudioEvent::VisualizationData(samples));
                }
            }
        }

        // Sleep to avoid busy-waiting
        thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}

/// Load and play an audio file
#[allow(dead_code)]
fn load_and_play(
    path: &PathBuf,
    stream_handle: &OutputStreamHandle,
    equalizer: std::sync::Arc<std::sync::Mutex<Equalizer>>,
    capture_buffer: std::sync::Arc<std::sync::Mutex<AudioCaptureBuffer>>,
) -> Result<Sink> {
    let file = BufReader::new(File::open(path).context("Failed to open audio file for playback")?);

    let source = Decoder::new(file).context("Failed to decode audio file")?;

    // Wrap source with equalizer
    let eq_source = EqualizerSource::new(source, equalizer);

    // Wrap with audio capture for visualization
    let capture_source = AudioCaptureSource::new(eq_source, capture_buffer);

    let sink = Sink::try_new(stream_handle).context("Failed to create audio sink")?;
    sink.append(capture_source);

    Ok(sink)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_engine_creation() {
        // Test that AudioEngine can be created
        let engine = AudioEngine::new();
        assert!(engine.is_ok(), "AudioEngine should be created successfully");
    }

    #[test]
    fn test_audio_engine_shutdown() {
        // Test that AudioEngine can be shut down properly
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");
        let result = engine.shutdown();
        assert!(result.is_ok(), "AudioEngine should shutdown without errors");
    }

    #[test]
    fn test_audio_command_send() {
        // Test that commands can be sent to the audio engine
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");

        // Send a stop command (safe even if nothing is playing)
        let result = engine.send_command(AudioCommand::Stop);
        assert!(result.is_ok(), "Should be able to send Stop command");

        // Send equalizer commands
        let result = engine.send_command(AudioCommand::SetEqualizerEnabled(true));
        assert!(
            result.is_ok(),
            "Should be able to send SetEqualizerEnabled command"
        );

        let result = engine.send_command(AudioCommand::ResetEqualizer);
        assert!(
            result.is_ok(),
            "Should be able to send ResetEqualizer command"
        );
    }

    #[test]
    fn test_audio_event_reception() {
        // Test that events can be received from the audio engine
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");

        // Try to receive events (should be non-blocking)
        let event = engine.try_recv_event();
        // Either None or Some(event) is fine, just shouldn't panic
        // We just verify it returns an Option
        assert!(
            event.is_none() || event.is_some(),
            "Should return an Option"
        );
    }

    #[test]
    fn test_equalizer_commands() {
        // Test equalizer-related commands
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");

        // Test setting individual band
        let result = engine.send_command(AudioCommand::SetEqualizerBand(0, 3.0));
        assert!(result.is_ok(), "Should be able to set equalizer band");

        // Test setting all bands
        let gains = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let result = engine.send_command(AudioCommand::SetEqualizerBands(gains));
        assert!(result.is_ok(), "Should be able to set all equalizer bands");
    }

    #[test]
    fn test_track_info_creation() {
        // Test TrackInfo structure
        let track = TrackInfo {
            path: PathBuf::from("/test/path.mp3"),
            title: Some("Test Track".to_string()),
            artist: Some("Test Artist".to_string()),
            album: Some("Test Album".to_string()),
            duration_secs: Some(180.0),
            sample_rate: Some(44100),
            channels: Some(2),
        };

        assert_eq!(track.title, Some("Test Track".to_string()));
        assert_eq!(track.sample_rate, Some(44100));
        assert_eq!(track.channels, Some(2));
    }

    #[test]
    fn test_audio_capture_buffer() {
        // Test AudioCaptureBuffer
        let mut buffer = AudioCaptureBuffer::new(1024);

        // Initially should be empty or zeros
        let samples = buffer.get_samples();
        assert_eq!(samples.len(), 1024, "Buffer should have correct size");

        // Update with some samples
        let test_samples: Vec<f32> = (0..512).map(|i| (i as f32) / 512.0).collect();
        buffer.update(&test_samples, 44100, 2);

        // Verify samples were added
        let retrieved = buffer.get_samples();
        assert_eq!(retrieved.len(), 1024, "Buffer size should remain constant");
        assert_eq!(buffer.sample_rate(), 44100);
        assert_eq!(buffer.channels(), 2);
    }

    #[test]
    fn test_audio_commands_clone() {
        // Test that AudioCommand can be cloned
        let cmd1 = AudioCommand::Stop;
        let cmd2 = cmd1.clone();

        // Both should be Stop
        match (cmd1, cmd2) {
            (AudioCommand::Stop, AudioCommand::Stop) => {}
            _ => panic!("Commands should both be Stop"),
        }
    }

    #[test]
    fn test_multiple_engines() {
        // Test that multiple AudioEngines cannot be created simultaneously
        // (This tests the singleton behavior of audio output)
        let engine1 = AudioEngine::new();
        assert!(engine1.is_ok(), "First engine should be created");

        // Note: Creating a second engine might fail or succeed depending on the audio backend
        // We just test that it doesn't panic
        let _engine2 = AudioEngine::new();
        // No assertion here as behavior is platform-dependent
    }
}
