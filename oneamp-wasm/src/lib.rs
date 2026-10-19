//! OneAmp's processing chain behind a plain C ABI, for WebAssembly hosts
//! (an `AudioWorkletProcessor`) and anything else that can call C.
//!
//! No `wasm-bindgen`, no imports: the module instantiates synchronously
//! inside an `AudioWorkletGlobalScope` from a `WebAssembly.Module` the
//! main thread compiled. Every buffer is allocated by `oneamp_new`; the
//! per-block calls (`oneamp_process`, the setters) never allocate.
//!
//! Protocol per block: write interleaved input frames to
//! `oneamp_buffer(h)`, call `oneamp_process(h, frames)`, read back the
//! returned number of samples (in `oneamp_output_channels(h)` channels)
//! from the same buffer.

use oneamp_core::dsp::channels::ChannelLayout;
use oneamp_core::dsp::{
    DspParams, ListeningContext, MediaGain, Processor, ProcessorConfig, ReplayGainMode,
};

pub struct Handle {
    processor: Processor,
    params: DspParams,
    buffer: Vec<f32>,
    channels: usize,
}

impl Handle {
    fn apply(&mut self, edit: impl FnOnce(&mut DspParams)) {
        edit(&mut self.params);
        self.processor.set_params(&self.params);
    }
}

