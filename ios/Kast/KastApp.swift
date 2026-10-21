import SwiftUI

@main
struct KastApp: App {
    @StateObject private var client = LinkClient.shared
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            DevicesView()
                .environmentObject(client)
                .onAppear { client.start() }
                .onChange(of: phase) { _, p in
                    // iOS corta las conexiones en segundo plano: se cierra la sesión limpiamente.
                    if p == .background, client.session != nil {
                        client.disconnect()
                        client.toast = "Sesión cerrada al salir de Kast"
                    }
                }
        }
    }
}

/// Sesión a pantalla completa (UIKit).
struct SessionScreen: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> SessionViewController { SessionViewController() }
    func updateUIViewController(_ vc: SessionViewController, context: Context) {}
}
