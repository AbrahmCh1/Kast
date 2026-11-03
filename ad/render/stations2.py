"""Escenas 02 a 10. Mismo mundo, misma cámara y mismo cursor (una instancia por escena, que entra y sale de cuadro)."""
import math
import random
import bpy
from mathutils import Vector, Euler
from . import kit, assets
from .kit import ramp, smooth, smoother, ease_in, ease_out, ease_in_out, back_out, spring, impulse, expo_out, lerp, clamp
from .assets import LAVENDER, BLUE, VIOLET, GREEN, M
from .stations import light_line, line_window, curve_point, head_light, tile_rise, CORE
import ad.timeline as T

TEX = "ad/out/tex/"


def tex(name):
    import os
    return os.path.abspath(TEX + name + ".png")


def vis_scale(t0, t1, s=1.0):
    """Escala 1 dentro de [t0, t1] y 0 fuera (para objetos que sólo existen en su escena, fuera de cuadro)."""
    return lambda t: (s, s, s) if t0 <= t <= t1 else (0.0, 0.0, 0.0)


def make_cursor(name, baker, pos_fn, rot_fn=None, stem_fn=None, squash_fn=None, life=(0, 99), rim=0.8, scale_fn=None):
    c = assets.cursor(name)
    r = c["root"]
    r.rotation_mode = "QUATERNION"
    baker.add(r, "location", lambda t: tuple(pos_fn(t)))
    if rot_fn:
        baker.add(r, "rotation_quaternion", lambda t: (lambda q: (q.w, q.x, q.y, q.z))(rot_fn(t).to_quaternion()))
    t0, t1 = life
    if scale_fn:
        baker.add(r, "scale", lambda t: (lambda s: (s, s, s))(scale_fn(t) if t0 <= t <= t1 else 0.0))
    else:
        baker.add(r, "scale", vis_scale(t0, t1), const=True)
    assets.cursor_shape(c, baker, stem_fn or (lambda t: 0.0), squash_fn or (lambda t: 0.0))
    baker.add(kit.mat_input(c["edge_mat"], "Emission Strength"), "default_value", lambda t: rim)
    return c


def click_squash(times):
    """Aplastamiento del cursor en cada clic: anticipación breve, compresión y rebote."""
    def f(t):
        s = 0.0
        for tc in times:
            s += 0.35 * smooth(ramp(t, tc - 0.12, tc - 0.04)) * (1 - smooth(ramp(t, tc - 0.04, tc)))
            if t >= tc:
                s += 0.7 * math.exp(-12 * (t - tc)) * max(0.0, math.cos(2 * math.pi * 2.2 * (t - tc)))
        return s
    return f


def stem_follow(pos_fn, k=0.9):
    """El tallo se retrasa según la aceleración lateral (diferencia de velocidades)."""
    def f(t):
        dt = 1 / 60
        v0 = (Vector(pos_fn(t)) - Vector(pos_fn(t - dt))) / dt
        v1 = (Vector(pos_fn(t - dt)) - Vector(pos_fn(t - 2 * dt))) / dt
        a = (v0 - v1).x / dt
        return clamp(-a * 0.004 * k, -1, 1)
    return f


def path_fn(keys):
    """Trayectoria por claves (t, (x,y,z)) con velocidad continua."""
    h = kit.hermite_track(keys)
    return lambda t: Vector(h(t))


# ================================================================ ESCENA 02 — EL UNIVERSO DE TUS PANTALLAS

BUTTON = Vector((0.0, 14.6, 0.72))
PANEL = Vector((0.0, 14.0, 0.95))


def scene2(baker, rig, aspect, ctx):
    kit.into(kit.collection("S2_Universo"))
    rnd = random.Random(7)
    s = ctx["s1"]["logo"]["s"]
    start = Vector((0.0, 0.95 - 0.004, 1.20 + (32 - 49) * s - 0.035))
    pts = [start, start + Vector((0.0, 0.25, -0.30)), Vector((0.15, 2.2, 0.40)), Vector((0.75, 4.4, 0.36)),
           Vector((-0.55, 6.8, 0.42)), Vector((0.65, 9.2, 0.38)), Vector((-0.35, 11.4, 0.48)), Vector((0.12, 13.2, 0.62)),
           BUTTON + Vector((0.0, -0.06, 0.0))]
    ln = light_line("S2Linea", [tuple(p) for p in pts], strength=8.0)
    t_go, t_hit = 1.42, T.PRESS

    def head(t):
        u = ramp(t, t_go, t_hit)
        return 0.5 * ease_in(u, 1.8) + 0.5 * u * u * (3 - 2 * u) * 0 + 0.5 * u

    line_window(baker, ln, lambda t: max(0.0, head(t) - 0.22) * (1 if t < t_hit else 1) + (smooth(ramp(t, t_hit, t_hit + 0.25)) * (1 - max(0.0, head(t) - 0.22))),
                head)
    head_light("S2LuzCabeza", baker, lambda t: curve_point(ln["core"], head(t)),
               lambda t: 2.5 * smooth(ramp(t, t_go, t_go + 0.1)) * (1 - smooth(ramp(t, t_hit, t_hit + 0.2))), radius=0.08)

    # Pantallas a lo largo del camino: se encienden cuando pasa la cabeza.
    imgs = ["win_code", "win_sheet", "win_photo", "mac_notes", "mac_photos", "win_plain", "mac_plain"]
    k = 0
    for i in range(26):
        u = 0.08 + 0.86 * i / 25
        p = curve_point(ln["core"], u)
        side = 1 if i % 2 == 0 else -1
        off = rnd.uniform(0.75, 2.6)
        w = rnd.choice([0.52, 0.62, 0.62, 0.72, 0.9])
        h = w * 9 / 16
        pos = Vector((p.x + side * off, p.y + rnd.uniform(-0.3, 0.3), 0.0))
        if (pos - BUTTON).length < 1.6 or pos.y < 1.8:
            continue
        # La cámara vuela por x≈0: ninguna pantalla en su pasillo (la atravesaría de lleno).
        if abs(pos.x) < w / 2 + 0.3:
            pos.x = math.copysign(w / 2 + 0.3, pos.x)
        mon = assets.monitor(f"S2Monitor{k}", w, h, tex(imgs[k % len(imgs)]), stand=True)
        mon["root"].location = pos
        yaw = math.atan2(-(p.x - pos.x) * 0.6, 1.0) + rnd.uniform(-0.15, 0.15)
        mon["root"].rotation_euler = (0, 0, -yaw * 0.9)
        # Llega la cabeza: parpadeo de la retroiluminación, LED ámbar -> blanco, luz al entorno.
        t_on = t_go + (t_hit - t_go) * (u + 0.02)
        em = kit.mat_input(mon["screen_mat"], "Emission Strength")
        baker.add(em, "default_value", (lambda t_on=t_on: (lambda t: 0.0 if t < t_on else
                  0.9 * (0.35 + 0.65 * smooth(ramp(t, t_on + 0.05, t_on + 0.22))) * (0.6 + 0.4 * abs(math.cos(55 * (t - t_on)))) ** (1 if t < t_on + 0.12 else 0)))())
        led = kit.mat_input(mon["led"].data.materials[0], "Strength")
        baker.add(led, "default_value", (lambda t_on=t_on: (lambda t: 2.0 if t < t_on else 3.0))())
        baker.add(mon["led"].data.materials[0].node_tree.nodes["Emission"].inputs["Color"], "default_value",
                  (lambda t_on=t_on: (lambda t: kit.srgb("#FFB347") if t < t_on else kit.srgb("#FFFFFF")))(), const=True)
        k += 1
    # Pantallas lejanas apagadas (geometría de fondo, profundidad).
    for i in range(16):
        pos = Vector((rnd.uniform(-9, 9), rnd.uniform(6, 30), 0.0))
        if abs(pos.x) < 3.5:
            pos.x += 4.0 * (1 if pos.x >= 0 else -1)
        mon = assets.monitor(f"S2Lejano{i}", 0.7, 0.4, None, stand=True)
        mon["root"].location = pos
        mon["root"].rotation_euler = (0, 0, rnd.uniform(-0.6, 0.6))

    # El icono de Kast como botón físico sobre un pedestal oscuro.
    ped = kit.box("S2Pedestal", (1.04, 0.36, 0.27), radius=0.03, loc=(BUTTON.x, BUTTON.y + 0.1, 0.135), mats=M("graphite"))
    plinth = kit.slab("S2Zocalo", 1.04, 1.04, 0.10, 0.25, bevel=0.012, mats=M("graphite"))
    plinth.location = BUTTON + Vector((0, 0.11, 0))
    lg = assets.logo("S2Boton", 0.9)
    lg["root"].location = BUTTON

    def press(t):
        d = 0.05 * smooth(ramp(t, t_hit - 0.03, t_hit)) * (1 - smooth(ramp(t, t_hit + 0.06, t_hit + 0.35)))
        d += 0.012 * impulse(t - t_hit - 0.2, 3.0, 7.0)
        return d

    def sink(t):
        return -1.2 * ease_in(ramp(t, t_hit + 0.10, t_hit + 0.42), 2.2)

    baker.add(lg["root"], "location", lambda t: tuple(BUTTON + Vector((0, press(t), sink(t)))))
    baker.add(plinth, "location", lambda t: tuple(BUTTON + Vector((0, 0.11, sink(t)))))
    baker.add(ped, "location", lambda t: (BUTTON.x, BUTTON.y + 0.1, 0.135 + sink(t)))
    baker.add(lg["root"], "scale", lambda t: (1.0, 1.0 - 4 * press(t), 1.0))
    ring_m = kit.emission_mat("S2OndaM", LAVENDER, 0.0)
    ring = kit.curve("S2Onda", kit.rounded_rect_points(0.95, 0.95, 0.24, 10) and [(x, 0, z) for x, z in kit.rounded_rect_points(0.95, 0.95, 0.24, 10)],
                     bevel=0.003, res=2, closed=True, smooth=False, mats=ring_m)
    ring.location = BUTTON + Vector((0, 0.02, 0))
    baker.add(ring, "scale", lambda t: (lambda r: (r, 1, r))(1.0 + 0.9 * expo_out(ramp(t, t_hit, t_hit + 0.6))))
    baker.add(kit.mat_input(ring_m, "Strength"), "default_value", lambda t: 4 * smooth(ramp(t, t_hit, t_hit + 0.03)) * (1 - smooth(ramp(t, t_hit + 0.05, t_hit + 0.55))))
    kit.area_light("S2Clave", (BUTTON.x - 1.8, BUTTON.y - 1.6, 2.4), tuple(BUTTON), 120, size=1.8, color="#F4F1EA")
    kit.area_light("S2Recorte", (BUTTON.x + 1.6, BUTTON.y + 1.4, 1.8), tuple(BUTTON), 90, size=1.2, color="#C9D1FF")

    # La ventana de Kast se despliega desde el botón.
    t_u0, t_u1 = t_hit + 0.08, t_hit + 0.40
    panel = kit.slab("S2Ventana", 2.4, 1.5, 0.035, 0.06, bevel=0.008, mats=M("card"))
    face = kit.plane("S2VentanaCara", 2.38, 1.48)
    face.parent = panel
    face.location = (0, -0.0185, 0)
    fm, _, fem = kit.screen_mat_tint("S2VentanaCaraM", tex("kast_ui_bg"), 0.0)
    kit.assign(face, fm)
    baker.add(fem, "default_value", lambda t: 0.85 * smooth(ramp(t, t_u0, t_u0 + 0.18)))

    def panel_loc(t):
        u = back_out(ramp(t, t_u0, t_u1), 1.15)
        start = BUTTON + Vector((0, 0.35, -0.2))
        return tuple(start.lerp(PANEL, u) + Vector((0, 0, 0.08 * math.sin(math.pi * clamp(u)))))

    def panel_scale(t):
        if t < t_u0:
            return (0.0, 0.0, 0.0)
        u = back_out(ramp(t, t_u0, t_u1), 1.3)
        sx = lerp(0.36, 1.0, u)
        return (sx, 1.0, lerp(0.58, 1.0, u))

    panel.rotation_mode = "XYZ"
    baker.add(panel, "location", panel_loc)
    baker.add(panel, "scale", panel_scale)
    baker.add(panel, "rotation_euler", lambda t: (math.radians(-18) * (1 - ease_out(ramp(t, t_u0, t_u1 + 0.1), 2)), 0.0, 0.0))

    # Cámara: baja bajo el logo, sigue la cabeza entre las pantallas y llega al botón.
    rig.key(1.62, pos=(0.05, -1.35, 1.08), target=(0.05, 2.4, 0.62), focal=40, fstop=3.5, roll=math.radians(0.8))
    rig.key(1.86, pos=(0.10, 0.55, 0.62), target=(0.30, 4.6, 0.42), focal=32, fstop=4.0, roll=math.radians(-2.5))
    rig.key(2.10, pos=(-0.15, 4.2, 0.55), target=(0.35, 9.0, 0.44), focal=30, fstop=4.0, roll=math.radians(2.0))
    rig.key(2.32, pos=(0.20, 8.6, 0.62), target=(0.05, 13.0, 0.62), focal=32, fstop=4.0, roll=math.radians(-1.0))
    rig.key(2.50, pos=(0.04, 12.05, 0.80), target=(0.0, BUTTON.y, 0.74), focal=35, fstop=4.0, roll=0.0)
    rig.shake(t_hit, (0, 1, 0), amp=0.008, freq=5.0, damp=8.0)
    return {"panel": panel}


