use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig, SampleFormat};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Audio output using cpal
pub struct CpalOutput {
    stream: Stream,
    sample_buffer: Arc<Mutex<VecDeque<f32>>>,
    is_playing: Arc<Mutex<bool>>,
    sample_rate: u32,
    channels: u16,
}

impl CpalOutput {
    /// Create a new audio output
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No output device available")?;
        
        // Get the default config and try to use it as a base
        let default_config = device
            .default_output_config()
            .context("Failed to get default output config")?;
        
        eprintln!("Default output config: {:?}", default_config);
        eprintln!("Requested: sample_rate={}, channels={}", sample_rate, channels);
        
        // Try to use the requested config, but fall back to default if needed
        let config = StreamConfig {
            channels: channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        
        // Check if the device supports f32 format
        let supported_configs = device
            .supported_output_configs()
            .context("Failed to get supported output configs")?;
        
        let mut found_f32 = false;
        for supported_config in supported_configs {
            if supported_config.sample_format() == SampleFormat::F32 {
                found_f32 = true;
                eprintln!("Found F32 support: {:?}", supported_config);
            }
        }
        
        if !found_f32 {
            eprintln!("Warning: F32 format may not be supported, trying anyway...");
        }
        
        let sample_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(sample_rate as usize)));
        let sample_buffer_clone = sample_buffer.clone();
        
        let is_playing = Arc::new(Mutex::new(true));
        let is_playing_clone = is_playing.clone();
        
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut buffer = sample_buffer_clone.lock().unwrap();
                let playing = *is_playing_clone.lock().unwrap();
                
                if !playing {
                    // Output silence when paused
                    data.fill(0.0);
                    return;
                }
                
                for sample in data.iter_mut() {
                    *sample = buffer.pop_front().unwrap_or(0.0);
                }
            },
            |err| {
                eprintln!("Audio stream error: {}", err);
            },
            None,
        ).context("Failed to build output stream")?;
        
        // Start the stream
        stream.play().context("Failed to start stream")?;
        
        eprintln!("CpalOutput created successfully");
        
        Ok(Self {
            stream,
            sample_buffer,
            is_playing,
            sample_rate,
            channels,
        })
    }
    
    /// Write samples to the output buffer
    pub fn write_samples(&self, samples: &[f32]) {
        if let Ok(mut buffer) = self.sample_buffer.lock() {
            buffer.extend(samples.iter().copied());
        }
    }
    
    /// Play the stream
    pub fn play(&self) -> Result<()> {
        if let Ok(mut playing) = self.is_playing.lock() {
            *playing = true;
        }
        self.stream.play()?;
        Ok(())
    }
    
    /// Pause the stream
    pub fn pause(&self) -> Result<()> {
        if let Ok(mut playing) = self.is_playing.lock() {
            *playing = false;
        }
        Ok(())
    }
    
    /// Clear the buffer
    pub fn clear(&self) {
        if let Ok(mut buffer) = self.sample_buffer.lock() {
            buffer.clear();
        }
    }
    
    /// Get the number of samples in the buffer
    pub fn buffer_len(&self) -> usize {
        self.sample_buffer.lock().map(|b| b.len()).unwrap_or(0)
    }
    
    /// Check if the buffer is nearly empty (needs more data)
    pub fn needs_data(&self) -> bool {
        self.buffer_len() < (self.sample_rate as usize * self.channels as usize / 4)
    }
    
    /// Get sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    
    /// Get channels
    pub fn channels(&self) -> u16 {
        self.channels
    }
}
