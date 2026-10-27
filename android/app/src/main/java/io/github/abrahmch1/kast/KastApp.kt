package io.github.abrahmch1.kast

import android.app.Application
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.widget.Toast
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors

/** Estado de la app y reparto de los eventos del núcleo (en el hilo principal, salvo el vídeo y la entrada remota). */
class KastApp : Application() {
    companion object {
        lateinit var app: KastApp
            private set

        const val CH_SHARE = "share"
        const val CH_REQUEST = "request"
        const val CH_FILES = "files"
        const val N_SHARE = 1
        const val N_FILES = 3
        fun requestNotification(req: Long) = 1000 + req.toInt()
    }

    val main = Handler(Looper.getMainLooper())
    val prefs by lazy { getSharedPreferences("kast", MODE_PRIVATE) }

    /** Pantalla grande (tableta o pantalla interactiva): además de ver otros equipos, puede compartir la suya. */
    val large get() = resources.configuration.smallestScreenWidthDp >= 600

    var devices: List<JSONObject> = emptyList()
        private set
    var host: JSONObject? = null
        private set

    /** Tamaño de la imagen que se envía (para traducir el ratón remoto a la pantalla). */
    @Volatile var streamSize = 1920 to 1080

    /** Vídeo de las sesiones abiertas (lo recoge SessionActivity). */
    @Volatile var frameSink: ((Long, ByteArray, Boolean) -> Unit)? = null

    private val listeners = LinkedHashSet<(JSONObject) -> Unit>()
    private val worker = Executors.newSingleThreadExecutor()

    fun listen(l: (JSONObject) -> Unit) { listeners.add(l) }
    fun unlisten(l: (JSONObject) -> Unit) { listeners.remove(l) }

    /** Comando al núcleo (en segundo plano: algunos, como crear un código, tardan un momento). */
    fun command(cmd: String, vararg args: Pair<String, Any?>) {
        val o = JSONObject().put("cmd", cmd)
        for ((k, v) in args) o.put(k, v ?: JSONObject.NULL)
        worker.execute { Native.command(o.toString()) }
    }

    override fun onCreate() {
        super.onCreate()
        app = this
        channels()
        Native.start(filesDir.absolutePath, deviceName(), large)
        command("downloads", "dir" to downloadsDir().absolutePath)
        command("perms", "input" to (ControlService.instance != null))
        command("host_state")
    }

    fun deviceName(): String =
        Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() }
            ?: "${Build.MANUFACTURER.replaceFirstChar { it.uppercase() }} ${Build.MODEL}"

    /** Descargas/Kast si se puede escribir ahí (Android 11+); si no, la carpeta de descargas de la app. */
    private fun downloadsDir(): File {
        val pub = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Kast")
        try {
            pub.mkdirs()
            val probe = File(pub, ".kast")
            probe.writeText("")
            probe.delete()
            return pub
        } catch (_: Exception) {
        }
        return getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: File(filesDir, "Descargas")
    }

    private fun channels() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_SHARE, "Compartir esta pantalla", NotificationManager.IMPORTANCE_LOW))
        nm.createNotificationChannel(NotificationChannel(CH_REQUEST, "Peticiones de conexión", NotificationManager.IMPORTANCE_HIGH))
        nm.createNotificationChannel(NotificationChannel(CH_FILES, "Archivos recibidos", NotificationManager.IMPORTANCE_DEFAULT))
    }

    /** Desde hilos de Rust. */
    fun onEvent(ev: JSONObject) {
        when (ev.optString("ev")) {
            "input" -> { ControlService.instance?.onInput(ev); return }
            "keyframe" -> { ShareService.instance?.keyframe(); return }
            "capture" -> { ShareService.instance?.capture(ev.optBoolean("on")); return }
        }
        main.post { dispatch(ev) }
    }

    private fun dispatch(ev: JSONObject) {
        when (ev.optString("ev")) {
            "devices" -> devices = ev.optJSONArray("devices").objects()
            "host" -> {
                host = ev
                ev.optJSONObject("me")?.let { streamSize = it.optInt("w", 1920) to it.optInt("h", 1080) }
                ShareService.instance?.update(ev)
                // Peticiones que ya no están (contestadas en otro sitio o caducadas): fuera sus avisos.
                val pending = ev.optJSONArray("requests").objects().map { it.optLong("req") }.toSet()
                for (r in shownRequests.toList()) if (r !in pending) closeRequest(r)
            }
            "toast" -> Toast.makeText(this, ev.optString("msg"), if (ev.optBoolean("err")) Toast.LENGTH_LONG else Toast.LENGTH_SHORT).show()
            "request" -> showRequest(ev)
            "clip_in" -> getSystemService(ClipboardManager::class.java)
                .setPrimaryClip(ClipData.newPlainText("Kast", ev.optString("text")))
            "files" -> filesReceived(ev)
        }
        for (l in listeners.toList()) l(ev)
    }

    // ------------------------------------------------------------ peticiones de conexión

    private val shownRequests = HashSet<Long>()

    private fun showRequest(ev: JSONObject) {
        val req = ev.optLong("req")
        shownRequests.add(req)
        val intent = Intent(this, ApproveActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_MULTIPLE_TASK)
            .putExtra("req", req).putExtra("name", ev.optString("name")).putExtra("platform", ev.optString("platform"))
            .putExtra("code", ev.optString("code")).putExtra("control", ev.optBoolean("control"))
        // Con «Mostrar sobre otras apps» se abre aunque Kast esté en segundo plano; si no, queda el aviso.
        try { startActivity(intent) } catch (_: Exception) { }
        val open = PendingIntent.getActivity(this, req.toInt(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        fun action(name: String, label: String) = Notification.Action.Builder(null, label,
            PendingIntent.getBroadcast(this, (req * 4 + label.length).toInt(),
                Intent(this, ActionReceiver::class.java).setAction(name).putExtra("req", req).putExtra("control", ev.optBoolean("control")),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)).build()
        val what = if (ev.optBoolean("control")) "ver y controlar" else "ver"
        val n = Notification.Builder(this, CH_REQUEST)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("${ev.optString("name")} quiere $what esta pantalla")
            .setContentText("Código ${ev.optString("code")}")
            .setCategory(Notification.CATEGORY_CALL)
            .setContentIntent(open)
            .setFullScreenIntent(open, true)
            .addAction(action(ActionReceiver.REJECT, "Rechazar"))
            .addAction(action(ActionReceiver.ACCEPT, "Aceptar"))
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(requestNotification(req), n)
    }

    fun answer(req: Long, accept: Boolean, remember: Boolean, control: Boolean) {
        command("answer", "req" to req, "accept" to accept, "remember" to remember, "control" to control)
        closeRequest(req)
    }

    private fun closeRequest(req: Long) {
        shownRequests.remove(req)
        getSystemService(NotificationManager::class.java).cancel(requestNotification(req))
        ApproveActivity.close(req)
    }

    private fun filesReceived(ev: JSONObject) {
        val n = Notification.Builder(this, CH_FILES)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("Archivos recibidos")
            .setContentText(ev.optString("first"))
            .setAutoCancel(true)
            .build()
        getSystemService(NotificationManager::class.java).notify(N_FILES, n)
    }
}

fun JSONArray?.objects(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }
