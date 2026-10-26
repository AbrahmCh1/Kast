import Foundation
import UIKit

/// Equipo con Kast en la red (o agregado por IP).
struct Device: Identifiable, Equatable {
    let id: String
    let name: String
    let platform: String
    let addr: String
    let online: Bool
    let sharing: Bool
    let known: Bool
    /// (ancho, alto) de cada pantalla, de izquierda a derecha.
    let displays: [[Int]]
    /// ID de 9 dígitos ("482 913 057").
    var shortId: String = ""

    var platformName: String { platform == "mac" ? "Mac" : platform == "windows" ? "Windows" : platform == "ios" ? "iPhone/iPad" : "" }
    var symbol: String { platform == "mac" ? "desktopcomputer" : platform == "windows" ? "pc" : "display" }
}

/// Estado de la sesión en curso.
struct Session: Equatable {
    var id: UInt64
    var peer: String
    var platform: String
    var state: String = "connecting"
    var code: String = ""
    var control = false
    var width = 0
    var height = 0
    var error = ""
    var note = ""
    /// Pantalla del otro equipo que se ve y cuántas tiene (para cambiar de una a otra).
    var monitor = 1
    var displays: [[Int]] = []
}

/// Cómo se entra en el otro equipo.
enum AccessMode: String, CaseIterable, Identifiable {
    case approval, password, pair
    var id: String { rawValue }
    var label: String {
        switch self {
        case .approval: return "Pedir permiso"
        case .password: return "Contraseña de acceso desatendido"
        case .pair: return "Código de emparejamiento"
        }
    }
}

/// El equipo pidió contraseña o código: se vuelve a abrir la conexión con ellos.
struct AuthRequest: Identifiable {
    let id = UUID()
    let device: Device
    let monitor: Int
    let control: Bool
    /// La contraseña o el código no eran correctos (si no, el equipo sólo acepta emparejados o con contraseña).
    let failed: Bool
}

/// Recibe los fotogramas (desde un hilo en segundo plano).
protocol VideoSink: AnyObject {
    func enqueue(avcc: Data, sps: Data?, pps: Data?)
}

/// Envoltura del núcleo en Rust (kastlink.h). Los eventos se publican en el hilo principal.
final class LinkClient: ObservableObject {
    static let shared = LinkClient()

    @Published var devices: [Device] = []
    @Published var session: Session? {
        didSet { sidLock.lock(); currentSid = session?.id ?? 0; sidLock.unlock() }
    }
    /// Sesión a la que pertenecen los fotogramas que se muestran (al cambiar de pantalla, los de la anterior se descartan).
    private let sidLock = NSLock()
    private var currentSid: UInt64 = 0
    private func isCurrent(_ sid: UInt64) -> Bool { sidLock.lock(); defer { sidLock.unlock() }; return sid == currentSid }
    @Published var toast: String?
    /// El equipo pidió contraseña o código.
    @Published var authRequest: AuthRequest?
    /// Última conexión pedida (para repetirla con contraseña o código).
    private var lastConnect: (Device, Int, Bool, AccessMode)?

    /// Visor de la sesión activa (se asigna desde el hilo principal; se lee desde el de vídeo).
    private let sinkLock = NSLock()
    private weak var _sink: VideoSink?
    var sink: VideoSink? {
        get { sinkLock.lock(); defer { sinkLock.unlock() }; return _sink }
        set { sinkLock.lock(); _sink = newValue; sinkLock.unlock() }
    }

    private var started = false

