"""Mezcla y máster: voz + música (que se aparta bajo la voz) + efectos, a -14 LUFS integrados y pico real <= -1,25 dBTP.

  python ad/audio/mix.py  ->  ad/audio/out/mix.wav (48 kHz, estéreo, 24 bits) y el informe de niveles
"""
import os
import re
import subprocess
import sys
import wave
import numpy as np

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, ROOT)
import ad.timeline as T  # noqa: E402
from ad.audio.vo import write_wav, SR  # noqa: E402

OUT = os.path.join(ROOT, "ad", "audio", "out")
TARGET_LUFS = -14.0
TP_MAX = -1.25


def read_wav(path):
    with wave.open(path, "rb") as w:
        n, ch, sw = w.getnframes(), w.getnchannels(), w.getsampwidth()
        assert w.getframerate() == SR, path
        raw = np.frombuffer(w.readframes(n), np.uint8)
    if sw == 3:
        b = raw.reshape(-1, 3).astype(np.int32)
        i = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
        i = np.where(i & 0x800000, i - 0x1000000, i)
        x = i.astype(np.float32) / 8388608
    elif sw == 2:
        x = raw.view(np.int16).astype(np.float32) / 32768
    else:
        x = raw.view(np.int32).astype(np.float32) / 2147483648
    x = x.reshape(-1, ch)
    if ch == 1:
        x = np.repeat(x, 2, 1)
    full = round(T.TOTAL * SR)
    if x.shape[0] < full:
        x = np.vstack([x, np.zeros((full - x.shape[0], 2), np.float32)])
    return x[:full]


def envelope(x, attack=0.03, release=0.25):
    """Envolvente de la voz (seguidor de picos con ataque y liberación), 0..1."""
    m = np.abs(x).max(1)
    blk = int(0.005 * SR)
    m = m[: m.size // blk * blk].reshape(-1, blk).max(1)
    env = np.zeros_like(m)
    a = np.exp(-0.005 / attack)
    r = np.exp(-0.005 / release)
    e = 0.0
    for i, v in enumerate(m):
        e = v + a * (e - v) if v > e else v + r * (e - v)
        env[i] = e
    env = np.clip(env / (np.percentile(env[env > 1e-4], 90) + 1e-9), 0, 1) if np.any(env > 1e-4) else env
    env = np.repeat(env, blk)
    return np.pad(env, (0, x.shape[0] - env.size), mode="edge")


def measure(path):
    """Sonoridad integrada (LUFS) y pico real (dBTP) con ffmpeg (EBU R128)."""
    r = subprocess.run(["ffmpeg", "-nostats", "-i", path, "-filter_complex", "ebur128=peak=true", "-f", "null", "-"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace").stderr
    tail = r[r.rfind("Summary:"):]
    lufs = float(re.search(r"I:\s+(-?[\d.]+) LUFS", tail).group(1))
    tp = float(re.search(r"Peak:\s+(-?[\d.inf]+) dBFS", tail).group(1))
    return lufs, tp


def limiter(x, ceiling_db, lookahead=0.004, release=0.08):
    """Limitador con anticipación sobre picos estimados con sobremuestreo x4 (pico real)."""
    up = 4
    n = x.shape[0]
    X = np.fft.rfft(x, axis=0)
    Xu = np.zeros((n * up // 2 + 1, 2), complex)
    Xu[: X.shape[0]] = X
    xu = np.fft.irfft(Xu, n * up, axis=0) * up
    pk = np.abs(xu).max(1).reshape(n, up).max(1)
    ceil = 10 ** (ceiling_db / 20)
    need = np.minimum(1.0, ceil / np.maximum(pk, 1e-9))
    la = int(lookahead * SR)
    # mínimo móvil hacia delante (anticipación) y liberación suave
    g = need.copy()
    for k in range(1, la + 1):
        g[:-k] = np.minimum(g[:-k], need[k:])
    rel = np.exp(-1 / (release * SR))
    out = np.empty_like(g)
    cur = 1.0
    for i, v in enumerate(g):
        cur = v if v < cur else v + rel * (cur - v)
        out[i] = cur
    return x * out[:, None]


def main():
    vo = read_wav(os.path.join(OUT, "vo.wav"))
    music = read_wav(os.path.join(OUT, "music.wav"))
    sfx = read_wav(os.path.join(OUT, "sfx.wav"))
    env = envelope(vo)
    speech = env > 0.08
    # La música se aparta bajo la voz lo justo para quedar al menos 12 dB por debajo de ella.
    duck_db = -9.0
    music_gain = 0.55
    for _ in range(12):
        duck = 10 ** (duck_db * env / 20)
        m = music * music_gain * duck[:, None]
        rv = np.sqrt(np.mean(vo[speech] ** 2))
        rm = np.sqrt(np.mean(m[speech] ** 2))
        diff = 20 * np.log10(rv / max(rm, 1e-9))
        if diff >= 12.5:
            break
        duck_db -= 1.0
    sfx_gain = 0.62
    mix = vo * 1.0 + m + sfx * sfx_gain
    # Cola: todo se apaga a cero en los últimos 30 ms (sin chasquido final).
    f = int(0.03 * SR)
    mix[-f:] *= np.linspace(1, 0, f)[:, None]
    tmp = os.path.join(OUT, "mix_pre.wav")
    write_wav(tmp, mix / max(1.0, np.abs(mix).max()))
    lufs, tp = measure(tmp)
    mix = (mix / max(1.0, np.abs(mix).max())) * 10 ** ((TARGET_LUFS - lufs) / 20)
    mix = limiter(mix, TP_MAX - 0.35)
    path = os.path.join(OUT, "mix.wav")
    write_wav(path, mix)
    lufs2, tp2 = measure(path)
    # Ajuste fino: el limitador baja un poco la sonoridad (más con una voz de picos marcados), así que se repite.
    for _ in range(4):
        if abs(lufs2 - TARGET_LUFS) <= 0.1:
            break
        mix = limiter(mix * 10 ** ((TARGET_LUFS - lufs2) / 20), TP_MAX - 0.35)
        write_wav(path, mix)
        lufs2, tp2 = measure(path)
    mono = mix.mean(1)
    corr = float(np.corrcoef(mix[:, 0], mix[:, 1])[0, 1])
    print(f"voz sobre música: {diff:.1f} dB (atenuación de la música bajo la voz: {duck_db:.0f} dB)")
    print(f"mezcla: {lufs2:.2f} LUFS integrados, pico real {tp2:.2f} dBTP, correlación L/R {corr:.2f}, "
          f"pico mono {20 * np.log10(np.abs(mono).max() + 1e-9):.2f} dBFS")


if __name__ == "__main__":
    main()
