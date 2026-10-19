//! The audio configuration as one versioned document. The host decides
//! where it lives (a file, a server profile, platform preferences); this
//! module only defines it, migrates it and validates it.

use crate::dsp::{DspParams, MultichannelMode, ReplayGainMode};
use crate::equalizer::EQ_MAX_DB;
use std::collections::BTreeMap;

/// Repeat mode. Only `One` is applied by the engine itself (the media
/// is prepared again as its own successor, gaplessly); `All` and `Off`
/// are queue policy, owned by whoever owns the queue
/// ([`crate::PlayQueue`] or the host).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

/// A 10-band curve plus its preamp.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EqCurve {
    pub gains_db: [f32; 10],
    pub preamp_db: f32,
}

impl Default for EqCurve {
    fn default() -> Self {
        Self {
            gains_db: [0.0; 10],
            preamp_db: 0.0,
        }
    }
}

impl EqCurve {
    pub fn sanitized(mut self) -> Self {
        for g in &mut self.gains_db {
            *g = if g.is_finite() {
                g.clamp(-EQ_MAX_DB, EQ_MAX_DB)
            } else {
                0.0
            };
        }
        self.preamp_db = if self.preamp_db.is_finite() {
            self.preamp_db.clamp(-EQ_MAX_DB, EQ_MAX_DB)
        } else {
            0.0
        };
        self
    }

    /// Curve from a gain list of any length (missing bands are flat).
    pub fn from_slice(gains: &[f32], preamp_db: f32) -> Self {
        let mut c = Self {
            preamp_db,
            ..Default::default()
        };
        for (d, s) in c.gains_db.iter_mut().zip(gains) {
            *d = *s;
        }
        c.sanitized()
    }
}

/// Winamp's EQ AUTO: per-media presets, resolved by stable media id.
///
/// Contract: the *global* curve is the one set by hand and is what gets
/// saved. An auto preset plays while its media plays and never becomes
/// the global curve; editing the EQ while a preset plays makes the
/// edited curve global (and ends the preset for that media).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EqAutoSettings {
    pub enabled: bool,
    /// Keyed by [`crate::MediaId`] string.
    pub presets: BTreeMap<String, EqCurve>,
}

impl EqAutoSettings {
    /// The preset for `media_id`, when AUTO is on and one exists.
    pub fn resolve(&self, media_id: &str) -> Option<EqCurve> {
        self.enabled
            .then(|| self.presets.get(media_id).copied())
            .flatten()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CrossfadeSettings {
    pub enabled: bool,
    /// Fade length, 0.5..=30 s.
    pub seconds: f32,
}

impl Default for CrossfadeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            seconds: 3.0,
        }
    }
}

/// Everything the user can set about audio, as one document.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// Format version; see [`AudioSettings::VERSION`].
    pub version: u32,
    /// 0..=[`crate::MAX_VOLUME`].
    pub volume: f32,
    pub muted: bool,
    pub balance: f32,
    pub eq_enabled: bool,
    /// The global (hand-set) curve.
    pub eq: EqCurve,
    pub eq_auto: EqAutoSettings,
    /// Kept separately from the mode so turning ReplayGain off and on
    /// restores the chosen mode.
    pub replaygain_enabled: bool,
    pub replaygain_mode: ReplayGainMode,
    pub replaygain_prevent_clipping: bool,
    pub loudness: bool,
    pub mono: bool,
    pub multichannel: MultichannelMode,
    pub crossfade: CrossfadeSettings,
    pub repeat: RepeatMode,
    /// Output device name; `None` = default.
    pub output_device: Option<String>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            volume: 1.0,
            muted: false,
            balance: 0.0,
            eq_enabled: false,
            eq: EqCurve::default(),
            eq_auto: EqAutoSettings::default(),
            replaygain_enabled: false,
            replaygain_mode: ReplayGainMode::Track,
            replaygain_prevent_clipping: false,
            loudness: false,
            mono: false,
            multichannel: MultichannelMode::FoldToStereo,
            crossfade: CrossfadeSettings::default(),
            repeat: RepeatMode::Off,
            output_device: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsError {
    Parse(String),
    /// Written by a newer version of the library.
    UnsupportedVersion(u32),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Parse(e) => write!(f, "invalid audio settings: {e}"),
            SettingsError::UnsupportedVersion(v) => write!(
                f,
                "audio settings version {v} is newer than supported ({})",
                AudioSettings::VERSION
            ),
        }
    }
}

impl std::error::Error for SettingsError {}

