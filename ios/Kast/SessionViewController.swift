import AVFoundation
import Combine
import UIKit

/// Pantalla de una sesión: vídeo del otro equipo y control con gestos, el teclado de iOS o uno físico.
final class SessionViewController: UIViewController, UIGestureRecognizerDelegate {
    private let client = LinkClient.shared
    private let video = VideoView()
    private let cursor = UIImageView(image: UIImage(systemName: "cursorarrow", withConfiguration: UIImage.SymbolConfiguration(pointSize: 22, weight: .regular)))
    private let keys = KeyInputView()
    private let waiting = UIStackView()
    private let waitTitle = UILabel(), waitCode = UILabel()
    private let bar = UIStackView()
    private let barBg = UIVisualEffectView(effect: UIBlurEffect(style: .systemUltraThinMaterialDark))
    private var modeButton: UIButton!
    private let toast = UILabel()
    private var cancellables = Set<AnyCancellable>()

    /// Tamaño de la imagen remota (píxeles).
    private var rw = 0, rh = 0
    /// Zoom (1 = toda la pantalla remota visible) y desplazamiento del centro, en puntos.
    private var zoom: CGFloat = 1, offset = CGPoint.zero
    /// Cursor en píxeles remotos.
    private var cx: CGFloat = 0, cy: CGFloat = 0
    private var trackpad = true
    private var host: String { client.session?.platform ?? "" }

