// AudioWorkletGlobalScope side of OneAmp's processing chain.
//
// The main thread compiles the wasm module and passes it in
// `processorOptions.module`; it is instantiated synchronously here (no
// imports, no fetch in the worklet). Buffers are prepared once by
// `oneamp_new`: the per-block path below allocates nothing, in JS or in
// wasm.

const RENDER_QUANTUM = 128;

class OneAmpProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const { module, channels = 2 } = options.processorOptions;
    this.wasm = new WebAssembly.Instance(module, {}).exports;
    this.channels = channels;
    // `sampleRate` is a global of the AudioWorkletGlobalScope.
    this.h = this.wasm.oneamp_new(sampleRate, channels, RENDER_QUANTUM);
    if (this.h === 0) {
      throw new Error(`OneAmp: unsupported format ${sampleRate} Hz / ${channels} ch`);
    }
    this.outChannels = this.wasm.oneamp_output_channels(this.h);
    this.bindBuffer();
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  // The view must be rebuilt if wasm memory ever grows (only
  // `params` messages allocate).
  bindBuffer() {
    const frames = this.wasm.oneamp_buffer_frames(this.h);
    this.view = new Float32Array(
      this.wasm.memory.buffer,
      this.wasm.oneamp_buffer(this.h),
      frames * this.channels,
    );
  }

  onMessage(m) {
    const w = this.wasm;
    const h = this.h;
    switch (m.type) {
      case 'eqEnabled': w.oneamp_set_eq_enabled(h, m.on ? 1 : 0); break;
      case 'eqBand': w.oneamp_set_eq_band(h, m.band, m.db); break;
      case 'preamp': w.oneamp_set_preamp(h, m.db); break;
      case 'volume': w.oneamp_set_volume(h, m.value); break;
      case 'balance': w.oneamp_set_balance(h, m.value); break;
      case 'mono': w.oneamp_set_mono(h, m.on ? 1 : 0); break;
      case 'loudness': w.oneamp_set_loudness(h, m.on ? 1 : 0, m.listeningVolume ?? 1); break;
      case 'replayGain': w.oneamp_set_replaygain(h, m.mode, m.preventClipping ? 1 : 0); break;
      case 'mediaGain':
        // Unknown figures as NaN. Call before the media's first block.
        w.oneamp_set_media_gain(
          h, m.trackDb ?? NaN, m.trackPeak ?? NaN, m.albumDb ?? NaN, m.albumPeak ?? NaN,
          m.context ?? 0,
        );
        break;
      case 'reset': w.oneamp_reset(h); break;
      case 'params': {
        const bytes = new TextEncoder().encode(JSON.stringify(m.params));
        const p = w.oneamp_alloc(bytes.length);
        new Uint8Array(w.memory.buffer, p, bytes.length).set(bytes);
        const rc = w.oneamp_set_params_json(h, p, bytes.length);
        w.oneamp_dealloc(p, bytes.length);
        this.bindBuffer();
        if (rc !== 0) this.port.postMessage({ type: 'error', message: 'invalid params' });
        break;
      }
      case 'dispose':
        w.oneamp_free(h);
        this.h = 0;
        break;
    }
  }

  process(inputs, outputs) {
    if (this.h === 0) return false;
    const input = inputs[0] || [];
    const output = outputs[0];
    const frames = output[0] ? output[0].length : RENDER_QUANTUM;
    if (this.view.buffer !== this.wasm.memory.buffer) this.bindBuffer();
    const ch = this.channels;
    const view = this.view;
    // Interleave (a missing input channel is silence).
    for (let c = 0; c < ch; c++) {
      const src = input[c];
      for (let i = 0; i < frames; i++) view[i * ch + c] = src ? src[i] : 0;
    }
    this.wasm.oneamp_process(this.h, frames);
    // De-interleave the output layout (folded to stereo for surround).
    const oc = this.outChannels;
    for (let c = 0; c < output.length; c++) {
      const dst = output[c];
      const sc = Math.min(c, oc - 1);
      for (let i = 0; i < frames; i++) dst[i] = view[i * oc + sc];
    }
    return true;
  }
}

registerProcessor('oneamp-processor', OneAmpProcessor);
