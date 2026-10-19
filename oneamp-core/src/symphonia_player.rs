use crate::dsp::channels::ChannelLayout;
use crate::error::{ErrorCategory, PlayerError};
use crate::media::FormatHint;
use std::time::Duration;
use symphonia::core::audio::Channels;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
/// Byte source for [`crate::SourceFactory`] implementations (symphonia's
/// trait: `Read + Seek + Send + Sync`, plus seekability and length).
pub use symphonia::core::io::MediaSource;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

/// One decoded block: interleaved `f32`, with the media frame its first
/// sample sits at and the format it was decoded in (a chained Ogg or a
/// reset stream can change format mid-media).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Frame index on the source's own timeline (0 = first sample the
    /// source delivers), at `rate`.
    pub start_frame: u64,
    pub rate: u32,
    pub layout: ChannelLayout,
    pub samples: Vec<f32>,
}

impl Block {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.layout.channels()).max(1)
    }
}

/// Symphonia demuxer + decoder. Pure decoding: the chain and the output
/// live in the engine, so two players (current + incoming) never share
/// filter state.
///
/// Gapless: symphonia 0.6 decoders trim encoder delay / padding by
/// default (`AudioDecoderOptions::gapless`), from the container's
/// gapless information (LAME/Xing for MP3, `iTunSMPB`/edit lists for
/// AAC in MP4, Ogg granule positions for Vorbis). Lossless containers
/// (FLAC, WAV, AIFF, ALAC) carry exact frame counts.
pub struct SymphoniaPlayer {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    sample_rate: u32,
    layout: ChannelLayout,
    source_bits: Option<u32>,
    codec: String,
    total_frames: Option<u64>,
    seekable: bool,
    /// Frame the next emitted sample sits at.
    next_frame: u64,
    /// After an accurate seek: drop everything before this frame.
    trim_until: Option<u64>,
    /// Packets dropped because they would not decode.
    pub decode_errors: u64,
}

fn layout_of(channels: Option<&Channels>) -> Option<ChannelLayout> {
    match channels? {
        Channels::Positioned(p) => ChannelLayout::from_mask(p.bits()),
        other => ChannelLayout::default_for(other.count() as u16),
    }
}

fn decode_error(context: &str, e: SymphoniaError) -> PlayerError {
    match e {
        SymphoniaError::IoError(io) => PlayerError::from_io(&io),
        SymphoniaError::Unsupported(what) => {
            PlayerError::unsupported(format!("{context}: unsupported {what}"))
        }
        other => PlayerError::new(ErrorCategory::Decode, format!("{context}: {other}")),
    }
}

