//! Equalizer presets module
//!
//! This module provides predefined equalizer presets for various music genres
//! and listening scenarios, as well as custom preset management.

use crate::equalizer::Equalizer;
use anyhow::{Context, Result};
#[cfg(feature = "serialization")]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Compute the recommended preamp (dB, typically <= 0) for a set of
/// band gains. Wraps [`Equalizer::headroom_db`] with the 10-band
/// padding the rest of the module uses for preset storage.
///
/// Used by [`BuiltinPresets`] so aggressive presets (Bass Boost,
/// Hip-Hop, Club) ship with a negative preamp baked in — applying the
/// preset alone fully sets up the filter chain *and* the pre-buffer
/// gain so a normalized track doesn't constantly trip the brickwall
/// limiter.
fn preamp_db_for_gains(gains: &[f32]) -> f32 {
    let mut eq = Equalizer::new(44100.0);
    let mut padded = [0.0_f32; 10];
    for (i, slot) in padded.iter_mut().enumerate() {
        if let Some(&g) = gains.get(i) {
            *slot = g;
        }
    }
    eq.set_all_gains(&padded);
    eq.headroom_db()
}

/// Re-export of the canonical 10-band EQ frequency series. The actual
/// values live in [`crate::equalizer::EQ_FREQUENCIES`] (ISO 266 octave
/// series) so the filter centers and the preset labels stay in sync
/// — v1 had two divergent arrays here and in `equalizer.rs`.
pub use crate::equalizer::EQ_FREQUENCIES;

/// Equalizer preset with name and gain values.
///
/// Carries a recommended `preamp_db` so aggressive presets can pre-set
/// the master pre-amp on apply — built-in presets compute it via
/// [`Equalizer::headroom_db`] so the cascaded EQ never clips a
/// normalized signal; user / `.eqf` presets can persist a hand-picked
/// value.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct EqualizerPreset {
    /// Name of the preset
    pub name: String,
    /// Gain values for each band (in dB, typically -12 to +12)
    pub gains: Vec<f32>,
    /// Optional description
    pub description: Option<String>,
    /// Master pre-amp gain (dB) the user should switch to when this
    /// preset is applied. Always ≤ 0 for built-in presets — the
    /// computed headroom needed for the cascaded EQ not to clip.
    /// Defaults to 0.0 on deserialization for backward compatibility
    /// with user preset files saved before the field existed.
    #[cfg_attr(feature = "serialization", serde(default))]
    pub preamp_db: f32,
}

impl EqualizerPreset {
    /// Create a new preset (preamp defaults to 0 dB — caller can fill
    /// it in afterwards if needed).
    pub fn new(name: String, gains: Vec<f32>) -> Self {
        Self {
            name,
            gains,
            description: None,
            preamp_db: 0.0,
        }
    }

    /// Create a preset with description (preamp defaults to 0 dB).
    pub fn with_description(name: String, gains: Vec<f32>, description: String) -> Self {
        Self {
            name,
            gains,
            description: Some(description),
            preamp_db: 0.0,
        }
    }

    /// Validate that the preset has the correct number of finite bands.
    pub fn is_valid(&self) -> bool {
        self.gains.len() == 10
            && self.gains.iter().all(|gain| gain.is_finite())
            && self.preamp_db.is_finite()
    }
}

/// Built-in equalizer presets
pub struct BuiltinPresets;

impl BuiltinPresets {
    /// Internal constructor: auto-derives `preamp_db` from the gain
    /// curve so applying a built-in preset is always safe with the
    /// new pre-buffer brickwall limiter (O13). Heavy presets (Bass
    /// Boost, Club, Metal) end up with a `preamp_db` around −7..−9 dB.
    fn build(name: &str, gains: Vec<f32>, description: &str) -> EqualizerPreset {
        let preamp_db = preamp_db_for_gains(&gains);
        EqualizerPreset {
            name: name.to_string(),
            gains,
            description: Some(description.to_string()),
            preamp_db,
        }
    }

    /// Get all built-in presets
    pub fn all() -> Vec<EqualizerPreset> {
        vec![
            Self::flat(),
            Self::rock(),
            Self::pop(),
            Self::jazz(),
            Self::classical(),
            Self::electronic(),
            Self::hip_hop(),
            Self::metal(),
            Self::acoustic(),
            Self::bass_boost(),
            Self::treble_boost(),
            Self::vocal_boost(),
            Self::laptop_speakers(),
            Self::headphones(),
            Self::large_hall(),
            Self::club(),
        ]
    }

