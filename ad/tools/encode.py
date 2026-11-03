"""Máster final: fotogramas 16 bits + mezcla -> MP4 (H.264 High, 60 fps, BT.709) con grano fino.
  python ad/tools/encode.py h|v"""
import os
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
aspect = sys.argv[1] if len(sys.argv) > 1 else "h"
size = {"h": "1920x1080", "v": "1080x1920"}[aspect]
out = os.path.join(ROOT, "ad", "out", f"Kast_UnSoloLugar_{size}.mp4")
cmd = [
    "ffmpeg", "-v", "error", "-y",
    "-framerate", "60", "-i", os.path.join(ROOT, "ad", "out", aspect, "f_%04d.png"),
    "-i", os.path.join(ROOT, "ad", "audio", "out", "mix.wav"),
    # Etalonaje: el negro del render se levanta al grafito del brief (#0A0A0C: «no es un negro vacío») con un pie
    # suave que deja intactos medios y altas. Después, grano fino temporal (también evita el banding a 8 bits).
    "-vf", "curves=all='0/0.032 0.04/0.06 0.15/0.16 1/1',noise=c0s=4:c0f=t+u,format=yuv420p",
    "-c:v", "libx264", "-preset", "slow", "-crf", "14", "-profile:v", "high", "-tune", "film", "-g", "120",
    "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv",
    "-c:a", "aac", "-b:a", "320k", "-ar", "48000",
    "-shortest", "-movflags", "+faststart", out,
]
subprocess.run(cmd, check=True)
print(out)