impl SymphoniaPlayer {
    /// Load a local file.
    pub fn load(path: &std::path::Path) -> Result<Self, PlayerError> {
        let file = std::fs::File::open(path)
            .map_err(|e| PlayerError::from_io(&e).with_recoverable(false))?;
        let hint = FormatHint {
            extension: path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_string),
            mime_type: None,
        };
        Self::open(Box::new(file), &hint)
    }

    /// Probe any `MediaSource` (file, HTTP, host-supplied).
    pub fn open(source: Box<dyn MediaSource>, hint: &FormatHint) -> Result<Self, PlayerError> {
        let seekable = source.is_seekable();
        let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
        let mut probe_hint = Hint::new();
        if let Some(ext) = hint.extension() {
            probe_hint.with_extension(&ext);
        }
        if let Some(mime) = &hint.mime_type {
            probe_hint.mime_type(mime);
        }
        let format_reader = symphonia::default::get_probe()
            .probe(
                &probe_hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| match e {
                SymphoniaError::IoError(io) => PlayerError::from_io(&io),
                other => PlayerError::unsupported(format!("unrecognised media format ({other})")),
            })?;
        Self::from_reader(format_reader, seekable)
    }

    fn from_reader(
        format_reader: Box<dyn FormatReader>,
        seekable: bool,
    ) -> Result<Self, PlayerError> {
        let track = format_reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| PlayerError::unsupported("no audio track"))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| PlayerError::unsupported("no audio codec parameters"))?
            .clone();
        let track_id = track.id;
        let time_base = track.time_base;
        let total_frames = track.num_frames;

        let sample_rate = params
            .sample_rate
            .filter(|&r| r > 0)
            .ok_or_else(|| PlayerError::unsupported("unknown sample rate"))?;
        let layout = layout_of(params.channels.as_ref())
            .or_else(|| ChannelLayout::default_for(2))
            .expect("stereo layout");
        // Demuxers set `bits_per_sample` only for integer PCM-style
        // codecs; FLAC leaves `sample_format` unset, so don't require it.
        // Above 24 bits the f32 pipeline can't carry every value.
        let is_float = matches!(
            params.sample_format,
            Some(
                symphonia::core::audio::sample::SampleFormat::F32
                    | symphonia::core::audio::sample::SampleFormat::F64
            )
        );
        let source_bits = params.bits_per_sample.filter(|&b| !is_float && b <= 24);
        let codec = symphonia::default::get_codecs()
            .get_audio_decoder(params.codec)
            .map(|d| d.codec.info.short_name.to_uppercase())
            .unwrap_or_else(|| params.codec.to_string().to_uppercase());

        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| decode_error("decoder", e))?;

        Ok(Self {
            format_reader,
            decoder,
            track_id,
            time_base,
            sample_rate,
            layout,
            source_bits,
            codec,
            total_frames,
            seekable,
            next_frame: 0,
            trim_until: None,
            decode_errors: 0,
        })
    }

    /// Metadata revisions read while probing (tags).
    pub(crate) fn format_reader_mut(&mut self) -> &mut dyn FormatReader {
        self.format_reader.as_mut()
    }

    fn ts_to_frame(&self, ts: Timestamp) -> u64 {
        let t = ts.get().max(0) as u64;
        match self.time_base {
            Some(tb) => {
                let (n, d) = (u64::from(tb.numer.get()), u64::from(tb.denom.get()));
                if n == 1 && d == u64::from(self.sample_rate) {
                    t
                } else {
                    ((t as u128 * n as u128 * self.sample_rate as u128 + d as u128 / 2) / d as u128)
                        as u64
                }
            }
            None => t,
        }
    }

    /// Position the stream so the next block starts exactly at `frame`.
    /// The demuxer finds the packet (its index, or a bisection over the
    /// byte source); samples between that packet's start and `frame` are
    /// decoded and dropped. Returns the frame actually reached (`frame`,
    /// or the end of the media when `frame` lies past it).
    pub fn seek_to_frame(&mut self, frame: u64) -> Result<u64, PlayerError> {
        if !self.seekable {
            return Err(PlayerError::unsupported("this source cannot seek"));
        }
        if let Some(total) = self.total_frames
            && frame >= total
        {
            // Past the end: there is nothing left to play.
            self.next_frame = total;
            self.trim_until = Some(u64::MAX);
            return Ok(total);
        }
        let secs = frame as f64 / f64::from(self.sample_rate);
        let time = Time::try_from_secs_f64(secs)
            .ok_or_else(|| PlayerError::invalid(format!("bad seek position {secs}s")))?;
        let to = SeekTo::Time {
            time,
            track_id: Some(self.track_id),
        };
        match self.format_reader.seek(SeekMode::Accurate, to) {
            Ok(_) => {
                self.decoder.reset();
                self.trim_until = Some(frame);
                self.next_frame = frame;
                Ok(frame)
            }
            Err(SymphoniaError::ResetRequired) => {
                self.decoder.reset();
                self.trim_until = Some(frame);
                self.next_frame = frame;
                Ok(frame)
            }
            Err(SymphoniaError::SeekError(_)) if self.total_frames.is_none() => {
                Err(PlayerError::unsupported("seek target outside the media"))
            }
            Err(e) => Err(decode_error("seek", e)),
        }
    }

    /// Decode the next block. `Ok(None)` = normal end of stream;
    /// recoverable decode errors skip the packet (counted in
    /// [`decode_errors`](Self::decode_errors)); transport errors come
    /// back as the [`PlayerError`] the source raised.
    pub fn decode_next(&mut self) -> Result<Option<Block>, PlayerError> {
        loop {
            if self.trim_until == Some(u64::MAX) {
                return Ok(None);
            }
            let packet = match self.format_reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                // Symphonia reports the end of most containers as an
                // unexpected EOF from the reader. Our network sources
                // never raise a bare UnexpectedEof for a cut transfer
                // (they raise a categorised error), so this is the end.
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof && e.get_ref().is_none() =>
                {
                    return Ok(None);
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.reset_track()?;
                    continue;
                }
                Err(e) => return Err(decode_error("read", e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let start = self.ts_to_frame(packet.pts);
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) => {
                    self.decode_errors += 1;
                    continue;
                }
                Err(e) => return Err(decode_error("decode", e)),
            };
            let mut samples = Vec::with_capacity(decoded.samples_interleaved());
            decoded.copy_to_vec_interleaved(&mut samples);
            let ch = usize::from(self.layout.channels()).max(1);
            if samples.len() % ch != 0 {
                samples.truncate(samples.len() - samples.len() % ch);
            }
            let mut start_frame = start;
            if let Some(target) = self.trim_until {
                let frames = (samples.len() / ch) as u64;
                if start + frames <= target {
                    continue;
                }
                if start < target {
                    let skip = ((target - start) as usize) * ch;
                    samples.drain(..skip);
                    start_frame = target;
                }
                self.trim_until = None;
            }
            if samples.is_empty() {
                continue;
            }
            self.next_frame = start_frame + (samples.len() / ch) as u64;
            return Ok(Some(Block {
                start_frame,
                rate: self.sample_rate,
                layout: self.layout,
                samples,
            }));
        }
    }

    /// The demuxer announced new codec parameters (chained Ogg): rebuild
    /// the decoder; later blocks carry the new format.
    fn reset_track(&mut self) -> Result<(), PlayerError> {
        let track = self
            .format_reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| PlayerError::unsupported("no audio track after reset"))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| PlayerError::unsupported("no codec parameters after reset"))?
            .clone();
        self.track_id = track.id;
        self.time_base = track.time_base;
        if let Some(r) = params.sample_rate.filter(|&r| r > 0) {
            self.sample_rate = r;
        }
        if let Some(l) = layout_of(params.channels.as_ref()) {
            self.layout = l;
        }
        self.decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| decode_error("decoder", e))?;
        Ok(())
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.layout.channels()
    }

    pub fn layout(&self) -> ChannelLayout {
        self.layout
    }

    /// Integer bit depth of a lossless source (≤ 24 bits), `None` for
    /// lossy codecs and wider PCM.
    pub fn source_bits(&self) -> Option<u32> {
        self.source_bits
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    pub fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }

    pub fn duration(&self) -> Option<Duration> {
        self.total_frames
            .map(|f| Duration::from_secs_f64(f as f64 / f64::from(self.sample_rate)))
    }

    pub fn is_seekable(&self) -> bool {
        self.seekable
    }

    /// Frame the next block starts at.
    pub fn position_frames(&self) -> u64 {
        self.next_frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> SymphoniaPlayer {
        let bytes = include_bytes!("../tests/fixtures/tone16.flac").to_vec();
        SymphoniaPlayer::open(
            Box::new(std::io::Cursor::new(bytes)),
            &FormatHint {
                extension: Some("flac".into()),
                mime_type: None,
            },
        )
        .unwrap()
    }

    fn drain(p: &mut SymphoniaPlayer) -> Vec<f32> {
        let mut all = Vec::new();
        while let Some(b) = p.decode_next().unwrap() {
            all.extend(b.samples);
        }
        all
    }

    #[test]
    fn flac_is_recognised_as_exact_16_bit() {
        let mut p = tone();
        assert_eq!(p.source_bits(), Some(16));
        let all = drain(&mut p);
        for s in &all {
            let v = s * 32768.0;
            assert_eq!(v, v.round(), "off the 16-bit grid");
        }
        assert_eq!(all.len(), 2 * 2205);
    }

    #[test]
    fn accurate_seek_lands_on_the_exact_frame() {
        let reference = drain(&mut tone());
        for target in [0u64, 1, 777, 1500, 2204] {
            let mut p = tone();
            assert_eq!(p.seek_to_frame(target).unwrap(), target);
            let first = p.decode_next().unwrap().unwrap();
            assert_eq!(first.start_frame, target);
            let mut got = first.samples;
            got.extend(drain(&mut p));
            assert_eq!(got, reference[(target as usize) * 2..], "target {target}");
        }
    }

    #[test]
    fn seek_past_the_end_ends_the_stream() {
        let mut p = tone();
        assert_eq!(p.seek_to_frame(10_000).unwrap(), 2205);
        assert!(p.decode_next().unwrap().is_none());
    }
}
