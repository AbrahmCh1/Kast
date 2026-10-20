// Monitores en macOS: enumeración, modos, principal, espejo y disposiciones (CoreGraphics).
import Foundation
import CoreGraphics

final class Monitor {
    let id: CGDirectDisplayID
    var index = 0
    var name: String
    let bounds: CGRect
    let primary: Bool
    let builtin: Bool
    var ddc: DDCChannel?

    init(id: CGDirectDisplayID, name: String) {
        self.id = id
        self.name = name
        bounds = CGDisplayBounds(id)
        primary = CGDisplayIsMain(id) != 0
        builtin = CGDisplayIsBuiltin(id) != 0
    }

    var mirrorOf: CGDirectDisplayID { CGDisplayMirrorsDisplay(id) }
    var hasControls: Bool { builtin ? BuiltinBrightness.get(id) != nil : ddc != nil }
}

private typealias InfoDictFn = @convention(c) (CGDirectDisplayID) -> Unmanaged<CFDictionary>?
private let coreDisplayInfo = loadSymbol("/System/Library/Frameworks/CoreDisplay.framework/CoreDisplay",
                                         "CoreDisplay_DisplayCreateInfoDictionary", as: InfoDictFn.self)

enum Displays {
    private static var avCache: [AVEntry]?

    /// Se llama cuando cambia la configuración de pantallas.
    static func invalidate() { avCache = nil }

    static func onlineIDs() -> [CGDirectDisplayID] {
        var n: UInt32 = 0
        CGGetOnlineDisplayList(0, nil, &n)
        var ids = [CGDirectDisplayID](repeating: 0, count: Int(n))
        CGGetOnlineDisplayList(n, &ids, &n)
        return Array(ids.prefix(Int(n)))
    }

    static func productName(_ id: CGDirectDisplayID) -> String? {
        guard let f = coreDisplayInfo, let d = f(id)?.takeRetainedValue() as? [String: Any],
              let names = d["DisplayProductName"] as? [String: String] else { return nil }
        return names["es_ES"] ?? names["en_US"] ?? names.values.first
    }

    /// Monitores ordenados de izquierda a derecha (1..n), con su canal DDC si lo tienen.
    static func list() -> [Monitor] {
        if avCache == nil { avCache = AVDiscovery.scan() }
        var free = avCache ?? []
        var mons = onlineIDs().map { id -> Monitor in
            let builtin = CGDisplayIsBuiltin(id) != 0
            return Monitor(id: id, name: builtin ? "Pantalla integrada" : (productName(id) ?? "Pantalla \(id)"))
        }
        mons.sort { $0.bounds.minX != $1.bounds.minX ? $0.bounds.minX < $1.bounds.minX : $0.bounds.minY < $1.bounds.minY }
        // Empareja cada monitor externo con su servicio AV: fabricante+modelo+serie, luego fabricante+modelo, luego orden.
        let ext = mons.filter { !$0.builtin }
        for pass in 0..<3 {
            for m in ext where m.ddc == nil {
                let v = CGDisplayVendorNumber(m.id), p = CGDisplayModelNumber(m.id), s = CGDisplaySerialNumber(m.id)
                let idx = free.firstIndex { e in
                    switch pass {
                    case 0: return e.vendor == v && e.model == p && e.serial == s
                    case 1: return e.vendor == v && e.model == p
                    default: return true
                    }
                }
                if let i = idx {
                    m.ddc = free[i].channel
                    if m.name.hasPrefix("Pantalla "), !free[i].name.isEmpty { m.name = free[i].name }
                    free.remove(at: i)
                }
            }
        }
        for (i, m) in mons.enumerated() { m.index = i + 1 }
        return mons
    }

    // MARK: modos

    static func allModes(_ id: CGDirectDisplayID) -> [CGDisplayMode] {
        let opts = [kCGDisplayShowDuplicateLowResolutionModes: kCFBooleanTrue] as CFDictionary
        return (CGDisplayCopyAllDisplayModes(id, opts) as? [CGDisplayMode] ?? []).filter { $0.isUsableForDesktopGUI() }
    }

    static func hz(_ m: CGDisplayMode) -> Int { m.refreshRate > 0 ? Int(m.refreshRate.rounded()) : 60 }

    /// "1920x1080@60", ordenados de mayor a menor y sin duplicados.
    static func modeList(_ id: CGDirectDisplayID) -> [String] {
        var seen = Set<String>()
        let sorted = allModes(id).sorted { ($0.width, $0.height, hz($0)) > ($1.width, $1.height, hz($1)) }
        return sorted.compactMap { m in
            let s = "\(m.width)x\(m.height)@\(hz(m))"
            return seen.insert(s).inserted ? s : nil
        }
    }

    static func currentMode(_ id: CGDirectDisplayID) -> (w: Int, h: Int, hz: Int) {
        guard let m = CGDisplayCopyDisplayMode(id) else { return (Int(CGDisplayPixelsWide(id)), Int(CGDisplayPixelsHigh(id)), 60) }
        return (m.width, m.height, hz(m))
    }

    /// Elige el modo pedido; entre iguales prefiere HiDPI (más nítido).
    static func findMode(_ id: CGDirectDisplayID, w: Int?, h: Int?, hz want: Int?, pixelWidth: Int? = nil) -> CGDisplayMode? {
        let cur = currentMode(id)
        let tw = w ?? cur.w, th = h ?? cur.h
        let c = allModes(id).filter { $0.width == tw && $0.height == th && (want == nil || hz($0) == want!) }
        if let pw = pixelWidth, let exact = c.first(where: { $0.pixelWidth == pw }) { return exact }
        return c.max { ($0.pixelWidth, $0.refreshRate) < ($1.pixelWidth, $1.refreshRate) }
    }

