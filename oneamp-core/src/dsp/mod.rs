//! The PCM processing chain, usable on its own.
//!
//! [`Processor`] is what a host that keeps control of playback (a
//! platform player, an `AudioWorklet`, a Media3 `AudioProcessor`) feeds
//! blocks of interleaved `f32` through. It owns no thread, no device and
//! no network access; everything it needs is allocated by [`Processor::new`]
//! and [`Processor::configure`], so [`Processor::process`],
//! [`Processor::set_params`] and [`Processor::set_media_gain`] never
//! allocate and never block — they are safe on a real-time audio thread.
//!
//! The full player ([`crate::AudioEngine`]) runs the same chain: one
//! [`TrackGain`] per decoded stream (so two crossfading tracks keep their
//! own normalisation) mixed into one [`Chain`].
//!
//! Order: per-track gain → fold-down (wider than stereo) → 10-band EQ →
//! loudness → mono → preamp + volume → balance → lookahead limiter.
//!
//! Accepted layouts: mono and stereo get every stage. Wider layouts are
//! folded to stereo ([`MultichannelMode::FoldToStereo`], the default) or
//! kept at full width ([`MultichannelMode::Passthrough`]), where only
//! the gain stages and the limiter apply — EQ, loudness, balance and
//! mono are stereo-only and [`ActiveProcessing`] reports them inactive.

pub mod channels;
pub mod stages;

use crate::equalizer::{EQ_MAX_DB, Equalizer};
use channels::{ChannelLayout, MAX_CHANNELS, StereoDownmix};
use stages::{Balance, GainRamp, Limiter, Loudness, mono_downmix};

/// Highest master volume: 1.5 = 150 % (+3.5 dB). The part above 1.0 is
/// applied ahead of the limiter, so loud masters are limited rather
/// than clipped.
pub const MAX_VOLUME: f32 = 1.5;

/// Which ReplayGain figure (if any) is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplayGainMode {
    /// No ReplayGain.
    Off,
    /// Track gain only (per-track normalisation). The default.
    #[default]
    Track,
    /// Album gain only; no gain when the album figure is missing
    /// (falling back to track gain would defeat album normalisation).
    Album,
    /// Chosen from the [`ListeningContext`]: album gain while an album
    /// plays in order, track gain for mixed or shuffled listening, each
    /// falling back to the other. With [`ListeningContext::Unknown`],
    /// album gain when present, else track gain.
    Auto,
}

/// How the media is being listened to, for [`ReplayGainMode::Auto`].
/// The host knows this (it owns the queue); the chain does not guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ListeningContext {
    #[default]
    Unknown,
    /// Consecutive tracks of one album, in album order.
    Album,
    /// Shuffle, a playlist mixing albums, a single track.
    Mixed,
}

/// Normalisation figures for one media, from its tags or supplied by the
/// host (a server that already measured the file). Gains in dB, peaks
/// linear (1.0 = full scale).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MediaGain {
    pub track_gain_db: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain_db: Option<f32>,
    pub album_peak: Option<f32>,
}

impl ReplayGainMode {
    /// Effective gain in dB. `0.0` when off or when the relevant figure
    /// is missing — summing zero leaves the signal untouched.
    pub fn gain_db(self, track_gain_db: Option<f32>, album_gain_db: Option<f32>) -> f32 {
        self.resolve(
            &MediaGain {
                track_gain_db,
                album_gain_db,
                ..Default::default()
            },
            ListeningContext::Unknown,
        )
        .0
    }

    /// (gain dB, peak of the figure used).
    pub fn resolve(self, g: &MediaGain, ctx: ListeningContext) -> (f32, Option<f32>) {
        let track = g.track_gain_db.map(|v| (v, g.track_peak));
        let album = g.album_gain_db.map(|v| (v, g.album_peak));
        let pick = match self {
            ReplayGainMode::Off => None,
            ReplayGainMode::Track => track,
            ReplayGainMode::Album => album,
            ReplayGainMode::Auto => match ctx {
                ListeningContext::Mixed => track.or(album),
                ListeningContext::Album | ListeningContext::Unknown => album.or(track),
            },
        };
        pick.filter(|(v, _)| v.is_finite()).unwrap_or((0.0, None))
    }
}

