<p align="center"><img src="assets/kast-256.png" width="96" alt="Kast"></p>

<h1 align="center">Kast</h1>

<p align="center">Control de monitores para <b>Windows</b> y <b>macOS</b>: brillo, contraste, entradas, resolución y disposición,<br>desde la bandeja o la barra de menús, con atajos globales y una interfaz moderna.</p>

<p align="center"><a href="https://github.com/AbrahmCh1/Kast/releases">Descargar</a> · <a href="#windows">Windows</a> · <a href="#macos">macOS</a></p>

<p align="center"><img src="assets/screenshot.png" width="760" alt="Interfaz de Kast"></p>

## Características

- **Brillo, contraste, volumen y entrada** de cada monitor por DDC/CI, en paralelo y respetando los tiempos del protocolo.
- **Calibración por monitor** para que el mismo porcentaje de brillo se vea igual en pantallas distintas.
- **Resolución, frecuencia, rotación y monitor principal**; activar y desactivar monitores.
- **Extender y duplicar** sobre todos los monitores conectados (no sólo los que recuerda Windows).
- **Disposiciones guardadas**: posición, resolución y monitores activos, restaurables con un clic o un atajo.
- **Favoritos**: secuencias de comandos (`b all 20; c all 40`) aplicables desde la bandeja o con un atajo.
- **Atajos globales** configurables, grabados pulsando la combinación.
- **Cambios seguros**: resolución, rotación, duplicar y disposiciones se revierten solos a los 15 s si no se confirman.
- **CLI completa** (`kast.exe`) para scripts.

# Windows

## Ligero de verdad

| | |
|---|---|
| `kast.exe` (CLI) | ~125 KB |
| `kastd.exe` (bandeja + atajos) | ~200 KB; **~250 KB de RAM** en reposo (2-3 MB tras haber abierto la interfaz), 0 % de CPU |
| Ventana de configuración | WebView2 creado **sólo mientras está abierta**; al cerrarla se liberan sus procesos |

Sin runtime, sin frameworks: Win32 + DXVA2 (DDC/CI) + CCD (`SetDisplayConfig`), compilado con `zig cc`.

## Requisitos

- Windows 10 u 11 (x64).
- [Runtime de WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) para la ventana de configuración (incluido en Windows 11).
- Para brillo/contraste/entrada: monitores con **DDC/CI** activado en su menú.

## Compilar

```powershell
winget install zig.zig
.\build.ps1
```

Genera `bin\kast.exe`, `bin\kastd.exe` y copia `WebView2Loader.dll` y un `kast.ini` de ejemplo.

## Uso

Ejecuta `kastd.exe` para tener Kast en la bandeja: clic para abrir la interfaz, clic derecho para favoritos y acciones rápidas. Para iniciarlo con Windows: **Ajustes → Iniciar con Windows** o `kast autostart on`.

Desde la terminal:

```powershell
kast list                      # monitores (1..n de izquierda a derecha, * = principal)
kast b all 40                  # brillo de todos al 40 %
kast b c +10                   # +10 % al monitor que tiene el cursor
kast "b all 20; c all 40"      # secuencias con ;
kast input 2 hdmi2             # cambiar entrada
kast mode 1 1920x1080@144      # resolución y frecuencia
kast topo extend               # extender a todos los monitores conectados
kast layout save escritorio    # guardar la disposición actual
kast fav Noche                 # aplicar un favorito
kast help                      # referencia completa
```

Monitores: `1`, `1,3`, `all`, `p` (principal) o `c` (el que tiene el cursor).

## Configuración

`kast.ini`, junto a los ejecutables (la interfaz lo edita por ti):

```ini
[favoritos]
Noche = b all 20; c all 40

[atajos]
ctrl+alt+up = b c +10
ctrl+alt+1 = fav Noche
ctrl+alt+num9 = layout load Desktop

[calibracion]
# brillo real mínimo y máximo (en % del monitor) para el 0 % y el 100 % de Kast
VY279HGR = 10 85
```

> `Ctrl + Alt` equivale a `AltGr`: evita combinaciones que uses para escribir.

# macOS

<p align="center"><img src="assets/screenshot-macos.png" width="640" alt="Kast en macOS"></p>

App nativa en Swift que vive en la **barra de menús**: control de brillo por monitor directamente en el menú, favoritos, extender/duplicar y la misma ventana de configuración que en Windows (WKWebView, creada sólo al abrirla). Binario universal (Apple Silicon e Intel), macOS 12 o posterior.

## Instalar

