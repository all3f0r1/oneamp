use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::{AudioCommand, AudioEvent, TrackInfo, Equalizer, AudioCaptureBuffer};
use crate::symphonia_player::SymphoniaPlayer;
use crate::cpal_output::CpalOutput;

/// Audio playback state
struct PlaybackState {
    player: SymphoniaPlayer,
    output: CpalOutput,
    is_paused: bool,
}

/// Main audio thread function using Symphonia + cpal
pub fn audio_thread_main_symphonia(
    command_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
) -> Result<()> {
    let mut playback: Option<PlaybackState> = None;
    let mut current_track: Option<TrackInfo> = None;
    
    // Create equalizer (shared between audio processing and command handling)
    let equalizer = Arc::new(Mutex::new(Equalizer::new(44100.0)));
    
    // Create audio capture buffer for visualization
    let capture_buffer = Arc::new(Mutex::new(AudioCaptureBuffer::new(2048)));
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
                    playback = None;
                    
                    // Load track metadata
                    match TrackInfo::from_file(&path) {
                        Ok(track_info) => {
                            current_track = Some(track_info.clone());
                            let _ = event_tx.send(AudioEvent::TrackLoaded(track_info));
                            
                            // Load and play the file
                            match load_and_play(&path, equalizer.clone(), capture_buffer.clone()) {
                                Ok(state) => {
                                    playback = Some(state);
                                    let _ = event_tx.send(AudioEvent::Playing);
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AudioEvent::Error(format!("Failed to play: {}", e)));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(AudioEvent::Error(format!("Failed to load track: {}", e)));
                        }
                    }
                }
                AudioCommand::Pause => {
                    if let Some(ref mut state) = playback {
                        if !state.is_paused {
                            let _ = state.output.pause();
                            state.is_paused = true;
                            let _ = event_tx.send(AudioEvent::Paused);
                        }
                    }
                }
                AudioCommand::Resume => {
                    if let Some(ref mut state) = playback {
                        if state.is_paused {
                            let _ = state.output.play();
                            state.is_paused = false;
                            let _ = event_tx.send(AudioEvent::Playing);
                        }
                    }
                }
                AudioCommand::Stop => {
                    playback = None;
                    current_track = None;
                    let _ = event_tx.send(AudioEvent::Stopped);
                }
                AudioCommand::Seek(pos) => {
                    if let Some(ref mut state) = playback {
                        // Perform the seek
                        match state.player.seek(pos) {
                            Ok(()) => {
                                // Clear the output buffer to avoid playing old samples
                                state.output.clear();
                                let _ = event_tx.send(AudioEvent::Playing);
                            }
                            Err(e) => {
                                let _ = event_tx.send(AudioEvent::Error(format!("Failed to seek: {}", e)));
                            }
                        }
                    }
                }
                AudioCommand::Next => {
                    // Stop current playback and request next track from GUI
                    playback = None;
                    current_track = None;
                    let _ = event_tx.send(AudioEvent::RequestNext);
                }
                AudioCommand::Previous => {
                    // Stop current playback and request previous track from GUI
                    playback = None;
                    current_track = None;
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
        
        // Decode and feed audio to output
        let mut end_of_stream = false;
        if let Some(ref mut state) = playback {
            if !state.is_paused {
                // Check if output needs more data
                if state.output.needs_data() {
                    match state.player.decode_next() {
                        Ok(Some(samples)) => {
                            if !samples.is_empty() {
                                state.output.write_samples(&samples);
                            }
                        }
                        Ok(None) => {
                            // End of stream
                            end_of_stream = true;
                        }
                        Err(e) => {
                            eprintln!("Decode error: {}", e);
                            // Continue playback despite errors
                        }
                    }
                }
                
                // Send position update (throttled)
                if last_position_update.elapsed() >= position_update_interval {
                    if let Some(ref track) = current_track {
                        let current_pos = state.player.current_position();
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
        
        // Handle end of stream outside the borrow
        if end_of_stream {
            playback = None;
            current_track = None;
            let _ = event_tx.send(AudioEvent::Finished);
        }
        
        // Small sleep to avoid busy-waiting
        thread::sleep(Duration::from_millis(1));
    }
    
    Ok(())
}

/// Load and start playing an audio file
fn load_and_play(
    path: &PathBuf,
    equalizer: Arc<Mutex<Equalizer>>,
    capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
) -> Result<PlaybackState> {
    // Create player
    let player = SymphoniaPlayer::load(path, equalizer, capture_buffer)
        .context("Failed to load audio file")?;
    
    // Create output
    let output = CpalOutput::new(player.sample_rate(), player.channels())
        .context("Failed to create audio output")?;
    
    Ok(PlaybackState {
        player,
        output,
        is_paused: false,
    })
}
