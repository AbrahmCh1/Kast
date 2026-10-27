package io.github.abrahmch1.kast

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Al encender el equipo (o al actualizar Kast): si estaba compartiendo, vuelve a compartir. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) = KastApp.app.resumeSharing()
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
