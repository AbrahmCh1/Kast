// Lenguaje de comandos de Kast (idéntico al de Windows) y estado para la interfaz.
import AppKit
import CoreGraphics

enum Op {
    case get, set(Int), rel(Int)
}

/// Calibración: puntos (nivel de Kast en %, valor real en % de la escala del monitor), unidos por tramos rectos.
struct Curve {
    var k: [Int], r: [Int]
    static let identity = Curve(k: [0, 100], r: [0, 100])

    /// "0:10 50:40 100:85" (nivel:valor) o, como antes, "10 85" (valores para el 0 % y el 100 %). Los niveles van
    /// de 0 a 100 en orden y los valores no bajan; entre el primero y el último, al menos 5 puntos.
    init?(_ s: String) {
        let t = s.split(whereSeparator: { $0 == " " || $0 == "\t" })
        var k: [Int] = [], r: [Int] = []
        if s.contains(":") {
            for p in t {
                let kv = p.split(separator: ":")
                guard kv.count == 2, let a = Int(kv[0]), let b = Int(kv[1]), (0...100).contains(a), (0...100).contains(b) else { return nil }
                k.append(a); r.append(b)
            }
        } else {
            guard t.count == 2, let lo = Int(t[0]), let hi = Int(t[1]), lo >= 0, hi <= 100 else { return nil }
            k = [0, 100]; r = [lo, hi]
        }
        guard k.count >= 2, k.count <= 8, k.first == 0, k.last == 100, r.last! - r.first! >= 5 else { return nil }
        for i in 1..<k.count where k[i] <= k[i - 1] || r[i] < r[i - 1] { return nil }
        self.k = k
        self.r = r
    }

    private init(k: [Int], r: [Int]) { self.k = k; self.r = r }

    /// Valor real del monitor (en % de su escala) -> nivel de Kast.
    func toUser(_ raw: Int) -> Int {
        if raw <= r[0] { return 0 }
        for i in 0..<(k.count - 1) where raw <= r[i + 1] {
            let dr = r[i + 1] - r[i], dk = k[i + 1] - k[i]
            return k[i] + ((raw - r[i]) * dk + dr / 2) / dr
        }
        return 100
    }

    /// Nivel de Kast (0-100) -> valor real del monitor (en % de su escala).
    func toRaw(_ user: Int) -> Int {
        for i in 0..<(k.count - 1) where user <= k[i + 1] {
            let dk = k[i + 1] - k[i]
            return r[i] + ((user - k[i]) * (r[i + 1] - r[i]) + dk / 2) / dk
        }
        return r[r.count - 1]
    }
}

private typealias LockFn = @convention(c) () -> Int32
private let lockScreenFn = loadSymbol("/System/Library/PrivateFrameworks/login.framework/Versions/Current/login",
                                      "SACLockScreenImmediate", as: LockFn.self)

enum Commands {
    static let bundleID = "io.github.abrahmch1.kast"
    static let noteUI = Notification.Name("io.github.abrahmch1.kast.ui")
    static let noteReload = Notification.Name("io.github.abrahmch1.kast.reload")
    static let noteStop = Notification.Name("io.github.abrahmch1.kast.stop")

    /// Dentro de la app de la barra de menús, estos comandos actúan sobre ella directamente.
    static var uiHook: (() -> Void)?
    static var reloadHook: (() -> Void)?
    static var quitHook: (() -> Void)?

    static var mons: [Monitor] = []
    private static var cache: [String: (cur: Int, max: Int, t: UInt64)] = [:]
    private static let cacheLock = NSLock()
    private static var favDepth = 0

    static let inputs: [(String, Int)] = [("vga1", 0x01), ("vga2", 0x02), ("dvi1", 0x03), ("dvi2", 0x04), ("dp1", 0x0F), ("dp2", 0x10),
                                          ("hdmi1", 0x11), ("hdmi2", 0x12), ("usbc", 0x1B), ("usbc2", 0x1C), ("dp", 0x0F), ("hdmi", 0x11)]

    static func clearCache() { cacheLock.lock(); cache.removeAll(); cacheLock.unlock() }

    private static func put(_ key: String, _ cur: Int, _ max: Int) {
        cacheLock.lock(); cache[key] = (cur, max, nowMs()); cacheLock.unlock()
    }