    // MARK: - Vista

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        view.clipsToBounds = true
        view.addSubview(video)
        cursor.tintColor = .white
        cursor.layer.shadowColor = UIColor.black.cgColor
        cursor.layer.shadowOpacity = 0.9
        cursor.layer.shadowRadius = 1.5
        cursor.layer.shadowOffset = .zero
        cursor.isHidden = true
        view.addSubview(cursor)
        keys.onKey = { [weak self] code, down in self?.client.key(code, down) }
        keys.onText = { [weak self] t in self?.type(t) }
        keys.keyBar.onKey = { [weak self] code in self?.press(code) }
        keys.keyBar.host = host
        view.addSubview(keys)
        setupWaiting()
        setupBar()
        setupGestures()
        client.sink = video
        client.$session.receive(on: RunLoop.main).sink { [weak self] s in self?.update(s) }.store(in: &cancellables)
        setupToast()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // Para el teclado físico (iPad) sin abrir el de pantalla.
        becomeFirstResponder()
    }

    override var canBecomeFirstResponder: Bool { true }
    override var prefersStatusBarHidden: Bool { true }
    override var prefersHomeIndicatorAutoHidden: Bool { true }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        layoutVideo()
    }

    private func update(_ s: Session?) {
        guard let s else { return }
        waitTitle.text = s.state == "waiting" ? "Acepta en \(s.peer)" : "Conectando con \(s.peer)…"
        waitCode.text = s.state == "waiting" ? s.code : ""
        waiting.isHidden = s.state == "live"
        bar.isHidden = s.state != "live"
        barBg.isHidden = bar.isHidden
        keys.keyBar.host = s.platform
        if s.state == "live", (s.width != rw || s.height != rh) {
            rw = s.width; rh = s.height
            cx = CGFloat(rw) / 2; cy = CGFloat(rh) / 2
            layoutVideo()
        }
        cursor.isHidden = !(s.state == "live" && s.control)
        modeButton.isHidden = !s.control
    }

    /// Avisos (portapapeles, notas de la sesión) sobre el vídeo.
    private func setupToast() {
        toast.font = .preferredFont(forTextStyle: .subheadline)
        toast.textColor = .white
        toast.backgroundColor = UIColor(white: 0.1, alpha: 0.85)
        toast.textAlignment = .center
        toast.numberOfLines = 2
        toast.layer.cornerRadius = 14
        toast.clipsToBounds = true
        toast.alpha = 0
        toast.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(toast)
        NSLayoutConstraint.activate([
            toast.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            toast.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -16),
            toast.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -40),
            toast.heightAnchor.constraint(greaterThanOrEqualToConstant: 34),
        ])
        client.$toast.receive(on: RunLoop.main).sink { [weak self] t in
            guard let self, let t else { return }
            self.toast.text = "   \(t)   "
            UIView.animate(withDuration: 0.2) { self.toast.alpha = 1 }
            UIView.animate(withDuration: 0.4, delay: 2.6) { self.toast.alpha = 0 }
        }.store(in: &cancellables)
    }

    private func setupWaiting() {
        waiting.axis = .vertical
        waiting.alignment = .center
        waiting.spacing = 14
        waitTitle.textColor = .white
        waitTitle.font = .preferredFont(forTextStyle: .title3)
        waitCode.textColor = .white
        waitCode.font = .monospacedDigitSystemFont(ofSize: 40, weight: .semibold)
        let hint = UILabel()
        hint.text = "Comprueba que el otro equipo muestra este código"
        hint.textColor = .lightGray
        hint.font = .preferredFont(forTextStyle: .footnote)
        let spinner = UIActivityIndicatorView(style: .medium)
        spinner.color = .white
        spinner.startAnimating()
        let cancel = UIButton(configuration: .gray(), primaryAction: UIAction(title: "Cancelar") { [weak self] _ in self?.client.disconnect() })
        [spinner, waitTitle, waitCode, hint, cancel].forEach(waiting.addArrangedSubview)
        waiting.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(waiting)
        NSLayoutConstraint.activate([waiting.centerXAnchor.constraint(equalTo: view.centerXAnchor), waiting.centerYAnchor.constraint(equalTo: view.centerYAnchor)])
    }

    private func barButton(_ symbol: String, _ label: String, _ action: @escaping () -> Void) -> UIButton {
        var c = UIButton.Configuration.plain()
        c.image = UIImage(systemName: symbol)
        c.baseForegroundColor = .white
        c.contentInsets = .init(top: 8, leading: 10, bottom: 8, trailing: 10)
        let b = UIButton(configuration: c, primaryAction: UIAction { _ in action() })
        b.accessibilityLabel = label
        return b
    }

    private func setupBar() {
        let blur = barBg
        blur.isHidden = true
        blur.layer.cornerRadius = 20
        blur.clipsToBounds = true
        modeButton = barButton("hand.point.up.left", "Modo trackpad o táctil") { [weak self] in self?.toggleMode() }
        let items = [
            barButton("xmark", "Desconectar") { [weak self] in self?.client.disconnect() },
            modeButton!,
            barButton("keyboard", "Teclado") { [weak self] in self?.toggleKeyboard() },
            barButton("doc.on.clipboard", "Enviar portapapeles") { [weak self] in self?.client.sendClipboard() },
            barButton("arrow.up.left.and.arrow.down.right", "Ver toda la pantalla") { [weak self] in self?.resetZoom() },
        ]
        items.forEach(bar.addArrangedSubview)
        bar.axis = .horizontal
        bar.isHidden = true
        bar.translatesAutoresizingMaskIntoConstraints = false
        blur.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(blur)
        view.addSubview(bar)
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 6),
            bar.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            blur.leadingAnchor.constraint(equalTo: bar.leadingAnchor, constant: -4), blur.trailingAnchor.constraint(equalTo: bar.trailingAnchor, constant: 4),
            blur.topAnchor.constraint(equalTo: bar.topAnchor), blur.bottomAnchor.constraint(equalTo: bar.bottomAnchor),
        ])
        // Con el teclado abierto, la barra sigue visible por encima del vídeo.
        view.bringSubviewToFront(blur)
        view.bringSubviewToFront(bar)
    }

    private func toggleMode() {
        trackpad.toggle()
        modeButton.configuration?.image = UIImage(systemName: trackpad ? "hand.point.up.left" : "hand.tap")
        client.toast = trackpad ? "Modo trackpad: desliza para mover el cursor, toca para hacer clic" : "Modo táctil: toca donde quieras hacer clic"
    }

    private func toggleKeyboard() {
        if keys.isFirstResponder { keys.resignFirstResponder(); becomeFirstResponder() } else { keys.becomeFirstResponder() }
    }

    // MARK: - Geometría

    /// Rectángulo (en la vista) donde se dibuja la imagen remota con el zoom actual.
    private var videoRect: CGRect {
        guard rw > 0, rh > 0 else { return view.bounds }
        let fit = AVMakeRect(aspectRatio: CGSize(width: rw, height: rh), insideRect: view.bounds)
        let w = fit.width * zoom, h = fit.height * zoom
        return CGRect(x: fit.midX - w / 2 + offset.x, y: fit.midY - h / 2 + offset.y, width: w, height: h)
    }

    private func clampOffset() {
        guard rw > 0 else { return }
        let fit = AVMakeRect(aspectRatio: CGSize(width: rw, height: rh), insideRect: view.bounds)
        let mx = max(0, (fit.width * zoom - view.bounds.width) / 2), my = max(0, (fit.height * zoom - view.bounds.height) / 2)
        offset.x = min(mx, max(-mx, offset.x))
        offset.y = min(my, max(-my, offset.y))
    }

    private func layoutVideo() {
        clampOffset()
        video.frame = videoRect
        placeCursor()
    }

    private func resetZoom() {
        zoom = 1; offset = .zero
        UIView.animate(withDuration: 0.2) { self.layoutVideo() }
    }

    private func toRemote(_ p: CGPoint) -> CGPoint {
        let r = videoRect
        return CGPoint(x: min(CGFloat(rw - 1), max(0, (p.x - r.minX) / r.width * CGFloat(rw))),
                       y: min(CGFloat(rh - 1), max(0, (p.y - r.minY) / r.height * CGFloat(rh))))
    }

    private func toView(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
        let r = videoRect
        return CGPoint(x: r.minX + x / CGFloat(max(rw, 1)) * r.width, y: r.minY + y / CGFloat(max(rh, 1)) * r.height)
    }

    private func placeCursor() {
        let p = toView(cx, cy)
        // La punta de la flecha está arriba a la izquierda del símbolo.
        cursor.frame.origin = CGPoint(x: p.x - 4, y: p.y - 3)
    }

    /// Con zoom, la vista sigue al cursor para que no salga de la pantalla.
    private func follow() {
        guard zoom > 1.01 else { return }
        let p = toView(cx, cy), m: CGFloat = 60, b = view.bounds
        if p.x < m { offset.x += m - p.x } else if p.x > b.width - m { offset.x -= p.x - (b.width - m) }
        if p.y < m { offset.y += m - p.y } else if p.y > b.height - m { offset.y -= p.y - (b.height - m) }
        layoutVideo()
    }

    private func moveCursor(to p: CGPoint) {
        cx = p.x; cy = p.y
        client.move(Int(cx), Int(cy))
        placeCursor()
    }

    // MARK: - Gestos

    private let tap1 = UITapGestureRecognizer(), tap2 = UITapGestureRecognizer(), secondary = UITapGestureRecognizer()
    private let pan1 = UIPanGestureRecognizer(), pan2 = UIPanGestureRecognizer(), scroll = UIPanGestureRecognizer()
    private let pinch = UIPinchGestureRecognizer(), hold = UILongPressGestureRecognizer(), hover = UIHoverGestureRecognizer()
    private var lastPan = CGPoint.zero, lastHold = CGPoint.zero, last2 = CGPoint.zero, startZoom: CGFloat = 1
    private enum Two { case none, scroll, zoom }
    private var two = Two.none
    private var wheelRest = CGPoint.zero

    private func setupGestures() {
        tap1.addTarget(self, action: #selector(onTap))
        tap2.numberOfTouchesRequired = 2
        tap2.addTarget(self, action: #selector(onTwoFingerTap))
        secondary.buttonMaskRequired = .secondary
        secondary.addTarget(self, action: #selector(onTwoFingerTap))
        pan1.maximumNumberOfTouches = 1
        pan1.addTarget(self, action: #selector(onPan))
        hold.minimumPressDuration = 0.35
        hold.allowableMovement = 12
        hold.addTarget(self, action: #selector(onHold))
        pan1.require(toFail: hold)
        pan2.minimumNumberOfTouches = 2
        pan2.maximumNumberOfTouches = 2
        pan2.addTarget(self, action: #selector(onTwo))
        pinch.addTarget(self, action: #selector(onTwo))
        // Rueda o desplazamiento con dos dedos en un trackpad o ratón (iPad).
        scroll.allowedScrollTypesMask = .all
        scroll.maximumNumberOfTouches = 0
        scroll.addTarget(self, action: #selector(onScroll))
        hover.addTarget(self, action: #selector(onHover))
        for g in [tap1, tap2, secondary, pan1, pan2, pinch, hold, scroll, hover] as [UIGestureRecognizer] {
            g.delegate = self
            view.addGestureRecognizer(g)
        }
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith o: UIGestureRecognizer) -> Bool {
        (g == pan2 && o == pinch) || (g == pinch && o == pan2) || g == hover || o == hover
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        // Los botones de la barra y del aviso de espera no son gestos sobre la pantalla remota.
        !(touch.view is UIControl) && client.session?.state == "live" && client.session?.control == true
            || (g == pinch || g == pan2) && client.session?.state == "live"
    }

    private func click(_ button: UInt8, at p: CGPoint) {
        client.button(button, true, Int(p.x), Int(p.y))
        client.button(button, false, Int(p.x), Int(p.y))
    }

    @objc private func onTap(_ g: UITapGestureRecognizer) {
        if !trackpad { moveCursor(to: toRemote(g.location(in: view))) }
        click(0, at: CGPoint(x: cx, y: cy))
    }

    @objc private func onTwoFingerTap(_ g: UITapGestureRecognizer) {
        if !trackpad || g == secondary { moveCursor(to: toRemote(g.location(in: view))) }
        click(1, at: CGPoint(x: cx, y: cy))
    }

    /// Un dedo: trackpad mueve el cursor (con aceleración); táctil arrastra con el botón pulsado.
    @objc private func onPan(_ g: UIPanGestureRecognizer) {
        let p = g.location(in: view)
        if trackpad {
            if g.state == .began { lastPan = p; return }
            let d = CGPoint(x: p.x - lastPan.x, y: p.y - lastPan.y)
            lastPan = p
            let v = g.velocity(in: view), speed = hypot(v.x, v.y)
            let accel = 1 + min(speed / 900, 1.8)
            let k = CGFloat(rw) / max(videoRect.width, 1) * zoom * accel
            moveCursor(to: CGPoint(x: min(CGFloat(rw - 1), max(0, cx + d.x * k)), y: min(CGFloat(rh - 1), max(0, cy + d.y * k))))
            follow()
        } else {
            let r = toRemote(p)
            switch g.state {
            case .began: moveCursor(to: r); client.button(0, true, Int(r.x), Int(r.y))
            case .changed: moveCursor(to: r)
            default: client.button(0, false, Int(r.x), Int(r.y))
            }
        }
    }

    /// Mantener pulsado: trackpad arrastra desde el cursor; táctil hace clic derecho.
    @objc private func onHold(_ g: UILongPressGestureRecognizer) {
        let p = g.location(in: view)
        if !trackpad {
            if g.state == .began { moveCursor(to: toRemote(p)); click(1, at: CGPoint(x: cx, y: cy)) }
            return
        }
        switch g.state {
        case .began:
            lastHold = p
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            client.button(0, true, Int(cx), Int(cy))
        case .changed:
            let k = CGFloat(rw) / max(videoRect.width, 1) * zoom
            moveCursor(to: CGPoint(x: min(CGFloat(rw - 1), max(0, cx + (p.x - lastHold.x) * k)), y: min(CGFloat(rh - 1), max(0, cy + (p.y - lastHold.y) * k))))
            lastHold = p
            follow()
        default:
            client.button(0, false, Int(cx), Int(cy))
        }
    }

    /// Dos dedos: pellizcar = zoom (y mover la vista); deslizar = rueda en el otro equipo.
    @objc private func onTwo(_ g: UIGestureRecognizer) {
        let p = pan2.location(in: view)
        switch g.state {
        case .began:
            if two == .none { last2 = p; startZoom = zoom; wheelRest = .zero }
        case .changed:
            if two == .none {
                if abs(pinch.scale - 1) > 0.12 { two = .zoom }
                else if hypot(p.x - last2.x, p.y - last2.y) > 14 { two = .scroll }
                else { return }
            }
            if two == .zoom {
                zoom = min(6, max(1, startZoom * pinch.scale))
                offset.x += p.x - last2.x; offset.y += p.y - last2.y
                last2 = p
                layoutVideo()
            } else if two == .scroll, g == pan2 {
                wheel(dx: -(p.x - last2.x), dy: p.y - last2.y)
                last2 = p
            }
        default:
            if pan2.state != .changed && pinch.state != .changed { two = .none }
        }
    }

    @objc private func onScroll(_ g: UIPanGestureRecognizer) {
        let t = g.translation(in: view)
        g.setTranslation(.zero, in: view)
        wheel(dx: -t.x, dy: t.y)
    }

    /// Puntero de un ratón o trackpad (iPad): mueve el cursor remoto donde está.
    @objc private func onHover(_ g: UIHoverGestureRecognizer) {
        guard client.session?.control == true, g.state == .changed || g.state == .began else { return }
        moveCursor(to: toRemote(g.location(in: view)))
    }

    /// Desplazamiento en puntos -> rueda (120 = una muesca ≈ 30 puntos).
    private func wheel(dx: CGFloat, dy: CGFloat) {
        wheelRest.x += dx * 4; wheelRest.y += dy * 4
        let ix = Int(wheelRest.x), iy = Int(wheelRest.y)
        if ix != 0 || iy != 0 {
            client.wheel(ix, iy)
            wheelRest.x -= CGFloat(ix); wheelRest.y -= CGFloat(iy)
        }
    }

    // MARK: - Teclado

    /// Tecla de la barra (con los modificadores fijados, que se sueltan después).
    private func press(_ code: String) {
        let mods = keys.keyBar.takeModifiers()
        mods.forEach { client.key($0, true) }
        client.tap(code)
        mods.reversed().forEach { client.key($0, false) }
    }

    /// Texto del teclado de iOS: con un modificador fijado, las letras van como teclas (Ctrl+C…).
    private func type(_ t: String) {
        if t == "\n" { return press("Enter") }
        if keys.keyBar.hasModifiers, t.count == 1, let code = Keys.code(for: t.first!) { return press(code) }
        client.text(t)
    }

    // Teclado físico cuando el de pantalla está cerrado.
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if !keys.handle(presses, down: true) { super.pressesBegan(presses, with: event) }
    }
    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if !keys.handle(presses, down: false) { super.pressesEnded(presses, with: event) }
    }
    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if !keys.handle(presses, down: false) { super.pressesCancelled(presses, with: event) }
    }
}

/// Recibe el teclado de iOS (texto) y el físico (teclas), con la barra de teclas especiales encima.
final class KeyInputView: UIView, UIKeyInput {
    var onText: ((String) -> Void)?
    var onKey: ((String, Bool) -> Void)?
    let keyBar = KeyBar()

    override var canBecomeFirstResponder: Bool { true }
    override var inputAccessoryView: UIView? { keyBar }
    var hasText: Bool { true }
    func insertText(_ text: String) { onText?(text) }
    func deleteBackward() { onKey?("Backspace", true); onKey?("Backspace", false) }

    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var spellCheckingType: UITextSpellCheckingType = .no
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
    var keyboardType: UIKeyboardType = .default

    /// Teclas de un teclado físico: se envían como teclas (no como texto). true si se atendieron.
    func handle(_ presses: Set<UIPress>, down: Bool) -> Bool {
        var handled = false
        for p in presses {
            guard let k = p.key, let code = Keys.code(k.keyCode) else { continue }
            onKey?(code, down)
            handled = true
        }
        return handled
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if !handle(presses, down: true) { super.pressesBegan(presses, with: event) }
    }
    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if !handle(presses, down: false) { super.pressesEnded(presses, with: event) }
    }
    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if !handle(presses, down: false) { super.pressesCancelled(presses, with: event) }
    }
}
