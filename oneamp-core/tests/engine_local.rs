//! The full engine on local files, with a memory output and no UI.

mod common;

use common::*;
use oneamp_core::dsp::MultichannelMode;
use oneamp_core::{
    AudioCommand, AudioEvent, AudioSettings, EngineConfig, MediaDescriptor, OutputPolicy,
    PlayerState, RepeatMode,
};
use std::time::Duration;

const T: Duration = Duration::from_secs(10);

fn loaded(rig: &mut Rig) -> oneamp_core::PlaybackInstance {
    match rig.wait_for(T, |e| matches!(e, AudioEvent::TrackLoaded { .. })) {
        AudioEvent::TrackLoaded { instance, .. } => instance,
        _ => unreachable!(),
    }
}

#[test]
fn plays_a_file_bit_exact_and_finishes() {
    let (path, src) = wav_file("exact.wav", 1, 48000, 2, 24000);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&path),
        paused: false,
    });
    rig.finished(T);
    let out = rig.take();
    assert_eq!(trim(&out, 2, latency(48000), src.len()), src);
    // Nothing but the drain padding after the media.
    assert!(
        out[latency(48000) * 2 + src.len()..]
            .iter()
            .all(|&s| s == 0.0)
    );
    let snap = rig.engine.snapshot();
    assert_eq!(snap.state, PlayerState::Ended);
}

#[test]
fn starts_at_a_position_without_playing_the_beginning() {
    let (path, src) = wav_file("start.wav", 2, 48000, 2, 48000);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&path).starting_at(Duration::from_millis(500)),
        paused: false,
    });
    rig.finished(T);
    let out = rig.take();
    let expect = &src[24000 * 2..];
    assert_eq!(trim(&out, 2, latency(48000), expect.len()), expect);
}

#[test]
fn load_paused_then_resume_and_seek_reaches_the_exact_frame() {
    let (path, src) = wav_file("seek.wav", 3, 44100, 2, 44100 * 2);
    let mut rig = Rig::new(EngineConfig::default(), Pace::RealTime(4.0));
    let load = rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&path),
        paused: true,
    });
    rig.wait_for(
        T,
        |e| matches!(e, AudioEvent::CommandCompleted { id, result: Ok(()), .. } if *id == load),
    );
    assert_eq!(rig.engine.snapshot().state, PlayerState::Ready);
    std::thread::sleep(Duration::from_millis(100));
    assert!(rig.take().iter().all(|&s| s == 0.0), "Ready must be silent");

    rig.send(AudioCommand::Resume);
    rig.wait_state(T, PlayerState::Playing);
    std::thread::sleep(Duration::from_millis(100));
    let seek = rig.send(AudioCommand::Seek(Duration::from_millis(1500)));
    let ev = rig.wait_for(T, |e| matches!(e, AudioEvent::Seeked { .. }));
    let AudioEvent::Seeked { position, .. } = ev else {
        unreachable!()
    };
    assert_eq!(position, Duration::from_millis(1500));
    assert!(rig.events.iter().any(
        |e| matches!(e, AudioEvent::CommandCompleted { id, result: Ok(()), .. } if *id == seek)
    ));
    rig.finished(T);
    let out = audible(&rig.take());
    // The run after the seek is exactly the source from 1.5 s on.
    let tail = &src[(44100 * 3 / 2) * 2..];
    let probe = &tail[..2000];
    let at = find(&out, probe).expect("post-seek audio present");
    assert_eq!(&out[at..at + tail.len()], tail);
}

#[test]
fn seeking_to_the_last_seconds_and_past_the_end_works() {
    let (path, src) = wav_file("tail.wav", 4, 48000, 1, 48000 * 3);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&path),
        paused: true,
    });
    rig.wait_state(T, PlayerState::Ready);
    // 0.5 s before the end: the old engine clamped this to end - 2 s.
    rig.send(AudioCommand::Seek(Duration::from_millis(2500)));
    rig.wait_for(T, |e| matches!(e, AudioEvent::Seeked { .. }));
    rig.send(AudioCommand::Resume);
    rig.finished(T);
    let out = rig.take();
    let tail = &src[48000 * 5 / 2..];
    assert_eq!(trim(&out, 1, latency(48000), tail.len()), tail);

    // Past the end: reaches the end, finishes normally.
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&path),
        paused: false,
    });
    rig.wait_state(T, PlayerState::Playing);
    rig.send(AudioCommand::Seek(Duration::from_secs(60)));
    rig.finished(T);
}

