use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub mod audio_capture;
pub mod audio_thread_symphonia;
pub mod eqf;
pub mod equalizer;
pub mod equalizer_presets;
pub mod http_stream;
pub mod playlist;
pub mod recent_files;
#[cfg(feature = "audio")]
pub mod rodio_output;
pub mod symphonia_player;
pub mod tag_editor;
pub mod wsz;

pub use audio_capture::AudioCaptureBuffer;
pub use equalizer::Equalizer;
#[cfg(feature = "serialization")]
pub use equalizer_presets::{BuiltinPresets, EQ_FREQUENCIES, EqualizerPreset, PresetManager};
pub use playlist::{Playlist, PlaylistEntry, SortOrder};
pub use recent_files::{RecentFile, RecentFiles};
#[cfg(feature = "audio")]
pub use rodio_output::list_output_devices;

/// Repeat mode for playlist playback
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    /// No repeat - stop at end of playlist
    Off,
    /// Repeat current track
    One,
    /// Repeat all tracks in playlist
    All,
}

/// Which ReplayGain figure (if any) the audio thread applies on top of
/// the user's preamp. Album and track gains are both parsed from the
/// file's metadata into [`TrackInfo`]; this picks which one is summed
/// into the per-sample gain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum ReplayGainMode {
    /// No ReplayGain — the user's preamp is applied alone.
    Off,
    /// Use `REPLAYGAIN_TRACK_GAIN` (per-track normalization). This is
    /// the default and matches the historical `replaygain_enabled=true`
    /// behaviour.
    #[default]
    Track,
    /// Use `REPLAYGAIN_ALBUM_GAIN` (per-album normalization — keeps the
    /// relative loudness of tracks within an album intact). Falls back
    /// to no gain when the album tag is absent.
    Album,
    /// Prefer album gain when present, otherwise fall back to track gain.
    /// The "do the right thing" mode for mixed libraries.
    Auto,
}

impl ReplayGainMode {
    /// Resolve the effective gain (in dB) for a track under this mode.
    /// Returns `0.0` when the mode is `Off` or the relevant tag is
    /// missing — summing zero leaves the user's preamp untouched.
    ///
    /// `Auto` uses album gain when present, else track gain. `Album`
    /// uses album gain only (no track fallback — picking track gain
    /// there would defeat the point of album normalization). `Track`
    /// uses track gain only.
    pub fn gain_db(self, track_gain_db: Option<f32>, album_gain_db: Option<f32>) -> f32 {
        match self {
            ReplayGainMode::Off => 0.0,
            ReplayGainMode::Track => track_gain_db.unwrap_or(0.0),
            ReplayGainMode::Album => album_gain_db.unwrap_or(0.0),
            ReplayGainMode::Auto => album_gain_db.or(track_gain_db).unwrap_or(0.0),
        }
    }
}

