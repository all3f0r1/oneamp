//! Kotlin/JNI bridge: `org.oneamp.OneAmpProcessor` (PCM chain over the
//! host's blocks) and `org.oneamp.OneAmpPlayer` (the full engine, whose
//! output the host pulls into `AudioTrack`/AAudio).
//!
//! Lifetimes: each Kotlin object owns one native handle (a `jlong`),
//! created by `nativeCreate` and released by `nativeDestroy` (Kotlin's
//! `close()`). Kotlin guarantees no call after `close()` and no `close()`
//! during a call (see the Kotlin wrappers' locks). Buffers are Java
//! arrays or direct `ByteBuffer`s owned by Kotlin; native code only
//! borrows them for the duration of a call.
//!
//! Errors: invalid input raises `IllegalArgumentException`, a bad
//! handle `IllegalStateException`; playback outcomes are events.

pub mod fd;
mod jni_access;
pub mod wire;

use jni::JNIEnv;
use jni::objects::{JByteBuffer, JClass, JFloatArray, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring};
use oneamp_core::dsp::channels::ChannelLayout;
use oneamp_core::dsp::{DspParams, ListeningContext, MediaGain, Processor, ProcessorConfig};
use oneamp_core::output::OutputFault;
use oneamp_core::output::pull::{PullConfig, PullHandle, pull_output};
use oneamp_core::{AudioEngine, EngineConfig, OutputPolicy};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn throw(env: &mut JNIEnv, class: &str, msg: &str) {
    let _ = env.throw_new(class, msg);
}

fn get_string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    env.get_string(s).ok().map(Into::into)
}

