package io.github.abrahmch1.kast

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.graphics.RectF
import android.os.Bundle
import android.os.SystemClock
import android.text.InputType
import android.view.Gravity
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.min

/**
 * Sesión con otro equipo: su pantalla y, con control, el ratón y el teclado. Con el dedo: tocar = clic, mantener =
 * clic derecho, arrastrar = arrastrar con el botón pulsado, dos dedos = rueda (y tocar con dos dedos = clic derecho).
 * Un ratón o un teclado conectados a este equipo funcionan tal cual.
 */
class SessionActivity : Activity() {
    companion object {
        fun open(ctx: Context, id: String, name: String, monitor: Int, control: Boolean, auth: String?, secret: String?) {
            ctx.startActivity(Intent(ctx, SessionActivity::class.java).putExtra("id", id).putExtra("name", name).putExtra("monitor", monitor)
                .putExtra("control", control).putExtra("auth", auth).putExtra("secret", secret))
        }
    }

    private val app get() = KastApp.app
    private lateinit var id: String
    private lateinit var peer: String
    private var monitor = 1
    private var wantControl = true
    private var session = 0L
    private var control = false
    private var displays = 0
    private var rw = 0
    private var rh = 0
    private var surface: SurfaceView? = null
    private var decoder: VideoDecoder? = null
    private lateinit var stage: FrameLayout
    private lateinit var waiting: LinearLayout
    private lateinit var waitTitle: TextView
    private lateinit var waitCode: TextView
    private lateinit var bar: LinearLayout
    private lateinit var title: TextView
    private lateinit var keys: KeyInput
    private var screenButton: View? = null
    private var keyboardButton: View? = null
    private var closing = false