/// What a fold-down does with sources wider than stereo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MultichannelMode {
    /// Fold to stereo (ITU-R BS.775 weights, normalised so the fold
    /// cannot clip) and run the whole chain.
    #[default]
    FoldToStereo,
    /// Keep every channel; only the gain stages and the limiter run.
    Passthrough,
}

/// Every user-facing parameter of the chain. `Copy`, so applying a whole
/// set is one assignment: there is never a half-applied state.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DspParams {
    pub eq_enabled: bool,
    /// One gain per [`crate::EQ_FREQUENCIES`] band, ±[`EQ_MAX_DB`].
    pub eq_gains_db: [f32; 10],
    /// Applied after the EQ, ±[`EQ_MAX_DB`].
    pub preamp_db: f32,
    pub replaygain: ReplayGainMode,
    /// Cap the ReplayGain so the media's own peak stays at or under full
    /// scale. Off by default: the limiter already catches overs, and
    /// capping makes peaky tracks quieter than their neighbours.
    pub replaygain_prevent_clipping: bool,
    /// Linear volume applied inside the chain, ahead of the limiter
    /// (0..=[`MAX_VOLUME`]). The player keeps attenuation in the output
    /// callback and passes only the part above unity here.
    pub volume: f32,
    /// -1 (left) ..= 1 (right).
    pub balance: f32,
    pub mono: bool,
    pub loudness: bool,
    /// Listening volume the loudness curve follows (0..=1). Separate from
    /// `volume` because a host may attenuate elsewhere.
    pub loudness_volume: f32,
    pub multichannel: MultichannelMode,
}

impl Default for DspParams {
    fn default() -> Self {
        Self {
            eq_enabled: false,
            eq_gains_db: [0.0; 10],
            preamp_db: 0.0,
            replaygain: ReplayGainMode::Off,
            replaygain_prevent_clipping: false,
            volume: 1.0,
            balance: 0.0,
            mono: false,
            loudness: false,
            loudness_volume: 1.0,
            multichannel: MultichannelMode::FoldToStereo,
        }
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl DspParams {
    /// Clamp every field into range and replace non-finite values, so a
    /// host bug cannot push NaN into the filters.
    pub fn sanitized(mut self) -> Self {
        for g in &mut self.eq_gains_db {
            *g = finite_or(*g, 0.0).clamp(-EQ_MAX_DB, EQ_MAX_DB);
        }
        self.preamp_db = finite_or(self.preamp_db, 0.0).clamp(-EQ_MAX_DB, EQ_MAX_DB);
        self.volume = finite_or(self.volume, 1.0).clamp(0.0, MAX_VOLUME);
        self.balance = finite_or(self.balance, 0.0).clamp(-1.0, 1.0);
        self.loudness_volume = finite_or(self.loudness_volume, 1.0).clamp(0.0, 1.0);
        self
    }
}

/// Smoothed per-media normalisation gain.
#[derive(Debug, Clone)]
pub struct TrackGain {
    ramp: GainRamp,
    media: MediaGain,
    context: ListeningContext,
    applied_db: f32,
}

impl TrackGain {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            ramp: GainRamp::new(sample_rate),
            media: MediaGain::default(),
            context: ListeningContext::Unknown,
            applied_db: 0.0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.ramp.set_sample_rate(sample_rate);
    }

    /// New media: its figures apply immediately (no ramp from the
    /// previous media's gain).
    pub fn set_media(&mut self, media: MediaGain, context: ListeningContext, params: &DspParams) {
        self.media = media;
        self.context = context;
        let g = self.linear(params);
        self.ramp.snap(g);
    }

    /// Parameters changed mid-media: ramp to the new gain.
    pub fn update(&mut self, params: &DspParams) {
        let g = self.linear(params);
        self.ramp.set(g);
    }

    fn linear(&mut self, params: &DspParams) -> f32 {
        let (db, peak) = params.replaygain.resolve(&self.media, self.context);
        let mut g = 10_f32.powf(db / 20.0);
        if params.replaygain_prevent_clipping
            && let Some(p) = peak.filter(|p| *p > 0.0 && p.is_finite())
        {
            g = g.min(1.0 / p);
        }
        self.applied_db = 20.0 * g.log10();
        g
    }

    /// Gain currently targeted, in dB.
    pub fn applied_db(&self) -> f32 {
        self.applied_db
    }

    pub fn is_identity(&self) -> bool {
        self.ramp.is_identity()
    }

    pub fn process(&mut self, samples: &mut [f32], channels: usize) {
        self.ramp.process(samples, channels);
    }
}

/// What the chain is actually doing right now — reported, not assumed.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ActiveProcessing {
    pub eq: bool,
    pub loudness_db: f32,
    pub mono: bool,
    pub balance: f32,
    /// Preamp plus in-chain volume, in dB.
    pub gain_db: f32,
    pub replaygain_db: f32,
    /// The limiter attenuated something in the last block.
    pub limiter_engaged: bool,
    /// Input layout label when it was folded to stereo ("5.1", …).
    pub fold_down_from: Option<String>,
    /// Every stage is idle: the output equals the input, delayed by
    /// [`Processor::latency_frames`].
    pub bypassed: bool,
}

