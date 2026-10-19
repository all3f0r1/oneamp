# Android: the Kotlin/JNI bridge

Crate `oneamp-android` (`cdylib`, `liboneamp_android.so`) + Kotlin
sources in `oneamp-android/kotlin/org/oneamp/`:

* `OneAmpProcessor` — the PCM chain over the host's blocks;
* `OneAmpPlayer` — the full engine; the host pulls its output;
* `AccessCallback` — resolves and renews remote access.

Build:

```sh
TC=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin
export CC_aarch64_linux_android=$TC/aarch64-linux-android24-clang AR_aarch64_linux_android=$TC/llvm-ar
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$TC/aarch64-linux-android24-clang
cargo build -p oneamp-android --release --target aarch64-linux-android
# → target/aarch64-linux-android/release/liboneamp_android.so (jniLibs/arm64-v8a)
```

The library needs only `libc`, `libm`, `libdl`; no cpal, no OpenSSL
(rustls). Keep the `org.oneamp` classes out of obfuscation
(`-keep class org.oneamp.** { native <methods>; *; }`).

## Lifetimes

Each Kotlin object owns one native handle, created in the constructor
and released by `close()` (`AutoCloseable`). `close()` waits for calls in
flight (read/write lock), later calls throw `IllegalStateException`;
`OneAmpPlayer.render` never waits — it returns silence while closing.
Arrays and direct buffers stay Kotlin's: native code borrows them only
for the call. `OneAmpProcessor` is single-threaded (the processing
thread); `OneAmpPlayer` is safe to call from several threads.

## Player protocol (JSON, version `OneAmpPlayer.protocolVersion` = 1)

Create: `OneAmpPlayer("""{"sampleRate":48000,"channels":2,"preloadLeadMs":10000}""", accessCallback)`.
The engine converts everything to that format (fixed output policy), so
transitions never need a new `AudioTrack`.

Commands (`send` returns the command id, -1 if the queue is full;
malformed JSON throws `IllegalArgumentException`):

```json
{"type":"load","paused":false,"media":{...}}
{"type":"resume"}  {"type":"pause"}  {"type":"stop"}
{"type":"seek","positionMs":61000}
{"type":"setNext","media":{...}}          // or "media": null
{"type":"setVolume","value":0.8}
{"type":"setRepeat","mode":"off|one|all"}
{"type":"settings","settings":{...}}       // oneamp_core::AudioSettings JSON
{"type":"routeChanged"}
```

Media (exactly one of `url`, `path`, `fd`):

```json
{"id":"item-42",
 "url":"https://…", "headers":{"…":"…"}, "accessCallback":true,
 "path":"/storage/…", "fd":57,
 "delivery":"auto|onDemand|live", "mimeType":"audio/flac", "extension":"flac",
 "durationMs":245000, "startMs":61000, "timelineOffsetMs":0, "seekable":null,
 "metadata":{"title":"…","artist":"…","album":"…","albumArtist":"…","trackNumber":3,"year":1994,"genre":"…"},
 "gain":{"trackGainDb":-7.2,"trackPeak":0.98,"albumGainDb":-6.5,"albumPeak":1.0},
 "context":"unknown|album|mixed"}
```

`fd`: a descriptor from `contentResolver.openFileDescriptor(uri, "r")!!.detachFd()`;
the player owns and closes it, and reopens it through `/proc/self/fd`
for each independent reader.

`accessCallback: true`: the player calls `AccessCallback.access(mediaId)`
and, after a 401, `renew(mediaId, reason, rejectedGeneration)` on its
decode threads. Both return `{"url":…, "headers":{…}, "expiresAtMs":…}`
or `{"error":{"category":"AccessExpired"|"AccessDenied","message":…}}`.

Events (`pollEvent(timeoutMs)`), each with a `type`: `commandCompleted`
(`id`, `instance`, `ok`, `error`), `trackLoaded` (`instance`, `track`),
`playing`, `paused`, `stopped`, `buffering`, `finished`, `position`
(`positionMs`, `durationMs`), `seeked`, `nextPrepared`, `nextFailed`,
`error` (`instance`, `error`), `icyMetadata`, `streamReconnecting`,
`streamReconnected`, `streamReconnectFailed`, `outputChanged`, `volume`,
`balance`, `equalizer`, `preamp`, `eqAuto`, `replayGain`, `loudness`,
`repeat`, `shuffle`, `outputDevice`, `eventsDropped`.
Errors: `{"category":"Network|Timeout|AccessExpired|AccessDenied|NotFound|ResourceChanged|InvalidResponse|Unsupported|Decode|Output|InvalidRequest|Cancelled|Internal","recoverable":bool,"message":"…"}`
— messages never contain a credential.

`snapshot()` returns the coherent state: `state`, `positionMs`,
`decodedPositionMs`, `durationMs`, `availableMs`, `capabilities`,
`source`, `output`, `processing`, `eq`, `next`, `lastError`,
`diagnostics`.

## Output and platform roles

The host owns the audio track and everything platform-level:

```kotlin
val player = OneAmpPlayer("""{"sampleRate":48000,"channels":2}""", access)
val track = AudioTrack.Builder()
    .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
        .setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_OUT_STEREO).build())
    .setTransferMode(AudioTrack.MODE_STREAM).build()
thread(name = "oneamp-out") {
    val buf = FloatArray(480 * 2)
    track.play()
    while (running) {
        player.render(buf, 480)
        track.write(buf, 0, buf.size, AudioTrack.WRITE_BLOCKING)
    }
}
```

* **Latency**: report the track's latency (from `AudioTimestamp`) with
  `setOutputLatencyNanos`, so `positionMs` is what is heard.
* **MediaSession** (Media3 `MediaSession` or `MediaSessionCompat`): the
  host maps transport controls to commands and events to the session's
  state and metadata. OneAmp does not touch it.
* **Audio focus**: on loss → `pause`; transient-can-duck → lower with
  `setVolume` (or the session's ducking); on gain → `resume`.
* **Foreground service**: the player lives in the host's
  `MediaSessionService`/foreground service; OneAmp's threads are plain
  native threads with no lifecycle of their own; `close()` on service
  destruction.
* **Interruptions and routes**: `ACTION_AUDIO_BECOMING_NOISY` → `pause`;
  `AudioDeviceCallback` / `AudioTrack.OnRoutingChangedListener` →
  `reportRouteChange()` (the engine rebuilds its output and resumes from
  the audible frame; recreate the `AudioTrack` if its format changed).

## Processing only, inside Media3

`OneAmpProcessor` fits a Media3 `AudioProcessor` handed to
`DefaultAudioSink.Builder().setAudioProcessors(arrayOf(…))`. What the
Media3 source (`DefaultAudioSink`) shows, and what follows:

* custom processors run **only for PCM** (`audio/raw`). In **offload**
  and **passthrough** the processing pipeline is empty: effects would be
  silently skipped. When the user asked for any effect, disable offload
  (`TrackSelectionParameters.AudioOffloadPreferences` mode disabled) and
  do not enable encoded passthrough; check
  `OneAmpProcessor.isBypassed` before allowing them back.
* custom processors run **before** Media3's own float/16-bit
  conversion, on the decoder's PCM encoding: accept
  `C.ENCODING_PCM_16BIT` and `C.ENCODING_PCM_FLOAT` (convert 16-bit to
  float in the processor), output `C.ENCODING_PCM_FLOAT`; throw
  `UnhandledAudioFormatException` for anything else (24/32-bit integer,
  big-endian) so the format is never passed through unprocessed by
  mistake.
* report the latency (`latencyFrames`) through
  `getDurationAfterProcessorApplied` where your Media3 version supports
  it; call `reset()` from `flush()`/`onFlush()` (seek) and
  `setMediaGain` when the media item changes; `flush(buffer)` at
  end-of-stream (`queueEndOfStream`).

A sketch (not compiled in this repository's CI — it needs the Android
SDK and Media3):

```kotlin
class OneAmpAudioProcessor(private val paramsJson: () -> String) : BaseAudioProcessor() {
    private var native: OneAmpProcessor? = null
    private var floats = FloatArray(0)

    override fun onConfigure(input: AudioFormat): AudioFormat {
        if (input.encoding != C.ENCODING_PCM_16BIT && input.encoding != C.ENCODING_PCM_FLOAT)
            throw UnhandledAudioFormatException(input)
        native?.close()
        native = OneAmpProcessor(input.sampleRate, input.channelCount).also { it.setParams(paramsJson()) }
        return AudioFormat(input.sampleRate, native!!.outputChannels, C.ENCODING_PCM_FLOAT)
    }

    override fun queueInput(inputBuffer: ByteBuffer) {
        val p = native ?: return
        val ch = inputAudioFormat.channelCount
        val samples = inputBuffer.remaining() / inputAudioFormat.bytesPerFrame * ch
        if (floats.size < samples) floats = FloatArray(samples)
        if (inputAudioFormat.encoding == C.ENCODING_PCM_16BIT) {
            for (i in 0 until samples) floats[i] = inputBuffer.short / 32768f
        } else {
            inputBuffer.asFloatBuffer().get(floats, 0, samples); inputBuffer.position(inputBuffer.limit())
        }
        val out = p.process(floats, samples / ch)
        replaceOutputBuffer(out * 4).asFloatBuffer().put(floats, 0, out)
    }

    override fun onQueueEndOfStream() {
        val p = native ?: return
        val tail = FloatArray(p.latencyFrames * p.outputChannels)
        val n = p.flush(tail)
        replaceOutputBuffer(n * 4).asFloatBuffer().put(tail, 0, n)
    }

    override fun onFlush() { native?.reset() }
    override fun onReset() { native?.close(); native = null }
}
```

## What is verified

See [matrix.md](matrix.md): the bridge is compiled for `aarch64` and
`x86_64` Android in CI, and executed on a desktop JVM (HotSpot) against
a host build — processor, player, local file and HTTP through
`AccessCallback`. It has **not** been run on ART, an emulator or a
device in this work (no KVM on the development machine).