#[test]
fn gapless_join_has_no_loss_or_duplication() {
    let (a, sa) = wav_file("ga.wav", 5, 44100, 2, 30000);
    let (b, sb) = wav_file("gb.wav", 6, 44100, 2, 25000);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&b))));
    let first = loaded(&mut rig);
    let second = loaded(&mut rig);
    assert_ne!(first, second);
    rig.finished(T);
    let out = rig.take();
    let mut expect = sa.clone();
    expect.extend(&sb);
    assert_eq!(trim(&out, 2, latency(44100), expect.len()), expect);
    assert_eq!(
        rig.events
            .iter()
            .filter(|e| matches!(e, AudioEvent::Finished))
            .count(),
        1
    );
}

#[test]
fn fixed_output_joins_different_rates_gaplessly() {
    let (a, sa) = wav_file("fa.wav", 7, 44100, 2, 44100);
    let (b, _) = wav_file("fb.wav", 8, 48000, 1, 24000);
    let cfg = EngineConfig {
        output_policy: OutputPolicy::Fixed {
            sample_rate: 48000,
            channels: 2,
        },
        ..Default::default()
    };
    let mut rig = Rig::new(cfg, Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&b))));
    rig.finished(T);
    let out = rig.take();
    let snap_reopens = rig
        .events
        .iter()
        .filter(|e| matches!(e, AudioEvent::OutputChanged(_)))
        .count();
    assert_eq!(snap_reopens, 1, "one output for both rates");
    // 1 s at 44.1 k → exactly 48 000 frames, then 24 000 mono frames
    // duplicated to stereo: no gap, no overlap.
    let lat = latency(48000) * 2;
    let audio = &out[lat..lat + (48000 + 24000) * 2];
    let b_start = 48000 * 2;
    assert_eq!(audio[b_start], audio[b_start + 1], "mono duplicated");
    assert!(audio[..b_start].iter().any(|&s| s != 0.0));
    let _ = sa;
    // The second media is the untouched mono source on both channels.
    let (_, sb) = common::wav(8, 48000, 1, 24000);
    let got_b: Vec<f32> = audio[b_start..].chunks(2).map(|f| f[0]).collect();
    assert_eq!(got_b, sb);
}

#[test]
fn native_policy_reopens_for_a_new_format_without_the_host() {
    let (a, sa) = wav_file("na.wav", 9, 44100, 2, 8000);
    let (b, sb) = wav_file("nb.wav", 10, 48000, 2, 8000);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&b))));
    rig.finished(T);
    let out = rig.take();
    assert!(find(&out, &sa[..400]).is_some());
    let at = find(&out, &sb[..400]).expect("second media played");
    assert_eq!(&out[at..at + sb.len()], &sb[..]);
    assert_eq!(
        rig.events
            .iter()
            .filter(|e| matches!(e, AudioEvent::OutputChanged(_)))
            .count(),
        2
    );
}

#[test]
fn crossfade_overlaps_by_its_length_and_keeps_each_media_gain() {
    let (a, sa) = wav_file("xa.wav", 11, 48000, 2, 48000);
    let (b, sb) = wav_file("xb.wav", 12, 48000, 2, 48000);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::SetCrossfade(true, 0.5));
    rig.send(AudioCommand::SetReplayGainMode(
        oneamp_core::ReplayGainMode::Track,
    ));
    // A carries a -6.02 dB track gain from the host, B none.
    let half = oneamp_core::MediaGain {
        track_gain_db: Some(-6.0206),
        ..Default::default()
    };
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a).with_gain(half),
        paused: false,
    });
    rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&b))));
    rig.finished(T);
    let out = rig.take();
    let lat = latency(48000) * 2;
    // Non-silent span: 1 s + 1 s − 0.5 s overlap, to the frame.
    let last = out.iter().rposition(|&s| s != 0.0).unwrap() / 2;
    let first = out.iter().position(|&s| s != 0.0).unwrap() / 2;
    assert_eq!(last - first + 1, 72_000);
    // Before the fade: A at its own gain.
    for i in (0..20_000).step_by(997) {
        assert!((out[lat + i] - sa[i] * 0.5).abs() < 1e-4, "A at {i}");
    }
    // After the fade: B untouched (its gain is unity, not A's).
    let b_solo = 24_000 * 2;
    let got = &out[lat + 48_000 * 2..lat + 72_000 * 2];
    assert_eq!(got, &sb[b_solo..]);
}

#[test]
fn repeat_one_replays_gaplessly_and_stops_when_turned_off() {
    let (a, sa) = wav_file("r1.wav", 13, 48000, 2, 12000);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::SetRepeatMode(RepeatMode::One));
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    let i1 = loaded(&mut rig);
    let i2 = loaded(&mut rig);
    assert_ne!(i1, i2, "each pass is its own instance");
    rig.send(AudioCommand::SetRepeatMode(RepeatMode::Off));
    rig.finished(T);
    let out = rig.take();
    let lat = latency(48000) * 2;
    let n = sa.len();
    assert_eq!(&out[lat..lat + n], &sa[..]);
    assert_eq!(
        &out[lat + n..lat + 2 * n],
        &sa[..],
        "second pass joins with no gap"
    );
}