    // MARK: selección de monitores

    /// "all" | "*" | "p" | "c" | "1" | "1,3"
    static func targets(_ s: String?) -> [Monitor]? {
        guard let s = s, !mons.isEmpty else { return nil }
        switch s.lowercased() {
        case "all", "*": return mons
        case "p":
            let p = mons.filter { $0.primary }
            return p.isEmpty ? nil : p
        case "c":
            guard let pt = CGEvent(source: nil)?.location else { return nil }
            var id: CGDirectDisplayID = 0
            var n: UInt32 = 0
            CGGetDisplaysWithPoint(pt, 1, &id, &n)
            let hit = mons.filter { $0.id == id }
            return n > 0 && !hit.isEmpty ? hit : nil
        default:
            var out: [Monitor] = []
            for part in s.split(separator: ",") {
                guard let n = Int(part), n >= 1, n <= mons.count else { return nil }
                if !out.contains(where: { $0 === mons[n - 1] }) { out.append(mons[n - 1]) }
            }
            return out.isEmpty ? nil : out
        }
    }

    private static func badTarget(_ s: String?) -> Int32 {
        say("monitor inválido: \(s ?? "") (1-\(mons.count), all, p, c)")
        return 1
    }

    // MARK: DDC / brillo

    /// pct: 0 = crudo, 1 = % (calibrado si es brillo o contraste), 2 = % sin calibrar. Devuelve el valor final por monitor.
    static func feature(_ list: [Monitor], code: UInt8, op: Op, pct: Int) -> [Int?] {
        var res = [Int?](repeating: nil, count: list.count)
        let lock = NSLock()
        DispatchQueue.concurrentPerform(iterations: list.count) { i in
            let v = featureOne(list[i], code: code, op: op, pct: pct)
            lock.lock(); res[i] = v; lock.unlock()
        }
        return res
    }

    private static func featureOne(_ m: Monitor, code: UInt8, op: Op, pct: Int) -> Int? {
        let cv = pct == 1 ? Config.calibration(m.name, code) : .identity
        if m.builtin {
            guard code == 0x10, let f = BuiltinBrightness.get(m.id) else { return nil }
            let rawpct = Int((f * 100).rounded())
            let base = pct > 0 ? cv.toUser(rawpct) : rawpct
            let nv: Int
            switch op {
            case .get: return base
            case .set(let v): nv = v
            case .rel(let d): nv = base + d
            }
            let c = min(100, max(0, nv))
            let raw = pct > 0 ? cv.toRaw(c) : c
            return BuiltinBrightness.set(m.id, Float(raw) / 100) ? c : nil
        }
        guard let ch = m.ddc else { return nil }
        let key = "\(m.id)-\(code)"
        var cur = 0, mx = 0, fresh = false
        cacheLock.lock()
        if let e = cache[key] {
            mx = e.max
            if nowMs() - e.t < 4000 { cur = e.cur; fresh = true }
        }
        cacheLock.unlock()
        let need: Bool
        switch op {
        case .get: need = true
        case .rel: need = !fresh
        case .set: need = pct > 0 && mx == 0
        }
        if need {
            guard let r = ch.read(code) else { return nil }
            cur = r.cur
            mx = r.max
            put(key, cur, mx)
        }
        let usePct = pct > 0 && mx > 0
        let base = usePct ? cv.toUser((cur * 100 + mx / 2) / mx) : cur
        let nv: Int
        switch op {
        case .get: return base
        case .set(let v): nv = v
        case .rel(let d): nv = base + d
        }
        let top = usePct ? 100 : (mx > 0 ? mx : 0xFFFF)
        let c = min(top, max(0, nv))
        let raw = usePct ? (cv.toRaw(c) * mx + 50) / 100 : c
        guard ch.write(code, UInt16(clamping: raw)) else { return nil }
        put(key, raw, mx)
        return c
    }

    private static func parseValue(_ s: String) -> Op? {
        let rel = s.hasPrefix("+") || s.hasPrefix("-")
        let body = s.hasPrefix("+") ? String(s.dropFirst()) : s
        let v: Int?
        if body.lowercased().hasPrefix("0x") { v = Int(body.dropFirst(2), radix: 16) } else { v = Int(body) }
        guard let n = v else { return nil }
        return rel ? .rel(n) : .set(n)
    }