# ================================================================ ESCENA 03 — TUS MONITORES, IGUALES

def scene3(baker, rig, aspect, ctx):
    kit.into(kit.collection("S3_Calibracion"))
    lift0, lift1 = 2.90, 3.22
    cards = []
    values = [(40, 62), (85, 81), (20, 70)]
    tints0 = ["#FFD9B0", "#C8D8FF", "#7A7A80"]
    for i in range(3):
        x = -0.56 + 0.56 * i
        depth = [0.30, 0.38, 0.32][i]
        card = kit.slab(f"S3Tarjeta{i}", 0.5, 0.74, 0.03, 0.05, bevel=0.006, mats=M("card_light"))
        root = kit.empty(f"S3Raiz{i}")
        card.parent = root
        scr = kit.plane(f"S3Pantalla{i}", 0.44, 0.2475)
        scr.parent = root
        scr.location = (0, -0.0165, 0.195)
        sm, tint, sem = kit.screen_mat_tint(f"S3PantallaM{i}", tex("calib"), 0.0)
        kit.assign(scr, sm)
        name = kit.text(f"S3Nombre{i}", f"Monitor {i + 1}", size=0.045, align="LEFT", mats=M("white_text"))
        name.parent = root
        name.location = (-0.22, -0.017, 0.02)
        sub = kit.text(f"S3Sub{i}", ["27\" · 4K", "27\" · 144 Hz", "24\" · Full HD"][i], font="regular", size=0.028, align="LEFT",
                       mats=M("sub_text"))
        sub.parent = root
        sub.location = (-0.22, -0.017, -0.035)
        lab = kit.text(f"S3Brillo{i}", "Brillo", font="regular", size=0.032, align="LEFT", mats=M("sub_text"))
        lab.parent = root
        lab.location = (-0.22, -0.017, -0.13)
        rail = kit.box(f"S3Riel{i}", (0.40, 0.008, 0.012), radius=0.004, mats=kit.mat(f"S3RielM{i}", "#5A5A62", rough=0.2, coat=1.0))
        rail.parent = root
        rail.location = (0, -0.02, -0.21)
        fill = kit.box(f"S3Relleno{i}", (0.40, 0.009, 0.013), radius=0.004, mats=kit.emission_mat(f"S3RellenoM{i}", LAVENDER, 1.6))
        fill.parent = root
        knob = kit.box(f"S3Mando{i}", (0.034, 0.03, 0.034), radius=0.0165, mats=M("ceramic"))
        knob.parent = root
        num = kit.text(f"S3Valor{i}", "", size=0.034, align="RIGHT", mats=M("white_text"))
        num.parent = root
        num.location = (0.22, -0.017, -0.13)
        v0, v1 = values[i]
        drag0, drag1 = 3.34, 3.98

        def val(t, v0=v0, v1=v1):
            return lerp(v0, v1, ease_in_out(ramp(t, drag0 + 0.04 * i, drag1 + 0.04 * i), 2.4))

        def fill_scale(t, val=val):
            return (max(1e-3, val(t) / 100), 1.0, 1.0)

        def fill_loc(t, val=val):
            return (-0.20 + 0.20 * val(t) / 100, -0.021, -0.21)

        def knob_loc(t, val=val):
            return (-0.20 + 0.40 * val(t) / 100, -0.03, -0.21)

        baker.add(fill, "scale", fill_scale)
        baker.add(fill, "location", fill_loc)
        baker.add(knob, "location", knob_loc)
        # Valor en número: un objeto de texto por valor entero, visible sólo en su tramo.
        # (sustituido por la barra para no recargar; el número final aparece al terminar)
        num.data.body = f"{v1}"
        baker.add(num, "scale", lambda t: (lambda k: (k, k, k))(smooth(ramp(t, drag1, drag1 + 0.15))))
        t0c = kit.srgb(tints0[i])

        def tint_fn(t, t0c=t0c, i=i):
            u = smooth(ramp(t, drag0 + 0.1, T.MATCH))
            c = [lerp(t0c[j], kit.srgb("#F2F2F2")[j], u) for j in range(3)]
            return (*c, 1.0)

        baker.add(tint, "default_value", tint_fn)
        baker.add(sem, "default_value", lambda t: 1.1 * smooth(ramp(t, lift0, lift0 + 0.25)))
        target = PANEL + Vector((x, -depth, -0.02))
        start = PANEL + Vector((x, -0.04, -0.02))

        def rloc(t, target=target, start=start, i=i):
            u = back_out(ramp(t, lift0 + 0.04 * i, lift1 + 0.04 * i), 1.25)
            p = start.lerp(target, u)
            p.z += 0.006 * kit.noise1(t * 0.7, i)
            return tuple(p)

        baker.add(root, "location", rloc)
        baker.add(root, "scale", lambda t: (0.0, 0.0, 0.0) if t < 2.86 else (1.0, 1.0, 1.0), const=True)
        cards.append({"root": root, "knob": knob, "val": val})

    # Barra general «Brillo · todas» con la que tira el cursor.
    master = kit.empty("S3General")
    master.location = PANEL + Vector((0, -0.42, -0.53))
    mt = kit.text("S3GeneralTit", "Brillo · todas", size=0.04, align="LEFT", mats=M("white_text"))
    mt.parent = master
    mt.location = (-0.62, 0, 0.055)
    mrail = kit.box("S3GeneralRiel", (1.24, 0.01, 0.016), radius=0.006, mats=kit.mat("S3GeneralRielM", "#5A5A62", rough=0.2, coat=1.0))
    mrail.parent = master
    mfill = kit.box("S3GeneralRelleno", (1.24, 0.011, 0.017), radius=0.006, mats=kit.emission_mat("S3GeneralRellenoM", LAVENDER, 2.0))
    mfill.parent = master
    mknob = kit.box("S3GeneralMando", (0.05, 0.04, 0.05), radius=0.024, mats=M("ceramic"))
    mknob.parent = master
    drag0, drag1 = 3.34, 3.98

    def mval(t):
        return lerp(45, 70, ease_in_out(ramp(t, drag0, drag1), 2.4)) + 1.2 * impulse(t - drag1, 3.0, 8.0)

    baker.add(mfill, "scale", lambda t: (mval(t) / 100, 1.0, 1.0))
    baker.add(mfill, "location", lambda t: (-0.62 + 0.62 * mval(t) / 100, -0.001, 0))
    baker.add(mknob, "location", lambda t: (-0.62 + 1.24 * mval(t) / 100, -0.012, 0))
    baker.add(mknob, "scale", lambda t: (1.0, 1.0 - 0.25 * smooth(ramp(t, drag0 - 0.06, drag0)) * (1 - smooth(ramp(t, drag1, drag1 + 0.12))), 1.0))
    baker.add(master, "scale", lambda t: (lambda k: (k, k, k))(0.0 if t < 2.9 else back_out(ramp(t, 2.92, 3.18), 1.4)))
    msg = kit.text("S3Mensaje", "El mismo 70 % en los tres", size=0.05, mats=M("white_text"))
    msg.location = PANEL + Vector((0, -0.46, -0.69))
    baker.add(msg, "scale", lambda t: (lambda k: (k, k, k))(back_out(ramp(t, T.MATCH + 0.05, T.MATCH + 0.3), 1.6)))

    # El cursor entra, toma el mando, lo arrastra y lo suelta.
    mk0 = master.location + Vector((-0.62 + 1.24 * 0.45, -0.035, 0.0))
    mk1 = master.location + Vector((-0.62 + 1.24 * 0.70, -0.035, 0.0))
    keys = [(2.95, mk0 + Vector((-1.2, -0.3, 0.55))), (3.25, mk0 + Vector((0.01, -0.02, 0.012))), (drag0, mk0 + Vector((0.0, -0.005, 0.0))),
            (drag1, mk1 + Vector((0.0, -0.005, 0.0))), (drag1 + 0.12, mk1 + Vector((0.0, -0.03, 0.02))),
            (4.40, mk1 + Vector((0.12, -0.08, 0.10))), (4.85, mk1 + Vector((1.4, -0.3, 0.55)))]
    cp = path_fn(keys)
    make_cursor("S3Cursor", baker, cp, rot_fn=lambda t: Euler((0, 0, math.radians(8 * kit.noise1(t, 3)))),
                stem_fn=stem_follow(cp), squash_fn=click_squash([drag0]), life=(2.9, 4.9))

    # Confirmación: los bordes de las tarjetas destellan y la línea llena la barra y escapa por su extremo.
    end = master.location + Vector((-0.62 + 1.24 * 0.70, -0.02, 0))
    first_rect = S4 + Vector((-0.68 - 0.31, -0.03, 0.16 + 0.35 + 0.02))
    # Vuela bajo, sin tocar el suelo: a ras de él quedaba fuera del cuadro mientras la cámara gira hacia las disposiciones.
    out = [master.location + Vector((-0.62, -0.02, 0)), end, end + Vector((0.35, -0.10, -0.15)), end + Vector((1.2, 0.2, -0.05)),
           Vector((2.4, 15.4, 0.30)), first_rect + Vector((-0.2, -0.3, 0.0)), first_rect]
    ln = light_line("S3Linea", [tuple(p) for p in out], strength=8.0)
    t_f0, t_f1, t_out = T.CONFIRM1 - 0.18, T.CONFIRM1, T.IGNITION - 0.22
    total = 0.0
    seg = [(out[i + 1] - out[i]).length for i in range(len(out) - 1)]
    total = sum(seg)
    u_end = seg[0] / total

    def head(t):
        if t < t_f1:
            return u_end * ease_in_out(ramp(t, t_f0, t_f1), 2)
        return u_end + (1 - u_end) * ease_in(ramp(t, t_f1 + 0.05, t_out), 1.6)

    line_window(baker, ln, lambda t: max(0.0, head(t) - 0.18) if t > t_f1 + 0.05 else 0.0, head)
    for c in cards:
        pass
    kit.area_light("S3Clave", tuple(PANEL + Vector((-1.4, -2.0, 1.4))), tuple(PANEL + Vector((0, -0.4, 0))), 60, size=2.0, color="#F4F1EA")
    kit.area_light("S3Recorte", tuple(PANEL + Vector((1.6, -0.2, 1.6))), tuple(PANEL + Vector((0, -0.4, 0))), 40, size=1.0, color="#C9D1FF")

    # Cámara: retrocede para ver la ventana desplegarse, entra y se queda frente a las tarjetas (calma).
    rig.key(2.70, pos=(0.02, 11.55, 0.98), target=(0.0, PANEL.y, 0.92), focal=30, fstop=5.0, roll=0.0)
    rig.key(3.10, pos=(0.0, 12.02, 0.84), target=(0.0, PANEL.y - 0.38, 0.79), focal=27, fstop=5.6, roll=0.0)
    rig.key(4.50, pos=(0.03, 12.10, 0.82), target=(0.02, PANEL.y - 0.38, 0.77), focal=27.5, fstop=5.6, roll=math.radians(0.4))
    rig.key(4.72, pos=(0.30, 12.30, 0.74), target=(0.95, 13.75, 0.38), focal=28, fstop=5.0, roll=math.radians(-1.0))
    rig.key(4.86, pos=(1.15, 13.05, 0.80), target=(2.30, 15.25, 0.32), focal=29, fstop=5.0, roll=math.radians(-1.8))
    return {}


