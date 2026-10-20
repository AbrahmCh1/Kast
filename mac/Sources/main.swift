// Kast para macOS. Con argumentos funciona como CLI (`kast b all 50`); sin ellos, como app de la barra de menús.
import AppKit

let cliArgs = CommandLine.arguments.dropFirst().filter { !$0.hasPrefix("-psn") && $0 != "--ui" }

if !cliArgs.isEmpty {
    exit(Commands.exec(cliArgs.joined(separator: " ")))
}

// Una sola instancia: si ya está en marcha, se le pide que abra su ventana.
let others = NSRunningApplication.runningApplications(withBundleIdentifier: Commands.bundleID)
    .filter { $0.processIdentifier != getpid() }
if !others.isEmpty {
    DistributedNotificationCenter.default().postNotificationName(Commands.noteUI, object: nil, userInfo: nil, deliverImmediately: true)
    exit(0)
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
