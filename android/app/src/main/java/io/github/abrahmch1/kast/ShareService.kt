package io.github.abrahmch1.kast

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Point
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import android.view.Surface
import android.view.WindowManager
import org.json.JSONObject
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * Comparte esta pantalla: la captura con MediaProjection en una pantalla virtual que dibuja en la entrada de un
 * codificador H.264 (MediaCodec) y entrega cada fotograma al núcleo, que lo envía a quien esté viendo. El codificador
 * sólo está encendido mientras hay alguien conectado.
 */
class ShareService : Service() {
    companion object {
        @Volatile var instance: ShareService? = null
            private set
        private const val STOP = "stop"
        /** Lado largo de la imagen enviada (las pantallas 4K se envían a 1080p: el codificador no da para más a 30 fps). */
        private const val MAX_SIDE = 1920
        private const val FPS = 30
        private const val BITRATE = 12_000_000

        /** Tras aceptar el permiso de captura ([code] y [data] de createScreenCaptureIntent). */
        fun start(ctx: Context, code: Int, data: Intent) {
            ctx.startForegroundService(Intent(ctx, ShareService::class.java).putExtra("code", code).putExtra("data", data))
        }

        fun stop(ctx: Context) {
            instance?.let { ctx.startService(Intent(ctx, ShareService::class.java).setAction(STOP)) }
        }
    }

