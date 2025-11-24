use rodio::Source;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Audio capture buffer for visualization
/// Stores the latest audio samples for visualization purposes
pub struct AudioCaptureBuffer {
    /// PCM samples (f32, interleaved stereo)
    samples: Vec<f32>,
    /// Sample rate
    sample_rate: u32,
    /// Number of channels
    channels: u16,
}

impl AudioCaptureBuffer {
    pub fn new(buffer_size: usize) -> Self {
        Self {
            samples: vec![0.0; buffer_size],
            sample_rate: 44100,
            channels: 2,
        }
    }

    /// Update the buffer with new samples
    pub fn update(&mut self, samples: &[f32], sample_rate: u32, channels: u16) {
        self.sample_rate = sample_rate;
        self.channels = channels;
        
        // Copy samples to buffer
        let copy_len = samples.len().min(self.samples.len());
        self.samples[..copy_len].copy_from_slice(&samples[..copy_len]);
        
        // Fill remaining with zeros if needed
        if copy_len < self.samples.len() {
            self.samples[copy_len..].fill(0.0);
        }
    }

    /// Get the current samples
    pub fn get_samples(&self) -> &[f32] {
        &self.samples
    }

    /// Get sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get number of channels
    pub fn channels(&self) -> u16 {
        self.channels
    }
}

/// Wrapper Source that captures audio data for visualization
pub struct AudioCaptureSource<I>
where
    I: Source<Item = i16>,
{
    inner: I,
    capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
    temp_buffer: Vec<f32>,
}

impl<I> AudioCaptureSource<I>
where
    I: Source<Item = i16>,
{
    pub fn new(inner: I, capture_buffer: Arc<Mutex<AudioCaptureBuffer>>) -> Self {
        Self {
            inner,
            capture_buffer,
            temp_buffer: Vec::with_capacity(2048),
        }
    }
}

impl<I> Iterator for AudioCaptureSource<I>
where
    I: Source<Item = i16>,
{
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        
        // Convert to f32 and store in temp buffer
        let f32_sample = sample as f32 / 32768.0;
        self.temp_buffer.push(f32_sample);
        
        // When we have enough samples, update the capture buffer
        if self.temp_buffer.len() >= 2048 {
            if let Ok(mut buffer) = self.capture_buffer.lock() {
                buffer.update(
                    &self.temp_buffer,
                    self.inner.sample_rate(),
                    self.inner.channels(),
                );
            }
            self.temp_buffer.clear();
        }
        
        Some(sample)
    }
}

impl<I> Source for AudioCaptureSource<I>
where
    I: Source<Item = i16>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}