/// Errors from configuring a processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DspError {
    UnsupportedSampleRate(u32),
    UnsupportedChannels(u16),
}

impl std::fmt::Display for DspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DspError::UnsupportedSampleRate(r) => write!(f, "unsupported sample rate {r} Hz"),
            DspError::UnsupportedChannels(c) => {
                write!(f, "unsupported channel count {c} (1..={MAX_CHANNELS})")
            }
        }
    }
}

impl std::error::Error for DspError {}

/// Rates the chain is specified for. Outside this the EQ's top band sits
/// past Nyquist (handled, but pointless) or the filters lose precision.
pub const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 8_000..=384_000;

/// The shared stages after the per-media gain.
#[derive(Debug, Clone)]
pub struct Chain {
    sample_rate: u32,
    layout: ChannelLayout,
    mode: MultichannelMode,
    /// Set when the input is folded to stereo.
    fold: Option<StereoDownmix>,
    eq: Equalizer,
    loudness: Loudness,
    gain: GainRamp,
    balance: Balance,
    limiter: Limiter,
    params: DspParams,
}

impl Chain {
    pub fn new(
        sample_rate: u32,
        layout: ChannelLayout,
        params: &DspParams,
    ) -> Result<Self, DspError> {
        if !SAMPLE_RATES.contains(&sample_rate) {
            return Err(DspError::UnsupportedSampleRate(sample_rate));
        }
        let rate = sample_rate as f32;
        let params = params.sanitized();
        let mut c = Self {
            sample_rate,
            layout,
            mode: params.multichannel,
            fold: None,
            eq: Equalizer::new(rate),
            loudness: Loudness::new(rate),
            gain: GainRamp::new(rate),
            balance: Balance::new(rate),
            limiter: Limiter::new(rate, 2),
            params,
        };
        c.apply_layout();
        c.apply(&params, true);
        c.eq.snap_to_targets();
        Ok(c)
    }

