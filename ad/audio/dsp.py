"""Herramientas de síntesis compartidas por music.py y sfx.py (sólo NumPy): envolventes, filtros evaluados en la
FFT, ruido con filtro móvil (STFT), reverberación por convolución, instrumentos comunes, WAV de 24 bits y la
revisión de los archivos sin escucharlos.

  python ad/audio/dsp.py <archivo.wav> ...     revisa archivos ya escritos
"""
import os
import sys
import wave
import zlib

import numpy as np

SR = 48000

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")    # la consola de Windows no es UTF-8 por defecto


# --- unidades -------------------------------------------------------------------------------------------------

def ns(seg):
    """Segundos → muestras."""
    return int(round(seg * SR))


def tv(n):
    return np.arange(n) / SR


def g(db):
    return 10 ** (db / 20)


def dbfs(x):
    return 20 * np.log10(max(float(x), 1e-12))


NOTAS = {"C": 0, "C#": 1, "D": 2, "D#": 3, "E": 4, "F": 5, "F#": 6, "G": 7, "G#": 8, "A": 9, "A#": 10, "B": 11}


def hz(nota):
    """'F#4' → Hz (La4 = 440)."""
    m = 12 * (int(nota[-1]) + 1) + NOTAS[nota[:-1]]
    return 440.0 * 2 ** ((m - 69) / 12)


def rng(nombre):
    """Generador con semilla fija sacada del nombre (hash() de Python cambia en cada ejecución; crc32 no)."""
    return np.random.default_rng(zlib.crc32(nombre.encode()))


# --- envolventes ----------------------------------------------------------------------------------------------

def env(n, a=0.002, d=0.2, hold=0.0):
    """Ataque coseno de a s, meseta de hold s y caída exponencial con constante d."""
    t = tv(n)
    e = np.where(t < a, 0.5 - 0.5 * np.cos(np.pi * t / max(a, 1e-9)), 1.0)
    return e * np.exp(-np.maximum(t - a - hold, 0) / d)


def puerta(n, dur, rel):
    """1 hasta dur y caída coseno de rel s."""
    u = np.clip((tv(n) - dur) / rel, 0, 1)
    return 0.5 + 0.5 * np.cos(np.pi * u)


def campana_env(n, p=0.5, a=2.0, b=1.5):
    """Curva 0 → 1 → 0 con el máximo en la fracción p (para los whooshes)."""
    u = np.linspace(0, 1, n)
    sube = np.sin(0.5 * np.pi * np.clip(u / p, 0, 1)) ** a
    baja = np.clip((1 - u) / (1 - p), 0, 1) ** b
    return np.where(u < p, sube, baja)


