// Runs oneamp-worklet.js in Node against a fake AudioWorkletGlobalScope:
// proves the glue code and the wasm module work together, block by
// block, without a browser. Usage: node test-worklet.mjs <oneamp_wasm.wasm>
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

let Registered;
globalThis.sampleRate = 48000;
globalThis.AudioWorkletProcessor = class { constructor() { this.port = { postMessage() {} }; } };
globalThis.registerProcessor = (name, cls) => { assert.equal(name, 'oneamp-processor'); Registered = cls; };
await import(new URL('./oneamp-worklet.js', import.meta.url));

const module = new WebAssembly.Module(readFileSync(process.argv[2]));
const p = new Registered({ processorOptions: { module, channels: 2 } });

const block = (n) => Float32Array.from({ length: 128 }, (_, i) => 0.5 * Math.sin((n * 128 + i) / 7));
const run = (blocks) => {
  const out = [];
  for (let n = 0; n < blocks; n++) {
    const l = block(n), r = block(n);
    const outputs = [[new Float32Array(128), new Float32Array(128)]];
    assert.equal(p.process([[l, r]], outputs), true);
    out.push(...outputs[0][0]);
  }
  return out;
};

// Bypassed: a pure delay of the limiter lookahead (95 frames at 48 kHz).
const lat = p.wasm.oneamp_latency_frames(p.h);
const out = run(4);
const input = [0, 1, 2, 3].flatMap((n) => [...block(n)]);
for (let i = lat; i < out.length; i++) assert.equal(out[i], input[i - lat]);

// -6 dB preamp through a message: the steady state is half amplitude.
p.onMessage({ type: 'preamp', db: -6.0206 });
const quiet = run(20);
const peak = Math.max(...quiet.slice(-1024).map(Math.abs));
assert.ok(Math.abs(peak - 0.25) < 0.01, `peak ${peak}`);

// JSON parameters (allocates; the view is rebound).
p.onMessage({ type: 'params', params: { mono: true, eq_enabled: true, eq_gains_db: [3,3,3,3,3,3,3,3,3,3] } });
run(2);
p.onMessage({ type: 'dispose' });
assert.equal(p.process([[block(0)]], [[new Float32Array(128)]]), false);
console.log('worklet ok: latency', lat, 'frames');
