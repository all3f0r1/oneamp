use rodio::Source;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use crate::equalizer::Equalizer;

/// A wrapper Source that applies equalization to another Source
pub struct EqualizerSource<S>
where
    S: Source<Item = i16>,
{
    source: S,
    equalizer: Arc<Mutex<Equalizer>>,
    buffer: Vec<i16>,
    buffer_pos: usize,
}

impl<S> EqualizerSource<S>
where
    S: Source<Item = i16>,
{
    pub fn new(source: S, equalizer: Arc<Mutex<Equalizer>>) -> Self {
        // Update equalizer sample rate to match source
        if let Ok(mut eq) = equalizer.lock() {
            eq.set_sample_rate(source.sample_rate() as f32);
        }
        
        Self {
            source,
            equalizer,
            buffer: Vec::new(),
            buffer_pos: 0,
        }
    }
}

impl<S> Iterator for EqualizerSource<S>
where
    S: Source<Item = i16>,
{
    type Item = i16;
    
    fn next(&mut self) -> Option<Self::Item> {
        // If we have buffered samples, return them first
        if self.buffer_pos < self.buffer.len() {
            let sample = self.buffer[self.buffer_pos];
            self.buffer_pos += 1;
            return Some(sample);
        }
        
        // Clear buffer and reset position
        self.buffer.clear();
        self.buffer_pos = 0;
        
        // Get number of channels
        let channels = self.source.channels();
        
        if channels == 1 {
            // Mono: process single sample
            if let Some(sample) = self.source.next() {
                if let Ok(mut eq) = self.equalizer.lock() {
                    let sample_f32 = sample as f32 / 32768.0;
                    let (left, _) = eq.process_stereo(sample_f32, sample_f32);
                    Some((left * 32768.0).clamp(-32768.0, 32767.0) as i16)
                } else {
                    Some(sample)
                }
            } else {
                None
            }
        } else if channels == 2 {
            // Stereo: process pair of samples
            let left = self.source.next()?;
            let right = self.source.next()?;
            
            if let Ok(mut eq) = self.equalizer.lock() {
                let left_f32 = left as f32 / 32768.0;
                let right_f32 = right as f32 / 32768.0;
                let (left_out, right_out) = eq.process_stereo(left_f32, right_f32);
                self.buffer.push((left_out * 32768.0).clamp(-32768.0, 32767.0) as i16);
                self.buffer.push((right_out * 32768.0).clamp(-32768.0, 32767.0) as i16);
            } else {
                self.buffer.push(left);
                self.buffer.push(right);
            }
            
            // Return first sample
            self.buffer_pos = 1;
            Some(self.buffer[0])
        } else {
            // Multi-channel: pass through without processing
            self.source.next()
        }
    }
}

impl<S> Source for EqualizerSource<S>
where
    S: Source<Item = i16>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.source.current_frame_len()
    }
    
    fn channels(&self) -> u16 {
        self.source.channels()
    }
    
    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
    
    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }
}
