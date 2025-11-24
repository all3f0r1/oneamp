use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use rodio::{Decoder, OutputStream, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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

/// Extract and display metadata from an audio file using Symphonia
fn display_metadata(file_path: &PathBuf) -> Result<()> {
    let file = File::open(file_path)
        .context("Failed to open audio file for metadata reading")?;
    
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    
    let mut hint = Hint::new();
    if let Some(ext) = file_path.extension() {
        hint.with_extension(ext.to_str().unwrap_or(""));
    }
    
    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();
    
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .context("Failed to probe audio file")?;
    
    let mut format = probed.format;
    
    println!("\n📀 Track Information:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    // Get metadata from the format
    if let Some(metadata_rev) = format.metadata().current() {
        for tag in metadata_rev.tags() {
            match tag.std_key {
                Some(symphonia::core::meta::StandardTagKey::TrackTitle) => {
                    println!("  Title:  {}", tag.value);
                }
                Some(symphonia::core::meta::StandardTagKey::Artist) => {
                    println!("  Artist: {}", tag.value);
                }
                Some(symphonia::core::meta::StandardTagKey::Album) => {
                    println!("  Album:  {}", tag.value);
                }
                Some(symphonia::core::meta::StandardTagKey::Date) => {
                    println!("  Year:   {}", tag.value);
                }
                _ => {}
            }
        }
    }
    
    // Get track information
    if let Some(track) = format.default_track() {
        let codec_params = &track.codec_params;
        
        if let Some(sample_rate) = codec_params.sample_rate {
            println!("  Sample Rate: {} Hz", sample_rate);
        }
        
        if let Some(channels) = codec_params.channels {
            println!("  Channels: {}", channels.count());
        }
        
        if let Some(n_frames) = codec_params.n_frames {
            if let Some(sample_rate) = codec_params.sample_rate {
                let duration_secs = n_frames / sample_rate as u64;
                println!("  Duration: {}:{:02}", duration_secs / 60, duration_secs % 60);
            }
        }
    }
    
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    
    Ok(())
}

/// Play an audio file using rodio
fn play_audio(file_path: &PathBuf) -> Result<()> {
    // Get a output stream handle to the default physical sound device
    let (_stream, stream_handle) = OutputStream::try_default()
        .context("Failed to get default audio output device")?;
    
    // Create a sink to control playback
    let sink = Sink::try_new(&stream_handle)
        .context("Failed to create audio sink")?;
    
    // Load the audio file
    let file = BufReader::new(
        File::open(file_path)
            .context("Failed to open audio file for playback")?
    );
    
    // Decode the audio file
    let source = Decoder::new(file)
        .context("Failed to decode audio file")?;
    
    // Get the total duration if available
    let total_duration = source.total_duration();
    let duration_for_display = total_duration;
    
    // Append the source to the sink
    sink.append(source);
    
    println!("🎵 Now playing: {}", file_path.display());
    
    // Create a progress bar if we know the duration
    if let Some(duration) = duration_for_display {
        let pb = ProgressBar::new(duration.as_secs());
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s {msg}")
                .unwrap()
                .progress_chars("#>-")
        );
        
        // Update progress bar
        while !sink.empty() {
            let elapsed = duration.as_secs().saturating_sub(
                sink.get_pos().as_secs()
            );
            pb.set_position(elapsed);
            thread::sleep(Duration::from_millis(100));
        }
        
        pb.finish_with_message("✓ Playback complete");
    } else {
        // If no duration available, just wait for playback to finish
        println!("⏸  Playing... (Press Ctrl+C to stop)");
        sink.sleep_until_end();
    }
    
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    // Verify the file exists
    if !args.file.exists() {
        anyhow::bail!("File not found: {}", args.file.display());
    }
    
    // Verify the file has a supported extension
    let ext = args.file.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    
    if !["mp3", "flac"].contains(&ext.to_lowercase().as_str()) {
        anyhow::bail!("Unsupported file format: {}. Supported formats: MP3, FLAC", ext);
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
