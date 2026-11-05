# Kast Virtual Display

Controlador de pantallas virtuales de Kast para Windows (Indirect Display Driver, IddCx sobre UMDF 2). Con él, quien ve
este equipo desde Kast Link le añade pantallas del tamaño y la frecuencia exactos de sus monitores, o le crea la suya si
no tiene monitor. Sustituye al controlador de Parsec (Parsec VDD), que Kast sigue usando si este no está instalado.

## Qué hace mejor que el de Parsec

- **Tamaño y frecuencia exactos** de cada pantalla (también ultrapanorámicas o de más de 60 Hz), no el modo más
  parecido de una lista fija. Los habituales más pequeños quedan disponibles en Configuración → Pantalla.
- **Nada que vigilar al milímetro**: Kast avisa cada 500 ms y las pantallas sólo se quitan si deja de hacerlo durante
  3 s (el de Parsec las quita a los ~100 ms). Un tirón del equipo ya no hace saltar las ventanas.
- **Viene con Kast**: se instala con un clic desde la app (Dispositivos → Instalar pantallas virtuales).
- Las pantallas aparecen como **«Kast»** y a 96 ppp, así Windows no les impone una escala y Kast pone la del monitor del
  visor.

## Piezas

| Archivo | Qué es |
| --- | --- |
| `KastDisplay/Driver.cpp`, `Driver.h` | El controlador: adaptador, pantallas a petición, modos, cadena de intercambio y vigilancia |
| `KastDisplay/Edid.h` | EDID 1.4 de cada pantalla (nombre, número de serie, modo preferido con tiempos CVT-RB) |
| `KastDisplay/KastDisplay.inf` | Instalación (dispositivo raíz `Root\KastDisplay`, clase Display) |
| `include/kastdisplay.h` | Cómo le pide pantallas una aplicación (interfaz e IOCTLs) |
| `build.ps1` | Compila y firma el paquete (`out\KastDisplay`) |
| `kast-vdd.cer` | Certificado público de Kast con el que va firmado |

## Compilar

```powershell
.\vdd\build.ps1 [-Version 1.2.3.0] [-Pfx firma.pfx -PfxPassword ...]
```

Usa el compilador de C++ de Visual Studio (Desarrollo para el escritorio con C++) y el WDK y el SDK de Windows como
paquetes NuGet (`packages.config`, se descargan solos en `vdd\packages`): no hace falta instalar el WDK. La CI lo
compila y lo firma con el certificado de Kast (secretos `KAST_VDD_PFX`, en base64, y `KAST_VDD_PFX_PASSWORD`).

## Firma e instalación

Es un controlador de modo usuario (UMDF): Windows no le exige la firma de Microsoft, pero sólo instala paquetes firmados
por alguien en quien confía. Va firmado con el certificado propio de Kast (`kast-vdd.cer`, sólo para firmar código), y
`kast-link vdd install` (lo que hace el botón de la app, con el permiso de administrador que pide Windows):

1. Añade el certificado a «Entidades de certificación raíz de confianza» y «Editores de confianza» del equipo.
2. Añade el paquete al almacén de controladores (`pnputil /add-driver`).
3. Crea el adaptador «Kast Virtual Display» (sin pantallas hasta que Kast las pida).

`kast-link vdd uninstall` deshace las tres cosas; `kast-link vdd status` dice qué controlador se usa.

## Licencia

Basado en el ejemplo
[IddSampleDriver](https://github.com/microsoft/Windows-driver-samples/tree/main/video/IndirectDisplay) de Microsoft.
Los archivos de esta carpeta están bajo la Microsoft Public License (MS-PL, `LICENSE.txt`); el resto de Kast, bajo MIT.
