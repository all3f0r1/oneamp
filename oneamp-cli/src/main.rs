use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use oneamp_core::{
    AudioCommand, AudioEngine, AudioEvent, MediaDescriptor, MediaId, PlayQueue, TrackInfo,
    is_stream_url,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A simple CLI audio player
#[derive(Parser, Debug)]
#[command(name = "oneamp-cli")]
#[command(about = "OneAmp - A Winamp-like audio player CLI for Linux", long_about = None)]
struct Args {
    /// Audio files or http(s) URLs to play, in order. Consecutive files
    /// join gaplessly.
    #[arg(value_name = "FILE", required = true)]
    files: Vec<PathBuf>,

    /// Show detailed metadata
    #[arg(short, long)]
    verbose: bool,

    /// Start the first item at this position (seconds)
    #[arg(long, value_name = "SECONDS")]
    start: Option<f64>,
}

const SUPPORTED: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "wav", "aac", "m4a", "m4b", "mp4", "alac", "aif", "aiff", "caf",
    "mka",
];

/// Display the track metadata the engine reads (same code path as the
/// desktop app).
fn display_metadata(file_path: &Path) -> Result<()> {
    let info = TrackInfo::from_file(file_path).context("Failed to read metadata")?;

    println!("\n📀 Track Information:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let fields = [
        ("Title:  ", info.title.clone()),
        ("Artist: ", info.artist.clone()),
        ("Album:  ", info.album.clone()),
        ("Year:   ", info.year.map(|y| y.to_string())),
        ("Codec:  ", info.codec.clone()),
        ("Sample Rate: ", info.sample_rate.map(|r| format!("{r} Hz"))),
        ("Channels: ", info.channels.map(|c| c.to_string())),
    ];
    for (label, value) in fields {
        if let Some(v) = value {
            println!("  {label}{v}");
        }
    }
    if let Some(d) = info.duration {
        let secs = d.as_secs();
        println!("  Duration: {}:{:02}", secs / 60, secs % 60);
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    Ok(())
}

fn descriptor(path: &Path) -> MediaDescriptor {
    match path.to_str().filter(|_| is_stream_url(path)) {
        Some(url) => MediaDescriptor::http(MediaId::new(url), url),
        None => MediaDescriptor::file(path),
    }
}

/// Play the items through the OneAmp engine: the queue says what comes
/// next, the engine prepares it ahead and joins it without a gap.
fn play(items: Vec<MediaDescriptor>) -> Result<()> {
    let engine = AudioEngine::new().context("Failed to start audio engine")?;
    let mut queue = PlayQueue::new(items);
    let first = queue.advance().cloned().context("nothing to play")?;
    engine.send(AudioCommand::Load {
        media: first,
        paused: false,
    })?;
    engine.send(AudioCommand::SetNext(queue.peek_next().cloned()))?;

    let pb = ProgressBar::new(0);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s {msg}",
            )
            .unwrap()
            .progress_chars("#>-"),
    );

    loop {
        let Some(event) = engine.recv_event_timeout(Duration::from_millis(100)) else {
            continue;
        };
        if let Some(next) = queue.follow(&event) {
            engine.send(AudioCommand::SetNext(next))?;
        }
        match event {
            AudioEvent::TrackLoaded { track, .. } => {
                pb.println(format!(
                    "🎵 Now playing: {}",
                    track.title.as_deref().unwrap_or(track.id.as_str())
                ));
                pb.set_position(0);
            }
            AudioEvent::Position { position, duration } => {
                if let Some(d) = duration {
                    pb.set_length(d.as_secs());
                }
                pb.set_position(position.as_secs());
            }
            AudioEvent::IcyMetadata(title) => pb.set_message(title),
            AudioEvent::Finished | AudioEvent::Stopped | AudioEvent::RequestNext => {
                pb.finish_with_message("✓ Playback complete");
                return Ok(());
            }
            AudioEvent::Error { error, .. } => {
                pb.abandon();
                anyhow::bail!(error);
            }
            _ => {}
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    for file in &args.files {
        if is_stream_url(file) {
            continue;
        }
        if !file.exists() {
            anyhow::bail!("File not found: {}", file.display());
        }
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !SUPPORTED.contains(&ext.to_lowercase().as_str()) {
            anyhow::bail!(
                "Unsupported file format: {}. Supported formats: MP3, FLAC, OGG, WAV, AAC, M4A, MP4, ALAC, AIFF, CAF, MKA",
                ext
            );
        }
    }

    println!("\n🎧 OneAmp CLI v{}", env!("CARGO_PKG_VERSION"));

    if args.verbose {
        for file in args.files.iter().filter(|f| !is_stream_url(f)) {
            display_metadata(file)?;
        }
    }

    let mut items: Vec<MediaDescriptor> = args.files.iter().map(|f| descriptor(f)).collect();
    if let (Some(start), Some(first)) = (args.start, items.first_mut()) {
        anyhow::ensure!(start >= 0.0 && start.is_finite(), "--start must be ≥ 0");
        *first = first.clone().starting_at(Duration::from_secs_f64(start));
    }
    play(items)?;

    println!("\n👋 Thanks for using OneAmp!\n");

    Ok(())
}
