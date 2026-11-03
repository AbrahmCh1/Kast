"""Música del anuncio: 100 BPM en re mayor. La percusión está hecha de sonidos de escritorio (clic de ratón como
aro, teclas mecánicas como hi-hat y palmada, el trinquete de la rueda como shaker, el relé de un monitor y un
pitido de encendido afinado en re), con mazos de vidrio y marimba, piano eléctrico FM, sub bajo y colchón.

  python ad/audio/music.py     → ad/audio/out/music.wav (48 kHz, estéreo, 24 bits, 21,2 s)
"""
import os
import sys

import numpy as np

AQUI = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.abspath(os.path.join(AQUI, "..", "..")))
import ad.timeline as T  # noqa: E402
import dsp as D  # noqa: E402

SALIDA = os.path.join(AQUI, "out", "music.wav")
N = D.ns(T.TOTAL)                 # 1 017 600 muestras
BEAT = T.BEAT
VOZ = [(p, p + (b - a)) for a, b, p in T.VO]   # frases de la locución en el anuncio

B = {k: np.zeros((2, N)) for k in ("bateria", "bajo", "teclas", "mazos", "colchon", "fx")}
BOMBOS = []                       # tiempos del bombo (para el bombeo del bajo y el colchón)


def b(x):
    """Tiempo de la rejilla (en tiempos desde el clic de 0,40 s)."""
    return T.beat(x)


def poner(bus, x, t, gan=1.0, pan=0.0):
    if x.ndim == 1:
        x = D.paneo(x, pan)
    D.sumar(B[bus], x, t, gan)


# --- percusión de escritorio ----------------------------------------------------------------------------------

def bombo(r, vel=1.0):
    """Bombo suave: seno con caída de tono que termina en La1 (quinta de re)."""
    n = D.ns(0.5)
    t = D.tv(n)
    y = np.sin(D.fase(55 + 115 * np.exp(-t / 0.018) + 18 * np.exp(-t / 0.1))) * D.env(n, 0.0015, 0.19)
    y += D.filtro(r.standard_normal(n) * D.env(n, 0.0003, 0.0025), hp=200, lp=2500) * 0.2
    return D.fades(np.tanh(1.5 * y) / np.tanh(1.5), 0.0005, 0.03) * vel


def tecla(r, vel=1.0, abierta=False):
    """Hi-hat con el chasquido de una tecla mecánica: ráfaga aguda y dos modos de la tapa de plástico."""
    n = D.ns(0.3 if abierta else 0.08)
    d = 0.07 if abierta else 0.014
    y = D.filtro(r.standard_normal(n), hp=6500, lp=13000) * D.env(n, 0.0003, d)
    y += D.modal(n, [(6900 * (1 + 0.02 * r.standard_normal()), 0.35, 0.006),
                     (9700 * (1 + 0.02 * r.standard_normal()), 0.2, 0.004)])
    return D.pico1(D.fades(y, 0.0002, 0.01)) * vel * 1.4


def palmada(r, vel=1.0):
    """Palmada: tres teclas que tocan fondo casi a la vez, más una cola corta de ruido y algo de aire."""
    n = D.ns(0.3)
    y = np.zeros(n)
    for k, dt in enumerate((0.0, 0.009, 0.02)):
        i = D.ns(dt)
        m = n - i
        c = D.modal(m, [(470 * (1 + 0.04 * r.standard_normal()), 0.7, 0.012),
                        (1120 * (1 + 0.03 * r.standard_normal()), 0.3, 0.007), (7200, 0.25, 0.003)])
        c += D.filtro(r.standard_normal(m) * D.env(m, 0.0002, 0.0012), hp=1500, lp=9000) * 0.8
        y[i:] += c * (0.6 + 0.2 * k)
    cola = D.filtro(r.standard_normal(n), hp=750, lp=1700) * D.env(n, 0.02, 0.055)
    aire = D.filtro(r.standard_normal(n), hp=6500, lp=12000) * D.env(n, 0.02, 0.045)
    y += 0.9 * cola + 0.35 * aire
    return D.pico1(D.fades(y, 0.0002, 0.02)) * vel


