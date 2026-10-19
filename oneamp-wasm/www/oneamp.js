// Main-thread helper: compile the module once, load the worklet, insert
// the processor between a source and a destination.

export async function createOneAmpNode(context, {
  wasmUrl = 'oneamp_wasm.wasm',
  workletUrl = 'oneamp-worklet.js',
  channels = 2,
} = {}) {
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  await context.audioWorklet.addModule(workletUrl);
  const node = new AudioWorkletNode(context, 'oneamp-processor', {
    numberOfInputs: 1,
    numberOfOutputs: 1,
    outputChannelCount: [2],
    channelCount: channels,
    channelCountMode: 'explicit',
    channelInterpretation: 'discrete',
    processorOptions: { module, channels },
  });
  const send = (m) => node.port.postMessage(m);
  return {
    node,
    setEqEnabled: (on) => send({ type: 'eqEnabled', on }),
    setEqBand: (band, db) => send({ type: 'eqBand', band, db }),
    setPreamp: (db) => send({ type: 'preamp', db }),
    setVolume: (value) => send({ type: 'volume', value }),
    setReplayGain: (mode, preventClipping = false) => send({ type: 'replayGain', mode, preventClipping }),
    setMediaGain: (gain) => send({ type: 'mediaGain', ...gain }),
    setParams: (params) => send({ type: 'params', params }),
    reset: () => send({ type: 'reset' }),
    dispose: () => send({ type: 'dispose' }),
  };
}