fn new_string(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

// ---------------------------------------------------------------------
// Processor

struct ProcessorHandle {
    processor: Processor,
    scratch: Vec<f32>,
    channels: usize,
}

/// # Safety
/// `h` is 0 or a handle from `nativeCreate` that Kotlin has not closed.
unsafe fn processor<'a>(h: jlong) -> Option<&'a mut ProcessorHandle> {
    // SAFETY: per the contract above.
    unsafe { (h as *mut ProcessorHandle).as_mut() }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeCreate(
    mut env: JNIEnv,
    _: JClass,
    rate: jint,
    channels: jint,
    max_frames: jint,
) -> jlong {
    let layout = u16::try_from(channels)
        .ok()
        .and_then(ChannelLayout::default_for);
    let (Some(layout), Ok(rate)) = (layout, u32::try_from(rate)) else {
        throw(
            &mut env,
            "java/lang/IllegalArgumentException",
            "unsupported channel count or rate",
        );
        return 0;
    };
    match Processor::new(
        ProcessorConfig {
            sample_rate: rate,
            layout,
        },
        &DspParams::default(),
    ) {
        Ok(processor) => {
            let ch = usize::from(layout.channels());
            let frames = (max_frames.max(1) as usize).max(processor.latency_frames());
            Box::into_raw(Box::new(ProcessorHandle {
                processor,
                scratch: vec![0.0; frames * ch],
                channels: ch,
            })) as jlong
        }
        Err(e) => {
            throw(
                &mut env,
                "java/lang/IllegalArgumentException",
                &e.to_string(),
            );
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeDestroy(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    if h != 0 {
        // SAFETY: Kotlin calls this once, after every other call returned.
        drop(unsafe { Box::from_raw(h as *mut ProcessorHandle) });
    }
}

/// Process `frames` interleaved frames of `buffer` in place; returns
/// the output sample count. No allocation unless `frames` exceeds the
/// size given at creation.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeProcess(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    buffer: JFloatArray,
    frames: jint,
) -> jint {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { processor(h) }) else {
        throw(
            &mut env,
            "java/lang/IllegalStateException",
            "processor closed",
        );
        return 0;
    };
    let n = frames.max(0) as usize * p.channels;
    if p.scratch.len() < n {
        p.scratch.resize(n, 0.0);
    }
    if env
        .get_float_array_region(&buffer, 0, &mut p.scratch[..n])
        .is_err()
    {
        return 0;
    }
    let out = p.processor.process(&mut p.scratch[..n]);
    let _ = env.set_float_array_region(&buffer, 0, &p.scratch[..out]);
    out as jint
}

/// Same over a direct `ByteBuffer` of native-order `float`s (Media3's
/// `AudioProcessor` buffers): zero copy.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeProcessDirect(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    buffer: JByteBuffer,
    frames: jint,
) -> jint {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { processor(h) }) else {
        throw(
            &mut env,
            "java/lang/IllegalStateException",
            "processor closed",
        );
        return 0;
    };
    let (Ok(addr), Ok(cap)) = (
        env.get_direct_buffer_address(&buffer),
        env.get_direct_buffer_capacity(&buffer),
    ) else {
        throw(
            &mut env,
            "java/lang/IllegalArgumentException",
            "not a direct buffer",
        );
        return 0;
    };
    let n = (frames.max(0) as usize * p.channels).min(cap / 4);
    if addr.align_offset(std::mem::align_of::<f32>()) != 0 {
        throw(
            &mut env,
            "java/lang/IllegalArgumentException",
            "buffer not float-aligned",
        );
        return 0;
    }
    // SAFETY: a live direct buffer of at least `n` aligned floats, owned
    // by Kotlin for the duration of this call.
    let data = unsafe { std::slice::from_raw_parts_mut(addr as *mut f32, n) };
    p.processor.process(data) as jint
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeSetParams(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    json: JString,
) {
    let Some(json) = get_string(&mut env, &json) else {
        return;
    };
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { processor(h) }) else {
        throw(
            &mut env,
            "java/lang/IllegalStateException",
            "processor closed",
        );
        return;
    };
    match serde_json::from_str::<DspParams>(&json) {
        Ok(params) => p.processor.set_params(&params),
        Err(e) => throw(
            &mut env,
            "java/lang/IllegalArgumentException",
            &e.to_string(),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeSetMediaGain(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    track_db: jfloat,
    track_peak: jfloat,
    album_db: jfloat,
    album_peak: jfloat,
    context: jint,
) {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { processor(h) }) else {
        throw(
            &mut env,
            "java/lang/IllegalStateException",
            "processor closed",
        );
        return;
    };
    let opt = |v: f32| v.is_finite().then_some(v);
    p.processor.set_media_gain(
        MediaGain {
            track_gain_db: opt(track_db),
            track_peak: opt(track_peak),
            album_gain_db: opt(album_db),
            album_peak: opt(album_peak),
        },
        match context {
            1 => ListeningContext::Album,
            2 => ListeningContext::Mixed,
            _ => ListeningContext::Unknown,
        },
    );
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeReset(_: JNIEnv, _: JClass, h: jlong) {
    // SAFETY: handle contract (module docs).
    if let Some(p) = unsafe { processor(h) } {
        p.processor.reset();
    }
}

/// End of stream: the held-back tail into `buffer`; returns its samples.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeFlush(
    env: JNIEnv,
    _: JClass,
    h: jlong,
    buffer: JFloatArray,
) -> jint {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { processor(h) }) else {
        return 0;
    };
    let n = p.processor.flush(&mut p.scratch);
    let _ = env.set_float_array_region(&buffer, 0, &p.scratch[..n]);
    n as jint
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeLatencyFrames(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    // SAFETY: handle contract (module docs).
    unsafe { processor(h) }.map_or(0, |p| p.processor.latency_frames() as jint)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeOutputChannels(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) -> jint {
    // SAFETY: handle contract (module docs).
    unsafe { processor(h) }.map_or(0, |p| jint::from(p.processor.output_channels()))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpProcessor_nativeIsBypassed(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) -> jboolean {
    // SAFETY: handle contract (module docs).
    unsafe { processor(h) }.map_or(0, |p| u8::from(p.processor.is_bypassed()))
}

// ---------------------------------------------------------------------
// Player

struct PlayerHandle {
    engine: AudioEngine,
    output: PullHandle,
    channels: usize,
    /// Render scratch; `try_lock` on the audio thread, never waits.
    scratch: Mutex<Vec<f32>>,
    vm: Arc<jni::JavaVM>,
    access: Option<jni::objects::GlobalRef>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PlayerConfig {
    sample_rate: u32,
    channels: u16,
    preload_lead_ms: u64,
    event_capacity: usize,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            preload_lead_ms: 10_000,
            event_capacity: 256,
        }
    }
}

/// # Safety
/// `h` is 0 or a live handle from `nativeCreate`.
unsafe fn player<'a>(h: jlong) -> Option<&'a PlayerHandle> {
    // SAFETY: per the contract above.
    unsafe { (h as *const PlayerHandle).as_ref() }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeCreate(
    mut env: JNIEnv,
    _: JClass,
    config: JString,
    access_callback: JObject,
) -> jlong {
    let json = get_string(&mut env, &config).unwrap_or_default();
    let cfg: PlayerConfig = match serde_json::from_str(if json.is_empty() { "{}" } else { &json }) {
        Ok(c) => c,
        Err(e) => {
            throw(
                &mut env,
                "java/lang/IllegalArgumentException",
                &e.to_string(),
            );
            return 0;
        }
    };
    let vm = match env.get_java_vm() {
        Ok(vm) => Arc::new(vm),
        Err(e) => {
            throw(&mut env, "java/lang/IllegalStateException", &e.to_string());
            return 0;
        }
    };
    let access = if access_callback.is_null() {
        None
    } else {
        env.new_global_ref(&access_callback).ok()
    };
    // The host's audio track runs at one format: everything is
    // converted to it, so transitions never need a new track.
    let (backend, output) = pull_output(PullConfig {
        device_format: Some((cfg.sample_rate, cfg.channels)),
    });
    let engine = AudioEngine::with_backend(
        Box::new(backend),
        EngineConfig {
            output_policy: OutputPolicy::Fixed {
                sample_rate: cfg.sample_rate,
                channels: cfg.channels,
            },
            preload_lead: Duration::from_millis(cfg.preload_lead_ms),
            event_capacity: cfg.event_capacity,
            ..Default::default()
        },
    );
    match engine {
        Ok(engine) => Box::into_raw(Box::new(PlayerHandle {
            engine,
            output,
            channels: usize::from(cfg.channels.max(1)),
            scratch: Mutex::new(vec![0.0; 8192 * usize::from(cfg.channels.max(1))]),
            vm,
            access,
        })) as jlong,
        Err(e) => {
            throw(&mut env, "java/lang/IllegalStateException", &e.to_string());
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeDestroy(_: JNIEnv, _: JClass, h: jlong) {
    if h != 0 {
        // SAFETY: Kotlin calls this once, after every other call returned.
        // Dropping the engine stops its thread and every worker.
        drop(unsafe { Box::from_raw(h as *mut PlayerHandle) });
    }
}

/// Queue a JSON command; returns its id (> 0). Invalid commands throw
/// `IllegalArgumentException`; a full queue returns -1.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeSend(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    command: JString,
) -> jlong {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { player(h) }) else {
        throw(&mut env, "java/lang/IllegalStateException", "player closed");
        return 0;
    };
    let Some(json) = get_string(&mut env, &command) else {
        return 0;
    };
    let factory = |media_id: &str| -> Option<Box<dyn oneamp_core::AccessProvider>> {
        let callback = p.access.clone()?;
        Some(Box::new(jni_access::JniAccess {
            vm: p.vm.clone(),
            callback,
            media_id: media_id.to_string(),
        }))
    };
    match wire::parse_command(&json, &factory) {
        Ok(cmd) => match p.engine.send(cmd) {
            Ok(id) => id.0 as jlong,
            Err(_) => -1,
        },
        Err(e) => {
            throw(&mut env, "java/lang/IllegalArgumentException", &e.message);
            0
        }
    }
}

/// Next event as JSON, waiting up to `timeout_ms`; null when none.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativePollEvent(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
    timeout_ms: jlong,
) -> jstring {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { player(h) }) else {
        return std::ptr::null_mut();
    };
    match p
        .engine
        .recv_event_timeout(Duration::from_millis(timeout_ms.max(0) as u64))
    {
        Some(e) => new_string(&mut env, &wire::event_json(&e).to_string()),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeSnapshot(
    mut env: JNIEnv,
    _: JClass,
    h: jlong,
) -> jstring {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { player(h) }) else {
        return std::ptr::null_mut();
    };
    let s = wire::snapshot_json(&p.engine.snapshot()).to_string();
    new_string(&mut env, &s)
}

/// Fill `buffer` with `frames` interleaved frames for the host's audio
/// track; returns the frames carrying audio (the rest is silence).
/// Never blocks.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeRender(
    env: JNIEnv,
    _: JClass,
    h: jlong,
    buffer: JFloatArray,
    frames: jint,
) -> jint {
    // SAFETY: handle contract (module docs).
    let Some(p) = (unsafe { player(h) }) else {
        return 0;
    };
    let Ok(mut scratch) = p.scratch.try_lock() else {
        return 0;
    };
    let n = (frames.max(0) as usize * p.channels).min(scratch.len());
    let real = p.output.render(&mut scratch[..n]);
    let _ = env.set_float_array_region(&buffer, 0, &scratch[..n]);
    real as jint
}

/// The host's measured output latency (AudioTrack timestamps).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeSetOutputLatency(
    _: JNIEnv,
    _: JClass,
    h: jlong,
    nanos: jlong,
) {
    // SAFETY: handle contract (module docs).
    if let Some(p) = unsafe { player(h) } {
        p.output
            .set_latency(Duration::from_nanos(nanos.max(0) as u64));
    }
}

/// Audio route changed (headphones unplugged, Bluetooth): the engine
/// rebuilds its output and resumes from the audible position.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeReportRouteChange(
    _: JNIEnv,
    _: JClass,
    h: jlong,
) {
    // SAFETY: handle contract (module docs).
    if let Some(p) = unsafe { player(h) } {
        p.output.report_fault(OutputFault::RouteChanged);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_oneamp_OneAmpPlayer_nativeProtocolVersion(
    _: JNIEnv,
    _: JClass,
) -> jint {
    wire::PROTOCOL_VERSION as jint
}
