package io.github.abrahmch1.kast

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.widget.CheckBox
import android.widget.LinearLayout
import java.lang.ref.WeakReference

/** Alguien pide ver (y controlar) esta pantalla: se comprueba el código y se acepta o se rechaza. */
class ApproveActivity : Activity() {
    companion object {
        /** Ventanas abiertas de cada petición (la del aviso a pantalla completa puede sumarse a la otra). */
        private val open = HashMap<Long, MutableList<WeakReference<ApproveActivity>>>()

        /** La petición ya se contestó (o caducó): se cierran sus ventanas. */
        fun close(req: Long) { open.remove(req)?.forEach { it.get()?.finish() } }
    }

    private var req = 0L

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setShowWhenLocked(true)
        setTurnScreenOn(true)
        setFinishOnTouchOutside(false)
        req = intent.getLongExtra("req", 0)
        open.getOrPut(req) { mutableListOf() }.add(WeakReference(this))
        val name = intent.getStringExtra("name").orEmpty()
        val platform = platformName(intent.getStringExtra("platform").orEmpty())
        val wantsControl = intent.getBooleanExtra("control", false)
        val remember = CheckBox(this).apply { text = "Recordar este dispositivo (no volver a preguntar)"; setTextColor(K.TEXT) }
        val control = CheckBox(this).apply { text = "Permitir que controle el teclado y el ratón"; setTextColor(K.TEXT); isChecked = true }
        setContentView(vbox(dp(14)) {
            setPadding(dp(28), dp(24), dp(28), dp(22))
            background = rounded(K.CARD, dp(14), 1)
            addView(label("$name${if (platform.isNotEmpty()) " ($platform)" else ""} quiere ${if (wantsControl) "ver y controlar" else "ver"} esta pantalla", 19f, bold = true))
            addView(label("Comprueba que el código coincide con el que se ve en ese equipo:", 14f, K.SUB))
            addView(label(intent.getStringExtra("code").orEmpty(), 36f, K.ACCENT, bold = true).apply { letterSpacing = 0.1f; gravity = Gravity.CENTER })
            addView(remember)
            if (wantsControl) addView(control)
            addView(hbox(dp(10)) {
                gravity = Gravity.END
                addView(button("Rechazar", danger = true) { answer(false, false, false) })
                addView(button("Aceptar", primary = true) { answer(true, remember.isChecked, wantsControl && control.isChecked) })
            })
        }, ViewGroup.LayoutParams(dp(520).coerceAtMost(resources.displayMetrics.widthPixels - dp(32)), ViewGroup.LayoutParams.WRAP_CONTENT))
        window.setBackgroundDrawableResource(android.R.color.transparent)
        window.setLayout(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT)
    }

    private fun answer(accept: Boolean, remember: Boolean, control: Boolean) {
        KastApp.app.answer(req, accept, remember, control)
        finish()
    }

    override fun onDestroy() {
        open[req]?.removeAll { it.get() == null || it.get() === this }
        super.onDestroy()
    }
}
