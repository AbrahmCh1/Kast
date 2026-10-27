package io.github.abrahmch1.kast

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.graphics.PointF
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.min

/**
 * Control remoto de esta pantalla con el servicio de Accesibilidad: el ratón del visor se convierte en gestos táctiles
 * (clic = toque, arrastrar = deslizar, rueda = desplazar, botón derecho = Atrás) y el teclado se escribe en el campo
 * de texto activo (Android 13+), con Esc = Atrás y la tecla Windows/⌘ = Inicio.
 */
class ControlService : AccessibilityService() {
    companion object {
        @Volatile var instance: ControlService? = null
            private set
        /** Duración de cada tramo de un arrastre. */
        private const val SEGMENT_MS = 16L
        /** Un botón pulsado sin moverse empieza a «mantener pulsado» pasado este tiempo. */
        private const val HOLD_MS = 250L
    }

    private val h = Handler(Looper.getMainLooper())

    override fun onServiceConnected() {
        instance = this
        Log.i("Kast", "control remoto disponible")
        KastApp.app.command("perms", "input" to true)
        // Android vuelve a enlazar el servicio (y a abrir la app) al encender o si la app se cerró.
        KastApp.app.main.postDelayed({ KastApp.app.resumeSharing() }, 1500)
    }

    override fun onDestroy() {
        // Android puede crear el servicio nuevo antes de destruir el anterior (p. ej. al volver a enlazarlo).
        if (instance === this) {
            instance = null
            Log.i("Kast", "control remoto no disponible")
            KastApp.app.command("perms", "input" to false)
        }
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}

    /** Evento del visor (desde un hilo de Rust). */
    fun onInput(ev: JSONObject) {
        h.post { handle(ev) }
    }

    private fun handle(ev: JSONObject) {
        when (ev.optString("t")) {
            "move" -> drag(point(ev))
            "button" -> {
                val down = ev.optBoolean("down")
                when (ev.optInt("b")) {
                    0 -> if (down) press(point(ev)) else release(point(ev))
                    1, 3 -> if (down) performGlobalAction(GLOBAL_ACTION_BACK)
                    2 -> if (down) performGlobalAction(GLOBAL_ACTION_HOME)
                }
            }
            "wheel" -> wheel(point(ev), ev.optInt("dx"), ev.optInt("dy"))
            "key" -> key(ev.optString("code"), ev.optBoolean("down"))
            "text" -> text(ev.optString("text"))
            "end" -> { if (pressed) release(last); mods = 0; lonely = false }
        }
    }

    // ------------------------------------------------------------ ratón -> gestos

    /** Píxel de la imagen enviada -> píxel de la pantalla (la pantalla virtual centra la imagen si cambia la forma). */
    private fun point(ev: JSONObject): PointF {
        val (sw, sh) = KastApp.app.streamSize
        val (dw, dh) = realSize(this)
        val s = min(sw.toFloat() / dw, sh.toFloat() / dh)
        val ox = (sw - dw * s) / 2
        val oy = (sh - dh * s) / 2
        val x = ((ev.optInt("x") - ox) / s).coerceIn(0f, dw - 1f)
        val y = ((ev.optInt("y") - oy) / s).coerceIn(0f, dh - 1f)
        return PointF(x, y)
    }

    private var pressed = false
    private var downAt = 0L
    private var downPos = PointF()
    /** Trazo en curso (botón pulsado y ya tocando la pantalla). */
    private var stroke: GestureDescription.StrokeDescription? = null
    private var last = PointF()
    private var target: PointF? = null
    private var lift = false
    private var busy = false
    private val hold = Runnable { if (pressed && stroke == null) begin() }

    private fun press(p: PointF) {
        cancelScroll()
        pressed = true
        downAt = SystemClock.uptimeMillis()
        downPos = p
        last = p
        target = null
        h.postDelayed(hold, HOLD_MS)
    }

    private fun drag(p: PointF) {
        if (!pressed) return
        target = p
        if (stroke == null) {
            if (hypot(p.x - downPos.x, p.y - downPos.y) > 8) begin()
        } else {
            pump()
        }
    }

    private fun release(p: PointF) {
        if (!pressed) return
        pressed = false
        h.removeCallbacks(hold)
        if (stroke == null) {
            // Clic: un toque de lo que duró (pulsación larga si se mantuvo).
            val ms = (SystemClock.uptimeMillis() - downAt).coerceIn(1, 3000)
            dispatch(GestureDescription.StrokeDescription(Path().apply { moveTo(downPos.x, downPos.y) }, 0, ms), null)
            return
        }
        target = p
        lift = true
        pump()
    }

    /** Empieza a tocar donde se pulsó el botón (y sigue mientras siga pulsado). */
    private fun begin() {
        val to = target ?: downPos
        val path = Path().apply { moveTo(downPos.x, downPos.y); lineTo(to.x, to.y) }
        val s = GestureDescription.StrokeDescription(path, 0, SEGMENT_MS, true)
        stroke = s
        last = to
        target = null
        dispatch(s, s)
    }

    /** Siguiente tramo del trazo (uno cada vez: los movimientos se juntan mientras el anterior se ejecuta). */
    private fun pump() {
        val s = stroke ?: return
        if (busy) return
        val to = target ?: if (lift) last else return
        val end = lift
        val path = Path().apply { moveTo(last.x, last.y); lineTo(to.x, to.y) }
        val next = s.continueStroke(path, 0, SEGMENT_MS, !end)
        last = to
        target = null
        if (end) {
            stroke = null
            lift = false
        } else {
            stroke = next
        }
        dispatch(next, if (end) null else next)
    }

