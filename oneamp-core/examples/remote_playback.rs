//! Play remote media the way a library client does: a stable media id,
//! an access the app resolves and renews, host metadata, a start
//! position, and the next item prepared ahead of time.
//!
//! ```text
//! cargo run -p oneamp-core --example remote_playback -- <url> [next-url] [--start SECS] [--device NAME]
//! ```
//!
//! `--list-devices` prints the output names; ALSA's discard device
//! ("Discard all samples…" on Linux) runs the whole cpal path silently.

use oneamp_core::access::{Access, AccessProvider, RenewReason};
use oneamp_core::{
    AudioCommand, AudioEngine, AudioEvent, ErrorCategory, MediaDescriptor, MediaId, MediaMetadata,
    PlayerError,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Stands in for the app's credential service. A real one would mint a
/// short-lived, media-scoped credential with the user's session (kept
/// out of URLs) and renew it only when the server rejects it.
struct AppAccess {
    url: String,
    renewals: AtomicU32,
}

impl AccessProvider for AppAccess {
    fn access(&self) -> Result<Access, PlayerError> {
        Ok(Access::new(&self.url))
    }

    fn renew(&self, rejected: &Access, reason: RenewReason) -> Result<Access, PlayerError> {
        let n = self.renewals.fetch_add(1, Ordering::Relaxed);
        eprintln!("renewal requested ({reason:?}) for {:?}", rejected);
        if n > 0 {
            // Never loop: refuse instead of minting forever.
            return Err(PlayerError::new(
                ErrorCategory::AccessExpired,
                "renewal refused",
            ));
        }
        Ok(Access::new(&self.url))
    }
}

fn media(id: &str, url: &str) -> MediaDescriptor {
    let local = !url.starts_with("http://") && !url.starts_with("https://");
    if local {
        return MediaDescriptor::file(url);
    }
    MediaDescriptor::http_with(
        MediaId::new(id),
        AppAccess {
            url: url.to_string(),
            renewals: AtomicU32::new(0),
        },
    )
    .with_metadata(MediaMetadata {
        title: Some(format!("Item {id}")),
        ..Default::default()
    })
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut urls = Vec::new();
    let (mut start, mut device) = (None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--start" => start = args.next().and_then(|s| s.parse::<f64>().ok()),
            "--device" => device = args.next(),
            "--list-devices" => {
                for d in oneamp_core::list_output_devices() {
                    println!("{d}");
                }
                return Ok(());
            }
            _ => urls.push(a),
        }
    }
    anyhow::ensure!(
        !urls.is_empty(),
        "usage: remote_playback <url> [next-url] [--start SECS] [--device NAME]"
    );

    let engine = AudioEngine::new()?;
    engine.send(AudioCommand::SetOutputDevice(device))?;
    let mut first = media("item-1", &urls[0]);
    if let Some(s) = start {
        first = first.starting_at(Duration::from_secs_f64(s));
    }
    let load = engine.send(AudioCommand::Load {
        media: first,
        paused: false,
    })?;
    if let Some(next) = urls.get(1) {
        engine.send(AudioCommand::SetNext(Some(media("item-2", next))))?;
    }

    loop {
        let Some(event) = engine.recv_event_timeout(Duration::from_secs(1)) else {
            let s = engine.snapshot();
            println!(
                "{:?} {:.1}s / {:?} (decoded to {:.1}s)",
                s.state,
                s.position.as_secs_f64(),
                s.duration.map(|d| d.as_secs()),
                s.decoded_position.as_secs_f64()
            );
            continue;
        };
        match event {
            AudioEvent::CommandCompleted { id, result, .. } if id == load => {
                println!("load: {result:?}");
                result?;
            }
            AudioEvent::TrackLoaded { instance, track } => {
                println!(
                    "now playing #{} {} ({:?})",
                    instance.0, track.id, track.title
                );
            }
            AudioEvent::OutputChanged(info) => println!("output: {info:?}"),
            AudioEvent::Error { error, .. } => {
                eprintln!("error: {error}");
                if !error.recoverable {
                    anyhow::bail!(error);
                }
            }
            AudioEvent::Finished => break,
            _ => {}
        }
    }
    let snap = engine.snapshot();
    println!("finished; diagnostics: {:?}", snap.diagnostics);
    engine.shutdown()
}
