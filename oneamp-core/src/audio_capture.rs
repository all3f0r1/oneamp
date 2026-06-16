/// Audio capture ring buffer for visualization.
///
/// Decoders push interleaved PCM samples via [`update`]. Visualization
/// readers (`compute_spectrum`, `compute_waveform`, etc.) pull a fixed-size
/// chronological window via [`snapshot_into`].
///
/// The internal storage is a power-of-two circular array; a monotonic
/// `write_pos` counter doubles as "samples ever pushed" so the reader
/// can detect a not-yet-full buffer and zero-pad the leading gap instead
/// of returning whatever stale data sits in the unwritten slots.
///
/// Replaces the v1 "copy new packet, zero-fill the rest" design which
/// destroyed temporal continuity and made the spectrum + oscilloscope
/// flicker between packets.
pub struct AudioCaptureBuffer {
    samples: Box<[f32]>,
    /// `samples.len() - 1`. Capacity is always a power of two so we can
    /// mask instead of taking a modulo on the hot path.
    mask: usize,
    /// Monotonic write counter. `samples[write_pos & mask]` is the next
    /// slot to write into; `write_pos` itself is the total count of
    /// samples ever pushed (wraps at `usize::MAX`, which never happens
    /// in practice — would take 6 million years at 44.1 kHz on 64-bit).
    write_pos: usize,
    sample_rate: u32,
    channels: u16,
}

impl AudioCaptureBuffer {
    /// Create a ring buffer holding at least `capacity` samples. Internally
    /// rounded up to the next power of two so [`update`] and
    /// [`snapshot_into`] can mask instead of modulo.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.next_power_of_two().max(2);
        Self {
            samples: vec![0.0; cap].into_boxed_slice(),
            mask: cap - 1,
            write_pos: 0,
            sample_rate: 44100,
            channels: 2,
        }
    }

    /// Push interleaved samples into the ring. Old data is overwritten
    /// circularly. If `samples.len()` exceeds the ring capacity, only the
    /// trailing `capacity` samples are written — the prefix would just be
    /// overwritten in the same call.
    ///
    /// Sample rate / channels are stored for downstream consumers but
    /// don't affect the ring layout.
    pub fn update(&mut self, samples: &[f32], sample_rate: u32, channels: u16) {
        self.sample_rate = sample_rate;
        self.channels = channels;

        if samples.is_empty() {
            return;
        }

        let cap = self.samples.len();
        let (skip, take) = if samples.len() > cap {
            (samples.len() - cap, cap)
        } else {
            (0, samples.len())
        };
        let payload = &samples[skip..skip + take];

        let start = self.write_pos & self.mask;
        let end_first = (start + payload.len()).min(cap);
        let first_len = end_first - start;
        self.samples[start..end_first].copy_from_slice(&payload[..first_len]);
        if first_len < payload.len() {
            let remaining = payload.len() - first_len;
            self.samples[..remaining].copy_from_slice(&payload[first_len..]);
        }
        self.write_pos = self.write_pos.wrapping_add(payload.len());
    }

    /// Copy the most-recent `dst.len()` samples into `dst` in chronological
    /// order (oldest first, newest last). When fewer samples have ever been
    /// pushed than `dst.len()`, the leading slots are zero-filled so the
    /// caller always sees a contiguous window of the requested size.
    pub fn snapshot_into(&self, dst: &mut [f32]) {
        if dst.is_empty() {
            return;
        }
        let cap = self.samples.len();
        let want = dst.len();
        let available = self.write_pos.min(cap);
        let lead = want.saturating_sub(available);
        if lead > 0 {
            dst[..lead].fill(0.0);
        }
        let to_copy = want - lead;
        if to_copy == 0 {
            return;
        }
        // The newest sample sits at `write_pos - 1` (in monotonic coords),
        // so the start of the window is `write_pos - to_copy`. Map back to
        // ring coordinates via mask.
        let start_ring = self.write_pos.wrapping_sub(to_copy) & self.mask;
        let end_first = (start_ring + to_copy).min(cap);
        let first_len = end_first - start_ring;
        dst[lead..lead + first_len].copy_from_slice(&self.samples[start_ring..end_first]);
        if first_len < to_copy {
            let remaining = to_copy - first_len;
            dst[lead + first_len..].copy_from_slice(&self.samples[..remaining]);
        }
    }

    /// Sample rate stamped onto the latest [`update`] call. Defaults to
    /// 44100 until the first push.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel count stamped onto the latest [`update`] call. Defaults
    /// to 2 until the first push.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Ring capacity in samples (always a power of two, possibly larger
    /// than the value passed to [`new`]).
    pub fn capacity(&self) -> usize {
        self.samples.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rounds_capacity_up_to_power_of_two() {
        let buf = AudioCaptureBuffer::new(1000);
        assert_eq!(buf.capacity(), 1024);
        let buf = AudioCaptureBuffer::new(2048);
        assert_eq!(buf.capacity(), 2048);
        let buf = AudioCaptureBuffer::new(0);
        assert_eq!(buf.capacity(), 2);
    }

    #[test]
    fn snapshot_zero_fills_leading_gap_when_not_yet_full() {
        let mut buf = AudioCaptureBuffer::new(8);
        buf.update(&[1.0, 2.0, 3.0], 44100, 1);
        let mut out = [-1.0_f32; 8];
        buf.snapshot_into(&mut out);
        // Leading 5 slots are the not-yet-written gap → zeros.
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn snapshot_returns_last_n_chronologically_across_wrap() {
        // Cap = 4; write 6 samples so positions 0,1 get overwritten by 4,5.
        let mut buf = AudioCaptureBuffer::new(4);
        buf.update(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 44100, 1);
        let mut out = [0.0_f32; 4];
        buf.snapshot_into(&mut out);
        assert_eq!(out, [3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn snapshot_smaller_than_buffer_returns_newest_tail() {
        let mut buf = AudioCaptureBuffer::new(8);
        for i in 1..=8u32 {
            buf.update(&[i as f32], 44100, 1);
        }
        let mut out = [0.0_f32; 3];
        buf.snapshot_into(&mut out);
        assert_eq!(out, [6.0, 7.0, 8.0]);
    }

    #[test]
    fn update_larger_than_capacity_keeps_trailing_window() {
        let mut buf = AudioCaptureBuffer::new(4);
        let huge: Vec<f32> = (0..100).map(|i| i as f32).collect();
        buf.update(&huge, 44100, 2);
        let mut out = [0.0_f32; 4];
        buf.snapshot_into(&mut out);
        // Last 4 samples of 0..100 are 96..99.
        assert_eq!(out, [96.0, 97.0, 98.0, 99.0]);
    }

    #[test]
    fn multiple_updates_preserve_chronological_order() {
        let mut buf = AudioCaptureBuffer::new(8);
        buf.update(&[1.0, 2.0], 44100, 2);
        buf.update(&[3.0, 4.0, 5.0], 48000, 2);
        buf.update(&[6.0], 48000, 2);
        let mut out = [0.0_f32; 6];
        buf.snapshot_into(&mut out);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        // Latest update's metadata wins.
        assert_eq!(buf.sample_rate(), 48000);
        assert_eq!(buf.channels(), 2);
    }

    #[test]
    fn empty_update_is_a_noop() {
        let mut buf = AudioCaptureBuffer::new(4);
        buf.update(&[1.0, 2.0], 44100, 1);
        buf.update(&[], 44100, 1);
        let mut out = [0.0_f32; 4];
        buf.snapshot_into(&mut out);
        assert_eq!(out, [0.0, 0.0, 1.0, 2.0]);
    }
}
