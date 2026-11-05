// Main-thread helper: compile the module once, load the worklet, insert
// the processor between a source and a destination.
//
// Errors raised by the processor (invalid parameters, a message after
// disposal, ...) arrive through `onError(message)`; they are never
// silently dropped.

export async function createOneAmpNode(context, {
  wasmUrl = 'oneamp_wasm.wasm',
  workletUrl = 'oneamp-worklet.js',
  channels = 2,
  onError = (message) => console.error(`OneAmp: ${message}`),
} = {}) {
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  await context.audioWorklet.addModule(workletUrl);
  const node = new AudioWorkletNode(context, 'oneamp-processor', {
    numberOfInputs: 1,
    numberOfOutputs: 1,
    outputChannelCount: [2],
    channelCount: channels,
    channelCountMode: 'explicit',
    // 'speakers': Web Audio's up/down-mix rules map the source onto the
    // processor's layout (mono reaches both channels, a 5.1 centre is
    // folded into stereo). 'discrete' would drop or zero channels.
    channelInterpretation: 'speakers',
    processorOptions: { module, channels },
  });
  node.port.onmessage = (e) => {
    if (e.data?.type === 'error') onError(e.data.message);
  };
  node.onprocessorerror = () => onError('processor error (constructor or process threw)');
  const send = (m, transfer) => node.port.postMessage(m, transfer);
  const utf8 = new TextEncoder();
  return {
    node,
    setEqEnabled: (on) => send({ type: 'eqEnabled', on }),
    setEqBand: (band, db) => send({ type: 'eqBand', band, db }),
    setPreamp: (db) => send({ type: 'preamp', db }),
    setVolume: (value) => send({ type: 'volume', value }),
    setReplayGain: (mode, preventClipping = false) => send({ type: 'replayGain', mode, preventClipping }),
    setMediaGain: (gain) => send({ type: 'mediaGain', ...gain }),
    // Whole `DspParams` document. The JSON is encoded here because the
    // AudioWorkletGlobalScope has no TextEncoder, but applying it still
    // allocates inside wasm on the audio thread: use it for occasional
    // changes (initial state, preset load), and the typed setters above
    // for anything driven continuously.
    setParams: (params) => {
      const bytes = utf8.encode(JSON.stringify(params));
      send({ type: 'params', bytes }, [bytes.buffer]);
    },
    reset: () => send({ type: 'reset' }),
    dispose: () => send({ type: 'dispose' }),
  };
}
