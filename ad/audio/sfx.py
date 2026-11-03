"""Efectos de sonido del anuncio en una sola pista estéreo y la lista de cues con su pico.

Jerarquía: el aterrizaje (17,20) es lo más fuerte y el clic inicial (0,40) lo segundo; los sonidos de marca
(confirmación 4,60 y conectado 9,40) van después; el resto es discreto. Cada cue se normaliza a su nivel de
diseño (0 dB = aterrizaje), se le quita el grave (salvo su capa sub), se suaviza la zona áspera de 3–5 kHz y
se le abre hueco a la locución en 1–4 kHz mientras habla.

  python ad/audio/sfx.py     → ad/audio/out/sfx.wav y ad/audio/out/sfx_cues.txt
"""
import json
import os
import sys

import numpy as np

AQUI = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.abspath(os.path.join(AQUI, "..", "..")))
import ad.timeline as T  # noqa: E402
import dsp as D  # noqa: E402

SALIDA = os.path.join(AQUI, "out", "sfx.wav")
LISTA = os.path.join(AQUI, "out", "sfx_cues.txt")
N = D.ns(T.TOTAL)
VOZ = [(p, p + (b - a)) for a, b, p in T.VO]


def palabras_voz(pausa_min=0.12):
    """Tramos con voz, palabra a palabra (marcas por carácter de la toma llevadas al anuncio); une pausas cortas."""
    ruta = os.path.join(AQUI, "..", "..", os.path.splitext(T.VO_TAKE)[0] + ".json")
    if not os.path.exists(ruta):
        return VOZ
    al = json.load(open(ruta, encoding="utf-8"))
    al = al.get("alignment", al)
    tramos = []
    for c, s, e in zip(al["characters"], al["character_start_times_seconds"], al["character_end_times_seconds"]):
        for a, b, p in T.VO:
            if c.isalnum() and a - 0.01 <= s <= b + 0.05:
                tramos.append([p + s - a, p + e - a])
    unidos = []
    for s, e in sorted(tramos):
        if unidos and s - unidos[-1][1] < pausa_min:
            unidos[-1][1] = max(unidos[-1][1], e)
        else:
            unidos.append([s, e])
    return [tuple(x) for x in unidos]


PALABRAS = palabras_voz()   # el hueco de los efectos sigue a las palabras (se abre en las pausas)
SALA = D.sala(1.4, 1.1, 0.5, "sala_sfx", pre=0.008, hp=220, lp=9000)
PRE = 0.03                  # margen antes de cada cue (los filtros no cortan el ataque)
PROF_HUECO = -6.0           # dB en 1–4 kHz mientras habla la locución
CUES = []                   # (tiempo, nombre, buffer procesado)


# --- utilidades -----------------------------------------------------------------------------------------------

def est(x, pan=0.0):
    return D.paneo(x, pan) if x.ndim == 1 else x


def capas(*items):
    """Suma capas (señal, retardo en s, paneo) en un buffer estéreo."""
    n = max(D.ns(dt) + x.shape[-1] for x, dt, _ in items)
    out = np.zeros((2, n))
    for x, dt, p in items:
        D.sumar(out, est(x, p), dt)
    return out


def alargar(x, n):
    return np.concatenate([x, np.zeros((2, n - x.shape[1]))], axis=1) if x.shape[1] < n else x


def cue(t, nombre, x, nivel, bajo=None, envio=0.12, hueco=True):
    """Registra un efecto con `nivel` dB de pico ya procesado (0 dB = el aterrizaje); `bajo` es su capa sub."""
    x = est(x)
    cola = SALA.shape[1] if envio else D.ns(0.05)

    def marco(y):
        return np.concatenate([np.zeros((2, D.ns(PRE))), y, np.zeros((2, cola))], axis=1)
    y = D.filtro(marco(x), hp=100, orden=4, picos=[(3700, -3.0, 1.0)])
    if hueco:
        y = D.hueco_voz(y, PALABRAS, PROF_HUECO, t0=t - PRE, rampa=0.04)
    if envio:
        y = y + envio * D.reverb(y, SALA)[:, :y.shape[1]]
    if bajo is not None:
        lo = D.filtro(marco(alargar(est(bajo), x.shape[1])), hp=28, lp=320)
        n = max(y.shape[1], lo.shape[1])
        y = alargar(y, n) + alargar(lo, n)
    CUES.append((t, nombre, y * D.g(nivel) / np.abs(y).max()))


# --- materiales -----------------------------------------------------------------------------------------------

def whoosh(r, dur, fc, bw=1.0, p=0.5, a=2.0, b=1.5, pan=0.0, tono=None, aleteo=0.0):
    """Ruido con banda móvil y nivel en campana; tono = (Hz inicial, Hz final, nivel) añade un seno que se desliza."""
    n = D.ns(dur)
    e = D.campana_env(n, p, a, b)
    y = D.ruido_movil(r, dur, fc, bw) * e
    if tono:
        f = np.geomspace(tono[0], tono[1], n)
        y += tono[2] * 3 * np.sin(D.fase(f)) * e
    if aleteo:
        y *= 1 + aleteo * np.sin(2 * np.pi * 11 * D.tv(n)) * np.exp(-D.tv(n) / 0.3)
    return est(D.fades(y, 0.003, 0.01), pan)


