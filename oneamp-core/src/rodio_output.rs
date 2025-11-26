use anyhow::{Context, Result};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Audio output using rodio (which wraps cpal with better ALSA handling)
pub struct RodioOutput {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Arc<Mutex<Sink>>,
    sample_rate: u32,
    channels: u16,
}

/// A simple source that reads from a buffer
struct BufferSource {
    buffer: Arc<Mutex<Vec<f32>>>,
    position: usize,
    sample_rate: u32,
    channels: u16,
}

impl BufferSource {
    fn new(buffer: Arc<Mutex<Vec<f32>>>, sample_rate: u32, channels: u16) -> Self {
        Self {
            buffer,
            position: 0,
            sample_rate,
            channels,
        }
    }
}

impl Iterator for BufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if let Ok(buffer) = self.buffer.lock() {
            if self.position < buffer.len() {
                let sample = buffer[self.position];
                self.position += 1;
                Some(sample)
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl Source for BufferSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl RodioOutput {
    /// Create a new audio output using rodio
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        eprintln!("RodioOutput::new - sample_rate={}, channels={}", sample_rate, channels);
        
        let (stream, stream_handle) = OutputStream::try_default()
            .context("Failed to get default audio output device")?;
        
        let sink = Sink::try_new(&stream_handle)
            .context("Failed to create audio sink")?;
        
        eprintln!("RodioOutput created successfully");
        
        Ok(Self {
            _stream: stream,
            stream_handle,
            sink: Arc::new(Mutex::new(sink)),
            sample_rate,
            channels,
        })
    }

    /// Write samples to the output
    pub fn write_samples(&self, samples: &[f32]) {
        if let Ok(sink) = self.sink.lock() {
            // Convert samples to i16 for rodio
            let buffer = Arc::new(Mutex::new(samples.to_vec()));
            let source = BufferSource::new(buffer, self.sample_rate, self.channels);
            sink.append(source);
        }
    }

    /// Play the stream
    pub fn play(&self) -> Result<()> {
        if let Ok(sink) = self.sink.lock() {
            sink.play();
        }
        Ok(())
    }

    /// Pause the stream
    pub fn pause(&self) -> Result<()> {
        if let Ok(sink) = self.sink.lock() {
            sink.pause();
        }
        Ok(())
    }

    /// Clear the buffer
    pub fn clear(&self) {
        if let Ok(mut sink) = self.sink.lock() {
            // Create a new sink to clear the buffer
            if let Ok(new_sink) = Sink::try_new(&self.stream_handle) {
                let was_paused = sink.is_paused();
                *sink = new_sink;
                if was_paused {
                    sink.pause();
                }
            }
        }
    }

    /// Get the number of samples in the buffer (approximation)
    pub fn buffer_len(&self) -> usize {
        // Rodio doesn't expose buffer length, so we approximate
        0
    }

    /// Check if the buffer needs more data
    pub fn needs_data(&self) -> bool {
        if let Ok(sink) = self.sink.lock() {
            // If sink is empty, we need more data
            sink.empty()
        } else {
            true
        }
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
