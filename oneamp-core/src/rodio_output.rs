use anyhow::{Context, Result};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Audio output using rodio (which wraps cpal with better ALSA handling)
pub struct RodioOutput {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Arc<Mutex<Sink>>,
    sample_buffer: Arc<Mutex<VecDeque<f32>>>,
    sample_rate: u32,
    channels: u16,
}

/// A source that reads from a shared buffer
struct StreamingSource {
    buffer: Arc<Mutex<VecDeque<f32>>>,
    sample_rate: u32,
    channels: u16,
    finished: bool,
}

impl StreamingSource {
    fn new(buffer: Arc<Mutex<VecDeque<f32>>>, sample_rate: u32, channels: u16) -> Self {
        Self {
            buffer,
            sample_rate,
            channels,
            finished: false,
        }
    }
}

impl Iterator for StreamingSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.pop_front()
        } else {
            None
        }
    }
}

impl Source for StreamingSource {
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
        
        let sample_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(sample_rate as usize * 2)));
        
        // Create a streaming source that will continuously read from the buffer
        let source = StreamingSource::new(sample_buffer.clone(), sample_rate, channels);
        sink.append(source);
        
        eprintln!("RodioOutput created successfully");
        
        Ok(Self {
            _stream: stream,
            stream_handle,
            sink: Arc::new(Mutex::new(sink)),
            sample_buffer,
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
        if let Ok(mut buffer) = self.sample_buffer.lock() {
            buffer.clear();
        }
    }

    /// Get the number of samples in the buffer
    pub fn buffer_len(&self) -> usize {
        self.sample_buffer.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Check if the buffer needs more data
    pub fn needs_data(&self) -> bool {
        // Keep at least 0.5 seconds of audio in the buffer
        let min_buffer_size = (self.sample_rate as usize * self.channels as usize) / 2;
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