# ================================================================ ESCENA 04 — UNA DISPOSICIÓN PARA CADA MOMENTO

S4 = Vector((4.0, 17.6, 0.0))


def scene4(baker, rig, aspect, ctx):
    kit.into(kit.collection("S4_Disposiciones"))
    t_ig = T.IGNITION
    mons = []
    xs = [-0.68, 0.0, 0.68]
    imgs = ["win_code", "win_photo", "win_sheet"]
    # Disposiciones: (x, y, z de la cabeza extra, giro, vertical) por monitor
    layouts = {
        "trabajo": [(-0.68, 0, 0, 0.0, 0), (0.0, 0, 0, 0.0, 0), (0.68, 0, 0, 0.0, 0)],
        "vertical": [(-0.66, 0, 0, 0.0, 0), (0.0, 0, 0, 0.0, 0), (0.58, 0.02, 0.05, 0.0, 1)],
        "juego": [(-0.62, -0.12, 0, 0.32, 0), (0.0, 0.05, 0.02, 0.0, 0), (0.62, -0.12, 0, -0.32, 0)],
        "presentacion": [(-0.80, 0.25, 0, 0.12, 0), (0.0, -0.08, 0.05, 0.0, 0), (0.80, 0.25, 0, -0.12, 0)],
        "cine": [(-0.78, 0.4, 0, 0.2, 0), (0.0, -0.12, 0.06, 0.0, 0), (0.78, 0.4, 0, -0.2, 0)],
    }
    seq = [(t_ig, "trabajo"), (5.62, "vertical"), (6.18, "juego"), (6.40, "presentacion"), (6.62, "cine"), (6.84, "trabajo")]

    def pose(i, t):
        cur = layouts[seq[0][1]][i]
        for (ts, name), nxt in zip(seq, seq[1:] + [(99, None)]):
            if t >= ts:
                cur = layouts[name][i]
        # Transición con muelle desde la disposición anterior.
        prev = None
        tsw = None
        for k in range(1, len(seq)):
            if t >= seq[k][0]:
                prev, tsw = layouts[seq[k - 1][1]][i], seq[k][0]
        if prev is None:
            return cur
        u = spring(t - tsw, 2.4, 7.5) if t - tsw < 2 else 1.0
        return tuple(lerp(a, b, u) for a, b in zip(prev, cur))

    for i in range(3):
        mon = assets.monitor(f"S4Monitor{i}", 0.62, 0.35, tex(imgs[i]), stand=True)
        root, head = mon["root"], mon["head"]
        rise0 = t_ig - 0.05 + 0.04 * i

        def rloc(t, i=i, rise0=rise0):
            x, y, z, yaw, port = pose(i, t)
            under = -0.85 * (1 - spring(t - rise0, 2.0, 7.0)) if t >= rise0 else -0.85
            return tuple(S4 + Vector((x, y, under)))

        def rrot(t, i=i):
            return (0.0, 0.0, pose(i, t)[3])

        def hrot(t, i=i):
            port = pose(i, t)[4]
            return (0.0, math.radians(90) * port, 0.0)

        def hloc(t, i=i):
            port = pose(i, t)[4]
            return (0.0, 0.0, 0.16 + 0.35 / 2 + 0.02 + port * (0.62 - 0.35) / 2 + pose(i, t)[2])

        baker.add(root, "location", rloc)
        baker.add(root, "rotation_euler", rrot)
        baker.add(head, "rotation_euler", hrot)
        baker.add(head, "location", hloc)
        em = kit.mat_input(mon["screen_mat"], "Emission Strength")
        baker.add(em, "default_value", (lambda i=i: (lambda t: 0.0 if t < t_ig + 0.1 * i else
                  1.0 * smooth(ramp(t, t_ig + 0.1 * i, t_ig + 0.1 * i + 0.25)) * (0.75 + 0.25 * abs(math.cos(40 * (t - t_ig)))) ** (1 if t < t_ig + 0.25 else 0)))())
        baker.add(kit.mat_input(mon["led"].data.materials[0], "Strength"), "default_value", lambda t: 3.0 if t > t_ig else 0.0)
        mons.append(mon)

    # La línea llega en espiral y dibuja tres rectángulos donde estarán las pantallas.
    for i, x in enumerate(xs):
        rr = [(S4.x + x + px, S4.y - 0.03, 0.16 + 0.35 / 2 + 0.02 + pz) for px, pz in kit.rounded_rect_points(0.62, 0.35, 0.012, 4)]
        rl = light_line(f"S4Rect{i}", rr + [rr[0]], core=0.0028, halo=0.009, strength=7.0, smooth_curve=False)
        a, b = t_ig - 0.22 + 0.04 * i, t_ig + 0.02 + 0.04 * i
        line_window(baker, rl, lambda t, b=b: smooth(ramp(t, b + 0.05, b + 0.3)), lambda t, a=a, b=b: ease_in_out(ramp(t, a, b), 2))

    # Contador de resolución y frecuencia, en celdas de ancho fijo.
    kit.flip_counter("S4Resolucion", baker, [(0.0, "1920 × 1080"), (5.52, "2560 × 1440"), (5.82, "3840 × 2160")], 0.09,
                     M("white_text"), tuple(S4 + Vector((0.0, -0.05, 1.10))), cell=0.09 * 0.53)
    kit.flip_counter("S4Hz", baker, [(0.0, " 60"), (5.98, "144")], 0.052, M("sub_text"), tuple(S4 + Vector((-0.05, -0.05, 0.98))),
                     cell=0.052 * 0.55)
    hz = kit.text("S4HzEtiqueta", "Hz", font="semibold", size=0.052, align="LEFT", mats=M("sub_text"))
    hz.location = S4 + Vector((0.04, -0.05, 0.98))
    baker.add(hz, "scale", lambda t: (lambda k: (k, k, k))(back_out(ramp(t, t_ig + 0.12, t_ig + 0.4), 1.5)))
    for o in bpy.data.objects:
        if o.name.startswith(("S4Resolucion", "S4Hz")) and o.type == "EMPTY":
            baker.add(o, "scale", lambda t: (lambda k: (k, k, k))(back_out(ramp(t, t_ig + 0.12, t_ig + 0.4), 1.5)))

    # Fichas de las disposiciones guardadas.
    names = [("Trabajo", 6.18 - 0.22), ("Juego", 6.18), ("Presentación", 6.40), ("Cine", 6.62)]
    x0 = -0.75
    for k, (lab, ton) in enumerate(names):
        w = 0.07 + 0.026 * len(lab)
        chip = kit.slab(f"S4Ficha{k}", w, 0.075, 0.02, 0.037, bevel=0.004, mats=M("card"))
        chip.location = S4 + Vector((x0 + w / 2, -0.55, 0.06))
        x0 += w + 0.05
        glow = kit.emission_mat(f"S4FichaLuzM{k}", LAVENDER, 0.0)
        edge = kit.curve(f"S4FichaLuz{k}", [(x, -0.0105, z) for x, z in kit.rounded_rect_points(w, 0.075, 0.037, 6)], bevel=0.0012,
                         res=2, closed=True, smooth=False, mats=glow)
        edge.parent = chip
        tx = kit.text(f"S4FichaTxt{k}", lab, size=0.032, mats=M("white_text"))
        tx.parent = chip
        tx.location = (0, -0.0115, -0.012)
        nxt = names[k + 1][1] if k + 1 < len(names) else 7.0
        baker.add(kit.mat_input(glow, "Strength"), "default_value",
                  lambda t, ton=ton, nxt=nxt: 4.0 * smooth(ramp(t, ton, ton + 0.05)) * (1 - 0.8 * smooth(ramp(t, nxt, nxt + 0.08))))
        baker.add(chip, "scale", lambda t, k=k: (lambda s: (s, s, s))(back_out(ramp(t, 5.9 + 0.05 * k, 6.15 + 0.05 * k), 1.6)))

    kit.area_light("S4Clave", tuple(S4 + Vector((-1.8, -2.0, 2.4))), tuple(S4 + Vector((0, 0, 0.5))), 110, size=2.0, color="#F4F1EA")
    kit.area_light("S4Recorte", tuple(S4 + Vector((1.5, 1.6, 1.6))), tuple(S4 + Vector((0, 0, 0.5))), 120, size=1.4, color="#C9D1FF")

    # Cámara: sigue la espiral y orbita un poco la disposición.
    rig.key(4.98, pos=tuple(S4 + Vector((-0.75, -2.65, 0.92))), target=tuple(S4 + Vector((-0.25, 0.0, 0.55))), focal=30, fstop=4.0,
            roll=math.radians(-1.5))
    c = S4 + Vector((0, 0, 0.5))
    for t, ang, dist, h, f in ((5.30, -26, 2.45, 0.80, 32), (6.05, -10, 2.30, 0.78, 33), (6.85, 8, 2.35, 0.86, 33)):
        a = math.radians(ang)
        rig.key(t, pos=tuple(c + Vector((dist * math.sin(a), -dist * math.cos(a), h - 0.5))), target=tuple(c + Vector((0, 0, 0.02))),
                focal=f, fstop=4.0, roll=math.radians(ang * 0.03))
    return {"mons": mons}


