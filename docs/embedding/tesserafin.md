# Integrating OneAmp in a Tesserafin client

What a Tesserafin client (web, Android, desktop) supplies so that
`oneamp-core` plays its music, and which responsibilities stay on the
Tesserafin side. Contracts referenced here are the ones found in
`tesserafin` (`docs/playback-credential-server-contract.md`,
`PlaybackCredentialsController`), `tesserafin-web`
(`docs/tesserafin/playback-credential-transport-design.md`) and
`tesserafin-mobile` / RFC-0006 (platform-owned responsibilities). Nothing
in OneAmp re-models a Tesserafin concept: it takes Tesserafin's values
as given.

## Responsibility split

| Tesserafin client keeps | OneAmp does |
|---|---|
| the library, the queue, shuffle order, "play next" | prepares and plays what the client says comes next (`SetNext`) |
| `PlaybackInfo` / delivery decision (direct stream, remux, transcode) | plays the resulting byte stream; refuses HLS/DASH unless given an adapter |
| minting/renewing playback capabilities with the durable token | asks `AccessProvider` for an access and to renew it after a 401; never sees the durable token |
| `PlaySessionId`, progress reports (`Sessions/Playing…`) | emits `TrackLoaded` / `Position` / `Finished` with the instance id to report against |
| storing settings, per-user sync | `AudioSettings` JSON in, applied atomically |
| MediaSession, audio focus, foreground service, notifications (RFC-0006 §6) | commands in, events out; nothing platform-specific |

## Per item: the media descriptor

| Tesserafin | `MediaDescriptor` |
|---|---|
| item `Id` | `id: MediaId::new(item_id)` — stable; never the URL |
| `Name`, `Artists`/`AlbumArtist`, `Album`, `IndexNumber`, `ProductionYear`, `Genres` | `metadata` |
| `RunTimeTicks` (100 ns ticks) | `duration = Duration::from_nanos(ticks * 100)` |
| `NormalizationGain` / `AlbumNormalizationGain` (LUFS scan, dB) | `gain: MediaGain { track_gain_db, album_gain_db, .. }` |
| album / shuffle playback | `listening_context: Album / Mixed` (for ReplayGain `Auto`) |
| resume position (`UserData.PlaybackPositionTicks`) | `start_position` — reached before any sample is output |
| media source `Container` / transcode container | `format.mime_type` or `format.extension` |
| transcode started at `StartTimeTicks` | `timeline_offset` = that time; positions stay on the item's timeline; a seek before it fails with `InvalidRequest` → request a new delivery at the target |
| radio / Live TV audio | `with_delivery(Delivery::Live)` |

Delivery URL families (from the web transport design):

* direct stream `/Audio/{id}/stream?static=true` (and `/stream.{container}`):
  on-demand, Range supported → seekable, resumable, gapless;
* `/Audio/{id}/universal`: may answer with a progressive transcode
  (often without range support → forward-only, the snapshot says
  `seek: false`) or with HLS → refused by the HTTP transport;
* HLS (`master.m3u8` …): needs a segmented adapter as
  `MediaLocation::Custom` — not provided by OneAmp.

Prefer direct stream for music the device can decode (symphonia: MP3,
AAC/ALAC in MP4, FLAC, Vorbis, WAV, AIFF, CAF, MKA); ask the server for a
progressive (non-HLS) transcode otherwise.

## Access: implementing `AccessProvider` on playback capabilities

The server contract: a capability is minted with
`POST /Playback/Capabilities` (durable session token **in a header**),
bound to user, device, session, play session, item, media source and a
scope set (`Media` for audio), valid 15 minutes, renewable only during
its last 5 minutes with `POST /Playback/Capabilities/{id}/Renew` (the
secret does not rotate: the URL stays valid), never after expiry. It
travels as the `playbackCapability` query parameter. No `ApiKey`
fallback exists or may be added.

```rust
struct CapabilityAccess { client: TesserafinClient, item: String, media_source: String, play_session: String }

impl AccessProvider for CapabilityAccess {
    fn access(&self) -> Result<Access, PlayerError> {
        let cap = self.client.mint_media_capability(&self.item, &self.media_source, &self.play_session)
            .map_err(to_player_error)?;                     // header-authenticated POST
        let mut a = Access::new(format!(
            "{}/Audio/{}/stream?static=true&mediaSourceId={}&playSessionId={}&playbackCapability={}",
            self.client.base, self.item, self.media_source, self.play_session, cap.secret));
        a.expires_at = Some(cap.expires_at);
        a.redirects = RedirectPolicy::Refuse;               // the server never redirects media
        Ok(a)
    }

    fn renew(&self, _rejected: &Access, _why: RenewReason) -> Result<Access, PlayerError> {
        // Called after a 401 (expired, unknown — e.g. server restart — or
        // revoked). An expired capability cannot be renewed: mint a new
        // one with the same bindings and the same single scope.
        self.access()
    }
}
```

`to_player_error` maps the mint answer: network failures → `Network`
(recoverable); 401 on the mint (durable token gone: logout, password
change) → `AccessExpired`; 403/404 (item no longer visible — refusals
are 404 by design) → `AccessDenied` / `NotFound`. OneAmp then:

* renews **once** per rejection and fails with `AccessExpired`
  (not recoverable) if the fresh capability is rejected too — no loop;
* never renews on `403` (`PlaybackCapability…Mismatch`): a narrower
  binding cannot be fixed by retrying;
* coordinates concurrent rejections (the playing media and its prepared
  repeat share one renewal);
* keeps the media id across renewals — progress reporting is unaffected;
* never logs the capability: URLs are redacted in errors and `Debug`.

Proactive renewal inside the last-5-minute window (to avoid a 401 at
all) stays in the client's broker, as on the web: since the secret does
not rotate, the URL OneAmp holds keeps working and nothing needs to be
told to the engine. Clear the broker's timers on stop, logout, server,
user or session change, as the web design requires.

Play sessions: one `PlaySessionId` per media load is the client's; map
it to OneAmp's `PlaybackInstance` from `TrackLoaded` / `CommandCompleted`.

## Queue and transitions

The client keeps its queue and, whenever its successor changes, sends
`SetNext(Some(next_descriptor))` (or `None` at the end without repeat).
On `TrackLoaded { instance, track }` for that successor, advance the
queue, report the new item as playing, and send the following
`SetNext`. Repeat One: `SetRepeatMode(One)` — OneAmp repeats gaplessly.
Shuffle: the client's predetermined order; OneAmp only sees "next".

A transition is committed about one second before it is heard (the
output buffer); a queue change after that applies to the item after it.

## Settings

Store `AudioSettings::to_json()` wherever the client keeps user
preferences (e.g. display preferences on the server) and apply with
`ApplySettings` at start and on sync. EQ AUTO presets are keyed by media
id — Tesserafin item ids work across devices.

## Errors to surface

| `ErrorCategory` | likely cause | client action |
|---|---|---|
| `Network`, `Timeout` (recoverable) | connectivity | `Resume` retries from the audible position |
| `AccessExpired` (not recoverable) | session ended, renewal refused | re-authenticate, reload at the last position |
| `AccessDenied`, `NotFound` | rights changed, item removed | skip, report |
| `ResourceChanged` | file replaced during playback | reload from the position (new validator) |
| `Unsupported` | HLS, codec | request another delivery |
| `InvalidRequest` | seek before `timeline_offset` | request a delivery starting at the target |
