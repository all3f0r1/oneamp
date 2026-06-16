use anyhow::{Context, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, Track};
use symphonia::core::io::{MediaSource, MediaSourceStream};
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
    /// Load an audio file and prepare for playback. Updates the shared
    /// equalizer's sample rate so its filter coefficients match the new
    /// track — call this when a track is about to start playing.
    pub fn load(
        path: &Path,
        equalizer: Arc<Mutex<Equalizer>>,
        capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
    ) -> Result<Self> {
        Self::load_inner(path, equalizer, capture_buffer, true)
    }

    /// Load an audio file without touching the shared equalizer's sample
    /// rate. Used by gapless preload: changing the EQ rate while the
    /// previous track is still feeding the device would corrupt that
    /// track's filter state. Caller must ensure the equalizer rate is
    /// updated separately when the preloaded player actually starts
    /// driving output.
    pub fn load_for_preload(
        path: &Path,
        equalizer: Arc<Mutex<Equalizer>>,
        capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
    ) -> Result<Self> {
        Self::load_inner(path, equalizer, capture_buffer, false)
    }

    /// Load from any `MediaSource` (file, HTTP stream, …). The hint
    /// extension is used by symphonia to short-circuit codec probing
    /// — pass the file extension, or the value mapped from a stream's
    /// `Content-Type`. `update_eq_rate` mirrors [`load_inner`]'s flag.
    pub fn load_from_source(
        source: Box<dyn MediaSource>,
        hint_extension: Option<&str>,
        equalizer: Arc<Mutex<Equalizer>>,
        capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
    ) -> Result<Self> {
        Self::load_inner_source(source, hint_extension, equalizer, capture_buffer, true)
    }

    fn load_inner(
        path: &Path,
        equalizer: Arc<Mutex<Equalizer>>,
        capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
        update_eq_rate: bool,
    ) -> Result<Self> {
        // Open the file
        let file = std::fs::File::open(path).context("Failed to open audio file")?;
        let ext = path.extension().and_then(|e| e.to_str());
        Self::load_inner_source(
            Box::new(file),
            ext,
            equalizer,
            capture_buffer,
            update_eq_rate,
        )
    }

    /// Inner loader that drives symphonia from a generic `MediaSource`.
    /// File and HTTP paths converge here so the EQ/decoder/track-pick
    /// logic stays in one place.
    fn load_inner_source(
        source: Box<dyn MediaSource>,
        hint_extension: Option<&str>,
        equalizer: Arc<Mutex<Equalizer>>,
        capture_buffer: Arc<Mutex<AudioCaptureBuffer>>,
        update_eq_rate: bool,
    ) -> Result<Self> {
        // Create media source stream
        let mss = MediaSourceStream::new(source, Default::default());

        // Create hint based on file extension / mime type
        let mut hint = Hint::new();
        if let Some(ext_str) = hint_extension {
            hint.with_extension(ext_str);
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
            .make(codec_params, &decoder_opts)
            .context("Failed to create decoder")?;

        if update_eq_rate && let Ok(mut eq) = equalizer.lock() {
            eq.set_sample_rate(sample_rate as f32);
            // Drop any filter state inherited from the previous track —
            // a stereo→mono→stereo transition would otherwise leak the
            // unused channel's tail into the new stream's first samples.
            eq.reset_state();
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

    /// Reset the decoder to a known-good state. A run of failed seeks
    /// can leave the decoder mid-frame (symphonia advances internal
    /// state even on a seek it ultimately rejects), so a subsequent
    /// `decode_next` would error on garbage. Calling `Decoder::reset`
    /// flushes that residue so the decoder can resume cleanly from
    /// whatever the format reader's current position is. Cheap and
    /// infallible — safe to call defensively after any seek failure.
    pub fn reset_decoder(&mut self) {
        self.decoder.reset();
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

        // Update position from the packet's timestamp BEFORE decoding so
        // a decode-error path still leaves `current_position` aligned with
        // what the format reader thinks it is. v1 accumulated from the
        // post-decode frame count, which drifted whenever the decoder
        // returned an empty buffer (DecodeError skip path) — eventually
        // diverging from the real stream position by several seconds on
        // glitchy MP3s.
        if let Some(time_base) = self.track.codec_params.time_base {
            let time = time_base.calc_time(packet.ts());
            // `Time::seconds` is u64 and `frac` is f64 in [0,1); merge
            // into a single f32. Resolution at the 1 ms level is fine for
            // a position slider.
            self.current_position = (time.seconds as f64 + time.frac) as f32;
        }

        // Decode the packet and convert to f32 samples
        let mut samples = match self.decoder.decode(&packet) {
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

        // Apply equalizer in-place — the decoded Vec is reused as the
        // output buffer so we avoid one Vec allocation per packet on
        // the audio hot path.
        self.apply_equalizer(&mut samples);

        // Update capture buffer for visualization
        if let Ok(mut buffer) = self.capture_buffer.lock() {
            buffer.update(&samples, self.sample_rate, self.channels);
        }

        Ok(Some(samples))
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

    /// Apply the shared equalizer to `samples` in place. No-op when the
    /// EQ is disabled, the mutex is poisoned, or the channel count
    /// isn't 1 or 2 (multichannel content passes through unprocessed —
    /// surround EQ is out of scope).
    fn apply_equalizer(&self, samples: &mut [f32]) {
        let Ok(mut eq) = self.equalizer.lock() else {
            return;
        };
        if !eq.is_enabled() {
            return;
        }
        match self.channels {
            1 => eq.process_mono_in_place(samples),
            2 => eq.process_stereo_in_place(samples),
            _ => {} // pass-through for surround
        }
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