    private fun dispatch(s: GestureDescription.StrokeDescription, cont: GestureDescription.StrokeDescription?) {
        busy = true
        val ok = dispatchGesture(GestureDescription.Builder().addStroke(s).build(), object : GestureResultCallback() {
            override fun onCompleted(g: GestureDescription?) { done(false) }
            override fun onCancelled(g: GestureDescription?) { done(cont != null && stroke == cont) }
        }, h)
        if (!ok) done(cont != null)
    }

    private fun done(broken: Boolean) {
        busy = false
        // Otro gesto (p. ej. del sistema) interrumpió el arrastre: se da por terminado.
        if (broken) { stroke = null; lift = false; target = null; return }
        if (stroke != null) pump() else nextScroll()
    }

    // ------------------------------------------------------------ rueda -> desplazar

    private var scrollAt = PointF()
    private var scrollX = 0f
    private var scrollY = 0f

    private fun cancelScroll() { scrollX = 0f; scrollY = 0f }

    private fun wheel(p: PointF, dx: Int, dy: Int) {
        if (pressed || stroke != null) return
        val (dw, dh) = realSize(this)
        // Una muesca (120) desplaza ~1/14 de la pantalla.
        scrollAt = p
        scrollX += dx / 120f * dw / 14
        scrollY += dy / 120f * dh / 14
        if (!busy) nextScroll()
    }

    /** Desliza lo acumulado y se detiene antes de soltar (sin inercia, como una rueda). */
    private fun nextScroll() {
        if (abs(scrollX) < 4 && abs(scrollY) < 4) return
        val (dw, dh) = realSize(this)
        val from = scrollAt
        val tx = (from.x + scrollX).coerceIn(1f, dw - 2f)
        val ty = (from.y + scrollY).coerceIn(1f, dh - 2f)
        scrollX = 0f
        scrollY = 0f
        if (abs(tx - from.x) < 2 && abs(ty - from.y) < 2) return
        val move = GestureDescription.StrokeDescription(Path().apply { moveTo(from.x, from.y); lineTo(tx, ty) }, 0, 90, true)
        val rest = move.continueStroke(Path().apply { moveTo(tx, ty) }, 0, 60, false)
        busy = true
        dispatchGesture(GestureDescription.Builder().addStroke(move).build(), object : GestureResultCallback() {
            override fun onCompleted(g: GestureDescription?) {
                dispatchGesture(GestureDescription.Builder().addStroke(rest).build(), object : GestureResultCallback() {
                    override fun onCompleted(g: GestureDescription?) { done(false) }
                    override fun onCancelled(g: GestureDescription?) { done(false) }
                }, h)
            }
            override fun onCancelled(g: GestureDescription?) { done(false) }
        }, h)
    }

    // ------------------------------------------------------------ teclado

    /** Modificadores pulsados (KeyEvent.META_*). */
    private var mods = 0
    /** La tecla Windows/⌘ se pulsó sola (al soltarla: Inicio). */
    private var lonely = false

    private fun key(code: String, down: Boolean) {
        val kc = Keys.toAndroid[code] ?: return
        val meta = Keys.meta(kc)
        if (meta != 0) {
            mods = if (down) mods or meta else mods and meta.inv()
            if (kc == KeyEvent.KEYCODE_META_LEFT || kc == KeyEvent.KEYCODE_META_RIGHT) {
                if (down) lonely = true else if (lonely) { lonely = false; performGlobalAction(GLOBAL_ACTION_HOME) }
                return
            }
        } else {
            lonely = false
        }
        if (kc == KeyEvent.KEYCODE_ESCAPE) {
            if (down) performGlobalAction(GLOBAL_ACTION_BACK)
            return
        }
        val ic = if (Build.VERSION.SDK_INT >= 33) inputMethod?.currentInputConnection else null
        if (ic != null) {
            val t = SystemClock.uptimeMillis()
            ic.sendKeyEvent(KeyEvent(t, t, if (down) KeyEvent.ACTION_DOWN else KeyEvent.ACTION_UP, kc, 0, mods,
                KeyCharacterMap.VIRTUAL_KEYBOARD, 0, KeyEvent.FLAG_FROM_SYSTEM, InputDevice.SOURCE_KEYBOARD))
            return
        }
        // Sin campo de texto: las flechas y Entrar mueven el foco (Android 13+).
        if (!down || Build.VERSION.SDK_INT < 33) return
        val action = when (kc) {
            KeyEvent.KEYCODE_DPAD_UP -> GLOBAL_ACTION_DPAD_UP
            KeyEvent.KEYCODE_DPAD_DOWN -> GLOBAL_ACTION_DPAD_DOWN
            KeyEvent.KEYCODE_DPAD_LEFT -> GLOBAL_ACTION_DPAD_LEFT
            KeyEvent.KEYCODE_DPAD_RIGHT -> GLOBAL_ACTION_DPAD_RIGHT
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> GLOBAL_ACTION_DPAD_CENTER
            else -> return
        }
        performGlobalAction(action)
    }

    /** Texto ya compuesto (con la distribución del teclado del visor, acentos incluidos). */
    private fun text(t: String) {
        if (t.isEmpty()) return
        if (Build.VERSION.SDK_INT >= 33) {
            val ic = inputMethod?.currentInputConnection
            if (ic != null) { ic.commitText(t, 1, null); return }
        }
        // Android 12 y anteriores: se añade al campo con el foco.
        val node = rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return
        val now = node.text?.toString().orEmpty()
        val start = node.textSelectionStart.takeIf { it >= 0 } ?: now.length
        val end = node.textSelectionEnd.takeIf { it >= 0 } ?: now.length
        val next = now.substring(0, min(start, end)) + t + now.substring(maxOf(start, end).coerceAtMost(now.length))
        node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, next)
        })
    }
}
