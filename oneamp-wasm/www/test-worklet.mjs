// Runs oneamp-worklet.js in Node against a fake AudioWorkletGlobalScope:
// proves the glue code and the wasm module work together, block by
// block, without a browser. Usage: node test-worklet.mjs <oneamp_wasm.wasm>
//
// The fake scope mirrors the real one where it matters: no TextEncoder /
// TextDecoder (absent from AudioWorkletGlobalScope in Chromium and
// Firefox), and messages go through the processor's port handler. The
// real-browser counterpart is test-browser.mjs.
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

// Main-thread side: encodes the JSON, as oneamp.js does.
const utf8 = new TextEncoder();
delete globalThis.TextEncoder;
delete globalThis.TextDecoder;
assert.equal(typeof TextEncoder, 'undefined');
assert.equal(typeof TextDecoder, 'undefined');

let Registered;
globalThis.sampleRate = 48000;
globalThis.AudioWorkletProcessor = class {
  constructor() {
    this.port = { posted: [], postMessage(m) { this.posted.push(m); } };
  }
};
globalThis.registerProcessor = (name, cls) => { assert.equal(name, 'oneamp-processor'); Registered = cls; };
await import(new URL('./oneamp-worklet.js', import.meta.url));

const module = new WebAssembly.Module(readFileSync(process.argv[2]));
const p = new Registered({ processorOptions: { module, channels: 2 } });
const send = (data) => p.port.onmessage({ data });
const errors = () => p.port.posted.splice(0).filter((m) => m.type === 'error');
const params = (obj) => send({ type: 'params', bytes: utf8.encode(JSON.stringify(obj)) });

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
const steadyPeak = () => Math.max(...run(20).slice(-1024).map(Math.abs));

// Bypassed: a pure delay of the limiter lookahead (95 frames at 48 kHz).
const lat = p.wasm.oneamp_latency_frames(p.h);
const out = run(4);
const input = [0, 1, 2, 3].flatMap((n) => [...block(n)]);
for (let i = lat; i < out.length; i++) assert.equal(out[i], input[i - lat]);

// -6 dB preamp through the typed message: the steady state is half amplitude.
send({ type: 'preamp', db: -6.0206 });
let peak = steadyPeak();
assert.ok(Math.abs(peak - 0.25) < 0.01, `typed preamp: peak ${peak}`);
assert.deepEqual(errors(), []);

// JSON parameters replace the whole set: back to 0 dB, then -6 dB again.
params({});
peak = steadyPeak();
assert.ok(Math.abs(peak - 0.5) < 0.01, `params {}: peak ${peak}`);
params({ preamp_db: -6.0206 });
peak = steadyPeak();
assert.ok(Math.abs(peak - 0.25) < 0.01, `params preamp_db: peak ${peak}`);
params({ mono: true, eq_enabled: true, eq_gains_db: [3, 3, 3, 3, 3, 3, 3, 3, 3, 3] });
run(2);
assert.deepEqual(errors(), []);

// Failures are reported, and leave the parameters untouched.
params({ preamp_db: -6.0206 });
run(20);
params({ eq_gains_db: 'loud' });
assert.match(errors()[0]?.message ?? '', /oneamp_set_params_json returned -1/);
peak = steadyPeak();
assert.ok(Math.abs(peak - 0.25) < 0.01, `after invalid params: peak ${peak}`);
send({ type: 'params', params: { preamp_db: 0 } }); // old object form
assert.match(errors()[0]?.message ?? '', /expected non-empty UTF-8 JSON bytes/);
send({ type: 'nope' });
assert.match(errors()[0]?.message ?? '', /unknown message type/);

send({ type: 'dispose' });
assert.equal(p.process([[block(0)]], [[new Float32Array(128)]]), false);
send({ type: 'dispose' });
assert.deepEqual(errors(), []);
send({ type: 'preamp', db: 0 });
assert.match(errors()[0]?.message ?? '', /processor disposed/);
console.log('worklet ok: latency', lat, 'frames');
