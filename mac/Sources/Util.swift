// Kast para macOS — utilidades compartidas: salida capturable, rutas y JSON.
import Foundation

enum Output {
    /// Cuando no es nil, `say` acumula aquí en lugar de imprimir (lo usa la interfaz).
    static var capture: String?

    static func say(_ s: String) {
        if capture != nil { capture! += s + "\n" } else { print(s); fflush(stdout) }
    }
}

func say(_ s: String) { Output.say(s) }

enum Paths {
    static let support: URL = {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let u = base.appendingPathComponent("Kast", isDirectory: true)
        try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
        return u
    }()
    static var ini: URL { support.appendingPathComponent("kast.ini") }
    static var log: URL { support.appendingPathComponent("kast.log") }
    static var layouts: URL {
        let u = support.appendingPathComponent("layouts", isDirectory: true)
        try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
        return u
    }

    /// Crea kast.ini a partir del de ejemplo incluido en la app la primera vez.
    static func ensureIni() {
        guard !FileManager.default.fileExists(atPath: ini.path) else { return }
        if let def = Bundle.main.url(forResource: "kast", withExtension: "ini") {
            try? FileManager.default.copyItem(at: def, to: ini)
        } else {
            try? "[favoritos]\n\n[atajos]\n".write(to: ini, atomically: true, encoding: .utf8)
        }
    }
}

func toJSON(_ obj: Any) -> String {
    guard let d = try? JSONSerialization.data(withJSONObject: obj), let s = String(data: d, encoding: .utf8) else { return "{}" }
    return s
}

/// Reloj monotónico en milisegundos.
func nowMs() -> UInt64 { DispatchTime.now().uptimeNanoseconds / 1_000_000 }

/// Ejecuta en el hilo principal (AppKit) y devuelve el resultado.
func onMain<T>(_ f: () -> T) -> T {
    if Thread.isMainThread { return f() }
    return DispatchQueue.main.sync(execute: f)
}

/// Carga un símbolo de un framework (APIs privadas o no presentes en el SDK).
func loadSymbol<T>(_ lib: String, _ name: String, as type: T.Type) -> T? {
    guard let h = dlopen(lib, RTLD_NOW), let p = dlsym(h, name) else { return nil }
    return unsafeBitCast(p, to: type)
}
