// Real-browser test of oneamp-wasm/www: the shipped oneamp.js helper and
// oneamp-worklet.js, rendered through an OfflineAudioContext in Chromium
// and Firefox (Playwright). Checks the sound itself, not only that calls
// return.
//
//   npm ci && npx playwright-core install chromium firefox
//   node test-browser.mjs <oneamp_wasm.wasm> [chromium|firefox ...]
import http from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, firefox } from 'playwright-core';

const wasmPath = process.argv[2];
if (!wasmPath || !existsSync(wasmPath)) {
  console.error('usage: node test-browser.mjs <oneamp_wasm.wasm> [chromium|firefox ...]');
  process.exit(2);
}
const engines = { chromium, firefox };
const selected = process.argv.slice(3).length ? process.argv.slice(3) : Object.keys(engines);
const www = path.join(path.dirname(fileURLToPath(import.meta.url)), '../www');

// Secure context (127.0.0.1) so AudioWorklet is available.
const files = {
  'oneamp.js': path.join(www, 'oneamp.js'),
  'oneamp-worklet.js': path.join(www, 'oneamp-worklet.js'),
  'oneamp_wasm.wasm': wasmPath,
};
const types = { '.js': 'text/javascript', '.wasm': 'application/wasm' };
const server = http.createServer((req, res) => {
  const name = new URL(req.url, 'http://x').pathname.slice(1);
  if (name === '') {
    res.writeHead(200, { 'content-type': 'text/html' });
    return res.end('<!doctype html><meta charset="utf-8"><title>OneAmp browser test</title>');
  }
  const file = files[name];
  if (!file) { res.writeHead(404); return res.end(); }
  res.writeHead(200, { 'content-type': types[path.extname(name)] });
  res.end(readFileSync(file));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}/`;

// Runs in the page. Returns raw figures; assertions happen in Node.
async function suite() {
  const { createOneAmpNode } = await import('./oneamp.js');
  const SR = 48000;
  const sine = (n) => 0.5 * Math.sin(n / 7);
  const peakOf = (a) => a.reduce((m, x) => Math.max(m, Math.abs(x)), 0);

  // Render `frames` of a 0.5 sine through a fresh node. `fill(c)` says
  // which input channels carry the sine (others are silent). `setup`
  // runs before rendering; a sentinel message (answered by an error,
  // port messages being ordered) proves the worklet has applied it.
  async function render({ frames, inChannels = 2, fill = () => true, setup }) {
    const ctx = new OfflineAudioContext(2, frames, SR);
    const errors = [];
    let synced;
    const sync = new Promise((r) => { synced = r; });
    const oa = await createOneAmpNode(ctx, {
      onError: (message) => (message.startsWith('__sync') ? synced() : errors.push(message)),
    });
    const buf = ctx.createBuffer(inChannels, frames, SR);
    const tone = Float32Array.from({ length: frames }, (_, i) => sine(i));
    for (let c = 0; c < inChannels; c++) if (fill(c)) buf.getChannelData(c).set(tone);
    const src = ctx.createBufferSource();
    src.buffer = buf;
    src.connect(oa.node).connect(ctx.destination);
    if (setup) setup(oa);
    oa.node.port.postMessage({ type: '__sync' });
    await Promise.race([sync, new Promise((r) => setTimeout(r, 5000))
      .then(() => errors.push('worklet never answered the sync message'))]);
    src.start();
    const out = await ctx.startRendering();
    return { L: out.getChannelData(0), R: out.getChannelData(1), errors };
  }

  // Largest |out[i] - k * in[i - 95]| over both channels.
  const delayError = ({ L, R }, k = 1, lat = 95) => {
    let e = 0;
    for (let i = lat; i < L.length; i++) {
      const x = k * Math.fround(sine(i - lat));
      e = Math.max(e, Math.abs(L[i] - x), Math.abs(R[i] - x));
    }
    return e;
  };
  const steady = ({ L, R, errors }) => ({ peakL: peakOf(L.slice(-4096)), peakR: peakOf(R.slice(-4096)), errors });

  const r = {};
  {
    const out = await render({ frames: 8192 });
    let lat = -1;
    for (let d = 0; d < 512 && lat < 0; d++) {
      let ok = true;
      for (let i = d; i < 4096 && ok; i++) ok = out.L[i] === Math.fround(sine(i - d));
      if (ok) lat = d;
    }
    r.bypass = { lat, error: delayError(out), errors: out.errors };
  }
  r.typedPreamp = steady(await render({ frames: 48000, setup: (oa) => oa.setPreamp(-6.0206) }));
  r.paramsPreamp = steady(await render({ frames: 48000, setup: (oa) => oa.setParams({ preamp_db: -6.0206 }) }));
  r.paramsInvalid = steady(await render({ frames: 24000, setup: (oa) => oa.setParams({ eq_gains_db: 'loud' }) }));
  {
    const out = await render({ frames: 8192, inChannels: 1 });
    r.mono = { error: delayError(out), errors: out.errors };
  }
  {
    // 5.1 (L R C LFE SL SR) with only the centre carrying signal.
    const out = await render({ frames: 8192, inChannels: 6, fill: (c) => c === 2 });
    r.centre = { error: delayError(out, Math.SQRT1_2), ...steady(out) };
  }
  return r;
}

const near = (x, want, tol) => Math.abs(x - want) < tol;
let failed = false;
for (const name of selected) {
  const browser = await engines[name].launch();
  const page = await browser.newPage();
  const logs = [];
  page.on('console', (m) => { if (m.type() === 'error') logs.push(m.text()); });
  page.on('pageerror', (e) => logs.push(`pageerror: ${e.message}`));
  await page.goto(base);
  console.log(`\n== ${name} ${browser.version()}`);
  let r;
  try {
    r = await page.evaluate(suite);
  } catch (e) {
    console.log(`FAIL  suite: ${String(e.message).split('\n')[0]}`);
    failed = true;
  }
  await browser.close();
  if (r) {
    const checks = [
      ['transparent bypass: exact 95-frame delay', r.bypass.lat === 95 && r.bypass.error === 0 && !r.bypass.errors.length,
        `latency=${r.bypass.lat} maxError=${r.bypass.error}`],
      ['typed setPreamp(-6 dB) halves the level', near(r.typedPreamp.peakL, 0.25, 0.01) && near(r.typedPreamp.peakR, 0.25, 0.01) && !r.typedPreamp.errors.length,
        `peak L/R=${r.typedPreamp.peakL.toFixed(4)}/${r.typedPreamp.peakR.toFixed(4)}`],
      ['setParams({ preamp_db: -6 }) halves the level', near(r.paramsPreamp.peakL, 0.25, 0.01) && near(r.paramsPreamp.peakR, 0.25, 0.01) && !r.paramsPreamp.errors.length,
        `peak L/R=${r.paramsPreamp.peakL.toFixed(4)}/${r.paramsPreamp.peakR.toFixed(4)} errors=${JSON.stringify(r.paramsPreamp.errors)}`],
      ['invalid setParams is reported, level untouched', r.paramsInvalid.errors.length === 1 && /returned -1/.test(r.paramsInvalid.errors[0]) && near(r.paramsInvalid.peakL, 0.5, 0.01),
        `errors=${JSON.stringify(r.paramsInvalid.errors)} peak=${r.paramsInvalid.peakL.toFixed(4)}`],
      ['mono source reaches both channels unchanged', r.mono.error === 0 && !r.mono.errors.length, `maxError=${r.mono.error}`],
      ['5.1 centre-only folds to both channels at -3 dB', r.centre.error < 1e-4 && near(r.centre.peakL, 0.5 * Math.SQRT1_2, 0.005) && near(r.centre.peakR, 0.5 * Math.SQRT1_2, 0.005) && !r.centre.errors.length,
        `peak L/R=${r.centre.peakL.toFixed(4)}/${r.centre.peakR.toFixed(4)} maxError=${r.centre.error.toExponential(2)}`],
    ];
    for (const [label, ok, detail] of checks) {
      console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}  (${detail})`);
      if (!ok) failed = true;
    }
  }
  if (logs.length) { console.log(`FAIL  console errors: ${logs.join(' | ')}`); failed = true; }
}
server.close();
process.exit(failed ? 1 : 0);