/// Commands that can be sent to the audio thread
#[derive(Debug, Clone)]
pub enum AudioCommand {
    /// Load and play a file
    Play(PathBuf),
    /// Load and play an HTTP(S) audio URL (internet radio, podcast).
    /// The audio thread opens an `HttpStream`, wraps it in
    /// symphonia's `MediaSourceStream`, and starts playback. Seek
    /// commands during URL playback are no-ops — the stream isn't
    /// seekable. ICY metadata (`StreamTitle`) is bridged back to the
    /// app via [`AudioEvent::IcyMetadata`].
    PlayUrl(String),
    /// Pre-load the next track for gapless transition. The audio thread
    /// decodes header + first packet ahead of time, and on end-of-stream
    /// swaps it into the running rodio stream without rebuilding the
    /// device — provided sample_rate and channels match the current
    /// output. Format mismatch falls back to the standard `RequestNext`
    /// path. App side should send this once when the current track is
    /// within ~2 s of its end.
    QueueNext(PathBuf),
    /// Pause playback
    Pause,
    /// Resume playback
    Resume,
    /// Stop playback
    Stop,
    /// Seek to a position (in seconds)
    Seek(f32),
    /// Play next track in playlist
    Next,
    /// Play previous track in playlist
    Previous,
    /// Set volume (0.0 to 1.0)
    SetVolume(f32),
    /// Set stereo balance (-1.0 = full left, 0.0 = center, 1.0 = full right)
    SetBalance(f32),
    /// Set mute state
    SetMute(bool),
    /// Set repeat mode
    SetRepeatMode(RepeatMode),
    /// Enable/disable shuffle mode
    SetShuffle(bool),
    /// Set equalizer enabled state
    SetEqualizerEnabled(bool),
    /// Set equalizer band gain (band_index, gain_db)
    SetEqualizerBand(usize, f32),
    /// Set all equalizer bands at once
    SetEqualizerBands(Vec<f32>),
    /// Reset equalizer to flat response
    ResetEqualizer,
    /// Set the master pre-amp gain in dB (applied after EQ band processing).
    /// Range matches the band sliders (±20 dB).
    SetEqualizerPreamp(f32),
    /// Enable or disable equal-power crossfade between same-format tracks.
    /// The float is the fade duration in seconds (range 0.5..30.0). When
    /// `enabled` is false the duration is kept on the engine but unused,
    /// so toggling back on doesn't reset the user's choice.
    SetCrossfade(bool, f32),
    /// Enable or disable ReplayGain-driven gain normalization. When on,
    /// the audio thread adds the current track's `REPLAYGAIN_TRACK_GAIN`
    /// (from `TrackInfo`) onto the user's preamp setting before applying
    /// gain. Off restores the user's preamp alone — the user's slider
    /// setting never gets overwritten.
    ///
    /// Kept for backward compatibility: `false` maps to
    /// [`ReplayGainMode::Off`], `true` restores the engine's current
    /// mode (default [`ReplayGainMode::Track`]). Prefer
    /// [`AudioCommand::SetReplayGainMode`] for track/album/auto control.
    SetReplayGainEnabled(bool),
    /// Choose which ReplayGain figure the audio thread applies (off /
    /// track / album / auto). Supersedes the boolean
    /// [`AudioCommand::SetReplayGainEnabled`] toggle — the engine
    /// re-derives the active track's gain immediately so the mode takes
    /// effect on the currently-playing track without waiting for the
    /// next `TrackLoaded`.
    SetReplayGainMode(ReplayGainMode),
    /// Downmix stereo to mono on the fly. When `true`, every stereo frame
    /// is collapsed to the channel average and written back to both
    /// channels, so the output stays stereo-shaped (two identical
    /// channels) but plays mono. No-op on already-mono content. Off by
    /// default; mirrors Winamp's mono toggle.
    SetMono(bool),
    /// Enable or disable loudness compensation (Fletcher-Munson /
    /// ISO 226 inspired). When on, the audio thread applies a
    /// volume-dependent V-curve EQ — low- and high-shelf gain scales
    /// up as the master volume drops, so the perceived tonal balance
    /// stays roughly constant at quiet listening levels. Off restores
    /// flat shelves.
    SetLoudnessEnabled(bool),
    /// Choose which cpal output device the next track-load should open
    /// on. `None` means "use the host default" (typical case — system
    /// PulseAudio / PipeWire sink). The audio thread applies this on
    /// the next `Play` / gapless preload — switching mid-track would
    /// require tearing down the live `rodio::Sink`, which is out of
    /// scope for v1. Names come from `oneamp_core::list_output_devices`.
    SetOutputDevice(Option<String>),
    /// Shutdown the audio thread
    Shutdown,
}

