"""Texturas de las pantallas (Pillow + NumPy): escritorios originales, la ventana de Kast, el Mac, el iPhone en modo
trackpad y la pantalla interactiva. python ad/render/textures.py -> ad/out/tex/*.png"""
import math
import os
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

OUT = os.path.join(os.path.dirname(__file__), "..", "out", "tex")
F = "C:/Windows/Fonts/"


def font(size, w="regular"):
    return ImageFont.truetype(F + {"regular": "segoeui.ttf", "semibold": "seguisb.ttf", "bold": "segoeuib.ttf",
                                   "light": "segoeuil.ttf", "mono": "consola.ttf"}[w], size)


def gradient(w, h, stops, angle=0.0, noise=2.2, seed=0):
    """Degradado multicolor (stops: [(pos, (r,g,b))]) en un ángulo, con ruido fino contra el banding."""
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    a = math.radians(angle)
    t = ((x / w - 0.5) * math.cos(a) + (y / h - 0.5) * math.sin(a)) + 0.5
    t = np.clip(t, 0, 1)
    img = np.zeros((h, w, 3), np.float32)
    ps = [s[0] for s in stops]
    for c in range(3):
        img[..., c] = np.interp(t, ps, [s[1][c] for s in stops])
    if noise:
        rng = np.random.default_rng(seed)
        img += rng.normal(0, noise, img.shape)
    return Image.fromarray(np.clip(img, 0, 255).astype(np.uint8))


def blobs(w, h, items, base, blur=180):
    """Manchas de color difuminadas (fondos abstractos originales)."""
    im = Image.new("RGB", (w, h), base)
    d = ImageDraw.Draw(im)
    for (cx, cy, r, col) in items:
        d.ellipse((cx * w - r * w, cy * h - r * w, cx * w + r * w, cy * h + r * w), fill=col)
    im = np.asarray(im.filter(ImageFilter.GaussianBlur(blur)), np.float32)
    im += np.random.default_rng(int(blur)).normal(0, 2.2, im.shape)   # grano fino contra el banding
    return Image.fromarray(np.clip(im, 0, 255).astype(np.uint8))


def rr(d, box, r, fill=None, outline=None, width=1):
    d.rounded_rectangle(box, r, fill=fill, outline=outline, width=width)


def window(im, box, title, dark=True, r=18):
    d = ImageDraw.Draw(im, "RGBA")
    x0, y0, x1, y1 = box
    # sombra
    sh = Image.new("RGBA", im.size, (0, 0, 0, 0))
    ImageDraw.Draw(sh).rounded_rectangle((x0 + 8, y0 + 18, x1 + 8, y1 + 26), r, fill=(0, 0, 0, 120))
    sh = sh.filter(ImageFilter.GaussianBlur(28))
    im.paste(sh, (0, 0), sh)
    d = ImageDraw.Draw(im, "RGBA")
    rr(d, box, r, fill=(32, 32, 36, 255) if dark else (246, 246, 248, 255))
    rr(d, (x0, y0, x1, y0 + 52), r, fill=(44, 44, 50, 255) if dark else (232, 232, 236, 255))
    d.rectangle((x0, y0 + 30, x1, y0 + 52), fill=(44, 44, 50, 255) if dark else (232, 232, 236, 255))
    d.text((x0 + 24, y0 + 12), title, font=font(22, "semibold"), fill=(230, 230, 235) if dark else (40, 40, 44))
    return d


