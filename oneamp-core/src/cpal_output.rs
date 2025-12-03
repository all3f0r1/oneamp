use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
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
    pub fn new(requested_sample_rate: u32, requested_channels: u16) -> Result<Self> {
        eprintln!(
            "CpalOutput::new - Requested: sample_rate={}, channels={}",
            requested_sample_rate, requested_channels
        );

        let host = cpal::default_host();
        eprintln!("Using audio host: {:?}", host.id());

        let device = host
            .default_output_device()
            .context("No output device available")?;

        eprintln!("Output device: {:?}", device.name());

        // Get the default config
        let default_config = device
            .default_output_config()
            .context("Failed to get default output config")?;

        eprintln!("Default output config: {:?}", default_config);

        // Use the default config as a base, but try to match requested parameters
        let sample_rate = requested_sample_rate;
        let channels = requested_channels;

        // Try to build stream with requested config first
        match Self::try_build_stream(&device, sample_rate, channels) {
            Ok((stream, actual_sample_rate, actual_channels)) => {
                eprintln!(
                    "Successfully created stream with sample_rate={}, channels={}",
                    actual_sample_rate, actual_channels
                );

                let sample_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(
                    actual_sample_rate as usize,
                )));
                let is_playing = Arc::new(Mutex::new(true));

                Ok(Self {
                    stream,
                    sample_buffer,
                    is_playing,
                    sample_rate: actual_sample_rate,
                    channels: actual_channels,
                })
            }
            Err(e) => {
                eprintln!("Failed to create stream with requested config: {}", e);

                // Fallback: try with default config
                eprintln!("Trying fallback with default config...");
                let fallback_sample_rate = default_config.sample_rate().0;
                let fallback_channels = default_config.channels();

                match Self::try_build_stream(&device, fallback_sample_rate, fallback_channels) {
                    Ok((stream, actual_sample_rate, actual_channels)) => {
                        eprintln!("Successfully created stream with fallback config: sample_rate={}, channels={}", actual_sample_rate, actual_channels);

                        let sample_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(
                            actual_sample_rate as usize,
                        )));
                        let is_playing = Arc::new(Mutex::new(true));

                        Ok(Self {
                            stream,
                            sample_buffer,
                            is_playing,
                            sample_rate: actual_sample_rate,
                            channels: actual_channels,
                        })
                    }
                    Err(e2) => Err(anyhow::anyhow!(
                        "Failed to create audio output: {} (fallback also failed: {})",
                        e,
                        e2
                    )),
                }
            }
        }
    }

    /// Try to build an output stream with the given parameters
    fn try_build_stream(
        device: &cpal::Device,
        sample_rate: u32,
        channels: u16,
    ) -> Result<(Stream, u32, u16)> {
        let config = StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let sample_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(sample_rate as usize)));
        let sample_buffer_clone = sample_buffer.clone();

        let is_playing = Arc::new(Mutex::new(true));
        let is_playing_clone = is_playing.clone();

        let stream = device
            .build_output_stream(
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
            )
            .context("Failed to build output stream")?;

        // Start the stream
        stream.play().context("Failed to start stream")?;

        Ok((stream, sample_rate, channels))
    }

    /// Write samples to the output buffer
    /// If the sample rate or channels don't match, this will need resampling
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
        // Keep at least 0.25 seconds of audio in the buffer
        let min_buffer_size = (self.sample_rate as usize * self.channels as usize) / 4;
        self.buffer_len() < min_buffer_size
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
