use anyhow::{Context, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use crate::audio_capture::AudioCaptureBuffer;
use crate::equalizer::Equalizer;

/// Symphonia-based audio player with seek support
pub struct SymphoniaPlayer {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    track: Track,
    sample_rate: u32,
    channels: u16,
    equalizer: Arc<Mutex<Equalizer>>,
    capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
    /// Current position in seconds (approximation)
    current_position: f32,
}

impl SymphoniaPlayer {
    /// Load an audio file and prepare for playback
    pub fn load(
        path: &Path,
        equalizer: Arc<Mutex<Equalizer>>,
        capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
    ) -> Result<Self> {
        // Open the file
        let file = std::fs::File::open(path).context("Failed to open audio file")?;

        // Create media source stream
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        // Create hint based on file extension
        let mut hint = Hint::new();
        if let Some(ext) = path.extension() {
            if let Some(ext_str) = ext.to_str() {
                hint.with_extension(ext_str);
            }
        }

        // Probe the media source
        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .context("Failed to probe audio file")?;

        let format_reader = probed.format;

        // Find the first supported audio track
        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .context("No supported audio tracks found")?
            .clone();

        let track_id = track.id;

        // Get codec parameters
        let codec_params = &track.codec_params;
        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

        // Create decoder
        let decoder_opts = DecoderOptions::default();
        let decoder = symphonia::default::get_codecs()
            .make(&codec_params, &decoder_opts)
            .context("Failed to create decoder")?;

        // Update equalizer sample rate
        if let Ok(mut eq) = equalizer.lock() {
            eq.set_sample_rate(sample_rate as f32);
        }

        Ok(Self {
            format_reader,
            decoder,
            track_id,
            track,
            sample_rate,
            channels,
            equalizer,
            capture_buffer,
            current_position: 0.0,
        })
    }

    /// Seek to a specific position in seconds
    pub fn seek(&mut self, seconds: f32) -> Result<()> {
        let time = Time::from(seconds as f64);

        let seek_to = SeekTo::Time {
            time,
            track_id: Some(self.track_id),
        };

        // Perform the seek
        match self.format_reader.seek(SeekMode::Accurate, seek_to) {
            Ok(seeked_to) => {
                // Reset the decoder after seeking
                self.decoder.reset();

                // Update current position
                let time_base = self
                    .track
                    .codec_params
                    .time_base
                    .unwrap_or(TimeBase::new(1, self.sample_rate));
                self.current_position = time_base.calc_time(seeked_to.actual_ts).seconds as f32;

                Ok(())
            }
            Err(SymphoniaError::ResetRequired) => {
                // Some formats require a reset
                self.decoder.reset();
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("Seek failed: {}", e)),
        }
    }

    /// Decode the next packet and return audio samples
    /// Returns None if end of stream
    pub fn decode_next(&mut self) -> Result<Option<Vec<f32>>> {
        // Get the next packet
        let packet = match self.format_reader.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(None); // End of stream
            }
            Err(SymphoniaError::ResetRequired) => {
                // Track changed, need to recreate decoder
                self.decoder.reset();
                return Ok(Some(Vec::new())); // Return empty buffer
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to read packet: {}", e));
            }
        };

        // Only decode packets for our track
        if packet.track_id() != self.track_id {
            return Ok(Some(Vec::new())); // Skip packets from other tracks
        }

        // Decode the packet and convert to f32 samples
        let samples = match self.decoder.decode(&packet) {
            Ok(decoded) => Self::convert_audio_buffer_static(&decoded, self.channels)?,
            Err(SymphoniaError::DecodeError(e)) => {
                // Skip decode errors and continue
                eprintln!("Decode error: {}", e);
                return Ok(Some(Vec::new()));
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to decode packet: {}", e));
            }
        };

        // Update position estimate
        let frames = samples.len() / self.channels as usize;
        self.current_position += frames as f32 / self.sample_rate as f32;

        // Apply equalizer
        let processed_samples = self.apply_equalizer(&samples);

        // Update capture buffer for visualization
        if let Ok(mut buffer) = self.capture_buffer.lock() {
            buffer.update(&processed_samples, self.sample_rate, self.channels);
        }

        Ok(Some(processed_samples))
    }

    /// Convert AudioBufferRef to interleaved f32 samples
    #[allow(dead_code)]
    fn convert_audio_buffer(&self, buffer: &AudioBufferRef) -> Result<Vec<f32>> {
        Self::convert_audio_buffer_static(buffer, self.channels)
    }

    /// Convert AudioBufferRef to interleaved f32 samples (static version)
    fn convert_audio_buffer_static(buffer: &AudioBufferRef, _channels: u16) -> Result<Vec<f32>> {
        match buffer {
            AudioBufferRef::F32(buf) => Ok(Self::interleave_f32(buf)),
            AudioBufferRef::U8(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::U16(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::U24(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::U32(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::S8(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::S16(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::S24(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::S32(buf) => Ok(Self::interleave_generic(buf)),
            AudioBufferRef::F64(buf) => Ok(Self::interleave_generic(buf)),
        }
    }

    /// Interleave f32 audio buffer
    fn interleave_f32(buffer: &symphonia::core::audio::AudioBuffer<f32>) -> Vec<f32> {
        let num_frames = buffer.frames();
        let num_channels = buffer.spec().channels.count();
        let mut output = Vec::with_capacity(num_frames * num_channels);

        for frame_idx in 0..num_frames {
            for ch_idx in 0..num_channels {
                output.push(buffer.chan(ch_idx)[frame_idx]);
            }
        }

        output
    }

    /// Interleave generic audio buffer
    fn interleave_generic<S>(buffer: &symphonia::core::audio::AudioBuffer<S>) -> Vec<f32>
    where
        S: symphonia::core::sample::Sample + symphonia::core::conv::IntoSample<f32>,
    {
        let num_frames = buffer.frames();
        let num_channels = buffer.spec().channels.count();
        let mut output = Vec::with_capacity(num_frames * num_channels);

        for frame_idx in 0..num_frames {
            for ch_idx in 0..num_channels {
                let sample = buffer.chan(ch_idx)[frame_idx];
                output.push(sample.into_sample());
            }
        }

        output
    }

    /// Apply equalizer to samples
    fn apply_equalizer(&self, samples: &[f32]) -> Vec<f32> {
        let Ok(mut eq) = self.equalizer.lock() else {
            return samples.to_vec();
        };

        if !eq.is_enabled() {
            return samples.to_vec();
        }

        let mut output = Vec::with_capacity(samples.len());

        if self.channels == 1 {
            // Mono: process as stereo (duplicate)
            for &sample in samples {
                let (left, _) = eq.process_stereo(sample, sample);
                output.push(left);
            }
        } else if self.channels == 2 {
            // Stereo: process pairs
            for chunk in samples.chunks_exact(2) {
                let (left, right) = eq.process_stereo(chunk[0], chunk[1]);
                output.push(left);
                output.push(right);
            }
        } else {
            // Multi-channel: pass through
            return samples.to_vec();
        }

        output
    }

    /// Get current position in seconds
    pub fn current_position(&self) -> f32 {
        self.current_position
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
