package org.oneamp

import java.util.concurrent.locks.ReentrantReadWriteLock
import kotlin.concurrent.read
import kotlin.concurrent.write

/**
 * The full OneAmp engine (sources, decoding, transitions, processing)
 * with its output pulled by the host: an `AudioTrack` write loop or an
 * AAudio callback calls [render].
 *
 * Messages are JSON (protocol version [protocolVersion]); see
 * `docs/embedding/android.md`. [send] returns once the command is
 * queued — its outcome arrives as a `commandCompleted` event with the
 * same id.
 *
 * The host keeps the platform roles: `MediaSession`, audio focus, the
 * foreground service, `ACTION_AUDIO_BECOMING_NOISY` — it turns those into
 * commands (`pause`, `resume`) and route changes ([reportRouteChange]).
 *
 * @param configJson `{"sampleRate": 48000, "channels": 2, "preloadLeadMs": 10000}`
 */
class OneAmpPlayer(configJson: String = "{}", accessCallback: AccessCallback? = null) : AutoCloseable {
    private val lock = ReentrantReadWriteLock()
    private var handle: Long = nativeCreate(configJson, accessCallback)

    private inline fun <T> use(block: (Long) -> T): T = lock.read {
        check(handle != 0L) { "OneAmpPlayer closed" }
        block(handle)
    }

    /** Queue a command; returns its id, or -1 when the queue is full. */
    fun send(commandJson: String): Long = use { nativeSend(it, commandJson) }

    /** Next event (JSON) or null after [timeoutMs]. */
    fun pollEvent(timeoutMs: Long): String? = use { nativePollEvent(it, timeoutMs) }

    /** Coherent state (JSON). */
    fun snapshot(): String = use { nativeSnapshot(it) }

    /**
     * Fill [buffer] with [frames] interleaved frames; returns the frames
     * carrying audio. Never blocks; returns 0 (silence) while closing.
     */
    fun render(buffer: FloatArray, frames: Int): Int {
        if (!lock.readLock().tryLock()) return 0
        try {
            return if (handle == 0L) 0 else nativeRender(handle, buffer, frames)
        } finally {
            lock.readLock().unlock()
        }
    }

    /** Output latency measured by the host (AudioTrack timestamps), for the audible position. */
    fun setOutputLatencyNanos(nanos: Long) = use { nativeSetOutputLatency(it, nanos) }

    /** Headphones unplugged, Bluetooth switched, …: the engine rebuilds its output in place. */
    fun reportRouteChange() = use { nativeReportRouteChange(it) }

    /** Stops the engine and every worker; waits for in-flight calls. */
    override fun close() = lock.write {
        if (handle != 0L) {
            nativeDestroy(handle)
            handle = 0L
        }
    }

    companion object {
        init { System.loadLibrary("oneamp_android") }
        val protocolVersion: Int get() = nativeProtocolVersion()
        @JvmStatic private external fun nativeCreate(config: String, access: AccessCallback?): Long
        @JvmStatic private external fun nativeDestroy(h: Long)
        @JvmStatic private external fun nativeSend(h: Long, command: String): Long
        @JvmStatic private external fun nativePollEvent(h: Long, timeoutMs: Long): String?
        @JvmStatic private external fun nativeSnapshot(h: Long): String
        @JvmStatic private external fun nativeRender(h: Long, buffer: FloatArray, frames: Int): Int
        @JvmStatic private external fun nativeSetOutputLatency(h: Long, nanos: Long)
        @JvmStatic private external fun nativeReportRouteChange(h: Long)
        @JvmStatic private external fun nativeProtocolVersion(): Int
    }
}