# ================================================================ ESCENAS 05 y 06 — EL OTRO EQUIPO / LA OTRA PANTALLA

LIST = Vector((4.0, 21.0, 0.0))
DESK = Vector((4.0, 22.4, 0.0))
MAC = Vector((6.5, 23.2, 0.0))


def scene5(baker, rig, aspect, ctx):
    kit.into(kit.collection("S5_ElOtroEquipo"))
    items = [("PC de escritorio", "Windows · 3 pantallas", "ico_monitor"), ("Mac mini", "macOS · 1 pantalla", "ico_box"),
             ("iPad", "iPadOS", "ico_tablet"), ("Pantalla de la sala", "Android · 86\"", "ico_display")]
    t_in = 6.92
    title = kit.text("S5Titulo", "DISPOSITIVOS", size=0.07, spacing=1.15, mats=M("sub_text"))
    title.location = LIST + Vector((0, -0.02, 1.92))
    baker.add(title, "scale", lambda t: (lambda k: (k, k, k))(back_out(ramp(t, t_in, t_in + 0.3), 1.4)) if t < 9.5 else (0, 0, 0))
    cards = []
    for k, (nm, sub, ico) in enumerate(items):
        root = kit.empty(f"S5Tarjeta{k}")
        card = kit.slab(f"S5TarjetaC{k}", 1.3, 0.25, 0.04, 0.05, bevel=0.006, mats=M("card_light"))
        card.parent = root
        icon, _ = kit.image_plane(f"S5Icono{k}", tex(ico), 0.11, 0.11, 2.2)
        icon.parent = root
        icon.location = (-0.53, -0.022, 0.0)
        t1 = kit.text(f"S5Nombre{k}", nm, size=0.052, align="LEFT", mats=M("white_text"))
        t1.parent = root
        t1.location = (-0.43, -0.022, 0.025)
        t2 = kit.text(f"S5Sub{k}", sub, font="regular", size=0.034, align="LEFT", mats=M("sub_text"))
        t2.parent = root
        t2.location = (-0.43, -0.022, -0.045)
        cards.append(root)
        base = LIST + Vector((0, 0, 1.62 - k * 0.31))
        mac = k == 1

        def loc(t, base=base, k=k, mac=mac):
            u = back_out(ramp(t, t_in + 0.06 * k, t_in + 0.35 + 0.06 * k), 1.3)
            p = base + Vector((0, 0.0, -0.6 * (1 - u)))
            if mac:
                r = smooth(ramp(t, 7.70, 8.05))
                p += Vector((0.0, -0.16 * r, 0.0))
            else:
                r = smooth(ramp(t, 7.74, 8.10))
                p += Vector((0.0, 0.06 * r, 0.0))
            # Se apartan para descubrir el escritorio (los de arriba a la izquierda, los de abajo a la derecha).
            g = ease_in(ramp(t, 9.48 + 0.04 * k, 9.85 + 0.04 * k), 2.2)
            p += Vector(((-1 if k % 2 == 0 else 1) * 3.0 * g, -0.3 * g, 0.4 * g))
            return tuple(p)

        baker.add(root, "location", loc)
        baker.add(root, "scale", lambda t, k=k: (lambda s: (s, s, s))(0.0 if t < t_in + 0.06 * k else (1.0 if t < 10.3 else 0.0)), const=True)
        baker.add(root, "rotation_euler", lambda t, k=k: (0.0, 0.0, (-1 if k % 2 == 0 else 1) * 0.5 * ease_in(ramp(t, 9.48, 9.9), 2)))
    # Botón «Conectar» y código en la tarjeta del Mac.
    mac_card = cards[1]
    btn = kit.slab("S5Conectar", 0.24, 0.085, 0.022, 0.042, bevel=0.004, mats=kit.mat("S5ConectarM", "#8197FF", rough=0.35, coat=0.5,
                                                                                       emit="#8197FF", strength=0.4))
    btn.parent = mac_card
    btn.location = (0.47, -0.03, 0.0)
    bt = kit.text("S5ConectarTxt", "Conectar", size=0.036, mats=kit.mat("S5ConectarTxtM", "#0B0B14", rough=0.5))
    bt.parent = btn
    bt.location = (0, -0.012, -0.012)
    tc = T.CONNECT_CLICK
    baker.add(btn, "location", lambda t: (0.47, -0.03 + 0.012 * smooth(ramp(t, tc - 0.03, tc)) * (1 - smooth(ramp(t, tc + 0.05, tc + 0.25))), 0.0))
    baker.add(btn, "scale", lambda t: (lambda k: (k, k, k))(1.0 if t < tc + 0.12 else max(0.0, 1 - ease_in(ramp(t, tc + 0.12, tc + 0.24), 2))))
    code_m = kit.mat("S5CodigoM", "#FFFFFF", rough=0.5, emit="#8197FF", strength=2.0)
    cnt = kit.flip_counter("S5Codigo", baker, [(0.0, "       "), (tc + 0.14, "482 913")], 0.075, code_m, (0, 0, 0), cell=0.04)
    cnt.parent = mac_card
    cnt.location = (0.40, -0.03, -0.025)
    baker.add(kit.mat_input(code_m, "Emission Color"), "default_value",
              lambda t: tuple(lerp(a, b, smooth(ramp(t, T.CONFIRM2, T.CONFIRM2 + 0.1))) for a, b in zip(kit.srgb(LAVENDER), kit.srgb(GREEN))))
    ok = kit.text("S5Cifrado", "Conectado · cifrado de extremo a extremo", font="regular", size=0.034, mats=kit.mat("S5CifradoM", "#6CCB5F", rough=0.5, emit="#6CCB5F", strength=1.6))
    ok.location = LIST + Vector((0, -0.2, 1.62 - 0.31 - 0.16))
    baker.add(ok, "scale", lambda t: (lambda k: (k, k, k))(back_out(ramp(t, T.CONFIRM2 + 0.05, T.CONFIRM2 + 0.3), 1.5) if t < 9.5 else 0.0))
    # Onda verde que recorre la lista.
    wave_m = kit.emission_mat("S5OndaM", GREEN, 0.0)
    wave = kit.box("S5Onda", (1.36, 0.005, 0.006), radius=0.002, mats=wave_m)
    baker.add(wave, "location", lambda t: tuple(LIST + Vector((0, -0.05, 1.75 - 1.25 * ease_in_out(ramp(t, T.CONFIRM2, T.CONFIRM2 + 0.4))))))
    baker.add(kit.mat_input(wave_m, "Strength"), "default_value",
              lambda t: 9 * smooth(ramp(t, T.CONFIRM2, T.CONFIRM2 + 0.05)) * (1 - smooth(ramp(t, T.CONFIRM2 + 0.3, T.CONFIRM2 + 0.45))))

    # El escritorio (PC con tres monitores) y el Mac, detrás.
    pc = []
    for i in range(3):
        mon = assets.monitor(f"S5PC{i}", 0.62, 0.35, None, stand=True)
        mon["root"].location = DESK + Vector((-0.68 + 0.68 * i, 0, 0))
        pc.append(mon)
    macmon = assets.monitor("S5MacMonitor", 0.62, 0.35, None, stand=True)
    macmon["root"].location = MAC
    macmon["root"].rotation_euler = (0, 0, math.radians(-14))
    mm = assets.mac_mini("S5MacMini")
    mm["root"].location = MAC + Vector((0.45, -0.15, 0))
    mm["root"].rotation_euler = (0, 0, math.radians(-14))
    scr = macmon["screen"]
    sm, wipe, sem = kit.screen_mat_wipe("S5MacPantallaM", tex("mac_code"), tex("mac_plain"), 0.0)
    kit.assign(scr, sm)
    baker.add(sem, "default_value", lambda t: 1.0 * smooth(ramp(t, 7.6, 7.9)))
    baker.add(wipe, "default_value", lambda t: 1.05 * ease_in_out(ramp(t, T.CONFIRM2 + 0.1, T.CONFIRM2 + 0.5)))
    kit.area_light("S5Clave", tuple(LIST + Vector((-1.6, -2.2, 2.6))), tuple(LIST + Vector((0, 0, 1.1))), 120, size=2.2, color="#F4F1EA")
    kit.area_light("S5Recorte", tuple(LIST + Vector((1.8, 0.6, 2.0))), tuple(LIST + Vector((0, 0, 1.1))), 70, size=1.4, color="#C9D1FF")

    # El cursor: llega por la derecha, pulsa Conectar y se queda mirando el código.
    btn_w = LIST + Vector((0.47, -0.16 - 0.04, 1.62 - 0.31))
    keys = [(7.80, btn_w + Vector((1.1, -0.2, -0.45))), (8.20, btn_w + Vector((0.03, -0.02, -0.02))), (tc, btn_w + Vector((0.0, -0.005, -0.012))),
            (8.75, btn_w + Vector((0.12, -0.06, -0.10))), (9.30, btn_w + Vector((0.16, -0.07, -0.12))), (9.75, btn_w + Vector((0.6, -0.1, -0.3)))]
    cp = path_fn(keys)
    make_cursor("S5Cursor", baker, cp, rot_fn=lambda t: Euler((0, 0, math.radians(-6 + 6 * kit.noise1(t, 5)))), stem_fn=stem_follow(cp),
                squash_fn=click_squash([tc]), life=(7.7, 9.9))

    # Cámara: sube sobre la disposición y desciende en vertical hasta la lista.
    rig.key(7.08, pos=tuple(LIST + Vector((0.05, -5.0, 1.75))), target=tuple(LIST + Vector((0.0, 0.0, 1.35))), focal=30, fstop=5.0, roll=0.0)
    rig.key(7.40, pos=tuple(LIST + Vector((0.1, -2.7, 2.45))), target=tuple(LIST + Vector((0.0, 0.2, 1.62))), focal=32, fstop=4.0, roll=0.0)
    rig.key(7.75, pos=tuple(LIST + Vector((0.05, -1.95, 1.55))), target=tuple(LIST + Vector((0.05, 0.0, 1.2))), focal=34, fstop=4.0, roll=0.0)
    rig.key(9.35, pos=tuple(LIST + Vector((0.18, -1.80, 1.30))), target=tuple(LIST + Vector((0.12, 0.0, 1.18))), focal=36, fstop=4.0,
            roll=math.radians(0.6))
    return {"pc": pc, "mac": macmon}


