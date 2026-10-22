// API en C del núcleo de Kast Link (link/src/mobile.rs), enlazado como biblioteca estática (libkast_link.a).
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Eventos en JSON (desde hilos en segundo plano): devices, session, clip, toast.
typedef void (*kl_event_cb)(void *ctx, const char *json);
// Fotograma H.264 en AVCC; sps/pps (sin código de inicio) sólo en los fotogramas clave.
typedef void (*kl_frame_cb)(void *ctx, uint64_t session, const uint8_t *avcc, size_t len,
                            const uint8_t *sps, size_t sps_len, const uint8_t *pps, size_t pps_len);

bool kl_start(const char *dir, const char *name, void *ctx, kl_event_cb ev, kl_frame_cb frame);
// {"cmd":"refresh"} · {"cmd":"add","addr"} · {"cmd":"forget","id"} · {"cmd":"disconnect","session"}
void kl_command(const char *json);
uint64_t kl_connect(const char *id, uint32_t monitor, bool control);
// Con la contraseña de acceso desatendido (auth = "password") o un código de emparejamiento (auth = "pair").
// El secreto no se envía: sólo una prueba ligada a la conexión. `id` también puede ser el ID de 9 dígitos.
uint64_t kl_connect_auth(const char *id, uint32_t monitor, bool control, const char *auth, const char *secret);

// Pide un fotograma clave (al abrir la vista o si el decodificador falló).
void kl_keyframe(uint64_t session);

void kl_mouse_move(uint64_t session, int32_t x, int32_t y);
void kl_mouse_button(uint64_t session, uint8_t button, bool down, int32_t x, int32_t y);
void kl_wheel(uint64_t session, int32_t dx, int32_t dy);
void kl_key(uint64_t session, const char *code, bool down);
void kl_text(uint64_t session, const char *text);
void kl_clip_text(uint64_t session, const char *text);
void kl_clip_png(uint64_t session, const uint8_t *data, size_t len);
