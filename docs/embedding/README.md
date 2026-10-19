# Embedding `oneamp-core`

`oneamp-core` is OneAmp's audio engine as a library. The desktop player
and the CLI are its first two clients; they use exactly the interfaces
described here.

| Document | For |
|---|---|
| this file | API, responsibilities, behaviour |
| [tesserafin.md](tesserafin.md) | what a Tesserafin client supplies (ids, metadata, delivery, access) |
| [android.md](android.md) | the Kotlin/JNI bridge, Media3, platform roles |
| [web.md](web.md) | the WebAssembly processor and its AudioWorklet |
| [matrix.md](matrix.md) | capabilities, limits and evidence per platform |

## Two ways in

**Full player — `AudioEngine`.** The host hands it `MediaDescriptor`s;
the engine opens, probes, decodes (on one worker thread per media),
processes, joins media gaplessly or crossfades, and writes to an output
the host chooses. No UI is involved anywhere.

**PCM processor — `dsp::Processor`.** A platform player keeps control of
playback and hands blocks of interleaved `f32`. No thread, no device, no
network; everything is allocated at construction, so `process`,
`set_params` and `set_media_gain` are real-time safe.

Both run the same chain: per-media gain (ReplayGain) → fold-down when
wider than stereo → 10-band EQ → loudness → mono → preamp + volume →
balance → 2 ms lookahead limiter.

## Cargo features

| feature | brings | needed for |
|---|---|---|
| *(none)* | `dsp`, `settings`, `queue`, EQ presets | the processor; builds for `wasm32-unknown-unknown` |
| `player` | symphonia, decode workers, engine, pull output, visualisation analysis | `AudioEngine` with a host output |
| `http` | ureq (rustls), Range and live sources, access contract | remote media |
| `cpal-output` | cpal | the desktop output backend |
| `skins` | zip, image | Winamp `.wsz` skins |
| `tags` | lofty | tag editing |
| `desktop` | `player` + `http` + `skins` + `tags`, playlist files, recent files | the desktop app |

Default: `desktop` + `cpal-output`. The CLI uses `player,http,cpal-output`;
the Android bridge `player,http`; the wasm crate none.

## Boundaries

What the library owns: decoding, byte transport for files and
progressive HTTP, buffering, the chain, transitions, the output ring,
timing, and the rules below about credentials.

What the host owns, and the library never re-models:

* **identity** — `MediaId` is the host's (a library item id, a path, a
  saved station URL). It never derives from a delivery URL;
* **the queue** — the host says what plays next (`SetNext`); shuffle
  order, history, "play next" are the host's. `PlayQueue` exists for hosts
  without a queue (the CLI);
* **delivery decisions** — direct file vs transcode, container, bitrate,
  start offset of a server transcode: the host describes the result in
  the descriptor (`timeline_offset`, `format`, `duration`);
* **credentials** — minted, stored and renewed by the host's
  `AccessProvider`; the engine only asks and obeys;
* **platform integration** — media sessions, audio focus, background
  execution, route policy, persistence of settings.

Segmented delivery (HLS, DASH) is not a byte stream and is refused by the
HTTP transport (`ErrorCategory::Unsupported`); a host that needs it
supplies an adapter as `MediaLocation::Custom(SourceFactory)`.

## Describing media

```rust
let media = MediaDescriptor::http_with(MediaId::new("item-42"), my_access_provider)
    .with_metadata(MediaMetadata { title: Some("…".into()), ..Default::default() })
    .with_duration(Duration::from_secs(245))
    .with_gain(MediaGain { track_gain_db: Some(-7.2), ..Default::default() })
    .with_listening_context(ListeningContext::Album)
    .starting_at(Duration::from_secs(61));
```

| field | meaning |
|---|---|
| `id` | stable identity, shown in diagnostics — never a secret |
| `location` | `File(path)`, `Http { access, delivery }`, `Custom(factory)` |
| `metadata` | host metadata; overrides tags field by field |
| `duration` | whole-media duration, if known |
| `format` | extension / MIME hints for the probe |
| `gain` | normalisation figures; replace the stream's tags |
| `listening_context` | album / mixed, for `ReplayGainMode::Auto` |
| `start_position` | reached before the first sample is output |
| `seekable` | `Some(false)` forbids seeking |
| `timeline_offset` | media time of the first delivered sample (a server transcode started mid-media); positions stay on the media's timeline, seeks before it fail with `InvalidRequest` |