    // The v2 preset curves are reasoned against the actual ISO 266
    // band centers used by the EQ (31.5, 63, 125, 250, 500, 1k, 2k,
    // 4k, 8k, 16k Hz). Band roles assumed when picking gains:
    //   0 31.5  — sub-bass (kick fundamental, room rumble)
    //   1 63    — bass (kick punch, low bass guitar fundamentals)
    //   2 125   — low-mid bass (warmth, body)
    //   3 250   — boom / mud zone, lower vocal fundamentals
    //   4 500   — mid (snare body, lower vocal harmonics)
    //   5 1k    — mid (vocal core, "honkiness")
    //   6 2k    — upper-mid (vocal presence, "edge")
    //   7 4k    — presence (articulation, snare crack)
    //   8 8k    — brilliance (sibilance, cymbal shimmer)
    //   9 16k   — air (top-end sheen)
    // Per-preset auto-preamp keeps the cascaded peak under 0 dBFS, so
    // none of these need a hand-tuned preamp — `BuiltinPresets::build`
    // computes it via `Equalizer::headroom_db`.

    /// Flat response (all bands at 0 dB).
    pub fn flat() -> EqualizerPreset {
        Self::build(
            "Flat",
            vec![0.0; 10],
            "No equalization — pure pass-through.",
        )
    }

    /// Rock: punch + presence, no mid scoop. Sits between Pop and Metal.
    pub fn rock() -> EqualizerPreset {
        Self::build(
            "Rock",
            vec![4.0, 3.0, 1.0, -1.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0],
            "Kick punch and cymbal shimmer; vocals stay natural.",
        )
    }

    /// Pop: vocal-forward shape. Slight bass body, mids lifted around
    /// the vocal core, gentle air on top.
    pub fn pop() -> EqualizerPreset {
        Self::build(
            "Pop",
            vec![2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 2.0, 1.0],
            "Vocal-forward with controlled bass and gentle air.",
        )
    }

    /// Jazz: warm low-mids for upright bass and brass body, soft top.
    pub fn jazz() -> EqualizerPreset {
        Self::build(
            "Jazz",
            vec![3.0, 2.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 2.0],
            "Warm fundamentals for upright bass and brass; cymbals breathe.",
        )
    }

    /// Classical: minimal coloration. Orchestral mixes are usually
    /// well-balanced from the source — a subtle low + air lift adds
    /// hall feel without touching the mids.
    pub fn classical() -> EqualizerPreset {
        Self::build(
            "Classical",
            vec![2.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0],
            "Subtle lift on lows + highs; midrange untouched.",
        )
    }

    /// Electronic: wide V-curve for synth-heavy mixes. Sub-bass impact
    /// + crisp leads, mids gently scooped.
    pub fn electronic() -> EqualizerPreset {
        Self::build(
            "Electronic",
            vec![6.0, 5.0, 3.0, 1.0, -1.0, 0.0, 2.0, 4.0, 5.0, 5.0],
            "Wide V-curve for sub-bass impact and crisp synth leads.",
        )
    }

    /// Hip-Hop: sub-bass weight plus vocal presence, sibilance kept
    /// under control (the 8/16 kHz dip).
    pub fn hip_hop() -> EqualizerPreset {
        Self::build(
            "Hip-Hop",
            vec![7.0, 6.0, 4.0, 1.0, 0.0, -1.0, 1.0, 3.0, 2.0, 1.0],
            "Sub-bass weight + vocal presence; tames sibilance.",
        )
    }

    /// Metal: classic V-curve mid scoop. Separates distorted guitars
    /// from drums by carving the 400-800 Hz "mud" zone.
    pub fn metal() -> EqualizerPreset {
        Self::build(
            "Metal",
            vec![5.0, 5.0, 3.0, 0.0, -3.0, -2.0, 1.0, 3.0, 5.0, 4.0],
            "Mid-scoop V; separates distorted guitars from drums.",
        )
    }

    /// Acoustic: gentle warmth + air, no scoop. Honors a relatively
    /// natural source (singer-songwriter, classical guitar, etc.).
    pub fn acoustic() -> EqualizerPreset {
        Self::build(
            "Acoustic",
            vec![3.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 2.0],
            "Gentle warmth + air for steel-string guitar and woodwinds.",
        )
    }

