// kast.ini (mismo formato que en Windows) e inicio automático.
import Foundation
import Carbon

struct Hotkey {
    let combo: String
    let keyCode: UInt32
    let mods: UInt32
    let action: String
}

enum Config {
    static let lock = NSLock()
    private(set) static var favs: [(name: String, act: String)] = []
    private(set) static var hotkeys: [Hotkey] = []
    /// Calibración por monitor (nombre en minúsculas): brillo y contraste.
    private(set) static var cals: [String: (b: Curve, c: Curve)] = [:]
    private(set) static var loaded = false

    private static let keyCodes: [String: Int] = {
        var m: [String: Int] = [
            "up": kVK_UpArrow, "down": kVK_DownArrow, "left": kVK_LeftArrow, "right": kVK_RightArrow,
            "pgup": kVK_PageUp, "pgdn": kVK_PageDown, "home": kVK_Home, "end": kVK_End,
            "ins": kVK_Help, "del": kVK_ForwardDelete, "space": kVK_Space, "tab": kVK_Tab,
            "esc": kVK_Escape, "enter": kVK_Return, "plus": kVK_ANSI_Equal, "minus": kVK_ANSI_Minus,
            "comma": kVK_ANSI_Comma, "period": kVK_ANSI_Period,
            "numplus": kVK_ANSI_KeypadPlus, "numminus": kVK_ANSI_KeypadMinus,
            "nummul": kVK_ANSI_KeypadMultiply, "numdiv": kVK_ANSI_KeypadDivide,
        ]
        let letters = [kVK_ANSI_A, kVK_ANSI_B, kVK_ANSI_C, kVK_ANSI_D, kVK_ANSI_E, kVK_ANSI_F, kVK_ANSI_G, kVK_ANSI_H,
                       kVK_ANSI_I, kVK_ANSI_J, kVK_ANSI_K, kVK_ANSI_L, kVK_ANSI_M, kVK_ANSI_N, kVK_ANSI_O, kVK_ANSI_P,
                       kVK_ANSI_Q, kVK_ANSI_R, kVK_ANSI_S, kVK_ANSI_T, kVK_ANSI_U, kVK_ANSI_V, kVK_ANSI_W, kVK_ANSI_X,
                       kVK_ANSI_Y, kVK_ANSI_Z]
        for (i, k) in letters.enumerated() { m[String(UnicodeScalar(UInt8(97 + i)))] = k }
        let digits = [kVK_ANSI_0, kVK_ANSI_1, kVK_ANSI_2, kVK_ANSI_3, kVK_ANSI_4, kVK_ANSI_5, kVK_ANSI_6, kVK_ANSI_7, kVK_ANSI_8, kVK_ANSI_9]
        let pad = [kVK_ANSI_Keypad0, kVK_ANSI_Keypad1, kVK_ANSI_Keypad2, kVK_ANSI_Keypad3, kVK_ANSI_Keypad4,
                   kVK_ANSI_Keypad5, kVK_ANSI_Keypad6, kVK_ANSI_Keypad7, kVK_ANSI_Keypad8, kVK_ANSI_Keypad9]
        for i in 0..<10 { m["\(i)"] = digits[i]; m["num\(i)"] = pad[i] }
        let f = [kVK_F1, kVK_F2, kVK_F3, kVK_F4, kVK_F5, kVK_F6, kVK_F7, kVK_F8, kVK_F9, kVK_F10,
                 kVK_F11, kVK_F12, kVK_F13, kVK_F14, kVK_F15, kVK_F16, kVK_F17, kVK_F18, kVK_F19, kVK_F20]
        for (i, k) in f.enumerated() { m["f\(i + 1)"] = k }
        return m
    }()

    /// "ctrl+alt+up" -> (código de tecla, modificadores Carbon). "win" equivale a ⌘ para compartir el ini con Windows.
    static func parseCombo(_ combo: String) -> (UInt32, UInt32)? {
        var mods: UInt32 = 0
        var key: Int?
        for raw in combo.lowercased().split(separator: "+") {
            let t = raw.trimmingCharacters(in: .whitespaces)
            switch t {
            case "ctrl", "control": mods |= UInt32(controlKey)
            case "alt", "opt", "option": mods |= UInt32(optionKey)
            case "shift": mods |= UInt32(shiftKey)
            case "cmd", "command", "win": mods |= UInt32(cmdKey)
            default:
                guard let k = keyCodes[t] else { return nil }
                key = k
            }
        }
        guard let k = key else { return nil }
        return (UInt32(k), mods)
    }

