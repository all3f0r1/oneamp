//! Channel layouts and the downmix the chain uses when a source has more
//! channels than the stereo stages (EQ, loudness, balance, mono) accept.
//!
//! Positions follow the WAVE/symphonia canonical order: an interleaved
//! frame lists its channels in ascending [`Speaker`] order.

/// A speaker position. The discriminant is the bit index of the matching
/// symphonia / WAVEFORMATEXTENSIBLE channel mask, so a mask maps onto a
/// layout without a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum Speaker {
    FrontLeft = 0,
    FrontRight = 1,
    FrontCenter = 2,
    Lfe = 3,
    RearLeft = 4,
    RearRight = 5,
    FrontLeftCenter = 6,
    FrontRightCenter = 7,
    RearCenter = 8,
    SideLeft = 9,
    SideRight = 10,
    /// Any position the downmix has no rule for (height channels, …).
    /// It is folded into both sides at -6 dB rather than dropped.
    Other = 31,
}

impl Speaker {
    fn from_bit(bit: u32) -> Self {
        use Speaker::*;
        match bit {
            0 => FrontLeft,
            1 => FrontRight,
            2 => FrontCenter,
            3 => Lfe,
            4 => RearLeft,
            5 => RearRight,
            6 => FrontLeftCenter,
            7 => FrontRightCenter,
            8 => RearCenter,
            9 => SideLeft,
            10 => SideRight,
            _ => Other,
        }
    }

    /// (left, right) weight of this speaker in a stereo fold-down,
    /// before normalisation. ITU-R BS.775: centre and surrounds at
    /// -3 dB; LFE is left out, as the recommendation does.
    fn stereo_weights(self) -> (f32, f32) {
        const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
        use Speaker::*;
        match self {
            FrontLeft => (1.0, 0.0),
            FrontRight => (0.0, 1.0),
            FrontCenter => (H, H),
            Lfe => (0.0, 0.0),
            RearLeft | SideLeft | FrontLeftCenter => (H, 0.0),
            RearRight | SideRight | FrontRightCenter => (0.0, H),
            RearCenter => (0.5, 0.5),
            Other => (0.5, 0.5),
        }
    }
}

/// Most channels a layout (and the processor) handles: 7.1 plus
/// headroom. Fixed so a layout is `Copy` and never allocates.
pub const MAX_CHANNELS: usize = 16;

/// Interleaved channel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelLayout {
    count: u8,
    speakers: [Speaker; MAX_CHANNELS],
}

impl ChannelLayout {
    pub const MONO: Self = Self::from_speakers_const(&[Speaker::FrontCenter]);
    pub const STEREO: Self = Self::from_speakers_const(&[Speaker::FrontLeft, Speaker::FrontRight]);

    const fn from_speakers_const(list: &[Speaker]) -> Self {
        let mut speakers = [Speaker::Other; MAX_CHANNELS];
        let mut i = 0;
        while i < list.len() {
            speakers[i] = list[i];
            i += 1;
        }
        Self {
            count: list.len() as u8,
            speakers,
        }
    }

    /// Layout from an explicit speaker list. `None` past
    /// [`MAX_CHANNELS`] or for an empty list.
    pub fn from_speakers(list: &[Speaker]) -> Option<Self> {
        (!list.is_empty() && list.len() <= MAX_CHANNELS).then(|| Self::from_speakers_const(list))
    }

    /// Layout from a WAVE-style channel mask (bit n = [`Speaker`] n).
    pub fn from_mask(mask: u64) -> Option<Self> {
        let mut list = [Speaker::Other; MAX_CHANNELS];
        let mut n = 0;
        for bit in 0..64 {
            if mask & (1 << bit) != 0 {
                if n == MAX_CHANNELS {
                    return None;
                }
                list[n] = Speaker::from_bit(bit);
                n += 1;
            }
        }
        Self::from_speakers(&list[..n])
    }

    /// The conventional layout for a bare channel count: mono, stereo,
    /// then the common WAVE defaults (quad, 5.0, 5.1, 6.1, 7.1). Other
    /// counts get [`Speaker::Other`] beyond the front pair.
    pub fn default_for(channels: u16) -> Option<Self> {
        use Speaker::*;
        let list: &[Speaker] = match channels {
            1 => &[FrontCenter],
            2 => &[FrontLeft, FrontRight],
            3 => &[FrontLeft, FrontRight, FrontCenter],
            4 => &[FrontLeft, FrontRight, RearLeft, RearRight],
            5 => &[FrontLeft, FrontRight, FrontCenter, RearLeft, RearRight],
            6 => &[FrontLeft, FrontRight, FrontCenter, Lfe, RearLeft, RearRight],
            7 => &[
                FrontLeft,
                FrontRight,
                FrontCenter,
                Lfe,
                RearCenter,
                SideLeft,
                SideRight,
            ],
            8 => &[
                FrontLeft,
                FrontRight,
                FrontCenter,
                Lfe,
                RearLeft,
                RearRight,
                SideLeft,
                SideRight,
            ],
            n if (n as usize) <= MAX_CHANNELS => {
                let mut list = [Other; MAX_CHANNELS];
                list[0] = FrontLeft;
                list[1] = FrontRight;
                return Self::from_speakers(&list[..n as usize]);
            }
            _ => return None,
        };
        Self::from_speakers(list)
    }

    pub fn channels(&self) -> u16 {
        u16::from(self.count)
    }

    pub fn speakers(&self) -> &[Speaker] {
        &self.speakers[..usize::from(self.count)]
    }