    fn apply_layout(&mut self) {
        let wide = self.layout.channels() > 2;
        self.fold = (wide && self.mode == MultichannelMode::FoldToStereo)
            .then(|| StereoDownmix::new(&self.layout));
        self.limiter
            .configure(self.sample_rate as f32, usize::from(self.output_channels()));
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn input_layout(&self) -> ChannelLayout {
        self.layout
    }

    /// Channels produced by [`process`](Self::process).
    pub fn output_channels(&self) -> u16 {
        if self.fold.is_some() {
            2
        } else {
            self.layout.channels()
        }
    }

    fn stereo_stages(&self) -> bool {
        self.output_channels() <= 2
    }

    /// Change rate or layout. Allocates; not for the real-time thread.
    /// Filter state is cleared.
    pub fn configure(&mut self, sample_rate: u32, layout: ChannelLayout) -> Result<(), DspError> {
        if !SAMPLE_RATES.contains(&sample_rate) {
            return Err(DspError::UnsupportedSampleRate(sample_rate));
        }
        let rate = sample_rate as f32;
        self.sample_rate = sample_rate;
        self.layout = layout;
        self.eq.set_sample_rate(rate);
        self.loudness.set_sample_rate(rate);
        self.gain.set_sample_rate(rate);
        self.balance.set_sample_rate(rate);
        self.apply_layout();
        self.reset();
        Ok(())
    }

    pub fn params(&self) -> &DspParams {
        &self.params
    }

    /// Apply a whole parameter set; gains and EQ ramp to their targets.
    /// A change of `multichannel` mode takes effect at the next
    /// [`configure`](Self::configure).
    pub fn set_params(&mut self, params: &DspParams) {
        self.apply(&params.sanitized(), false);
    }

    fn apply(&mut self, p: &DspParams, snap: bool) {
        if self.eq.is_enabled() != p.eq_enabled {
            self.eq.set_enabled(p.eq_enabled);
        }
        if self.eq.get_all_gains() != p.eq_gains_db {
            self.eq.set_all_gains(&p.eq_gains_db);
        }
        self.loudness.configure(p.loudness, p.loudness_volume);
        let gain = 10_f32.powf(p.preamp_db / 20.0) * p.volume;
        let gain = if p.preamp_db.abs() < 1e-3 && p.volume == 1.0 {
            1.0
        } else {
            gain
        };
        if snap {
            self.gain.snap(gain);
            self.balance.snap(p.balance);
        } else {
            self.gain.set(gain);
            self.balance.set(p.balance);
        }
        let mode = self.params.multichannel;
        self.params = *p;
        self.params.multichannel = mode;
        if snap {
            self.mode = p.multichannel;
            self.params.multichannel = p.multichannel;
        }
    }

    /// Drop all filter history and the limiter's delay line (seek, new
    /// media). Ramps jump to their targets.
    pub fn reset(&mut self) {
        self.eq.reset_state();
        self.eq.snap_to_targets();
        self.loudness.reset();
        self.limiter.reset();
        self.gain.snap(self.gain.target());
        self.balance.snap(self.params.balance);
    }

    /// Frames of delay the chain adds (the limiter's lookahead).
    pub fn latency_frames(&self) -> usize {
        self.limiter.latency_frames()
    }

    /// Every stage idle: only the limiter's fixed delay remains.
    pub fn is_bypassed(&self) -> bool {
        self.fold.is_none()
            && self.eq.is_transparent()
            && self.loudness.is_transparent()
            && !(self.params.mono && self.layout.channels() == 2)
            && self.gain.is_identity()
            && self.balance.is_identity()
            && !self.limiter.engaged
    }

    pub fn active(&self) -> ActiveProcessing {
        let stereo = self.stereo_stages();
        ActiveProcessing {
            eq: stereo && !self.eq.is_transparent(),
            loudness_db: if stereo && self.loudness.is_enabled() {
                Loudness::comp_db_for_volume(self.params.loudness_volume)
            } else {
                0.0
            },
            mono: stereo && self.params.mono && self.output_channels() == 2,
            balance: if self.output_channels() == 2 {
                self.params.balance
            } else {
                0.0
            },
            gain_db: 20.0 * self.gain.target().max(1e-9).log10(),
            replaygain_db: 0.0,
            limiter_engaged: self.limiter.engaged,
            fold_down_from: self.fold.map(|_| self.layout.label()),
            bypassed: self.is_bypassed(),
        }
    }

    /// Process interleaved input-layout frames in place. Returns the
    /// number of valid output samples at the front of `buf` (fewer than
    /// `buf.len()` when the input is folded to stereo). `tap` sees the
    /// post-EQ signal — the player's visualisation reads it there.
    pub fn process_with_tap(&mut self, buf: &mut [f32], mut tap: impl FnMut(&[f32], u16)) -> usize {
        let in_ch = usize::from(self.layout.channels());
        let usable = buf.len() - buf.len() % in_ch;
        let n = match &self.fold {
            Some(fold) => fold.apply_in_place(&mut buf[..usable]),
            None => usable,
        };
        let out = &mut buf[..n];
        let ch = usize::from(self.output_channels());
        if ch <= 2 {
            match ch {
                1 => self.eq.process_mono_in_place(out),
                _ => self.eq.process_stereo_in_place(out),
            }
            tap(out, ch as u16);
            self.loudness.process(out, ch);
            if self.params.mono {
                mono_downmix(out, ch);
            }
        } else {
            tap(out, ch as u16);
        }
        self.gain.process(out, ch);
        self.balance.process(out, ch);
        self.limiter.process(out);
        n
    }

    pub fn process(&mut self, buf: &mut [f32]) -> usize {
        self.process_with_tap(buf, |_, _| {})
    }
}

/// Settings for a standalone [`Processor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessorConfig {
    pub sample_rate: u32,
    pub layout: ChannelLayout,
}