    func start() {
        guard !started else { return }
        started = true
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("KastLink")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let ctx = Unmanaged.passUnretained(self).toOpaque()
        _ = kl_start(dir.path, UIDevice.current.name, ctx, { ctx, json in
            guard let ctx, let json else { return }
            let me = Unmanaged<LinkClient>.fromOpaque(ctx).takeUnretainedValue()
            let data = Data(bytes: json, count: strlen(json))
            DispatchQueue.main.async { me.handle(data) }
        }, { ctx, session, avcc, len, sps, spsLen, pps, ppsLen in
            guard let ctx, let avcc else { return }
            let me = Unmanaged<LinkClient>.fromOpaque(ctx).takeUnretainedValue()
            let frame = Data(bytes: avcc, count: len)
            let s = (sps != nil && spsLen > 0) ? Data(bytes: sps!, count: spsLen) : nil
            let p = (pps != nil && ppsLen > 0) ? Data(bytes: pps!, count: ppsLen) : nil
            guard me.isCurrent(session) else { return }
            me.sink?.enqueue(avcc: frame, sps: s, pps: p)
        })
        // Pruebas: conectar solo (KAST_AUTOCONNECT=id o ip:dirección:puerto).
        if let id = ProcessInfo.processInfo.environment["KAST_AUTOCONNECT"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                // KAST_AUTOAUTH=password:secreto o pair:código
                let auth = ProcessInfo.processInfo.environment["KAST_AUTOAUTH"]?.split(separator: ":", maxSplits: 1).map(String.init)
                let mode = auth.flatMap { AccessMode(rawValue: $0[0]) } ?? .approval
                self.connect(Device(id: id, name: id, platform: "", addr: "", online: true, sharing: true, known: false, displays: []),
                             monitor: 1, control: true, mode: mode, secret: auth?.count == 2 ? auth![1] : "")
            }
        }
    }

    /// Pruebas (KAST_AUTOTEST): una secuencia de entrada fija al empezar la sesión.
    private func autotest() {
        // KAST_AUTOSWITCH=n: a los 3 s cambia a la pantalla n (prueba del cambio de pantalla sin volver a preguntar).
        if let n = ProcessInfo.processInfo.environment["KAST_AUTOSWITCH"].flatMap(Int.init), session?.monitor != n {
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { self.switchScreen(to: n) }
        }
        guard ProcessInfo.processInfo.environment["KAST_AUTOTEST"] != nil else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            self.move(100, 80)
            self.button(0, true, 100, 80); self.button(0, false, 100, 80)
            self.wheel(0, -120)
            self.text("Hola ñ 😀")
            self.tap("Escape")
            self.key("SuperLeft", true); self.tap("KeyC"); self.key("SuperLeft", false)
            kl_clip_text(self.sid, "portapapeles desde iOS")
        }
    }

    private func handle(_ data: Data) {
        guard let v = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
        switch v["ev"] as? String {
        case "devices":
            let list = (v["devices"] as? [[String: Any]] ?? []).map { d in
                Device(id: d["id"] as? String ?? "", name: d["name"] as? String ?? "", platform: d["platform"] as? String ?? "",
                       addr: d["addr"] as? String ?? "", online: d["online"] as? Bool ?? false, sharing: d["sharing"] as? Bool ?? true,
                       known: d["known"] as? Bool ?? false, displays: d["displays"] as? [[Int]] ?? [], shortId: d["short_id"] as? String ?? "")
            }
            if list != devices { devices = list }
        case "session":
            guard let id = (v["session"] as? NSNumber)?.uint64Value, var s = session, s.id == id else { return }
            s.state = v["state"] as? String ?? s.state
            if let c = v["code"] as? String { s.code = c }
            if let c = v["control"] as? Bool { s.control = c }
            if let w = v["w"] as? Int { s.width = w }
            if let h = v["h"] as? Int { s.height = h }
            if let ds = v["displays"] as? [[Int]], !ds.isEmpty { s.displays = ds }
            if let e = v["error"] as? String { s.error = e }
            if let n = v["note"] as? String, !n.isEmpty { s.note = n; toast = "\(s.peer): \(n)" }
            if s.state == "ended", !s.error.isEmpty { toast = s.error }
            if s.state == "ended", v["need_auth"] as? Bool == true, let (d, m, c, mode) = lastConnect {
                authRequest = AuthRequest(device: d, monitor: m, control: c, failed: mode != .approval)
            }
            let wasLive = session?.state == "live"
            session = s
            if s.state == "live", !wasLive { autotest() }
        case "clip":
            if let t = v["text"] as? String {
                UIPasteboard.general.string = t
                toast = "Copiado del otro equipo"
            } else if let b = v["png"] as? String, let d = Data(base64Encoded: b), let img = UIImage(data: d) {
                UIPasteboard.general.image = img
                toast = "Imagen copiada del otro equipo"
            }
        case "toast":
            toast = v["msg"] as? String
        default:
            break
        }
    }

    // MARK: - Comandos

    private func command(_ obj: [String: Any]) {
        guard let d = try? JSONSerialization.data(withJSONObject: obj), let s = String(data: d, encoding: .utf8) else { return }
        kl_command(s)
    }

    func refresh() { command(["cmd": "refresh"]) }
    func add(_ addr: String) { command(["cmd": "add", "addr": addr]) }
    func forget(_ id: String) { command(["cmd": "forget", "id": id]) }

    func connect(_ d: Device, monitor: Int, control: Bool, mode: AccessMode = .approval, secret: String = "") {
        lastConnect = (d, monitor, control, mode)
        let id = mode == .approval
            ? kl_connect(d.id, UInt32(monitor), control)
            : kl_connect_auth(d.id, UInt32(monitor), control, mode.rawValue, secret)
        session = Session(id: id, peer: d.name, platform: d.platform, monitor: monitor, displays: d.displays)
    }

    /// Cambia a otra pantalla del mismo equipo: abre la nueva antes de cerrar la anterior, así el otro equipo no
    /// vuelve a preguntar (la petición incluye todas sus pantallas; ver link/src/agent.rs, Grant).
    func switchScreen(to monitor: Int) {
        guard let old = session, old.monitor != monitor, let (d, _, control, _) = lastConnect else { return }
        connect(d, monitor: monitor, control: control)
        command(["cmd": "disconnect", "session": old.id])
    }

    func disconnect() {
        guard let s = session else { return }
        command(["cmd": "disconnect", "session": s.id])
        session = nil
    }

    // MARK: - Entrada

    var sid: UInt64 { session?.id ?? 0 }
    func move(_ x: Int, _ y: Int) { kl_mouse_move(sid, Int32(x), Int32(y)) }
    func button(_ b: UInt8, _ down: Bool, _ x: Int, _ y: Int) { kl_mouse_button(sid, b, down, Int32(x), Int32(y)) }
    func wheel(_ dx: Int, _ dy: Int) { kl_wheel(sid, Int32(dx), Int32(dy)) }
    func key(_ code: String, _ down: Bool) { kl_key(sid, code, down) }
    func tap(_ code: String) { key(code, true); key(code, false) }
    func text(_ t: String) { kl_text(sid, t) }

    /// Envía el portapapeles de iOS (texto o imagen) al otro equipo.
    func sendClipboard() {
        let pb = UIPasteboard.general
        if let t = pb.string {
            kl_clip_text(sid, t)
            toast = "Portapapeles enviado"
        } else if let img = pb.image, let png = img.pngData() {
            png.withUnsafeBytes { kl_clip_png(sid, $0.bindMemory(to: UInt8.self).baseAddress, png.count) }
            toast = "Imagen enviada"
        } else {
            toast = "El portapapeles está vacío"
        }
    }
}
