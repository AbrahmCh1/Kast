"""Hoja de contactos de fotogramas para revisarlos: python ad/tools/sheet.py salida.png fotograma1.png ... [--cols 3]"""
import sys
from PIL import Image, ImageDraw, ImageFont

args = [a for a in sys.argv[1:] if not a.startswith("--")]
cols = 3
for a in sys.argv[1:]:
    if a.startswith("--cols="):
        cols = int(a.split("=")[1])
out, files = args[0], args[1:]
ims = [Image.open(f).convert("RGB") for f in files]
w = 640
ims = [im.resize((w, round(im.height * w / im.width)), Image.LANCZOS) for im in ims]
h = max(im.height for im in ims)
rows = (len(ims) + cols - 1) // cols
sheet = Image.new("RGB", (cols * w + (cols + 1) * 6, rows * (h + 22) + 6), (40, 40, 40))
d = ImageDraw.Draw(sheet)
try:
    font = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 14)
except OSError:
    font = None
for i, (im, f) in enumerate(zip(ims, files)):
    x = 6 + (i % cols) * (w + 6)
    y = 6 + (i // cols) * (h + 22)
    sheet.paste(im, (x, y + 18))
    d.text((x, y), f.replace("\\", "/").split("/")[-1], fill=(220, 220, 220), font=font)
sheet.save(out)
print(out)
