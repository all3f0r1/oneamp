use anyhow::{Context, Result, anyhow};
use std::path::Path;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

/// Symphonia-based decoder with seek support. Pure decoding: EQ,
/// visualization capture and the rest of the chain live in the engine,
/// so two players (current + crossfade-incoming) never share filter
/// state.
///
/// Gapless: symphonia 0.6 decoders trim encoder delay / padding by
/// default (`AudioDecoderOptions::gapless`), so MP3/AAC albums join
/// sample-accurately.
pub struct SymphoniaPlayer {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    sample_rate: u32,
    channels: u16,
    /// Bit depth of lossless integer sources (FLAC, ALAC, PCM). `None`
    /// for lossy codecs, whose float output never sits on an integer grid.
    source_bits: Option<u32>,
    /// Current position in seconds (from packet timestamps)
    current_position: f32,
}

impl SymphoniaPlayer {
    /// Load an audio file.
    pub fn load(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).context("Failed to open audio file")?;
        let ext = path.extension().and_then(|e| e.to_str());
        Self::load_from_source(Box::new(file), ext)
    }

    /// Load from any `MediaSource` (file, HTTP stream, …). The hint
    /// extension short-circuits codec probing — pass the file extension,
    /// or the value mapped from a stream's `Content-Type`.
    pub fn load_from_source(
        source: Box<dyn MediaSource>,
        hint_extension: Option<&str>,
    ) -> Result<Self> {
        let mss = MediaSourceStream::new(source, Default::default());

        let mut hint = Hint::new();
        if let Some(ext_str) = hint_extension {
            hint.with_extension(ext_str);
        }

        let format_reader = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .context("Failed to probe audio file")?;

        let track = format_reader
            .default_track(TrackType::Audio)
            .context("No supported audio tracks found")?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .context("No audio codec parameters")?
            .clone();
        let track_id = track.id;
        let time_base = track.time_base;

        let sample_rate = params.sample_rate.unwrap_or(44100);
        let channels = params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(2);
        let source_bits = params
            .sample_format
            .filter(|f| {
                !matches!(
                    f,
                    symphonia::core::audio::sample::SampleFormat::F32
                        | symphonia::core::audio::sample::SampleFormat::F64
                )
            })
            .and(params.bits_per_sample);

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .context("Failed to create decoder")?;

        Ok(Self {
            format_reader,
            decoder,
            track_id,
            time_base,
            sample_rate,
            channels,
            source_bits,
            current_position: 0.0,
        })
    }

    /// Seek to a specific position in seconds
    pub fn seek(&mut self, seconds: f32) -> Result<()> {
        let time = Time::try_from_secs_f64(seconds.max(0.0) as f64)
            .ok_or_else(|| anyhow!("Invalid seek position {seconds}"))?;
        let seek_to = SeekTo::Time {
            time,
            track_id: Some(self.track_id),
        };

        match self.format_reader.seek(SeekMode::Accurate, seek_to) {
            Ok(seeked_to) => {
                self.decoder.reset();
                if let Some(t) = self
                    .time_base
                    .and_then(|tb| tb.calc_time(seeked_to.actual_ts))
                {
                    self.current_position = t.as_secs_f64() as f32;
                }
                Ok(())
            }
            Err(SymphoniaError::ResetRequired) => {
                self.decoder.reset();
                Ok(())
            }
            Err(e) => Err(anyhow!("Seek failed: {}", e)),
        }
    }

    /// Reset the decoder to a known-good state after failed seeks
    /// (symphonia may leave it mid-frame). Cheap and infallible.
    pub fn reset_decoder(&mut self) {
        self.decoder.reset();
    }

    /// Decode the next packet into interleaved f32 samples.
    /// `Ok(None)` = end of stream; an empty Vec = nothing for us in this
    /// packet (other track, recoverable decode error).
    pub fn decode_next(&mut self) -> Result<Option<Vec<f32>>> {
        let packet = match self.format_reader.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => return Ok(None),
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(SymphoniaError::ResetRequired) => {
                self.decoder.reset();
                return Ok(Some(Vec::new()));
            }
            Err(e) => return Err(anyhow!("Failed to read packet: {}", e)),
        };

        if packet.track_id != self.track_id {
            return Ok(Some(Vec::new()));
        }

        // Position from the packet timestamp before decoding, so a
        // decode-error skip still leaves it aligned with the reader.
        if let Some(t) = self.time_base.and_then(|tb| tb.calc_time(packet.pts)) {
            self.current_position = t.as_secs_f64() as f32;
        }

        match self.decoder.decode(&packet) {
            Ok(decoded) => {
                let mut samples = Vec::with_capacity(decoded.samples_interleaved());
                decoded.copy_to_vec_interleaved(&mut samples);
                Ok(Some(samples))
            }
            Err(SymphoniaError::DecodeError(e)) => {
                eprintln!("Decode error: {}", e);
                Ok(Some(Vec::new()))
            }
            Err(e) => Err(anyhow!("Failed to decode packet: {}", e)),
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

    /// Integer bit depth of a lossless source, `None` for lossy.
    pub fn source_bits(&self) -> Option<u32> {
        self.source_bits
    }
}