def clic(r, vel=1.0):
    """Aro hecho con el microinterruptor de un ratón: tic seco y dos modos de la carcasa."""
    n = D.ns(0.07)
    y = D.modal(n, [(1350 * (1 + 0.01 * r.standard_normal()), 0.8, 0.009), (2450, 0.25, 0.005),
                    (6100, 0.3, 0.003), (190, 0.35, 0.016)])
    y += D.filtro(r.standard_normal(n) * D.env(n, 0.0001, 0.0006), hp=4000, lp=12000) * 1.4
    return D.pico1(D.fades(y, 0.0002, 0.01)) * vel


def rueda(r, vel=1.0, dur=0.075, dientes=6, acelera=0.0):
    """Shaker con el trinquete de la rueda del ratón: dientes diminutos dentro de un gesto que crece y cae."""
    n = D.ns(dur + 0.03)
    y = np.zeros(n)
    u = np.linspace(0, 1, dientes)
    pos = dur * (u + acelera * (u ** 2 - u)) + r.uniform(-0.0015, 0.0015, dientes)
    m = D.ns(0.012)
    for k, t0 in enumerate(np.clip(pos, 0, dur)):
        i = D.ns(t0)
        a = np.sin(np.pi * (k + 0.5) / dientes) ** 1.5
        d = D.modal(m, [(5600 * (1 + 0.04 * r.standard_normal()), 0.6, 0.0015),
                        (8300 * (1 + 0.04 * r.standard_normal()), 0.5, 0.001)])
        d += r.standard_normal(m) * D.env(m, 0.0001, 0.0005) * 0.5
        y[i:i + m] += a * d[:n - i]
    return D.pico1(D.fades(D.filtro(y, hp=4500), 0.0002, 0.01)) * vel * 1.3


def rele(r, vel=1.0):
    """Relé de un monitor: armadura y contacto con 6,5 ms de separación y un timbre metálico corto."""
    n = D.ns(0.15)
    y = D.modal(n, [(210, 0.7, 0.025), (1900, 0.35, 0.007), (5700, 0.2, 0.004)])
    y += D.filtro(r.standard_normal(n) * D.env(n, 0.0001, 0.0006), hp=2000, lp=11000)
    i = D.ns(0.0065)
    y2 = D.modal(n - i, [(2600, 0.2, 0.006), (7900, 0.25, 0.012), (11200, 0.1, 0.008)])
    y2 += D.filtro(r.standard_normal(n - i) * D.env(n - i, 0.0001, 0.0005), hp=3000, lp=12000) * 0.7
    y[i:] += 0.6 * y2
    return D.pico1(D.fades(y, 0.0002, 0.02)) * vel


def encendido(f, vel=1.0, dur=0.5):
    """Pitido de encendido: sube desde una quinta abajo hasta f (re) y se apaga."""
    n = D.ns(dur)
    t = D.tv(n)
    ph = D.fase(f * 2 ** (-7 / 12 * np.exp(-t / 0.025)))
    y = (np.sin(ph) + 0.18 * np.sin(2 * ph) + 0.05 * np.sin(3 * ph)) * D.env(n, 0.004, 0.12)
    return D.fades(y, 0.001, 0.02) * vel


# --- instrumentos tonales -------------------------------------------------------------------------------------

def piano_el(f, dur, vel=0.6):
    """Piano eléctrico FM: cuerpo 1:1 con índice que cae y una púa 1:14 brevísima (la campanita del tine)."""
    rel = 0.22
    n = D.ns(dur + rel + 0.02)
    t = D.tv(n)
    w = 2 * np.pi * f * t
    i1 = (0.5 + 0.9 * vel) * np.exp(-t / 0.4) + 0.12
    y = np.sin(w + i1 * np.sin(w))
    y += 0.4 * vel * np.sin(w + 0.8 * vel * np.exp(-t / 0.03) * np.sin(14 * w)) * np.exp(-t / 0.12)
    y *= D.env(n, 0.003, 1.5 * (220 / f) ** 0.45) * D.puerta(n, dur, rel)
    return D.fades(y * vel, 0.001, 0.01)