#[test]
fn replacing_the_next_media_cancels_the_old_preparation() {
    // Long enough that the replacement arrives before A's end is
    // written to the output (the transition is committed then).
    let (a, sa) = wav_file("qa.wav", 14, 48000, 2, 96000);
    let (b, sb) = wav_file("qb.wav", 15, 48000, 2, 6000);
    let (c, sc) = wav_file("qc.wav", 16, 48000, 2, 6000);
    let cfg = EngineConfig {
        preload_lead: Duration::from_secs(60),
        ..Default::default()
    };
    let mut rig = Rig::new(cfg, Pace::RealTime(2.0));
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&b))));
    rig.wait_for(T, |e| matches!(e, AudioEvent::NextPrepared { .. }));
    rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&c))));
    rig.finished(T);
    let out = audible(&rig.take());
    assert!(find(&out, &sa[..200]).is_some());
    assert!(
        find(&out, &sb[..200]).is_none(),
        "replaced media must not play"
    );
    let at = find(&out, &sc[..200]).expect("replacement played");
    assert_eq!(&out[at..at + sc.len()], &sc[..]);
}

#[test]
fn abandoned_loads_never_leak_into_the_next_one() {
    let (a, _) = wav_file("la.wav", 17, 48000, 2, 48000);
    let (b, sb) = wav_file("lb.wav", 18, 48000, 2, 9600);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    let mut ids = Vec::new();
    for i in 0..40 {
        ids.push(rig.send(match i % 4 {
            0 => AudioCommand::Load {
                media: MediaDescriptor::file(&a),
                paused: false,
            },
            1 => AudioCommand::Seek(Duration::from_millis(300)),
            2 => AudioCommand::Pause,
            _ => AudioCommand::Stop,
        }));
    }
    let last = rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&b),
        paused: false,
    });
    rig.finished(T);
    // Exactly one TrackLoaded after the final load, for its instance.
    let done = rig
        .events
        .iter()
        .position(|e| matches!(e, AudioEvent::CommandCompleted { id, .. } if *id == last))
        .expect("last load completed");
    let instance = match &rig.events[done] {
        AudioEvent::CommandCompleted { instance, .. } => instance.unwrap(),
        _ => unreachable!(),
    };
    let loads_after: Vec<_> = rig.events[done.saturating_sub(1)..]
        .iter()
        .filter_map(|e| match e {
            AudioEvent::TrackLoaded { instance, .. } => Some(*instance),
            _ => None,
        })
        .collect();
    assert_eq!(loads_after, vec![instance]);
    // Each command got at most one completion.
    for id in ids {
        let n = rig
            .events
            .iter()
            .filter(|e| matches!(e, AudioEvent::CommandCompleted { id: i, .. } if *i == id))
            .count();
        assert!(n <= 1);
    }
    let out = rig.take();
    let at = find(&out, &sb[..200]).expect("final media played");
    assert_eq!(&out[at..at + sb.len()], &sb[..]);
}

#[test]
fn a_slow_event_consumer_keeps_memory_bounded() {
    let (a, _) = wav_file("slow.wav", 19, 48000, 2, 48000);
    let cfg = EngineConfig {
        event_capacity: 16,
        ..Default::default()
    };
    let rig = Rig::new(cfg, Pace::RealTime(4.0));
    for _ in 0..30 {
        rig.send(AudioCommand::SetVolume(0.5));
        rig.send(AudioCommand::SetRepeatMode(RepeatMode::Off));
    }
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    std::thread::sleep(Duration::from_millis(400));
    // Nobody read anything: still bounded, losses counted.
    let snap = rig.engine.snapshot();
    assert!(snap.diagnostics.events_dropped > 0);
    let mut n = 0;
    let mut reported = false;
    while let Some(e) = rig.engine.try_recv_event() {
        reported |= matches!(e, AudioEvent::EventsDropped(_));
        n += 1;
    }
    assert!(n <= 17, "{n} events queued");
    assert!(reported);
}

#[test]
fn surround_is_folded_for_a_stereo_chain_or_passed_through() {
    let (p, _) = wav_file("six.wav", 20, 48000, 6, 4800);
    let mut rig = Rig::new(EngineConfig::default(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&p),
        paused: false,
    });
    rig.wait_state(T, PlayerState::Playing);
    let snap = rig.engine.snapshot();
    assert_eq!(snap.output.as_ref().unwrap().input_channels, 2);
    assert_eq!(snap.processing.chain.fold_down_from.as_deref(), Some("5.1"));
    rig.finished(T);

    let settings = AudioSettings {
        multichannel: MultichannelMode::Passthrough,
        ..Default::default()
    };
    rig.send(AudioCommand::ApplySettings(Box::new(settings)));
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&p),
        paused: false,
    });
    rig.wait_state(T, PlayerState::Playing);
    let snap = rig.engine.snapshot();
    assert_eq!(snap.output.as_ref().unwrap().input_channels, 6);
    assert_eq!(snap.processing.chain.fold_down_from, None);
}