def tic(r, fuerza=1.0, modo=6500, cuerpo=0.0):
    """Tic diminuto: ráfaga aguda y un modo; cuerpo añade un golpecito medio."""
    n = D.ns(0.05)
    y = D.filtro(r.standard_normal(n) * D.env(n, 0.0001, 0.0005), hp=4000, lp=12000)
    y += D.modal(n, [(modo * (1 + 0.03 * r.standard_normal()), 0.3, 0.003)])
    if cuerpo:
        y += cuerpo * D.modal(n, [(900, 0.5, 0.008), (420, 0.4, 0.012)])
    return D.fades(y * fuerza, 0.0001, 0.01)


def golpe_suave(r, f0=95, f1=55, caida=0.12, ruido=0.5):
    """Golpe blando: seno que cae (capa sub) y un ruido sordo de 150–700 Hz (capa media)."""
    n = D.ns(caida * 5)
    t = D.tv(n)
    sub = np.sin(D.fase(f1 + (f0 - f1) * np.exp(-t / 0.03))) * D.env(n, 0.002, caida)
    sordo = D.filtro(r.standard_normal(n) * D.env(n, 0.001, 0.02), hp=150, lp=700) * ruido
    return D.fades(sordo, 0.0005, 0.01), D.fades(sub, 0.001, 0.02)


def destellos(r, dur, cantidad, frecs, caidas=(0.02, 0.07), decae=0.6):
    """Chispitas: senos brevísimos en notas de re mayor muy agudas, repartidos en el tiempo y el estéreo."""
    n = D.ns(dur + 0.15)
    out = np.zeros((2, n))
    for tt in np.sort(r.uniform(0, dur, cantidad)):
        f = frecs[r.integers(len(frecs))] * (1 + 0.002 * r.standard_normal())
        m = D.ns(0.15)
        y = D.modal(m, [(f, 1.0, r.uniform(*caidas)), (f * 2.0, 0.15, 0.01)])
        a = (1 - decae * tt / dur) * r.uniform(0.5, 1.0)
        D.sumar(out, D.paneo(D.fades(y, 0.0015, 0.01) * a, r.uniform(-0.85, 0.85)), tt)
    return out


def aire(r, dur, p=0.5, hp=6000):
    """Soplo de aire muy agudo, en campana."""
    n = D.ns(dur)
    y = D.filtro(r.standard_normal(n), hp=hp, lp=13000) * D.campana_env(n, p, 2.0, 2.0)
    return D.fades(y, 0.005, 0.01)


def trazo(r, dur, f0, f1, tono=None, chispas=80):
    """Trazo de luz: ruido que sube de banda y crece, con chispitas y, si se pide, un tono fino que sube."""
    n = D.ns(dur)
    u = np.linspace(0, 1, n)
    e = u ** 1.3 * D.puerta(n, dur - 0.04, 0.04)
    y = D.ruido_movil(r, dur, [(0, f0), (1, f1)], 0.6) * e * 0.6
    if tono:
        y += 0.5 * np.sin(D.fase(np.geomspace(tono[0], tono[1], n))) * e
    imp = np.zeros(n)
    idx = r.integers(0, n, int(chispas * dur))
    imp[idx] = r.uniform(-1, 1, len(idx))
    y += D.filtro(imp, hp=5000, lp=11000) * 3 * e
    return D.fades(y, 0.004, 0.006)


def solapa(r, dur=0.03):
    """Solapa de un marcador mecánico (split-flap): dos o tres clacs diminutos muy seguidos."""
    n = D.ns(dur + 0.03)
    y = np.zeros(n)
    for k in range(r.integers(2, 4)):
        i = D.ns(k * 0.011 + r.uniform(0, 0.002))
        m = n - i
        c = D.modal(m, [(1650 * (1 + 0.05 * r.standard_normal()), 0.4, 0.006), (2700, 0.15, 0.004),
                        (6300 * (1 + 0.04 * r.standard_normal()), 0.3, 0.003), (320, 0.3, 0.008)])
        c += D.filtro(r.standard_normal(m) * D.env(m, 0.0001, 0.0004), hp=3000, lp=11000) * 0.6
        y[i:] += c * (1.0 if k == 0 else 0.55)
    return D.fades(y, 0.0001, 0.01)


def marcador(r, digitos, sep=0.019):
    """Un cambio del contador: una solapa por dígito, escalonadas y repartidas en estéreo."""
    return capas(*[(solapa(r), k * sep + r.uniform(0, 0.004), -0.5 + k / max(digitos - 1, 1)) for k in range(digitos)])


def zip_(r, dur, f0, f1, pan, tono=0.25):
    """Zip: barrido rápido de banda estrecha con un seno que lo sigue."""
    return whoosh(r, dur, [(0, f0), (1, f1)], 0.5, p=0.6, a=1.5, b=1.2, pan=pan, tono=(f0 * 0.5, f1 * 0.5, tono))