    private static func inputName(_ v: Int) -> String { inputs.first { $0.1 == v & 0xFF }?.0 ?? "?" }

    private static func report(_ list: [Monitor], _ vals: [Int?], code: UInt8, pct: Bool, get: Bool) -> Int32 {
        var rc: Int32 = 0
        for (m, v) in zip(list, vals) {
            guard let v = v else { say("\(m.index) \(m.name): sin respuesta DDC/CI"); rc = 1; continue }
            if code == 0x60 { say("\(m.index) \(m.name): \(inputName(v)) (0x\(String(format: "%02X", v & 0xFF)))") }
            else if pct { say("\(m.index) \(m.name): \(v)%") }
            else { say("\(m.index) \(m.name): \(v)\(get ? "" : "")") }
        }
        return rc
    }

    private static func featureCmd(_ av: [String], _ code: UInt8, _ pct: Int) -> Int32 {
        let t = av.count > 1 ? av[1] : "all"
        guard let list = targets(t) else { return badTarget(t) }
        var op = Op.get
        if av.count > 2 {
            guard let o = parseValue(av[2]) else { say("valor inválido: \(av[2])"); return 1 }
            op = o
        }
        let isGet: Bool
        if case .get = op { isGet = true } else { isGet = false }
        return report(list, feature(list, code: code, op: op, pct: pct), code: code, pct: pct > 0, get: isGet)
    }

    // MARK: intérprete

    /// Ejecuta "cmd a b; cmd2 x".
    @discardableResult
    static func exec(_ line: String) -> Int32 {
        var rc: Int32 = 0
        for seg in line.split(separator: ";") {
            let av = seg.split(whereSeparator: { $0 == " " || $0 == "\t" || $0 == "\n" }).map(String.init)
            if !av.isEmpty { rc |= run(av) }
        }
        return rc
    }

    private static var appRunning: Bool {
        NSRunningApplication.runningApplications(withBundleIdentifier: bundleID).contains { $0.processIdentifier != getpid() }
    }

    private static func post(_ n: Notification.Name) {
        DistributedNotificationCenter.default().postNotificationName(n, object: nil, userInfo: nil, deliverImmediately: true)
    }

    private static func launchApp(_ extra: [String]) -> Int32 {
        let bundle = Bundle.main.bundleURL
        guard bundle.pathExtension == "app" else { say("Kast.app no encontrado (ejecuta kast desde dentro de Kast.app)"); return 1 }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        p.arguments = ["-a", bundle.path] + (extra.isEmpty ? [] : ["--args"] + extra)
        do { try p.run(); p.waitUntilExit() } catch { return 1 }
        return p.terminationStatus == 0 ? 0 : 1
    }