def scene6(baker, rig, aspect, ctx):
    kit.into(kit.collection("S6_LaOtraPantalla"))
    pc = ctx["s5"]["pc"]
    imgs = ["win_code", "win_plain", None]
    for i, mon in enumerate(pc):
        if i < 2:
            sm, tint, sem = kit.screen_mat_tint(f"S6PCM{i}", tex(imgs[i]), 0.0)
            kit.assign(mon["screen"], sm)
            baker.add(sem, "default_value", lambda t, i=i: 1.0 * smooth(ramp(t, 9.55 + 0.06 * i, 9.85 + 0.06 * i)))
        baker.add(kit.mat_input(mon["led"].data.materials[0], "Strength"), "default_value", lambda t: 3.0 if t > 9.5 else 0.0)
    # Monitor 3: el escritorio del Mac entra con un barrido de luz.
    sm3, wipe3, sem3 = kit.screen_mat_wipe("S6PC3M", tex("win_sheet"), tex("mac_notes"), 0.0)
    kit.assign(pc[2]["screen"], sm3)
    baker.add(sem3, "default_value", lambda t: 1.0 * smooth(ramp(t, 9.62, 9.9)))
    baker.add(wipe3, "default_value", lambda t: 1.05 * ease_in_out(ramp(t, T.MAC_APPEARS, T.MAC_APPEARS + 0.38), 2))
    # Costura de luz entre el monitor 2 y el 3 cuando el cursor cruza.
    seam_m = kit.emission_mat("S6CosturaM", LAVENDER, 0.0)
    m3x = DESK.x + 0.34
    seam = kit.box("S6Costura", (0.004, 0.004, 0.36), radius=0.0015, loc=(m3x, DESK.y - 0.02, 0.16 + 0.35 / 2 + 0.02), mats=seam_m)
    t_cross = 10.24
    baker.add(kit.mat_input(seam_m, "Strength"), "default_value", lambda t: 10 * smooth(ramp(t, t_cross - 0.03, t_cross)) * (1 - smooth(ramp(t, t_cross + 0.03, t_cross + 0.3))))
    # La pantalla virtual (violeta): se dibuja en el aire a la derecha del monitor 3 y se llena de luz.
    vx = DESK.x + 0.68 * 2
    vz = 0.16 + 0.35 / 2 + 0.02
    vpts = [(vx + px, DESK.y - 0.01, vz + pz) for px, pz in kit.rounded_rect_points(0.62, 0.35, 0.012, 4)]
    top = len(vpts) // 4
    vpts = vpts[top:] + vpts[:top]
    vl = light_line("S6Virtual", vpts + [vpts[0]], core=0.003, halo=0.010, strength=7.0, smooth_curve=False)
    for o in (vl["core"], vl["halo"]):
        kit.assign(o, kit.emission_mat("S6VirtualNucleoM", "#E7DEFF", 7.0) if o is vl["core"] else o.data.materials[0])
    v0, v1 = 10.42, 10.66
    line_window(baker, vl, lambda t: 0.0, lambda t: ease_in_out(ramp(t, v0, v1), 2))
    vs = kit.plane("S6VirtualPantalla", 0.6, 0.335, loc=(vx, DESK.y - 0.008, vz))
    vm, vtint, vem = kit.screen_mat_tint("S6VirtualPantallaM", tex("mac_photos"), 0.0)
    kit.assign(vs, vm)
    vtint.default_value = kit.srgb("#E2D6FF")
    baker.add(vem, "default_value", lambda t: 1.3 * smooth(ramp(t, v1 - 0.05, v1 + 0.2)))
    vlight = kit.area_light("S6VirtualLuz", (vx, DESK.y - 0.05, vz), (vx, DESK.y - 2.0, vz), 0.0, size=0.62, size_y=0.35, color=VIOLET)
    baker.add(vlight.data, "energy", lambda t: 25 * smooth(ramp(t, v1 - 0.05, v1 + 0.2)))

    # El cursor cruza del PC al Mac y entra en la pantalla virtual; se sacude de alegría.
    zc = vz + 0.02
    keys = [(9.95, Vector((DESK.x - 0.45, DESK.y - 0.03, zc + 0.05))), (t_cross, Vector((m3x, DESK.y - 0.03, zc))),
            (10.62, Vector((DESK.x + 0.92, DESK.y - 0.03, zc - 0.04))), (10.80, Vector((vx - 0.05, DESK.y - 0.035, zc - 0.02))),
            (11.05, Vector((vx + 0.02, DESK.y - 0.035, zc + 0.01))), (11.25, Vector((vx + 0.6, DESK.y - 0.2, zc + 0.5)))]
    cp = path_fn(keys)
    joy = 10.86

    def jscale(t):
        return 1.0 + 0.22 * math.sin(math.pi * ramp(t, joy, joy + 0.22)) + 0.04 * impulse(t - joy - 0.22, 4.0, 9.0)

    make_cursor("S6Cursor", baker, cp, rot_fn=lambda t: Euler((0, 0, math.radians(14 * math.sin(2 * math.pi * 4 * (t - joy)) * smooth(ramp(t, joy, joy + 0.05))
                                                                   * (1 - smooth(ramp(t, joy + 0.15, joy + 0.3)))))),
                stem_fn=stem_follow(cp, 1.2), scale_fn=jscale, life=(9.9, 11.3))
    kit.area_light("S6Clave", tuple(DESK + Vector((-1.4, -2.4, 2.2))), tuple(DESK + Vector((0.4, 0, 0.5))), 90, size=2.2, color="#F4F1EA")

    rig.key(9.62, pos=tuple(DESK + Vector((0.25, -2.55, 0.85))), target=tuple(DESK + Vector((0.3, 0.0, 0.48))), focal=35, fstop=4.0, roll=0.0)
    rig.key(10.25, pos=tuple(DESK + Vector((0.62, -2.30, 0.76))), target=tuple(DESK + Vector((0.78, 0.0, 0.45))), focal=37, fstop=3.8,
            roll=math.radians(-0.4))
    rig.key(10.95, pos=tuple(DESK + Vector((0.85, -2.20, 0.74))), target=tuple(DESK + Vector((0.98, 0.0, 0.45))), focal=38, fstop=3.5,
            roll=math.radians(-0.6))
    return {"virtual": Vector((vx, DESK.y, vz))}


