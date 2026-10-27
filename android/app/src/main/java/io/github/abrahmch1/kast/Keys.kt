package io.github.abrahmch1.kast

import android.view.KeyEvent

/** Teclas: nombre W3C (lo que viaja, ver link/src/input.rs) <-> código de tecla de Android. */
object Keys {
    private val pairs: List<Pair<String, Int>> = buildList {
        for (i in 0 until 26) add("Key${'A' + i}" to KeyEvent.KEYCODE_A + i)
        for (i in 0..9) add("Digit$i" to KeyEvent.KEYCODE_0 + i)
        for (i in 0..9) add("Numpad$i" to KeyEvent.KEYCODE_NUMPAD_0 + i)
        for (i in 1..12) add("F$i" to KeyEvent.KEYCODE_F1 + i - 1)
        addAll(listOf(
            "Enter" to KeyEvent.KEYCODE_ENTER, "Backspace" to KeyEvent.KEYCODE_DEL, "Delete" to KeyEvent.KEYCODE_FORWARD_DEL,
            "Tab" to KeyEvent.KEYCODE_TAB, "Space" to KeyEvent.KEYCODE_SPACE, "Escape" to KeyEvent.KEYCODE_ESCAPE,
            "ArrowLeft" to KeyEvent.KEYCODE_DPAD_LEFT, "ArrowRight" to KeyEvent.KEYCODE_DPAD_RIGHT,
            "ArrowUp" to KeyEvent.KEYCODE_DPAD_UP, "ArrowDown" to KeyEvent.KEYCODE_DPAD_DOWN,
            "Home" to KeyEvent.KEYCODE_MOVE_HOME, "End" to KeyEvent.KEYCODE_MOVE_END,
            "PageUp" to KeyEvent.KEYCODE_PAGE_UP, "PageDown" to KeyEvent.KEYCODE_PAGE_DOWN, "Insert" to KeyEvent.KEYCODE_INSERT,
            "Minus" to KeyEvent.KEYCODE_MINUS, "Equal" to KeyEvent.KEYCODE_EQUALS,
            "BracketLeft" to KeyEvent.KEYCODE_LEFT_BRACKET, "BracketRight" to KeyEvent.KEYCODE_RIGHT_BRACKET,
            "Backslash" to KeyEvent.KEYCODE_BACKSLASH, "Semicolon" to KeyEvent.KEYCODE_SEMICOLON,
            "Quote" to KeyEvent.KEYCODE_APOSTROPHE, "Backquote" to KeyEvent.KEYCODE_GRAVE,
            "Comma" to KeyEvent.KEYCODE_COMMA, "Period" to KeyEvent.KEYCODE_PERIOD, "Slash" to KeyEvent.KEYCODE_SLASH,
            "ShiftLeft" to KeyEvent.KEYCODE_SHIFT_LEFT, "ShiftRight" to KeyEvent.KEYCODE_SHIFT_RIGHT,
            "ControlLeft" to KeyEvent.KEYCODE_CTRL_LEFT, "ControlRight" to KeyEvent.KEYCODE_CTRL_RIGHT,
            "AltLeft" to KeyEvent.KEYCODE_ALT_LEFT, "AltRight" to KeyEvent.KEYCODE_ALT_RIGHT,
            "MetaLeft" to KeyEvent.KEYCODE_META_LEFT, "MetaRight" to KeyEvent.KEYCODE_META_RIGHT,
            "CapsLock" to KeyEvent.KEYCODE_CAPS_LOCK, "NumLock" to KeyEvent.KEYCODE_NUM_LOCK, "ScrollLock" to KeyEvent.KEYCODE_SCROLL_LOCK,
            "NumpadAdd" to KeyEvent.KEYCODE_NUMPAD_ADD, "NumpadSubtract" to KeyEvent.KEYCODE_NUMPAD_SUBTRACT,
            "NumpadMultiply" to KeyEvent.KEYCODE_NUMPAD_MULTIPLY, "NumpadDivide" to KeyEvent.KEYCODE_NUMPAD_DIVIDE,
            "NumpadDecimal" to KeyEvent.KEYCODE_NUMPAD_DOT, "NumpadEnter" to KeyEvent.KEYCODE_NUMPAD_ENTER,
            "NumpadEqual" to KeyEvent.KEYCODE_NUMPAD_EQUALS, "ContextMenu" to KeyEvent.KEYCODE_MENU,
            "PrintScreen" to KeyEvent.KEYCODE_SYSRQ, "Pause" to KeyEvent.KEYCODE_BREAK,
            "AudioVolumeUp" to KeyEvent.KEYCODE_VOLUME_UP, "AudioVolumeDown" to KeyEvent.KEYCODE_VOLUME_DOWN,
            "AudioVolumeMute" to KeyEvent.KEYCODE_VOLUME_MUTE, "MediaPlayPause" to KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE,
            "MediaTrackNext" to KeyEvent.KEYCODE_MEDIA_NEXT, "MediaTrackPrevious" to KeyEvent.KEYCODE_MEDIA_PREVIOUS,
        ))
    }

    /** W3C -> Android (el visor de escritorio llama SuperLeft a la tecla Windows/⌘). */
    val toAndroid: Map<String, Int> = pairs.toMap() + mapOf("SuperLeft" to KeyEvent.KEYCODE_META_LEFT, "SuperRight" to KeyEvent.KEYCODE_META_RIGHT,
        "IntlBackslash" to KeyEvent.KEYCODE_BACKSLASH)

    /** Android -> W3C (teclado físico en el visor). */
    val toW3c: Map<Int, String> = pairs.associate { (c, k) -> k to c }

    /** Bits de KeyEvent.META_* de un modificador (0 si no lo es). */
    fun meta(kc: Int): Int = when (kc) {
        KeyEvent.KEYCODE_SHIFT_LEFT -> KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_LEFT_ON
        KeyEvent.KEYCODE_SHIFT_RIGHT -> KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_RIGHT_ON
        KeyEvent.KEYCODE_CTRL_LEFT -> KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON
        KeyEvent.KEYCODE_CTRL_RIGHT -> KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_RIGHT_ON
        KeyEvent.KEYCODE_ALT_LEFT -> KeyEvent.META_ALT_ON or KeyEvent.META_ALT_LEFT_ON
        KeyEvent.KEYCODE_ALT_RIGHT -> KeyEvent.META_ALT_ON or KeyEvent.META_ALT_RIGHT_ON
        KeyEvent.KEYCODE_META_LEFT -> KeyEvent.META_META_ON or KeyEvent.META_META_LEFT_ON
        KeyEvent.KEYCODE_META_RIGHT -> KeyEvent.META_META_ON or KeyEvent.META_META_RIGHT_ON
        else -> 0
    }
}