`Delivery::Auto` decides from the first response: a `206` proves a finite,
seekable file; ICY headers or a body without length mean live; a length
without range support means finite and forward-only. `OnDemand` and
`Live` force the choice. `Content-Length` alone never decides.

### Remote media in detail

* **On demand** (`http_range::RangeSource`): bounded `Range` requests
  (`HttpConfig::request_bytes`, default 1 MiB); seeks are done by the
  demuxer (its index or bisection) and become a request at the byte it
  asks for. A cut transfer resumes at the exact byte with `If-Range` and
  the first response's validator; a changed resource (validator, size, a
  `200` to a range request, `416`) fails with `ResourceChanged` instead
  of splicing two files. A short body is an interruption, never the end:
  the end is the length `Content-Range` announced. Reconnects are bounded
  (`max_retries`, backoff). A server that ignores `Range` still plays,
  forward only, and reports `seek: false`.
* **Live** (`http_stream::HttpStream`): ICY metadata stripped and
  published (`IcyMetadata` events); a dropped connection reconnects at
  the live edge (1, 2, 5, 10 s); never seekable.
* **Cancellation**: `Stop`, a new `Load` or a replaced `SetNext` cancel
  the worker; connects and reads notice within ~50 ms and the command
  returns at once. Ceiling, stated: an abandoned connect thread lives
  until the connect/response timeout; a pump thread blocked in a stalled
  socket read lives until the request's body timeout (ranged requests)
  or the OS closes the socket (live streams).

### Access contract (`access`)

```rust
pub trait AccessProvider: Send + Sync {
    fn access(&self) -> Result<Access, PlayerError>;
    fn renew(&self, rejected: &Access, reason: RenewReason) -> Result<Access, PlayerError>;
}
```

`Access` = URL + headers + optional expiry + `RedirectPolicy`. Engine
rules:

* `401` → one `renew`; a renewed access rejected again is final
  (`AccessExpired`, not recoverable). No reconnect loop.
* `403` → `AccessDenied`, never renewed. `404` → `NotFound`.
* Concurrent rejections of one access renew once (`CoordinatedAccess`,
  applied by `MediaDescriptor::http_with`).
* `renew` must not widen rights; returning an error is always allowed.
* Redirects: headers only to the issuing origin unless
  `HeadersToAnyOrigin`; `https → http` refused; `Refuse` makes any
  redirect an error.
* `Debug` of `Access`, error messages and diagnostics carry
  `error::redact_url` output: no query value, no header value, no
  user-info.

## Commands, results, events, snapshot

* `AudioEngine::send(cmd) -> Result<CommandId, SendError>`: queued, not
  executed. The queue is bounded (`command_capacity`); a full queue is
  `SendError::QueueFull`.
* Transport commands (`Load`, `Resume`, `Pause`, `Stop`, `Seek`,
  `SetNext`, `ApplySettings`, `OutputRouteChanged`) are answered by
  `AudioEvent::CommandCompleted { id, instance, result }`: `Load` when the
  media is ready, `Seek` when the position is reached. A command whose
  media is dropped first is answered `Cancelled`. Settings commands are
  answered by their `…Updated` echo.
* Every load gets a `PlaybackInstance`; `TrackLoaded`, `Seeked`,
  `Error`, `CommandCompleted` carry it. Results of an abandoned
  preparation are discarded, never applied to what follows.
* Events go through a bounded queue: replaceable updates (position,
  volume, balance, EQ, reconnect attempt) are merged; past capacity the
  oldest events are dropped and an `EventsDropped(n)` is delivered.
* `AudioEngine::snapshot()` is coherent and always current: an event is
  released only after the snapshot reflecting it was published.

| operation | command |
|---|---|
| prepare | `Load { media, paused: true }` → `Ready` |
| start | `Load { paused: false }`, or `Resume` after a prepared load |
| start paused | `Load { paused: true }` |
| seek | `Seek(Duration)` |
| stop | `Stop` (releases media, next media and the output) |
| close | `AudioEngine::shutdown()` (joins the thread; workers and device released) |
| retry after a recoverable error | `Resume` (continues from the audible position; live: rejoins the edge) |

States: `Idle`, `Preparing`, `Ready`, `Playing`, `Paused`, `Buffering`
(data temporarily unavailable), `Ended` (normal end), `Error`
(`last_error.recoverable` tells a network failure from a terminal one).

The snapshot also gives: `position` (audible), `decoded_position`,
`duration`, `available` ranges, `capabilities` (seek / pause /
resume / live), `source` format, `output` (what the output really does),
`processing` (stages actually active, crossfading, resampling,
`untouched_to_output`), `eq` (global vs active curve, auto preset),
`next` (pending / preparing / ready / failed), `diagnostics` (underruns,
overflow waits, output errors and reopenings, dropped events, skipped
corrupt packets, transport reconnects).