/// Create a processor for `rate` Hz, `channels` interleaved channels and
/// blocks of at most `max_frames` frames. Null when the format is
/// unsupported.
#[unsafe(no_mangle)]
pub extern "C" fn oneamp_new(rate: u32, channels: u32, max_frames: u32) -> *mut Handle {
    let Ok(ch) = u16::try_from(channels) else {
        return std::ptr::null_mut();
    };
    let Some(layout) = ChannelLayout::default_for(ch) else {
        return std::ptr::null_mut();
    };
    let config = ProcessorConfig {
        sample_rate: rate,
        layout,
    };
    let params = DspParams::default();
    match Processor::new(config, &params) {
        Ok(processor) => {
            // Room for a block or the limiter tail, whichever is longer.
            let frames = (max_frames as usize).max(processor.latency_frames()).max(1);
            Box::into_raw(Box::new(Handle {
                buffer: vec![0.0; frames * usize::from(ch)],
                channels: usize::from(ch),
                processor,
                params,
            }))
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `h` must come from [`oneamp_new`] and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_free(h: *mut Handle) {
    if !h.is_null() {
        // SAFETY: created by `Box::into_raw` in `oneamp_new`.
        drop(unsafe { Box::from_raw(h) });
    }
}

/// Run `f` on the handle, or return `default` for a null pointer.
///
/// # Safety
/// `h` must be null or a live pointer from [`oneamp_new`], not used
/// concurrently (one audio thread owns it).
unsafe fn with<R>(h: *mut Handle, default: R, f: impl FnOnce(&mut Handle) -> R) -> R {
    // SAFETY: per the caller contract above.
    match unsafe { h.as_mut() } {
        Some(h) => f(h),
        None => default,
    }
}

/// The interleaved block buffer (input and output).
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_buffer(h: *mut Handle) -> *mut f32 {
    unsafe { with(h, std::ptr::null_mut(), |h| h.buffer.as_mut_ptr()) }
}

/// Capacity of the buffer, in frames of input channels.
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_buffer_frames(h: *mut Handle) -> u32 {
    unsafe { with(h, 0, |h| (h.buffer.len() / h.channels) as u32) }
}

/// Process `frames` frames in place; returns the output sample count.
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_process(h: *mut Handle, frames: u32) -> u32 {
    unsafe {
        with(h, 0, |h| {
            let n = (frames as usize * h.channels).min(h.buffer.len());
            h.processor.process(&mut h.buffer[..n]) as u32
        })
    }
}

/// End of stream: write the held-back tail to the buffer; returns its
/// sample count.
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_flush(h: *mut Handle) -> u32 {
    unsafe { with(h, 0, |h| h.processor.flush(&mut h.buffer) as u32) }
}

/// Clear filter history and the lookahead (seek, new media).
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_reset(h: *mut Handle) {
    unsafe { with(h, (), |h| h.processor.reset()) }
}

/// Delay the chain adds, in frames.
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_latency_frames(h: *mut Handle) -> u32 {
    unsafe { with(h, 0, |h| h.processor.latency_frames() as u32) }
}

/// Channels of the output (2 when a surround input is folded).
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_output_channels(h: *mut Handle) -> u32 {
    unsafe { with(h, 0, |h| u32::from(h.processor.output_channels())) }
}

/// 1 when every stage is idle (output = input, delayed).
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_is_bypassed(h: *mut Handle) -> u32 {
    unsafe { with(h, 0, |h| u32::from(h.processor.is_bypassed())) }
}

/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_eq_enabled(h: *mut Handle, on: u32) {
    unsafe { with(h, (), |h| h.apply(|p| p.eq_enabled = on != 0)) }
}

/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_eq_band(h: *mut Handle, band: u32, db: f32) {
    unsafe {
        with(h, (), |h| {
            if let Some(g) = h.params.eq_gains_db.get_mut(band as usize) {
                *g = db;
                let p = h.params;
                h.apply(|q| *q = p);
            }
        })
    }
}

/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_preamp(h: *mut Handle, db: f32) {
    unsafe { with(h, (), |h| h.apply(|p| p.preamp_db = db)) }
}

/// Linear volume inside the chain, ahead of the limiter (0..=1.5).
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_volume(h: *mut Handle, volume: f32) {
    unsafe { with(h, (), |h| h.apply(|p| p.volume = volume)) }
}

/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_balance(h: *mut Handle, balance: f32) {
    unsafe { with(h, (), |h| h.apply(|p| p.balance = balance)) }
}

/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_mono(h: *mut Handle, on: u32) {
    unsafe { with(h, (), |h| h.apply(|p| p.mono = on != 0)) }
}

/// Loudness compensation following `listening_volume` (0..=1).
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_loudness(h: *mut Handle, on: u32, listening_volume: f32) {
    unsafe {
        with(h, (), |h| {
            h.apply(|p| {
                p.loudness = on != 0;
                p.loudness_volume = listening_volume;
            })
        })
    }
}

/// `mode`: 0 off, 1 track, 2 album, 3 auto.
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_replaygain(h: *mut Handle, mode: u32, prevent_clipping: u32) {
    let mode = match mode {
        1 => ReplayGainMode::Track,
        2 => ReplayGainMode::Album,
        3 => ReplayGainMode::Auto,
        _ => ReplayGainMode::Off,
    };
    unsafe {
        with(h, (), |h| {
            h.apply(|p| {
                p.replaygain = mode;
                p.replaygain_prevent_clipping = prevent_clipping != 0;
            })
        })
    }
}

/// Gains of the media about to play (dB; peaks linear). NaN = unknown.
/// `context`: 0 unknown, 1 album in order, 2 mixed/shuffle.
///
/// # Safety
/// See [`with`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_media_gain(
    h: *mut Handle,
    track_db: f32,
    track_peak: f32,
    album_db: f32,
    album_peak: f32,
    context: u32,
) {
    let opt = |v: f32| v.is_finite().then_some(v);
    let gain = MediaGain {
        track_gain_db: opt(track_db),
        track_peak: opt(track_peak),
        album_gain_db: opt(album_db),
        album_peak: opt(album_peak),
    };
    let ctx = match context {
        1 => ListeningContext::Album,
        2 => ListeningContext::Mixed,
        _ => ListeningContext::Unknown,
    };
    unsafe { with(h, (), |h| h.processor.set_media_gain(gain, ctx)) }
}

/// Apply a whole `DspParams` JSON document (missing fields default).
/// Allocates: call it outside the audio callback, or accept the cost of
/// a rare parameter change. Returns 0 on success, -1 on invalid JSON.
///
/// # Safety
/// See [`with`]; `json` must point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_set_params_json(h: *mut Handle, json: *const u8, len: u32) -> i32 {
    if json.is_null() {
        return -1;
    }
    // SAFETY: caller guarantees `len` readable bytes at `json`.
    let bytes = unsafe { std::slice::from_raw_parts(json, len as usize) };
    let Ok(params) = serde_json::from_slice::<DspParams>(bytes) else {
        return -1;
    };
    unsafe {
        with(h, -1, |h| {
            h.apply(|p| *p = params);
            0
        })
    }
}

/// Scratch allocation for passing strings in (JSON).
#[unsafe(no_mangle)]
pub extern "C" fn oneamp_alloc(len: u32) -> *mut u8 {
    let mut v = vec![0u8; len as usize].into_boxed_slice();
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` must come from one [`oneamp_alloc`] call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oneamp_dealloc(ptr: *mut u8, len: u32) {
    if !ptr.is_null() {
        // SAFETY: reconstructs the boxed slice `oneamp_alloc` leaked.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len as usize)) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_abi_round_trip() {
        unsafe {
            assert!(oneamp_new(1000, 2, 128).is_null(), "rate out of range");
            let h = oneamp_new(48000, 2, 128);
            assert!(!h.is_null());
            assert_eq!(oneamp_buffer_frames(h), 128);
            assert_eq!(oneamp_is_bypassed(h), 1);
            let lat = oneamp_latency_frames(h) as usize;
            let buf = std::slice::from_raw_parts_mut(oneamp_buffer(h), 256);
            for (i, s) in buf.iter_mut().enumerate() {
                *s = (i as f32 / 256.0) - 0.5;
            }
            let input = buf.to_vec();
            assert_eq!(oneamp_process(h, 128), 256);
            let buf = std::slice::from_raw_parts(oneamp_buffer(h), 256);
            assert_eq!(
                &buf[lat * 2..],
                &input[..256 - lat * 2],
                "bypass = pure delay"
            );

            oneamp_set_preamp(h, -6.0206);
            assert_eq!(oneamp_is_bypassed(h), 0);
            let json = br#"{"mono":true,"preamp_db":-3.0}"#;
            let p = oneamp_alloc(json.len() as u32);
            std::ptr::copy_nonoverlapping(json.as_ptr(), p, json.len());
            assert_eq!(oneamp_set_params_json(h, p, json.len() as u32), 0);
            assert_eq!(oneamp_set_params_json(h, p, 3), -1);
            oneamp_dealloc(p, json.len() as u32);
            oneamp_reset(h);
            assert_eq!(oneamp_flush(h) as usize, lat * 2);
            oneamp_free(h);

            let six = oneamp_new(48000, 6, 64);
            assert_eq!(oneamp_output_channels(six), 2);
            assert_eq!(oneamp_process(six, 64), 128);
            oneamp_free(six);
        }
    }
}
