//! Read/write Winamp's `.eqf` equalizer preset files.
//!
//! ## Format
//!
//! - 31-byte header: `b"Winamp EQ library file v1.1\x1a!--"`.
//! - 257-byte preset name (null-padded UTF-8).
//! - 11 bytes of EQ data: `[preamp, band_0, band_1, ..., band_9]`.
//!
//! Each byte encodes a gain in dB on a roughly 0.635 dB step:
//! `byte = round((1 - (gain + 20) / 40) * 63)` (so 0 = +20 dB, 63 = -20 dB,
//! 32 ≈ 0 dB). Anything beyond ±20 dB is clamped — Winamp's own UI ranges
//! over the same span so this isn't a lossy reduction in practice.
//!
//! Files written here always contain a single preset (Winamp itself
//! supports multi-preset libraries, but our writer doesn't need to). The
//! reader stops after the first preset and ignores any trailing data.
//!
//! Both functions error on a missing/unrecognised header. Truncated
//! payloads also error rather than silently filling with defaults — a
//! corrupt file shouldn't quietly load as "all bands at -19 dB".

use anyhow::{Context, Result, anyhow};
use std::io::{Read, Write};

/// Magic prefix written verbatim at the start of every `.eqf`. The 4 trailing
/// bytes (`\x1a!--`) form an EOF marker + version field per Winamp's loader.
const EQF_MAGIC: &[u8] = b"Winamp EQ library file v1.1\x1a!--";
/// Length of the preset-name slot. Always null-padded; trailing nulls are
/// trimmed on read.
const PRESET_NAME_LEN: usize = 257;
/// Number of EQ bytes following the name: preamp + 10 band gains.
const EQ_BYTES: usize = 11;

/// In-memory representation of a single `.eqf` preset.
#[derive(Debug, Clone, PartialEq)]
pub struct EqfPreset {
    pub name: String,
    /// dB, ±20 dB range (clamped on encode).
    pub preamp_db: f32,
    /// 10 band gains in dB, low → high frequency. Always exactly 10 entries.
    pub bands: [f32; 10],
}

/// Convert dB ∈ [-20, +20] to the byte Winamp writes. Saturates outside the
/// range — `.eqf` can't represent gains beyond ±20 dB.
fn encode_db(db: f32) -> u8 {
    let clamped = db.clamp(-20.0, 20.0);
    let v = ((1.0 - (clamped + 20.0) / 40.0) * 63.0).round() as i32;
    v.clamp(0, 63) as u8
}

/// Inverse of `encode_db`. Bytes outside 0..=63 are clipped (some buggy
/// editors stash junk in the high bits — the spec says 6-bit values).
fn decode_db(b: u8) -> f32 {
    20.0 - (b.min(63) as f32 / 63.0) * 40.0
}

/// Serialise a single preset to a writer. Always emits the magic header
/// followed by name + 11 EQ bytes — exactly the layout `read_eqf` expects.
pub fn write_eqf<W: Write>(w: &mut W, preset: &EqfPreset) -> Result<()> {
    w.write_all(EQF_MAGIC).context("writing EQF magic header")?;

    let mut name_buf = [0u8; PRESET_NAME_LEN];
    let bytes = preset.name.as_bytes();
    let n = bytes.len().min(PRESET_NAME_LEN);
    name_buf[..n].copy_from_slice(&bytes[..n]);
    w.write_all(&name_buf).context("writing preset name")?;

    let mut data = [0u8; EQ_BYTES];
    data[0] = encode_db(preset.preamp_db);
    for (i, &gain) in preset.bands.iter().enumerate() {
        data[1 + i] = encode_db(gain);
    }
    w.write_all(&data).context("writing EQ bytes")?;

    Ok(())
}

/// Read the first preset from an EQF stream. Returns the preset name
/// (null-trimmed UTF-8 lossy), the preamp gain, and the 10 band gains.
/// Trailing presets in a multi-entry library are ignored.
pub fn read_eqf<R: Read>(r: &mut R) -> Result<EqfPreset> {
    let mut header = [0u8; 31];
    r.read_exact(&mut header)
        .context("reading EQF magic header")?;
    if &header[..] != EQF_MAGIC {
        return Err(anyhow!(
            "not a Winamp .eqf file: header mismatch (got {:?})",
            &header[..28]
        ));
    }

    let mut name_buf = [0u8; PRESET_NAME_LEN];
    r.read_exact(&mut name_buf).context("reading preset name")?;
    let null = name_buf
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_buf.len());
    let name = String::from_utf8_lossy(&name_buf[..null]).into_owned();

    let mut data = [0u8; EQ_BYTES];
    r.read_exact(&mut data).context("reading EQ bytes")?;
    let preamp_db = decode_db(data[0]);
    let mut bands = [0.0f32; 10];
    for (i, slot) in bands.iter_mut().enumerate() {
        *slot = decode_db(data[1 + i]);
    }

    Ok(EqfPreset {
        name,
        preamp_db,
        bands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_zero_db() {
        let preset = EqfPreset {
            name: "Flat".to_string(),
            preamp_db: 0.0,
            bands: [0.0; 10],
        };
        let mut buf = Vec::new();
        write_eqf(&mut buf, &preset).unwrap();
        let read = read_eqf(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(read.name, "Flat");
        assert!(read.preamp_db.abs() < 0.5); // step ≈ 0.635 dB
        for &b in &read.bands {
            assert!(b.abs() < 0.5);
        }
    }

    #[test]
    fn round_trip_full_range() {
        let preset = EqfPreset {
            name: "Test".to_string(),
            preamp_db: 20.0,
            bands: [-20.0, -10.0, -5.0, -2.0, 0.0, 2.0, 5.0, 10.0, 15.0, 20.0],
        };
        let mut buf = Vec::new();
        write_eqf(&mut buf, &preset).unwrap();
        let read = read_eqf(&mut Cursor::new(&buf)).unwrap();
        // Quantization step ≈ 0.635 dB → tolerate that.
        assert!((read.preamp_db - 20.0).abs() < 0.7);
        for (got, want) in read.bands.iter().zip(preset.bands.iter()) {
            assert!((got - want).abs() < 0.7, "got {} want {}", got, want);
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bad = vec![0u8; 31];
        bad[..4].copy_from_slice(b"junk");
        let err = read_eqf(&mut Cursor::new(bad)).unwrap_err();
        assert!(err.to_string().contains("not a Winamp"));
    }

    #[test]
    fn rejects_truncated() {
        let preset = EqfPreset {
            name: "X".into(),
            preamp_db: 0.0,
            bands: [0.0; 10],
        };
        let mut buf = Vec::new();
        write_eqf(&mut buf, &preset).unwrap();
        buf.truncate(31 + 257 + 5); // drop part of the EQ bytes
        let err = read_eqf(&mut Cursor::new(buf)).unwrap_err();
        assert!(err.to_string().contains("EQ bytes"));
    }

    #[test]
    fn name_above_257_bytes_is_truncated_not_corrupt() {
        let long = "A".repeat(500);
        let preset = EqfPreset {
            name: long,
            preamp_db: 0.0,
            bands: [0.0; 10],
        };
        let mut buf = Vec::new();
        write_eqf(&mut buf, &preset).unwrap();
        let read = read_eqf(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(read.name.len(), 257);
        assert!(read.name.chars().all(|c| c == 'A'));
    }
}