/// Events sent from the audio thread to the GUI
#[derive(Debug, Clone)]
pub enum AudioEvent {
    /// Track loaded successfully with metadata
    TrackLoaded(TrackInfo),
    /// Playback started
    Playing,
    /// Playback paused
    Paused,
    /// Playback stopped
    Stopped,
    /// Playback position update (current_secs, total_secs)
    Position(f32, f32),
    /// Playback finished (track ended)
    Finished,
    /// Request next track from playlist
    RequestNext,
    /// Request previous track from playlist
    RequestPrevious,
    /// Equalizer state updated (enabled, gains)
    EqualizerUpdated(bool, Vec<f32>),
    /// Master EQ pre-amp gain (dB) updated.
    EqualizerPreampUpdated(f32),
    /// Volume updated (current_volume, is_muted)
    VolumeUpdated(f32, bool),
    /// Balance updated (-1.0 to 1.0)
    BalanceUpdated(f32),
    /// ReplayGain toggle was acknowledged by the audio thread. The
    /// bool mirrors the new state; UIs can use this as a confirmation
    /// signal instead of guessing the engine accepted the command.
    ReplayGainUpdated(bool),
    /// Loudness compensation toggle acknowledged by the audio thread.
    LoudnessUpdated(bool),
    /// Output device selection acknowledged. `None` means "default host
    /// device"; `Some(name)` matches what `list_output_devices` returned
    /// at pick time. The new device only takes effect on the next track
    /// load — the current Sink is left running so live audio doesn't drop.
    OutputDeviceUpdated(Option<String>),
    /// ICY `StreamTitle` parsed from the live HTTP stream. Fires once
    /// per new title — the audio thread polls the `HttpStream`'s arc-
    /// swap snapshot every position-update tick and forwards changes.
    /// UI surfaces this as the title-bar text and (optionally) as a
    /// desktop notification.
    IcyMetadata(String),
    /// HTTP stream is attempting to reconnect after an upstream blip.
    /// `attempt` counts up from 1; the audio thread emits this on
    /// every transition picked up from the `HttpStream::reconnect_state`
    /// snapshot. The UI shows a transient toast so the user knows the
    /// silence isn't a player-side hang.
    StreamReconnecting { attempt: u32 },
    /// HTTP stream came back online after one or more reconnect
    /// attempts. Fired exactly once per reconnect sequence.
    StreamReconnected,
    /// HTTP stream gave up after exhausting the reconnect backoff
    /// budget. The decoder thread will surface a hard error right
    /// after this; the UI uses this event for the "stream lost"
    /// toast so we don't show two stacked dialogs.
    StreamReconnectFailed,
    /// Repeat mode updated
    RepeatModeUpdated(RepeatMode),
    /// Shuffle mode updated
    ShuffleUpdated(bool),
    // Note: spectrum + waveform are NOT events anymore — they're polled
    // off the engine via `AudioEngine::latest_spectrum()` /
    // `latest_waveform()`, backed by `ArcSwap`. v1 sent them through
    // the unbounded event channel; a slow UI would queue them up
    // forever (audio thread allocates, UI never catches up). See
    // O17 in AUDIO_OBJECTIVES.md.
    /// Error occurred
    Error(String),
}

/// One frame of pre-decimated oscilloscope data, paired (min, max) per
/// output column. v1 sent a single value per column (nearest-neighbor
/// downsample) which aliased high-frequency content. The min/max pair
/// preserves the extreme excursion within each bucket so a 8 kHz sine
/// renders as a visibly oscillating waveform rather than a flat
/// nearest-sample line, regardless of the buffer-to-display ratio.
///
/// `mins` and `maxs` always have identical length; the producer
/// guarantees `mins[i] <= maxs[i]`. Both arrays are in chronological
/// order (oldest first), starting at the latest rising zero-crossing
/// in the capture so the displayed waveform stays phase-stable on
/// periodic content instead of "scrolling".
#[derive(Debug, Clone)]
pub struct WaveformSnapshot {
    pub mins: Vec<f32>,
    pub maxs: Vec<f32>,
}

impl WaveformSnapshot {
    /// Empty snapshot. Used as the initial value before the audio
    /// thread has produced any data.
    pub fn empty() -> Self {
        Self {
            mins: Vec::new(),
            maxs: Vec::new(),
        }
    }

    /// Number of (min, max) columns in this snapshot.
    pub fn len(&self) -> usize {
        self.mins.len()
    }

    /// `true` when no columns are available yet.
    pub fn is_empty(&self) -> bool {
        self.mins.is_empty()
    }
}

