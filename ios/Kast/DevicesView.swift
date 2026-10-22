import SwiftUI

/// Equipos con Kast en la red: tocar uno para verlo y controlarlo.
struct DevicesView: View {
    @EnvironmentObject var client: LinkClient
    @State private var picked: Device?
    @State private var adding = false
    @State private var newAddr = ""
    @State private var byId = false

    private var online: [Device] { client.devices.filter(\.online) }
    private var offline: [Device] { client.devices.filter { !$0.online } }

    var body: some View {
        NavigationStack {
            List {
                if client.devices.isEmpty {
                    ContentUnavailableView {
                        Label("Buscando equipos…", systemImage: "wifi")
                    } description: {
                        Text("Abre Kast en tu PC o tu Mac, en la misma red Wi-Fi. Si no aparece, agrégalo con su IP.")
                    } actions: {
                        Button("Agregar por IP") { adding = true }
                    }
                }
                if !online.isEmpty {
                    Section("En la red") { ForEach(online) { row($0) } }
                }
                if !offline.isEmpty {
                    Section("Recordados") { ForEach(offline) { row($0) } }
                }
            }
            .navigationTitle("Kast")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Menu {
                        Button { byId = true } label: { Label("Conectar por ID", systemImage: "number") }
                        Button { adding = true } label: { Label("Agregar por IP", systemImage: "network") }
                    } label: { Label("Agregar", systemImage: "plus") }
                }
            }
            .refreshable { client.refresh() }
            .alert("Agregar dispositivo", isPresented: $adding) {
                TextField("192.168.1.20", text: $newAddr)
                    .keyboardType(.numbersAndPunctuation)
                    .textInputAutocapitalization(.never)
                Button("Agregar") { client.add(newAddr); newAddr = "" }
                Button("Cancelar", role: .cancel) {}
            } message: {
                Text("IP del equipo con Kast (en la misma red).")
            }
            .sheet(item: $picked) { d in
                ConnectSheet(device: d) { monitor, control, mode, secret in
                    picked = nil
                    client.connect(d, monitor: monitor, control: control, mode: mode, secret: secret)
                }
                .presentationDetents([.medium, .large])
            }
            // El equipo pidió contraseña o código (o no eran correctos): se piden aquí.
            .sheet(item: $client.authRequest) { r in
                ConnectSheet(device: r.device, mode: .password, monitor: r.monitor,
                             message: r.failed ? "No era correcta: inténtalo de nuevo." : "\(r.device.name) pide la contraseña de acceso desatendido o un código de emparejamiento.") { monitor, control, mode, secret in
                    client.authRequest = nil
                    client.connect(r.device, monitor: monitor, control: control, mode: mode, secret: secret)
                }
                .presentationDetents([.medium, .large])
            }
            .sheet(isPresented: $byId) {
                ByIdSheet { id, mode, secret in
                    byId = false
                    let d = Device(id: id, name: "ID \(id)", platform: "", addr: "", online: true, sharing: true, known: false, displays: [], shortId: id)
                    client.connect(d, monitor: 1, control: true, mode: mode, secret: secret)
                }
                .presentationDetents([.medium, .large])
            }
            .fullScreenCover(isPresented: Binding(get: { client.session != nil && client.session?.state != "ended" },
                                                  set: { if !$0 { client.disconnect() } })) {
                SessionScreen().ignoresSafeArea()
            }
            .overlay(alignment: .bottom) { Toast(text: $client.toast) }
        }
    }

    private func row(_ d: Device) -> some View {
        Button { if d.online { picked = d } else { client.refresh() } } label: {
            HStack(spacing: 14) {
                Image(systemName: d.symbol)
                    .font(.title2)
                    .frame(width: 44, height: 44)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
                VStack(alignment: .leading, spacing: 2) {
                    Text(d.name).font(.headline)
                    Text([d.platformName, d.shortId.isEmpty ? "" : "ID \(d.shortId)", d.addr].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.subheadline).foregroundStyle(.secondary)
                    if d.online && !d.sharing {
                        Text("No permite que otros vean sus pantallas").font(.caption).foregroundStyle(.orange)
                    }
                }
                Spacer()
                Circle().fill(d.online ? .green : .gray.opacity(0.4)).frame(width: 9, height: 9)
            }
        }
        .tint(.primary)
        .disabled(d.online && !d.sharing)
        .swipeActions {
            Button("Olvidar", role: .destructive) { client.forget(d.id) }
        }
    }
}