def colchon(notas, dur, vel, nombre, corte=950, ataque=0.35, rel=0.6):
    """Colchón aditivo: tres voces desafinadas (izquierda, centro, derecha) con armónicos que caen pasado el corte."""
    r = D.rng(nombre)
    n = D.ns(dur + rel)
    t = D.tv(n)
    out = np.zeros((2, n))
    for nota in notas:
        f0 = D.hz(nota)
        for cents, pan in ((-7, -0.6), (0, 0.0), (7, 0.6)):
            f = f0 * 2 ** (cents / 1200)
            y = np.zeros(n)
            for k in range(1, int(2.5 * corte / f) + 1):
                y += (1 / k) / np.sqrt(1 + (k * f / corte) ** 4) * np.sin(2 * np.pi * k * f * t + r.uniform(0, 6.3))
            out += D.paneo(y, pan)
    e = np.where(t < ataque, 0.5 - 0.5 * np.cos(np.pi * t / ataque), 1.0) * D.puerta(n, dur, rel)
    return out * e * vel / len(notas)


def sub(f, dur, vel=1.0):
    """Sub bajo redondo: seno con un pequeño golpe de tono y saturación suave (armónicos para altavoces chicos)."""
    n = D.ns(dur + 0.06)
    t = D.tv(n)
    y = np.sin(D.fase(f * (1 + 0.04 * np.exp(-t / 0.012)))) * D.env(n, 0.005, 0.6) * D.puerta(n, dur, 0.05) * vel
    return D.fades(np.tanh(1.8 * y) / np.tanh(1.8) + 0.08 * y * y, 0.002, 0.01)


def golpe_grave(f, dur=1.6, vel=1.0):
    """Golpe grave afinado: cae desde 2,6× hasta f, con su octava para que se note en altavoces pequeños."""
    n = D.ns(dur)
    t = D.tv(n)
    ph = D.fase(f * (1 + 1.6 * np.exp(-t / 0.05)))
    y = np.sin(ph) * D.env(n, 0.003, dur * 0.4) + 0.3 * np.sin(2 * ph) * D.env(n, 0.003, dur * 0.2)
    return D.fades(np.tanh(1.4 * y * vel) / np.tanh(1.4), 0.001, 0.05)


def reverso(seco, ir, largo, seco_g=0.35):
    """Seco más su reverberación, al revés: crece durante `largo` s hasta el ataque."""
    w = D.reverb(seco, ir)
    w[:, :seco.shape[1]] += seco_g * seco
    y = w[:, ::-1][:, -D.ns(largo):]
    u = np.linspace(0, 1, y.shape[1])
    return D.fades(y * u ** 1.5, 0.0, 0.004)


# --- armonía --------------------------------------------------------------------------------------------------

ACORDES = {  # bajo, piano eléctrico, colchón
    "D": ("D2", ["F#3", "A3", "D4", "E4"], ["D3", "A3", "F#4"]),
    "Bm7": ("B1", ["F#3", "A3", "B3", "D4"], ["B2", "F#3", "D4"]),
    "Gmaj7": ("G1", ["G3", "B3", "D4", "F#4"], ["G2", "D3", "B3"]),
    "A7sus4": ("A1", ["G3", "A3", "D4", "E4"], ["A2", "E3", "D4"]),
    "A7": ("A1", ["G3", "A3", "C#4", "E4"], ["A2", "E3", "C#4"]),
    "Em7": ("E2", ["G3", "B3", "D4", "E4"], ["E3", "G3", "B3"]),
    "Gsube": ("G1", ["B3", "D4", "F#4", "A4"], ["G2", "D3", "B3", "F#4"]),
    "Asus": ("A1", ["D4", "E4", "A4", "B4"], ["A2", "E3", "D4"]),
    "Asube": ("A1", ["C#4", "E4", "A4", "B4"], ["A2", "E3", "C#4", "E4"]),
    "Dfin": ("D2", ["D3", "A3", "E4", "F#4", "A4", "D5"], ["D3", "A3", "F#4", "A4"]),
}

# (tiempo de la rejilla, acorde); None = la pausa dramática
PROG = [(0, "D"), (4, "D"), (6, "Bm7"), (8, "Gmaj7"), (10, "A7sus4"), (11, "A7"),
        (12, "D"), (14, "Bm7"), (16, "Gmaj7"), (18, "A7sus4"), (19, "A7"),
        (20, "Bm7"), (22, "Em7"), (24, "Gsube"), (26, "Asus"), (26.5, "Asube"), (27, None), (28, "Dfin")]

NIVEL_COLCHON = {0: 0.55, 4: 0.28, 6: 0.28, 8: 0.3, 10: 0.3, 11: 0.3, 12: 0.3, 14: 0.3, 16: 0.34, 18: 0.48,
                 19: 0.52, 20: 0.6, 22: 0.6, 24: 0.6, 26: 0.6, 26.5: 0.6, 28: 0.75}


