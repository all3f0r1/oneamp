// Headless example, no Android or UI dependency: runs on a desktop JVM
// against the host build of liboneamp_android, and is what CI executes.
//
//   kotlinc kotlin examples-kotlin -include-runtime -d example.jar
//   java -Djava.library.path=target/debug -jar example.jar <file-or-url>

import org.oneamp.AccessCallback
import org.oneamp.OneAmpPlayer
import org.oneamp.OneAmpProcessor
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.sin
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

fun processorDemo() {
    OneAmpProcessor(sampleRate = 48_000, channels = 2, maxFrames = 1024).use { p ->
        p.setParams("""{"eq_enabled": true, "eq_gains_db": [6,3,0,0,0,0,0,0,0,0], "preamp_db": -6.0}""")
        p.setMediaGain(trackDb = -3f)
        val tone = FloatArray(1024 * 2) { i -> 0.5f * sin(2 * PI * 440 * (i / 2) / 48_000).toFloat() }
        val block = FloatArray(tone.size)
        var out = 0
        repeat(10) {
            tone.copyInto(block)
            out = p.process(block, 1024)
        }
        val tail = FloatArray(1024 * 2)
        val t = p.flush(tail)
        check(out == 2048) { "processed $out samples" }
        check(t == p.latencyFrames * 2) { "tail $t" }
        println("processor: latency ${p.latencyFrames} frames, output ${p.outputChannels} ch, last block peak ${block.maxOf { abs(it) }}")
    }
}

/** Stands in for the app's token service: a short-lived URL per media. */
class DemoAccess(private val url: String) : AccessCallback {
    override fun access(mediaId: String) = """{"url": "$url"}"""
    override fun renew(mediaId: String, reason: String, rejectedGeneration: Long) =
        """{"error": {"category": "AccessExpired", "message": "demo cannot renew"}}"""
}

fun playerDemo(source: String) {
    val remote = source.startsWith("http://") || source.startsWith("https://")
    val media = if (remote) {
        """{"id": "demo-item", "url": "$source", "accessCallback": true, "metadata": {"title": "Demo"}}"""
    } else {
        """{"id": "demo-file", "path": "$source"}"""
    }
    OneAmpPlayer("""{"sampleRate": 48000, "channels": 2}""", if (remote) DemoAccess(source) else null).use { player ->
        // The "device": a thread pulling 10 ms blocks, like an AudioTrack loop.
        val running = AtomicBoolean(true)
        val rendered = AtomicLong()
        val device = Thread {
            val buf = FloatArray(480 * 2)
            while (running.get()) {
                rendered.addAndGet(player.render(buf, 480).toLong())
                Thread.sleep(2) // faster than real time: a test, not a speaker
            }
        }.apply { start() }

        val load = player.send("""{"type": "load", "media": $media}""")
        println("load queued as command $load")
        var done = false
        while (!done) {
            val event = player.pollEvent(5_000) ?: error("no event in 5 s: ${player.snapshot()}")
            when {
                event.contains("\"type\":\"trackLoaded\"") -> println("event: $event")
                event.contains("\"type\":\"commandCompleted\"") -> {
                    println("event: $event")
                    check(event.contains("\"ok\":true")) { "load failed: $event" }
                }
                event.contains("\"type\":\"finished\"") -> done = true
                event.contains("\"type\":\"error\"") -> error("playback error: $event")
            }
        }
        running.set(false)
        device.join()
        println("player: rendered ${rendered.get()} frames; snapshot ${player.snapshot().take(160)}…")
        check(rendered.get() > 0)
    }
}

fun main(args: Array<String>) {
    println("OneAmp protocol v${OneAmpPlayer.protocolVersion}")
    processorDemo()
    if (args.isNotEmpty()) playerDemo(args[0])
    println("ok")
}