impl ProcessorConfig {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, DspError> {
        Ok(Self {
            sample_rate,
            layout: ChannelLayout::default_for(channels)
                .ok_or(DspError::UnsupportedChannels(channels))?,
        })
    }
}

/// The chain for a host that keeps control of playback. See the module
/// documentation for the threading contract.
#[derive(Debug, Clone)]
pub struct Processor {
    track: TrackGain,
    chain: Chain,
    params: DspParams,
}

impl Processor {
    pub fn new(config: ProcessorConfig, params: &DspParams) -> Result<Self, DspError> {
        let chain = Chain::new(config.sample_rate, config.layout, params)?;
        let params = *chain.params();
        let mut track = TrackGain::new(config.sample_rate as f32);
        track.set_media(MediaGain::default(), ListeningContext::Unknown, &params);
        Ok(Self {
            track,
            chain,
            params,
        })
    }

    /// Change rate or layout (format change). Allocates; call it off the
    /// real-time thread or between streams.
    pub fn configure(&mut self, config: ProcessorConfig) -> Result<(), DspError> {
        self.chain.configure(config.sample_rate, config.layout)?;
        self.track.set_sample_rate(config.sample_rate as f32);
        Ok(())
    }

    pub fn params(&self) -> &DspParams {
        &self.params
    }

    /// Apply a complete parameter set atomically. Real-time safe.
    pub fn set_params(&mut self, params: &DspParams) {
        self.params = params.sanitized();
        self.chain.set_params(&self.params);
        self.track.update(&self.params);
    }

    /// Normalisation figures of the media about to play. Call before its
    /// first block (with [`reset`](Self::reset) when the previous media's
    /// tail must not ring into it). Real-time safe.
    pub fn set_media_gain(&mut self, gain: MediaGain, context: ListeningContext) {
        self.track.set_media(gain, context, &self.params);
    }

    /// After a seek or a media change: clears filter history and the
    /// limiter delay line, so nothing from before rings into what
    /// follows.
    pub fn reset(&mut self) {
        self.chain.reset();
    }

    /// Process interleaved frames in place; returns the valid output
    /// sample count (see [`Chain::process_with_tap`]).
    pub fn process(&mut self, buf: &mut [f32]) -> usize {
        let in_ch = usize::from(self.chain.input_layout().channels());
        self.track.process(buf, in_ch);
        self.chain.process(buf)
    }

