// App de la barra de menús: menú con brillo por monitor, atajos globales (Carbon) y ventana de
// configuración (la misma ui.html que en Windows, en un WKWebView que sólo existe mientras está abierta).
import AppKit
import Carbon
import WebKit

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate, NSWindowDelegate, WKScriptMessageHandler {
    static var shared: AppDelegate?

    private var item: NSStatusItem!
    private var window: NSWindow?
    private var web: WKWebView?
    /// Todo comando se ejecuta aquí, en serie, para no bloquear la interfaz.
    private let worker = DispatchQueue(label: "io.github.abrahmch1.kast.worker", qos: .userInitiated)
    private var hotkeyRefs: [EventHotKeyRef] = []
    private var paused = false
    private var log = ""
    private var menuMons: [(index: Int, name: String, value: Int)] = []
    private var sliders: [Int: NSSlider] = [:]
    private var pending: [Int: Int] = [:]
    private let pendingLock = NSLock()
    // Kast Link: agente en segundo plano (stdin/stdout, una línea JSON por mensaje).
    private var linkProc: Process?
    private var linkIn: FileHandle?
    private var linkBuf = Data()
    private var linkRestarts = 0
    private var linkMissing = false
    private var quitting = false

    // MARK: arranque

    func applicationDidFinishLaunching(_ notification: Notification) {
        AppDelegate.shared = self
        Commands.uiHook = { [weak self] in self?.openUI() }
        Commands.reloadHook = { [weak self] in self?.reload() }
        Commands.quitHook = { NSApp.terminate(nil) }
        buildMainMenu()

        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let img = NSImage(systemSymbolName: "sun.max", accessibilityDescription: "Kast") {
            img.isTemplate = true
            item.button?.image = img
        }
        item.button?.toolTip = "Kast — pantallas"
        let menu = NSMenu()
        menu.delegate = self
        item.menu = menu

        installHotkeyHandler()
        reload()

        let dnc = DistributedNotificationCenter.default()
        dnc.addObserver(self, selector: #selector(noteUI), name: Commands.noteUI, object: nil)
        dnc.addObserver(self, selector: #selector(noteReload), name: Commands.noteReload, object: nil)
        dnc.addObserver(self, selector: #selector(noteStop), name: Commands.noteStop, object: nil)
        CGDisplayRegisterReconfigurationCallback({ _, flags, _ in
            if flags.contains(.beginConfigurationFlag) { return }
            DispatchQueue.main.async { AppDelegate.shared?.displaysChanged() }
        }, nil)

        refreshMenuValues()
        signal(SIGPIPE, SIG_IGN)   // escribir al agente si acaba de terminar no debe cerrar la app
        linkStart()
        if CommandLine.arguments.contains("--ui") { openUI() }
    }

    func applicationWillTerminate(_ notification: Notification) {
        quitting = true
        try? linkIn?.close()   // el agente termina al cerrarse su stdin
        linkProc?.terminate()
    }

    // MARK: Kast Link

    private func linkStart() {
        guard let exe = Bundle.main.url(forAuxiliaryExecutable: "kast-link") else { linkMissing = true; return }
        let p = Process()
        p.executableURL = exe
        p.arguments = ["agent"]
        let input = Pipe(), output = Pipe()
        p.standardInput = input
        p.standardOutput = output
        p.standardError = FileHandle.nullDevice
        output.fileHandleForReading.readabilityHandler = { [weak self] h in
            let data = h.availableData
            if data.isEmpty { h.readabilityHandler = nil; return }
            DispatchQueue.main.async { self?.linkData(data) }
        }
        p.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async {
                guard let self = self, !self.quitting else { return }
                self.linkIn = nil
                self.linkProc = nil
                if self.linkRestarts < 5 {
                    self.linkRestarts += 1
                    DispatchQueue.main.asyncAfter(deadline: .now() + 3) { self.linkStart() }
                }
            }
        }
        do { try p.run() } catch { linkMissing = true; return }
        linkProc = p
        linkIn = input.fileHandleForWriting
        linkMissing = false
    }

    private func linkData(_ data: Data) {
        linkBuf.append(data)
        while let nl = linkBuf.firstIndex(of: 0x0A) {
            let line = String(data: linkBuf[linkBuf.startIndex..<nl], encoding: .utf8) ?? ""
            linkBuf.removeSubrange(linkBuf.startIndex...nl)
            if line.isEmpty { continue }
            // Alguien pide ver nuestras pantallas: se muestra la ventana para aceptar o rechazar.
            if line.contains("\"ev\":\"request\"") || line.contains("\"ev\":\"show\"") {
                openUI()
                NSApp.requestUserAttention(.criticalRequest)
            }
            web?.evaluateJavaScript("window.__kastReceive(\(line))", completionHandler: nil)
        }
    }

    private func linkSend(_ json: String) {
        guard let h = linkIn, let data = (json + "\n").data(using: .utf8) else {
            send(linkMissing ? ["t": "link", "ev": "missing"]
                             : ["t": "link", "ev": "toast", "msg": "Kast Link se está reiniciando", "err": true])
            return
        }
        h.write(data)
    }

    @objc private func noteUI() { openUI() }
    @objc private func noteReload() { reload() }
    @objc private func noteStop() { NSApp.terminate(nil) }

    private func buildMainMenu() {
        // Sin barra de menús visible (app de accesorio), pero da los atajos ⌘C/⌘V/⌘W/⌘Q a la ventana.
        let main = NSMenu()
        let appItem = NSMenuItem()
        main.addItem(appItem)
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Cerrar ventana", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        appMenu.addItem(withTitle: "Salir de Kast", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        let editItem = NSMenuItem()
        main.addItem(editItem)
        let edit = NSMenu(title: "Edición")
        edit.addItem(withTitle: "Deshacer", action: Selector(("undo:")), keyEquivalent: "z")
        edit.addItem(withTitle: "Rehacer", action: Selector(("redo:")), keyEquivalent: "Z")
        edit.addItem(withTitle: "Cortar", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copiar", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Pegar", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Seleccionar todo", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = edit
        NSApp.mainMenu = main
    }

    // MARK: configuración y atajos

    func reload() {
        let warnings = Config.load()
        log = warnings + registerHotkeys()
        try? log.write(to: Paths.log, atomically: true, encoding: .utf8)
    }

    private func installHotkeyHandler() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, _ -> OSStatus in
            var hk = EventHotKeyID()
            GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                              nil, MemoryLayout<EventHotKeyID>.size, nil, &hk)
            AppDelegate.shared?.hotkeyPressed(Int(hk.id))
            return noErr
        }, 1, &spec, nil, nil)
    }

    /// Registra los atajos de kast.ini (ninguno mientras la interfaz graba una combinación).
    private func registerHotkeys() -> String {
        for r in hotkeyRefs { UnregisterEventHotKey(r) }
        hotkeyRefs.removeAll()
        Config.lock.lock()
        let list = Config.hotkeys
        let nfav = Config.favs.count
        Config.lock.unlock()
        if paused { return "" }
        var out = ""
        for (i, h) in list.enumerated() {
            var ref: EventHotKeyRef?
            let st = RegisterEventHotKey(h.keyCode, h.mods, EventHotKeyID(signature: OSType(0x4B41_5354), id: UInt32(i + 1)),
                                         GetApplicationEventTarget(), 0, &ref)
            if st == noErr, let r = ref { hotkeyRefs.append(r) } else { out += "'\(h.combo)' ya está en uso por otro programa\n" }
        }
        return out + "\(hotkeyRefs.count) atajos activos, \(nfav) favoritos\n"
    }

    fileprivate func hotkeyPressed(_ id: Int) {
        Config.lock.lock()
        let act = id >= 1 && id <= Config.hotkeys.count ? Config.hotkeys[id - 1].action : nil
        Config.lock.unlock()
        guard let a = act else { return }
        run(a)
    }

    /// Ejecuta en el hilo de trabajo y avisa a la interfaz para que se actualice.
    private func run(_ line: String) {
        worker.async { [weak self] in
            Commands.exec(line)
            DispatchQueue.main.async { self?.send(["t": "ext"]) }
        }
    }

    private func displaysChanged() {
        worker.async { [weak self] in
            Displays.invalidate()
            Commands.clearCache()
            DispatchQueue.main.async {
                guard let self = self else { return }
                if self.web != nil { self.requestState() }
                self.refreshMenuValues()
            }
        }
    }

    // MARK: menú de la barra

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        sliders.removeAll()
        let open = menu.addItem(withTitle: "Abrir Kast…", action: #selector(openUIAction), keyEquivalent: ",")
        open.target = self
        menu.addItem(.separator())
        for m in menuMons { menu.addItem(sliderItem(m.index, m.name, m.value)) }
        if !menuMons.isEmpty { menu.addItem(.separator()) }
        Config.lock.lock()
        let favs = Config.favs
        Config.lock.unlock()
        for f in favs.prefix(30) {
            let it = menu.addItem(withTitle: f.name, action: #selector(favAction(_:)), keyEquivalent: "")
            it.target = self
            it.representedObject = f.name
        }
        if !favs.isEmpty { menu.addItem(.separator()) }
        for (title, cmd) in [("Extender pantallas", "topo extend"), ("Duplicar pantallas", "topo clone"), ("Apagar pantallas", "off")] {
            let it = menu.addItem(withTitle: title, action: #selector(cmdAction(_:)), keyEquivalent: "")
            it.target = self
            it.representedObject = cmd
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: "Salir de Kast", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        refreshMenuValues()
    }

    private func sliderItem(_ index: Int, _ name: String, _ value: Int) -> NSMenuItem {
        let v = NSView(frame: NSRect(x: 0, y: 0, width: 270, height: 46))
        let label = NSTextField(labelWithString: "\(index)  \(name)")
        label.font = .menuFont(ofSize: 12)
        label.textColor = .secondaryLabelColor
        label.lineBreakMode = .byTruncatingTail
        label.frame = NSRect(x: 20, y: 26, width: 230, height: 16)
        let s = NSSlider(value: Double(value), minValue: 0, maxValue: 100, target: self, action: #selector(sliderMoved(_:)))
        s.frame = NSRect(x: 18, y: 4, width: 234, height: 22)
        s.tag = index
        s.isContinuous = true
        v.addSubview(label)
        v.addSubview(s)
        sliders[index] = s
        let it = NSMenuItem()
        it.view = v
        return it
    }

    @objc private func sliderMoved(_ s: NSSlider) {
        let i = s.tag, v = Int(s.doubleValue.rounded())
        if let k = menuMons.firstIndex(where: { $0.index == i }) { menuMons[k].value = v }
        pendingLock.lock()
        let queued = pending[i] != nil
        pending[i] = v
        pendingLock.unlock()
        if queued { return }   // ya hay una orden en cola: tomará el último valor
        worker.async { [weak self] in
            guard let self = self else { return }
            self.pendingLock.lock()
            let val = self.pending.removeValue(forKey: i)
            self.pendingLock.unlock()
            if let val = val { Commands.exec("b \(i) \(val)") }
        }
    }

    private func refreshMenuValues() {
        worker.async { [weak self] in
            let vals = Commands.quickBrightness()
            DispatchQueue.main.async {
                guard let self = self else { return }
                self.menuMons = vals
                for m in vals { self.sliders[m.index]?.doubleValue = Double(m.value) }
            }
        }
    }

    @objc private func openUIAction() { openUI() }
    @objc private func favAction(_ sender: NSMenuItem) { if let n = sender.representedObject as? String { run("fav \(n)") } }
    @objc private func cmdAction(_ sender: NSMenuItem) { if let c = sender.representedObject as? String { run(c) } }

    // MARK: ventana de configuración

    func openUI() {
        if let w = window {
            NSApp.activate(ignoringOtherApps: true)
            w.makeKeyAndOrderFront(nil)
            return
        }
        let uc = WKUserContentController()
        uc.add(self, name: "kast")
        // Puente con la API de WebView2 que usa ui.html (chrome.webview.postMessage / evento "message").
        let shim = """
        window.KAST = { platform: 'mac', mica: 1, version: '\(kastVersion)' };
        (function () {
          const listeners = [];
          window.chrome = window.chrome || {};
          window.chrome.webview = {
            postMessage: function (m) { window.webkit.messageHandlers.kast.postMessage(m); },
            addEventListener: function (t, f) { if (t === 'message') listeners.push(f); }
          };
          window.__kastReceive = function (d) { listeners.forEach(function (f) { f({ data: d }); }); };
        })();
        """
        uc.addUserScript(WKUserScript(source: shim, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        let cfg = WKWebViewConfiguration()
        cfg.userContentController = uc
        let web = WKWebView(frame: .zero, configuration: cfg)
        web.setValue(false, forKey: "drawsBackground")

        let win = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 740),
                           styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        win.title = "Kast"
        win.minSize = NSSize(width: 760, height: 520)
        win.isReleasedWhenClosed = false
        win.delegate = self
        let fx = NSVisualEffectView()
        fx.material = .underWindowBackground
        fx.blendingMode = .behindWindow
        fx.state = .followsWindowActiveState
        win.contentView = fx
        web.frame = fx.bounds
        web.autoresizingMask = [.width, .height]
        fx.addSubview(web)
        if let url = Bundle.main.url(forResource: "ui", withExtension: "html") {
            web.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
        }
        win.center()
        window = win
        self.web = web
        NSApp.activate(ignoringOtherApps: true)
        win.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        web?.configuration.userContentController.removeScriptMessageHandler(forName: "kast")
        web?.removeFromSuperview()
        web = nil
        window = nil
        if paused {
            paused = false
            log = registerHotkeys()
        }
    }

    private func send(_ obj: [String: Any]) {
        web?.evaluateJavaScript("window.__kastReceive(\(toJSON(obj)))", completionHandler: nil)
    }

    private func requestState() {
        let l = log
        worker.async { [weak self] in
            let st = Commands.state(log: l)
            DispatchQueue.main.async { self?.send(st) }
        }
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let s = message.body as? String else { return }
        let verb: String, arg: String
        if let nl = s.firstIndex(of: "\n") {
            verb = String(s[..<nl])
            arg = String(s[s.index(after: nl)...])
        } else {
            verb = s
            arg = ""
        }
        switch verb {
        case "state":
            requestState()
        case "exec":
            worker.async { [weak self] in
                Output.capture = ""
                let rc = Commands.exec(arg)
                let out = Output.capture ?? ""
                Output.capture = nil
                DispatchQueue.main.async { self?.send(["t": "done", "rc": Int(rc), "out": out]) }
            }
        case "save":
            try? arg.write(to: Paths.ini, atomically: true, encoding: .utf8)
            reload()
            send(["t": "saved", "autostart": Autostart.enabled, "log": log])
        case "capture":
            paused = arg == "1"
            _ = registerHotkeys()
        case "autostart":
            _ = Autostart.set(arg == "1")
            send(["t": "autostart", "autostart": Autostart.enabled])
        case "link":
            linkSend(arg)
        case "folder":
            NSWorkspace.shared.open(Paths.support)
        default:
            break
        }
    }
}