impl AudioSettings {
    pub const VERSION: u32 = 1;

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings serialise")
    }

    /// Parse, migrate and sanitize. Unknown fields are ignored and
    /// missing ones take their defaults, so older documents import.
    pub fn from_json(json: &str) -> Result<Self, SettingsError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| SettingsError::Parse(e.to_string()))?;
        let version = value
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(Self::VERSION as u64) as u32;
        if version > Self::VERSION {
            return Err(SettingsError::UnsupportedVersion(version));
        }
        // Version 1 is the first; later versions migrate `value` here.
        let mut s: Self =
            serde_json::from_value(value).map_err(|e| SettingsError::Parse(e.to_string()))?;
        s.version = Self::VERSION;
        Ok(s.sanitized())
    }

    pub fn sanitized(mut self) -> Self {
        let f = |v: f32, d: f32| if v.is_finite() { v } else { d };
        self.volume = f(self.volume, 1.0).clamp(0.0, crate::dsp::MAX_VOLUME);
        self.balance = f(self.balance, 0.0).clamp(-1.0, 1.0);
        self.eq = self.eq.sanitized();
        for c in self.eq_auto.presets.values_mut() {
            *c = c.sanitized();
        }
        self.crossfade.seconds = f(self.crossfade.seconds, 3.0).clamp(0.5, 30.0);
        self
    }

    /// The effective ReplayGain mode.
    pub fn replaygain(&self) -> ReplayGainMode {
        if self.replaygain_enabled {
            self.replaygain_mode
        } else {
            ReplayGainMode::Off
        }
    }

    /// Chain parameters for a media, applying `curve` (an auto preset or
    /// the global curve). Volume follows the player's split: the part
    /// above unity in the chain, the rest left to the output.
    pub fn dsp_params(&self, curve: &EqCurve) -> DspParams {
        let audible = if self.muted { 0.0 } else { self.volume };
        DspParams {
            eq_enabled: self.eq_enabled,
            eq_gains_db: curve.gains_db,
            preamp_db: curve.preamp_db,
            replaygain: self.replaygain(),
            replaygain_prevent_clipping: self.replaygain_prevent_clipping,
            volume: if self.muted {
                1.0
            } else {
                self.volume.max(1.0)
            },
            balance: self.balance,
            mono: self.mono,
            loudness: self.loudness,
            loudness_volume: audible.min(1.0),
            multichannel: self.multichannel,
        }
    }

    /// Attenuation left to the output (0..=1).
    pub fn device_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume.min(1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_sanitizes() {
        let mut s = AudioSettings {
            volume: 1.2,
            eq_enabled: true,
            ..Default::default()
        };
        s.eq.gains_db[3] = 4.5;
        s.eq_auto.enabled = true;
        s.eq_auto
            .presets
            .insert("item-1".into(), EqCurve::from_slice(&[1.0; 10], -2.0));
        let back = AudioSettings::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);

        let wild = r#"{"version":1,"volume":99,"balance":-5,"eq":{"gains_db":[50,0,0,0,0,0,0,0,0,0],"preamp_db":0},"future_field":true}"#;
        let s = AudioSettings::from_json(wild).unwrap();
        assert_eq!(s.volume, crate::dsp::MAX_VOLUME);
        assert_eq!(s.balance, -1.0);
        assert_eq!(s.eq.gains_db[0], EQ_MAX_DB);
    }

    #[test]
    fn rejects_newer_versions_and_imports_older_partial_documents() {
        assert_eq!(
            AudioSettings::from_json(r#"{"version":7}"#),
            Err(SettingsError::UnsupportedVersion(7))
        );
        let s = AudioSettings::from_json(r#"{"mono":true}"#).unwrap();
        assert!(s.mono);
        assert_eq!(s.version, AudioSettings::VERSION);
    }

    #[test]
    fn eq_auto_resolves_only_when_enabled() {
        let mut a = EqAutoSettings::default();
        a.presets
            .insert("x".into(), EqCurve::from_slice(&[3.0], 0.0));
        assert_eq!(a.resolve("x"), None);
        a.enabled = true;
        assert_eq!(a.resolve("x").unwrap().gains_db[0], 3.0);
        assert_eq!(a.resolve("y"), None);
    }

    #[test]
    fn volume_split_and_replaygain_toggle() {
        let s = AudioSettings {
            volume: 1.4,
            replaygain_enabled: false,
            replaygain_mode: ReplayGainMode::Album,
            ..Default::default()
        };
        let p = s.dsp_params(&s.eq);
        assert_eq!(p.volume, 1.4);
        assert_eq!(s.device_volume(), 1.0);
        assert_eq!(p.replaygain, ReplayGainMode::Off);
        let on = AudioSettings {
            replaygain_enabled: true,
            ..s
        };
        assert_eq!(on.replaygain(), ReplayGainMode::Album);
    }
}