    /// End of stream: write the samples still held in the limiter's
    /// lookahead into `out` and return how many were written (at most
    /// `out.len()`, rounded down to whole frames). Call once; then
    /// [`reset`](Self::reset) before reuse.
    pub fn flush(&mut self, out: &mut [f32]) -> usize {
        let in_ch = usize::from(self.chain.input_layout().channels());
        let frames = self.latency_frames().min(out.len() / in_ch);
        let buf = &mut out[..frames * in_ch];
        buf.fill(0.0);
        self.chain.process(buf)
    }

    /// Delay the chain adds, in frames. Constant for a configuration.
    pub fn latency_frames(&self) -> usize {
        self.chain.latency_frames()
    }

    pub fn output_channels(&self) -> u16 {
        self.chain.output_channels()
    }

    pub fn is_bypassed(&self) -> bool {
        self.track.is_identity() && self.chain.is_bypassed()
    }

    pub fn active(&self) -> ActiveProcessing {
        let mut a = self.chain.active();
        a.replaygain_db = self.track.applied_db();
        a.bypassed = self.is_bypassed();
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frames: usize, ch: usize, amp: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let s = amp * (std::f32::consts::TAU * 1000.0 * n as f32 / 48000.0).sin();
                std::iter::repeat_n(s, ch)
            })
            .collect()
    }

    #[test]
    fn replaygain_mode_resolution() {
        assert_eq!(ReplayGainMode::Off.gain_db(Some(-6.0), Some(-3.0)), 0.0);
        assert_eq!(ReplayGainMode::Track.gain_db(Some(-6.0), Some(-3.0)), -6.0);
        assert_eq!(ReplayGainMode::Track.gain_db(None, Some(-3.0)), 0.0);
        assert_eq!(ReplayGainMode::Album.gain_db(Some(-6.0), Some(-3.0)), -3.0);
        assert_eq!(ReplayGainMode::Album.gain_db(Some(-6.0), None), 0.0);
        assert_eq!(ReplayGainMode::Auto.gain_db(Some(-6.0), Some(-3.0)), -3.0);
        assert_eq!(ReplayGainMode::Auto.gain_db(Some(-6.0), None), -6.0);
        let g = MediaGain {
            track_gain_db: Some(-6.0),
            album_gain_db: Some(-3.0),
            ..Default::default()
        };
        assert_eq!(
            ReplayGainMode::Auto.resolve(&g, ListeningContext::Mixed).0,
            -6.0
        );
        assert_eq!(
            ReplayGainMode::Auto.resolve(&g, ListeningContext::Album).0,
            -3.0
        );
    }

    #[test]
    fn default_processor_is_bypassed_and_only_delays() {
        let cfg = ProcessorConfig::new(48000, 2).unwrap();
        let mut p = Processor::new(cfg, &DspParams::default()).unwrap();
        assert!(p.is_bypassed());
        let input = sine(1024, 2, 0.5);
        let mut buf = input.clone();
        let n = p.process(&mut buf);
        assert_eq!(n, buf.len());
        let lat = p.latency_frames() * 2;
        assert_eq!(&buf[lat..], &input[..input.len() - lat]);
        // Flush returns exactly the held-back tail.
        let mut tail = vec![0.0; 4096];
        let t = p.flush(&mut tail);
        assert_eq!(t, lat);
        assert_eq!(&tail[..t], &input[input.len() - lat..]);
    }

    #[test]
    fn replaygain_and_peak_protection_are_applied() {
        let cfg = ProcessorConfig::new(48000, 1).unwrap();
        let params = DspParams {
            replaygain: ReplayGainMode::Track,
            ..Default::default()
        };
        let mut p = Processor::new(cfg, &params).unwrap();
        p.set_media_gain(
            MediaGain {
                track_gain_db: Some(-6.0206),
                ..Default::default()
            },
            ListeningContext::Unknown,
        );
        let mut buf = vec![0.8_f32; 2000];
        p.process(&mut buf);
        assert!((buf[1999] - 0.4).abs() < 1e-3, "{}", buf[1999]);
        assert!((p.active().replaygain_db + 6.02).abs() < 0.01);

        // +6 dB on a track peaking at 0.9: capped to 1/0.9.
        p.set_params(&DspParams {
            replaygain_prevent_clipping: true,
            ..params
        });
        p.set_media_gain(
            MediaGain {
                track_gain_db: Some(6.0),
                track_peak: Some(0.9),
                ..Default::default()
            },
            ListeningContext::Unknown,
        );
        let mut buf = vec![0.9_f32; 2000];
        p.process(&mut buf);
        assert!((buf[1999] - 1.0).abs() < 1e-3, "{}", buf[1999]);
    }

    #[test]
    fn parameter_changes_ramp_instead_of_stepping() {
        let cfg = ProcessorConfig::new(48000, 1).unwrap();
        let mut p = Processor::new(cfg, &DspParams::default()).unwrap();
        let mut warm = vec![0.5_f32; 1000];
        p.process(&mut warm);
        p.set_params(&DspParams {
            preamp_db: -12.0,
            ..Default::default()
        });
        let mut buf = vec![0.5_f32; 2000];
        p.process(&mut buf);
        let max_step = buf
            .windows(2)
            .fold(0.0_f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(max_step < 0.01, "step {max_step}");
        assert!((buf[1999] - 0.5 * 10_f32.powf(-0.6)).abs() < 1e-4);
    }

    #[test]
    fn surround_is_folded_and_reported() {
        let cfg = ProcessorConfig::new(48000, 6).unwrap();
        let mut p = Processor::new(
            cfg,
            &DspParams {
                eq_enabled: true,
                eq_gains_db: [3.0; 10],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.output_channels(), 2);
        let mut buf = sine(512, 6, 0.3);
        let n = p.process(&mut buf);
        assert_eq!(n, 512 * 2);
        let a = p.active();
        assert_eq!(a.fold_down_from.as_deref(), Some("5.1"));
        assert!(a.eq);
    }

    #[test]
    fn passthrough_keeps_channels_and_reports_stereo_stages_off() {
        let cfg = ProcessorConfig::new(48000, 6).unwrap();
        let mut p = Processor::new(
            cfg,
            &DspParams {
                eq_enabled: true,
                eq_gains_db: [3.0; 10],
                multichannel: MultichannelMode::Passthrough,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.output_channels(), 6);
        let mut buf = sine(256, 6, 0.3);
        assert_eq!(p.process(&mut buf), 256 * 6);
        assert!(!p.active().eq);
    }

    #[test]
    fn reset_clears_history() {
        let cfg = ProcessorConfig::new(48000, 2).unwrap();
        let params = DspParams {
            eq_enabled: true,
            eq_gains_db: [6.0; 10],
            ..Default::default()
        };
        let mut a = Processor::new(cfg, &params).unwrap();
        let mut b = Processor::new(cfg, &params).unwrap();
        let mut noise = sine(4096, 2, 0.9);
        a.process(&mut noise);
        a.reset();
        let mut x = sine(1024, 2, 0.2);
        let mut y = x.clone();
        a.process(&mut x);
        b.process(&mut y);
        assert_eq!(x, y);
    }

    #[test]
    fn rejects_bad_config_and_sanitizes_params() {
        assert!(ProcessorConfig::new(48000, 0).is_err());
        let cfg = ProcessorConfig {
            sample_rate: 1000,
            layout: ChannelLayout::STEREO,
        };
        assert!(Processor::new(cfg, &DspParams::default()).is_err());
        let p = DspParams {
            preamp_db: f32::NAN,
            volume: 9.0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(p.preamp_db, 0.0);
        assert_eq!(p.volume, MAX_VOLUME);
    }
}