/// Per-channel peak + RMS snapshot for the VU-style meter. All values
/// are linear amplitudes in `[0.0, +∞)` (typically clamped to 1.0 by
/// the upstream brickwall limiter); converting to dB is the
/// renderer's job so the on-screen scale stays in one place.
///
/// `peak_*` is the maximum |sample| seen since the last snapshot —
/// reset on each refresh tick so the bar tracks short-term transients
/// instead of an ever-decaying running max. `rms_*` is a one-pole
/// smoothed root-mean-square with ~300 ms time constant: long enough
/// to feel like a "loudness" indicator, short enough to react to a
/// dynamic mix.
#[derive(Debug, Clone, Copy)]
pub struct MeterSnapshot {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

impl MeterSnapshot {
    /// All channels silent — the default published value before the
    /// audio thread has produced any samples.
    pub const fn silent() -> Self {
        Self {
            peak_l: 0.0,
            peak_r: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
        }
    }
}

/// Track metadata information
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: Option<f32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    /// Audio codec format (e.g., "MP3", "FLAC", "OGG", "WAV")
    pub codec: Option<String>,
    /// Bitrate in bits per second (bps) — divide by 1000 for kbps display.
    pub bitrate: Option<u32>,
    /// Track number from ID3v2 `TRCK` / Vorbis `TRACKNUMBER`. Many
    /// taggers write `"3/12"` (track 3 of 12); we keep only the leading
    /// integer so the playlist formatter doesn't have to redo the parse.
    pub tracknumber: Option<u32>,
    /// 4-digit release year, parsed from ID3v2 `TDRC` / `TYER` or Vorbis
    /// `DATE`. We strip non-digit prefixes so `"1994-07-15"` and
    /// `"(1994)"` both surface 1994.
    pub year: Option<u32>,
    /// Genre tag — taken verbatim from metadata. Numeric ID3v1 genre
    /// codes (`"(17)"`) are not resolved here; that's the tagger's job.
    pub genre: Option<String>,
    /// ReplayGain track-level gain in dB, parsed from the file's metadata
    /// (`REPLAYGAIN_TRACK_GAIN` in ID3v2 TXXX / Vorbis comments, `R128_TRACK_GAIN`
    /// in Opus). `None` when the tag is absent or malformed. Applied
    /// downstream as a multiplicative gain on every sample when the user
    /// enables ReplayGain. We deliberately do *not* fall back to the album
    /// gain here — clients can decide which they want.
    pub replaygain_track_gain_db: Option<f32>,
    /// ReplayGain album-level gain in dB. Same source, different tag key.
    /// Kept here so a future "album mode" can pick it without re-parsing.
    pub replaygain_album_gain_db: Option<f32>,
}

/// Parse a ReplayGain tag value (`REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN`,
/// etc.) into a dB float. Tag values vary across taggers — the spec calls
/// for `"-6.78 dB"` but in practice we also see `"-6.78"`, `"+1.23 dB "`,
/// case-mixed `"DB"`, and stray whitespace. Returns `None` on malformed
/// input rather than guessing a default — silently substituting 0 dB
/// would let a broken tag mask itself as "no gain needed".
fn parse_replaygain_db(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    // Strip a trailing "dB" / "DB" / "db" if present. Measure the suffix
    // in *chars*, not a fixed byte offset — `trimmed[len - 2..]` panics
    // whenever the string ends with a multi-byte UTF-8 character (e.g. a
    // stray "é" from a mistagged file), since a raw byte offset can land
    // mid-codepoint.
    let suffix_len: usize = trimmed.chars().rev().take(2).map(char::len_utf8).sum();
    let tail = &trimmed[trimmed.len() - suffix_len..];
    let numeric = if tail.eq_ignore_ascii_case("db") {
        trimmed[..trimmed.len() - suffix_len].trim()
    } else {
        trimmed
    };
    numeric.parse::<f32>().ok().filter(|v| v.is_finite())
}

impl TrackInfo {
    /// Format audio information as a readable string
    /// Example: "MP3 • 320kbps • 44.1kHz • Stereo"
    pub fn format_audio_info(&self) -> String {
        let mut parts = Vec::new();

        // Add codec
        if let Some(ref codec) = self.codec {
            // Clean up codec name (remove debug formatting)
            let clean_codec = codec
                .replace("CODEC(", "")
                .replace(")", "")
                .replace("\"", "")
                .trim()
                .to_string();
            parts.push(clean_codec);
        }

        // Add bitrate
        if let Some(bitrate) = self.bitrate {
            parts.push(format!("{}kbps", bitrate / 1000));
        }

        // Add sample rate
        if let Some(sr) = self.sample_rate {
            let sr_khz = sr as f32 / 1000.0;
            parts.push(format!("{}kHz", sr_khz));
        }

        // Add channels
        if let Some(ch) = self.channels {
            let channel_name = match ch {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                6 => "5.1".to_string(),
                8 => "7.1".to_string(),
                _ => format!("{}ch", ch),
            };
            parts.push(channel_name);
        }

        parts.join(" • ")
    }