    /// Lee kast.ini. Devuelve los avisos para kast.log.
    @discardableResult
    static func load() -> String {
        Paths.ensureIni()
        var log = ""
        var f: [(String, String)] = [], h: [Hotkey] = [], c: [String: (b: Curve, c: Curve)] = [:]
        let text = (try? String(contentsOf: Paths.ini, encoding: .utf8)) ?? ""
        var sec = 0
        for (n, rawLine) in text.components(separatedBy: .newlines).enumerated() {
            let line = rawLine.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "\u{FEFF}", with: "")
            if line.isEmpty || line.hasPrefix("#") || line.hasPrefix(";") { continue }
            if line.hasPrefix("[") {
                let s = line.lowercased()
                sec = s.hasPrefix("[favoritos]") || s.hasPrefix("[favorites]") ? 1
                    : s.hasPrefix("[atajos]") || s.hasPrefix("[hotkeys]") ? 0
                    : s.hasPrefix("[calibracion]") || s.hasPrefix("[calibration]") ? 3
                    : s.hasPrefix("[calibracion-contraste]") || s.hasPrefix("[calibration-contrast]") ? 4 : 2
                continue
            }
            guard let eq = line.firstIndex(of: "=") else { log += "línea \(n + 1): falta '='\n"; continue }
            let key = line[..<eq].trimmingCharacters(in: .whitespaces)
            let act = line[line.index(after: eq)...].trimmingCharacters(in: .whitespaces)
            switch sec {
            case 1: f.append((key, act))
            case 0:
                if let r = parseCombo(key) { h.append(Hotkey(combo: key, keyCode: r.0, mods: r.1, action: act)) }
                else { log += "línea \(n + 1): tecla inválida '\(key)'\n" }
            case 3, 4:
                if let cv = Curve(act) {
                    var e = c[key.lowercased()] ?? (b: Curve.identity, c: Curve.identity)
                    if sec == 3 { e.b = cv } else { e.c = cv }
                    c[key.lowercased()] = e
                } else { log += "línea \(n + 1): calibración inválida (usa: monitor = 0:mínimo 50:medio 100:máximo)\n" }
            default: break
            }
        }
        lock.lock()
        favs = f.map { (name: $0.0, act: $0.1) }
        hotkeys = h
        cals = c
        loaded = true
        lock.unlock()
        return log
    }

    static func ensureLoaded() { if !loaded { load() } }

    static func favorite(_ name: String) -> String? {
        lock.lock(); defer { lock.unlock() }
        return favs.first { $0.name.caseInsensitiveCompare(name) == .orderedSame }?.act
    }

    /// Curva de calibración del brillo (0x10) o el contraste (0x12) de un monitor; sin calibrar, la identidad.
    static func calibration(_ monitor: String, _ code: UInt8) -> Curve {
        lock.lock(); defer { lock.unlock() }
        guard let e = cals[monitor.lowercased()] else { return .identity }
        return code == 0x10 ? e.b : code == 0x12 ? e.c : .identity
    }
}

/// Inicio con la sesión mediante un LaunchAgent del usuario.
enum Autostart {
    static let label = "io.github.abrahmch1.kast"
    static var plist: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }

    static var enabled: Bool { FileManager.default.fileExists(atPath: plist.path) }

    static func set(_ on: Bool) -> Bool {
        if !on { try? FileManager.default.removeItem(at: plist); return true }
        guard let exe = Bundle.main.executablePath else { return false }
        let dict: [String: Any] = ["Label": label, "ProgramArguments": [exe], "RunAtLoad": true, "ProcessType": "Interactive"]
        guard let data = try? PropertyListSerialization.data(fromPropertyList: dict, format: .xml, options: 0) else { return false }
        try? FileManager.default.createDirectory(at: plist.deletingLastPathComponent(), withIntermediateDirectories: true)
        return (try? data.write(to: plist)) != nil
    }
}
