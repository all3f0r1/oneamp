# Capabilities, limits and evidence

Evidence levels: **T** automated test (name given) · **C** compiled in CI
(job `embedding` or `test`) · **R** run by hand during development, not
automated · **—** not verified. "Simulated" means a memory output or a
local HTTP server, not a real device or network.

## Engine (`oneamp-core`, Linux desktop)

| capability | evidence |
|---|---|
| local playback, sample-exact | T `engine_local::plays_a_file_bit_exact_and_finishes` (simulated output) |
| start at a position without hearing the beginning | T `starts_at_a_position_without_playing_the_beginning` |
| prepare / start paused / resume / seek exact frame | T `load_paused_then_resume_and_seek_reaches_the_exact_frame` |
| seek in the last seconds, past the end | T `seeking_to_the_last_seconds_and_past_the_end_works` |
| gapless, same format | T `gapless_join_has_no_loss_or_duplication`, `remote_media_join_gaplessly_and_repeat` |
| gapless across rates (fixed output) | T `fixed_output_joins_different_rates_gaplessly` |
| format change without host (native output) | T `native_policy_reopens_for_a_new_format_without_the_host` |
| crossfade length exact, per-media ReplayGain | T `crossfade_overlaps_by_its_length_and_keeps_each_media_gain` |
| Repeat One gapless (local, remote) | T `repeat_one_replays_gaplessly_…`, `remote_media_join_gaplessly_and_repeat` |
| next media replaced before commit | T `replacing_the_next_media_cancels_the_old_preparation` |
| rapid command sequences, no leak between loads | T `abandoned_loads_never_leak_into_the_next_one` |
| slow event consumer, bounded memory | T `a_slow_event_consumer_keeps_memory_bounded`, `engine::events` unit tests |
| multichannel fold-down / passthrough | T `surround_is_folded_…`, `dsp::channels` unit tests |
| audible position vs consumed frames (±80 ms) | T `audible_position_tracks_what_the_device_consumed` (simulated clock) |
| route change: reopen, resume from audible frame | T `route_change_reopens_and_resumes_from_the_audible_frame` (simulated fault) |
| EQ AUTO by media id, global curve preserved | T `eq_auto_follows_the_media_and_never_overwrites_the_global_curve` |
| threads released (local, connect, stalled read, preparation) | T `threads::every_scenario_releases_its_threads` |
| cpal output, real ALSA device | R `remote_playback --device "Discard all samples…"` (null device: open, callbacks, gapless remote transition, 0 underruns) |
| cpal device loss → reopen | — (code path shared with the simulated route change; no hardware unplug test) |

## Remote (`http`)

| capability | evidence |
|---|---|
| on-demand via bounded Range requests | T `engine_http::on_demand_media_plays_exactly_through_ranged_requests` |
| seek through demuxer → byte range | T `seeking_remote_media_uses_the_demuxer_and_byte_ranges` |
| Range ignored (200): forward-only, seek refused | T `a_server_ignoring_range_plays_forward_only_and_refuses_seeks` |
| invalid `Content-Range` | T `an_invalid_content_range_fails_the_load` |
| cut transfer resumed at the exact byte (`If-Range`) | T `a_cut_transfer_resumes_at_the_exact_byte` |
| resource changed → `ResourceChanged`, no splice | T `a_resource_changed_mid_read_is_an_error_not_a_splice` |
| 401 → one renewal, identity kept | T `an_expired_access_is_renewed_once_and_the_identity_is_kept` |
| renewal refused / rejected again → final, no loop | T `a_refused_renewal_…`, `a_renewed_access_rejected_again_is_final` |
| 403 never renewed | T `forbidden_is_a_refusal_and_never_triggers_renewal` |
| concurrent renewals coordinated | T `access::tests::concurrent_rejections_renew_once` |
| redirects: secrets kept to the origin, refusal policy | T `redirects_never_carry_secrets_to_another_origin` |
| credentials redacted in errors / Debug | T `error::tests::redact_url_…`, `access::tests::static_access_…`, `a_renewed_access_rejected_again_is_final` |
| cancel during connect / read / preparation | T `threads::every_scenario_releases_its_threads`, `stream_responsiveness` |
| live ICY stream, titles, no seek | T `live_streams_carry_icy_titles_and_cannot_seek`, `http_stream` unit tests |
| HLS/DASH refused with a clear error | T `segmented_delivery_is_refused_with_a_clear_error` |
| transcode starting mid-media (`timeline_offset`) | T `a_delivery_starting_mid_media_reports_the_media_timeline` |
| against a real Tesserafin server | — |

## Formats (gapless claims)

| format | exact length after decoding | evidence |
|---|---|---|
| WAV / FLAC | yes | T `codec_gapless`, engine tests |
| MP3 (LAME/Xing) | yes | T `codec_gapless` |
| Ogg Vorbis | yes | T `codec_gapless` |
| AAC in MP4 | **no** — priming/padding kept (symphonia ignores `elst`) | T `codec_gapless::aac_in_mp4_is_not_gapless_yet` |
| ALAC, AIFF, CAF | not tested | — |

## Processor, desktop players, other platforms

| surface | evidence | limits |
|---|---|---|
| `dsp::Processor` (bypass exactness, ramps, flush, reset, fold-down, sanitizing) | T `dsp::tests::*`, `dsp::stages::tests::*` | stereo stages need ≤ 2 channels (fold or passthrough) |
| settings JSON (versioning, migration, sanitizing) | T `settings::tests::*` | — |
| desktop app on the new API | C + T (78 desktop unit tests); R launched in Xephyr, played a file | GUI flows not re-tested by hand beyond launch + play |
| CLI on the new API (no skins/tags/desktop deps) | C; R `remote_playback` example (same engine) | CLI itself not run against a device |
| WebAssembly module | C `embedding` (wasm32 build, zero imports); T `test-worklet.mjs` in Node | not run in a browser |
| Android bridge | C `embedding` (aarch64 + x86_64 Android, JNI export count); T `wire` unit tests; R+CI Kotlin example on a desktop JVM (processor, player, file, HTTP via `AccessCallback`) | **not run on ART, emulator or device** |
| macOS / Windows | C `cross-platform-check` (`cargo check --all --all-targets`, lib tests) | engine integration tests run on Linux only |

## Known limits

* A transition is committed ~1 s (output buffer) before it is heard.
* Position accuracy is bounded by the latency the backend reports.
* `MultichannelMode::Passthrough` disables EQ, loudness, balance, mono.
* Pump threads stuck in a stalled socket outlive cancellation until the
  body timeout (ranged) or the OS (live).
* AAC-in-MP4 gapless needs edit-list support in symphonia.
* HLS/DASH need a host adapter.
