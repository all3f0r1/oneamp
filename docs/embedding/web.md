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
* Parameters arrive as port messages (`eqBand`, `preamp`, `replayGain`,
  `mediaGain`, `reset`, …) and are applied between blocks on the audio
  thread. `params` (a full `DspParams` JSON) allocates and is meant for
  occasional changes.

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

Verified: the module builds with zero imports, and
`www/test-worklet.mjs` runs the worklet file against it in Node (fake
`AudioWorkletGlobalScope`): bypass is an exact 95-frame delay at 48 kHz,
a −6 dB preamp halves the level, JSON parameters apply, disposal stops
processing. Not verified in a real browser in this work.
