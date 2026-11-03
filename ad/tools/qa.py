"""Validación técnica de los másters: duración, resolución, fps, fotogramas, negros, sonoridad y pico real, y
capturas de los momentos clave. python ad/tools/qa.py -> ad/out/qa/informe.md"""
import json
import os
import re
import subprocess
import sys
import numpy as np

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, ROOT)
import ad.timeline as T  # noqa: E402

QA = os.path.join(ROOT, "ad", "out", "qa")
KEY = [("El clic", 0.42), ("Logo", 1.45), ("Universo de pantallas", 2.0), ("Botón de Kast", 2.5), ("Calibración", 3.7),
       ("Confirmación", 4.62), ("Disposiciones", 5.95), ("Dispositivos", 8.6), ("Conectado", 9.42), ("La otra pantalla", 10.2),
       ("Pantalla virtual", 10.85), ("Windows", 11.5), ("Mac", 12.15), ("iPhone", 12.8), ("Android", 13.95),
       ("Todas tus pantallas", 15.2), ("Un solo lugar", 16.3), ("Aterrizaje", 17.22), ("Kast", 17.8), ("Firma sonora", 18.5),
       ("Firma", 20.5)]
# Momentos en los que el brief pide la imagen despejada a propósito (no cuentan como fotogramas vacíos).
DESPEJADO = [(16.95, 17.15, "escena 9: «durante una fracción de segundo, la imagen queda despejada» antes del aterrizaje")]


def probe(path):
    r = subprocess.run(["ffprobe", "-v", "error", "-count_frames", "-show_streams", "-show_format", "-of", "json", path],
                       capture_output=True, text=True, check=True)
    return json.loads(r.stdout)


def loudness(path):
    r = subprocess.run(["ffmpeg", "-nostats", "-i", path, "-filter_complex", "ebur128=peak=true", "-f", "null", "-"],
                       capture_output=True, text=True, encoding="utf-8", errors="replace").stderr
    tail = r[r.rfind("Summary:"):]
    return float(re.search(r"I:\s+(-?[\d.]+) LUFS", tail).group(1)), float(re.search(r"Peak:\s+(-?[\d.inf]+) dBFS", tail).group(1))


def frame_stats(path, w, h):
    """Brillo medio, diferencia con el anterior y percentil 99,9 de cada fotograma (decodificado a 192x108, para que
    una pantalla lejana encendida cuente): un fotograma vacío es el que no tiene nada por encima del grafito del fondo,
    por oscuro que sea el plano."""
    # Se lee el plano de luma tal cual (rango TV, 16–235) y se pasa a 0–1 aquí: la conversión a gris de ffmpeg tras
    # escalar aplasta las sombras y daría por negros planos que no lo son.
    r = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-vf", "scale=192:108:flags=area", "-pix_fmt", "yuv420p",
                        "-f", "rawvideo", "-"], capture_output=True, check=True).stdout
    y = np.frombuffer(r, np.uint8).reshape(-1, 192 * 108 * 3 // 2)[:, :192 * 108].reshape(-1, 108, 192).astype(np.float32)
    a = np.clip((y - 16) / 219, 0, 1)
    return a.mean((1, 2)), np.abs(np.diff(a, axis=0)).mean((1, 2)), np.percentile(a.reshape(len(a), -1), 99.9, axis=1)


def main():
    os.makedirs(QA, exist_ok=True)
    lines = ["# Validación técnica — Kast · Un solo lugar", ""]
    ok_all = True
    for aspect, size in (("h", "1920x1080"), ("v", "1080x1920")):
        path = os.path.join(ROOT, "ad", "out", f"Kast_UnSoloLugar_{size}.mp4")
        if not os.path.exists(path):
            lines += [f"## {size}", "", "No existe todavía.", ""]
            ok_all = False
            continue
        p = probe(path)
        v = next(s for s in p["streams"] if s["codec_type"] == "video")
        a = next(s for s in p["streams"] if s["codec_type"] == "audio")
        dur = float(p["format"]["duration"])
        frames = int(v["nb_read_frames"])
        fps = eval(v["r_frame_rate"])
        w, h = v["width"], v["height"]
        lufs, tp = loudness(path)
        lum, diff, top = frame_stats(path, w, h)
        black = [i for i in range(len(lum)) if top[i] < 0.10 and (i / 60 > 0.06)
                 and not any(a <= i / 60 <= b for a, b, _ in DESPEJADO)]
        checks = [
            ("Duración 21,2 s", abs(dur - T.TOTAL) < 0.03, f"{dur:.3f} s"),
            ("Fotogramas 1272", frames == T.FRAMES, str(frames)),
            ("60 fps", abs(fps - 60) < 1e-6, f"{fps:g}"),
            (f"Resolución {size}", f"{w}x{h}" == size, f"{w}x{h}"),
            ("Audio 48 kHz estéreo", a["sample_rate"] == "48000" and a["channels"] == 2, f"{a['sample_rate']} Hz, {a['channels']} canales"),
            ("Duración del audio", abs(float(a.get("duration", dur)) - T.TOTAL) < 0.05, f"{float(a.get('duration', dur)):.3f} s"),
            ("Sonoridad −14 LUFS (±0,5)", abs(lufs + 14) <= 0.5, f"{lufs:.2f} LUFS"),
            ("Pico real ≤ −1,25 dBTP", tp <= -1.25 + 0.05, f"{tp:.2f} dBTP"),
            ("Sin fotogramas negros o vacíos", not black,
             (f"{len(black)} sin nada por encima del grafito" if black else "ninguno") + f" (el más oscuro: brillo medio {lum.min():.3f})"),
            ("Sin saltos de imagen (cortes)", float(diff.max()) < 0.35, f"diferencia máxima entre fotogramas {diff.max():.3f}"),
        ]
        lines += [f"## {size}", "", "| Comprobación | Resultado | Valor |", "| --- | --- | --- |"]
        for name, ok, val in checks:
            ok_all &= bool(ok)
            lines.append(f"| {name} | {'✔' if ok else '✘'} | {val} |")
        lines.append("")
        # Capturas de los momentos clave.
        cap = os.path.join(QA, aspect)
        os.makedirs(cap, exist_ok=True)
        for name, t in KEY:
            fn = os.path.join(cap, f"{t:05.2f}_{name.replace(' ', '_')}.png")
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", f"{t:.3f}", "-i", path, "-frames:v", "1", fn], check=True)
        for a_, b_, why in DESPEJADO:
            n = sum(1 for i in range(len(lum)) if a_ <= i / 60 <= b_ and top[i] < 0.10)
            lines.append(f"Imagen despejada a propósito, {a_:.2f}–{b_:.2f} s ({n} fotogramas): {why}.")
        if black:
            lines.append("Fotogramas vacíos: " + ", ".join(f"{i} ({i / 60:.2f} s)" for i in black[:40]))
        lines.append(f"Capturas: ad/out/qa/{aspect}/ ({len(KEY)} momentos clave).")
        lines.append("")
    with open(os.path.join(QA, "informe.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    print("\n".join(lines))
    print("TODO OK" if ok_all else "HAY PROBLEMAS")


if __name__ == "__main__":
    main()