def armonia():
    r = D.rng("armonia")
    for i, (bt, nom) in enumerate(PROG):
        if nom is None:
            continue
        fin = PROG[i + 1][0] if i + 1 < len(PROG) else None
        _, piano, notas_c = ACORDES[nom]

        # colchón: muy discreto, sigue los acordes; en la pausa se corta rápido
        if nom == "Dfin":
            poner("colchon", colchon(notas_c, 2.4, NIVEL_COLCHON[bt], "c_fin", corte=1100, ataque=0.08, rel=1.6),
                  b(bt))
        else:
            dur = (fin - bt) * BEAT
            corta = PROG[i + 1][1] is None
            poner("colchon", colchon(notas_c, dur, NIVEL_COLCHON[bt], f"c{bt}",
                                     ataque=0.9 if bt == 0 else 0.3, rel=0.12 if corta else 0.6), b(bt))

        # piano eléctrico desde el compás 2
        if bt < 4:
            continue
        largo = (fin - bt) if fin is not None else 0
        if nom == "Dfin":
            golpes = [(0, 2.6 / BEAT, 0.72)]
        elif bt < 12:
            golpes = [(0, largo * 0.95, 0.55)] + ([(1.5, 0.45, 0.3)] if largo >= 2 else [])
        elif bt < 24:
            golpes = [(0, 1.2, 0.55), (1.5, 0.4, 0.42)] if largo >= 2 else [(0, 0.85, 0.5)]
            if bt < 16:
                golpes = [(o, d, v * 0.85) for o, d, v in golpes]
            if bt >= 20:
                golpes.append((0.75, 0.2, 0.28))
        else:
            golpes = [(0, largo * 0.97, 0.5)] + ([(1.5, 0.4, 0.3)] if largo >= 2 else [])
        for off, dur_b, vel in golpes:
            for k, nota in enumerate(piano):
                f = D.hz(nota)
                v = vel * (1 + 0.06 * r.standard_normal())
                pan = float(np.clip((np.log2(f / 260)) * 0.5, -0.4, 0.4))
                poner("teclas", piano_el(f, dur_b * BEAT, v), b(bt + off) + 0.006 * k, pan=pan)


# --- bajo -----------------------------------------------------------------------------------------------------

def bajo():
    # compás 1: golpe grave con el clic; compás 3: insinuado en 1 y 3
    poner("bajo", golpe_grave(D.hz("D1"), 1.4, 0.7), b(0))
    poner("bajo", sub(D.hz("G1"), 0.9 * BEAT, 0.8), b(8))
    poner("bajo", sub(D.hz("A1"), 0.9 * BEAT, 0.8), b(10))
    # compases 4–6: tresillo 3+3+2 (raíz, raíz, octava)
    for bt, raiz in ((12, "D2"), (14, "B1"), (16, "G1"), (18, "A1"), (20, "B1"), (22, "E2")):
        f = D.hz(raiz)
        golpes = [(0, 0.7, f, 1.0), (0.75, 0.65, f, 0.85), (1.5, 0.45, 2 * f, 0.7)]
        if bt == 22:
            golpes[-1] = (1.5, 0.45, D.hz("F#2"), 0.8)    # paso hacia sol
        for off, dur_b, ff, v in golpes:
            poner("bajo", sub(ff, dur_b * BEAT, v), b(bt + off))
    # compás 7: notas largas, se corta en la pausa
    poner("bajo", sub(D.hz("G1"), 1.9 * BEAT, 0.85), b(24))
    poner("bajo", sub(D.hz("A1"), 0.95 * BEAT, 0.85), b(26))
    # aterrizaje: golpe grave en re
    poner("bajo", golpe_grave(D.hz("D1"), 2.2, 1.0), b(28))
    poner("bajo", sub(D.hz("D2"), 1.2, 0.45), b(28))
    # firma: un re grave suave bajo el re agudo final
    poner("bajo", sub(D.hz("D2"), 0.5, 0.3), b(30.75))


# --- batería de escritorio ------------------------------------------------------------------------------------