def toque_vidrio(r, grave=False):
    """Toque de dedo sobre vidrio: transitorio corto y modos de la placa."""
    n = D.ns(0.12)
    if grave:
        modos = [(900, 0.6, 0.02), (2400, 0.25, 0.012), (5200, 0.08, 0.004), (300, 0.45, 0.02)]
    else:
        modos = [(1900, 0.45, 0.012), (5800, 0.2, 0.004), (620, 0.4, 0.01)]
    y = D.modal(n, [(f * (1 + 0.02 * r.standard_normal()), a, d) for f, a, d in modos])
    y += D.filtro(r.standard_normal(n) * D.env(n, 0.0002, 0.001), hp=800, lp=6000) * 0.5
    return D.fades(y, 0.0002, 0.02)


# --- escenas --------------------------------------------------------------------------------------------------

def s1_el_clic():
    r = D.rng("s1")
    # 0,00–0,35: fibras de luz que pasan
    for k, (t, dur, f, pan) in enumerate(((0.04, 0.12, 2200, (-0.9, -0.3)), (0.08, 0.10, 6500, (0.8, 0.3)),
                                          (0.13, 0.13, 1600, (-0.5, -0.9)), (0.17, 0.10, 7200, (0.4, 0.9)),
                                          (0.22, 0.11, 2600, (-0.8, -0.2)), (0.26, 0.09, 6000, (0.9, 0.5)))):
        y = whoosh(r, dur, [(0, f * 0.6), (0.45, f), (1, f * 0.45)], 0.7, p=0.45, a=2.0, b=2.0, pan=pan)
        cue(t, f"fibra_luz_{k + 1}", y, -25 + 1.2 * k, envio=0.08)
    # 0,13–0,40: el cursor se precipita hacia la cámara
    dur = T.CLICK - 0.13
    y = whoosh(r, dur, [(0, 350), (1, 3200)], 0.9, p=0.97, a=3.0, b=1.0, tono=(220, 880, 0.12))
    cuerpo = whoosh(r, dur, [(0, 140), (1, 420)], 1.0, p=0.97, a=3.0, b=1.0)
    cue(0.13, "cursor_se_acerca", y + 0.6 * cuerpo, -13, envio=0.05)

    # 0,40: EL CLIC (héroe): tic + cuerpo + golpe sub + anillo de vidrio en re
    sub = np.sin(D.fase(42 + 13 * np.exp(-D.tv(D.ns(0.8)) / 0.06))) * D.env(D.ns(0.8), 0.003, 0.18)
    sub = D.fades(np.tanh(1.3 * sub) / np.tanh(1.3), 0.0, 0.1)
    anillo = capas((D.vidrio(D.hz("D6"), 0.45, brillo=0.5, caida=0.3), 0, 0),
                   (D.vidrio(D.hz("A6"), 0.12, brillo=0.3, caida=0.25), 0, 0))
    m = D.ns(0.2)
    golpe = D.filtro(r.standard_normal(m) * D.env(m, 0.001, 0.025), hp=150, lp=1800) * 0.6
    principal = capas((D.clic_satinado(r, cuerpo=1.0, tic=0.9), 0, 0.0), (anillo, 0.004, 0.1),
                      (D.fades(D.modal(D.ns(0.1), [(180, 0.35, 0.03)]) + golpe[:D.ns(0.1)], 0.0, 0.03), 0, 0.0))
    cue(T.CLICK, "CLIC_heroe", principal, -2.5, bajo=sub * 0.5, envio=0.2)

    # 0,42–0,90: fragmentos de chispa; 0,80: el cursor parpadea
    cue(0.42, "chispas", destellos(r, 0.46, 14, [D.hz("F#8"), D.hz("A8"), D.hz("D9"), D.hz("E8")]), -27,
        envio=0.25)
    cue(0.80, "tic_cursor", tic(r, 1.0, 6200, cuerpo=0.4), -27, envio=0.1)

    # 0,95–1,40: la línea dibuja el logo; 1,25 saltito; 1,35 «pop» al llenarse
    cue(T.LOGO_DRAW[0], "trazo_logo", trazo(r, T.LOGO_DRAW[1] - T.LOGO_DRAW[0], 400, 6000,
                                            tono=(D.hz("D4"), D.hz("A5"))), -20, envio=0.15)
    cue(1.25, "salto_whoosh", whoosh(r, 0.16, [(0, 900), (0.5, 2000), (1, 1100)], 1.0, pan=(-0.2, 0.2)), -22)
    n = D.ns(0.15)
    tt = D.tv(n)
    pop = np.sin(D.fase(D.hz("D5") * (1 - 0.5 * np.exp(-tt / 0.006)))) * D.env(n, 0.001, 0.035)
    pop += 0.4 * np.sin(2 * np.pi * 150 * tt) * D.env(n, 0.001, 0.02)
    cue(1.35, "pop_logo", D.fades(pop, 0.0005, 0.01), -17, envio=0.15)