#[test]
fn audible_position_tracks_what_the_device_consumed() {
    let (p, _) = wav_file("pos.wav", 21, 48000, 2, 48000 * 3);
    let mut rig = Rig::new(EngineConfig::default(), Pace::RealTime(1.0));
    rig.handle.set_latency(Duration::ZERO);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&p),
        paused: false,
    });
    rig.wait_state(T, PlayerState::Playing);
    std::thread::sleep(Duration::from_millis(800));
    let consumed = rig.collected.lock().unwrap().len() as f64 / 2.0 / 48000.0;
    let snap = rig.engine.snapshot();
    let pos = snap.position.as_secs_f64();
    // The device has consumed `consumed` s (incl. 2 ms of limiter
    // pre-roll); the decoder is ~0.5 s+ ahead. Snapshot granularity 50 ms.
    assert!(
        (pos - consumed).abs() < 0.08,
        "position {pos} vs consumed {consumed}"
    );
    assert!(snap.decoded_position.as_secs_f64() > pos + 0.2);
}

#[test]
fn route_change_reopens_and_resumes_from_the_audible_frame() {
    let (p, src) = wav_file("route.wav", 22, 48000, 2, 48000 * 2);
    let mut rig = Rig::new(EngineConfig::default(), Pace::RealTime(2.0));
    rig.handle.set_latency(Duration::ZERO);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&p),
        paused: false,
    });
    rig.wait_state(T, PlayerState::Playing);
    std::thread::sleep(Duration::from_millis(300));
    rig.handle
        .report_fault(oneamp_core::output::OutputFault::RouteChanged);
    rig.finished(T);
    let reopened = rig
        .events
        .iter()
        .filter(|e| matches!(e, AudioEvent::OutputChanged(_)))
        .count();
    assert_eq!(reopened, 2);
    // Every source frame was heard; at most the few frames in flight at
    // the switch may repeat.
    let out = audible(&rig.take());
    let probe_end = &src[src.len() - 2000..];
    assert!(find(&out, probe_end).is_some(), "played to the end");
    let nonzero = out.iter().filter(|&&s| s != 0.0).count();
    assert!(nonzero >= src.iter().filter(|&&s| s != 0.0).count());
}

#[test]
fn eq_auto_follows_the_media_and_never_overwrites_the_global_curve() {
    use oneamp_core::{EqCurve, MediaId};
    let (a, _) = wav_file("eqa.wav", 24, 48000, 2, 24000);
    let (b, _) = wav_file("eqb.wav", 25, 48000, 2, 24000);
    let mut rig = Rig::new(EngineConfig::default(), Pace::RealTime(2.0));
    let global = EqCurve::from_slice(&[1.0; 10], -1.0);
    let preset = EqCurve::from_slice(&[6.0; 10], -4.0);
    rig.send(AudioCommand::SetEqualizerBands(global.gains_db.to_vec()));
    rig.send(AudioCommand::SetEqualizerPreamp(global.preamp_db));
    rig.send(AudioCommand::SetEqAutoPreset(
        MediaId::from_path(&a),
        Some(preset),
    ));
    rig.send(AudioCommand::SetEqAuto(true));
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&b))));
    rig.wait_for(T, |e| matches!(e, AudioEvent::EqAutoApplied(Some(_))));
    let snap = rig.engine.snapshot();
    assert_eq!(snap.eq.active, preset);
    assert_eq!(snap.eq.global, global, "preset never becomes global");
    // The next media has no preset: back to the global curve.
    rig.wait_for(T, |e| matches!(e, AudioEvent::EqAutoApplied(None)));
    assert_eq!(rig.engine.snapshot().eq.active, global);
    rig.finished(T);

    // A hand edit while a preset plays makes the edited curve global.
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::file(&a),
        paused: false,
    });
    rig.wait_for(T, |e| matches!(e, AudioEvent::EqAutoApplied(Some(_))));
    rig.send(AudioCommand::SetEqualizerBand(0, 2.0));
    rig.wait_for(T, |e| matches!(e, AudioEvent::EqAutoApplied(None)));
    let mut edited = preset;
    edited.gains_db[0] = 2.0;
    let snap = rig.engine.snapshot();
    assert_eq!(snap.eq.global, edited);
    assert_eq!(snap.eq.auto_for, None);
}
