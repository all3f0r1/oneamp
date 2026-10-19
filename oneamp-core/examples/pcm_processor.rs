//! The processing chain on its own: blocks in, blocks out, no thread,
//! no device, no network — what a platform player embeds.
//!
//! ```text
//! cargo run -p oneamp-core --no-default-features --example pcm_processor
//! ```

use oneamp_core::dsp::{
    DspParams, ListeningContext, MediaGain, Processor, ProcessorConfig, ReplayGainMode,
};
use oneamp_core::settings::{AudioSettings, EqCurve};

fn main() {
    // The app's stored settings document (versioned JSON), turned into
    // chain parameters.
    let settings = AudioSettings {
        eq_enabled: true,
        eq: EqCurve::from_slice(&[4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0], -3.0),
        replaygain_enabled: true,
        replaygain_mode: ReplayGainMode::Auto,
        ..Default::default()
    };
    let json = settings.to_json();
    let restored = AudioSettings::from_json(&json).expect("round trip");
    let params: DspParams = restored.dsp_params(&restored.eq);

    // Allocation happens here, never in `process`.
    let config = ProcessorConfig::new(48_000, 2).expect("stereo at 48 kHz");
    let mut p = Processor::new(config, &params).expect("valid config");
    // Gains the app's library already knows; album context for Auto.
    p.set_media_gain(
        MediaGain {
            track_gain_db: Some(-7.1),
            album_gain_db: Some(-6.4),
            ..Default::default()
        },
        ListeningContext::Album,
    );

    let mut block = vec![0.0f32; 512 * 2];
    let mut t = 0usize;
    let mut out_samples = 0;
    for _ in 0..100 {
        for frame in block.chunks_mut(2) {
            let s = 0.5 * (std::f32::consts::TAU * 220.0 * t as f32 / 48_000.0).sin();
            frame.fill(s);
            t += 1;
        }
        out_samples += p.process(&mut block);
    }
    // End of the stream: the limiter's lookahead still holds a few frames.
    let mut tail = vec![0.0f32; p.latency_frames() * 2];
    out_samples += p.flush(&mut tail);

    let active = p.active();
    println!("in {} samples, out {out_samples}", 100 * 512 * 2);
    println!(
        "latency {} frames, ReplayGain {:.1} dB (album, from Auto), EQ on: {}, bypassed: {}",
        p.latency_frames(),
        active.replaygain_db,
        active.eq,
        active.bypassed
    );
    // Seek or media change: drop filter history.
    p.reset();
}
