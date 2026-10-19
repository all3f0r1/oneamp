//! Remote playback against a local HTTP server: ranges, interruptions,
//! changed resources, expiring access, cancellation, live streams.

mod common;

use common::*;
use oneamp_core::access::{Access, AccessProvider, RedirectPolicy, RenewReason};
use oneamp_core::{
    AudioCommand, AudioEvent, EngineConfig, ErrorCategory, HttpConfig, MediaDescriptor, MediaId,
    PlayerError, PlayerState, RepeatMode,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const T: Duration = Duration::from_secs(15);

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

fn remote(server: &TestServer, id: &str) -> MediaDescriptor {
    MediaDescriptor::http(MediaId::new(id), server.url("/media/x.wav"))
}

fn load_result(rig: &mut Rig, id: oneamp_core::CommandId) -> Result<(), PlayerError> {
    match rig.wait_for(
        T,
        |e| matches!(e, AudioEvent::CommandCompleted { id: i, .. } if *i == id),
    ) {
        AudioEvent::CommandCompleted { result, .. } => result,
        _ => unreachable!(),
    }
}

#[test]
fn on_demand_media_plays_exactly_through_ranged_requests() {
    let (bytes, src) = wav(31, 48000, 2, 48000);
    let server = TestServer::start(bytes, Behavior::default());
    let mut rig = Rig::new(cfg(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: remote(&server, "item-31"),
        paused: false,
    });
    rig.finished(T);
    assert_eq!(audible(&rig.take()), src);
    // Bounded requests, not one unbounded GET.
    assert!(
        server
            .requests()
            .iter()
            .all(|r| r.to_lowercase().contains("range: bytes="))
    );
    assert!(server.count.load(Ordering::SeqCst) > 3);
}

#[test]
fn seeking_remote_media_uses_the_demuxer_and_byte_ranges() {
    let (bytes, src) = wav(32, 48000, 2, 96000);
    let server = TestServer::start(bytes, Behavior::default());
    let mut rig = Rig::new(cfg(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: remote(&server, "item-32"),
        paused: true,
    });
    rig.wait_state(T, PlayerState::Ready);
    assert!(rig.engine.snapshot().capabilities.seek);
    rig.send(AudioCommand::Seek(Duration::from_millis(1250)));
    rig.wait_for(T, |e| matches!(e, AudioEvent::Seeked { .. }));
    rig.send(AudioCommand::Resume);
    rig.finished(T);
    assert_eq!(audible(&rig.take()), &src[60000 * 2..]);
    // The demuxer turned the seek into a request at (or just before,
    // on a packet boundary) the target's byte offset: 44-byte header +
    // 60 000 frames × 4 bytes. Never a proportional guess, never from 0.
    let off = 44 + 60000 * 4;
    let starts: Vec<usize> = server
        .requests()
        .iter()
        .filter_map(|r| {
            let l = r
                .lines()
                .find(|l| l.to_lowercase().starts_with("range: bytes="))?;
            l.split('=').nth(1)?.split('-').next()?.parse().ok()
        })
        .collect();
    assert!(
        starts.iter().any(|&s| s <= off && off - s < 16 * 1024),
        "{starts:?}"
    );
}

#[test]
fn a_server_ignoring_range_plays_forward_only_and_refuses_seeks() {
    let (bytes, src) = wav(33, 48000, 2, 24000);
    let server = TestServer::start(
        bytes,
        Behavior {
            ignore_range: true,
            ..Default::default()
        },
    );
    let mut rig = Rig::new(cfg(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: remote(&server, "item-33"),
        paused: true,
    });
    rig.wait_state(T, PlayerState::Ready);
    assert!(!rig.engine.snapshot().capabilities.seek);
    let seek = rig.send(AudioCommand::Seek(Duration::from_millis(100)));
    let r = load_result(&mut rig, seek);
    assert_eq!(r.unwrap_err().category, ErrorCategory::Unsupported);
    rig.send(AudioCommand::Resume);
    rig.finished(T);
    assert_eq!(audible(&rig.take()), src);
}

#[test]
fn an_invalid_content_range_fails_the_load() {
    let (bytes, _) = wav(34, 48000, 2, 4800);
    let server = TestServer::start(
        bytes,
        Behavior {
            bad_content_range: true,
            ..Default::default()
        },
    );
    let mut rig = Rig::new(cfg(), Pace::Free);
    let id = rig.send(AudioCommand::Load {
        media: remote(&server, "item-34"),
        paused: false,
    });
    let e = load_result(&mut rig, id).unwrap_err();
    assert_eq!(e.category, ErrorCategory::InvalidResponse);
    assert_eq!(rig.engine.snapshot().state, PlayerState::Error);
}

#[test]
fn a_cut_transfer_resumes_at_the_exact_byte() {
    let (bytes, src) = wav(35, 48000, 2, 48000);
    let server = TestServer::start(
        bytes,
        Behavior {
            // Second request (first after the probe) dies mid-body.
            cut: Some((2, 10_000)),
            ..Default::default()
        },
    );
    let mut rig = Rig::new(cfg(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: remote(&server, "item-35"),
        paused: false,
    });
    rig.finished(T);
    assert_eq!(
        audible(&rig.take()),
        src,
        "no loss, no duplicate, no restart"
    );
    let reqs = server.requests();
    assert!(
        reqs.iter()
            .any(|r| r.to_lowercase().contains("if-range: \"v1\""))
    );
}

#[test]
fn a_resource_changed_mid_read_is_an_error_not_a_splice() {
    let (bytes, src) = wav(36, 48000, 2, 48000);
    let server = TestServer::start(
        bytes,
        Behavior {
            change_after: Some(2),
            ..Default::default()
        },
    );
    let mut rig = Rig::new(cfg(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: remote(&server, "item-36"),
        paused: false,
    });
    let e = rig.wait_for(T, |e| matches!(e, AudioEvent::Error { .. }));
    let AudioEvent::Error { error, .. } = e else {
        unreachable!()
    };
    assert_eq!(error.category, ErrorCategory::ResourceChanged);
    assert!(!error.recoverable);
    std::thread::sleep(Duration::from_millis(100));
    let out = audible(&rig.take());
    assert!(out.len() < src.len());
    assert_eq!(
        out,
        &src[..out.len()],
        "only the original bytes were played"
    );
}

struct Tokens {
    url: String,
    first: String,
    renewed: Option<String>,
    renewals: AtomicUsize,
    seen: Mutex<Vec<String>>,
}

impl AccessProvider for Tokens {
    fn access(&self) -> Result<Access, PlayerError> {
        Ok(Access::new(&self.url).with_header("Authorization", &self.first))
    }
    fn renew(&self, rejected: &Access, _: RenewReason) -> Result<Access, PlayerError> {
        self.renewals.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap()
            .push(rejected.headers[0].1.clone());
        match &self.renewed {
            Some(t) => Ok(Access::new(&self.url).with_header("Authorization", t)),
            None => Err(PlayerError::new(
                ErrorCategory::AccessExpired,
                "capability expired",
            )),
        }
    }
}

fn tokens(server: &TestServer, first: &str, renewed: Option<&str>) -> std::sync::Arc<Tokens> {
    std::sync::Arc::new(Tokens {
        url: server.url("/Audio/1/stream"),
        first: first.into(),
        renewed: renewed.map(str::to_string),
        renewals: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    })
}

struct Shared(std::sync::Arc<Tokens>);
impl AccessProvider for Shared {
    fn access(&self) -> Result<Access, PlayerError> {
        self.0.access()
    }
    fn renew(&self, r: &Access, why: RenewReason) -> Result<Access, PlayerError> {
        self.0.renew(r, why)
    }
}

#[test]
fn an_expired_access_is_renewed_once_and_the_identity_is_kept() {
    let (bytes, src) = wav(37, 48000, 2, 24000);
    let server = TestServer::start(
        bytes,
        Behavior {
            require_token: Some("good".into()),
            ..Default::default()
        },
    );
    let t = tokens(&server, "stale", Some("good"));
    let mut rig = Rig::new(cfg(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::http_with(MediaId::new("item-37"), Shared(t.clone())),
        paused: false,
    });
    let ev = rig.wait_for(T, |e| matches!(e, AudioEvent::TrackLoaded { .. }));
    let AudioEvent::TrackLoaded { track, .. } = ev else {
        unreachable!()
    };
    assert_eq!(track.id.as_str(), "item-37");
    rig.finished(T);
    assert_eq!(audible(&rig.take()), src);
    assert_eq!(t.renewals.load(Ordering::SeqCst), 1);
}

#[test]
fn a_refused_renewal_fails_without_retrying_forever() {
    let (bytes, _) = wav(38, 48000, 2, 4800);
    let server = TestServer::start(
        bytes,
        Behavior {
            require_token: Some("good".into()),
            ..Default::default()
        },
    );
    let t = tokens(&server, "stale", None);
    let mut rig = Rig::new(cfg(), Pace::Free);
    let id = rig.send(AudioCommand::Load {
        media: MediaDescriptor::http_with(MediaId::new("item-38"), Shared(t.clone())),
        paused: false,
    });
    let e = load_result(&mut rig, id).unwrap_err();
    assert_eq!(e.category, ErrorCategory::AccessExpired);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    assert_eq!(t.renewals.load(Ordering::SeqCst), 1);
}

#[test]
fn a_renewed_access_rejected_again_is_final() {
    let (bytes, _) = wav(39, 48000, 2, 4800);
    let server = TestServer::start(
        bytes,
        Behavior {
            require_token: Some("good".into()),
            ..Default::default()
        },
    );
    let t = tokens(&server, "stale", Some("still-stale"));
    let mut rig = Rig::new(cfg(), Pace::Free);
    let id = rig.send(AudioCommand::Load {
        media: MediaDescriptor::http_with(MediaId::new("item-39"), Shared(t.clone())),
        paused: false,
    });
    let e = load_result(&mut rig, id).unwrap_err();
    assert_eq!(e.category, ErrorCategory::AccessExpired);
    assert!(!e.recoverable);
    assert!(
        !e.message.contains("still-stale"),
        "secret leaked: {}",
        e.message
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(server.count.load(Ordering::SeqCst), 2);
    assert_eq!(t.renewals.load(Ordering::SeqCst), 1);
}

#[test]
fn forbidden_is_a_refusal_and_never_triggers_renewal() {
    let (bytes, _) = wav(40, 48000, 2, 4800);
    let server = TestServer::start(
        bytes,
        Behavior {
            forbid: true,
            ..Default::default()
        },
    );
    let t = tokens(&server, "token", Some("other"));
    let mut rig = Rig::new(cfg(), Pace::Free);
    let id = rig.send(AudioCommand::Load {
        media: MediaDescriptor::http_with(MediaId::new("item-40"), Shared(t.clone())),
        paused: false,
    });
    let e = load_result(&mut rig, id).unwrap_err();
    assert_eq!(e.category, ErrorCategory::AccessDenied);
    assert_eq!(t.renewals.load(Ordering::SeqCst), 0);
}

#[test]
fn remote_media_join_gaplessly_and_repeat() {
    let (ba, sa) = wav(43, 44100, 2, 22050);
    let (bb, sb) = wav(44, 44100, 2, 11025);
    let sa_srv = TestServer::start(ba, Behavior::default());
    let sb_srv = TestServer::start(bb, Behavior::default());
    let mut rig = Rig::new(cfg(), Pace::Free);
    rig.send(AudioCommand::Load {
        media: remote(&sa_srv, "a"),
        paused: false,
    });
    rig.send(AudioCommand::SetNext(Some(remote(&sb_srv, "b"))));
    rig.finished(T);
    let mut expect = sa.clone();
    expect.extend(&sb);
    assert_eq!(audible(&rig.take()), expect);

    // Repeat One on a remote media: two exact passes, each a new request.
    rig.send(AudioCommand::SetRepeatMode(RepeatMode::One));
    rig.send(AudioCommand::Load {
        media: remote(&sb_srv, "b"),
        paused: false,
    });
    rig.wait_for(T, |e| matches!(e, AudioEvent::TrackLoaded { .. }));
    rig.wait_for(T, |e| matches!(e, AudioEvent::TrackLoaded { .. }));
    rig.send(AudioCommand::SetRepeatMode(RepeatMode::Off));
    rig.finished(T);
    let out = audible(&rig.take());
    assert_eq!(&out[..sb.len()], &sb[..]);
    assert_eq!(&out[sb.len()..2 * sb.len()], &sb[..]);
}

#[test]
fn live_streams_carry_icy_titles_and_cannot_seek() {
    let (bytes, _) = wav(45, 48000, 2, 96000);
    let server = TestServer::start(
        bytes,
        Behavior {
            icy: Some((8192, "Artist - Song".into())),
            ..Default::default()
        },
    );
    let mut rig = Rig::new(cfg(), Pace::RealTime(1.0));
    rig.send(AudioCommand::Load {
        media: MediaDescriptor::http(MediaId::new("station"), server.url("/live")),
        paused: false,
    });
    let ev = rig.wait_for(T, |e| matches!(e, AudioEvent::IcyMetadata(_)));
    assert!(matches!(ev, AudioEvent::IcyMetadata(t) if t == "Artist - Song"));
    let snap = rig.engine.snapshot();
    assert!(snap.capabilities.live);
    assert!(!snap.capabilities.seek);
    assert_eq!(snap.duration, None);
    rig.send(AudioCommand::Stop);
}

#[test]
fn redirects_never_carry_secrets_to_another_origin() {
    let (bytes, src) = wav(46, 48000, 2, 4800);
    let target = TestServer::start(bytes, Behavior::default());
    let front = TestServer::start(
        Vec::new(),
        Behavior {
            redirect_to: Some(target.url("/cdn/x.wav")),
            ..Default::default()
        },
    );
    for (policy, forwarded) in [
        (RedirectPolicy::HeadersSameOriginOnly, false),
        (RedirectPolicy::HeadersToAnyOrigin, true),
    ] {
        target.state.lock().unwrap().requests.clear();
        let mut access = Access::new(front.url("/x")).with_header("Authorization", "Bearer SECRET");
        access.redirects = policy;
        let mut rig = Rig::new(cfg(), Pace::Free);
        rig.send(AudioCommand::Load {
            media: MediaDescriptor::http_with(
                MediaId::new("r"),
                oneamp_core::StaticAccess::from_access(access),
            ),
            paused: false,
        });
        rig.finished(T);
        assert_eq!(audible(&rig.take()), src);
        let leaked = target
            .requests()
            .iter()
            .any(|r| r.to_lowercase().contains("authorization"));
        assert_eq!(leaked, forwarded, "{policy:?}");
    }
    // Refuse: a redirect is an error.
    let mut access = Access::new(front.url("/x"));
    access.redirects = RedirectPolicy::Refuse;
    let mut rig = Rig::new(cfg(), Pace::Free);
    let id = rig.send(AudioCommand::Load {
        media: MediaDescriptor::http_with(
            MediaId::new("r"),
            oneamp_core::StaticAccess::from_access(access),
        ),
        paused: false,
    });
    assert_eq!(
        load_result(&mut rig, id).unwrap_err().category,
        ErrorCategory::InvalidResponse
    );
}

#[test]
fn segmented_delivery_is_refused_with_a_clear_error() {
    let server = TestServer::start(
        b"#EXTM3U\n".to_vec(),
        Behavior {
            content_type: Some("application/vnd.apple.mpegurl".into()),
            ..Default::default()
        },
    );
    let mut rig = Rig::new(cfg(), Pace::Free);
    let id = rig.send(AudioCommand::Load {
        media: remote(&server, "hls"),
        paused: false,
    });
    let e = load_result(&mut rig, id).unwrap_err();
    assert_eq!(e.category, ErrorCategory::Unsupported);
    assert!(e.message.contains("adapter"));
}

#[test]
fn a_delivery_starting_mid_media_reports_the_media_timeline() {
    // The server transcoded from 10 s: the bytes start there.
    let (bytes, src) = wav(47, 48000, 2, 48000);
    let server = TestServer::start(bytes, Behavior::default());
    let mut rig = Rig::new(cfg(), Pace::RealTime(1.0));
    rig.handle.set_latency(Duration::ZERO);
    rig.send(AudioCommand::Load {
        media: remote(&server, "item-47")
            .with_timeline_offset(Duration::from_secs(10))
            .starting_at(Duration::from_secs(10))
            .with_duration(Duration::from_secs(11)),
        paused: false,
    });
    rig.wait_state(T, PlayerState::Playing);
    std::thread::sleep(Duration::from_millis(300));
    let snap = rig.engine.snapshot();
    assert!(
        snap.position >= Duration::from_secs(10),
        "{:?}",
        snap.position
    );
    assert_eq!(snap.duration, Some(Duration::from_secs(11)));
    let id = rig.send(AudioCommand::Seek(Duration::from_secs(5)));
    assert_eq!(
        load_result(&mut rig, id).unwrap_err().category,
        ErrorCategory::InvalidRequest
    );
    let id = rig.send(AudioCommand::Seek(Duration::from_millis(10_500)));
    assert!(load_result(&mut rig, id).is_ok());
    rig.finished(T);
    let out = audible(&rig.take());
    let tail = &src[24000 * 2..];
    assert_eq!(&out[out.len() - tail.len()..], tail);
}