    private val listener: (JSONObject) -> Unit = { ev -> if (ev.optString("ev") == "session" && ev.optLong("session") == session) onSession(ev) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        id = intent.getStringExtra("id").orEmpty()
        peer = intent.getStringExtra("name").orEmpty()
        monitor = intent.getIntExtra("monitor", 1)
        wantControl = intent.getBooleanExtra("control", true)

        title = label(peer, 14f, K.SUB)
        bar = hbox(dp(8)) {
            setPadding(dp(12), dp(6), dp(8), dp(6))
            setBackgroundColor(0xFF141414.toInt())
            addView(title, LinearLayout.LayoutParams(0, -2, 1f))
            keyboardButton = button("Teclado") { toggleKeyboard() }.also { addView(it) }
            screenButton = button("Pantalla") { chooseScreen() }.also { addView(it) }
            addView(button("Desconectar", danger = true) { finish() })
        }
        stage = FrameLayout(this).apply { setBackgroundColor(0xFF000000.toInt()) }
        waitTitle = label("Conectando con $peer…", 20f, bold = true).apply { gravity = Gravity.CENTER }
        waitCode = label("", 40f, K.ACCENT, bold = true).apply { gravity = Gravity.CENTER; letterSpacing = 0.1f }
        waiting = vbox(dp(12)) {
            gravity = Gravity.CENTER
            addView(waitTitle)
            addView(waitCode)
            addView(label("", 14f, K.SUB).apply { gravity = Gravity.CENTER; tag = "sub" })
        }
        stage.addView(waiting, FrameLayout.LayoutParams(-1, -1))
        keys = KeyInput(this)
        stage.addView(keys, FrameLayout.LayoutParams(1, 1))
        val touch = View(this)
        touch.setOnTouchListener { _, e -> touch(e); true }
        touch.setOnGenericMotionListener { _, e -> generic(e) }
        stage.addView(touch, FrameLayout.LayoutParams(-1, -1))
        stage.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> layoutVideo() }
        setContentView(vbox {
            setBackgroundColor(0xFF000000.toInt())
            addView(bar, LinearLayout.LayoutParams(-1, -2))
            addView(stage, LinearLayout.LayoutParams(-1, 0, 1f))
        })
        updateBar()
        immersive()
        app.listen(listener)
        app.frameSink = { s, data, key -> if (s == session) decoder?.push(data, key) }
        connect(intent.getStringExtra("auth"), intent.getStringExtra("secret"))
    }

    private fun immersive() {
        window.insetsController?.let {
            it.hide(WindowInsets.Type.statusBars() or WindowInsets.Type.navigationBars())
            it.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }

    private fun connect(auth: String?, secret: String?) {
        session = Native.connect(id, monitor, wantControl, auth, secret)
    }

    override fun onDestroy() {
        app.unlisten(listener)
        app.frameSink = null
        app.command("disconnect", "session" to session)
        decoder?.release()
        super.onDestroy()
    }

    // ------------------------------------------------------------ estado de la sesión

    private fun onSession(ev: JSONObject) {
        val sub = waiting.findViewWithTag<TextView>("sub")
        when (ev.optString("state")) {
            "connecting" -> { waitTitle.text = "Conectando con $peer…"; waitCode.text = "" }
            "waiting" -> {
                waitTitle.text = "Acepta en ${ev.optString("peer", peer)}"
                waitCode.text = ev.optString("code")
                sub.text = "Comprueba que allí se ve este mismo código"
            }
            "live" -> {
                control = ev.optBoolean("control")
                displays = ev.optJSONArray("displays")?.length() ?: 0
                rw = ev.optInt("w")
                rh = ev.optInt("h")
                waiting.visibility = View.GONE
                val note = ev.optString("note")
                if (note.isNotEmpty()) Toast.makeText(this, note, Toast.LENGTH_LONG).show()
                updateBar()
                showVideo()
            }
            "ended" -> {
                if (closing) return
                val err = ev.optString("error")
                if (ev.optBoolean("need_auth")) return askSecret(err)
                if (err.isNotEmpty()) Toast.makeText(this, err, Toast.LENGTH_LONG).show()
                finish()
            }
        }
    }

    /** El equipo sólo acepta con contraseña o código. */
    private fun askSecret(err: String) {
        val f = field("Contraseña o código de emparejamiento", password = true)
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(peer)
            .setMessage(err)
            .setView(vbox { setPadding(dp(20), dp(8), dp(20), 0); addView(f) })
            .setPositiveButton("Conectar") { _, _ ->
                val s = f.text.toString().trim()
                val auth = if (Regex("^[A-Za-z0-9]{4}-?[A-Za-z0-9]{4}$").matches(s)) "pair" else "password"
                connect(auth, s)
            }
            .setNegativeButton("Cancelar") { _, _ -> finish() }
            .setOnCancelListener { finish() }
            .show()
    }

    private fun updateBar() {
        title.text = if (displays > 1) "$peer · pantalla $monitor" else peer
        keyboardButton?.visibility = if (control) View.VISIBLE else View.GONE
        screenButton?.visibility = if (displays > 1) View.VISIBLE else View.GONE
    }

    private fun chooseScreen() {
        val items = (1..displays).map { "Pantalla $it" }.toTypedArray()
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(peer)
            .setSingleChoiceItems(items, monitor - 1) { d, i ->
                d.dismiss()
                if (i + 1 == monitor) return@setSingleChoiceItems
                // Otra pantalla del mismo equipo: entra sin volver a preguntar (mismo grupo de conexiones).
                closing = true
                app.command("disconnect", "session" to session)
                decoder?.release()
                decoder = null
                surface?.let { stage.removeView(it) }
                surface = null
                waiting.visibility = View.VISIBLE
                monitor = i + 1
                closing = false
                connect(null, null)
            }
            .show()
    }

    // ------------------------------------------------------------ vídeo

    private fun showVideo() {
        if (surface != null) return
        val sv = SurfaceView(this)
        sv.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
                decoder?.release()
                val s = session
                decoder = VideoDecoder(holder.surface, rw, rh) { Native.keyframe(s) }
                Native.keyframe(s)
            }
            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}
            override fun surfaceDestroyed(holder: SurfaceHolder) {
                decoder?.release()
                decoder = null
            }
        })
        surface = sv
        stage.addView(sv, 0, FrameLayout.LayoutParams(1, 1))
        layoutVideo()
    }

    /** Rectángulo de la imagen remota dentro de la vista (con franjas si la forma no coincide). */
    private fun videoRect(): RectF {
        if (rw <= 0 || rh <= 0 || stage.width == 0) return RectF(0f, 0f, stage.width.toFloat(), stage.height.toFloat())
        val s = min(stage.width.toFloat() / rw, stage.height.toFloat() / rh)
        val w = rw * s
        val h = rh * s
        val x = (stage.width - w) / 2
        val y = (stage.height - h) / 2
        return RectF(x, y, x + w, y + h)
    }

    private fun layoutVideo() {
        val sv = surface ?: return
        val r = videoRect()
        val lp = FrameLayout.LayoutParams(r.width().toInt(), r.height().toInt())
        lp.leftMargin = r.left.toInt()
        lp.topMargin = r.top.toInt()
        sv.layoutParams = lp
    }

    /** Punto de la vista -> píxel de la pantalla remota. */
    private fun remote(x: Float, y: Float): Pair<Int, Int> {
        val r = videoRect()
        val px = ((x - r.left) / r.width() * rw).toInt().coerceIn(0, (rw - 1).coerceAtLeast(0))
        val py = ((y - r.top) / r.height() * rh).toInt().coerceIn(0, (rh - 1).coerceAtLeast(0))
        return px to py
    }

    // ------------------------------------------------------------ dedos y ratón

    private val slop by lazy { ViewConfiguration.get(this).scaledTouchSlop.toFloat() }
    private var downX = 0f
    private var downY = 0f
    private var downAt = 0L
    private var moved = false
    private var dragging = false
    private var fingers = 0
    private var multi = false
    private var scrolled = false
    private var lastY = 0f
    private var lastX = 0f
    private var acc = 0f
    private var accX = 0f
    private val longPress = Runnable {
        if (!moved && fingers == 1) {
            val (x, y) = remote(downX, downY)
            click(1, x, y)
            moved = true
        }
    }

    private fun click(b: Int, x: Int, y: Int) {
        Native.mouseMove(session, x, y)
        Native.mouseButton(session, b, true, x, y)
        Native.mouseButton(session, b, false, x, y)
    }

    private fun touch(e: MotionEvent) {
        if (!control || rw == 0) return
        if (e.isFromSource(InputDevice.SOURCE_MOUSE)) return mouse(e)
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = e.x; downY = e.y; downAt = SystemClock.uptimeMillis()
                moved = false; dragging = false; fingers = 1; multi = false; scrolled = false
                stage.postDelayed(longPress, 500)
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                stage.removeCallbacks(longPress)
                fingers = e.pointerCount
                if (dragging) { val (x, y) = remote(e.getX(0), e.getY(0)); Native.mouseButton(session, 0, false, x, y); dragging = false }
                multi = true
                lastX = avg(e) { e.getX(it) }; lastY = avg(e) { e.getY(it) }
                acc = 0f; accX = 0f
            }
            MotionEvent.ACTION_MOVE -> {
                if (multi) {
                    // Dos dedos: rueda (120 = una muesca cada ~40 dp).
                    val ax = avg(e) { e.getX(it) }
                    val ay = avg(e) { e.getY(it) }
                    acc += (ay - lastY) / dp(40) * 120
                    accX += (ax - lastX) / dp(40) * 120
                    lastX = ax; lastY = ay
                    if (abs(acc) >= 30 || abs(accX) >= 30) {
                        scrolled = true
                        Native.wheel(session, -accX.toInt(), acc.toInt())
                        acc = 0f; accX = 0f
                    }
                    return
                }
                if (!moved && hypot(e.x - downX, e.y - downY) > slop) {
                    moved = true
                    stage.removeCallbacks(longPress)
                    val (x, y) = remote(downX, downY)
                    Native.mouseMove(session, x, y)
                    Native.mouseButton(session, 0, true, x, y)
                    dragging = true
                }
                if (dragging) { val (x, y) = remote(e.x, e.y); Native.mouseMove(session, x, y) }
            }
            MotionEvent.ACTION_POINTER_UP -> {
                if (!scrolled && SystemClock.uptimeMillis() - downAt < 400) {
                    val (x, y) = remote(downX, downY)
                    click(1, x, y)
                    scrolled = true
                }
            }
            MotionEvent.ACTION_UP -> {
                stage.removeCallbacks(longPress)
                val (x, y) = remote(e.x, e.y)
                when {
                    dragging -> Native.mouseButton(session, 0, false, x, y)
                    !moved && !multi -> click(0, x, y)
                }
                dragging = false
                fingers = 0
            }
            MotionEvent.ACTION_CANCEL -> {
                stage.removeCallbacks(longPress)
                if (dragging) { val (x, y) = remote(e.x, e.y); Native.mouseButton(session, 0, false, x, y) }
                dragging = false
            }
        }
    }

    private inline fun avg(e: MotionEvent, f: (Int) -> Float): Float = (0 until e.pointerCount).map(f).average().toFloat()

    /** Ratón conectado a este equipo: movimientos, botones y rueda tal cual. */
    private fun mouse(e: MotionEvent) {
        val (x, y) = remote(e.x, e.y)
        when (e.actionMasked) {
            MotionEvent.ACTION_MOVE -> Native.mouseMove(session, x, y)
            MotionEvent.ACTION_BUTTON_PRESS, MotionEvent.ACTION_BUTTON_RELEASE -> {
                val b = when (e.actionButton) {
                    MotionEvent.BUTTON_PRIMARY -> 0
                    MotionEvent.BUTTON_SECONDARY -> 1
                    MotionEvent.BUTTON_TERTIARY -> 2
                    MotionEvent.BUTTON_BACK -> 3
                    MotionEvent.BUTTON_FORWARD -> 4
                    else -> return
                }
                Native.mouseMove(session, x, y)
                Native.mouseButton(session, b, e.actionMasked == MotionEvent.ACTION_BUTTON_PRESS, x, y)
            }
        }
    }

    private fun generic(e: MotionEvent): Boolean {
        if (!control || rw == 0 || !e.isFromSource(InputDevice.SOURCE_CLASS_POINTER)) return false
        when (e.actionMasked) {
            MotionEvent.ACTION_HOVER_MOVE -> { val (x, y) = remote(e.x, e.y); Native.mouseMove(session, x, y) }
            MotionEvent.ACTION_SCROLL -> Native.wheel(session, (e.getAxisValue(MotionEvent.AXIS_HSCROLL) * 120).toInt(),
                (e.getAxisValue(MotionEvent.AXIS_VSCROLL) * 120).toInt())
            else -> return false
        }
        return true
    }

    // ------------------------------------------------------------ teclado

    private fun toggleKeyboard() {
        val imm = getSystemService(InputMethodManager::class.java)
        keys.requestFocus()
        imm.showSoftInput(keys, 0)
    }

    /** Teclado físico: cada tecla por su posición (como en el escritorio). Atrás cierra la sesión. */
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (event.keyCode == KeyEvent.KEYCODE_BACK) return super.dispatchKeyEvent(event)
        if (control && event.device?.isVirtual == false) {
            val code = Keys.toW3c[event.keyCode] ?: return super.dispatchKeyEvent(event)
            if (event.action == KeyEvent.ACTION_DOWN && event.repeatCount == 0 || event.action == KeyEvent.ACTION_UP) {
                Native.key(session, code, event.action == KeyEvent.ACTION_DOWN)
            } else if (event.action == KeyEvent.ACTION_DOWN) {
                Native.key(session, code, true)
            }
            return true
        }
        return super.dispatchKeyEvent(event)
    }

    private fun tap(code: String) {
        Native.key(session, code, true)
        Native.key(session, code, false)
    }

    /** Recibe el teclado en pantalla: el texto va tal cual; borrar y Entrar, como teclas. */
    private inner class KeyInput(ctx: Context) : View(ctx) {
        init {
            isFocusable = true
            isFocusableInTouchMode = true
        }

        override fun onCheckIsTextEditor() = true

        override fun onCreateInputConnection(out: EditorInfo): InputConnection {
            // Sin sugerencias ni composición: cada carácter llega en cuanto se escribe.
            out.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
            out.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_ACTION_NONE
            return object : BaseInputConnection(this, false) {
                override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
                    val t = text.toString()
                    if (t == "\n") tap("Enter") else if (t.isNotEmpty()) Native.text(session, t)
                    return true
                }

                override fun deleteSurroundingText(before: Int, after: Int): Boolean {
                    repeat(before.coerceAtMost(64)) { tap("Backspace") }
                    repeat(after.coerceAtMost(64)) { tap("Delete") }
                    return true
                }

                override fun sendKeyEvent(event: KeyEvent): Boolean {
                    val code = Keys.toW3c[event.keyCode] ?: return true
                    if (event.action == KeyEvent.ACTION_DOWN || event.action == KeyEvent.ACTION_UP) {
                        Native.key(session, code, event.action == KeyEvent.ACTION_DOWN)
                    }
                    return true
                }

                override fun performEditorAction(actionCode: Int): Boolean { tap("Enter"); return true }
            }
        }
    }
}