def bateria():
    r = D.rng("bateria")

    def golpe(inst, x, vel, gan=1.0, pan=0.0, **kw):
        if inst is bombo:
            BOMBOS.append(b(x))
        poner("bateria", inst(r, vel, **kw), b(x), gan, pan)

    # compás 3 (encendido): bombo en 1 y 3, rueda muy suave
    golpe(rele, 8, 0.35, pan=0.2)
    for x in (8, 10):
        golpe(bombo, x, 0.75)
    for x in np.arange(8.5, 12, 1.0):
        golpe(rueda, x, 0.18, pan=0.3)

    # compases 4–6
    for bar in (12, 16, 20):
        for s in range(16):
            x = bar + s / 4
            mas = 1.12 if bar == 20 else 1.0               # el compás 6 es el más lleno
            if s in (0, 8) or (s == 10) or (bar == 20 and s == 3):
                golpe(bombo, x, {0: 1.0, 8: 0.85, 10: 0.45, 3: 0.35}[s] * mas)
            if s in (4, 12):
                golpe(palmada, x, 0.8 * mas, pan=0.05)
            if s % 2 == 0:
                golpe(tecla, x, (0.32 if s % 4 == 0 else 0.62) * (0.8 if bar == 12 else 1.0), pan=0.35)
            elif bar == 20:
                golpe(tecla, x, 0.2, pan=0.35)
            if bar == 20 and s in (6, 14):                  # hi-hat abierto en el compás más lleno
                golpe(tecla, x, 0.3, pan=0.3, abierta=True)
            if bar == 20 and s == 11:                       # palmada fantasma
                golpe(palmada, x, 0.3, pan=-0.1)
            if bar == 12 and s % 4 == 2:
                golpe(rueda, x, 0.4, pan=-0.35)
            elif bar > 12:
                golpe(rueda, x, 0.45 if s % 4 == 2 else 0.22, pan=-0.35)
            if s == 7 and bar != 16:
                golpe(clic, x, 0.42, pan=-0.15)
            if s == 14 and bar != 16:                       # doble clic
                golpe(clic, x, 0.5, pan=-0.15)
                golpe(clic, x + 0.12, 0.38, pan=-0.15)
    golpe(rele, 12, 0.6, pan=0.2)
    golpe(rele, 18, 0.45, pan=-0.2)
    golpe(rele, 20, 0.7, pan=0.2)

    # crecida (mitad del compás 5): hi-hat en semicorcheas, redoble de clics y la rueda que se acelera
    for s in range(8):
        golpe(tecla, 18 + s / 4 + 0.125, 0.15 + 0.03 * s, pan=0.35)
    for x, v in ((19, 0.25), (19.25, 0.3), (19.5, 0.34), (19.625, 0.4), (19.75, 0.47), (19.875, 0.55)):
        golpe(clic, x, v, pan=-0.15)
    golpe(rueda, 19, 0.5, pan=-0.2, dur=0.55, dientes=28, acelera=-0.6)
    golpe(tecla, 19.5, 0.35, pan=0.3, abierta=True)

    # compás 7: más ligero
    golpe(bombo, 24, 0.85)
    golpe(bombo, 26, 0.6)
    golpe(palmada, 25, 0.5, pan=0.05)
    for x in np.arange(24, 27, 0.5):
        golpe(tecla, x, 0.25 if x % 1 == 0 else 0.4, pan=0.35)
        golpe(rueda, x + 0.25, 0.22, pan=-0.35)

    # aterrizaje
    golpe(bombo, 28, 1.0)


# --- mazos ----------------------------------------------------------------------------------------------------