# ================================================================ ESCENA 07 — CUATRO MUNDOS, UN LUGAR

GLYPH = Vector((4.0, 29.0, 0.0))
OFF = (16.62, 16.98)        # al final todo se apaga: el escenario queda despejado para el cierre


def off(t):
    return 1 - smooth(ramp(t, *OFF))
GW, GH = 4.6, 3.1
STATIONS = {
    "windows": GLYPH + Vector((-1.72, 0.98, 0)),
    "mac": GLYPH + Vector((1.72, 0.98, 0)),
    "iphone": GLYPH + Vector((1.72, -0.98, 0)),
    "android": GLYPH + Vector((-1.72, -0.98, 0)),
}
COLORS = {"windows": "#4361EE", "mac": "#D9DCE3", "iphone": "#FFFFFF", "android": GREEN}


def floor_rect_pts(cx, cy, w, h, r, seg=10):
    return [(cx + x, cy + z, 0.006) for x, z in kit.rounded_rect_points(w, h, r, seg)]


def scene7(baker, rig, aspect, ctx):
    kit.into(kit.collection("S7_CuatroMundos"))
    vpos = ctx["s6"]["virtual"]
    times = T.PLATFORMS
    # Las cuatro ramas salen de la pantalla virtual, vuelan en arco y aterrizan en las esquinas del gran icono.
    corner = {
        "windows": GLYPH + Vector((-GW / 2 + 0.55, GH / 2 - 0.55, 0.006)),
        "mac": GLYPH + Vector((GW / 2 - 0.55, GH / 2 - 0.55, 0.006)),
        "iphone": GLYPH + Vector((GW / 2 - 0.55, -GH / 2 + 0.55, 0.006)),
        "android": GLYPH + Vector((-GW / 2 + 0.55, -GH / 2 + 0.55, 0.006)),
    }
    for k, (name, t_land) in enumerate(times.items()):
        c = corner[name]
        mid = (vpos + c) / 2 + Vector((0, 0, 1.8 + 0.3 * k))
        pts = [vpos + Vector((0, -0.02, 0.18)), vpos + Vector((0.0, 0.3, 0.9)), mid, c + Vector((0, 0, 0.6)), c]
        ln = light_line(f"S7Rama{name}", [tuple(p) for p in pts], core=0.003, halo=0.010, strength=7.0)
        kit.assign(ln["core"], kit.emission_mat(f"S7RamaNucleo{name}", COLORS[name] if name != "iphone" else "#FFFFFF", 7.0))
        t0 = 10.95
        line_window(baker, ln, lambda t, t_land=t_land: ease_in_out(ramp(t, t0, t_land), 2) - 0.35 + 0.35 * smooth(ramp(t, t_land, t_land + 0.3)),
                    lambda t, t_land=t_land: ease_in_out(ramp(t, t0, t_land), 2))
        for r in range(2):
            rm = kit.emission_mat(f"S7Aterriza{name}{r}", COLORS[name], 0.0)
            ring = kit.curve(f"S7Aterriza{name}{r}", [(math.cos(a), math.sin(a), 0.0) for a in [i * 2 * math.pi / 96 for i in range(96)]],
                             bevel=0.006, res=2, closed=True, smooth=False, mats=rm)
            ring.location = c + Vector((0, 0, 0.002))
            tr = t_land + 0.06 * r
            baker.add(ring, "scale", lambda t, tr=tr, r=r: (lambda s: (s, s, 1))(0.01 + (0.9 + 0.5 * r) * expo_out(ramp(t, tr, tr + 0.7))))
            baker.add(kit.mat_input(rm, "Strength"), "default_value",
                      lambda t, tr=tr, r=r: (5 - 2 * r) * smooth(ramp(t, tr, tr + 0.02)) * (1 - smooth(ramp(t, tr + 0.08, tr + 0.65))))
    # El contorno del icono en el suelo: cada rama sigue hasta la siguiente esquina (sentido horario).
    order = ["windows", "mac", "iphone", "android"]
    outline = floor_rect_pts(GLYPH.x, GLYPH.y, GW, GH, 1.0, 12)
    # Empieza en la esquina superior izquierda (después del arco).
    def nearest(p):
        return min(range(len(outline)), key=lambda i: (Vector(outline[i]) - p).length)
    idx = {n: nearest(corner[n]) for n in order}
    n = len(outline)
    for k, name in enumerate(order):
        a = idx[name]
        b = idx[order[(k + 1) % 4]]
        seg = [outline[(a + j) % n] for j in range(((b - a) % n) + 1)]
        if name == "iphone":
            # El borde inferior deja libre el hueco del pie (como en el logo, el pie va aparte).
            pass
        ln = light_line(f"S7Borde{name}", seg, core=0.010, halo=0.03, strength=4.0, smooth_curve=False)
        t0 = times[name] + 0.05
        t1 = t0 + 0.9 if name != "android" else t0 + 0.5
        line_window(baker, ln, lambda t: smooth(ramp(t, *OFF)), lambda t, t0=t0, t1=t1: ease_in_out(ramp(t, t0, t1), 2))
    # Pie y mitad derecha encendida (al final, desde arriba se lee el logo).
    stand = light_line("S7Pie", [(GLYPH.x - 1.0, GLYPH.y - GH / 2 - 0.7, 0.006), (GLYPH.x + 1.0, GLYPH.y - GH / 2 - 0.7, 0.006)],
                       core=0.010, halo=0.03, strength=4.0, smooth_curve=False)
    line_window(baker, stand, lambda t: 0.5 - 0.5 * ease_in_out(ramp(t, 14.05, 14.35), 2) * off(t),
                lambda t: 0.5 + 0.5 * ease_in_out(ramp(t, 14.05, 14.35), 2) * off(t))
    half = kit.plane("S7Mitad", GW / 2 - 0.25, GH - 0.25)
    half.rotation_euler = (math.radians(-90), 0, 0)
    half.location = (GLYPH.x + GW / 4 - 0.06, GLYPH.y, 0.004)
    hm = kit.mat("S7MitadM", "#101012", rough=0.4, emit=LAVENDER, strength=0.0)
    kit.assign(half, hm)
    baker.add(kit.mat_input(hm, "Emission Strength"), "default_value", lambda t: 0.10 * smooth(ramp(t, 14.15, 14.5)) * (1 - smooth(ramp(t, 16.6, 17.0))))

    # --- Dioramas (cada uno bajo una raíz: escala 1,5 y se hunde en el suelo al final)
    # La pantalla de Android espera bajo el suelo y sale al llegar la cámara: delante de la estación de Windows la
    # taparía justo cuando la voz la nombra.
    rise = (13.25, 13.70)
    roots = {}
    for name, pos in STATIONS.items():
        r = kit.empty(f"S7Raiz{name}", loc=tuple(pos))
        r.scale = (1.8, 1.8, 1.8)
        roots[name] = r
        up = (lambda t: -1.9 * (1 - back_out(ramp(t, *rise), 1.1))) if name == "android" else (lambda t: 0.0)
        baker.add(r, "location", lambda t, pos=pos, up=up: tuple(pos + Vector((0, 0, up(t) - 1.4 * ease_in(ramp(t, *OFF), 2)))))
    W = Vector((0, 0, 0))
    for i in range(3):
        mon = assets.monitor(f"S7Win{i}", 0.34, 0.19, tex(["win_code", "win_photo", "win_sheet"][i]), stand=True)
        mon["root"].location = W + Vector((-0.38 + 0.38 * i, 0.15, 0))
        mon["root"].rotation_euler = (0, 0, [0.18, 0, -0.18][i])
        mon["root"].parent = roots["windows"]
        # La primera estación se enciende mientras la cámara va hacia ella (si no, el giro cruza la sala a oscuras) y
        # termina antes de que la voz diga «Windows».
        baker.add(kit.mat_input(mon["screen_mat"], "Emission Strength"), "default_value",
                  lambda t, i=i: 1.0 * smooth(ramp(t, times["windows"] - 0.42 + 0.05 * i, times["windows"] - 0.12 + 0.05 * i)) * off(t))
    for j, lab in enumerate(("Win", "Alt", "Tab")):
        key = kit.box(f"S7Tecla{j}", (0.11, 0.1, 0.045), radius=0.016, mats=M("graphite"))
        key.parent = roots["windows"]
        kt = kit.text(f"S7TeclaTxt{j}", lab, size=0.032, mats=M("white_text"))
        kt.parent = key
        kt.rotation_euler = (0, 0, 0)
        kt.location = (0, 0, 0.0235)
        ph = j * 2 * math.pi / 3
        baker.add(key, "location", lambda t, ph=ph: tuple(W + Vector((0.42 * math.cos(ph + 1.4 * t), -0.35 + 0.18 * math.sin(ph + 1.4 * t),
                                                                       0.55 + 0.05 * math.sin(2 * t + ph)))))
        baker.add(key, "rotation_euler", lambda t, ph=ph: (0.4 * math.sin(t + ph), 0.3 * math.cos(t * 1.3 + ph), 0.6 * t + ph))
        baker.add(key, "scale", lambda t, j=j: (lambda s: (s, s, s))(back_out(ramp(t, times["windows"] + 0.1 + 0.06 * j, times["windows"] + 0.35 + 0.06 * j), 1.6)))
    Mc = Vector((0, 0, 0))
    mm = assets.mac_mini("S7MacMini")
    mm["root"].location = Mc + Vector((0.32, -0.12, 0))
    mm["root"].parent = roots["mac"]
    mmon = assets.monitor("S7MacMon", 0.5, 0.28, tex("mac_notes"), stand=True)
    mmon["root"].location = Mc + Vector((-0.05, 0.2, 0))
    mmon["root"].parent = roots["mac"]
    baker.add(kit.mat_input(mmon["screen_mat"], "Emission Strength"), "default_value", lambda t: 1.0 * smooth(ramp(t, times["mac"] - 0.08, times["mac"] + 0.12)) * off(t))
    mmon2 = assets.monitor("S7MacMon2", 0.5, 0.28, tex("mac_photos"), stand=True)
    mmon2["root"].parent = roots["mac"]
    baker.add(mmon2["root"], "location", lambda t: tuple(Mc + Vector((-0.59 - 0.6 * (1 - spring(t - times["mac"] - 0.15, 2.4, 8.0)) if t > times["mac"] + 0.15 else -1.19, 0.2, 0))))
    baker.add(kit.mat_input(mmon2["screen_mat"], "Emission Strength"), "default_value", lambda t: 1.0 * smooth(ramp(t, times["mac"] + 0.35, times["mac"] + 0.6)) * off(t))
    bar = kit.slab("S7BarraMenu", 0.62, 0.03, 0.012, 0.01, bevel=0.002, mats=kit.mat("S7BarraMenuM", "#EDEDF0", rough=0.4, emit="#FFFFFF", strength=0.3))
    bar.parent = roots["mac"]
    bar.location = Mc + Vector((-0.32, 0.05, 0.74))
    baker.add(bar, "scale", lambda t: (lambda s: (s, s, s))(back_out(ramp(t, times["mac"] + 0.05, times["mac"] + 0.3), 1.5)))
    I = Vector((0, 0, 0))
    ph = assets.phone("S7iPhone", tex("iphone"))
    ph["root"].parent = roots["iphone"]
    ph["root"].location = I + Vector((0.16, -0.05, 0.22))
    ph["root"].scale = (2.4, 2.4, 2.4)
    ph["root"].rotation_euler = (math.radians(-28), 0, math.radians(-14))
    baker.add(kit.mat_input(ph["screen_mat"], "Emission Strength"), "default_value", lambda t: 1.0 * smooth(ramp(t, times["iphone"] - 0.08, times["iphone"] + 0.12)) * off(t))
    ipad = assets.phone("S7iPad", tex("mac_photos"), w=0.25, h=0.18)
    ipad["root"].parent = roots["iphone"]
    ipad["root"].location = I + Vector((-0.22, 0.22, 0.22))
    ipad["root"].scale = (1.6, 1.6, 1.6)
    ipad["root"].rotation_euler = (math.radians(-12), 0, math.radians(14))
    baker.add(kit.mat_input(ipad["screen_mat"], "Emission Strength"), "default_value", lambda t: 1.0 * smooth(ramp(t, times["iphone"] + 0.1, times["iphone"] + 0.3)) * off(t))
    for r in range(3):
        rm = kit.emission_mat(f"S7ToqueM{r}", "#FFFFFF", 0.0)
        ring = kit.curve(f"S7Toque{r}", [(0.03 * math.cos(a), -0.001, 0.03 * math.sin(a)) for a in [i * 2 * math.pi / 48 for i in range(48)]],
                         bevel=0.0012, res=2, closed=True, smooth=False, mats=rm)
        ring.parent = ph["root"]
        ring.location = (0, -0.006, -0.01)
        tr = times["iphone"] + 0.2 + r * T.BEAT * 0.5
        baker.add(ring, "scale", lambda t, tr=tr: (lambda s: (s, 1, s))(0.2 + 1.2 * expo_out(ramp(t, tr, tr + 0.4))))
        baker.add(kit.mat_input(rm, "Strength"), "default_value", lambda t, tr=tr: 3 * smooth(ramp(t, tr, tr + 0.02)) * (1 - smooth(ramp(t, tr + 0.05, tr + 0.4))))
    A = Vector((0, 0, 0))
    big = assets.monitor("S7Android", 1.0, 0.5625, tex("android"), stand=False)
    big["root"].parent = roots["android"]
    big["root"].location = A + Vector((0.0, 0.25, 0.0))
    stand_l = kit.box("S7AndroidPata", (0.05, 0.05, 0.36), radius=0.01, loc=tuple(A + Vector((0.0, 0.3, 0.18))), mats=M("anodized"))
    stand_l.parent = roots["android"]
    big["head"].location = (0, 0, 0.36 + 0.28 + 0.02)
    baker.add(kit.mat_input(big["screen_mat"], "Emission Strength"), "default_value", lambda t: 1.0 * smooth(ramp(t, times["android"] - 0.08, times["android"] + 0.12)) * off(t))
    tm = kit.emission_mat("S7AndroidToqueM", GREEN, 0.0)
    tap = kit.curve("S7AndroidToque", [(0.06 * math.cos(a), 0, 0.06 * math.sin(a)) for a in [i * 2 * math.pi / 64 for i in range(64)]],
                    bevel=0.002, res=2, closed=True, smooth=False, mats=tm)
    tap.parent = roots["android"]
    tap.location = A + Vector((0.2, 0.25 - 0.012, 0.36 + 0.28 + 0.08))
    tt = times["android"] + 0.18
    baker.add(tap, "scale", lambda t: (lambda s: (s, 1, s))(0.2 + 2.5 * expo_out(ramp(t, tt, tt + 0.5))))
    baker.add(kit.mat_input(tm, "Strength"), "default_value", lambda t: 5 * smooth(ramp(t, tt, tt + 0.02)) * (1 - smooth(ramp(t, tt + 0.05, tt + 0.5))))
    for name, pos in STATIONS.items():
        L = kit.area_light(f"S7Luz{name}", tuple(pos + Vector((0, -1.6, 2.2))), tuple(pos + Vector((0, 0, 0.3))), 0.0, size=1.6, color=COLORS[name])
        # La de Windows sube antes: la energía que se acerca la saca de la oscuridad mientras la cámara llega.
        lead = 0.45 if name == "windows" else 0.25
        baker.add(L.data, "energy", lambda t, name=name, lead=lead: 160 * smooth(ramp(t, times[name] - lead, times[name] + 0.05)) * off(t))
    cen = kit.area_light("S7Cenital", tuple(GLYPH + Vector((0, -1.0, 6.0))), tuple(GLYPH), 400, size=5.0, color="#E8ECFF")
    baker.add(cen.data, "energy", lambda t: 400 * off(t))

    # Cámara: recorre las esquinas en el orden de la voz y se eleva hasta ver el icono completo.
    def cam_at(st, t, back=-1.15, h=0.85, side=0.0, f=34, tgt_z=0.32):
        p = STATIONS[st]
        rig.key(t, pos=tuple(p + Vector((side, back, h))), target=tuple(p + Vector((0, 0.15, tgt_z))), focal=f, fstop=4.0,
                roll=math.radians(side * 2))

    # Plano medio que sube y barre las cuatro esquinas en el orden de la voz (energía fluida).
    G = GLYPH
    rig.key(11.20, pos=(4.1, 23.3, 2.3), target=tuple(STATIONS["windows"] + Vector((0.3, 0.0, 0.3))), focal=34, fstop=5.6, roll=math.radians(0.5))
    rig.key(11.45, pos=(2.45, 27.05, 3.05), target=tuple(STATIONS["windows"] + Vector((0.0, 0.25, 0.5))), focal=38, fstop=5.0, roll=math.radians(1.0))
    rig.key(12.10, pos=(5.05, 26.55, 1.30), target=tuple(STATIONS["mac"] + Vector((-0.2, 0.2, 0.60))), focal=38, fstop=5.0, roll=math.radians(-1.5))
    rig.key(12.74, pos=(6.95, 25.85, 0.98), target=tuple(STATIONS["iphone"] + Vector((-0.1, 0.1, 0.4))), focal=40, fstop=5.0, roll=math.radians(-0.8))
    rig.key(13.90, pos=(1.35, 25.55, 1.15), target=tuple(STATIONS["android"] + Vector((0.1, 0.4, 0.95))), focal=36, fstop=5.0, roll=math.radians(1.2))
    return {}


