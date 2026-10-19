package org.oneamp

import java.nio.ByteBuffer
import java.util.concurrent.locks.ReentrantReadWriteLock
import kotlin.concurrent.read
import kotlin.concurrent.write

/**
 * OneAmp's PCM chain (EQ, preamp, ReplayGain, loudness, balance, mono,
 * limiter) over blocks of interleaved float PCM the host already decoded.
 *
 * Threading: one thread processes (the host's audio/processing thread).
 * Parameter setters may be called from that same thread between blocks.
 * [close] waits for an in-flight call to return, then frees the native
 * state; any later call throws [IllegalStateException].
 *
 * @param maxFrames largest block [process] receives; sizes the native
 *   scratch so processing never allocates.
 */
class OneAmpProcessor(sampleRate: Int, val channels: Int, maxFrames: Int = 4096) : AutoCloseable {
    private val lock = ReentrantReadWriteLock()
    private var handle: Long = nativeCreate(sampleRate, channels, maxFrames)

    private inline fun <T> use(block: (Long) -> T): T = lock.read {
        check(handle != 0L) { "OneAmpProcessor closed" }
        block(handle)
    }

    /** In place; returns the output sample count (fewer for surround: folded to stereo). */
    fun process(buffer: FloatArray, frames: Int): Int = use { nativeProcess(it, buffer, frames) }

    /** Zero-copy over a direct, native-order float [ByteBuffer] (Media3 `AudioProcessor`). */
    fun process(buffer: ByteBuffer, frames: Int): Int = use { nativeProcessDirect(it, buffer, frames) }

    /** A complete `DspParams` JSON document; missing fields take defaults. */
    fun setParams(json: String) = use { nativeSetParams(it, json) }

    /**
     * Normalisation of the media about to play (dB; peaks linear; NaN =
     * unknown). [context]: 0 unknown, 1 album in order, 2 mixed/shuffle.
     */
    fun setMediaGain(
        trackDb: Float = Float.NaN, trackPeak: Float = Float.NaN,
        albumDb: Float = Float.NaN, albumPeak: Float = Float.NaN, context: Int = 0,
    ) = use { nativeSetMediaGain(it, trackDb, trackPeak, albumDb, albumPeak, context) }

    /** After a seek or a media change. */
    fun reset() = use { nativeReset(it) }

    /** End of stream: writes the held-back tail; returns its sample count. */
    fun flush(buffer: FloatArray): Int = use { nativeFlush(it, buffer) }

    val latencyFrames: Int get() = use { nativeLatencyFrames(it) }
    val outputChannels: Int get() = use { nativeOutputChannels(it) }
    val isBypassed: Boolean get() = use { nativeIsBypassed(it) }

    override fun close() = lock.write {
        if (handle != 0L) {
            nativeDestroy(handle)
            handle = 0L
        }
    }

    private companion object {
        init { System.loadLibrary("oneamp_android") }
        @JvmStatic external fun nativeCreate(rate: Int, channels: Int, maxFrames: Int): Long
        @JvmStatic external fun nativeDestroy(h: Long)
        @JvmStatic external fun nativeProcess(h: Long, buffer: FloatArray, frames: Int): Int
        @JvmStatic external fun nativeProcessDirect(h: Long, buffer: ByteBuffer, frames: Int): Int
        @JvmStatic external fun nativeSetParams(h: Long, json: String)
        @JvmStatic external fun nativeSetMediaGain(h: Long, trackDb: Float, trackPeak: Float, albumDb: Float, albumPeak: Float, context: Int)
        @JvmStatic external fun nativeReset(h: Long)
        @JvmStatic external fun nativeFlush(h: Long, buffer: FloatArray): Int
        @JvmStatic external fun nativeLatencyFrames(h: Long): Int
        @JvmStatic external fun nativeOutputChannels(h: Long): Int
        @JvmStatic external fun nativeIsBypassed(h: Long): Boolean
    }
}