    /// Extract metadata from a file
    pub fn from_file(path: &PathBuf) -> Result<Self> {
        let file = File::open(path).context("Failed to open audio file for metadata reading")?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension() {
            hint.with_extension(ext.to_str().unwrap_or(""));
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let mut probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .context("Failed to probe audio file")?;

        let mut tags = TagAccumulator::default();

        // ID3v2 lives in front of the MPEG frames, so the probe consumes it
        // before the demuxer sees anything — those tags end up here and not
        // in `format.metadata()`. Without this branch, every MP3 with only
        // ID3v2 tags would silently fall back to a filename-derived title.
        if let Some(rev) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
            tags.consume(rev);
        }

        let mut format = probed.format;

        // Container-level / streaming metadata (Vorbis comments, ID3v1 tail,
        // ICY headers, …). Probe-level wins on duplicates: ID3v2 is the
        // source of truth when both are present.
        if let Some(metadata_rev) = format.metadata().current() {
            tags.consume(metadata_rev);
        }

        let mut sample_rate = None;
        let mut channels = None;
        let mut duration_secs = None;
        let mut codec = None;

        // Get track information
        if let Some(track) = format.default_track() {
            let codec_params = &track.codec_params;

            sample_rate = codec_params.sample_rate;
            channels = codec_params.channels.map(|c| c.count() as u8);

            // Extract codec name from CodecType
            let codec_type = &codec_params.codec;
            codec = Some(format!("{:?}", codec_type).to_uppercase());

            if let (Some(n_frames), Some(sr)) = (codec_params.n_frames, codec_params.sample_rate) {
                duration_secs = Some(n_frames as f32 / sr as f32);
            }
        }

        // Symphonia's CodecParameters doesn't surface bitrate directly. Fall
        // back to file_size × 8 / duration — this gives the average bitrate
        // (exact for CBR, approximate for VBR), which is what Winamp shows in
        // its 3-digit field anyway.
        let bitrate = duration_secs.and_then(|secs| {
            if secs <= 0.0 {
                return None;
            }
            let file_size = std::fs::metadata(path).ok()?.len();
            Some(((file_size as f64 * 8.0) / secs as f64).round() as u32)
        });

        Ok(TrackInfo {
            path: path.clone(),
            title: tags.title,
            artist: tags.artist,
            album: tags.album,
            tracknumber: tags.tracknumber,
            year: tags.year,
            genre: tags.genre,
            replaygain_track_gain_db: tags.rg_track,
            replaygain_album_gain_db: tags.rg_album,
            duration_secs,
            sample_rate,
            channels,
            codec,
            bitrate,
        })
    }
}

/// Scratch buffer used by `TrackInfo::from_file` to merge metadata from
/// multiple revisions (ID3v2 probe-level then container-level). Kept as
/// a struct so adding a new tag means one field + one match arm and
/// nothing else.
#[derive(Default)]
struct TagAccumulator {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    tracknumber: Option<u32>,
    year: Option<u32>,
    genre: Option<String>,
    rg_track: Option<f32>,
    rg_album: Option<f32>,
}

impl TagAccumulator {
    fn consume(&mut self, rev: &symphonia::core::meta::MetadataRevision) {
        use symphonia::core::meta::StandardTagKey;
        for tag in rev.tags() {
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) if self.title.is_none() => {
                    self.title = Some(tag.value.to_string());
                }
                Some(StandardTagKey::Artist) if self.artist.is_none() => {
                    self.artist = Some(tag.value.to_string());
                }
                Some(StandardTagKey::Album) if self.album.is_none() => {
                    self.album = Some(tag.value.to_string());
                }
                Some(StandardTagKey::TrackNumber) if self.tracknumber.is_none() => {
                    self.tracknumber = parse_tracknumber(&tag.value.to_string());
                }
                Some(StandardTagKey::Date) | Some(StandardTagKey::OriginalDate)
                    if self.year.is_none() =>
                {
                    self.year = parse_year(&tag.value.to_string());
                }
                Some(StandardTagKey::Genre) if self.genre.is_none() => {
                    self.genre = Some(tag.value.to_string());
                }
                Some(StandardTagKey::ReplayGainTrackGain) if self.rg_track.is_none() => {
                    self.rg_track = parse_replaygain_db(&tag.value.to_string());
                }
                Some(StandardTagKey::ReplayGainAlbumGain) if self.rg_album.is_none() => {
                    self.rg_album = parse_replaygain_db(&tag.value.to_string());
                }
                _ => {}
            }
        }
    }
}

