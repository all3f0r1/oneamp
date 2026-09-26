use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use oneamp_core::{AudioCommand, AudioEngine, AudioEvent, TrackInfo};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// A simple CLI audio player for MP3 and FLAC files
#[derive(Parser, Debug)]
#[command(name = "oneamp-cli")]
#[command(about = "OneAmp - A Winamp-like audio player CLI for Linux", long_about = None)]
struct Args {
    /// Path to the audio file to play
    #[arg(value_name = "FILE")]
    file: PathBuf,

    /// Show detailed metadata
    #[arg(short, long)]
    verbose: bool,
}

/// Display the track metadata the engine reads (same code path as the
/// desktop app).
fn display_metadata(file_path: &PathBuf) -> Result<()> {
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
    if let Some(secs) = info.duration_secs {
        let secs = secs as u64;
        println!("  Duration: {}:{:02}", secs / 60, secs % 60);
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    Ok(())
}

/// Play an audio file through the OneAmp engine (native-rate cpal
/// output, same DSP chain as the desktop app).
fn play_audio(file_path: &Path) -> Result<()> {
    let engine = AudioEngine::new().context("Failed to start audio engine")?;
    engine.send_command(AudioCommand::Play(file_path.to_path_buf()))?;

    println!("🎵 Now playing: {}", file_path.display());

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
        while let Some(event) = engine.try_recv_event() {
            match event {
                AudioEvent::Position(pos, total) => {
                    if total > 0.0 {
                        pb.set_length(total as u64);
                    }
                    pb.set_position(pos as u64);
                }
                AudioEvent::Finished | AudioEvent::Stopped | AudioEvent::RequestNext => {
                    pb.finish_with_message("✓ Playback complete");
                    return Ok(());
                }
                AudioEvent::Error(e) => {
                    pb.abandon();
                    anyhow::bail!(e);
                }
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Verify the file exists
    if !args.file.exists() {
        anyhow::bail!("File not found: {}", args.file.display());
    }

    // Verify the file has a supported extension
    let ext = args.file.extension().and_then(|e| e.to_str()).unwrap_or("");

    const SUPPORTED: &[&str] = &[
        "mp3", "flac", "ogg", "oga", "wav", "aac", "m4a", "m4b", "mp4", "alac", "aif", "aiff",
        "caf", "mka",
    ];
    if !SUPPORTED.contains(&ext.to_lowercase().as_str()) {
        anyhow::bail!(
            "Unsupported file format: {}. Supported formats: MP3, FLAC, OGG, WAV, AAC, M4A, MP4, ALAC, AIFF, CAF, MKA",
            ext
        );
    }

    println!("\n🎧 OneAmp CLI v{}", env!("CARGO_PKG_VERSION"));

    // Display metadata
    if args.verbose {
        display_metadata(&args.file)?;
    }

    // Play the audio file
    play_audio(&args.file)?;

    println!("\n👋 Thanks for using OneAmp!\n");

    Ok(())
}
