// EDID 1.4 de las pantallas de Kast: nombre «Kast», número de serie propio y, si cabe en un EDID básico, el modo pedido
// como modo preferido (tiempos CVT con borrado reducido). El tamaño físico corresponde a 96 ppp, así Windows propone el
// 100 % de escala y Kast pone la del monitor del visor.
#pragma once

#include <windows.h>
#include <cmath>
#include <cstring>

namespace Kast
{
    constexpr size_t EdidSize = 128;

    // Número de serie (bytes 12 a 15) de un EDID de Kast.
    inline ULONG EdidSerial(const BYTE* e)
    {
        return e[12] | (e[13] << 8) | (e[14] << 16) | ((ULONG)e[15] << 24);
    }

    // Descriptor de texto (nombre, número de serie…): hasta 13 caracteres, terminado en salto de línea.
    inline void EdidText(BYTE* d, BYTE tag, const char* text)
    {
        memset(d, 0, 18);
        d[3] = tag;
        size_t n = strlen(text);
        if (n > 13) n = 13;
        memcpy(d + 5, text, n);
        if (n < 13)
        {
            d[5 + n] = 0x0A;
            for (size_t i = n + 1; i < 13; i++) d[5 + i] = 0x20;
        }
    }

    // Modo preferido (Detailed Timing Descriptor). Falso si no cabe en un EDID básico (más de 4095 píxeles o reloj
    // por encima de 655 MHz): entonces el modo sólo va en la lista que da el controlador.
    inline bool EdidTiming(BYTE* d, DWORD w, DWORD h, DWORD hz)
    {
        const DWORD hfp = 48, hsync = 32, hblank = 160;
        const DWORD vfp = 3, vsync = 5;
        // Borrado vertical mínimo de 460 µs (CVT con borrado reducido).
        DWORD vblank = (DWORD)ceil(460e-6 * hz * (h + 20.0)) + 1;
        if (vblank < vfp + vsync + 6) vblank = vfp + vsync + 6;
        const ULONGLONG clock = (ULONGLONG)(w + hblank) * (h + vblank) * hz / 10000; // en unidades de 10 kHz
        if (w > 4095 || h > 4095 || clock > 0xFFFF) return false;
        const DWORD wmm = w * 254 / 960, hmm = h * 254 / 960; // a 96 ppp
        d[0] = (BYTE)(clock & 0xFF);
        d[1] = (BYTE)(clock >> 8);
        d[2] = (BYTE)(w & 0xFF);
        d[3] = (BYTE)(hblank & 0xFF);
        d[4] = (BYTE)(((w >> 8) << 4) | (hblank >> 8));
        d[5] = (BYTE)(h & 0xFF);
        d[6] = (BYTE)(vblank & 0xFF);
        d[7] = (BYTE)(((h >> 8) << 4) | (vblank >> 8));
        d[8] = (BYTE)(hfp & 0xFF);
        d[9] = (BYTE)(hsync & 0xFF);
        d[10] = (BYTE)(((vfp & 0xF) << 4) | (vsync & 0xF));
        d[11] = (BYTE)(((hfp >> 8) << 6) | ((hsync >> 8) << 4) | ((vfp >> 4) << 2) | (vsync >> 4));
        d[12] = (BYTE)(wmm & 0xFF);
        d[13] = (BYTE)(hmm & 0xFF);
        d[14] = (BYTE)(((wmm >> 8) << 4) | ((hmm >> 8) & 0xF));
        d[15] = 0;
        d[16] = 0;
        d[17] = 0x1A; // digital, sincronías separadas: horizontal positiva, vertical negativa (CVT-RB)
        return true;
    }

    inline void BuildEdid(BYTE (&e)[EdidSize], DWORD w, DWORD h, DWORD hz, ULONG serial)
    {
        memset(e, 0, EdidSize);
        static const BYTE header[8] = { 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00 };
        memcpy(e, header, 8);
        // Fabricante «KST» (tres letras de 5 bits) y producto.
        const WORD maker = (WORD)((('K' - '@') << 10) | (('S' - '@') << 5) | ('T' - '@'));
        e[8] = (BYTE)(maker >> 8);
        e[9] = (BYTE)(maker & 0xFF);
        e[10] = 0x01;
        e[11] = 0x4B;
        e[12] = (BYTE)(serial & 0xFF);
        e[13] = (BYTE)((serial >> 8) & 0xFF);
        e[14] = (BYTE)((serial >> 16) & 0xFF);
        e[15] = (BYTE)(serial >> 24);
        e[16] = 1;    // semana
        e[17] = 36;   // 2026
        e[18] = 1;    // EDID 1.4
        e[19] = 4;
        e[20] = 0xA5; // digital, 8 bits por color, DisplayPort
        const DWORD wcm = w * 254 / 9600, hcm = h * 254 / 9600;
        e[21] = (BYTE)(wcm > 255 ? 255 : wcm);
        e[22] = (BYTE)(hcm > 255 ? 255 : hcm);
        e[23] = 0x78; // gamma 2,2
        // Cromaticidad sRGB.
        static const BYTE chroma[10] = { 0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54 };
        memcpy(e + 25, chroma, 10);
        for (int i = 38; i < 54; i++) e[i] = 0x01; // sin modos estándar
        const bool timing = EdidTiming(e + 54, w, h, hz);
        // RGB, sRGB por defecto y, si hay modo preferido, que es el nativo.
        e[24] = timing ? 0x06 : 0x04;
        if (!timing) EdidText(e + 54, 0x10, ""); // descriptor vacío
        EdidText(e + 72, 0xFC, "Kast");
        // Límites: 24-240 Hz, 1-255 kHz, hasta 2550 MHz.
        BYTE* r = e + 90;
        memset(r, 0, 18);
        r[3] = 0xFD;
        r[5] = 24;
        r[6] = 240;
        r[7] = 1;
        r[8] = 255;
        r[9] = 255;
        r[10] = 0x01;
        r[11] = 0x0A;
        for (int i = 12; i < 18; i++) r[i] = 0x20;
        char sn[14];
        sprintf_s(sn, "KAST-%08lX", serial);
        EdidText(e + 108, 0xFF, sn);
        e[126] = 0; // sin bloques de extensión
        BYTE sum = 0;
        for (size_t i = 0; i < EdidSize - 1; i++) sum += e[i];
        e[127] = (BYTE)(0x100 - sum);
    }
}
