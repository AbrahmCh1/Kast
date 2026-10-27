package io.github.abrahmch1.kast

import org.json.JSONObject
import java.nio.ByteBuffer

/**
 * Núcleo de Kast Link en Rust (link/src/android): descubre equipos, cifra las conexiones y, en pantallas grandes,
 * comparte esta pantalla. Los eventos y el vídeo llegan desde hilos de Rust.
 */
object Native {
    init {
        System.loadLibrary("kast_link")
    }

    /** Arranca el cliente y, con [host], el anfitrión (esta pantalla se puede ver y controlar desde otros equipos). */
    @JvmStatic external fun start(dir: String, name: String, host: Boolean): Boolean

    /** Comando en JSON (ver link/src/mobile.rs y link/src/android/host.rs). */
    @JvmStatic external fun command(json: String)

    /** Conecta con un equipo; [auth] = "password" o "pair" con su [secret]. Devuelve el número de sesión. */
    @JvmStatic external fun connect(id: String, monitor: Int, control: Boolean, auth: String?, secret: String?): Long

    @JvmStatic external fun keyframe(session: Long)
    @JvmStatic external fun mouseMove(session: Long, x: Int, y: Int)

    /** Botón: 0 izquierdo, 1 derecho, 2 central, 3 atrás, 4 adelante. */
    @JvmStatic external fun mouseButton(session: Long, button: Int, down: Boolean, x: Int, y: Int)

    /** Rueda en 1/120 de muesca (positivo: hacia arriba / a la derecha). */
    @JvmStatic external fun wheel(session: Long, dx: Int, dy: Int)

    /** Tecla por su nombre W3C (KeyA, Enter, ShiftLeft…). */
    @JvmStatic external fun key(session: Long, code: String, down: Boolean)
    @JvmStatic external fun text(session: Long, text: String)
    @JvmStatic external fun clipText(session: Long, text: String)

    /** Fotograma del codificador de esta pantalla (búfer directo de MediaCodec y sus flags). */
    @JvmStatic external fun hostFrame(buf: ByteBuffer, offset: Int, size: Int, flags: Int)

    @JvmStatic
    fun onEvent(json: String) {
        val ev = try { JSONObject(json) } catch (_: Exception) { return }
        KastApp.app.onEvent(ev)
    }

    @JvmStatic
    fun onFrame(session: Long, data: ByteArray, key: Boolean) {
        KastApp.app.frameSink?.invoke(session, data, key)
    }
}
