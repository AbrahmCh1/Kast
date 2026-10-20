// DDC/CI en macOS.
// - Monitores externos en Apple Silicon: IOAVService (API privada de IOKit, la misma que usan
//   MonitorControl y m1ddc). En Macs Intel no hay DDC: sólo funciona el resto de Kast.
// - Pantalla integrada: DisplayServices (privada) para el brillo.
import Foundation
import IOKit
import CoreGraphics

private let iokitPath = "/System/Library/Frameworks/IOKit.framework/IOKit"
private typealias AVCreateFn = @convention(c) (CFAllocator?, io_service_t) -> Unmanaged<CFTypeRef>?
private typealias AVIOFn = @convention(c) (CFTypeRef, UInt32, UInt32, UnsafeMutableRawPointer, UInt32) -> IOReturn
private let avCreate = loadSymbol(iokitPath, "IOAVServiceCreateWithService", as: AVCreateFn.self)
private let avRead = loadSymbol(iokitPath, "IOAVServiceReadI2C", as: AVIOFn.self)
private let avWrite = loadSymbol(iokitPath, "IOAVServiceWriteI2C", as: AVIOFn.self)

private let dsPath = "/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices"
private typealias DSGetFn = @convention(c) (CGDirectDisplayID, UnsafeMutablePointer<Float>) -> Int32
private typealias DSSetFn = @convention(c) (CGDirectDisplayID, Float) -> Int32
private let dsGet = loadSymbol(dsPath, "DisplayServicesGetBrightness", as: DSGetFn.self)
private let dsSet = loadSymbol(dsPath, "DisplayServicesSetBrightness", as: DSSetFn.self)

enum BuiltinBrightness {
    /// 0...1
    static func get(_ id: CGDirectDisplayID) -> Float? {
        guard let f = dsGet else { return nil }
        var v: Float = 0
        return f(id, &v) == 0 ? v : nil
    }
    static func set(_ id: CGDirectDisplayID, _ v: Float) -> Bool {
        guard let f = dsSet else { return false }
        return f(id, max(0, min(1, v))) == 0
    }
}

/// Servicio AV externo con los datos del monitor al que pertenece.
struct AVEntry {
    let vendor: UInt32
    let model: UInt32
    let serial: UInt32
    let name: String
    let channel: DDCChannel
}

/// Canal DDC/CI hacia un monitor. Respeta ~60 ms entre órdenes: DDC/CI exige 50 ms o el monitor las ignora.
final class DDCChannel {
    private let service: CFTypeRef
    private var last: UInt64 = 0
    private let lock = NSLock()

    init(_ service: CFTypeRef) { self.service = service }

    private func pace() {
        let gap: UInt64 = 60
        let el = nowMs() - last
        if last != 0 && el < gap { usleep(UInt32((gap - el) * 1000)) }
    }

    func write(_ vcp: UInt8, _ value: UInt16) -> Bool {
        guard let w = avWrite else { return false }
        lock.lock(); defer { lock.unlock() }
        var d: [UInt8] = [0x84, 0x03, vcp, UInt8(value >> 8), UInt8(value & 0xFF), 0]
        d[5] = 0x6E ^ 0x51 ^ d[0] ^ d[1] ^ d[2] ^ d[3] ^ d[4]
        for _ in 0..<3 {
            pace()
            let r = d.withUnsafeMutableBytes { w(service, 0x37, 0x51, $0.baseAddress!, UInt32($0.count)) }
            last = nowMs()
            if r == KERN_SUCCESS { return true }
        }
        return false
    }

    func read(_ vcp: UInt8) -> (cur: Int, max: Int)? {
        guard let w = avWrite, let rd = avRead else { return nil }
        lock.lock(); defer { lock.unlock() }
        var req: [UInt8] = [0x82, 0x01, vcp, 0]
        req[3] = 0x6E ^ 0x51 ^ req[0] ^ req[1] ^ req[2]
        for _ in 0..<3 {
            pace()
            let wr = req.withUnsafeMutableBytes { w(service, 0x37, 0x51, $0.baseAddress!, UInt32($0.count)) }
            usleep(40_000)
            var buf = [UInt8](repeating: 0, count: 12)
            let rr = buf.withUnsafeMutableBytes { rd(service, 0x37, 0x51, $0.baseAddress!, UInt32($0.count)) }
            last = nowMs()
            guard wr == KERN_SUCCESS, rr == KERN_SUCCESS else { continue }
            // Respuesta: ... 0x02 (código) 0x00 (resultado) vcp tipo maxH maxL curH curL checksum
            for i in 0..<(buf.count - 7) where buf[i] == 0x02 && buf[i + 1] == 0x00 && buf[i + 2] == vcp {
                let mx = Int(buf[i + 4]) << 8 | Int(buf[i + 5])
                let cur = Int(buf[i + 6]) << 8 | Int(buf[i + 7])
                return (cur, mx)
            }
        }
        return nil
    }
}

enum AVDiscovery {
    /// Recorre el registro de IOKit: cada framebuffer (AppleCLCD2 / IOMobileFramebufferShim) publica los
    /// datos del monitor, y el siguiente DCPAVServiceProxy "External" es su canal I2C.
    static func scan() -> [AVEntry] {
        guard let create = avCreate else { return [] }
        var out: [AVEntry] = []
        var iter = io_iterator_t()
        let root = IORegistryGetRootEntry(kIOMainPortDefault)
        defer { IOObjectRelease(root) }
        guard IORegistryEntryCreateIterator(root, kIOServicePlane, IOOptionBits(kIORegistryIterateRecursively), &iter) == KERN_SUCCESS else { return [] }
        defer { IOObjectRelease(iter) }
        var attrs: (UInt32, UInt32, UInt32, String)?
        var entry = IOIteratorNext(iter)
        while entry != 0 {
            var nameBuf = [CChar](repeating: 0, count: 128)
            IORegistryEntryGetName(entry, &nameBuf)
            let name = String(cString: nameBuf)
            if name == "AppleCLCD2" || name == "IOMobileFramebufferShim" {
                if let da = IORegistryEntryCreateCFProperty(entry, "DisplayAttributes" as CFString, kCFAllocatorDefault, 0)?
                    .takeRetainedValue() as? [String: Any],
                   let pa = da["ProductAttributes"] as? [String: Any] {
                    attrs = (UInt32(truncatingIfNeeded: (pa["LegacyManufacturerID"] as? Int) ?? 0),
                             UInt32(truncatingIfNeeded: (pa["ProductID"] as? Int) ?? 0),
                             UInt32(truncatingIfNeeded: (pa["SerialNumber"] as? Int) ?? 0),
                             (pa["ProductName"] as? String) ?? "")
                }
            } else if name == "DCPAVServiceProxy" {
                let loc = IORegistryEntryCreateCFProperty(entry, "Location" as CFString, kCFAllocatorDefault, 0)?
                    .takeRetainedValue() as? String
                if loc == "External", let a = attrs, let svc = create(kCFAllocatorDefault, entry)?.takeRetainedValue() {
                    out.append(AVEntry(vendor: a.0, model: a.1, serial: a.2, name: a.3, channel: DDCChannel(svc)))
                    attrs = nil
                }
            }
            IOObjectRelease(entry)
            entry = IOIteratorNext(iter)
        }
        return out
    }
}