    static func run(_ av: [String]) -> Int32 {
        let c = av[0].lowercased()
        switch c {
        case "help", "-h", "--help": say(help); return 0
        case "version", "--version", "-v": say("kast \(kastVersion)"); return 0
        case "sleep":
            usleep(UInt32(max(0, Int(av.count > 1 ? av[1] : "0") ?? 0)) * 1000)
            return 0
        case "off":
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/usr/bin/pmset")
            p.arguments = ["displaysleepnow"]
            do { try p.run(); p.waitUntilExit() } catch { return 1 }
            return p.terminationStatus == 0 ? 0 : 1
        case "lock":
            guard let f = lockScreenFn else { say("no se pudo bloquear la sesión"); return 1 }
            _ = f()
            return 0
        case "daemon":
            if uiHook != nil || appRunning { say("el servicio ya está activo"); return 0 }
            return launchApp([])
        case "ui":
            if let h = uiHook { DispatchQueue.main.async(execute: h); return 0 }
            if appRunning { post(noteUI); return 0 }
            return launchApp(["--ui"])
        case "stop":
            if let h = quitHook { DispatchQueue.main.async(execute: h); return 0 }
            guard appRunning else { say("el servicio no está activo"); return 1 }
            post(noteStop); say("servicio detenido"); return 0
        case "reload":
            if let h = reloadHook { DispatchQueue.main.async(execute: h); return 0 }
            guard appRunning else { say("el servicio no está activo"); return 1 }
            post(noteReload); say("configuración recargada (ver kast.log)"); return 0
        case "status":
            let on = uiHook != nil || appRunning
            say(on ? "activo" : "detenido"); return on ? 0 : 1
        case "autostart":
            guard av.count > 1, ["on", "off"].contains(av[1].lowercased()) else { say("uso: autostart on|off"); return 1 }
            let ok = Autostart.set(av[1].lowercased() == "on")
            say(ok ? "ok" : "error"); return ok ? 0 : 1
        case "topo":
            switch av.count > 1 ? av[1].lowercased() : "" {
            case "extend":
                if !Displays.anyMirrored { say("las pantallas ya están extendidas"); return 0 }
                return Displays.unmirrorAll() ? 0 : 1
            case "clone":
                return Displays.mirrorAll() ? 0 : 1
            default:
                say("uso: topo extend|clone"); return 1
            }
        case "layout": return layoutCmd(av)
        case "fav": return favCmd(av)
        case "rot", "disable", "enable":
            say("\(c): no disponible en macOS"); return 1
        default: break
        }

        Config.ensureLoaded()
        mons = Displays.list()
        switch c {
        case "list", "ls": return listCmd()
        case "b", "bright": return featureCmd(av, 0x10, 1)
        case "braw": return featureCmd(av, 0x10, 2)
        case "craw": return featureCmd(av, 0x12, 2)
        case "c", "contrast": return featureCmd(av, 0x12, 1)
        case "vol": return featureCmd(av, 0x62, 1)
        case "vcp":
            guard av.count >= 3 else { say("uso: vcp <mon> <código> [valor]"); return 1 }
            let s = av[2].lowercased()
            guard let code = s.hasPrefix("0x") ? UInt8(s.dropFirst(2), radix: 16) : UInt8(s) else { say("código inválido: \(av[2])"); return 1 }
            return featureCmd([av[0], av[1]] + (av.count > 3 ? [av[3]] : []), code, 0)
        case "input":
            guard av.count >= 2 else { say("uso: input <mon> [hdmi1|hdmi2|dp1|dp2|usbc|dvi1|vga1|0xNN]"); return 1 }
            guard let list = targets(av[1]) else { return badTarget(av[1]) }
            if av.count < 3 { return report(list, feature(list, code: 0x60, op: .get, pct: 0), code: 0x60, pct: false, get: true) }
            let s = av[2].lowercased()
            guard let v = inputs.first(where: { $0.0 == s })?.1 ?? (s.hasPrefix("0x") ? Int(s.dropFirst(2), radix: 16) : Int(s)), v > 0
            else { say("fuente inválida: \(av[2])"); return 1 }
            return report(list, feature(list, code: 0x60, op: .set(v), pct: 0), code: 0x60, pct: false, get: false)
        case "power":
            guard av.count >= 3 else { say("uso: power <mon> on|standby|off|hardoff"); return 1 }
            guard let list = targets(av[1]) else { return badTarget(av[1]) }
            let states = ["on", "standby", "suspend", "off", "hardoff"]
            guard let i = states.firstIndex(of: av[2].lowercased()) else { say("estado inválido: \(av[2])"); return 1 }
            return report(list, feature(list, code: 0xD6, op: .set(i + 1), pct: 0), code: 0xD6, pct: false, get: false)
        case "mode": return modeCmd(av)
        case "modes":
            guard let list = targets(av.count > 1 ? av[1] : "p"), list.count == 1 else { return badTarget(av.count > 1 ? av[1] : nil) }
            var lines: [String] = []
            for m in Displays.modeList(list[0].id) {
                let parts = m.split(separator: "@")
                let res = String(parts[0]), hz = String(parts[1])
                if let last = lines.last, last.hasPrefix(res + " @") { lines[lines.count - 1] += " " + hz } else { lines.append("\(res) @ \(hz)") }
            }
            lines.forEach(say)
            return 0
        case "primary":
            guard let list = targets(av.count > 1 ? av[1] : nil), list.count == 1 else { return badTarget(av.count > 1 ? av[1] : nil) }
            if list[0].primary { return 0 }
            return Displays.setPrimary(list[0], mons) ? 0 : 1
        default:
            say("comando desconocido: \(av[0])  (kast help)")
            return 1
        }
    }