def mazos():
    V, M = D.vidrio, D.marimba
    # (tiempo, nota, instrumento, intensidad, paneo)
    notas = [
        # compás 1: pocas notas
        (0, "D5", V, 0.5, 0.15), (0, "A4", M, 0.45, -0.1), (1.5, "F#5", M, 0.35, -0.3),
        (2.75, "E5", M, 0.3, 0.3), (3.5, "A4", M, 0.3, -0.1),
        # compás 2: casi nada
        (7.5, "B4", M, 0.22, 0.2),
        # encendido
        (8, "D5", V, 0.32, -0.2), (8, "A5", V, 0.22, 0.25),
        # final del compás 4: respuesta corta
        (15.5, "F#5", M, 0.28, -0.3), (15.75, "A5", M, 0.24, 0.3),
    ]
    # compás 5: ostinato en corcheas
    arpegio = {"G": ["G4", "D5", "B4", "D5"], "A": ["A4", "E5", "C#5", "E5"],
               "Bm": ["B4", "F#4", "D5", "F#4", "A4", "F#4", "D5", "B4"],
               "Em": ["E4", "B4", "G4", "B4", "D5", "B4", "G4", "E4"]}
    for k in range(8):
        tonos = arpegio["G" if k < 4 else "A"]
        notas.append((16 + 0.5 * k, tonos[k % 4], M, 0.3 if k % 2 == 0 else 0.24, -0.35 if k % 2 == 0 else 0.35))
    # compás 6: semicorcheas con acentos 3+3+2
    for k in range(16):
        tonos = arpegio["Bm" if k < 8 else "Em"]
        acento = (k % 8) in (0, 3, 6)
        notas.append((20 + 0.25 * k, tonos[k % 8], M, 0.38 if acento else 0.22, -0.35 if k % 2 == 0 else 0.35))
    # compás 7: línea de vidrio que sube
    for x, nota, v in ((24, "B4", 0.25), (24.5, "D5", 0.28), (25, "F#5", 0.31), (25.5, "A5", 0.34),
                       (26, "C#5", 0.3), (26.25, "E5", 0.33), (26.5, "A5", 0.36), (26.75, "B5", 0.4)):
        notas.append((x, nota, V, v, 0.25 * np.sin(3 * x)))
    # aterrizaje: re mayor
    notas += [(28, "D5", V, 0.42, -0.2), (28, "F#5", V, 0.32, 0.2), (28, "A5", V, 0.28, 0.0), (28, "D4", M, 0.45, 0.0)]
    # firma de Kast: arpegio re–fa#–la–re' en semicorcheas
    for k, (nota, v) in enumerate((("D5", 0.4), ("F#5", 0.44), ("A5", 0.48), ("D6", 0.6))):
        x = 30 + 0.25 * k
        notas.append((x, nota, V, v, -0.3 + 0.2 * k))
        notas.append((x, nota[:-1] + str(int(nota[-1]) - 1), M, 0.2, -0.3 + 0.2 * k))
    for x, nota, inst, v, pan in notas:
        f = D.hz(nota)
        y = inst(f, v, brillo=0.8, caida=1.1) if (inst is V and x >= 30) else inst(f, v)
        poner("mazos", y, b(x), pan=pan)
    poner("mazos", D.campanita(D.hz("D6"), 0.25, caida=1.5), b(30.75), pan=0.1)
    poner("mazos", D.campanita(D.hz("A6"), 0.08, caida=1.2), b(30.75), pan=-0.1)


# --- efectos musicales: subida inicial, encendido, crecida, swell inverso y clic final ---------------------------

def efectos(ir_larga):
    r = D.rng("fx")
    # 0,06–0,40: vidrio en re al revés que se aspira hasta el primer golpe
    seco = sum(D.paneo(D.vidrio(D.hz(nn), v), p) for nn, v, p in (("D5", 0.6, -0.3), ("A5", 0.45, 0.3),
                                                                      ("D6", 0.3, 0.0)))
    ini = reverso(seco, ir_larga, 0.34, seco_g=0.5)
    poner("fx", ini, b(0) - ini.shape[1] / D.SR, 0.9)
    # 5,20: pitido de encendido afinado en re
    poner("fx", encendido(D.hz("D5"), 0.5), b(8), pan=-0.15)
    poner("fx", encendido(D.hz("A5"), 0.25), b(8) + 0.02, pan=0.2)
    # 11,20–12,40: ruido que sube durante la crecida
    dur = b(20) - b(18)
    y = D.ruido_movil(r, dur, [(0, 900), (1, 7000)], 0.8) * np.linspace(0, 1, D.ns(dur)) ** 2.5 * 0.25
    poner("fx", D.fades(y, 0.01, 0.006), b(18), pan=0.0)
    # 16,60–17,20: re mayor al revés hacia el aterrizaje
    seco = np.zeros((2, D.ns(2.5)))
    for k, nota in enumerate(ACORDES["Dfin"][1]):
        D.sumar(seco, D.paneo(piano_el(D.hz(nota), 1.5, 0.6), 0.3 * np.sin(k)), 0.0, 1.0)
    for nota in ("D5", "F#5", "A5"):
        D.sumar(seco, D.paneo(D.vidrio(D.hz(nota), 0.5), 0.0), 0.0, 1.0)
    sw = reverso(seco, ir_larga, b(28) - b(27), seco_g=0.4)
    poner("fx", sw, b(28) - sw.shape[1] / D.SR, 0.75)
    # 19,00: clic satinado diminuto que cierra la firma
    poner("fx", D.clic_satinado(r, cuerpo=0.6, tic=0.5, tono=1.4) * 0.3, b(31), pan=0.1)