/// Elegir pantalla, si se controla y cómo se entra (permiso, contraseña o código).
struct ConnectSheet: View {
    let device: Device
    var message = ""
    let connect: (Int, Bool, AccessMode, String) -> Void
    @State private var monitor = 1
    @State private var mode = AccessMode.approval
    @State private var secret = ""
    @AppStorage("control") private var control = true

    init(device: Device, mode: AccessMode = .approval, monitor: Int = 1, message: String = "",
         connect: @escaping (Int, Bool, AccessMode, String) -> Void) {
        self.device = device
        self.message = message
        self.connect = connect
        _mode = State(initialValue: mode)
        _monitor = State(initialValue: monitor)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Pantalla", selection: $monitor) {
                        if device.displays.isEmpty { Text("Pantalla 1").tag(1) }
                        ForEach(Array(device.displays.enumerated()), id: \.offset) { i, d in
                            Text("Pantalla \(i + 1) · \(d.first ?? 0) × \(d.last ?? 0)").tag(i + 1)
                        }
                    }
                    Toggle("Controlar teclado y ratón", isOn: $control)
                } footer: {
                    if !message.isEmpty { Text(message).foregroundStyle(.orange) }
                }
                AccessSection(mode: $mode, secret: $secret, name: device.name)
                Section {
                    Button { connect(monitor, control, mode, secret) } label: {
                        Label("Conectar", systemImage: "play.fill").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(mode != .approval && secret.trimmingCharacters(in: .whitespaces).isEmpty)
                    .listRowInsets(EdgeInsets())
                }
            }
            .navigationTitle(device.name)
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

/// Cómo se entra: permiso, contraseña de acceso desatendido o código de emparejamiento.
struct AccessSection: View {
    @Binding var mode: AccessMode
    @Binding var secret: String
    let name: String

    var body: some View {
        Section {
            Picker("Acceso", selection: $mode) {
                ForEach(AccessMode.allCases) { Text($0.label).tag($0) }
            }
            switch mode {
            case .approval: EmptyView()
            case .password: SecureField("Contraseña", text: $secret).textContentType(.password)
            case .pair:
                TextField("Código (p. ej. K7QM-4ZP2)", text: $secret)
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
            }
        } footer: {
            switch mode {
            case .approval: Text("\(name) tendrá que aceptar la conexión y verás un código para comprobarla.")
            case .password: Text("Entra sin que nadie acepte. La contraseña no se envía: Kast sólo demuestra que la conoces.")
            case .pair: Text("Código de un solo uso (en \(name): Kast → Dispositivos → Emparejar). Después entrarás sin preguntar.")
            }
        }
    }
}

/// Conectar escribiendo el ID de 9 dígitos del otro equipo.
struct ByIdSheet: View {
    let connect: (String, AccessMode, String) -> Void
    @State private var id = ""
    @State private var mode = AccessMode.approval
    @State private var secret = ""

    private var digits: String { id.filter(\.isNumber) }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("482 913 057", text: $id).keyboardType(.numberPad).font(.title3.monospacedDigit())
                } header: { Text("ID del otro equipo") } footer: {
                    Text("En el otro equipo: Kast → Dispositivos → Tu ID. De momento, tiene que estar en tu misma red.")
                }
                AccessSection(mode: $mode, secret: $secret, name: "el otro equipo")
                Section {
                    Button { connect(digits, mode, secret) } label: {
                        Label("Conectar", systemImage: "play.fill").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(digits.count != 9 || (mode != .approval && secret.trimmingCharacters(in: .whitespaces).isEmpty))
                    .listRowInsets(EdgeInsets())
                }
            }
            .navigationTitle("Conectar por ID")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

/// Aviso breve en la parte inferior.
struct Toast: View {
    @Binding var text: String?

    var body: some View {
        if let t = text {
            Text(t)
                .font(.subheadline)
                .padding(.horizontal, 16).padding(.vertical, 10)
                .background(.regularMaterial, in: Capsule())
                .padding(.bottom, 24)
                .transition(.move(edge: .bottom).combined(with: .opacity))
                .task(id: t) {
                    try? await Task.sleep(for: .seconds(3))
                    withAnimation { text = nil }
                }
        }
    }
}