    /// Bass Boost: low-shelf style emphasis with everything above
    /// 250 Hz left untouched — no muddying of vocals or guitars.
    pub fn bass_boost() -> EqualizerPreset {
        Self::build(
            "Bass Boost",
            vec![8.0, 7.0, 5.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "Low-shelf boost for bass-heavy genres or weak speakers.",
        )
    }

    /// Treble Boost: high-shelf style emphasis above 1 kHz. Useful on
    /// muffled sources or for users who prefer a bright tilt.
    pub fn treble_boost() -> EqualizerPreset {
        Self::build(
            "Treble Boost",
            vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 7.0, 7.0],
            "High-shelf lift; brightens muffled sources.",
        )
    }

    /// Vocal Boost: the v2 fix. v1 boosted 250 Hz, which adds *mud* to
    /// vocals, not intelligibility. Speech / vocal energy lives in the
    /// 1-4 kHz octave; a mild cut around 63-125 Hz keeps the boost
    /// from competing with bass instruments below it.
    pub fn vocal_boost() -> EqualizerPreset {
        Self::build(
            "Vocal Boost",
            vec![-1.0, -1.0, 0.0, 1.0, 2.0, 4.0, 5.0, 4.0, 1.0, 0.0],
            "Centers the 1-4 kHz vocal range; trims lower-mid mud.",
        )
    }

    /// Laptop Speakers: restores the missing low end AND tames the
    /// typical 1-3 kHz "honkiness" of small drivers in plastic
    /// enclosures.
    pub fn laptop_speakers() -> EqualizerPreset {
        Self::build(
            "Laptop Speakers",
            vec![6.0, 6.0, 4.0, 2.0, 0.0, -1.0, -1.0, 1.0, 3.0, 3.0],
            "Restores missing bass; tames laptop hi-mid honkiness.",
        )
    }

    /// Headphones: a mild Harman-target-style tilt — gentle low-shelf
    /// for the missing room bass, gentle top-end lift for the
    /// brightness most "neutral" cans miss. No mid coloration.
    pub fn headphones() -> EqualizerPreset {
        Self::build(
            "Headphones",
            vec![3.0, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 2.0],
            "Mild Harman-target tilt for neutral headphones.",
        )
    }

    /// Large Hall: extended lows for room sense, gentle mid carve for
    /// separation, soft air for shimmer.
    pub fn large_hall() -> EqualizerPreset {
        Self::build(
            "Large Hall",
            vec![5.0, 4.0, 2.0, 1.0, 0.0, 0.0, -1.0, 0.0, 2.0, 3.0],
            "Extended lows + soft air for concert-hall size.",
        )
    }

    /// Club: punchy lows for dance music, bright vocals on top, with
    /// the mid honkiness kept neutral.
    pub fn club() -> EqualizerPreset {
        Self::build(
            "Club",
            vec![6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 2.0, 4.0, 5.0, 4.0],
            "Punchy lows for dance music with bright vocals.",
        )
    }

    /// Get preset by name
    pub fn get_by_name(name: &str) -> Option<EqualizerPreset> {
        Self::all().into_iter().find(|p| p.name == name)
    }
}

/// Custom preset manager
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct PresetManager {
    /// User-defined custom presets
    custom_presets: HashMap<String, EqualizerPreset>,
}

impl PresetManager {
    /// Create a new preset manager
    pub fn new() -> Self {
        Self {
            custom_presets: HashMap::new(),
        }
    }

    /// Add a custom preset
    pub fn add_preset(&mut self, preset: EqualizerPreset) -> Result<()> {
        if !preset.is_valid() {
            anyhow::bail!("Invalid preset: must have exactly 10 bands");
        }
        self.custom_presets.insert(preset.name.clone(), preset);
        Ok(())
    }

    /// Remove a custom preset
    pub fn remove_preset(&mut self, name: &str) -> Option<EqualizerPreset> {
        self.custom_presets.remove(name)
    }

    /// Get a custom preset by name
    pub fn get_preset(&self, name: &str) -> Option<&EqualizerPreset> {
        self.custom_presets.get(name)
    }

    /// Get all custom presets
    pub fn custom_presets(&self) -> Vec<&EqualizerPreset> {
        self.custom_presets.values().collect()
    }

    /// Get all presets (built-in + custom)
    pub fn all_presets(&self) -> Vec<EqualizerPreset> {
        let mut presets = BuiltinPresets::all();
        presets.extend(self.custom_presets.values().cloned());
        presets
    }

    /// Check if a preset name exists (built-in or custom)
    pub fn preset_exists(&self, name: &str) -> bool {
        BuiltinPresets::get_by_name(name).is_some() || self.custom_presets.contains_key(name)
    }

