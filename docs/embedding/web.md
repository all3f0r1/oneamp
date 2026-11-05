# Web: the processing chain in an AudioWorklet

`oneamp-wasm` builds the chain (no decoder, no transport, no device) as
a WebAssembly module with a plain C ABI and **no imports**:

```sh
cargo build -p oneamp-wasm --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/oneamp_wasm.wasm oneamp-wasm/www/
cp oneamp-core/tests/fixtures/a.ogg oneamp-wasm/www/tone.ogg
(cd oneamp-wasm/www && python3 -m http.server)   # then open http://localhost:8000
```

* `www/oneamp.js` (main thread) compiles the module once and creates the
  `AudioWorkletNode`, passing the compiled `WebAssembly.Module` in
  `processorOptions`.
* `www/oneamp-worklet.js` instantiates it synchronously in the
  `AudioWorkletGlobalScope`, with buffers prepared once by `oneamp_new`
  (128-frame render quantum). Per block: interleave into the wasm buffer,
  `oneamp_process`, de-interleave — no allocation in JS or wasm.
* The node uses `channelInterpretation: 'speakers'`: Web Audio's
  up/down-mix rules map the source onto the processor's layout, so a mono
  source reaches both channels and a 5.1 source folds its centre into
  stereo (−3 dB). `'discrete'` would play mono on the left only and drop
  the centre.
* Parameters arrive as port messages (`eqBand`, `preamp`, `volume`,
  `replayGain`, `mediaGain`, `reset`, …) through typed setters
  (`setEqBand`, `setPreamp`, …) and are applied between blocks on the
  audio thread. Use these for anything changed after construction or
  driven continuously.
* `setParams` sends a whole `DspParams` document. `AudioWorkletGlobalScope`
  has no `TextEncoder` (Chromium, Firefox), so `oneamp.js` encodes the
  JSON to UTF-8 on the main thread and transfers the `Uint8Array`.
  Moving the encoding does **not** make it real-time safe: applying it
  still allocates inside wasm on the audio thread (scratch buffer, JSON
  parse). Keep it for occasional changes (initial state, preset load).
* Failures are never silent: a message that throws (invalid `DspParams`,
  `oneamp_set_params_json` returning −1, an unknown type, a message after
  `dispose`) is posted back as `{ type: 'error', message }`, and
  `createOneAmpNode(ctx, { onError })` receives it (default:
  `console.error`). A rejected `params` leaves the previous parameters
  in place.

The browser keeps decoding (`<audio>`, MSE, WebCodecs) and the media
pipeline; OneAmp only processes. Porting the native player to the
browser is out of scope.

Exports: `oneamp_new(rate, channels, max_frames)`, `oneamp_free`,
`oneamp_buffer`, `oneamp_buffer_frames`, `oneamp_process(h, frames) →
samples`, `oneamp_flush`, `oneamp_reset`, `oneamp_latency_frames`,
`oneamp_output_channels`, `oneamp_is_bypassed`, `oneamp_set_eq_enabled`,
`oneamp_set_eq_band`, `oneamp_set_preamp`, `oneamp_set_volume`,
`oneamp_set_balance`, `oneamp_set_mono`, `oneamp_set_loudness`,
`oneamp_set_replaygain(mode 0..3, prevent_clipping)`,
`oneamp_set_media_gain(track_db, track_peak, album_db, album_peak,
context)` (NaN = unknown), `oneamp_set_params_json`, `oneamp_alloc`,
`oneamp_dealloc`.

Verified, in CI:

* the module builds with zero imports;
* `www/test-worklet.mjs` runs the worklet file against it in Node, in a
  fake `AudioWorkletGlobalScope` without `TextEncoder`/`TextDecoder`:
  bypass is an exact 95-frame delay at 48 kHz, the typed preamp and
  `params` both change the samples, invalid parameters are reported and
  change nothing, disposal stops processing;
* `browser-test/test-browser.mjs` renders `oneamp.js` + the worklet
  through an `OfflineAudioContext` in Chromium and Firefox (Playwright)
  and checks the output: transparent 95-frame delay, `setPreamp` and
  `setParams` at −6 dB halve the level, an invalid `setParams` reaches
  `onError` with the level untouched, a mono source is identical on both
  channels, a 5.1 centre-only source reaches both channels at −3 dB.

```sh
cd oneamp-wasm/browser-test
npm ci && npx playwright-core install chromium firefox
npm test   # expects the release wasm32 build under target/
```

Not covered: Safari / WebKit, a live `AudioContext` on a device, and
`<audio>` → `MediaElementAudioSourceNode` routing.
