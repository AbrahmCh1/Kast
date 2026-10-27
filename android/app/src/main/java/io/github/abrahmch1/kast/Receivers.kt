package io.github.abrahmch1.kast

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Al encender el equipo (o al actualizar Kast): si estaba compartiendo, vuelve a compartir. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val app = KastApp.app
        if (!app.large || !app.prefs.getBoolean("share", false) || ShareService.instance != null) return
        // Hace falta una actividad para el permiso de captura (con «Mostrar sobre otras apps» se puede abrir desde aquí).
        try {
            ctx.startActivity(Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("autoshare", true))
        } catch (_: Exception) {
        }
    }
}

/** Botones de los avisos de petición de conexión. */
class ActionReceiver : BroadcastReceiver() {
    companion object {
        const val ACCEPT = "io.github.abrahmch1.kast.ACCEPT"
        const val REJECT = "io.github.abrahmch1.kast.REJECT"
    }

    override fun onReceive(ctx: Context, intent: Intent) {
        val req = intent.getLongExtra("req", 0)
        when (intent.action) {
            ACCEPT -> KastApp.app.answer(req, accept = true, remember = false, control = intent.getBooleanExtra("control", false))
            REJECT -> KastApp.app.answer(req, accept = false, remember = false, control = false)
        }
    }
}