# --- mezcla ---------------------------------------------------------------------------------------------------

def bombeo(prof, tau):
    """Ganancia que baja con cada bombo y se recupera (sidechain)."""
    e = np.ones(N)
    m = D.ns(tau * 5)
    k = np.arange(m)
    curva = 1 - prof * np.exp(-k / D.ns(tau)) * np.clip(k / D.ns(0.004), 0, 1)
    for tb in BOMBOS:
        i = D.ns(tb)
        j = min(N, i + m)
        e[i:j] = np.minimum(e[i:j], curva[:j - i])
    return e


def mezclar():
    ir = D.sala(3.0, 2.3, 1.1, "sala_musica", pre=0.02, hp=200, lp=8000)
    armonia()
    bajo()
    bateria()
    mazos()
    efectos(ir)

    t = D.tv(N)
    B["bajo"] = D.filtro(B["bajo"], hp=28, lp=420) * bombeo(0.45, 0.07)
    B["colchon"] = D.filtro(B["colchon"], hp=110) * bombeo(0.3, 0.16)
    B["teclas"] = D.filtro(B["teclas"], hp=90) * bombeo(0.12, 0.12)
    # trémolo estéreo lento del piano (en oposición: la suma mono no cambia)
    lfo = 0.16 * np.sin(2 * np.pi * t / (2 * BEAT))
    B["teclas"] *= np.stack([1 + lfo, 1 - lfo])
    B["bateria"] = D.filtro(B["bateria"], hp=35)

    nivel = {"bateria": -1.0, "bajo": -3.0, "teclas": -5.0, "mazos": -4.0, "colchon": -10.0, "fx": -5.0}
    envio_sala = {"bateria": 0.08, "teclas": 0.22, "mazos": 0.32, "colchon": 0.3, "fx": 0.0, "bajo": 0.0}
    envio_eco = {"teclas": 0.06, "mazos": 0.2}
    # pausa dramática (16,60–17,20): todo se apaga salvo el swell inverso (bus fx)
    pausa = np.clip((t - b(27)) / 0.08, 0, 1) * np.clip((b(28) - t) / 0.2, 0, 1)
    for k in B:
        if k != "fx":
            B[k] *= 1 - 0.95 * pausa
    seco = sum(D.g(nivel[k]) * B[k] for k in B)
    envio = sum(D.g(nivel[k]) * envio_sala[k] * B[k] for k in B)
    humedo = D.reverb(envio, ir)[:, :N]
    humedo += D.eco(sum(D.g(nivel[k]) * envio_eco[k] * B[k] for k in envio_eco), 0.75 * BEAT, fb=0.35)
    mix = seco + humedo * (1 - 0.85 * pausa)

    mix = D.filtro(mix, hp=25, picos=[(3800, -2.0, 1.0), (11000, 2.5, 0.6)])
    mix = D.hueco_voz(mix, VOZ, -4.5)
    # cola: se apaga suave hasta el silencio a los 21,05 s
    u = np.clip((t - 19.4) / (21.05 - 19.4), 0, 1)
    mix *= np.cos(0.5 * np.pi * u) ** 2
    mix *= D.g(-6.0) / np.abs(mix).max()
    return mix, {k: B[k] * D.g(nivel[k]) for k in B}


if __name__ == "__main__":
    mix, buses = mezclar()
    D.escribir_wav(SALIDA, mix)
    print("buses (RMS dB antes de normalizar, por compás):")
    cortes = [0, 0.4, 2.8, 5.2, 7.6, 10.0, 12.4, 14.8, 16.6, 17.2, 18.4, 19.2, 21.2]
    print("   " + " ".join(f"{c:6.2f}" for c in cortes[:-1]))
    for k, x in buses.items():
        print(f"   {k:8s}" + " ".join(f"{D.dbfs(np.sqrt(np.mean(x[:, D.ns(a):D.ns(c)] ** 2)) + 1e-12):6.1f}"
                                     for a, c in zip(cortes, cortes[1:])))
    D.revisar(SALIDA, VOZ)