# ================================================================ ESCENAS 08, 09 y 10 — LA GRAN IDEA, EL CIERRE, LA FIRMA

def scene8(baker, rig, aspect, ctx):
    kit.into(kit.collection("S8_LaGranIdea"))
    # Las cuatro ramas convergen en el centro y forman la base tipográfica (sobre el suelo, dentro del icono).
    base_y = GLYPH.y - 0.05
    for k, (name, pos) in enumerate(STATIONS.items()):
        tgt = Vector((GLYPH.x + (-1.3 if pos.x < GLYPH.x else 1.3), base_y, 0.006))
        mid = Vector((pos.x * 0.35 + tgt.x * 0.65, pos.y * 0.6 + tgt.y * 0.4, 0.006))
        ln = light_line(f"S8Converge{name}", [tuple(pos + Vector((0, 0, 0.006))), tuple(mid), tuple(tgt)],
                        core=0.004, halo=0.012, strength=6.0)
        kit.assign(ln["core"], kit.emission_mat(f"S8ConvergeNucleo{name}", COLORS[name] if name != "iphone" else "#FFFFFF", 6.0))
        line_window(baker, ln, lambda t: ease_in_out(ramp(t, 14.62, 14.98), 2), lambda t: ease_in_out(ramp(t, 14.32, 14.72), 2))
    base = light_line("S8Base", [(GLYPH.x - 1.3, base_y, 0.006), (GLYPH.x + 1.3, base_y, 0.006)], core=0.006, halo=0.02, strength=6.0,
                      smooth_curve=False)
    line_window(baker, base, lambda t: 0.5 - 0.5 * ease_out(ramp(t, 14.62, 14.95), 2) + 0.5 * ease_in(ramp(t, 16.62, 16.98), 2),
                lambda t: 0.5 + 0.5 * ease_out(ramp(t, 14.62, 14.95), 2) - 0.5 * ease_in(ramp(t, 16.62, 16.98), 2))
    # Por un instante la línea se divide en los cuatro colores y vuelve a unirse.
    for k, name in enumerate(STATIONS):
        off = (k - 1.5) * 0.035
        sl = light_line(f"S8Color{name}", [(GLYPH.x - 1.3, base_y + off, 0.006), (GLYPH.x + 1.3, base_y + off, 0.006)], core=0.004,
                        halo=0.012, strength=6.0, smooth_curve=False)
        kit.assign(sl["core"], kit.emission_mat(f"S8ColorNucleo{name}", COLORS[name], 6.0))
        sp = lambda t: smooth(ramp(t, 15.62, 15.72)) * (1 - smooth(ramp(t, 15.86, 15.98)))
        line_window(baker, sl, lambda t, sp=sp: 0.5 - 0.5 * sp(t), lambda t, sp=sp: 0.5 + 0.5 * sp(t))
    # Texto sobre el suelo: sube desde dentro del suelo (el suelo hace de máscara) y se hunde al final.
    for line_txt, ton, dy, size in (("TODAS TUS PANTALLAS.", T.TEXT1, 0.30, 0.30), ("UN SOLO LUGAR.", T.TEXT2, -0.24, 0.30)):
        tx = kit.text(f"S8Texto{ton}", line_txt, font="semibold", size=size, spacing=1.02, mats=M("white_text"))
        tx.rotation_euler = (0, 0, 0)       # tumbado en el suelo, legible desde arriba
        loc0 = Vector((GLYPH.x, base_y + dy, 0.0))

        def tloc(t, loc0=loc0, ton=ton):
            up = back_out(ramp(t, ton, ton + 0.32), 1.2)
            down = ease_in(ramp(t, 16.62, 16.92), 2)
            return tuple(loc0 + Vector((0, 0, -0.06 + 0.07 * up - 0.08 * down)))

        baker.add(tx, "location", tloc)
        tx.data.extrude = 0.02
        baker.add(tx, "scale", lambda t, ton=ton: (lambda k: (k, k, 1.0))(1.0 if t >= ton - 0.02 else 0.0), const=True)
    # Cámara: sube hasta ver el icono completo desde arriba y desciende despacio en oblicuo (contiene la respiración).
    rig.key(14.80, pos=tuple(GLYPH + Vector((0.0, -0.6, 5.4))), target=tuple(GLYPH + Vector((0.0, -0.1, 0.0))), focal=32, fstop=8.0, roll=0.0)
    rig.key(16.30, pos=tuple(GLYPH + Vector((0.0, -1.6, 4.7))), target=tuple(GLYPH + Vector((0.0, -0.05, 0.0))), focal=32, fstop=8.0, roll=0.0)
    return {}


