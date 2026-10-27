package io.github.abrahmch1.kast

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.content.res.ColorStateList
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.roundToInt

/** Colores de Kast (los del tema oscuro de la app de escritorio, src/ui.html). */
object K {
    const val BG = 0xFF202020.toInt()
    const val CARD = 0xFF2B2B2B.toInt()
    const val CARD2 = 0xFF333333.toInt()
    const val STROKE = 0x14FFFFFF
    const val TEXT = Color.WHITE
    const val SUB = 0xFFA9A9A9.toInt()
    const val FAINT = 0xFF7D7D7D.toInt()
    const val ACCENT = 0xFF8197FF.toInt()
    const val ON_ACCENT = 0xFF0B0B14.toInt()
    const val DANGER = 0xFFFF99A4.toInt()
    const val OK = 0xFF6CCB5F.toInt()
}

fun Context.dp(v: Number): Int = (v.toFloat() * resources.displayMetrics.density).roundToInt()

fun rounded(color: Int, radius: Int, stroke: Int = 0, strokeColor: Int = K.STROKE) = GradientDrawable().apply {
    setColor(color)
    cornerRadius = radius.toFloat()
    if (stroke > 0) setStroke(stroke, strokeColor)
}

fun Context.label(text: CharSequence, size: Float = 14f, color: Int = K.TEXT, bold: Boolean = false): TextView = TextView(this).apply {
    this.text = text
    textSize = size
    setTextColor(color)
    if (bold) typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
}

fun Context.button(text: CharSequence, primary: Boolean = false, danger: Boolean = false, onClick: (View) -> Unit): Button = Button(this).apply {
    this.text = text
    isAllCaps = false
    textSize = 14f
    minHeight = 0
    minimumHeight = 0
    minWidth = 0
    minimumWidth = 0
    setPadding(dp(16), dp(9), dp(16), dp(9))
    stateListAnimator = null
    val bg = if (primary) K.ACCENT else K.CARD2
    setTextColor(if (primary) K.ON_ACCENT else if (danger) K.DANGER else K.TEXT)
    background = RippleDrawable(ColorStateList.valueOf(0x33FFFFFF), rounded(bg, dp(8), if (primary) 0 else 1), null)
    setOnClickListener(onClick)
}

fun Context.vbox(gap: Int = 0, block: LinearLayout.() -> Unit = {}): LinearLayout = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    if (gap > 0) {
        showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
        dividerDrawable = GradientDrawable().apply { setSize(0, gap) }
    }
    block()
}

fun Context.hbox(gap: Int = 0, block: LinearLayout.() -> Unit = {}): LinearLayout = LinearLayout(this).apply {
    orientation = LinearLayout.HORIZONTAL
    gravity = Gravity.CENTER_VERTICAL
    if (gap > 0) {
        showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
        dividerDrawable = GradientDrawable().apply { setSize(gap, 0) }
    }
    block()
}

fun Context.card(block: LinearLayout.() -> Unit = {}): LinearLayout = vbox(dp(10)) {
    background = rounded(K.CARD, dp(10), 1)
    setPadding(dp(18), dp(16), dp(18), dp(16))
    block()
}

fun Context.field(hint: String, password: Boolean = false): EditText = EditText(this).apply {
    this.hint = hint
    setTextColor(K.TEXT)
    setHintTextColor(K.FAINT)
    textSize = 16f
    isSingleLine = true
    inputType = if (password) InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_CLASS_TEXT
}

/** Espacio flexible en una fila. */
fun Context.spring(): View = View(this).apply { layoutParams = LinearLayout.LayoutParams(0, 1, 1f) }

fun View.margins(l: Int = 0, t: Int = 0, r: Int = 0, b: Int = 0): View {
    val lp = (layoutParams as? ViewGroup.MarginLayoutParams) ?: LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT)
    lp.setMargins(l, t, r, b)
    layoutParams = lp
    return this
}

fun platformName(p: String) = when (p) {
    "windows" -> "Windows"
    "mac" -> "Mac"
    "ios" -> "iPhone/iPad"
    "android" -> "Android"
    else -> ""
}
