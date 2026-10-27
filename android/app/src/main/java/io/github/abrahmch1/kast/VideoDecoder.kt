package io.github.abrahmch1.kast

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import java.nio.ByteBuffer

/**
 * Decodifica el H.264 de una sesión (Annex B, con SPS/PPS en los fotogramas clave) y lo dibuja en [surface] en cuanto
 * sale, sin esperar. Si se atasca o falla, descarta hasta el siguiente fotograma clave y lo pide con [needKey].
 */
class VideoDecoder(private val surface: Surface, private val w: Int, private val h: Int, private val needKey: () -> Unit) {
    private val thread = HandlerThread("kast-decodificador").apply { start() }
    private val handler = Handler(thread.looper)
    private val lock = Object()
    private var codec: MediaCodec? = null
    private val inputs = ArrayDeque<Int>()
    private val queue = ArrayDeque<ByteArray>()
    private var waitKey = true
    private var pts = 0L
    @Volatile private var closed = false

    fun push(data: ByteArray, key: Boolean) {
        synchronized(lock) {
            if (closed) return
            if (waitKey && !key) return
            if (key) waitKey = false
            if (codec == null && !start(data)) { waitKey = true; return }
            // Muy atrasado (decodificador lento): se salta hasta el siguiente fotograma clave.
            if (queue.size > 20) {
                queue.clear()
                waitKey = true
                needKey()
                return
            }
            queue.addLast(data)
            feed()
        }
    }

    private fun start(first: ByteArray): Boolean {
        val nals = nalUnits(first)
        val sps = nals.firstOrNull { it.type == 7 } ?: return false
        val pps = nals.firstOrNull { it.type == 8 } ?: return false
        val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h).apply {
            setByteBuffer("csd-0", ByteBuffer.wrap(START + first.copyOfRange(sps.start, sps.end)))
            setByteBuffer("csd-1", ByteBuffer.wrap(START + first.copyOfRange(pps.start, pps.end)))
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            setInteger(MediaFormat.KEY_PRIORITY, 0)
        }
        return try {
            val c = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            c.setCallback(object : MediaCodec.Callback() {
                override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
                    synchronized(lock) { if (!closed) { inputs.addLast(index); feed() } }
                }

                override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
                    try { codec.releaseOutputBuffer(index, true) } catch (_: Exception) { }
                }

                override fun onError(codec: MediaCodec, e: MediaCodec.CodecException) {
                    Log.w("Kast", "decodificador: $e")
                    synchronized(lock) { reset() }
                    needKey()
                }

                override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {}
            }, handler)
            c.configure(fmt, surface, null, 0)
            c.start()
            codec = c
            true
        } catch (e: Exception) {
            Log.w("Kast", "no se pudo crear el decodificador: $e")
            false
        }
    }

    private fun feed() {
        val c = codec ?: return
        while (inputs.isNotEmpty() && queue.isNotEmpty()) {
            val i = inputs.removeFirst()
            val data = queue.removeFirst()
            try {
                val buf = c.getInputBuffer(i) ?: continue
                buf.clear()
                buf.put(data, 0, minOf(data.size, buf.remaining()))
                pts += 16_666
                c.queueInputBuffer(i, 0, buf.position(), pts, 0)
            } catch (e: Exception) {
                Log.w("Kast", "decodificador: $e")
                reset()
                needKey()
                return
            }
        }
    }

    /** Se descarta el decodificador: el siguiente fotograma clave crea otro. */
    private fun reset() {
        val c = codec ?: return
        codec = null
        inputs.clear()
        queue.clear()
        waitKey = true
        handler.post { try { c.stop() } catch (_: Exception) { }; c.release() }
    }

    fun release() {
        synchronized(lock) {
            closed = true
            reset()
        }
        thread.quitSafely()
    }

    private class Nal(val start: Int, val end: Int, val type: Int)

    private companion object {
        val START = byteArrayOf(0, 0, 0, 1)

        /** Unidades NAL de un Annex B (sin el código de inicio). */
        fun nalUnits(b: ByteArray): List<Nal> {
            val starts = ArrayList<Int>()
            var i = 0
            while (i + 3 <= b.size) {
                if (b[i].toInt() == 0 && b[i + 1].toInt() == 0 && b[i + 2].toInt() == 1) { starts.add(i + 3); i += 3 } else i++
            }
            return starts.mapIndexed { n, s ->
                var e = if (n + 1 < starts.size) starts[n + 1] - 3 else b.size
                while (e > s && b[e - 1].toInt() == 0) e--
                Nal(s, e, if (s < b.size) b[s].toInt() and 0x1f else 0)
            }
        }
    }
}