def s2_universo():
    r = D.rng("s2")
    # 1,45–2,50: ~20 pantallas que se encienden una a una (tono que sube por la pentatónica de re)
    escala = ["D4", "E4", "F#4", "A4", "B4", "D5", "E5", "F#5", "A5"]
    for i in range(26):
        t = 1.42 + 1.08 * (0.10 + 0.86 * i / 25)
        f = D.hz(escala[min(i * len(escala) // 26, len(escala) - 1)])
        y = D.blip(f, 0.08, desde=0.9, caida=0.025, armonico=0.15)
        chispa = tic(r, 0.08, 7000)
        y[:len(chispa)] += chispa
        pan = float(np.clip(0.65 * np.sin(2.3 * i + 0.5), -0.7, 0.7))
        cue(t, f"pantalla_enciende_{i + 1:02d}", est(y, pan), -24 + 5 * i / 25, envio=0.12)

    # 2,50: pulsación: golpe grave satinado + tecla + whoosh de anillo que se abre
    medio, sub = golpe_suave(r, 120, 68, 0.09, ruido=0.8)
    tecla = D.clic_satinado(r, cuerpo=0.6, tic=0.7, tono=1.3)
    anillo_i = whoosh(r, 0.45, [(0, 2600), (1, 700)], 0.8, p=0.12, a=1.5, b=2.0, pan=(0.0, -0.8))
    anillo_d = whoosh(r, 0.45, [(0, 2400), (1, 650)], 0.8, p=0.12, a=1.5, b=2.0, pan=(0.0, 0.8))
    tono = D.vidrio(D.hz("D5"), 0.12, brillo=0.3, caida=0.4)
    cue(T.PRESS, "pulsacion_boton", capas((medio, 0, 0), (tecla, 0, 0), (anillo_i, 0.01, 0), (anillo_d, 0.01, 0),
                                          (tono, 0.0, 0)), -8, bajo=sub, envio=0.18)

    # 2,58–2,90: la ventana se despliega y se posa
    w = whoosh(r, 0.32, [(0, 500), (0.7, 2000), (1, 1200)], 1.1, p=0.7)
    medio, sub = golpe_suave(r, 90, 60, 0.07, ruido=0.4)
    cue(2.58, "ventana_despliega", capas((w, 0, 0), (medio, 0.30, 0), (tic(r, 0.4, 5800), 0.30, 0)), -15,
        bajo=capas((sub, 0.30, 0)), envio=0.12)


def s3_monitores_iguales():
    r = D.rng("s3")
    for k, pan in enumerate((-0.5, 0.0, 0.5)):
        cue(2.92 + 0.04 * k, f"tarjeta_sube_{k + 1}",
            whoosh(r, 0.22, [(0, 600), (0.6, 1500), (1, 900)], 0.9, p=0.6, pan=pan), -23)

    def clic_chico():
        return D.clic_satinado(r, cuerpo=0.7, tic=0.5, tono=1.2)
    cue(3.25, "agarra_deslizador", clic_chico(), -20, envio=0.08)
    # 3,34–3,98: deslizamiento vidrioso que sube de tono
    dur = 3.98 - 3.34
    n = D.ns(dur)
    tt = D.tv(n)
    f = D.hz("A4") * 2 ** (np.linspace(0, 1, n) ** 1.3) * (1 + 0.003 * np.sin(2 * np.pi * 5 * tt))
    ph = D.fase(f)
    e = D.env(n, 0.03, 10) * D.puerta(n, dur - 0.06, 0.06)
    y = (np.sin(ph) + 0.25 * np.sin(2 * ph) + 0.1 * np.sin(2.76 * ph)) * e
    roce = D.ruido_movil(r, dur, [(0, 900), (1, 1800)], 0.4) * e * 0.08
    cue(3.34, "deslizador_vidrio", D.paneo(D.fades(y + roce, 0.005, 0.01), (-0.2, 0.3)), -28, envio=0.15)
    cue(3.98, "suelta_deslizador", clic_chico(), -21, envio=0.08)
    # 4,20: brillo suave cuando las tres pantallas coinciden
    n = D.ns(0.7)
    tt = D.tv(n)
    acorde = sum(np.sin(2 * np.pi * D.hz(nn) * tt + k) for k, nn in enumerate(("D6", "A6", "D7")))
    acorde *= D.env(n, 0.06, 0.18) * (1 + 0.3 * np.sin(2 * np.pi * 7 * tt))
    brillo = capas((D.fades(acorde, 0.002, 0.02) * 0.3, 0, 0),
                   (destellos(r, 0.3, 8, [D.hz("F#8"), D.hz("A8"), D.hz("D9")]), 0.02, 0),
                   (aire(r, 0.5, 0.3) * 0.3, 0, 0))
    cue(T.MATCH, "brillo_coinciden", brillo, -23, envio=0.25)

    # 4,60: CONFIRMACIÓN (marca 1): re → la en mazos de vidrio, brillante y corto
    re_ = D.vidrio(D.hz("D5"), 0.9, brillo=0.9, caida=0.45)
    la = D.vidrio(D.hz("A5"), 1.0, brillo=1.0, caida=0.55)
    chispa = D.campanita(D.hz("A6"), 0.12, caida=0.4)
    cue(T.CONFIRM1, "CONFIRMACION_marca", capas((re_, 0, -0.15), (la, 0.09, 0.15), (chispa, 0.09, 0.2)), -12.0,
        envio=0.2)


def s4_disposiciones():
    r = D.rng("s4")
    cue(4.62, "whoosh_linea_luz", whoosh(r, 0.36, [(0, 700), (0.5, 2600), (1, 1500)], 0.8, p=0.5,
                                         pan=(-0.7, 0.7)), -16)
    # 4,98–5,22: tics de la línea de luz que se aceleran hacia el encendido
    tics = []
    t = 0.0
    for k in range(9):
        tics.append((tic(r, 0.4 + 0.07 * k, 6500), t, 0.4 * np.sin(k)))
        t += 0.042 - 0.0035 * k
    cue(4.98, "tics_linea_luz", capas(*tics), -23, envio=0.1)

    # 5,20: ENCENDIDO: crecida de encendido + barrido anamórfico
    n = D.ns(0.9)
    tt = D.tv(n)
    sube = 0.95 + 0.05 * (1 - np.exp(-tt / 0.08))
    e = (1 - np.exp(-tt / 0.06)) * np.exp(-tt / 0.35)
    tono = sum(a * np.sin(D.fase(D.hz(nn) * sube)) for nn, a in (("D3", 0.6), ("A3", 0.35), ("D4", 0.25)))
    ruido = D.ruido_movil(r, 0.9, [(0, 300), (0.3, 2500), (1, 900)], 0.9) * 0.15
    grave = np.sin(D.fase(D.hz("D2") * sube)) * e
    swish = whoosh(r, 0.24, [(0, 8000), (1, 3500)], 0.6, p=0.25, a=1.5, b=1.5, pan=(-0.9, 0.9))
    cue(T.IGNITION, "ENCENDIDO", capas((D.fades((tono + ruido) * e, 0.002, 0.05), 0, 0), (swish * 1.5, 0, 0)), -9,
        bajo=D.fades(grave * 0.7, 0.002, 0.05), envio=0.2)

    # 5,20–5,50: tres monitores pesados suben y se asientan
    for k, pan in enumerate((-0.6, 0.0, 0.6)):
        sube_w = whoosh(r, 0.18, [(0, 300), (1, 900)], 1.0, p=0.8, pan=pan)
        medio, sub = golpe_suave(r, 70, 48, 0.12, ruido=0.9)
        traqueteo = tic(r, 0.3, 1800, cuerpo=0.5)
        cue(5.20 + 0.04 * k, f"monitor_pesado_{k + 1}",
            capas((sube_w * 0.5, 0, 0), (medio, 0.18, pan), (traqueteo, 0.19, pan)), -13,
            bajo=capas((sub, 0.18, pan)), envio=0.12)

    # contador split-flap
    for t, dig in ((5.52, 4), (5.82, 3), (5.98, 4)):
        cue(t, f"contador_{t:.2f}", marcador(r, dig), -21, envio=0.06)

    # 5,62: un monitor gira a vertical: zumbido mecánico corto + golpe seco
    dur = 0.16
    n = D.ns(dur)
    tt = D.tv(n)
    f = np.linspace(140, 210, n)
    ph = D.fase(f)
    motor = sum(np.sin(k * ph) / k for k in range(1, 11)) * (1 + 0.3 * np.sin(2 * np.pi * 37 * tt))
    motor = D.filtro(motor, lp=2200) + D.ruido_movil(r, dur, [(0, 800), (1, 1200)], 0.5) * 0.15
    motor *= D.env(n, 0.02, 10) * D.puerta(n, dur - 0.03, 0.03)
    medio, sub = golpe_suave(r, 110, 70, 0.06, ruido=0.7)
    golpe = medio + D.modal(len(medio), [(380, 0.4, 0.03)])
    cue(5.62, "monitor_gira", capas((D.fades(motor, 0.005, 0.01) * 0.5, 0, 0.3), (golpe, dur, 0.3),
                                    (tic(r, 0.6, 2600), dur, 0.3)), -16, bajo=capas((sub, dur, 0.3)), envio=0.1)

    # 5,96–6,84: se encienden los chips de disposición (clic + notita) y los monitores se reacomodan
    for k, (t, nota) in enumerate(zip((5.96, 6.18, 6.40, 6.62, 6.84), ("D5", "E5", "F#5", "A5", "B5"))):
        clic = D.clic_satinado(r, cuerpo=0.5, tic=0.6, tono=1.5)
        nota_ = D.blip(D.hz(nota), 0.12, desde=0.97, caida=0.05, armonico=0.1) * 0.25
        desliza = whoosh(r, 0.22, [(0, 300), (0.5, 800), (1, 500)], 1.0, p=0.5, pan=-0.4 + 0.2 * k)
        cue(t, f"chip_disposicion_{k + 1}", capas((clic, 0, -0.3 + 0.15 * k), (nota_, 0.002, -0.3 + 0.15 * k),
                                                  (desliza * 0.35, 0.02, 0)), -19, envio=0.12)


def s5_otro_equipo():
    r = D.rng("s5")
    cue(6.90, "camara_sube", whoosh(r, 0.5, [(0, 500), (0.6, 2200), (1, 3000)], 1.6, p=0.6), -18, envio=0.1)
    for k, pan in enumerate((-0.45, -0.15, 0.15, 0.45)):
        cue(6.92 + 0.07 * k, f"lista_tarjeta_{k + 1}",
            whoosh(r, 0.17, [(0, 650), (0.6, 1600), (1, 1000)], 0.9, p=0.6, pan=pan), -25)
    cue(7.70, "tarjeta_al_frente", whoosh(r, 0.35, [(0, 600), (1, 2400)], 1.0, p=0.85, a=2.5, b=1.0), -17)
    cue(7.72, "tarjetas_atras_izq", whoosh(r, 0.3, [(0, 1800), (1, 500)], 1.0, p=0.3, pan=(-0.3, -0.8)), -23)
    cue(7.76, "tarjetas_atras_der", whoosh(r, 0.3, [(0, 1700), (1, 480)], 1.0, p=0.3, pan=(0.3, 0.8)), -23)
    tecla = D.clic_satinado(r, cuerpo=1.0, tic=0.8, tono=1.1)
    cue(T.CONNECT_CLICK, "clic_conectar", capas((tecla, 0, 0.1), (tic(r, 0.3, 7000), 0.004, 0.1)), -13, envio=0.12)
    cue(8.49, "digitos_giran", capas(*[(solapa(r), 0.035 * k, -0.3 + 0.12 * k) for k in range(6)]), -20, envio=0.06)

    # 9,40: CONECTADO (marca 2): re → la → re', abierto y cálido, con una ola de brillo
    notas = [(D.vidrio(D.hz("D5"), 0.85, brillo=0.7, caida=0.55), 0.0, -0.25),
             (D.vidrio(D.hz("A5"), 0.9, brillo=0.7, caida=0.55), 0.08, 0.2),
             (D.vidrio(D.hz("D6"), 1.0, brillo=0.75, caida=0.6), 0.16, 0.0),
             (D.marimba(D.hz("D4"), 0.45), 0.0, 0.0),
             (D.campanita(D.hz("D6"), 0.18, caida=0.8), 0.16, 0.1)]
    ola = destellos(r, 0.6, 18, [D.hz("D8"), D.hz("F#8"), D.hz("A8"), D.hz("D9")], decae=0.2)
    ola *= np.sin(np.pi * np.clip(D.tv(ola.shape[1]) / 0.75, 0, 1))
    cue(T.CONFIRM2, "CONECTADO_marca", capas(*notas, (ola * 0.18, 0.2, 0), (aire(r, 0.7, 0.4) * 0.12, 0.15, 0)), -12.0,
        envio=0.18)

    # 9,48–9,90: las tarjetas salen volando a izquierda y derecha
    cue(9.48, "tarjeta_vuela_izq", whoosh(r, 0.38, [(0, 2000), (1, 600)], 0.9, p=0.25, pan=(0.0, -0.9)), -17)
    cue(9.52, "tarjeta_vuela_der", whoosh(r, 0.38, [(0, 1900), (1, 560)], 0.9, p=0.25, pan=(0.0, 0.9)), -18)


def s6_otra_pantalla():
    r = D.rng("s6")
    # 9,55–9,90: dos monitores se encienden
    for t, nota, pan in ((9.60, "A4", -0.4), (9.82, "D5", 0.4)):
        y = capas((D.blip(D.hz(nota), 0.16, desde=0.85, caida=0.06, armonico=0.25), 0.006, pan),
                  (tic(r, 0.5, 3200, cuerpo=0.4), 0, pan))
        cue(t, f"monitor_enciende_{t:.2f}", y, -20, envio=0.15)
    # 9,70–10,08: barrido de luz sobre la pantalla
    barrido = whoosh(r, 0.38, [(0, 5000), (1, 9000)], 0.5, p=0.5, pan=(-0.6, 0.6))
    cue(T.MAC_APPEARS, "barrido_luz", capas((barrido, 0, 0), (destellos(r, 0.34, 10, [D.hz("A8"), D.hz("D9")]) * 0.5,
                                                              0.02, 0)), -22, envio=0.2)
    # 10,24: el cursor cruza de una pantalla a otra (zip izquierda → derecha)
    cue(10.24, "cursor_cruza_zip", zip_(r, 0.13, 2000, 8000, (-0.9, 0.9), tono=0.1), -21, envio=0.1)
    # 10,42–10,66: pantalla virtual violeta: trazo y luz que la llena
    cue(10.42, "pantalla_violeta_trazo", est(trazo(r, 0.24, 700, 5000), (-0.3, 0.3)), -23, envio=0.15)
    n = D.ns(0.6)
    tt = D.tv(n)
    llena = sum(a * np.sin(2 * np.pi * D.hz(nn) * tt + k) for k, (nn, a) in enumerate((("D4", 1.0), ("A4", 0.6))))
    llena = llena * D.env(n, 0.1, 0.2) * 0.5 + aire(r, 0.6, 0.25, hp=5500)
    chispas = destellos(r, 0.4, 8, [D.hz("A8"), D.hz("D9")], decae=0.3) * 0.4
    cue(10.62, "pantalla_violeta_llena", capas((D.fades(llena, 0.002, 0.02), 0, 0), (chispas, 0.05, 0)), -25,
        envio=0.3)
    # 10,86: el cursor se menea de alegría: tres notitas diminutas
    y = capas(*[(D.blip(D.hz(nn), 0.07, desde=0.92, caida=0.02, armonico=0.1), 0.05 * k, -0.2 + 0.2 * k)
                for k, nn in enumerate(("A5", "D6", "F#6"))])
    cue(10.86, "cursor_contento", y, -25, envio=0.15)


def s7_cuatro_mundos():
    r = D.rng("s7")
    grua = whoosh(r, 0.5, [(0, 150), (0.5, 600), (1, 300)], 1.0, p=0.5, pan=(-0.2, 0.3))
    grua += est(aire(r, 0.5, 0.5, hp=5500) * 0.5, (-0.4, 0.5))     # cuerpo grave y velocidad en el aire, no en la voz
    cue(10.95, "grua_whoosh", grua, -13, envio=0.12)
    for k, pan in enumerate(((0, -0.8), (0, -0.3), (0, 0.3), (0, 0.8))):
        cue(11.05 + 0.07 * k, f"rama_luz_{k + 1}", zip_(r, 0.14, 2500, 9000, pan, tono=0.1), -25, envio=0.1)
    # cuatro estaciones aterrizan: golpe blando + nota de re mayor
    for nombre, nota, pan in (("windows", "D5", -0.5), ("mac", "F#5", 0.5), ("iphone", "A5", -0.25),
                              ("android", "D6", 0.25)):
        medio, sub = golpe_suave(r, 85, 55, 0.14, ruido=0.9)
        ping = D.vidrio(D.hz(nota), 0.4, brillo=0.35, caida=0.6)     # suave: cae sobre el nombre de la plataforma
        cue(T.PLATFORMS[nombre], f"estacion_{nombre}", capas((medio, 0, pan), (ping, 0.004, pan)), -11,
            bajo=est(sub, pan), envio=0.18)
    # 12,25: un monitor entra deslizándose, mecánico
    dur = 0.3
    n = D.ns(dur)
    tt = D.tv(n)
    rodar = D.ruido_movil(r, dur, [(0, 300), (1, 700)], 0.9) * (1 + 0.5 * np.sin(2 * np.pi * 45 * tt))
    rodar *= D.env(n, 0.04, 10) * D.puerta(n, dur - 0.04, 0.04) * 0.4
    medio, sub = golpe_suave(r, 100, 62, 0.08, ruido=0.7)
    cue(12.25, "monitor_entra", capas((est(D.fades(rodar, 0.005, 0.01), (0.8, 0.3)), 0, 0), (medio, dur - 0.02, 0.3),
                                      (tic(r, 0.5, 2400, cuerpo=0.6), dur - 0.02, 0.3)), -15,
        bajo=capas((sub, dur - 0.02, 0.3)), envio=0.1)
    for t in (12.90, 13.20, 13.50):
        cue(t, f"toque_trackpad_{t:.2f}", est(toque_vidrio(r), 0.15), -23, envio=0.1)


def s8_gran_idea():
    r = D.rng("s8")
    cue(14.05, "lineas_se_extienden", capas((est(trazo(r, 0.3, 800, 4500), (-0.2, -0.8)), 0, 0),
                                            (est(trazo(r, 0.3, 850, 4700), (0.2, 0.8)), 0.01, 0)), -23, envio=0.12)
    cue(14.08, "toque_pantalla_grande", est(toque_vidrio(r, grave=True), -0.1), -17, envio=0.15)
    for k, (t, pan) in enumerate(zip((14.32, 14.50, 14.66, 14.82), ((-0.9, 0), (0.9, 0), (-0.6, 0), (0.6, 0)))):
        cue(t, f"lineas_convergen_{k + 1}", zip_(r, 0.15, 6000, 1500, pan, tono=0.15), -23, envio=0.1)
    cue(T.TEXT1, "texto_sube_1", whoosh(r, 0.4, [(0, 350), (1, 1600)], 1.0, p=0.6), -20)
    brillo = capas((destellos(r, 0.36, 10, [D.hz("F#8"), D.hz("A8"), D.hz("D9")]), 0, 0), (aire(r, 0.36) * 0.4, 0, 0))
    cue(15.62, "brillo_corto", brillo, -25, envio=0.2)
    cue(T.TEXT2, "texto_sube_2", whoosh(r, 0.4, [(0, 380), (1, 1700)], 1.0, p=0.6), -20)

    # 16,60: todo se apaga (pitidos que bajan) y el texto se hunde
    pitidos = [(D.blip(D.hz(nn), 0.1, desde=1.06, caida=0.04, armonico=0.15), 0.06 * k, 0.3 - 0.2 * k)
               for k, nn in enumerate(("D6", "A5", "F#5", "D5"))]
    n = D.ns(0.4)
    baja = np.sin(D.fase(np.geomspace(440, 110, n))) * D.env(n, 0.005, 0.12)
    hunde = whoosh(r, 0.35, [(0, 1400), (1, 350)], 1.0, p=0.3)
    cue(T.PAUSE, "todo_se_apaga", capas(*pitidos, (D.fades(baja, 0.002, 0.02) * 0.4, 0, 0), (hunde * 0.6, 0.02, 0)),
        -15, envio=0.15)


def s9_cierre():
    r = D.rng("s9")
    # 16,62–17,20: swell inverso hacia el aterrizaje
    largo = T.LANDING - 16.62
    n = D.ns(1.2)
    golpe = D.filtro(r.standard_normal(n) * D.env(n, 0.001, 0.12), hp=400, lp=6000)
    sala = D.sala(2.5, 2.2, 1.0, "sala_swell", pre=0.0)
    w = D.reverb(golpe, sala)
    w[:, :n] += 0.3 * golpe
    w = w[:, ::-1][:, -D.ns(largo):]
    u = np.linspace(0, 1, w.shape[1])
    sube = D.ruido_movil(r, largo, [(0, 300), (1, 3000)], 1.0) * u ** 3 * 0.15
    swell = D.fades(w * u ** 1.5 + sube, 0.01, 0.004)
    cue(16.62, "swell_inverso", swell, -10, envio=0.0)

    # 17,20: ATERRIZAJE (héroe): golpe grave profundo + cuerpo + cola de alas
    n = D.ns(1.4)
    tt = D.tv(n)
    ph = D.fase(38 + 37 * np.exp(-tt / 0.05))
    grave = np.sin(ph) * D.env(n, 0.002, 0.45) + 0.3 * np.sin(2 * ph) * D.env(n, 0.002, 0.25)
    grave = D.fades(np.tanh(1.6 * grave) / np.tanh(1.6), 0.0, 0.3)
    m = D.ns(0.3)
    punch = D.fades(np.sin(D.fase(90 + 70 * np.exp(-D.tv(m) / 0.02))) * D.env(m, 0.001, 0.06), 0.0, 0.05)
    sordo = D.filtro(r.standard_normal(m) * D.env(m, 0.001, 0.04), hp=60, lp=700)
    clic = D.clic_satinado(r, cuerpo=0.7, tic=0.6)
    ala_i = whoosh(r, 0.75, [(0, 2200), (0.3, 1500), (1, 500)], 1.1, p=0.18, a=1.5, b=2.0, pan=(0.0, -0.85),
                   aleteo=0.25)
    ala_d = whoosh(r, 0.75, [(0, 2100), (0.3, 1400), (1, 480)], 1.1, p=0.18, a=1.5, b=2.0, pan=(0.0, 0.85),
                   aleteo=0.25)
    cuerpo = capas((punch + sordo, 0, 0), (clic * 0.6, 0, 0), (ala_i * 0.6, 0.02, 0), (ala_d * 0.6, 0.03, 0))
    cue(T.LANDING, "ATERRIZAJE_heroe", cuerpo, 0.0, bajo=grave * 0.9, envio=0.25)

    # 17,24–17,75: la baldosa sube (golpe) y la luz dibuja el logo
    medio, sub = golpe_suave(r, 90, 58, 0.1, ruido=0.7)
    cue(17.24, "baldosa_sube", medio, -16, bajo=sub * 0.5, envio=0.1)
    cue(T.LOGO2[0], "trazo_logo_final", trazo(r, T.LOGO2[1] - T.LOGO2[0], 600, 5000, tono=(D.hz("D5"), D.hz("D7"))),
        -22, envio=0.15)
    cue(18.05, "salto_final", whoosh(r, 0.15, [(0, 900), (0.5, 2000), (1, 1100)], 1.0, pan=(0.2, -0.1)), -21)
    # 18,40: clic satinado que cae con la firma musical
    anillo = D.vidrio(D.hz("D6"), 0.12, brillo=0.4, caida=0.3)
    cue(T.FANFARE, "clic_satinado_firma", capas((D.clic_satinado(r, cuerpo=0.9, tic=0.8), 0, 0), (anillo, 0.003, 0.1)),
        -12, envio=0.18)


# --- mezcla, archivo y lista ----------------------------------------------------------------------------------

def construir():
    s1_el_clic()
    s2_universo()
    s3_monitores_iguales()
    s4_disposiciones()
    s5_otro_equipo()
    s6_otra_pantalla()
    s7_cuatro_mundos()
    s8_gran_idea()
    s9_cierre()
    mix = np.zeros((2, N))
    for t, _, y in CUES:
        D.sumar(mix, y, t - PRE)
    k = D.g(-3.8)                       # 0 dB de diseño = −3,8 dBFS
    pk = np.abs(mix).max() * k
    if pk > D.g(-3.5):                  # donde se solapan, nunca por encima de −3,5 dBFS
        k *= D.g(-3.5) / pk
    return D.fades(mix * k, 0.002, 0.05), k


if __name__ == "__main__":
    mix, k = construir()
    D.escribir_wav(SALIDA, mix)
    filas = sorted((t, nom, D.dbfs(np.abs(y).max() * k)) for t, nom, y in CUES)
    with open(LISTA, "w", encoding="utf-8") as f:
        f.write("# Kast: efectos (sfx.wav, 48 kHz, 24 bits). tiempo (s) · nombre · pico del cue en la pista (dBFS)\n")
        for t, nom, p in filas:
            f.write(f"{t:7.3f}  {nom:28s} {p:6.1f}\n")
    print(f"{len(filas)} cues → {LISTA}")
    print("los más fuertes: " + " · ".join(f"{nom} {p:.1f}" for t, nom, p in sorted(filas, key=lambda x: -x[2])[:6]))
    D.revisar(SALIDA, VOZ)
