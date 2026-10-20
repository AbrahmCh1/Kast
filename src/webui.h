/* Ventana de configuración (WebView2), creada sólo mientras está abierta. */
#pragma once
#include <wchar.h>

void ui_open(void);
void ui_close(void);
int ui_is_open(void);
void ui_post_json(const char *json);

/* Implementadas por daemon.c */
void on_ui_message(const wchar_t *msg);
void on_ui_closed(void);
