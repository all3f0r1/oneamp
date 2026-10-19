//! Band-limited resampling (rubato FFT) and per-media format conversion.
//!
//! Two users: the cpal output, when a device refuses the stream's rate
//! (one continuous resampler for the life of the output), and the engine
//! under [`crate::OutputPolicy::Fixed`], where every media is converted to
//! the fixed format before the chain so transitions between different
//! rates stay gapless and crossfades mix like with like.

use crate::dsp::channels::{ChannelLayout, StereoDownmix, map_channels};
use crate::error::PlayerError;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// Input chunk of the resampler, in frames.
const CHUNK: usize = 1024;

/// Fixed-ratio resampler with the input buffering rubato's fixed chunk
/// size needs.
pub struct Resample {
    fft: Fft<f32>,
    channels: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
    chunk_out: Vec<f32>,
    frames_in: u64,
    frames_out: u64,
    /// Output frames still to drop to cancel the filter delay (per-media
    /// conversion: the media must come out exactly as long as it went in).
    skip: usize,
    trim_delay: bool,
}

impl Resample {
    /// `trim_delay`: drop the filter's leading delay so output frame n
    /// lines up with input frame n·ratio (per-media conversion). A
    /// continuous output stream keeps it (it is just latency there).
    pub fn new(from: u32, to: u32, channels: usize, trim_delay: bool) -> Result<Self, PlayerError> {
        let fft = Fft::<f32>::new(
            from as usize,
            to as usize,
            CHUNK,
            channels,
            FixedSync::Input,
        )
        .map_err(|e| PlayerError::internal(format!("resampler: {e}")))?;
        let chunk_out = vec![0.0; fft.output_frames_max() * channels];
        let skip = if trim_delay { fft.output_delay() } else { 0 };
        Ok(Self {
            fft,
            channels,
            pending: Vec::new(),
            out: Vec::new(),
            chunk_out,
            frames_in: 0,
            frames_out: 0,
            skip,
            trim_delay,
        })
    }

    pub fn ratio(&self) -> f64 {
        self.fft.resample_ratio()
    }

    /// Output frames of delay the resampler adds (0 when trimmed).
    pub fn delay_frames(&self) -> usize {
        if self.trim_delay {
            0
        } else {
            self.fft.output_delay()
        }
    }

    /// Input frames held back waiting for a full chunk.
    pub fn pending_frames(&self) -> usize {
        self.pending.len() / self.channels
    }

    /// Feed interleaved input; returns every full chunk converted so far.
    pub fn process(&mut self, input: &[f32]) -> &[f32] {
        self.pending.extend_from_slice(input);
        self.out.clear();
        let ch = self.channels;
        self.frames_in += (input.len() / ch) as u64;
        let mut offset = 0;
        loop {
            let need = self.fft.input_frames_next();
            if self.pending.len() - offset < need * ch {
                break;
            }
            let frames_out = self.chunk_out.len() / ch;
            let inp =
                InterleavedSlice::new(&self.pending[offset..offset + need * ch], ch, need).unwrap();
            let mut outp = InterleavedSlice::new_mut(&mut self.chunk_out, ch, frames_out).unwrap();
            if let Ok((_, produced)) = self.fft.process_into_buffer(&inp, &mut outp, None) {
                let mut data = &self.chunk_out[..produced * ch];
                let drop = self.skip.min(produced);
                self.skip -= drop;
                data = &data[drop * ch..];
                self.out.extend_from_slice(data);
                self.frames_out += (produced - drop) as u64;
            }
            offset += need * ch;
        }
        self.pending.drain(..offset);
        &self.out
    }

    /// End of input: push silence through until every real input frame
    /// has come out, trim the surplus, and reset.
    pub fn flush(&mut self) -> Vec<f32> {
        let mut target = (self.frames_in as f64 * self.ratio()).round() as u64;
        if !self.trim_delay {
            target += self.fft.output_delay() as u64;
        }
        let silence = vec![0.0; CHUNK * self.channels];
        let mut tail = Vec::new();
        let mut guard = 0;
        while self.frames_out < target && guard < 64 {
            tail.extend_from_slice(self.process(&silence));
            guard += 1;
        }
        let surplus = self.frames_out.saturating_sub(target) as usize * self.channels;
        tail.truncate(tail.len().saturating_sub(surplus));
        self.reset();
        tail
    }

    pub fn reset(&mut self) {
        self.fft.reset();
        self.pending.clear();
        self.frames_in = 0;
        self.frames_out = 0;
        self.skip = if self.trim_delay {
            self.fft.output_delay()
        } else {
            0
        };
    }
}

/// Converts one media's blocks to the pipeline format: rate first, then
/// channels (fold-down when wider than the target, mono duplicated,
/// stereo placed on the first pair of a wider target).
pub struct Converter {
    from_rate: u32,
    from: ChannelLayout,
    to_rate: u32,
    to_channels: usize,
    fold: Option<StereoDownmix>,
    resampler: Option<Resample>,
    mapped: Vec<f32>,
}

