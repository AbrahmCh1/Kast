import SwiftUI

/// Equipos con Kast en la red: tocar uno para verlo y controlarlo.
struct DevicesView: View {
    @EnvironmentObject var client: LinkClient
    @State private var picked: Device?
    @State private var adding = false
    @State private var newAddr = ""

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
                    Button { adding = true } label: { Label("Agregar por IP", systemImage: "plus") }
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
                ConnectSheet(device: d) { monitor, control in
                    picked = nil
                    client.connect(d, monitor: monitor, control: control)
                }
                .presentationDetents([.medium])
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
                    Text([d.platformName, d.addr].filter { !$0.isEmpty }.joined(separator: " · "))
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

/// Elegir pantalla y si se controla.
struct ConnectSheet: View {
    let device: Device
    let connect: (Int, Bool) -> Void
    @State private var monitor = 1
    @AppStorage("control") private var control = true

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
                    Text("\(device.name) tendrá que aceptar la conexión y verás un código para comprobarla.")
                }
                Section {
                    Button { connect(monitor, control) } label: {
                        Label("Conectar", systemImage: "play.fill").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .listRowInsets(EdgeInsets())
                }
            }
            .navigationTitle(device.name)
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