LOGO2_Z = 0.98


def scene9(baker, rig, aspect, ctx):
    kit.into(kit.collection("S9_ElCierre"))
    tl = T.LANDING
    spot = Vector((GLYPH.x + 0.05, GLYPH.y - 0.75, 0.207))
    corner = Vector((GLYPH.x + 0.30, GLYPH.y - 0.06, LOGO2_Z - 0.30))
    # El cursor cae desde arriba y aterriza con el golpe grave; al sonar la firma salta y hace clic en el logo.
    def pos(t):
        if t < tl:
            u = ramp(t, 16.92, tl)
            return spot + Vector((0.05 * (1 - u), 0.0, 2.6 * (1 - u * u)))
        if t < 18.05:
            return spot.copy()
        u = ramp(t, 18.05, T.FANFARE)
        p = spot.lerp(corner, ease_in_out(u, 2))
        p.z += 0.22 * math.sin(math.pi * u)
        return p

    def squash(t):
        s = 0.0
        if t >= tl:
            s += 0.95 * math.exp(-9 * (t - tl)) * max(0.0, math.cos(2 * math.pi * 1.8 * (t - tl)))
        s += 0.4 * smooth(ramp(t, 17.95, 18.05)) * (1 - smooth(ramp(t, 18.05, 18.11)))
        if t >= T.FANFARE:
            s += 0.6 * math.exp(-12 * (t - T.FANFARE)) * max(0.0, math.cos(2 * math.pi * 2.2 * (t - T.FANFARE)))
        return s

    def stem(t):
        return 1.0 * impulse(t - tl, 2.0, 4.5) + 0.8 * impulse(t - 18.42, 3.0, 6.0)

    def joy(t):
        return 1.0 + 0.20 * math.sin(math.pi * ramp(t, T.FANFARE + 0.08, T.FANFARE + 0.32))

    def rot(t):
        # De pie, mirando a cámara, con un leve giro hacia el logo al asentarse.
        return Euler((0.0, 0.0, math.radians(-8 * smooth(ramp(t, tl + 0.3, tl + 0.7)))))

    make_cursor("S9Cursor", baker, pos, rot_fn=rot, stem_fn=stem, squash_fn=squash, life=(16.9, 21.3), scale_fn=joy)
    # El brillo del cursor que cae se proyecta en el suelo antes de que entre en cuadro: un charco lavanda que se cierra
    # y se intensifica al acercarse, y cede a las ondas al aterrizar (la bajada no queda a oscuras).
    # La potencia crece con la altura para que el charco se lea desde arriba (en el suelo cae con la distancia al cuadrado).
    head_light("S9BrilloCaida", baker, lambda t: pos(t) + Vector((0.0, 0.0, 0.10)),
               lambda t: (40.0 + 70.0 * (pos(t).z - spot.z)) * smooth(ramp(t, 16.86, 17.00))
               * (1 - smooth(ramp(t, tl + 0.02, tl + 0.20))), radius=0.08)
    # Onda en el suelo desde el punto de contacto.
    for k in range(2):
        rm = kit.emission_mat(f"S9OndaM{k}", LAVENDER, 0.0)
        ring = kit.curve(f"S9Onda{k}", [(math.cos(a), math.sin(a), 0.0) for a in [i * 2 * math.pi / 128 for i in range(128)]],
                         bevel=0.0018, res=2, closed=True, smooth=False, mats=rm)
        ring.location = spot + Vector((0.02, -0.03, -0.203))
        t0 = tl + 0.05 * k
        baker.add(ring, "scale", lambda t, t0=t0, k=k: (lambda r: (r, r, 1))(0.01 + (1.4 + 0.6 * k) * expo_out(ramp(t, t0, t0 + 0.9))))
        baker.add(kit.mat_input(rm, "Strength"), "default_value", lambda t, t0=t0, k=k: (2.6 - 1.0 * k) * smooth(ramp(t, t0, t0 + 0.02)) * (1 - smooth(ramp(t, t0 + 0.05, t0 + 0.7))))
    # El logo de Kast se levanta del suelo detrás del cursor y la línea lo traza.
    lg = assets.logo("Logo2", 0.9)
    lg["root"].location = (GLYPH.x, GLYPH.y, LOGO2_Z)
    tile_rise(baker, lg, T.LOGO2[0] - 0.06, half=0.45)
    d0, d1 = T.LOGO2
    for part, a, b in ((lg["outline"], d0, d0 + 0.32), (lg["stand"], d0 + 0.30, d1)):
        baker.add(part.data, "bevel_factor_end", lambda t, a=a, b=b: max(1e-4, ease_in_out(ramp(t, a, b), 2.2)))
    baker.add(lg["fill"], "scale", lambda t: (max(1e-4, back_out(ramp(t, d0 + 0.22, d0 + 0.40), 1.2)), 1.0, 1.0))
    baker.add(kit.mat_input(lg["hair"].data.materials[0], "Strength"), "default_value", lambda t: 0.35 * smooth(ramp(t, d0 + 0.2, d1 + 0.2)))
    kit.area_light("S9Clave", tuple(GLYPH + Vector((-1.8, -2.6, 2.6))), tuple(GLYPH + Vector((0, 0, 0.6))), 110, size=2.0, color="#F4F1EA")
    kit.area_light("S9Recorte", tuple(GLYPH + Vector((1.6, 1.6, 2.0))), tuple(GLYPH + Vector((0, 0, 0.6))), 120, size=1.2, color="#C9D1FF")
    # Cámara: anticipa la llegada (encuadra el punto de aterrizaje antes de que entre el cursor) y pasa de la vista
    # oblicua a una frontal limpia, muy despacio.
    rig.key(16.95, pos=tuple(GLYPH + Vector((0.05, -2.75, 1.45))), target=tuple(spot + Vector((0.0, 0.0, 0.05))), focal=40, fstop=3.5, roll=0.0)
    rig.key(17.28, pos=tuple(GLYPH + Vector((0.05, -2.62, 1.20))), target=tuple(spot + Vector((0.0, 0.1, 0.12))), focal=40, fstop=3.5, roll=0.0)
    rig.shake(tl, (0, 0, -1), amp=0.012, freq=5.0, damp=8.0)
    rig.key(18.70, pos=tuple(GLYPH + Vector((0.0, -3.15, 1.15))), target=tuple(GLYPH + Vector((0.04, 0.0, 0.72))), focal=40, fstop=4.0, roll=0.0)
    rig.key(19.20, pos=tuple(GLYPH + Vector((0.0, -3.1, 1.12))), target=tuple(GLYPH + Vector((0.04, 0.0, 0.72))), focal=40, fstop=4.0)
    return {"logo": lg}


def scene10(baker, rig, aspect, ctx):
    kit.into(kit.collection("S10_Firma"))
    t0 = T.S10[0]
    if aspect == "v":
        lines = [("Gratis y de código abierto", 0.075, "regular", 0.38, M("sub_text")),
                 ("Windows · macOS · iPhone · iPad · Android", 0.07, "regular", 0.27, M("sub_text")),
                 ("www.kastdesk.com", 0.095, "semibold", 0.11, M("white_text"))]
    else:
        lines = [("Gratis y de código abierto · Windows · macOS · iPhone · iPad · Android", 0.066, "regular", 0.28, M("sub_text")),
                 ("www.kastdesk.com", 0.085, "semibold", 0.12, M("white_text"))]
    for k, (txt, size, fnt, dz, m) in enumerate(lines):
        o = kit.text(f"S10Linea{k}", txt, font=fnt, size=size, mats=m)
        base = Vector((GLYPH.x, GLYPH.y - 0.05, dz))

        def loc(t, base=base, k=k):
            u = ease_out(ramp(t, t0 + 0.1 + 0.12 * k, t0 + 0.6 + 0.12 * k), 3)
            # Sube desde dentro del suelo, que hace de máscara.
            return tuple(base + Vector((0, 0, -0.42 * (1 - u))))

        baker.add(o, "location", loc)
    # La cámara retrocede apenas y sube para dejar sitio a la firma.
    rig.key(21.2, pos=tuple(GLYPH + Vector((0.0, -3.45, 1.02))), target=tuple(GLYPH + Vector((0.03, 0.0, 0.62))), focal=40, fstop=5.6, roll=0.0)
    return {}
