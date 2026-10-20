/* kast — núcleo compartido entre kast.exe (CLI) y kastd.exe (bandeja + atajos + UI). */
#pragma once
#define WIN32_LEAN_AND_MEAN
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0A00
#endif
#include <windows.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

/* Versión: build.ps1 -Version 1.2.3 define KAST_VERSION_RAW=1.2.3 (sin comillas, para no depender
 * de cómo cada shell pasa las comillas a zig) y aquí se convierte en cadena. */
#define KAST_STR2(x) #x
#define KAST_STR(x) KAST_STR2(x)
#ifdef KAST_VERSION_RAW
#define KAST_VERSION KAST_STR(KAST_VERSION_RAW)
#else
#define KAST_VERSION "dev"
#endif

#define WNDCLASS_NAME  "KastDaemon"
#define WM_KAST_RELOAD (WM_APP + 1)
#define WM_KAST_OPENUI (WM_APP + 2)
#define WM_KAST_RESULT (WM_APP + 3) /* lParam: char* JSON (malloc) para la UI */
#define WM_KAST_TRAY   (WM_APP + 4)
#define WM_KAST_LINK   (WM_APP + 5) /* lParam: char* línea JSON del agente de Kast Link (malloc); NULL = terminó */
#define MAXHK  128
#define MAXFAV 64

typedef struct { char *p; size_t n, cap; } Sb;
void sb_add(Sb *s, const char *p, size_t n);
void sb_printf(Sb *s, const char *fmt, ...);
void sb_json(Sb *s, const char *str);

typedef struct { char *name, *act; } Fav;
typedef struct { UINT mods, vk; char *combo, *act; } Hotkey;

extern char exedir[MAX_PATH];
extern Fav favs[MAXFAV];
extern int nfav;
extern Hotkey hks[MAXHK];
extern int nhk;
extern CRITICAL_SECTION cfg_cs;

void core_init(void);
int exec_line(char *s);
int say(const char *fmt, ...);
void out_capture(void);
char *out_take(void);
void load_config(Sb *log);
void ini_path(char *out);
char *state_json(const char *log);
int autostart_get(void);
int autostart_set(int on);
void cache_clear(void);
void free_phys(void);
HWND find_daemon(void);
/* Disposiciones con sesiones de Kast Link: el servicio pasa "save"/"load" y la ruta del .link al agente.
 * En kast.exe (sin agente) se reenvía al servicio por WM_COPYDATA. */
extern void (*link_layout_hook)(const char *op, const char *path);
#define KAST_COPY_LINK_LAYOUT 0x4B4C4E4Bu
/* Disposiciones con sesiones de Kast Link: el servicio pasa "save"/"load" y la ruta del .link al agente.
 * En kast.exe (sin agente) se reenvía al servicio por WM_COPYDATA. */
extern void (*link_layout_hook)(const char *op, const char *path);
#define KAST_COPY_LINK_LAYOUT 0x4B4C4E4Bu
int launch_daemon(const char *args);