    /// Save custom presets to JSON file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        #[cfg(feature = "serialization")]
        {
            let json =
                serde_json::to_string_pretty(self).context("Failed to serialize preset manager")?;
            fs::write(path, json).context("Failed to write preset file")?;
            Ok(())
        }
        #[cfg(not(feature = "serialization"))]
        {
            anyhow::bail!("Cannot save presets: serialization feature not enabled")
        }
    }

    /// Load custom presets from JSON file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let json = fs::read_to_string(path).context("Failed to read preset file")?;
        #[cfg(feature = "serialization")]
        {
            let manager =
                serde_json::from_str(&json).context("Failed to deserialize preset manager")?;
            Ok(manager)
        }
        #[cfg(not(feature = "serialization"))]
        {
            anyhow::bail!("Cannot load presets: serialization feature not enabled")
        }
    }

    /// Load or create new if file doesn't exist
    pub fn load_or_new<P: AsRef<Path>>(path: P) -> Self {
        Self::load(path).unwrap_or_else(|_| Self::new())
    }

    /// Clear all custom presets
    pub fn clear(&mut self) {
        self.custom_presets.clear();
    }

    /// Get number of custom presets
    pub fn custom_count(&self) -> usize {
        self.custom_presets.len()
    }

    /// Get total number of presets (built-in + custom)
    pub fn total_count(&self) -> usize {
        BuiltinPresets::all().len() + self.custom_presets.len()
    }
}

impl Default for PresetManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_presets() {
        let presets = BuiltinPresets::all();
        assert!(
            presets.len() >= 10,
            "Should have at least 10 built-in presets"
        );

