"""Coloca la locución en el anuncio: corta la toma elegida por frases (en los silencios entre ellas) y pone cada una
en su ancla de ad/timeline.py. La voz no se acelera ni se estira: sólo cambian las pausas.

  python ad/audio/vo.py  ->  ad/audio/out/vo.wav (48 kHz, estéreo, 24 bits, 21,2 s)
"""
import os
import subprocess
import sys
import numpy as np

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, ROOT)
import ad.timeline as T  # noqa: E402

SR = 48000
OUT = os.path.join(ROOT, "ad", "audio", "out")


def decode(path):
    """mp3 -> float32 mono a 48 kHz (ffmpeg)."""
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-ac", "1", "-ar", str(SR), "-f", "f32le", "-"],
                         capture_output=True, check=True).stdout
    return np.frombuffer(raw, np.float32).copy()


def write_wav(path, x):
    """Estéreo float (-1..1) -> WAV PCM de 24 bits."""
    import wave
    x = np.clip(x, -1, 1)
    i = np.round(x * 8388607).astype(np.int32)
    b = np.zeros((i.size, 3), np.uint8)
    flat = i.reshape(-1)
    b[:, 0] = flat & 0xFF
    b[:, 1] = (flat >> 8) & 0xFF
    b[:, 2] = (flat >> 16) & 0xFF
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(3)
        w.setframerate(SR)
        w.writeframes(b.tobytes())


def highpass(x, fc=80.0):
    """Paso alto de 2.º orden (biquad) para quitar retumbe sin tocar la voz."""
    w0 = 2 * np.pi * fc / SR
    q = 0.707
    al = np.sin(w0) / (2 * q)
    cw = np.cos(w0)
    b0, b1, b2 = (1 + cw) / 2, -(1 + cw), (1 + cw) / 2
    a0, a1, a2 = 1 + al, -2 * cw, 1 - al
    b0, b1, b2, a1, a2 = b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0
    y = np.zeros_like(x)
    x1 = x2 = y1 = y2 = 0.0
    for n in range(x.size):
        xn = x[n]
        yn = b0 * xn + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2
        y[n] = yn
        x2, x1, y2, y1 = x1, xn, y1, yn
    return y


def compress(x, ratio=3.0, over=6.0, ceiling=11.0, attack=0.003, release=0.08):
    """Compresor de locución: 3:1 por encima del nivel hablado + 6 dB y techo de picos a + 11 dB (con 2 ms de
    anticipación), para que el limitador del máster apenas trabaje. Detector por bloques de 1 ms."""
    blk = SR // 1000
    n = -(-x.size // blk)
    xp = np.zeros(n * blk, np.float32)
    xp[:x.size] = x
    xb = xp.reshape(n, blk)
    pk = np.abs(xb).max(1)
    rms = np.sqrt((xb ** 2).mean(1))
    level = 20 * np.log10(np.sqrt(np.mean(rms[rms > rms.max() * 0.01] ** 2)))
    a_att, a_rel = np.exp(-1 / (attack * 1000)), np.exp(-1 / (release * 1000))
    env = np.zeros(n)
    e = 0.0
    for i in range(n):
        a = a_att if pk[i] > e else a_rel
        e = a * e + (1 - a) * pk[i]
        env[i] = e
    edb = 20 * np.log10(env + 1e-9)
    g = np.minimum(0.0, (level + over - edb) * (1 - 1 / ratio))
    g = np.minimum(g, level + ceiling - 20 * np.log10(pk + 1e-9))
    # Anticipación (la reducción empieza 2 ms antes del pico) y recuperación suave de 30 ms.
    g = np.min(np.stack([np.roll(g, -k) for k in range(3)] + [np.roll(g, k) for k in range(1, 3)]), 0)
    r = np.exp(-1 / 30)
    for i in range(1, n):
        g[i] = min(g[i], r * g[i - 1])
    gs = np.interp(np.arange(x.size), np.arange(n) * blk + blk / 2, g)
    return x * 10 ** (gs / 20)


def main():
    take = decode(os.path.join(ROOT, T.VO_TAKE))
    take = compress(highpass(take))
    out = np.zeros(round(T.TOTAL * SR), np.float32)
    segs = T.VO
    for k, (a, b, at) in enumerate(segs):
        # Un poco antes y después (respiración y cola natural), sin invadir la frase vecina.
        nxt = segs[k + 1][0] if k + 1 < len(segs) else len(take) / SR
        prv = segs[k - 1][1] if k else 0.0
        a0 = max(0.0, a - 0.06, prv)
        b1 = min(b + 0.16, nxt - 0.02)
        s = take[int(a0 * SR):int(b1 * SR)].copy()
        f = int(0.012 * SR)
        s[:f] *= np.linspace(0, 1, f)
        g = int(0.06 * SR)
        s[-g:] *= np.linspace(1, 0, g) ** 2
        i = int((at - (a - a0)) * SR)
        out[i:i + s.size] += s[:max(0, out.size - i)]
        print(f"frase {k + 1}: toma {a:.2f}-{b:.2f} s -> anuncio {at:.2f}-{at + (b - a):.2f} s")
    # Nivel hablado fijo (-16 dBFS RMS, con el que se equilibraron música y efectos), sin pasar de -1 dBFS de pico.
    h = SR // 200
    e = np.sqrt((out[:out.size // h * h].reshape(-1, h) ** 2).mean(1))
    e = e[e > e.max() * 0.08]
    out *= min(10 ** (-16 / 20) / np.sqrt(np.mean(e ** 2)), 0.89 / np.max(np.abs(out)))
    os.makedirs(OUT, exist_ok=True)
    write_wav(os.path.join(OUT, "vo.wav"), np.stack([out, out], 1))
    print("vo.wav", f"{out.size / SR:.2f} s")


if __name__ == "__main__":
    main()
