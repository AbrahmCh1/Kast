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

## Estructura

```
src/core.c      Windows: monitores, DDC/CI, comandos y configuración
src/cli.c       Windows: kast.exe
src/daemon.c    Windows: kastd.exe (bandeja, atajos y cola de trabajo)
src/webui.c     Windows: ventana WebView2 alojada en C (COM a mano)
src/ui.html     interfaz compartida por Windows y macOS
mac/Sources/    macOS: app de la barra de menús en Swift (mismo lenguaje de comandos)
tools/          generador del icono
third_party/    SDK de WebView2 (licencia BSD de Microsoft)
```

Las versiones se publican automáticamente al subir una etiqueta `v*`: GitHub Actions compila ambos sistemas y adjunta los paquetes al Release.

## Licencia

[MIT](LICENSE). El SDK de WebView2 en `third_party/webview2` se distribuye bajo su propia [licencia](third_party/webview2/LICENSE.txt).
