//! Gapless claims, per format: a decoded file must contain exactly the
//! samples that were encoded — encoder delay and padding removed — or a
//! gapless join cannot be sample-accurate. The fixtures are ffmpeg
//! encodes of 1.0 s (44 100 frames) and 0.7 s (30 870 frames) tones.

use oneamp_core::symphonia_player::SymphoniaPlayer;
use std::path::Path;

fn frames(name: &str) -> (u64, u64) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let mut p = SymphoniaPlayer::load(&path).unwrap();
    let mut n = 0u64;
    let mut first = None;
    while let Some(b) = p.decode_next().unwrap() {
        first.get_or_insert(b.start_frame);
        n += b.frames() as u64;
    }
    (first.unwrap_or(0), n)
}

#[test]
fn lossless_and_lossy_formats_decode_to_their_exact_length() {
    for (name, expected) in [
        ("a.flac", 44_100),
        ("a.mp3", 44_100),
        ("b.mp3", 30_870),
        ("a.ogg", 44_100),
    ] {
        let (start, n) = frames(name);
        println!("{name}: first frame {start}, {n} frames");
        assert_eq!(start, 0, "{name} starts late");
        assert_eq!(n, expected, "{name} decodes to the wrong length");
    }
}

/// Not claimed: symphonia's MP4 reader ignores the edit list (`elst`)
/// that carries AAC's encoder delay, so AAC in MP4 decodes with its
/// priming and padding frames. This test pins the current behaviour so
/// the capability matrix stays honest; it will fail (good news) the day
/// the demuxer trims them.
#[test]
fn aac_in_mp4_is_not_gapless_yet() {
    let (_, n) = frames("a.m4a");
    assert_ne!(n, 44_100);
    assert_eq!(n % 1024, 0, "whole AAC frames, untrimmed");
}
