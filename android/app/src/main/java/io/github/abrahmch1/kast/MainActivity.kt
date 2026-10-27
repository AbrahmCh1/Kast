package io.github.abrahmch1.kast

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionConfig
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import android.text.format.DateUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.CheckBox
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject

/**
 * Pantalla principal: los equipos de la red (para verlos y controlarlos) y, en pantallas grandes, «Esta pantalla»
 * (compartirla, permisos, emparejamiento, acceso desatendido y quién tiene acceso).
 */
class MainActivity : Activity() {
    companion object {
        private const val REQ_SHARE = 1
    }

    private val app get() = KastApp.app
    private lateinit var hostCol: LinearLayout
    private lateinit var devCol: LinearLayout
    private var pairDeadline = 0L
    private var pairLabel: TextView? = null
    /** Pantalla elegida de cada equipo (con varias). */
    private val monitorOf = HashMap<String, Int>()
    private var autoShare = false

    private val listener: (JSONObject) -> Unit = { ev ->
        when (ev.optString("ev")) {
            "host" -> renderHost()
            "devices" -> renderDevices()
        }
    }

    private val tick = object : Runnable {
        override fun run() {
            pairLabel?.let { l ->
                val left = ((pairDeadline - SystemClock.elapsedRealtime()) / 1000).coerceAtLeast(0)
                l.text = "Caduca en ${left / 60}:${"%02d".format(left % 60)} · un solo uso"
            }
            app.main.postDelayed(this, 1000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        hostCol = vbox(dp(14))
        devCol = vbox(dp(14))
        val pad = dp(if (app.large) 28 else 16)
        fun scroll(v: View) = ScrollView(this).apply {
            isFillViewport = true
            addView(v.apply { setPadding(pad, pad, pad, pad) })
        }
        val content: View = if (app.large) {
            hbox {
                addView(scroll(hostCol), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
                addView(scroll(devCol), LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1.3f))
            }
        } else {
            scroll(devCol)
        }
        setContentView(FrameLayout(this).apply {
            setBackgroundColor(K.BG)
            addView(content, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        })
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 2)
        }
        autoShare = intent?.getBooleanExtra("autoshare", false) == true
        if (autoShare && ShareService.instance == null) startShare()
    }

    override fun onNewIntent(intent: Intent?) {
        super.onNewIntent(intent)
        if (intent?.getBooleanExtra("autoshare", false) == true && ShareService.instance == null) {
            autoShare = true
            startShare()
        }
    }

    override fun onResume() {
        super.onResume()
        app.listen(listener)
        renderHost()
        renderDevices()
        app.command("refresh")
        app.command("perms", "input" to (ControlService.instance != null))
        app.command("host_state")
        app.main.post(tick)
    }

    override fun onPause() {
        app.unlisten(listener)
        app.main.removeCallbacks(tick)
        super.onPause()
    }

    // ------------------------------------------------------------ compartir

    private fun startShare() {
        val mpm = getSystemService(MediaProjectionManager::class.java)
        val i = if (Build.VERSION.SDK_INT >= 34) mpm.createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())
                else mpm.createScreenCaptureIntent()
        startActivityForResult(i, REQ_SHARE)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQ_SHARE) return
        if (resultCode == RESULT_OK && data != null) {
            ShareService.start(this, resultCode, data)
            // Al arrancar el equipo se comparte sola: no hace falta dejar la app delante.
            if (autoShare) moveTaskToBack(true)
        } else {
            Toast.makeText(this, "Sin permiso para capturar la pantalla", Toast.LENGTH_SHORT).show()
        }
        autoShare = false
        app.main.postDelayed({ renderHost() }, 500)
    }

    // ------------------------------------------------------------ esta pantalla

    private fun renderHost() {
        if (!app.large) return
        hostCol.removeAllViews()
        pairLabel = null
        val h = app.host
        val me = h?.optJSONObject("me")
        val sharing = h?.optBoolean("sharing") == true && ShareService.instance != null
        val sessions = h?.optJSONArray("sessions").objects()
        hostCol.addView(label("Esta pantalla", 13f, K.SUB, bold = true))
        hostCol.addView(card {
            addView(label(me?.optString("name") ?: app.deviceName(), 22f, bold = true))
            addView(hbox(dp(8)) {
                addView(label("ID ${me?.optString("short_id") ?: "…"}", 15f, K.SUB))
                addView(button("Copiar") {
                    getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("ID", me?.optString("short_id").orEmpty()))
                    Toast.makeText(this@MainActivity, "ID copiado", Toast.LENGTH_SHORT).show()
                })
            })
            val status = when {
                !sharing -> "Sin compartir: los demás equipos no pueden verla."
                sessions.isEmpty() -> "Compartiendo. Desde Kast en tu PC: Dispositivos → ${me?.optString("name")} → Ver o Controlar."
                else -> "Compartiendo con " + sessions.joinToString { it.optString("peer") + if (it.optBoolean("control")) " (controla)" else " (ve)" }
            }
            addView(label(status, 14f, if (sharing) K.OK else K.SUB))
            addView(if (sharing) button("Dejar de compartir", danger = true) { ShareService.stop(this@MainActivity); app.main.postDelayed({ renderHost() }, 400) }
                    else button("Compartir esta pantalla", primary = true) { startShare() })
            for (s in sessions) {
                addView(hbox(dp(10)) {
                    addView(label("${s.optString("peer")} · ${if (s.optBoolean("control")) "ve y controla" else "sólo ve"}", 14f), LinearLayout.LayoutParams(0, -2, 1f))
                    addView(button("Desconectar", danger = true) { app.command("end", "session" to s.optLong("session")) })
                })
            }
        })

        hostCol.addView(label("Permisos", 13f, K.SUB, bold = true))
        hostCol.addView(card {
            val control = ControlService.instance != null
            addView(permRow("Control con teclado y ratón", if (control) "Activado" else "Activa «Kast» en Accesibilidad para que puedan controlarla", control) {
                startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            })
            val overlay = Settings.canDrawOverlays(this@MainActivity)
            addView(permRow("Avisos de conexión encima de otras apps", if (overlay) "Activado" else "Para aceptar conexiones aunque Kast no esté abierta", overlay) {
                startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            })
        })

        hostCol.addView(label("Acceso", 13f, K.SUB, bold = true))
        val access = h?.optJSONObject("access")
        hostCol.addView(card {
            addView(label("Emparejar un dispositivo", 16f, bold = true))
            val pairing = h?.optJSONObject("pairing")
            if (pairing != null) {
                pairDeadline = SystemClock.elapsedRealtime() + pairing.optLong("secs") * 1000
                addView(label("En tu PC o Mac: Kast → Dispositivos → ${me?.optString("name")} → «Contraseña o código», con este código:", 14f, K.SUB))
                addView(label(pairing.optString("code"), 34f, K.ACCENT, bold = true).apply { letterSpacing = 0.08f })
                pairLabel = label("", 13f, K.FAINT).also { addView(it) }
                addView(label(if (pairing.optBoolean("control")) "Podrá ver y controlar sin preguntar" else "Podrá ver sin preguntar", 13f, K.SUB))
                addView(button("Cancelar") { app.command("pair_cancel") })
            } else {
                addView(label("Da acceso permanente a un dispositivo con un código de un solo uso.", 14f, K.SUB))
                addView(hbox(dp(8)) {
                    addView(button("Crear código (ver y controlar)", primary = true) { app.command("pair_start", "control" to true) })
                    addView(button("Sólo ver") { app.command("pair_start", "control" to false) })
                })
            }
        })
        hostCol.addView(card {
            val unattended = access?.optBoolean("unattended") == true
            addView(label("Acceso desatendido", 16f, bold = true))
            addView(label(if (unattended) "Activado: se puede entrar con la contraseña sin que nadie acepte (" +
                (if (access?.optBoolean("unattended_control") == true) "ver y controlar" else "sólo ver") + ")."
                else "Entrar con una contraseña aunque no haya nadie delante de la pantalla.", 14f, if (unattended) K.OK else K.SUB))
            addView(hbox(dp(8)) {
                addView(button(if (unattended) "Cambiar contraseña" else "Activar con contraseña", primary = !unattended) { passwordDialog(unattended) })
                if (unattended) addView(button("Desactivar", danger = true) { app.command("access_set", "unattended" to false) })
            })
        })
        hostCol.addView(card {
            val reject = access?.optString("unknown") == "reject"
            addView(label("Dispositivos desconocidos", 16f, bold = true))
            addView(label(if (reject) "Se rechazan: sólo entran los emparejados o con la contraseña." else "Se pregunta en esta pantalla, con un código para comprobar.", 14f, K.SUB))
            addView(hbox(dp(8)) {
                addView(button("Preguntar", primary = !reject) { app.command("access_set", "unknown" to "ask") })
                addView(button("Rechazar", primary = reject) { app.command("access_set", "unknown" to "reject") })
            })
        })

        val trusted = h?.optJSONArray("trusted").objects()
        if (trusted.isNotEmpty()) {
            hostCol.addView(label("Dispositivos con acceso", 13f, K.SUB, bold = true))
            hostCol.addView(card {
                for (t in trusted) {
                    addView(hbox(dp(10)) {
                        addView(vbox {
                            addView(label(t.optString("name"), 15f, bold = true))
                            val seen = t.optLong("last_seen")
                            addView(label(listOf(platformName(t.optString("platform")), "ID " + t.optString("short_id"),
                                if (t.optBoolean("control")) "ver y controlar" else "sólo ver",
                                "último acceso: " + if (seen > 0) DateUtils.getRelativeTimeSpanString(seen * 1000) else "nunca")
                                .filter { it.isNotBlank() }.joinToString(" · "), 13f, K.SUB))
                        }, LinearLayout.LayoutParams(0, -2, 1f))
                        addView(button("Quitar acceso", danger = true) { app.command("revoke", "key" to t.optString("key")) })
                    })
                }
            })
        }
    }

    private fun permRow(title: String, sub: String, ok: Boolean, fix: () -> Unit) = hbox(dp(10)) {
        addView(vbox {
            addView(label(title, 15f, bold = true))
            addView(label(sub, 13f, if (ok) K.OK else K.SUB))
        }, LinearLayout.LayoutParams(0, -2, 1f))
        if (!ok) addView(button("Activar", primary = true) { fix() })
    }

    private fun passwordDialog(change: Boolean) {
        val pw = field("Contraseña (mínimo 8 caracteres)", password = true)
        val pw2 = field("Repite la contraseña", password = true)
        val control = CheckBox(this).apply {
            text = "Permitir controlar (si no, sólo ver)"
            setTextColor(K.TEXT)
            isChecked = app.host?.optJSONObject("access")?.optBoolean("unattended_control") ?: true
        }
        val box = vbox(dp(8)) { setPadding(dp(20), dp(8), dp(20), 0); addView(pw); addView(pw2); addView(control) }
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(if (change) "Cambiar la contraseña" else "Acceso desatendido")
            .setView(box)
            .setPositiveButton("Guardar") { _, _ ->
                val a = pw.text.toString()
                when {
                    a.length < 8 -> Toast.makeText(this, "La contraseña debe tener al menos 8 caracteres", Toast.LENGTH_LONG).show()
                    a != pw2.text.toString() -> Toast.makeText(this, "Las contraseñas no coinciden", Toast.LENGTH_LONG).show()
                    else -> app.command("access_set", "password" to a, "unattended_control" to control.isChecked)
                }
            }
            .setNegativeButton("Cancelar", null)
            .show()
    }

    // ------------------------------------------------------------ equipos

    private fun renderDevices() {
        devCol.removeAllViews()
        devCol.addView(hbox(dp(8)) {
            addView(vbox {
                addView(label("Kast", 26f, bold = true))
                addView(label("Ve y controla tus equipos de la red", 14f, K.SUB))
            }, LinearLayout.LayoutParams(0, -2, 1f))
            addView(button("Conectar por ID") { idDialog() })
            addView(button("Agregar por IP") { ipDialog() })
        })
        val list = app.devices
        if (list.isEmpty()) {
            devCol.addView(card {
                addView(label("Buscando equipos en tu red…", 16f, bold = true))
                addView(label("Abre Kast en tu PC o Mac (pestaña Dispositivos) en la misma red. Si no aparece, agrégalo por su IP.", 14f, K.SUB))
            })
        }
        for (d in list) devCol.addView(deviceCard(d))
    }

    private fun deviceCard(d: JSONObject) = card {
        val id = d.optString("id")
        val online = d.optBoolean("online")
        val sharing = d.optBoolean("sharing")
        val displays = d.optJSONArray("displays")?.length() ?: 0
        addView(label(d.optString("name"), 17f, bold = true))
        addView(label(listOf(platformName(d.optString("platform")), d.optString("short_id").let { if (it.isNotEmpty()) "ID $it" else "" },
            if (displays > 0) "$displays pantalla${if (displays == 1) "" else "s"}" else "",
            when { !online -> "sin conexión"; !sharing -> "no comparte sus pantallas"; else -> "en línea" })
            .filter { it.isNotBlank() }.joinToString(" · "), 13f, if (online && sharing) K.SUB else K.FAINT))
        if (displays > 1) {
            addView(hbox(dp(6)) {
                addView(label("Pantalla", 13f, K.SUB))
                val sel = monitorOf[id] ?: 1
                for (m in 1..displays) addView(button("$m", primary = m == sel) { monitorOf[id] = m; renderDevices() })
            })
        }
        addView(hbox(dp(8)) {
            addView(button("Controlar", primary = true) { open(d, control = true) })
            addView(button("Ver") { open(d, control = false) })
            addView(spring())
            addView(button("Más…") { more(d) })
        })
        alpha = if (online) 1f else 0.6f
    }

    private fun more(d: JSONObject) {
        val items = arrayOf("Conectar con contraseña…", "Emparejar con código…", "Olvidar")
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(d.optString("name"))
            .setItems(items) { _, i ->
                when (i) {
                    0 -> secretDialog(d.optString("id"), d.optString("name"), "password", monitorOf[d.optString("id")] ?: 1)
                    1 -> secretDialog(d.optString("id"), d.optString("name"), "pair", monitorOf[d.optString("id")] ?: 1)
                    2 -> app.command("forget", "id" to d.optString("id"))
                }
            }
            .show()
    }

    private fun open(d: JSONObject, control: Boolean, auth: String? = null, secret: String? = null) {
        val id = d.optString("id")
        SessionActivity.open(this, id, d.optString("name"), monitorOf[id] ?: 1, control, auth, secret)
    }

    /** Contraseña de acceso desatendido o código de emparejamiento. */
    fun secretDialog(id: String, name: String, auth: String, monitor: Int) {
        val f = field(if (auth == "password") "Contraseña" else "Código (p. ej. K7QM-4ZP2)", password = auth == "password")
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(if (auth == "password") "Contraseña de $name" else "Emparejar con $name")
            .setView(vbox { setPadding(dp(20), dp(8), dp(20), 0); addView(f) })
            .setPositiveButton("Conectar") { _, _ -> SessionActivity.open(this, id, name, monitor, true, auth, f.text.toString()) }
            .setNegativeButton("Cancelar", null)
            .show()
    }

    private fun idDialog() {
        val f = field("ID de 9 dígitos").apply { inputType = android.text.InputType.TYPE_CLASS_NUMBER }
        val pw = field("Contraseña o código (opcional)", password = true)
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("Conectar por ID")
            .setMessage("El equipo debe estar en tu red.")
            .setView(vbox(dp(8)) { setPadding(dp(20), dp(8), dp(20), 0); addView(f); addView(pw) })
            .setPositiveButton("Conectar") { _, _ ->
                val secret = pw.text.toString().trim()
                // Un código de emparejamiento es de 8 letras y números (con o sin guion); lo demás, contraseña.
                val auth = when {
                    secret.isEmpty() -> null
                    Regex("^[A-Za-z0-9]{4}-?[A-Za-z0-9]{4}$").matches(secret) -> "pair"
                    else -> "password"
                }
                SessionActivity.open(this, f.text.toString(), "ID ${f.text}", 1, true, auth, secret.ifEmpty { null })
            }
            .setNegativeButton("Cancelar", null)
            .show()
    }

    private fun ipDialog() {
        val f = field("192.168.1.20")
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle("Agregar por IP")
            .setView(vbox { setPadding(dp(20), dp(8), dp(20), 0); addView(f) })
            .setPositiveButton("Agregar") { _, _ -> app.command("add", "addr" to f.text.toString()) }
            .setNegativeButton("Cancelar", null)
            .show()
        f.gravity = Gravity.START
    }
}