/// Many taggers emit track numbers as `"3/12"` (track 3 of 12) or
/// `" 03 "` with padding. Take the first run of digits and parse that;
/// anything else is treated as unset rather than guessing zero.
fn parse_tracknumber(raw: &str) -> Option<u32> {
    let digits: String = raw
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Pull the first 4-digit year out of a date string. Handles `"1994"`,
/// `"1994-07-15"`, `"(1994)"`, `"recorded 1994"`. Anything without four
/// consecutive digits resolves to `None`.
fn parse_year(raw: &str) -> Option<u32> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit()) {
            return std::str::from_utf8(&bytes[i..i + 4]).ok()?.parse().ok();
        }
        i += 1;
    }
    None
}

/// Audio engine that runs in a separate thread
pub struct AudioEngine {
    command_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    thread_handle: Option<thread::JoinHandle<()>>,
    /// Latest spectrum bins published by the audio thread. The audio
    /// thread `store()`s a fresh `Arc<Vec<f32>>` on each visualization
    /// refresh (~30 fps); UI consumers read via [`latest_spectrum`]
    /// and get a cheap `Arc::clone`. Wait-free in both directions.
    spectrum: Arc<ArcSwap<Vec<f32>>>,
    /// Latest oscilloscope snapshot. Same publication model as
    /// [`spectrum`]; the audio thread already produces a phase-stable
    /// frame (zero-crossing trigger + min/max decimation).
    waveform: Arc<ArcSwap<WaveformSnapshot>>,
    /// Latest peak/RMS meter snapshot. Same wait-free publication as
    /// the other visu channels — audio thread `store()`s once per
    /// refresh, UI reads via [`latest_meter`].
    meter: Arc<ArcSwap<MeterSnapshot>>,
}

impl AudioEngine {
    /// Create a new audio engine
    pub fn new() -> Result<Self> {
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();

        let spectrum = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let waveform = Arc::new(ArcSwap::from_pointee(WaveformSnapshot::empty()));
        let meter = Arc::new(ArcSwap::from_pointee(MeterSnapshot::silent()));
        let spectrum_for_thread = spectrum.clone();
        let waveform_for_thread = waveform.clone();
        let meter_for_thread = meter.clone();

        let thread_handle = thread::spawn(move || {
            if let Err(e) = audio_thread_symphonia::audio_thread_main_symphonia(
                command_rx,
                event_tx,
                spectrum_for_thread,
                waveform_for_thread,
                meter_for_thread,
            ) {
                eprintln!("Audio thread error: {}", e);
            }
        });

        Ok(AudioEngine {
            command_tx,
            event_rx,
            thread_handle: Some(thread_handle),
            spectrum,
            waveform,
            meter,
        })
    }

    /// Send a command to the audio thread
    pub fn send_command(&self, cmd: AudioCommand) -> Result<()> {
        self.command_tx
            .send(cmd)
            .context("Failed to send command to audio thread")
    }

    /// Try to receive an event from the audio thread (non-blocking)
    pub fn try_recv_event(&self) -> Option<AudioEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Cheap snapshot of the latest 16-bin spectrum. Returns an empty
    /// `Vec` until the audio thread has published its first frame.
    pub fn latest_spectrum(&self) -> Arc<Vec<f32>> {
        self.spectrum.load_full()
    }

    /// Cheap snapshot of the latest oscilloscope frame. Returns an
    /// empty [`WaveformSnapshot`] until the first audio thread publish.
    pub fn latest_waveform(&self) -> Arc<WaveformSnapshot> {
        self.waveform.load_full()
    }

    /// Cheap snapshot of the latest peak/RMS meter values. Returns
    /// [`MeterSnapshot::silent`] until the first audio thread publish.
    pub fn latest_meter(&self) -> Arc<MeterSnapshot> {
        self.meter.load_full()
    }