    private static func modeCmd(_ av: [String]) -> Int32 {
        guard av.count >= 3 else { say("uso: mode <mon> AxB[@Hz] | @Hz"); return 1 }
        guard let list = targets(av[1]) else { return badTarget(av[1]) }
        let spec = av[2].lowercased()
        let parts = spec.split(separator: "@", omittingEmptySubsequences: false)
        var w: Int?, h: Int?
        if !parts[0].isEmpty {
            let wh = parts[0].split(separator: "x")
            guard wh.count == 2, let a = Int(wh[0]), let b = Int(wh[1]) else { say("modo inválido: \(av[2])"); return 1 }
            w = a; h = b
        }
        let hz = parts.count > 1 ? Int(parts[1]) : nil
        var rc: Int32 = 0
        for m in list {
            guard let mode = Displays.findMode(m.id, w: w, h: h, hz: hz) else { say("\(m.index) \(m.name): modo no soportado"); rc = 1; continue }
            if !Displays.setMode(m.id, mode) { say("\(m.index) \(m.name): fallo al aplicar"); rc = 1 }
        }
        return rc
    }

    private static func layoutCmd(_ av: [String]) -> Int32 {
        if av.count >= 3 && !Layouts.validName(av[2]) { say("nombre inválido: \(av[2])"); return 1 }
        let sub = av.count > 1 ? av[1].lowercased() : "list"
        switch sub {
        case "save" where av.count >= 3:
            guard Layouts.save(av[2]) else { say("layout: error al guardar"); return 1 }
            say("layout '\(av[2])' guardado"); return 0
        case "load" where av.count >= 3:
            guard let ok = Layouts.load(av[2]) else { say("layout '\(av[2])' no existe"); return 1 }
            return ok ? 0 : 1
        case "del" where av.count >= 3:
            return Layouts.delete(av[2]) ? 0 : 1
        case "list":
            Layouts.names().forEach(say); return 0
        default:
            say("uso: layout [list] | layout save|load|del <nombre>"); return 1
        }
    }

    private static func favCmd(_ av: [String]) -> Int32 {
        Config.ensureLoaded()
        let name = av.dropFirst().joined(separator: " ")
        if name.isEmpty {
            Config.lock.lock(); let all = Config.favs; Config.lock.unlock()
            for f in all { say("\(f.name) = \(f.act)") }
            return 0
        }
        guard let act = Config.favorite(name) else { say("favorito no encontrado: \(name)"); return 1 }
        guard favDepth <= 4 else { say("favoritos anidados demasiado profundo"); return 1 }
        favDepth += 1
        defer { favDepth -= 1 }
        return exec(act)
    }

    private static func listCmd() -> Int32 {
        if mons.isEmpty { say("sin monitores"); return 1 }
        let b = feature(mons, code: 0x10, op: .get, pct: 1)
        func pad(_ s: String, _ n: Int) -> String { s.count >= n ? s : s + String(repeating: " ", count: n - s.count) }
        say("#   \(pad("Monitor", 24)) \(pad("Modo", 16)) \(pad("Posición", 12)) Brillo")
        for (i, m) in mons.enumerated() {
            let cm = Displays.currentMode(m.id)
            let mirror = m.mirrorOf != kCGNullDirectDisplay ? " (espejo)" : ""
            say("\(m.index)\(m.primary ? "*" : " ")  \(pad(m.name, 24)) \(pad("\(cm.w)x\(cm.h)@\(cm.hz)", 16)) \(pad("\(Int(m.bounds.minX)),\(Int(m.bounds.minY))", 12)) \(b[i].map { "\($0)%" } ?? "-")\(mirror)")
        }
        return 0
    }

    /// Brillo de cada monitor para el menú de la barra (rápido: sólo lee el brillo).
    static func quickBrightness() -> [(index: Int, name: String, value: Int)] {
        Config.ensureLoaded()
        mons = Displays.list()
        let v = feature(mons, code: 0x10, op: .get, pct: 1)
        return mons.enumerated().compactMap { i, m in v[i].map { (m.index, m.name, $0) } }
    }