        // All presets should be valid
        for preset in &presets {
            assert!(preset.is_valid(), "Preset {} should be valid", preset.name);
        }
    }

    #[test]
    fn test_preset_validation() {
        let valid = EqualizerPreset::new("Test".to_string(), vec![0.0; 10]);
        assert!(valid.is_valid());

        let invalid = EqualizerPreset::new("Test".to_string(), vec![0.0; 5]);
        assert!(!invalid.is_valid());
    }

    /// Position (in EQ_FREQUENCIES) of the band with the highest gain.
    fn peak_band(preset: &EqualizerPreset) -> usize {
        preset
            .gains
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    #[test]
    fn vocal_boost_peaks_in_voice_band_not_lower_mid() {
        // The whole point of the v2 Vocal Boost is to put the lift
        // where vocal intelligibility lives (1-4 kHz, i.e. band 5/6/7),
        // not at 250 Hz (band 3) which was the v1 layout and which
        // adds muddiness instead of presence. This test would catch a
        // regression where someone "fixes" Vocal Boost by moving the
        // peak back into the lower mids.
        let preset = BuiltinPresets::vocal_boost();
        let peak = peak_band(&preset);
        assert!(
            (5..=7).contains(&peak),
            "Vocal Boost peak should be in 1-4 kHz (band 5..=7), got band {} ({})",
            peak,
            EQ_FREQUENCIES[peak]
        );
        // Lower-mid mud zone (250-500 Hz) must NOT be the loudest band.
        for i in [3, 4] {
            assert!(
                preset.gains[i] <= preset.gains[peak] - 1.0,
                "Vocal Boost shouldn't peak at {} Hz; that adds mud, not presence",
                EQ_FREQUENCIES[i]
            );
        }
    }

    #[test]
    fn bass_boost_only_touches_low_end() {
        // v2 Bass Boost is a clean low-shelf — everything above 250 Hz
        // (band 3) should be exactly 0, so the preset doesn't muddy
        // the vocal or guitar range when stacked with anything else.
        let preset = BuiltinPresets::bass_boost();
        for (i, (gain, freq)) in preset
            .gains
            .iter()
            .zip(EQ_FREQUENCIES.iter())
            .enumerate()
            .skip(4)
        {
            assert_eq!(
                *gain, 0.0,
                "Bass Boost should not affect band {} ({} Hz)",
                i, freq
            );
        }
    }

    #[test]
    fn treble_boost_only_touches_high_end() {
        // Mirror of `bass_boost_only_touches_low_end` — bands below
        // 1 kHz (band 5) must be 0 dB.
        let preset = BuiltinPresets::treble_boost();
        for (i, (gain, freq)) in preset
            .gains
            .iter()
            .zip(EQ_FREQUENCIES.iter())
            .enumerate()
            .take(5)
        {
            assert_eq!(
                *gain, 0.0,
                "Treble Boost should not affect band {} ({} Hz)",
                i, freq
            );
        }
    }

    #[test]
    fn flat_preset_has_zero_preamp() {
        let flat = BuiltinPresets::flat();
        // Flat → no boost anywhere → no headroom needed.
        assert_eq!(flat.preamp_db, 0.0);
    }

    #[test]
    fn boost_presets_get_negative_preamp() {
        // Auto-computed preamp must compensate for the worst-case
        // cascaded peak. Heavy-boost presets MUST end up with a
        // negative preamp; a non-negative value here would mean the
        // headroom utility didn't see the boost, which would defeat
        // the whole point of O14.
        for name in ["Bass Boost", "Hip-Hop", "Metal", "Club", "Treble Boost"] {
            let preset = BuiltinPresets::get_by_name(name).expect(name);
            assert!(
                preset.preamp_db < -3.0,
                "{}: preamp should be <= -3 dB (was {})",
                name,
                preset.preamp_db
            );
        }
    }

    #[test]
    fn user_constructed_preset_defaults_to_zero_preamp() {
        // The hand constructors (used by tests and by external callers
        // who haven't computed headroom themselves) leave preamp_db at
        // 0.0 — opt-in, not surprising.
        let p = EqualizerPreset::new("Plain".to_string(), vec![6.0; 10]);
        assert_eq!(p.preamp_db, 0.0);
        let p = EqualizerPreset::with_description(
            "Plain".to_string(),
            vec![6.0; 10],
            "desc".to_string(),
        );
        assert_eq!(p.preamp_db, 0.0);
    }

    #[test]
    fn test_get_by_name() {
        let rock = BuiltinPresets::get_by_name("Rock");
        assert!(rock.is_some());
        assert_eq!(rock.unwrap().name, "Rock");

        let nonexistent = BuiltinPresets::get_by_name("Nonexistent");
        assert!(nonexistent.is_none());
    }

    #[test]
    fn test_preset_manager() {
        let mut manager = PresetManager::new();

        let custom = EqualizerPreset::new("My Preset".to_string(), vec![1.0; 10]);
        manager.add_preset(custom.clone()).unwrap();

        assert_eq!(manager.custom_count(), 1);
        assert!(manager.preset_exists("My Preset"));

        let retrieved = manager.get_preset("My Preset");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "My Preset");
    }

    #[test]
    fn test_remove_preset() {
        let mut manager = PresetManager::new();

        let custom = EqualizerPreset::new("Test".to_string(), vec![0.0; 10]);
        manager.add_preset(custom).unwrap();

        assert_eq!(manager.custom_count(), 1);

        let removed = manager.remove_preset("Test");
        assert!(removed.is_some());
        assert_eq!(manager.custom_count(), 0);
    }

    #[test]
    fn test_all_presets() {
        let mut manager = PresetManager::new();

        let custom = EqualizerPreset::new("Custom".to_string(), vec![0.0; 10]);
        manager.add_preset(custom).unwrap();

        let all = manager.all_presets();
        let builtin_count = BuiltinPresets::all().len();

        assert_eq!(all.len(), builtin_count + 1);
    }

    #[test]
    fn test_invalid_preset() {
        let mut manager = PresetManager::new();

        let invalid = EqualizerPreset::new("Invalid".to_string(), vec![0.0; 5]);
        let result = manager.add_preset(invalid);

        assert!(result.is_err());
    }

    #[test]
    fn test_non_finite_preset_is_invalid() {
        let mut manager = PresetManager::new();

        let invalid_gain = EqualizerPreset::new("Invalid Gain".to_string(), vec![f32::NAN; 10]);
        assert!(manager.add_preset(invalid_gain).is_err());

        let mut invalid_preamp = EqualizerPreset::new("Invalid Preamp".to_string(), vec![0.0; 10]);
        invalid_preamp.preamp_db = f32::INFINITY;
        assert!(manager.add_preset(invalid_preamp).is_err());
    }

    #[test]
    fn test_eq_frequencies() {
        // ISO 266 octave series. The series doubles each step (modulo
        // the standard 31.5 → 63 rounding), so a sanity check on the
        // endpoints plus the doubling cadence covers any drift between
        // the equalizer module and this presets module.
        assert_eq!(EQ_FREQUENCIES.len(), 10);
        assert_eq!(EQ_FREQUENCIES[0], 31.5);
        assert_eq!(EQ_FREQUENCIES[9], 16000.0);
        for w in EQ_FREQUENCIES.windows(2) {
            let ratio = w[1] / w[0];
            assert!(
                (ratio - 2.0).abs() < 0.06,
                "expected ~1 octave between {} and {}, got ratio {}",
                w[0],
                w[1],
                ratio
            );
        }
    }
}