## Time

Positions are `Duration`s; internally frames (`u64`) at the source rate.
The audible position is the decoded position minus everything between
the decoder and the speaker — the chain lookahead, the output ring, a
resampler, and the device latency as the backend reports it (cpal:
callback timestamps; pull hosts: `PullHandle::set_latency`). Its accuracy
is bounded by that report; tests check it within 80 ms of what a memory
device consumed.

Seeks use the demuxer's accurate mode and drop the samples between the
packet it lands on and the target; the reported position is the frame
reached (the end of the media for a target past it). There is no "last
seconds" clamp any more.

## Transitions

* `SetNext(Some(media))` tells the engine what follows; it starts
  preparing `EngineConfig::preload_lead` before the end (default 10 s,
  plus the crossfade). `SetNext` with another media cancels the
  preparation in flight; `SetNext(None)` clears it.
* A transition is committed once the current media's last sample is
  written to the output (~1 s before it is heard); a later `SetNext`
  applies to the media after it.
* **Gapless**: same pipeline format → the output is untouched and the
  next media's first sample follows the last one (tested sample-exact,
  local and remote). Formats with exact gapless data: WAV/AIFF/FLAC/ALAC
  by construction, MP3 (LAME/Xing), Ogg Vorbis — tested. AAC in MP4 is
  *not*: symphonia ignores the `elst` edit list (a test pins this).
* **Output policy**: `NativePerMedia` (desktop default) opens the output
  at each media's rate; a format change reopens it without host action
  (short gap). `Fixed { rate, channels }` converts every media (sinc
  resampling with its delay trimmed, fold-down) so joins and crossfades
  work across rates and the output never reopens.
* **Crossfade**: equal-power, starting exactly `seconds` before the end;
  each media keeps its own ReplayGain until the end of the fade. The EQ
  follows the incoming media from its first contributing sample
  (ramped): its auto preset, or the global curve.
* **Repeat One** is the engine's: the media is prepared again as its own
  successor, joined gaplessly. Repeat All/Off and shuffle are the queue
  owner's.
* `SetStopAfterCurrent(true)` finishes after the current media.

## Settings

`settings::AudioSettings` is the whole user configuration, versioned
JSON (`to_json` / `from_json`: unknown fields ignored, missing fields
defaulted, newer versions refused, values sanitized). `ApplySettings`
applies it atomically. The host chooses where it lives and how it syncs.

`EqAutoSettings` maps media ids to curves. A preset plays while its media
plays and never becomes the global curve; a hand edit while it plays
makes the edited curve global.

ReplayGain modes: `Off`, `Track`, `Album`, `Auto` (album gain in an album
context, track gain for mixed listening; with an unknown context, album
when present else track). `replaygain_prevent_clipping` caps the gain by
the media's peak.

## Channels and outputs

`dsp::channels::ChannelLayout` represents speaker positions (from
symphonia's mask). Wider-than-stereo sources are folded to stereo with
ITU-R BS.775 weights, normalised so the fold cannot clip, unless
`MultichannelMode::Passthrough` keeps every channel (then only gains and
the limiter apply, and the snapshot shows the stereo stages inactive).
Nothing is silently dropped any more.

Outputs implement `output::OutputBackend` / `OutputStream`: `cpal`
(desktop), `pull` (host-driven: Android, offline render, tests). Writes
never drop audio; what the ring cannot take stays pending
(`overflow_waits`). Device loss and route changes reopen the output and
resume from the audible frame (`output_reopens`).
`OutputInfo` states the conversions really performed (resampling,
channel mapping, dither) and `shared_mode: true`: no path is claimed
bit-perfect up to the DAC; `untouched_to_output` only says that nothing
between decoder and output modified the samples.

## Threads

The engine thread never does I/O. One worker per media in preparation or
playback (plus an HTTP pump thread per open connection and a short-lived
connect thread per request). All are released when their media is
dropped; `shutdown()` returns after the engine thread has released
everything. The cpal callback never locks and never allocates (beyond a
one-time scratch growth).

## Examples

* `cargo run -p oneamp-core --example remote_playback -- <url> [next] [--start S] [--device NAME]`
* `cargo run -p oneamp-core --no-default-features --example pcm_processor`
* Kotlin: `oneamp-android/examples-kotlin/Example.kt`
* Web: `oneamp-wasm/www/`