    /// Short human label for diagnostics ("stereo", "5.1", "8ch").
    pub fn label(&self) -> String {
        match (self.count, self.speakers().contains(&Speaker::Lfe)) {
            (1, _) => "mono".into(),
            (2, _) => "stereo".into(),
            (6, true) => "5.1".into(),
            (8, true) => "7.1".into(),
            (n, _) => format!("{n}ch"),
        }
    }
}

/// Precomputed stereo fold-down matrix for one layout. Rows are
/// normalised together so a full-scale signal on every input channel
/// cannot exceed full scale on either output — the "level management"
/// half of a downmix; without it a 5.1 fold-down clips at +7.7 dB.
#[derive(Debug, Clone, Copy)]
pub struct StereoDownmix {
    channels: usize,
    left: [f32; MAX_CHANNELS],
    right: [f32; MAX_CHANNELS],
}

impl StereoDownmix {
    pub fn new(layout: &ChannelLayout) -> Self {
        let mut left = [0.0; MAX_CHANNELS];
        let mut right = [0.0; MAX_CHANNELS];
        for (i, s) in layout.speakers().iter().enumerate() {
            (left[i], right[i]) = s.stereo_weights();
        }
        let n = layout.speakers().len();
        let peak = left[..n]
            .iter()
            .sum::<f32>()
            .max(right[..n].iter().sum::<f32>());
        if peak > 1.0 {
            for w in left.iter_mut().chain(right.iter_mut()) {
                *w /= peak;
            }
        }
        Self {
            channels: n,
            left,
            right,
        }
    }

    pub fn input_channels(&self) -> usize {
        self.channels
    }

    /// Fold `buf` (interleaved, `input_channels()` wide) to stereo in
    /// place. Returns the number of valid output samples at the front of
    /// `buf` (2 per input frame). Never allocates: each output frame is
    /// written at or before the input frame it was read from.
    pub fn apply_in_place(&self, buf: &mut [f32]) -> usize {
        let ch = self.channels;
        let frames = buf.len() / ch;
        for f in 0..frames {
            let frame = &buf[f * ch..f * ch + ch];
            let (mut l, mut r) = (0.0, 0.0);
            for (i, &s) in frame.iter().enumerate() {
                l += s * self.left[i];
                r += s * self.right[i];
            }
            buf[2 * f] = l;
            buf[2 * f + 1] = r;
        }
        frames * 2
    }
}

/// Map interleaved frames of `src_ch` channels onto `dst_ch`, for an
/// output whose channel count differs from what the chain produced:
/// mono is copied to every output, stereo goes to the first pair, and
/// any further output channels stay silent. Never drops input: callers
/// fold anything wider than the output with [`StereoDownmix`] first.
pub fn map_channels(src: &[f32], src_ch: usize, dst_ch: usize, out: &mut Vec<f32>) {
    out.clear();
    for frame in src.chunks_exact(src_ch) {
        for c in 0..dst_ch {
            out.push(match (src_ch, frame.get(c)) {
                (1, _) => frame[0],
                (_, Some(&s)) => s,
                (_, None) => 0.0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_one_folds_down_without_clipping_or_losing_channels() {
        let layout = ChannelLayout::default_for(6).unwrap();
        assert_eq!(layout.label(), "5.1");
        let dm = StereoDownmix::new(&layout);
        // Full scale on every channel: the fold must stay within ±1.
        let mut buf = vec![1.0_f32; 6 * 4];
        let n = dm.apply_in_place(&mut buf);
        assert_eq!(n, 8);
        for &s in &buf[..n] {
            assert!(s <= 1.0 + 1e-6 && s > 0.9, "{s}");
        }
        // Centre only: equal on both sides, and present (not dropped).
        let mut buf = vec![0.0, 0.0, 0.5, 0.0, 0.0, 0.0];
        dm.apply_in_place(&mut buf);
        assert!(buf[0] > 0.1 && (buf[0] - buf[1]).abs() < 1e-6);
        // Rear left only lands on the left.
        let mut buf = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.0];
        dm.apply_in_place(&mut buf);
        assert!(buf[0] > 0.1 && buf[1] == 0.0);
        // LFE is left out, per BS.775.
        let mut buf = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        dm.apply_in_place(&mut buf);
        assert_eq!(&buf[..2], &[0.0, 0.0]);
    }

    #[test]
    fn stereo_downmix_of_stereo_is_identity() {
        let dm = StereoDownmix::new(&ChannelLayout::STEREO);
        let mut buf = vec![0.25, -0.5, 0.75, 0.1];
        let orig = buf.clone();
        assert_eq!(dm.apply_in_place(&mut buf), 4);
        assert_eq!(buf, orig);
    }

    #[test]
    fn mask_maps_to_canonical_order() {
        // FL | FR | FC | LFE | SL | SR (bits 0,1,2,3,9,10).
        let l = ChannelLayout::from_mask(0b110_0000_1111).unwrap();
        assert_eq!(
            l.speakers(),
            &[
                Speaker::FrontLeft,
                Speaker::FrontRight,
                Speaker::FrontCenter,
                Speaker::Lfe,
                Speaker::SideLeft,
                Speaker::SideRight
            ]
        );
    }

    #[test]
    fn map_channels_upmixes_mono_and_pads_extra() {
        let mut out = Vec::new();
        map_channels(&[0.1, 0.2], 1, 2, &mut out);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2]);
        map_channels(&[0.1, 0.2], 2, 4, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.0, 0.0]);
    }
}