1. Descarga `Kast-<versión>-macos.zip` de [Releases](https://github.com/AbrahmCh1/Kast/releases), descomprímelo y mueve **Kast.app** a *Aplicaciones*.
2. La app no está firmada por Apple: la primera vez, **clic derecho → Abrir** (o `xattr -dr com.apple.quarantine /Applications/Kast.app`).
3. Para usar la CLI desde la terminal:

```bash
sudo ln -sf /Applications/Kast.app/Contents/MacOS/Kast /usr/local/bin/kast
```

Los comandos, favoritos, atajos y la calibración son los mismos que en Windows. La configuración vive en `~/Library/Application Support/Kast/kast.ini`. Los atajos usan `ctrl`, `alt` (⌥), `shift` y `cmd` (⌘), y no requieren permisos de accesibilidad.

## Qué funciona en macOS

| Función | Estado |
|---|---|
| Brillo de la pantalla integrada | ✓ |
| Brillo, contraste, volumen y entrada de monitores externos (DDC/CI) | ✓ en Apple Silicon · ✗ en Macs Intel |
| Resolución, frecuencia y monitor principal | ✓ |
| Extender / duplicar, disposiciones guardadas | ✓ |
| Favoritos, atajos globales, inicio con la sesión | ✓ |
| Rotar o desactivar monitores | ✗ (macOS no lo permite con API pública) |

## Compilar

```bash
bash mac/build.sh 1.0.0    # genera mac/build/Kast.app (requiere Xcode o Command Line Tools)
```

# Kast Link (experimental)

Ver y **controlar** otro equipo (como AnyDesk) en uno de tus monitores, por la red local y **cifrado**. Es la primera fase de un sistema para unificar varios equipos (Windows y macOS) y asignar pantallas de uno a otro.

**Se usa desde la app:** Kast → **Dispositivos** («Conectar a dispositivo»).

1. Abre Kast en los dos equipos. Se detectan solos en la red local (o usa **Agregar dispositivo** con la IP del otro).
2. En el que va a ver: elige el dispositivo, **qué pantalla suya** ver y **dónde mostrarla** (una ventana o a pantalla completa en uno de tus monitores) y pulsa **Conectar**. Con **Controlar teclado y ratón** (activado por defecto) puedes escribir, hacer clic, arrastrar y desplazarte en el otro equipo.
3. En el otro equipo, Kast muestra la petición con un **código de 6 dígitos**: comprueba que coincide con el que ves y pulsa **Aceptar**. Puedes permitir el control o dejar que solo vea. Con **Recordar este dispositivo** no volverá a preguntar.

En la ventana del visor, `Esc` la cierra y `F11` alterna pantalla completa; al controlar, todas las teclas van al otro equipo, así que se usan `Ctrl+Alt+Shift+F` (pantalla completa) y `Ctrl+Alt+Shift+Q` (cerrar). Entre Windows y Mac, `Ctrl` y `Cmd` se intercambian para que los atajos (copiar, pegar…) funcionen como esperas. Desde **Dispositivos** se ven las sesiones activas y se pueden cortar, y el interruptor **Permitir que otros vean mis pantallas** desactiva las peticiones entrantes. La primera vez, el sistema pide permisos: en Windows, el del firewall (puertos 47800 TCP y 47801 UDP, redes privadas); en Mac, **Red local** (para encontrar otros equipos) y **Grabación de pantalla** (para compartir) y **Accesibilidad** (para que te controlen; Kast → Dispositivos muestra «Dar permiso»). En Windows, las ventanas de administrador y los avisos de UAC no aceptan el control remoto, igual que en otras herramientas sin servicio del sistema.

**En una disposición (Windows):** si guardas una disposición de pantallas mientras tienes otro equipo a pantalla completa en uno de tus monitores, la disposición lo recuerda. Al aplicarla (desde Kast o con su atajo), se restauran los monitores y se vuelve a abrir esa pantalla remota en su monitor, con control si lo tenía. Las sesiones que ya coinciden se mantienen, y las de pantalla completa que no forman parte de la disposición se cierran. Para que no haya que aceptar nada, marca **Recordar este dispositivo** en el otro equipo.

**Windows y macOS se entienden entre sí**: cualquiera puede compartir y cualquiera puede ver.

| | Windows | macOS |
|---|---|---|
| Captura | DXGI Desktop Duplication | ScreenCaptureKit (NV12 directo de la GPU) |
| H.264 | Media Foundation | VideoToolbox (hardware) |
| Formato en la red | Annex B | AVCC ↔ Annex B, con el SPS ajustado a latencia mínima |

| Medido en red local (1080p, 60 fps) | |
|---|---|
| Captura + codificación H.264 | ~4 ms por fotograma |
| Decodificación | ~5 ms |
| Latencia de la captura a la imagen lista | ~5-10 ms (+ red) |
| Ancho de banda (escritorio) | 2-5 Mbps; hasta 20 Mbps con vídeo |

- **Seguridad:** cada equipo tiene una identidad propia (X25519) y la conexión usa Noise XX (autenticación mutua + ChaCha20-Poly1305). El equipo que comparte siempre decide: acepta o rechaza cada petición, y ambos muestran el mismo código derivado del cifrado para descartar a alguien en medio. Si la identidad de un dispositivo conocido cambia, la conexión se bloquea.
- **Arquitectura:** la app arranca `kast-link agent`, que se comunica con ella por stdin/stdout (una línea JSON por mensaje), anuncia el equipo por UDP y abre las ventanas del visor.
- **Todo nativo:** las APIs de captura y vídeo que ya trae cada sistema, sin códecs ni librerías externas. Escrito en Rust.
- **Hoja de ruta:** cruce transparente del ratón entre monitores de distintos equipos, portapapeles compartido, pantallas virtuales (equipos sin monitor) y varias pantallas a la vez.

Compilar: `build.ps1` lo incluye si tienes Rust (toolchain MSVC, o GNU con las binutils de MinGW-w64). Pruebas: `cargo test --manifest-path link/Cargo.toml` y `python3 link/tests/e2e.py <kast-link>`.

## Estructura

```
src/core.c      Windows: monitores, DDC/CI, comandos y configuración
src/cli.c       Windows: kast.exe
src/daemon.c    Windows: kastd.exe (bandeja, atajos y cola de trabajo)
src/webui.c     Windows: ventana WebView2 alojada en C (COM a mano)
src/ui.html     interfaz compartida por Windows y macOS
mac/Sources/    macOS: app de la barra de menús en Swift (mismo lenguaje de comandos)
link/           Kast Link: pantalla remota cifrada por la red local (Rust)
tools/          generador del icono
third_party/    SDK de WebView2 (licencia BSD de Microsoft)
```

Las versiones se publican automáticamente al subir una etiqueta `v*`: GitHub Actions compila ambos sistemas y adjunta los paquetes al Release.

## Licencia

[MIT](LICENSE). El SDK de WebView2 en `third_party/webview2` se distribuye bajo su propia [licencia](third_party/webview2/LICENSE.txt).