    /// Estado completo para la interfaz (mismo formato que kastd.exe en Windows).
    static func state(log: String) -> [String: Any] {
        Config.ensureLoaded()
        mons = Displays.list()
        var vals = [[Int?]](repeating: [nil, nil, nil, nil, nil], count: mons.count)   // b, c, in, braw, craw
        let lock = NSLock()
        DispatchQueue.concurrentPerform(iterations: mons.count) { i in
            let m = mons[i]
            let cb = Config.calibration(m.name, 0x10), cc = Config.calibration(m.name, 0x12)
            var r: [Int?] = [nil, nil, nil, nil, nil]
            func pct(_ x: (cur: Int, max: Int)) -> Int { x.max > 0 ? (x.cur * 100 + x.max / 2) / x.max : x.cur }
            if m.builtin {
                if let f = BuiltinBrightness.get(m.id) { let p = Int((f * 100).rounded()); r[0] = cb.toUser(p); r[3] = p }
            } else if let ch = m.ddc {
                if let b = ch.read(0x10) { r[0] = cb.toUser(pct(b)); r[3] = pct(b); put("\(m.id)-16", b.cur, b.max) }
                if let c = ch.read(0x12) { r[1] = cc.toUser(pct(c)); r[4] = pct(c); put("\(m.id)-18", c.cur, c.max) }
                if let x = ch.read(0x60) { r[2] = x.cur & 0xFF }
            }
            lock.lock(); vals[i] = r; lock.unlock()
        }
        func json(_ v: Int?) -> Any { v.map { $0 as Any } ?? NSNull() }
        let list: [[String: Any]] = mons.enumerated().map { i, m in
            let cm = Displays.currentMode(m.id)
            return ["i": m.index, "name": m.name, "dev": "ID \(m.id)", "primary": m.primary,
                    "x": Int(m.bounds.minX), "y": Int(m.bounds.minY), "w": Int(m.bounds.width), "h": Int(m.bounds.height),
                    "pw": cm.w, "ph": cm.h, "hz": cm.hz, "rot": Int(CGDisplayRotation(m.id)),
                    "b": json(vals[i][0]), "c": json(vals[i][1]), "in": json(vals[i][2]), "braw": vals[i][3] ?? 0,
                    "craw": vals[i][4] ?? 0, "modes": Displays.modeList(m.id)]
        }
        let ini = (try? String(contentsOf: Paths.ini, encoding: .utf8)) ?? ""
        return ["t": "state", "platform": "mac", "mons": list, "layouts": Layouts.names(), "autostart": Autostart.enabled,
                "vanish": false, "dir": Paths.support.path, "log": log, "ini": ini]
    }

    static let help = """
    kast \(kastVersion) — control de monitores (macOS)
      list                        monitores (1..n de izquierda a derecha, * = principal)
      b   [mon] [valor]           brillo     (valor: 0-100 | +10 | -10)
      braw [mon] [valor]          brillo en la escala real del monitor (sin calibración)
      c   [mon] [valor]           contraste
      craw [mon] [valor]          contraste en la escala real del monitor (sin calibración)
      vol [mon] [valor]           volumen de los altavoces del monitor
      input <mon> [fuente]        hdmi1 hdmi2 dp1 dp2 usbc dvi1 vga1 | 0xNN
      power <mon> on|standby|off|hardoff   energía vía DDC/CI
      vcp <mon> <código> [valor]  código VCP crudo (0x10 o 16)
      off                         apaga las pantallas (reposo)
      lock                        bloquea la sesión
      mode <mon> AxB[@Hz] | @Hz   resolución / frecuencia
      modes [mon]                 modos disponibles
      primary <mon>               establecer como principal
      topo extend|clone           extender o duplicar todas las pantallas
      layout [list] | layout save|load|del <nombre>   disposición de pantallas
      fav [nombre]                ejecutar un favorito (sin nombre: listar)
      sleep <ms>                  pausa (para secuencias)
      ui                          abrir la interfaz
      daemon | stop | reload | status   app de la barra de menús
      autostart on|off            iniciar Kast con la sesión

    mon: 1 | 1,3 | all | p (principal) | c (el que tiene el cursor)
    secuencias con ';'   kast "b all 30; c all 50"
    DDC/CI (brillo, contraste, entrada) en monitores externos requiere Apple Silicon.
    Configuración: ~/Library/Application Support/Kast/kast.ini
    """
}
