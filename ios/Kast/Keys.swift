import UIKit

/// Nombres de tecla W3C (los mismos que envía el visor de escritorio, ver link/src/input.rs).
enum Keys {
    /// Teclado físico (UIKey.keyCode, uso HID) -> nombre.
    static func code(_ k: UIKeyboardHIDUsage) -> String? {
        let r = k.rawValue
        switch r {
        case 0x04...0x1D: return "Key" + String(UnicodeScalar(UInt8(65 + r - 0x04)))
        case 0x1E...0x26: return "Digit\(r - 0x1D)"
        case 0x27: return "Digit0"
        case 0x3A...0x45: return "F\(r - 0x39)"
        case 0x59...0x61: return "Numpad\(r - 0x58)"
        default: break
        }
        return [
            0x28: "Enter", 0x29: "Escape", 0x2A: "Backspace", 0x2B: "Tab", 0x2C: "Space", 0x2D: "Minus", 0x2E: "Equal",
            0x2F: "BracketLeft", 0x30: "BracketRight", 0x31: "Backslash", 0x32: "Backslash", 0x33: "Semicolon", 0x34: "Quote",
            0x35: "Backquote", 0x36: "Comma", 0x37: "Period", 0x38: "Slash", 0x39: "CapsLock", 0x46: "PrintScreen",
            0x47: "ScrollLock", 0x48: "Pause", 0x49: "Insert", 0x4A: "Home", 0x4B: "PageUp", 0x4C: "Delete", 0x4D: "End",
            0x4E: "PageDown", 0x4F: "ArrowRight", 0x50: "ArrowLeft", 0x51: "ArrowDown", 0x52: "ArrowUp", 0x53: "NumLock",
            0x54: "NumpadDivide", 0x55: "NumpadMultiply", 0x56: "NumpadSubtract", 0x57: "NumpadAdd", 0x58: "NumpadEnter",
            0x62: "Numpad0", 0x63: "NumpadDecimal", 0x64: "IntlBackslash", 0x65: "ContextMenu",
            0xE0: "ControlLeft", 0xE1: "ShiftLeft", 0xE2: "AltLeft", 0xE3: "SuperLeft",
            0xE4: "ControlRight", 0xE5: "ShiftRight", 0xE6: "AltRight", 0xE7: "SuperRight",
        ][r]
    }

    /// Letras y números escritos con el teclado de iOS cuando hay un modificador fijado (Ctrl+C…).
    static func code(for ch: Character) -> String? {
        guard let a = ch.lowercased().unicodeScalars.first, ch.unicodeScalars.count == 1 else { return nil }
        switch a {
        case "a"..."z": return "Key" + String(a).uppercased()
        case "0"..."9": return "Digit" + String(a)
        case " ": return "Space"
        default: return nil
        }
    }

    /// Modificador tal como lo verá el equipo remoto. iOS cuenta como Mac: hacia Windows, ⌘ llega como Ctrl
    /// y Ctrl como Win (link/src/input.rs map_modifier), así que aquí se envía el que produce la tecla pedida.
    enum Mod: String, CaseIterable {
        case ctrl, alt, shift, meta

        func code(host: String) -> String {
            switch self {
            case .ctrl: return host == "windows" ? "SuperLeft" : "ControlLeft"
            case .meta: return host == "windows" ? "ControlLeft" : "SuperLeft"
            case .alt: return "AltLeft"
            case .shift: return "ShiftLeft"
            }
        }

        func label(host: String) -> String {
            switch self {
            case .ctrl: return "Ctrl"
            case .alt: return host == "windows" ? "Alt" : "⌥"
            case .shift: return "⇧"
            case .meta: return host == "windows" ? "Win" : "⌘"
            }
        }
    }

    /// Atajos habituales según el sistema remoto: (nombre, modificadores, tecla).
    static func shortcuts(host: String) -> [(String, [Mod], String)] {
        let cmd: Mod = host == "windows" ? .ctrl : .meta
        var list: [(String, [Mod], String)] = [
            ("Copiar", [cmd], "KeyC"), ("Pegar", [cmd], "KeyV"), ("Cortar", [cmd], "KeyX"),
            ("Deshacer", [cmd], "KeyZ"), ("Seleccionar todo", [cmd], "KeyA"), ("Buscar", [cmd], "KeyF"),
        ]
        if host == "windows" {
            list += [("Cambiar de app", [.alt], "Tab"), ("Escritorio", [.meta], "KeyD"), ("Explorador", [.meta], "KeyE"),
                     ("Cerrar ventana", [.alt], "F4"), ("Administrador de tareas", [.ctrl, .shift], "Escape")]
        } else {
            list += [("Cambiar de app", [.meta], "Tab"), ("Spotlight", [.meta], "Space"),
                     ("Cerrar ventana", [.meta], "KeyW"), ("Salir de la app", [.meta], "KeyQ")]
        }
        return list
    }
}