    private val thread = HandlerThread("kast-compartir").apply { start() }
    private val handler = Handler(thread.looper)
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private var codec: MediaCodec? = null
    private var input: Surface? = null
    private var drain: Thread? = null
    @Volatile private var running = false
    /** Cuándo salió el último fotograma clave. */
    @Volatile private var lastKey = 0L
    private var w = 1920
    private var h = 1080
    private var fps = FPS
    private var multicast: WifiManager.MulticastLock? = null
    private var wifi: WifiManager.WifiLock? = null
    private var viewers = 0
    /** Mientras alguien está conectado la pantalla se enciende y no se apaga. */
    private var awake: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP) {
            stopSharing(user = true)
            return START_NOT_STICKY
        }
        // Android volvió a crear el servicio tras cerrar la app: hace falta otra vez el permiso de captura.
        if (intent == null) {
            stopSelf()
            KastApp.app.resumeSharing()
            return START_NOT_STICKY
        }
        // Android 14: el servicio debe estar en primer plano (tipo «proyección») antes de pedir la captura.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(KastApp.N_SHARE, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(KastApp.N_SHARE, notification())
        }
        if (projection != null) return START_NOT_STICKY
        val code = intent?.getIntExtra("code", 0) ?: 0
        val data: Intent? = if (Build.VERSION.SDK_INT >= 33) intent?.getParcelableExtra("data", Intent::class.java)
                            else @Suppress("DEPRECATION") intent?.getParcelableExtra("data")
        val p = data?.let { getSystemService(MediaProjectionManager::class.java).getMediaProjection(code, it) }
        if (p == null) {
            stopSharing(user = false)
            return START_NOT_STICKY
        }
        p.registerCallback(object : MediaProjection.Callback() {
            // Se dejó de compartir desde el sistema (o Android cortó la captura): si la pantalla se dejó compartiendo,
            // vuelve a compartir.
            override fun onStop() {
                handler.post { stopSharing(user = false) }
                KastApp.app.main.postDelayed({ KastApp.app.resumeSharing() }, 3000)
            }
        }, handler)
        projection = p
        pickSize()
        display = p.createVirtualDisplay("Kast", w, h, resources.displayMetrics.densityDpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, null, null, handler)
        val wm = applicationContext.getSystemService(WifiManager::class.java)
        multicast = wm.createMulticastLock("kast").apply { setReferenceCounted(false); acquire() }
        @Suppress("DEPRECATION")
        wifi = wm.createWifiLock(if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF, "kast")
            .apply { setReferenceCounted(false); acquire() }
        KastApp.app.prefs.edit().putBoolean("share", true).apply()
        KastApp.app.command("video", "w" to w, "h" to h, "fps" to fps)
        KastApp.app.command("share", "enabled" to true)
        Log.i("Kast", "compartiendo ${w}x$h a $fps fps")
        return START_STICKY
    }

    /** Tamaño de la imagen: la pantalla reducida a MAX_SIDE, en un tamaño que acepte el codificador. */
    private fun pickSize() {
        val (dw, dh) = realSize(this)
        val s = min(1.0, MAX_SIDE.toDouble() / max(dw, dh))
        w = ((dw * s).roundToInt() + 1) and 1.inv()
        h = ((dh * s).roundToInt() + 1) and 1.inv()
        val caps = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .firstOrNull { it.isEncoder && it.supportedTypes.any { t -> t.equals(MediaFormat.MIMETYPE_VIDEO_AVC, true) } }
            ?.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)?.videoCapabilities ?: return
        if (!caps.isSizeSupported(w, h)) {
            val (aw, ah) = caps.widthAlignment to caps.heightAlignment
            w = (w + aw - 1) / aw * aw
            h = (h + ah - 1) / ah * ah
        }
        fps = if (caps.areSizeAndRateSupported(w, h, FPS.toDouble())) FPS
              else caps.getSupportedFrameRatesFor(w, h).upper.toInt().coerceIn(10, FPS)
    }

    /** Hay (o ya no hay) alguien viendo: se enciende o se apaga el codificador. */
    fun capture(on: Boolean) = handler.post { if (on) startEncoder() else stopEncoder() }

    fun keyframe() {
        val asked = SystemClock.elapsedRealtime()
        try { codec?.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) }) } catch (_: Exception) { }
        // Con la pantalla quieta no llegan imágenes nuevas al codificador (Android repite la última sólo 10 veces) y
        // sin una nueva no sale el fotograma clave: si no ha salido enseguida, se reinicia el codificador, cuya
        // primera imagen siempre es clave.
        handler.postDelayed({
            if (codec != null && lastKey < asked) {
                stopEncoder()
                startEncoder()
            }
        }, 250)
    }

    private fun format(full: Boolean) = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h).apply {
        setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
        setInteger(MediaFormat.KEY_BIT_RATE, BITRATE)
        setInteger(MediaFormat.KEY_FRAME_RATE, fps)
        // Fotogramas clave cuando los pide un visor (y uno de vez en cuando).
        setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 10)
        // Al quedarse quieta la pantalla se repite la imagen unas veces: así se afina la nitidez.
        setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
        if (full) {
            setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline)
            setInteger(MediaFormat.KEY_LEVEL, MediaCodecInfo.CodecProfileLevel.AVCLevel41)
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            if (Build.VERSION.SDK_INT >= 29) setInteger(MediaFormat.KEY_PREPEND_HEADER_TO_SYNC_FRAMES, 1)
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LATENCY, 1)
        }
    }

    private fun startEncoder() {
        if (codec != null) return
        val vd = display ?: return
        val c = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        try {
            c.configure(format(full = true), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        } catch (e: Exception) {
            Log.w("Kast", "codificador: sin opciones de baja latencia ($e)")
            c.reset()
            c.configure(format(full = false), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        }
        val s = c.createInputSurface()
        c.start()
        vd.surface = s
        codec = c
        input = s
        running = true
        @Suppress("DEPRECATION")
        awake = getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.SCREEN_BRIGHT_WAKE_LOCK or PowerManager.ACQUIRE_CAUSES_WAKEUP, "kast:sesion")
            .apply { setReferenceCounted(false); acquire() }
        drain = Thread({
            val info = MediaCodec.BufferInfo()
            while (running) {
                val i = try { c.dequeueOutputBuffer(info, 100_000) } catch (_: Exception) { break }
                if (i < 0) continue
                val buf = c.getOutputBuffer(i)
                if (buf != null && info.size > 0) Native.hostFrame(buf, info.offset, info.size, info.flags)
                if (info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0) lastKey = SystemClock.elapsedRealtime()
                c.releaseOutputBuffer(i, false)
            }
        }, "kast-codificador").apply { start() }
        Log.i("Kast", "codificador encendido")
    }

    private fun stopEncoder() {
        val c = codec ?: return
        running = false
        drain?.join(1000)
        drain = null
        display?.surface = null
        try { c.stop() } catch (_: Exception) { }
        c.release()
        input?.release()
        codec = null
        input = null
        awake?.release()
        awake = null
        Log.i("Kast", "codificador apagado")
    }

    /** Estado del anfitrión: el aviso dice cuántos están conectados. */
    fun update(host: JSONObject) {
        val n = host.optJSONArray("sessions")?.length() ?: 0
        if (n == viewers || projection == null) return
        viewers = n
        getSystemService(NotificationManager::class.java).notify(KastApp.N_SHARE, notification())
    }

    private fun notification(): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 1, Intent(this, ShareService::class.java).setAction(STOP), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, KastApp.CH_SHARE)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("Compartiendo esta pantalla")
            .setContentText(when (viewers) { 0 -> "Nadie conectado"; 1 -> "1 dispositivo conectado"; else -> "$viewers dispositivos conectados" })
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, "Dejar de compartir", stop).build())
            .setOngoing(true)
            .build()
    }

    /** [user]: lo pidió el usuario (no se vuelve a compartir al reiniciar). */
    private fun stopSharing(user: Boolean) {
        if (user) KastApp.app.prefs.edit().putBoolean("share", false).apply()
        KastApp.app.command("share", "enabled" to false)
        stopEncoder()
        display?.release()
        display = null
        projection?.stop()
        projection = null
        multicast?.release()
        wifi?.release()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (projection != null) stopSharing(user = false)
        instance = null
        thread.quitSafely()
        super.onDestroy()
    }
}

/** Tamaño real de la pantalla en píxeles (con la orientación actual). */
fun realSize(ctx: Context): Pair<Int, Int> {
    val wm = ctx.getSystemService(WindowManager::class.java)
    if (Build.VERSION.SDK_INT >= 30) {
        val b = wm.maximumWindowMetrics.bounds
        return b.width() to b.height()
    }
    val p = Point()
    @Suppress("DEPRECATION")
    wm.defaultDisplay.getRealSize(p)
    return p.x to p.y
}
