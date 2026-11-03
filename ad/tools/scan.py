"""Revisión automática de una secuencia: brillo medio por fotograma, tramos casi negros y saltos bruscos entre
fotogramas consecutivos (posibles cortes o teletransportes de cámara).
  python ad/tools/scan.py ad/out/prev/p_ [fps]"""
import glob
import sys
import numpy as np
from PIL import Image

prefix = sys.argv[1]
fps = float(sys.argv[2]) if len(sys.argv) > 2 else 60.0
files = sorted(glob.glob(prefix + "*.png"))
prev = None
lum, diff = [], []
for f in files:
    im = np.asarray(Image.open(f).convert("L").resize((160, 90)), np.float32) / 255
    lum.append(im.mean())
    diff.append(0.0 if prev is None else float(np.abs(im - prev).mean()))
    prev = im
lum, diff = np.array(lum), np.array(diff)
print(f"{len(files)} fotogramas; brillo medio {lum.mean():.3f}")
dark = lum < 0.025
runs, start = [], None
for i, d in enumerate(dark):
    if d and start is None:
        start = i
    if (not d or i == len(dark) - 1) and start is not None:
        end = i if not d else i + 1
        if end - start >= 6:
            runs.append((start, end))
        start = None
print("tramos casi negros (>= 0,1 s):", ", ".join(f"{a / fps:.2f}-{b / fps:.2f} s" for a, b in runs) or "ninguno")
med = np.median(diff[1:])
jumps = [i for i in range(1, len(diff)) if diff[i] > max(0.06, 6 * med)]
print("saltos bruscos:", ", ".join(f"{i / fps:.2f} s ({diff[i]:.3f})" for i in jumps) or "ninguno")
for s in range(0, int(len(files) / fps) + 1):
    seg = lum[int(s * fps):int((s + 1) * fps)]
    if seg.size:
        print(f"  {s:2d}-{s + 1:2d} s  brillo {seg.mean():.3f}  min {seg.min():.3f}")