def fades(x, a=0.002, b=0.005):
    """Rampas cortas en los bordes para no dejar clics."""
    x = np.array(x, dtype=float)
    n = x.shape[-1]
    na, nb = min(ns(a), n // 2), min(ns(b), n // 2)
    if na:
        x[..., :na] *= 0.5 - 0.5 * np.cos(np.pi * np.arange(na) / na)
    if nb:
        x[..., n - nb:] *= 0.5 + 0.5 * np.cos(np.pi * (np.arange(nb) + 1) / nb)
    return x


def pico1(x):
    """Normaliza a pico 1."""
    m = np.abs(x).max()
    return x / m if m > 0 else x


# --- osciladores ----------------------------------------------------------------------------------------------

def fase(f):
    """Fase acumulada de una frecuencia que cambia (array en Hz)."""
    return 2 * np.pi * np.cumsum(f) / SR


def modal(n, parciales):
    """Síntesis modal: suma de senos que decaen, [(Hz, amplitud, constante de caída en s)]."""
    t = tv(n)
    y = np.zeros(n)
    for f, a, d in parciales:
        if 20 < f < SR * 0.45:
            y += a * np.exp(-t / d) * np.sin(2 * np.pi * f * t)
    return y


# --- filtros --------------------------------------------------------------------------------------------------
# Prototipos analógicos (Butterworth, campana de RBJ) evaluados en la rejilla de la FFT: fase causal y sin bucles.

_QS = {1: [], 2: [0.7071], 4: [0.5412, 1.3066]}


def _resp(f, hp, lp, orden, picos):
    H = np.ones(len(f), complex)
    for fc, cual in ((hp, "hp"), (lp, "lp")):
        if not fc:
            continue
        s = 1j * f / fc
        if orden == 1:
            H *= s / (s + 1) if cual == "hp" else 1 / (s + 1)
        for q in _QS.get(orden, []):
            H *= (s * s if cual == "hp" else 1) / (s * s + s / q + 1)
    for fc, gdb, q in picos:
        A = 10 ** (gdb / 40)
        s = 1j * f / fc
        H *= (s * s + s * A / q + 1) / (s * s + s / (A * q) + 1)
    return H


def filtro(x, hp=None, lp=None, orden=2, picos=(), cola=0.25):
    """Paso alto / paso bajo Butterworth (orden 1, 2 o 4) y campanas [(Hz, dB, Q)]. x (n,) o (canales, n)."""
    x = np.asarray(x, float)
    n = x.shape[-1]
    N = 1 << int(np.ceil(np.log2(n + ns(cola))))
    H = _resp(np.fft.rfftfreq(N, 1 / SR), hp, lp, orden, picos)
    return np.fft.irfft(np.fft.rfft(x, N, axis=-1) * H, N, axis=-1)[..., :n]


def conv(x, h):
    """Convolución por FFT de cada canal de x con su canal de h; devuelve la longitud completa."""
    x, h = np.atleast_2d(x), np.atleast_2d(h)
    n = x.shape[-1] + h.shape[-1] - 1
    N = 1 << int(np.ceil(np.log2(n)))
    return np.fft.irfft(np.fft.rfft(x, N, axis=-1) * np.fft.rfft(h, N, axis=-1), N, axis=-1)[..., :n]


# --- STFT: filtros que cambian con el tiempo ------------------------------------------------------------------

def _ventana(nfft):
    return np.hanning(nfft + 1)[:-1]


def stft(x, nfft=1024, hop=256):
    xp = np.concatenate([np.zeros(nfft), x, np.zeros(nfft + hop)])
    fr = np.lib.stride_tricks.sliding_window_view(xp, nfft)[::hop]
    return np.fft.rfft(fr * _ventana(nfft), axis=1)


def istft(X, n, nfft=1024, hop=256):
    w = _ventana(nfft)
    fr = np.fft.irfft(X, nfft, axis=1) * w
    F = len(fr)
    y = np.zeros(F * hop + nfft)
    nrm = np.zeros(F * hop + nfft)
    for j in range(nfft // hop):
        y[j * hop:j * hop + F * hop] += fr[:, j * hop:(j + 1) * hop].reshape(-1)
        nrm[j * hop:j * hop + F * hop] += np.tile(w[j * hop:(j + 1) * hop] ** 2, F)
    return (y / np.maximum(nrm, 1e-9))[nfft:nfft + n]


def espectral(x, mascara, nfft=1024, hop=256, t0=0.0):
    """Aplica una ganancia que cambia en el tiempo: mascara(t (F,1) en s, f (1,B) en Hz) → (F,B)."""
    if x.ndim == 2:
        return np.stack([espectral(c, mascara, nfft, hop, t0) for c in x])
    X = stft(x, nfft, hop)
    t = t0 + (np.arange(len(X)) * hop - nfft / 2) / SR
    f = np.fft.rfftfreq(nfft, 1 / SR)
    return istft(X * mascara(t[:, None], f[None, :]), len(x), nfft, hop)


def banda(f, fc, bw):
    """Campana gaussiana en octavas centrada en fc."""
    return np.exp(-0.5 * (np.log2(np.maximum(f, 1.0) / fc) / bw) ** 2)


def nivelar(y, win=0.02):
    """Divide por el RMS móvil: deja el ruido filtrado a nivel 1 para que la envolvente mande."""
    k = max(1, ns(win))
    c = np.concatenate([[0.0], np.cumsum(y * y)])
    ms = (c[k:] - c[:-k]) / k
    izq = k // 2
    ms = np.concatenate([np.full(izq, ms[0]), ms, np.full(len(y) - len(ms) - izq, ms[-1])])
    return y / np.sqrt(ms + 1e-12)


def ruido_movil(r, dur, fc, bw=1.0, nfft=512):
    """Ruido con un paso banda que se mueve: fc = [(fracción, Hz), ...] interpolado en escala logarítmica."""
    n = ns(dur)
    x = r.standard_normal(n + nfft)
    us, fs = zip(*fc)
    bws = bw if isinstance(bw, (list, tuple)) else [(0, bw), (1, bw)]
    ub, bb = zip(*bws)

    def m(t, f):
        u = np.clip(t / dur, 0, 1)
        c = np.exp(np.interp(u, us, np.log(fs)))
        return banda(f, c, np.interp(u, ub, bb))
    return nivelar(espectral(x, m, nfft, nfft // 4)[:n], 0.012)


# --- estéreo y colocación -------------------------------------------------------------------------------------

def paneo(x, p=0.0):
    """Mono → estéreo de potencia constante; p en [-1, 1]: número, (inicio, fin) o [(fracción, p), ...]."""
    n = len(x)
    if np.isscalar(p):
        pp = np.full(n, float(p))
    elif len(p) == 2 and np.isscalar(p[0]):
        pp = np.linspace(p[0], p[1], n)
    else:
        u, v = zip(*p)
        pp = np.interp(np.linspace(0, 1, n), u, v)
    th = (np.clip(pp, -1, 1) + 1) * np.pi / 4
    return np.stack([x * np.cos(th), x * np.sin(th)])


def sumar(dest, x, t, gan=1.0):
    """Suma x (canales, n) en dest a partir del segundo t, recortando lo que se salga."""
    i = ns(t)
    N = dest.shape[-1]
    if x.ndim == 1:
        x = paneo(x)
    a, b = max(i, 0), min(N, i + x.shape[-1])
    if b > a:
        dest[:, a:b] += gan * x[:, a - i:b - i]


# --- reverberación y eco --------------------------------------------------------------------------------------

def sala(dur, t60_grave, t60_agudo, nombre, pre=0.012, hp=160, lp=9500):
    """Respuesta al impulso estéreo: ruido que decae más rápido en los agudos (dos canales sin correlación)."""
    r = rng(nombre)
    n = ns(dur)

    def m(t, f):
        u = np.clip(np.log2(np.maximum(f, 200) / 200) / np.log2(40), 0, 1)
        t60 = t60_grave + (t60_agudo - t60_grave) * u
        return np.exp(-6.91 * np.maximum(t, 0) / t60)
    ir = np.stack([espectral(r.standard_normal(n), m, 2048, 512) for _ in range(2)])
    ir *= 1 - np.exp(-tv(n) / 0.008)
    ir = filtro(ir, hp=hp, lp=lp)
    ir = np.concatenate([np.zeros((2, ns(pre))), ir], axis=1)
    return fades(ir / np.sqrt(np.sum(ir ** 2) / 2), 0.0, 0.05)


def reverb(x, ir):
    """Señal húmeda (sólo la cola) de x (2, n) con una sala estéreo; mezcla un poco los lados."""
    if x.ndim == 1:
        x = np.stack([x, x])
    entrada = np.stack([0.75 * x[0] + 0.25 * x[1], 0.25 * x[0] + 0.75 * x[1]])
    return conv(entrada, ir)


def eco(x, t, fb=0.35, rebotes=6, lp=3500, hp=250):
    """Eco ping-pong (sólo húmedo): cada rebote cambia de lado y se oscurece."""
    n = x.shape[-1]
    out = np.zeros_like(x)
    d = ns(t)
    s = x.mean(0)
    for k in range(1, rebotes + 1):
        if k * d >= n:
            break
        s = filtro(s, hp=hp, lp=lp, orden=1)
        out[(k + 1) % 2, k * d:] += s[:n - k * d] * fb ** (k - 1)
    return out


# --- locución: hueco en 1–4 kHz -------------------------------------------------------------------------------

def actividad(t, frases, rampa=0.06):
    """0..1: cuánto está hablando la locución en el instante t."""
    a = np.zeros_like(t, dtype=float)
    for s, e in frases:
        a = np.maximum(a, np.clip((t - (s - rampa)) / rampa, 0, 1) * np.clip((e + rampa - t) / rampa, 0, 1))
    return a


def zona_voz(f):
    """1 en 1,1–3,8 kHz con transiciones suaves hasta 0,7 y 5,2 kHz."""
    lf = np.log2(np.maximum(f, 1.0))
    a = np.clip((lf - np.log2(700)) / (np.log2(1100) - np.log2(700)), 0, 1)
    b = np.clip((np.log2(5200) - lf) / (np.log2(5200) - np.log2(3800)), 0, 1)
    return (0.5 - 0.5 * np.cos(np.pi * a)) * (0.5 - 0.5 * np.cos(np.pi * b))


def hueco_voz(x, frases, prof_db, t0=0.0, rampa=0.06):
    """Ecualizador dinámico: baja prof_db la zona de la voz mientras habla (x (2, n) que empieza en t0)."""
    fin = t0 + x.shape[-1] / SR
    if not any(s - 0.1 < fin and e + 0.1 > t0 for s, e in frases):
        return x
    return espectral(x, lambda t, f: g(prof_db * actividad(t, frases, rampa) * zona_voz(f)), 1024, 256, t0)


# --- instrumentos comunes (la música y los efectos comparten el timbre de marca) ------------------------------

def vidrio(f, vel=1.0, brillo=0.6, caida=1.0, r=None):
    """Mazo de vidrio: parciales casi armónicos con dos modos inarmónicos de barra libre y un batido lento."""
    d = caida * 1.25 * (600 / f) ** 0.35
    parc = [(1.0, 1.0, d), (1.0017, 0.3, d * 0.9), (2.0, 0.2, d * 0.45), (2.76, 0.26 * brillo, d * 0.26),
            (4.07, 0.13 * brillo, d * 0.16), (5.40, 0.06 * brillo, d * 0.09)]
    n = ns(min(5.5 * d, 4.5))
    y = modal(n, [(f * k, a, dd) for k, a, dd in parc])
    r = r or rng(f"vidrio{f:.1f}")
    golpe = filtro(r.standard_normal(n) * env(n, 0.0002, 0.0012), hp=2500, lp=9000) * 0.25 * brillo
    return fades((y + golpe) * vel, 0.0006, 0.02)


def marimba(f, vel=1.0, r=None):
    """Marimba: fundamental, el modo afinado a ~4× y uno a ~9,4× (rápidos), con el golpe blando del mazo."""
    d = float(np.clip(0.7 * (330 / f) ** 0.6, 0.18, 1.4))
    n = ns(min(5 * d, 4.0))
    y = modal(n, [(f, 1.0, d), (f * 3.93, 0.3 * vel, d * 0.22), (f * 9.4, 0.08 * vel, d * 0.07)])
    r = r or rng(f"marimba{f:.1f}")
    y += filtro(r.standard_normal(n) * env(n, 0.0004, 0.003), hp=200, lp=1800) * 0.15
    return fades(y * vel, 0.001, 0.02)


def campanita(f, vel=1.0, caida=1.6):
    """Campana suave: seno con octava y duodécima casi armónicas, con un brillo que tiembla."""
    n = ns(min(5 * caida, 5.0))
    t = tv(n)
    y = modal(n, [(f, 1.0, caida), (2.0 * f, 0.35, caida * 0.55), (3.01 * f, 0.12, caida * 0.3),
                  (4.2 * f, 0.05, caida * 0.15)])
    y *= 1 + 0.08 * np.sin(2 * np.pi * 4.5 * t)
    return fades(y * vel, 0.002, 0.02)


def clic_satinado(r, cuerpo=1.0, tic=1.0, tono=1.0):
    """Clic satinado de dos capas: tic agudo brevísimo y un cuerpo de plástico blando."""
    n = ns(0.12)
    t_ = filtro(r.standard_normal(n) * env(n, 0.0002, 0.0007), hp=5500, lp=12000) * 1.6
    t_ += modal(n, [(7300, 0.22, 0.004), (10100, 0.1, 0.0025)])
    c = modal(n, [(520 * tono, 0.9, 0.026), (1180 * tono, 0.3, 0.014), (2050 * tono, 0.08, 0.008)])
    c += filtro(r.standard_normal(n) * env(n, 0.0004, 0.004), hp=300, lp=1500) * 0.5
    return fades(tic * t_ + cuerpo * c, 0.0002, 0.03)


def blip(f, dur=0.08, desde=0.94, caida=0.035, armonico=0.2):
    """Pitido corto que llega a su nota desde abajo (o desde arriba si desde > 1)."""
    n = ns(dur)
    t = tv(n)
    ph = fase(f * (1 - (1 - desde) * np.exp(-t / 0.008)))
    y = (np.sin(ph) + armonico * np.sin(2 * ph)) * env(n, 0.002, caida)
    return fades(y, 0.001, 0.01)


# --- WAV de 24 bits -------------------------------------------------------------------------------------------

def escribir_wav(ruta, x):
    """x (2, n) en [-1, 1] → WAV PCM de 24 bits (empaquetado a mano: 3 bytes little-endian por muestra)."""
    os.makedirs(os.path.dirname(os.path.abspath(ruta)), exist_ok=True)
    v = np.clip(np.round(x.T * 8388607.0), -8388608, 8388607).astype("<i4")
    b = np.ascontiguousarray(v).reshape(-1).view(np.uint8).reshape(-1, 4)[:, :3].tobytes()
    with wave.open(ruta, "wb") as w:
        w.setnchannels(x.shape[0])
        w.setsampwidth(3)
        w.setframerate(SR)
        w.writeframes(b)


def leer_wav(ruta):
    with wave.open(ruta, "rb") as w:
        ch, sw, sr, nf = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
        raw = w.readframes(nf)
    assert sw == 3, "se esperaba PCM de 24 bits"
    b = np.frombuffer(raw, np.uint8).reshape(-1, 3).astype(np.int32)
    v = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
    v = np.where(v >= 1 << 23, v - (1 << 24), v)
    return v.reshape(-1, ch).T / 8388608.0, sr, sw


# --- revisión sin escuchar ------------------------------------------------------------------------------------

BANDAS = [(0, 120, "<120"), (120, 1000, "120-1k"), (1000, 4000, "1k-4k"), (4000, 24000, ">4k")]


def revisar(ruta, frases=()):
    """Imprime duración, formato, picos, RMS por segundo, bordes, mono y reparto espectral del archivo escrito."""
    x, sr, sw = leer_wav(ruta)
    n = x.shape[1]
    print(f"== {ruta}")
    print(f"  {n} muestras · {n / sr:.4f} s · {sr} Hz · {x.shape[0]} canales · {8 * sw} bits")
    pk = [np.abs(c).max() for c in x]
    recortes = int(np.sum(np.abs(x) >= 0.99999))
    print(f"  pico L {dbfs(pk[0]):6.2f} dBFS · R {dbfs(pk[1]):6.2f} dBFS · muestras recortadas: {recortes}")
    mono = x.mean(0)
    perdida = dbfs(np.sqrt(np.mean(mono ** 2))) - dbfs(np.sqrt(np.mean(x ** 2)))
    corr = np.corrcoef(x[0], x[1])[0, 1]
    print(f"  mono: pico {dbfs(np.abs(mono).max()):6.2f} dBFS · RMS mono − RMS estéreo {perdida:+.2f} dB · "
          f"correlación L/R {corr:.2f}")
    a, b = np.abs(x[:, :int(0.05 * sr)]).max(), np.abs(x[:, n - int(0.2 * sr):]).max()
    print(f"  bordes: primeros 0,05 s pico {dbfs(a):7.1f} dBFS · últimos 0,20 s pico {dbfs(b):7.1f} dBFS")
    rms = [dbfs(np.sqrt(np.mean(x[:, k * sr:(k + 1) * sr] ** 2))) for k in range(int(np.ceil(n / sr)))]
    print("  RMS por segundo (dBFS):")
    for k in range(0, len(rms), 6):
        print("   " + "  ".join(f"{s:2d}s {v:6.1f}" for s, v in zip(range(k, k + 6), rms[k:k + 6])))
    P = np.abs(np.fft.rfft(x, axis=1)) ** 2
    P = P.sum(0)
    f = np.fft.rfftfreq(n, 1 / sr)
    tot = P.sum()
    print("  reparto espectral: " + " · ".join(f"{nom} Hz {100 * P[(f >= lo) & (f < hi)].sum() / tot:5.1f} %"
                                              for lo, hi, nom in BANDAS))
    if frases:
        X = np.abs(stft(mono, 2048, 512)) ** 2
        t = (np.arange(len(X)) * 512 - 1024) / sr
        fb = np.fft.rfftfreq(2048, 1 / sr)
        voz = actividad(t, frases) > 0.99
        zona = (fb >= 1000) & (fb < 4000)

        def parte(sel):
            return 100 * X[sel][:, zona].sum() / max(X[sel].sum(), 1e-20)
        print(f"  parte en 1–4 kHz: {parte(voz):5.1f} % mientras habla la locución · {parte(~voz):5.1f} % en sus pausas")
    return x


if __name__ == "__main__":
    sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")))
    import ad.timeline as T
    for p in sys.argv[1:]:
        revisar(p, [(s, s + (b - a)) for a, b, s in T.VO])