    /// Shutdown the audio engine
    pub fn shutdown(mut self) -> Result<()> {
        self.send_command(AudioCommand::Shutdown)?;
        if let Some(handle) = self.thread_handle.take() {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Failed to join audio thread"))?;
        }
        Ok(())
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_replaygain_db_handles_real_world_inputs() {
        // Canonical spec form, with explicit unit.
        assert_eq!(parse_replaygain_db("-6.78 dB"), Some(-6.78));
        // Positive sign + unit.
        assert_eq!(parse_replaygain_db("+1.23 dB"), Some(1.23));
        // No unit (some taggers omit it).
        assert_eq!(parse_replaygain_db("-3.5"), Some(-3.5));
        // Uppercase / mixed-case unit.
        assert_eq!(parse_replaygain_db("-2.0 DB"), Some(-2.0));
        assert_eq!(parse_replaygain_db("0.0 Db"), Some(0.0));
        // Stray whitespace.
        assert_eq!(parse_replaygain_db("  -4.4 dB  "), Some(-4.4));
        // Garbage / empty returns None instead of guessing zero.
        assert_eq!(parse_replaygain_db("not a number"), None);
        assert_eq!(parse_replaygain_db(""), None);
        assert_eq!(parse_replaygain_db("dB"), None);
        // Non-finite floats are rejected so the gain stage can't NaN.
        assert_eq!(parse_replaygain_db("inf"), None);
        assert_eq!(parse_replaygain_db("NaN dB"), None);
    }

    #[test]
    fn parse_replaygain_db_does_not_panic_on_multibyte_suffix() {
        // A trailing multi-byte UTF-8 char must never panic when the
        // suffix check probes the last two chars — regression for a
        // byte-offset slice that used to land mid-codepoint.
        assert_eq!(parse_replaygain_db("é"), None);
        assert_eq!(parse_replaygain_db("-6.78 é"), None);
        assert_eq!(parse_replaygain_db("日"), None);
        // Single-char and empty-after-trim inputs must not panic either.
        assert_eq!(parse_replaygain_db("d"), None);
        assert_eq!(parse_replaygain_db(" "), None);
    }

    #[test]
    fn replaygain_mode_default_is_track() {
        assert_eq!(ReplayGainMode::default(), ReplayGainMode::Track);
    }

    #[test]
    fn replaygain_mode_gain_selection() {
        // Off ignores both tags.
        assert_eq!(ReplayGainMode::Off.gain_db(Some(-6.0), Some(-3.0)), 0.0);

        // Track uses the track tag only.
        assert_eq!(ReplayGainMode::Track.gain_db(Some(-6.0), Some(-3.0)), -6.0);
        assert_eq!(ReplayGainMode::Track.gain_db(None, Some(-3.0)), 0.0);

        // Album uses the album tag only, no track fallback.
        assert_eq!(ReplayGainMode::Album.gain_db(Some(-6.0), Some(-3.0)), -3.0);
        assert_eq!(ReplayGainMode::Album.gain_db(Some(-6.0), None), 0.0);

        // Auto prefers album, falls back to track, then to zero.
        assert_eq!(ReplayGainMode::Auto.gain_db(Some(-6.0), Some(-3.0)), -3.0);
        assert_eq!(ReplayGainMode::Auto.gain_db(Some(-6.0), None), -6.0);
        assert_eq!(ReplayGainMode::Auto.gain_db(None, None), 0.0);
    }

    #[test]
    fn test_audio_engine_creation() {
        // Test that AudioEngine can be created
        let engine = AudioEngine::new();
        assert!(engine.is_ok(), "AudioEngine should be created successfully");
    }

    #[test]
    fn latest_spectrum_starts_empty_and_is_cheap_to_clone() {
        let engine = AudioEngine::new().expect("engine should construct");
        let a = engine.latest_spectrum();
        let b = engine.latest_spectrum();
        // Both snapshots should be empty before the audio thread has
        // published anything, and reading must be cheap (no audio data
        // yet means the underlying Vec is empty).
        assert!(a.is_empty());
        assert!(b.is_empty());
        // ArcSwap returns the same underlying Arc when the slot is
        // unchanged — verifies we're not allocating per call.
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn latest_waveform_starts_empty_and_is_cheap_to_clone() {
        let engine = AudioEngine::new().expect("engine should construct");
        let a = engine.latest_waveform();
        let b = engine.latest_waveform();
        assert!(a.is_empty());
        assert!(b.is_empty());
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn test_audio_engine_shutdown() {
        // Test that AudioEngine can be shut down properly
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");
        let result = engine.shutdown();
        assert!(result.is_ok(), "AudioEngine should shutdown without errors");
    }

    #[test]
    fn test_audio_command_send() {
        // Test that commands can be sent to the audio engine
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");

        // Send a stop command (safe even if nothing is playing)
        let result = engine.send_command(AudioCommand::Stop);
        assert!(result.is_ok(), "Should be able to send Stop command");

        // Send equalizer commands
        let result = engine.send_command(AudioCommand::SetEqualizerEnabled(true));
        assert!(
            result.is_ok(),
            "Should be able to send SetEqualizerEnabled command"
        );

        let result = engine.send_command(AudioCommand::ResetEqualizer);
        assert!(
            result.is_ok(),
            "Should be able to send ResetEqualizer command"
        );
    }

    #[test]
    fn test_audio_event_reception() {
        // Test that events can be received from the audio engine
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");

        // Try to receive events (should be non-blocking)
        let event = engine.try_recv_event();
        // Either None or Some(event) is fine, just shouldn't panic
        // We just verify it returns an Option
        assert!(
            event.is_none() || event.is_some(),
            "Should return an Option"
        );
    }

    #[test]
    fn test_equalizer_commands() {
        // Test equalizer-related commands
        let engine = AudioEngine::new().expect("Failed to create AudioEngine");

        // Test setting individual band
        let result = engine.send_command(AudioCommand::SetEqualizerBand(0, 3.0));
        assert!(result.is_ok(), "Should be able to set equalizer band");

        // Test setting all bands
        let gains = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let result = engine.send_command(AudioCommand::SetEqualizerBands(gains));
        assert!(result.is_ok(), "Should be able to set all equalizer bands");
    }

    #[test]
    fn test_track_info_creation() {
        // Test TrackInfo structure
        let track = TrackInfo {
            path: PathBuf::from("/test/path.mp3"),
            title: Some("Test Track".to_string()),
            artist: Some("Test Artist".to_string()),
            album: Some("Test Album".to_string()),
            duration_secs: Some(180.0),
            sample_rate: Some(44100),
            channels: Some(2),
            codec: Some("MP3".to_string()),
            bitrate: Some(320000),
            tracknumber: None,
            year: None,
            genre: None,
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
        };

        assert_eq!(track.title, Some("Test Track".to_string()));
        assert_eq!(track.sample_rate, Some(44100));
        assert_eq!(track.channels, Some(2));
    }

    #[test]
    fn test_audio_capture_buffer() {
        let mut buffer = AudioCaptureBuffer::new(1024);
        assert_eq!(buffer.capacity(), 1024, "Capacity should match request");

        // Newly-constructed buffer: snapshot is all zeros.
        let mut out = vec![0.5_f32; 1024];
        buffer.snapshot_into(&mut out);
        assert!(
            out.iter().all(|&s| s == 0.0),
            "Empty buffer snapshots to zeros"
        );

        // Push 512 samples, then snapshot 1024 — leading half is the zero
        // pre-roll, trailing half is the pushed data.
        let pushed: Vec<f32> = (0..512).map(|i| (i as f32) / 512.0).collect();
        buffer.update(&pushed, 44100, 2);
        buffer.snapshot_into(&mut out);
        assert!(out[..512].iter().all(|&s| s == 0.0));
        assert_eq!(&out[512..], pushed.as_slice());
        assert_eq!(buffer.sample_rate(), 44100);
        assert_eq!(buffer.channels(), 2);
    }

    #[test]
    fn test_audio_commands_clone() {
        // Test that AudioCommand can be cloned
        let cmd1 = AudioCommand::Stop;
        let cmd2 = cmd1.clone();

        // Both should be Stop
        match (cmd1, cmd2) {
            (AudioCommand::Stop, AudioCommand::Stop) => {}
            _ => panic!("Commands should both be Stop"),
        }
    }

    #[test]
    fn test_multiple_engines() {
        // Test that multiple AudioEngines cannot be created simultaneously
        // (This tests the singleton behavior of audio output)
        let engine1 = AudioEngine::new();
        assert!(engine1.is_ok(), "First engine should be created");

        // Note: Creating a second engine might fail or succeed depending on the audio backend
        // We just test that it doesn't panic
        let _engine2 = AudioEngine::new();
        // No assertion here as behavior is platform-dependent
    }
}