def taskbar(im, kind="win"):
    w, h = im.size
    d = ImageDraw.Draw(im, "RGBA")
    if kind == "win":
        d.rectangle((0, h - 64, w, h), fill=(24, 26, 34, 230))
        cx = w // 2 - 5 * 30
        for i in range(6):
            rr(d, (cx + i * 60, h - 52, cx + i * 60 + 40, h - 12), 8,
               fill=[(80, 140, 255), (255, 255, 255), (129, 151, 255), (108, 203, 95), (255, 180, 80), (200, 200, 210)][i] + (230,))
        d.text((w - 140, h - 46), "10:24", font=font(22), fill=(230, 230, 235))
    else:
        d.rectangle((0, 0, w, 34), fill=(250, 250, 252, 120))
        d.text((24, 4), "  Finder   Archivo   Edición   Ver", font=font(20, "semibold"), fill=(20, 20, 24))
        d.text((w - 120, 4), "10:24", font=font(20), fill=(20, 20, 24))
        rr(d, (w // 2 - 360, h - 96, w // 2 + 360, h - 14), 26, fill=(255, 255, 255, 70))
        for i in range(9):
            col = [(255, 255, 255), (129, 151, 255), (255, 120, 90), (108, 203, 95), (255, 200, 60), (90, 170, 255),
                   (230, 230, 235), (167, 139, 250), (40, 40, 44)][i]
            rr(d, (w // 2 - 330 + i * 74, h - 86, w // 2 - 330 + i * 74 + 60, h - 26), 14, fill=col + (255,))


def wall_win(w=1920, h=1080):
    im = blobs(w, h, [(0.25, 0.7, 0.35, (40, 80, 200)), (0.7, 0.35, 0.3, (90, 120, 255)), (0.55, 0.9, 0.25, (20, 40, 120)),
                      (0.9, 0.8, 0.2, (129, 151, 255))], (10, 16, 40), 200)
    return im


def wall_mac(w=1920, h=1080):
    return blobs(w, h, [(0.2, 0.3, 0.3, (120, 60, 160)), (0.7, 0.6, 0.35, (230, 120, 90)), (0.4, 0.9, 0.3, (60, 40, 120)),
                        (0.9, 0.2, 0.2, (255, 190, 120))], (30, 20, 50), 220)


def desk_win(content):
    im = wall_win()
    w, h = im.size
    if content == "code":
        d = window(im, (240, 120, 1500, 900), "main.rs — Kast")
        y = 190
        rng = np.random.default_rng(3)
        cols = [(129, 151, 255), (167, 139, 250), (108, 203, 95), (230, 230, 235), (255, 180, 80)]
        for i in range(24):
            x = 300 + int(rng.integers(0, 5)) * 32
            for j in range(int(rng.integers(2, 6))):
                ww = int(rng.integers(40, 180))
                rr(d, (x, y, x + ww, y + 14), 4, fill=cols[int(rng.integers(0, 5))] + (220,))
                x += ww + 14
            y += 28
    elif content == "sheet":
        d = window(im, (300, 150, 1650, 950), "Ventas 2026.xlsx", dark=False)
        for r in range(22):
            for c in range(8):
                x0 = 330 + c * 160
                y0 = 220 + r * 32
                d.rectangle((x0, y0, x0 + 158, y0 + 30), outline=(215, 215, 222), fill=(255, 255, 255) if r else (232, 236, 250))
                if r and c:
                    d.text((x0 + 60, y0 + 4), f"{(r * 37 + c * 91) % 997:>4}", font=font(18), fill=(60, 60, 66))
        for c in range(7):
            d.rectangle((1180 + c * 56, 860 - (c * 47 % 230) - 60, 1220 + c * 56, 860), fill=(67, 97, 238))
    elif content == "photo":
        d = window(im, (200, 110, 1720, 960), "Atardecer.jpg")
        ph = gradient(1440, 700, [(0, (255, 170, 90)), (0.45, (240, 110, 120)), (0.8, (80, 60, 140)), (1, (20, 20, 50))], 90)
        im.paste(ph, (240, 200))
        d = ImageDraw.Draw(im, "RGBA")
        d.ellipse((900, 520, 1080, 700), fill=(255, 230, 170, 255))
        d.rectangle((240, 760, 1680, 900), fill=(14, 14, 30, 255))
    taskbar(im, "win")
    return im


def desk_mac(content=None):
    im = wall_mac()
    if content == "notes":
        d = window(im, (360, 160, 1500, 880), "Notas", dark=False, r=22)
        d.text((420, 240), "Ideas para el lanzamiento", font=font(40, "semibold"), fill=(30, 30, 34))
        for i in range(9):
            rr(d, (420, 320 + i * 52, 420 + (900 - (i * 97) % 380), 340 + i * 52), 8, fill=(200, 200, 210))
    elif content == "photos":
        d = window(im, (300, 140, 1620, 900), "Fotos", dark=True, r=22)
        for r in range(3):
            for c in range(5):
                g = gradient(230, 200, [(0, (120 + 25 * c, 90 + 30 * r, 200 - 20 * c)), (1, (40, 30 + 20 * r, 90))], 30 + 30 * c)
                im.paste(g, (340 + c * 252, 220 + r * 222))
    taskbar(im, "mac")
    return im


def kast_ui(w=2400, h=1500, cards=True):
    """La ventana de Kast (interfaz oscura): barra lateral y tres monitores con su brillo."""
    im = Image.new("RGB", (w, h), (32, 32, 32))
    d = ImageDraw.Draw(im, "RGBA")
    d.rectangle((0, 0, 460, h), fill=(26, 26, 28))
    # marca
    rr(d, (40, 40, 112, 112), 18, fill=(52, 52, 58))
    rr(d, (58, 62, 94, 87), 6, outline=(255, 255, 255), width=4)
    d.rectangle((76, 62, 94, 87), fill=(255, 255, 255))
    d.line((66, 98, 86, 98), fill=(255, 255, 255), width=4)
    d.text((134, 44), "Kast", font=font(40, "semibold"), fill=(255, 255, 255))
    d.text((134, 92), "Tus pantallas", font=font(22), fill=(169, 169, 169))
    for i, (lab, on) in enumerate((("Monitores", True), ("Disposiciones", False), ("Dispositivos", False), ("Atajos", False),
                                   ("Ajustes", False))):
        y = 200 + i * 74
        if on:
            rr(d, (24, y - 12, 436, y + 48), 12, fill=(255, 255, 255, 18))
            rr(d, (24, y + 2, 30, y + 34), 3, fill=(129, 151, 255))
        d.text((64, y), lab, font=font(28, "semibold" if on else "regular"), fill=(255, 255, 255) if on else (190, 190, 195))
    d.text((520, 60), "Monitores", font=font(56, "semibold"), fill=(255, 255, 255))
    d.text((520, 136), "Brillo, contraste y entrada de cada pantalla", font=font(26), fill=(169, 169, 169))
    for i in range(3 if cards else 0):
        x0 = 520 + i * 620
        rr(d, (x0, 230, x0 + 580, 1380), 22, fill=(255, 255, 255, 14), outline=(255, 255, 255, 20), width=2)
        rr(d, (x0 + 40, 280, x0 + 540, 560), 14, fill=(20, 20, 24))
        g = gradient(480, 260, [(0, (67, 97, 238)), (1, (129, 151, 255))], 20)
        im.paste(g, (x0 + 50, 290))
        d.text((x0 + 40, 600), f"Monitor {i + 1}", font=font(36, "semibold"), fill=(255, 255, 255))
        d.text((x0 + 40, 652), ["27\" · 4K", "27\" · 144 Hz", "24\" · Full HD"][i], font=font(24), fill=(169, 169, 169))
        for j, (lab, v) in enumerate((("Brillo", [40, 85, 20][i]), ("Contraste", [70, 50, 60][i]), ("Volumen", [30, 0, 55][i]))):
            y = 760 + j * 170
            d.text((x0 + 40, y), lab, font=font(28), fill=(220, 220, 225))
            d.text((x0 + 470, y), f"{v}", font=font(28, "semibold"), fill=(255, 255, 255))
            rr(d, (x0 + 40, y + 62, x0 + 540, y + 72), 5, fill=(255, 255, 255, 60))
            rr(d, (x0 + 40, y + 62, x0 + 40 + 5 * v, y + 72), 5, fill=(129, 151, 255))
            d.ellipse((x0 + 40 + 5 * v - 16, y + 51, x0 + 40 + 5 * v + 16, y + 83), fill=(255, 255, 255))
    return im


def calib_screen(w=1920, h=1080):
    """Lo que muestran los tres monitores al calibrar: una página clara con fotos (así se notan el tono y el brillo)."""
    im = Image.new("RGB", (w, h), (236, 236, 238))
    d = ImageDraw.Draw(im)
    d.rectangle((0, 0, w, 70), fill=(250, 250, 252))
    d.text((40, 16), "Galería", font=font(30, "semibold"), fill=(30, 30, 34))
    for r in range(2):
        for c in range(4):
            g = gradient(400, 380, [(0, (255, 200 - 30 * c, 140 + 20 * r)), (0.6, (200 - 30 * c, 120, 160 + 20 * c)),
                                    (1, (60, 60 + 30 * r, 110))], 60 + 25 * c)
            im.paste(g, (60 + c * 460, 110 + r * 470))
    return im


def iphone_trackpad(w=1170, h=2532):
    im = Image.new("RGB", (w, h), (12, 12, 14))
    d = ImageDraw.Draw(im, "RGBA")
    g = gradient(w - 80, 700, [(0, (40, 80, 200)), (1, (129, 151, 255))], 30)
    im.paste(g, (40, 220))
    d = ImageDraw.Draw(im, "RGBA")
    d.text((70, 120), "PC de escritorio", font=font(56, "semibold"), fill=(255, 255, 255))
    rr(d, (40, 980, w - 40, 2140), 48, fill=(255, 255, 255, 16), outline=(255, 255, 255, 30), width=3)
    d.text((w // 2 - 120, 1520), "Trackpad", font=font(52), fill=(255, 255, 255, 90))
    rr(d, (40, 2190, w // 2 - 15, 2420), 40, fill=(255, 255, 255, 30))
    rr(d, (w // 2 + 15, 2190, w - 40, 2420), 40, fill=(255, 255, 255, 30))
    return im


def android_home(w=3840, h=2160):
    im = blobs(w, h, [(0.3, 0.4, 0.3, (40, 110, 70)), (0.7, 0.6, 0.35, (60, 160, 90)), (0.5, 0.9, 0.3, (20, 60, 50))], (10, 30, 24), 260)
    d = ImageDraw.Draw(im, "RGBA")
    d.text((180, 140), "10:24", font=font(260, "light"), fill=(255, 255, 255))
    d.text((190, 440), "Sala de juntas", font=font(80), fill=(220, 240, 225))
    cols = [(108, 203, 95), (129, 151, 255), (255, 180, 80), (255, 255, 255), (90, 170, 255), (167, 139, 250)]
    for r in range(2):
        for c in range(6):
            x0 = 1500 + c * 360
            y0 = 900 + r * 420
            rr(d, (x0, y0, x0 + 240, y0 + 240), 60, fill=cols[(r * 6 + c) % 6] + (230,))
            rr(d, (x0 - 10, y0 + 270, x0 + 250, y0 + 300), 10, fill=(255, 255, 255, 120))
    return im


def mac_code(w=1920, h=1080):
    """El Mac pide aceptar la conexión y muestra el mismo código que el otro equipo."""
    im = desk_mac(None)
    sh = Image.new("RGBA", im.size, (0, 0, 0, 0))
    ImageDraw.Draw(sh).rounded_rectangle((560, 300, 1360, 800), 34, fill=(0, 0, 0, 140))
    sh = sh.filter(ImageFilter.GaussianBlur(30))
    im.paste(sh, (0, 0), sh)
    d = ImageDraw.Draw(im, "RGBA")
    rr(d, (560, 280, 1360, 780), 34, fill=(36, 36, 40, 255))
    d.text((960, 360), "PC de escritorio quiere conectarse", font=font(38, "semibold"), fill=(255, 255, 255), anchor="mm")
    d.text((960, 420), "Comprueba que el código coincide", font=font(28), fill=(170, 170, 175), anchor="mm")
    d.text((960, 540), "482 913", font=font(120, "semibold"), fill=(129, 151, 255), anchor="mm")
    rr(d, (620, 660, 940, 740), 18, fill=(255, 255, 255, 30))
    d.text((780, 700), "Rechazar", font=font(30), fill=(255, 153, 164), anchor="mm")
    rr(d, (980, 660, 1300, 740), 18, fill=(129, 151, 255, 255))
    d.text((1140, 700), "Aceptar", font=font(30, "semibold"), fill=(11, 11, 20), anchor="mm")
    return im


def icon(kind, s=256):
    """Iconos de dispositivo de línea blanca (como los de la interfaz de Kast), con transparencia."""
    im = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    W = 14
    if kind == "monitor":
        d.rounded_rectangle((30, 46, 226, 176), 18, outline=(255, 255, 255), width=W)
        d.line((96, 214, 160, 214), fill=(255, 255, 255), width=W)
        d.line((128, 176, 128, 214), fill=(255, 255, 255), width=W)
    elif kind == "box":
        d.rounded_rectangle((34, 92, 222, 170), 22, outline=(255, 255, 255), width=W)
        d.ellipse((168, 124, 182, 138), fill=(255, 255, 255))
    elif kind == "tablet":
        d.rounded_rectangle((58, 28, 198, 228), 22, outline=(255, 255, 255), width=W)
        d.line((112, 196, 144, 196), fill=(255, 255, 255), width=W)
    elif kind == "display":
        d.rounded_rectangle((20, 40, 236, 170), 12, outline=(255, 255, 255), width=W)
        d.line((70, 170, 50, 226), fill=(255, 255, 255), width=W)
        d.line((186, 170, 206, 226), fill=(255, 255, 255), width=W)
    return im


def main():
    os.makedirs(OUT, exist_ok=True)
    items = {
        "win_code": lambda: desk_win("code"), "win_sheet": lambda: desk_win("sheet"), "win_photo": lambda: desk_win("photo"),
        "win_plain": lambda: desk_win(None), "mac_notes": lambda: desk_mac("notes"), "mac_photos": lambda: desk_mac("photos"),
        "mac_plain": lambda: desk_mac(None), "kast_ui": kast_ui, "calib": calib_screen, "iphone": iphone_trackpad,
        "android": android_home, "mac_code": mac_code, "kast_ui_bg": lambda: kast_ui(cards=False),
        "ico_monitor": lambda: icon("monitor"), "ico_box": lambda: icon("box"), "ico_tablet": lambda: icon("tablet"),
        "ico_display": lambda: icon("display"),
    }
    for k, fn in items.items():
        fn().save(os.path.join(OUT, k + ".png"))
        print(k)


if __name__ == "__main__":
    main()
