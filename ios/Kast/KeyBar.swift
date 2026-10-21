import UIKit

/// Barra sobre el teclado de iOS: teclas que no tiene (Esc, Tab, flechas, F1-F12…), modificadores que se fijan
/// con un toque (se sueltan tras la siguiente tecla) y atajos del sistema remoto.
final class KeyBar: UIInputView {
    /// Tecla normal (con los modificadores fijados, ver takeModifiers).
    var onKey: ((String) -> Void)?
    /// Sistema remoto: cambia las etiquetas (Win/⌘) y los atajos.
    var host = "" { didSet { if host != oldValue { rebuild() } } }

    private var latched: [Keys.Mod] = []
    private var modButtons: [Keys.Mod: UIButton] = [:]
    private let row = UIStackView(), fnRow = UIStackView()
    private var showFn = false

    init() {
        super.init(frame: CGRect(x: 0, y: 0, width: 0, height: 48), inputViewStyle: .keyboard)
        allowsSelfSizing = true
        let column = UIStackView(arrangedSubviews: [scroller(fnRow), scroller(row)])
        column.axis = .vertical
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: leadingAnchor), column.trailingAnchor.constraint(equalTo: trailingAnchor),
            column.topAnchor.constraint(equalTo: topAnchor, constant: 4), column.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
        ])
        rebuild()
    }
    required init?(coder: NSCoder) { fatalError() }

    var hasModifiers: Bool { !latched.isEmpty }

    /// Códigos de los modificadores fijados (y se sueltan).
    func takeModifiers() -> [String] {
        let codes = latched.map { $0.code(host: host) }
        latched.removeAll()
        refreshMods()
        return codes
    }

    private func scroller(_ stack: UIStackView) -> UIScrollView {
        stack.axis = .horizontal
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        let sv = UIScrollView()
        sv.showsHorizontalScrollIndicator = false
        sv.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: sv.contentLayoutGuide.leadingAnchor, constant: 8),
            stack.trailingAnchor.constraint(equalTo: sv.contentLayoutGuide.trailingAnchor, constant: -8),
            stack.topAnchor.constraint(equalTo: sv.contentLayoutGuide.topAnchor),
            stack.bottomAnchor.constraint(equalTo: sv.contentLayoutGuide.bottomAnchor),
            stack.heightAnchor.constraint(equalTo: sv.frameLayoutGuide.heightAnchor),
            sv.heightAnchor.constraint(equalToConstant: 40),
        ])
        return sv
    }

    private func key(_ title: String, symbol: String? = nil, _ action: @escaping () -> Void) -> UIButton {
        var c = UIButton.Configuration.gray()
        c.title = symbol == nil ? title : nil
        c.image = symbol.map { UIImage(systemName: $0) } ?? nil
        c.cornerStyle = .medium
        c.contentInsets = .init(top: 6, leading: 12, bottom: 6, trailing: 12)
        let b = UIButton(configuration: c, primaryAction: UIAction { _ in
            UIDevice.current.playInputClick()
            action()
        })
        b.accessibilityLabel = title
        return b
    }

    private func rebuild() {
        row.arrangedSubviews.forEach { $0.removeFromSuperview() }
        fnRow.arrangedSubviews.forEach { $0.removeFromSuperview() }
        modButtons.removeAll()
        let send: (String) -> () -> Void = { [weak self] code in { self?.onKey?(code) } }
        row.addArrangedSubview(key("Esc", send("Escape")))
        row.addArrangedSubview(key("Tab", send("Tab")))
        for m in [Keys.Mod.ctrl, .alt, .meta, .shift] {
            let b = key(m.label(host: host)) { [weak self] in self?.toggle(m) }
            modButtons[m] = b
            row.addArrangedSubview(b)
        }
        for (name, sym, code) in [("Izquierda", "arrow.left", "ArrowLeft"), ("Arriba", "arrow.up", "ArrowUp"),
                                  ("Abajo", "arrow.down", "ArrowDown"), ("Derecha", "arrow.right", "ArrowRight")] {
            row.addArrangedSubview(key(name, symbol: sym, send(code)))
        }
        row.addArrangedSubview(key("Supr", send("Delete")))
        row.addArrangedSubview(key("Inicio", send("Home")))
        row.addArrangedSubview(key("Fin", send("End")))
        row.addArrangedSubview(key("RePág", send("PageUp")))
        row.addArrangedSubview(key("AvPág", send("PageDown")))
        row.addArrangedSubview(key("Fn") { [weak self] in self?.toggleFn() })
        // Atajos del sistema remoto (con sus modificadores, sin tocar los fijados).
        let menu = UIMenu(children: Keys.shortcuts(host: host).map { name, mods, code in
            UIAction(title: name) { [weak self] _ in
                guard let self else { return }
                let saved = self.latched
                self.latched = mods
                self.onKey?(code)
                self.latched = saved
                self.refreshMods()
            }
        })
        var c = UIButton.Configuration.gray()
        c.image = UIImage(systemName: "command")
        c.title = "Atajos"
        c.imagePadding = 4
        c.cornerStyle = .medium
        let shortcuts = UIButton(configuration: c)
        shortcuts.menu = menu
        shortcuts.showsMenuAsPrimaryAction = true
        row.addArrangedSubview(shortcuts)
        for n in 1...12 { fnRow.addArrangedSubview(key("F\(n)", send("F\(n)"))) }
        fnRow.superview?.isHidden = !showFn
        refreshMods()
    }

    private func toggle(_ m: Keys.Mod) {
        if let i = latched.firstIndex(of: m) { latched.remove(at: i) } else { latched.append(m) }
        refreshMods()
    }

    private func refreshMods() {
        for (m, b) in modButtons {
            var c = b.configuration
            c?.baseBackgroundColor = latched.contains(m) ? .tintColor : nil
            c?.baseForegroundColor = latched.contains(m) ? .white : nil
            b.configuration = c
        }
    }

    private func toggleFn() {
        showFn.toggle()
        fnRow.superview?.isHidden = !showFn
        invalidateIntrinsicContentSize()
    }
}