    static func configure(_ body: (CGDisplayConfigRef?) -> Void) -> Bool {
        var cfg: CGDisplayConfigRef?
        guard CGBeginDisplayConfiguration(&cfg) == .success else { return false }
        body(cfg)
        let r = CGCompleteDisplayConfiguration(cfg, .permanently)
        invalidate()
        return r == .success
    }

    static func setMode(_ id: CGDirectDisplayID, _ mode: CGDisplayMode) -> Bool {
        configure { CGConfigureDisplayWithDisplayMode($0, id, mode, nil) }
    }

    /// El principal es el que está en el origen (0,0): se desplazan todos.
    static func setPrimary(_ target: Monitor, _ mons: [Monitor]) -> Bool {
        let px = target.bounds.minX, py = target.bounds.minY
        return configure { cfg in
            for m in mons where m.mirrorOf == kCGNullDirectDisplay {
                CGConfigureDisplayOrigin(cfg, m.id, Int32(m.bounds.minX - px), Int32(m.bounds.minY - py))
            }
        }
    }

    static func mirrorAll() -> Bool {
        let main = CGMainDisplayID()
        return configure { cfg in
            for id in onlineIDs() where id != main { CGConfigureDisplayMirrorOfDisplay(cfg, id, main) }
        }
    }

    static func unmirrorAll() -> Bool {
        configure { cfg in
            for id in onlineIDs() { CGConfigureDisplayMirrorOfDisplay(cfg, id, kCGNullDirectDisplay) }
        }
    }

    static var anyMirrored: Bool { onlineIDs().contains { CGDisplayMirrorsDisplay($0) != kCGNullDirectDisplay } }
}

// MARK: disposiciones guardadas

struct SavedDisplay: Codable {
    var vendor: UInt32
    var model: UInt32
    var serial: UInt32
    var name: String
    var x: Int32
    var y: Int32
    var w: Int
    var h: Int
    var pw: Int
    var hz: Int
    var mirrorOf: Int?   // índice dentro de la lista guardada
}

enum Layouts {
    static func url(_ name: String) -> URL { Paths.layouts.appendingPathComponent(name + ".json") }

    static func names() -> [String] {
        let files = (try? FileManager.default.contentsOfDirectory(atPath: Paths.layouts.path)) ?? []
        return files.filter { $0.hasSuffix(".json") }.map { String($0.dropLast(5)) }.sorted()
    }

    static func validName(_ n: String) -> Bool {
        !n.isEmpty && n.rangeOfCharacter(from: CharacterSet(charactersIn: "/\\:*?\"<>|")) == nil
    }

    static func save(_ name: String) -> Bool {
        let ids = Displays.onlineIDs()
        let list: [SavedDisplay] = ids.map { id in
            let b = CGDisplayBounds(id)
            let mode = CGDisplayCopyDisplayMode(id)
            let master = CGDisplayMirrorsDisplay(id)
            return SavedDisplay(vendor: CGDisplayVendorNumber(id), model: CGDisplayModelNumber(id), serial: CGDisplaySerialNumber(id),
                                name: Displays.productName(id) ?? "", x: Int32(b.minX), y: Int32(b.minY),
                                w: mode?.width ?? Int(b.width), h: mode?.height ?? Int(b.height), pw: mode?.pixelWidth ?? 0,
                                hz: mode.map(Displays.hz) ?? 60,
                                mirrorOf: master == kCGNullDirectDisplay ? nil : ids.firstIndex(of: master))
        }
        guard let data = try? JSONEncoder().encode(list) else { return false }
        return (try? data.write(to: url(name))) != nil
    }

    static func load(_ name: String) -> Bool? {
        guard let data = try? Data(contentsOf: url(name)), let saved = try? JSONDecoder().decode([SavedDisplay].self, from: data) else { return nil }
        var free = Displays.onlineIDs()
        var map: [Int: CGDirectDisplayID] = [:]
        for pass in 0..<3 {
            for (i, s) in saved.enumerated() where map[i] == nil {
                let hit = free.firstIndex { id in
                    let v = CGDisplayVendorNumber(id), m = CGDisplayModelNumber(id)
                    switch pass {
                    case 0: return v == s.vendor && m == s.model && CGDisplaySerialNumber(id) == s.serial
                    case 1: return v == s.vendor && m == s.model
                    default: return Displays.productName(id) == s.name
                    }
                }
                if let k = hit { map[i] = free[k]; free.remove(at: k) }
            }
        }
        return Displays.configure { cfg in
            for (i, s) in saved.enumerated() {
                guard let id = map[i] else { continue }
                if let mode = Displays.findMode(id, w: s.w, h: s.h, hz: s.hz, pixelWidth: s.pw) {
                    CGConfigureDisplayWithDisplayMode(cfg, id, mode, nil)
                }
                let master = s.mirrorOf.flatMap { map[$0] } ?? kCGNullDirectDisplay
                CGConfigureDisplayMirrorOfDisplay(cfg, id, master)
                if master == kCGNullDirectDisplay { CGConfigureDisplayOrigin(cfg, id, s.x, s.y) }
            }
        }
    }

    static func delete(_ name: String) -> Bool { (try? FileManager.default.removeItem(at: url(name))) != nil }
}