impl Converter {
    pub fn new(
        from_rate: u32,
        from: ChannelLayout,
        to_rate: u32,
        to_channels: u16,
    ) -> Result<Self, PlayerError> {
        let src_ch = usize::from(from.channels());
        let to = usize::from(to_channels);
        let fold = (src_ch > 2 && src_ch > to).then(|| StereoDownmix::new(&from));
        let work_ch = if fold.is_some() { 2 } else { src_ch };
        let resampler = (from_rate != to_rate)
            .then(|| Resample::new(from_rate, to_rate, work_ch, true))
            .transpose()?;
        Ok(Self {
            from_rate,
            from,
            to_rate,
            to_channels: to,
            fold,
            resampler,
            mapped: Vec::new(),
        })
    }

    pub fn matches(&self, rate: u32, layout: &ChannelLayout) -> bool {
        self.from_rate == rate && self.from == *layout
    }

    /// (rate, channels) this converter produces.
    pub fn target(&self) -> (u32, u16) {
        (self.to_rate, self.to_channels as u16)
    }

    pub fn is_identity(&self) -> bool {
        self.fold.is_none()
            && self.resampler.is_none()
            && usize::from(self.from.channels()) == self.to_channels
    }

    pub fn resampling(&self) -> bool {
        self.resampler.is_some()
    }

    fn finish_channels(&mut self, data: Vec<f32>, ch: usize) -> Vec<f32> {
        if ch == self.to_channels {
            return data;
        }
        map_channels(&data, ch, self.to_channels, &mut self.mapped);
        std::mem::take(&mut self.mapped)
    }

    pub fn convert(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        let mut ch = usize::from(self.from.channels());
        if let Some(fold) = &self.fold {
            let n = fold.apply_in_place(&mut samples);
            samples.truncate(n);
            ch = 2;
        }
        if let Some(r) = self.resampler.as_mut() {
            samples = r.process(&samples).to_vec();
        }
        self.finish_channels(samples, ch)
    }

    /// End of media: the resampler's held-back frames.
    pub fn flush(&mut self) -> Vec<f32> {
        let ch = if self.fold.is_some() {
            2
        } else {
            usize::from(self.from.channels())
        };
        match self.resampler.as_mut() {
            Some(r) => {
                let tail = r.flush();
                self.finish_channels(tail, ch)
            }
            None => Vec::new(),
        }
    }

    /// After a seek: drop what was held for the old position.
    pub fn reset(&mut self) {
        if let Some(r) = self.resampler.as_mut() {
            r.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| 0.5 * (std::f32::consts::TAU * 1000.0 * n as f32 / rate).sin())
            .collect()
    }

    #[test]
    fn resampler_preserves_tone_frequency_and_level() {
        let mut r = Resample::new(44_100, 48_000, 1, false).unwrap();
        let input = tone(44_100.0, 44_100);
        let mut out = Vec::new();
        for chunk in input.chunks(777) {
            out.extend_from_slice(r.process(chunk));
        }
        assert!((46_000..=48_000).contains(&out.len()), "{}", out.len());
        let peak = out[4000..].iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak {peak}");
        let delay = r.fft.output_delay();
        out.extend(r.flush());
        assert_eq!(out.len(), 48_000 + delay);
    }

    #[test]
    fn per_media_conversion_keeps_exact_length_and_alignment() {
        // 44.1 → 48 kHz with the delay trimmed: exactly len·ratio frames,
        // and the tone's phase lines up (no leading silence).
        let mut c = Converter::new(44_100, ChannelLayout::MONO, 48_000, 1).unwrap();
        let input = tone(44_100.0, 44_100);
        let mut out = Vec::new();
        for chunk in input.chunks(1000) {
            out.extend(c.convert(chunk.to_vec()));
        }
        out.extend(c.flush());
        assert_eq!(out.len(), 48_000);
        let reference = tone(48_000.0, 48_000);
        let err = out[1000..47_000]
            .iter()
            .zip(&reference[1000..47_000])
            .fold(0.0_f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(err < 0.02, "max error {err}");
    }

    #[test]
    fn converter_folds_surround_and_upmixes_mono() {
        let six = ChannelLayout::default_for(6).unwrap();
        let mut c = Converter::new(48_000, six, 48_000, 2).unwrap();
        assert_eq!(c.convert(vec![0.1; 6 * 10]).len(), 20);
        let mut m = Converter::new(48_000, ChannelLayout::MONO, 48_000, 2).unwrap();
        assert_eq!(m.convert(vec![0.3, 0.4]), vec![0.3, 0.3, 0.4, 0.4]);
        let id = Converter::new(48_000, ChannelLayout::STEREO, 48_000, 2).unwrap();
        assert!(id.is_identity());
    }
}
