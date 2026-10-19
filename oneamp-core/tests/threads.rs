//! Thread release. One test function in its own binary: counting the
//! process's threads is meaningless while other tests run beside it.

mod common;

use common::*;
use oneamp_core::{
    AudioCommand, AudioEvent, EngineConfig, ErrorCategory, HttpConfig, MediaDescriptor, MediaId,
    PlayerState,
};
use std::time::{Duration, Instant};

const T: Duration = Duration::from_secs(15);

fn settle(before: usize, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while thread_count() > before && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        thread_count() <= before,
        "{what}: {} threads left (was {before})",
        thread_count()
    );
}

fn cfg() -> EngineConfig {
    EngineConfig {
        http: HttpConfig {
            connect_timeout: Duration::from_secs(1),
            response_timeout: Duration::from_secs(1),
            idle_timeout: Duration::from_secs(2),
            request_bytes: 32 * 1024,
            read_ahead_bytes: 64 * 1024,
            max_retries: 3,
        },
        ..Default::default()
    }
}

#[test]
fn every_scenario_releases_its_threads() {
    let before = thread_count();

    // Local playback with a prepared successor, headless.
    {
        let (p, _) = wav_file("threads.wav", 51, 48000, 2, 4800);
        let mut rig = Rig::new(cfg(), Pace::Free);
        rig.send(AudioCommand::Load {
            media: MediaDescriptor::file(&p),
            paused: false,
        });
        rig.send(AudioCommand::SetNext(Some(MediaDescriptor::file(&p))));
        rig.finished(T);
    }
    settle(before, "local playback");

    // Stop while connecting to a server that never answers.
    {
        let server = TestServer::start(
            Vec::new(),
            Behavior {
                hang: true,
                ..Default::default()
            },
        );
        let mut rig = Rig::new(cfg(), Pace::Free);
        let load = rig.send(AudioCommand::Load {
            media: MediaDescriptor::http(MediaId::new("hang"), server.url("/x")),
            paused: false,
        });
        std::thread::sleep(Duration::from_millis(100));
        let t0 = Instant::now();
        rig.send(AudioCommand::Stop);
        rig.wait_for(T, |e| matches!(e, AudioEvent::Stopped));
        assert!(
            t0.elapsed() < Duration::from_millis(300),
            "stop took {:?}",
            t0.elapsed()
        );
        assert!(rig.events.iter().any(|ev| matches!(
            ev,
            AudioEvent::CommandCompleted { id, result: Err(e), .. }
                if *id == load && e.category == ErrorCategory::Cancelled
        )));
    }
    settle(before, "cancelled connect");

    // Stop while a read is stalled mid-body.
    {
        let (bytes, _) = wav(52, 48000, 2, 48000);
        let server = TestServer::start(
            bytes,
            Behavior {
                stall_after: Some(20_000),
                ..Default::default()
            },
        );
        let mut rig = Rig::new(cfg(), Pace::Free);
        rig.send(AudioCommand::Load {
            media: MediaDescriptor::http(MediaId::new("stall"), server.url("/x")),
            paused: false,
        });
        rig.wait_for(T, |e| matches!(e, AudioEvent::TrackLoaded { .. }));
        std::thread::sleep(Duration::from_millis(200));
        let t0 = Instant::now();
        rig.send(AudioCommand::Stop);
        rig.wait_for(T, |e| matches!(e, AudioEvent::Stopped));
        assert!(t0.elapsed() < Duration::from_millis(300));
    }
    settle(before, "cancelled read");

    // A remote successor still connecting is replaced, then playback stops.
    {
        let (bytes, _) = wav(53, 48000, 2, 96000);
        let server = TestServer::start(bytes, Behavior::default());
        let hang = TestServer::start(
            Vec::new(),
            Behavior {
                hang: true,
                ..Default::default()
            },
        );
        let cfg = EngineConfig {
            preload_lead: Duration::from_secs(60),
            ..cfg()
        };
        let mut rig = Rig::new(cfg, Pace::RealTime(1.0));
        rig.send(AudioCommand::Load {
            media: MediaDescriptor::http(MediaId::new("a"), server.url("/a")),
            paused: false,
        });
        rig.send(AudioCommand::SetNext(Some(MediaDescriptor::http(
            MediaId::new("slow-next"),
            hang.url("/n"),
        ))));
        rig.wait_state(T, PlayerState::Playing);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            rig.engine.snapshot().next.as_ref().map(|n| n.state),
            Some(oneamp_core::engine::NextState::Preparing)
        );
        rig.send(AudioCommand::SetNext(None));
        std::thread::sleep(Duration::from_millis(100));
        assert!(rig.engine.snapshot().next.is_none());
        rig.send(AudioCommand::Stop);
        rig.wait_for(T, |e| matches!(e, AudioEvent::Stopped));
    }
    settle(before, "replaced preparation");
}
