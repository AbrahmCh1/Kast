// Kast Virtual Display: cómo pedirle pantallas al controlador desde una aplicación.
//
// La aplicación (kast-link) abre la interfaz GUID_KAST_DISPLAY del adaptador y le pide pantallas con IOCTLs
// (METHOD_BUFFERED). Cada pantalla tiene el tamaño y la frecuencia pedidos, exactos. Mientras haya pantallas, la
// aplicación tiene que hablar con el controlador al menos cada KAST_WATCHDOG_MS (IOCTL_KAST_PING sirve): si se cierra o
// se cuelga, el controlador las quita solas y Windows lleva sus ventanas a las pantallas que quedan.
#pragma once

// Interfaz del adaptador: {b7440647-3c97-4630-81a9-3768a8de278e}. Para un GUID: const GUID g = KAST_DISPLAY_INTERFACE;
#define KAST_DISPLAY_INTERFACE { 0xb7440647, 0x3c97, 0x4630, { 0x81, 0xa9, 0x37, 0x68, 0xa8, 0xde, 0x27, 0x8e } }

#define KAST_PROTOCOL 1
#define KAST_MAX_MONITORS 16
#define KAST_WATCHDOG_MS 3000

#define KAST_IOCTL(n) CTL_CODE(FILE_DEVICE_UNKNOWN, 0x900 + (n), METHOD_BUFFERED, FILE_ANY_ACCESS)

// Salida: KAST_INFO.
#define IOCTL_KAST_INFO KAST_IOCTL(0)
// Entrada: KAST_ADD. Salida: KAST_ADDED.
#define IOCTL_KAST_ADD KAST_IOCTL(1)
// Entrada: KAST_REMOVE.
#define IOCTL_KAST_REMOVE KAST_IOCTL(2)
// Sin datos: sigue ahí (ver KAST_WATCHDOG_MS).
#define IOCTL_KAST_PING KAST_IOCTL(3)

typedef struct KAST_INFO {
    unsigned long Protocol;
    unsigned long MaxMonitors;
    unsigned long Monitors;
    unsigned long WatchdogMs;
} KAST_INFO;

typedef struct KAST_ADD {
    unsigned long Width;
    unsigned long Height;
    // Frecuencia en Hz (0: 60).
    unsigned long RefreshHz;
    unsigned long Reserved;
} KAST_ADD;

typedef struct KAST_ADDED {
    // Número de la pantalla para KAST_REMOVE (también es el número de serie de su EDID).
    unsigned long Id;
} KAST_ADDED;

typedef struct KAST_REMOVE {
    unsigned long Id;
} KAST_REMOVE;
