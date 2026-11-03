"""Las estaciones del anuncio, una función por escena. Cada una crea sus objetos y registra su animación (funciones
del tiempo) en el horneador. Tiempos en ad/timeline.py."""
import math
import random
import bpy
from mathutils import Vector, Euler, Quaternion
from . import kit, assets
from .kit import ramp, smooth, ease_in, ease_out, ease_in_out, back_out, spring, impulse, expo_out, lerp, clamp
from .assets import LAVENDER, M
import ad.timeline as T

CORE = "#E4E8FF"


def light_line(name, pts, core=0.0035, halo=0.011, strength=7.0, smooth_curve=True):
    """La línea de luz: núcleo casi blanco, halo lavanda translúcido y una luz que acompaña a la cabeza."""
    cm = kit.emission_mat(name + "Nucleo", CORE, strength)
    hm = bpy.data.materials.new(name + "Halo")
    nt = hm.node_tree
    for n in list(nt.nodes):
        nt.nodes.remove(n)
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Color"].default_value = kit.srgb(LAVENDER)
    em.inputs["Strength"].default_value = strength * 0.22
    tr = nt.nodes.new("ShaderNodeBsdfTransparent")
    mix = nt.nodes.new("ShaderNodeMixShader")
    mix.inputs[0].default_value = 0.35
    nt.links.new(tr.outputs[0], mix.inputs[1])
    nt.links.new(em.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], out.inputs["Surface"])
    c = kit.curve(name, pts, bevel=core, res=4, mats=cm, smooth=smooth_curve)
    h = kit.curve(name + "Halo", pts, bevel=halo, res=4, mats=hm, smooth=smooth_curve)
    h.visible_shadow = False
    c.visible_shadow = False
    return {"core": c, "halo": h, "core_mat": cm, "halo_em": em.inputs["Strength"], "core_em": kit.mat_input(cm, "Strength")}


def line_window(baker, line, start_fn, end_fn, strength_fn=None):
    """Tramo visible de la línea (0..1 a lo largo de la curva)."""
    for o in (line["core"], line["halo"]):
        baker.add(o.data, "bevel_factor_start", lambda t: clamp(start_fn(t)))
        baker.add(o.data, "bevel_factor_end", lambda t: clamp(max(end_fn(t), start_fn(t) + 1e-4)))
        baker.add(o, "hide_render", lambda t: 1.0 if clamp(end_fn(t)) - clamp(start_fn(t)) < 0.003 else 0.0, const=True)
    if strength_fn:
        s0 = line["core_em"].default_value
        h0 = line["halo_em"].default_value
        baker.add(line["core_em"], "default_value", lambda t: s0 * strength_fn(t))
        baker.add(line["halo_em"], "default_value", lambda t: h0 * strength_fn(t))


def curve_point(obj, u):
    """Punto a lo largo de una curva (0..1, por longitud) en coordenadas del mundo (para luces que siguen la cabeza)."""
    if obj.name not in _CACHE:
        cu = obj.data
        sp = cu.splines[0]
        res = 48
        pts = []
        if sp.type == "BEZIER":
            bps = sp.bezier_points
            n = len(bps) - 1 + (1 if sp.use_cyclic_u else 0)
            from mathutils.geometry import interpolate_bezier
            for i in range(n):
                a, b = bps[i], bps[(i + 1) % len(bps)]
                seg = interpolate_bezier(a.co, a.handle_right, b.handle_left, b.co, res)
                pts += seg if i == 0 else seg[1:]
        else:
            pts = [Vector(p.co[:3]) for p in sp.points]
        mw = obj.matrix_world.copy()
        pts = [mw @ Vector(p) for p in pts]
        acc = [0.0]
        for i in range(1, len(pts)):
            acc.append(acc[-1] + (pts[i] - pts[i - 1]).length)
        _CACHE[obj.name] = (pts, acc)
    pts, acc = _CACHE[obj.name]
    L = acc[-1] * clamp(u)
    for i in range(1, len(acc)):
        if acc[i] >= L:
            f = (L - acc[i - 1]) / max(1e-9, acc[i] - acc[i - 1])
            return pts[i - 1].lerp(pts[i], f)
    return pts[-1]


_CACHE = {}


def head_light(name, baker, pos_fn, energy_fn, color=LAVENDER, radius=0.05):
    ld = bpy.data.lights.new(name, "POINT")
    ld.color = kit.srgb(color)[:3]
    ld.shadow_soft_size = radius
    o = kit.link(bpy.data.objects.new(name, ld))
    baker.add(o, "location", lambda t: tuple(pos_fn(t)))
    baker.add(ld, "energy", energy_fn)
    return o


def tile_rise(baker, lg, t0, half=0.31):
    """La baldosa del logo se levanta girando sobre su canto inferior, con un pequeño rebote al asentarse."""
    root = lg["root"]
    base = root.location.copy()
    root.rotation_mode = "XYZ"

    def angle(t):
        if t < t0:
            return math.radians(-90)
        return math.radians(-90) * (1 - spring(t - t0, 2.6, 8.5))

    def rot(t):
        return (angle(t), 0.0, 0.0)

    def loc(t):
        a = angle(t)
        # El canto inferior queda fijo: el centro gira alrededor de él.
        return (base.x, base.y + half * math.sin(a) * -1.0 * -1.0 * 0 + (-half * math.sin(a)), base.z - half + half * math.cos(a))

    baker.add(root, "rotation_euler", rot)
    baker.add(root, "location", loc)
    for part in (lg["tile"], lg["hair"], lg["outline"], lg["fill"], lg["stand"]):
        baker.add(part, "hide_render", lambda t: 1.0 if t < t0 - 0.01 else 0.0, const=True)


# ================================================================ ESCENA 01 — EL CLIC

S1_GLASS_Y = 0.0
S1_CENTER = Vector((0.0, 0.0, 1.20))
LOGO1_Y = 0.95


def scene1(baker, rig, aspect):
    kit.into(kit.collection("S1_ElClic"))
    rnd = random.Random(1)
    t_click = T.CLICK
    tip_rest = Vector((-0.06, S1_GLASS_Y, 1.30))

    # --- energía en la profundidad y fibras de luz (cables de señal) con paralaje real
    spark = kit.box("S1Chispa", (0.05, 0.05, 0.05), radius=0.024, mats=kit.emission_mat("S1ChispaM", CORE, 0.0))
    spark.location = (0.0, 34.0, 1.25)
    baker.add(kit.mat_input(spark.data.materials[0], "Strength"), "default_value",
              lambda t: 60 * smooth(ramp(t, 0.0, 0.10)) * (1 - smooth(ramp(t, 0.20, 0.32))))
    for i in range(16):
        x = rnd.uniform(-1.6, 1.6)
        z = 1.2 + rnd.uniform(-0.9, 0.9)
        if abs(x) < 0.25 and abs(z - 1.2) < 0.25:
            x += 0.5 * (1 if x >= 0 else -1)
        L = rnd.uniform(1.5, 5.0)
        y0 = rnd.uniform(14, 40)
        speed = rnd.uniform(45, 110)
        t0 = rnd.uniform(0.02, 0.18)
        f = light_line(f"S1Fibra{i}", [(x, 0, z), (x, L, z)], core=0.0016, halo=0.005, strength=rnd.uniform(4, 9), smooth_curve=False)
        for o in (f["core"], f["halo"]):
            baker.add(o, "location", (lambda y0=y0, sp=speed, t0=t0: (lambda t: (0.0, y0 - sp * max(0.0, t - t0), 0.0)))())
        line_window(baker, f, lambda t: 0.0, lambda t: 1.0,
                    (lambda t0=t0: (lambda t: smooth(ramp(t, t0, t0 + 0.04)) * (1 - smooth(ramp(t, 0.42, 0.6)))))())

    # --- el cursor
    c = assets.cursor("Cursor")
    root = c["root"]
    approach0 = 0.13

    def cur_pos(t):
        if t < approach0:
            return Vector((0.0, 60.0, 1.0))
        if t < t_click:
            u = (t - approach0) / (t_click - approach0)
            y = lerp(32.0, S1_GLASS_Y, u)               # velocidad constante: el tamaño aparente se dispara
            x = lerp(0.35, tip_rest.x, smooth(u))
            z = lerp(0.85, tip_rest.z, smooth(u))
            return Vector((x, y, z))
        if t < 1.20:
            # Golpe contra el cristal: avanza un poco y rebota con un muelle.
            d = -0.045 * impulse(t - t_click, 3.2, 7.5)
            p = tip_rest.copy()
            p.y += d
            return p
        # Salto: anticipación (se agacha) y sale arriba a la derecha.
        u = ramp(t, 1.24, 1.56)
        p = tip_rest.copy()
        p.z -= 0.02 * smooth(ramp(t, 1.14, 1.24)) * (1 - smooth(ramp(t, 1.24, 1.30)))
        p += Vector((1.15, -0.25, 0.75)) * ease_in(u, 2.2) + Vector((0, 0, 0.12)) * math.sin(math.pi * u)
        return p

    def cur_rot(t):
        if t < t_click:
            u = ramp(t, approach0, t_click)
            e = Euler((math.radians(lerp(-35, 0, ease_out(u, 2))), math.radians(lerp(50, 0, ease_out(u, 2))),
                       math.radians(lerp(-30, 0, ease_out(u, 2)))))
        elif t < 1.20:
            # Mira al espectador (inclina la punta) y vuelve.
            look = smooth(ramp(t, 0.62, 0.74)) * (1 - smooth(ramp(t, 1.02, 1.14)))
            e = Euler((math.radians(-7 * look), 0, math.radians(-9 * look + 2.5 * impulse(t - t_click, 2.6, 6))))
        else:
            u = ramp(t, 1.24, 1.56)
            e = Euler((math.radians(-12 * ease_in(u)), math.radians(-25 * ease_in(u)), math.radians(18 * ease_in(u))))
        q = e.to_quaternion()
        return (q.w, q.x, q.y, q.z)

    root.rotation_mode = "QUATERNION"
    baker.add(root, "location", lambda t: tuple(cur_pos(t)))
    baker.add(root, "rotation_quaternion", cur_rot)
    baker.add(root, "scale", lambda t: (1.0, 1.0, 1.0) if t < 1.75 else (0.0, 0.0, 0.0), const=True)

    # Parpadeo: flecha -> barra de texto -> flecha (0,80 s).
    tb = 0.70

    def arrow_scale(t):
        k = 1 - smooth(ramp(t, tb, tb + 0.035)) + smooth(ramp(t, tb + 0.085, tb + 0.12))
        k = clamp(k)
        return (k, 1.0, 1.0)

    def ibeam_scale(t):
        k = smooth(ramp(t, tb + 0.02, tb + 0.05)) * (1 - smooth(ramp(t, tb + 0.075, tb + 0.105)))
        return (k, 1.0, 1.0)

    for o in (c["base"], c["body"]):
        baker.add(o, "scale", arrow_scale)
    baker.add(c["ibeam"], "scale", ibeam_scale)

    def squash(t):
        s = 0.0
        if t >= t_click:
            s += 0.85 * math.exp(-11 * (t - t_click)) * max(0.0, math.cos(2 * math.pi * 2.0 * (t - t_click)))
        s += 0.55 * smooth(ramp(t, 1.14, 1.24)) * (1 - smooth(ramp(t, 1.24, 1.29)))
        return s

    def stem(t):
        v = 0.0
        if t >= t_click:
            v += 0.9 * impulse(t - t_click, 2.4, 5.5)
        v -= 0.7 * impulse(t - 1.25, 2.2, 5.0)
        return v

    assets.cursor_shape(c, baker, stem, squash)
    baker.add(kit.mat_input(c["edge_mat"], "Emission Strength"), "default_value",
              lambda t: 0.6 + 4.0 * smooth(ramp(t, 0.30, t_click)) * (1 - 0.75 * smooth(ramp(t, 0.6, 1.0))))

    # --- onda en el «cristal» y fragmentos de energía
    for k in range(2):
        rm = kit.emission_mat(f"S1OndaM{k}", LAVENDER, 0.0)
        ring_pts = [(math.cos(a) * 1.0, 0, math.sin(a) * 1.0) for a in [i * 2 * math.pi / 96 for i in range(96)]]
        ring = kit.curve(f"S1Onda{k}", ring_pts, bevel=0.0007, res=2, closed=True, smooth=False, mats=rm)
        ring.location = (tip_rest.x + 0.002, S1_GLASS_Y + 0.004, tip_rest.z - 0.004)
        ring.visible_shadow = False
        t0 = t_click + 0.05 * k
        baker.add(ring, "scale", (lambda t0=t0, k=k: (lambda t: (lambda r: (r, r, 1.0))(0.002 + (0.26 + 0.14 * k) * expo_out(ramp(t, t0, t0 + 0.6)))))())
        baker.add(kit.mat_input(rm, "Strength"), "default_value",
                  (lambda t0=t0, k=k: (lambda t: (3.5 - 1.5 * k) * smooth(ramp(t, t0, t0 + 0.02)) * (1 - smooth(ramp(t, t0 + 0.04, t0 + 0.5)))))())
    for i in range(10):
        a = rnd.uniform(0, 2 * math.pi)
        el = rnd.uniform(-0.5, 0.5)
        sp = rnd.uniform(0.35, 1.3)
        d = Vector((math.cos(a), -rnd.uniform(0.2, 0.9), math.sin(a))).normalized()
        shard = kit.box(f"S1Fragmento{i}", (0.003, 0.003, 0.010), radius=0.001, mats=kit.emission_mat(f"S1FragM{i}", CORE, 0.0))
        shard.rotation_euler = (rnd.uniform(0, 3), rnd.uniform(0, 3), rnd.uniform(0, 3))
        o0 = tip_rest + Vector((0.02, -0.01, -0.05))
        baker.add(shard, "location", (lambda d=d, sp=sp, o0=o0: (lambda t: tuple(o0 + d * sp * 0.30 * expo_out(ramp(t, t_click, t_click + 0.7)))))())
        baker.add(shard, "scale", (lambda: (lambda t: (lambda s: (s, s, s))(max(0.0, 1 - ramp(t, t_click + 0.15, t_click + 0.55)) if t >= t_click else 0.0)))())
        baker.add(kit.mat_input(shard.data.materials[0], "Strength"), "default_value",
                  (lambda: (lambda t: 7 * (1 - smooth(ramp(t, t_click + 0.08, t_click + 0.5))) if t >= t_click else 0.0))())

    # --- el logo de Kast, detrás del cursor, trazado por la línea
    lg = assets.logo("Logo1", 0.62)
    lg["root"].location = (0.0, LOGO1_Y, 1.20)
    tile_rise(baker, lg, T.LOGO_DRAW[0] - 0.14)
    d0, d1 = T.LOGO_DRAW
    for part, a, b in ((lg["outline"], d0, d0 + 0.32), (lg["stand"], d0 + 0.30, d1)):
        baker.add(part.data, "bevel_factor_end", (lambda a=a, b=b: (lambda t: max(1e-4, ease_in_out(ramp(t, a, b), 2.2))))())
    baker.add(lg["fill"], "scale", lambda t: (max(1e-4, back_out(ramp(t, d0 + 0.22, d0 + 0.40), 1.2)), 1.0, 1.0))
    glyph_em = kit.mat_input(M("glyph"), "Emission Strength")
    baker.add(glyph_em, "default_value", lambda t: 0.55 + 0.9 * (1 - smooth(ramp(t, d1, d1 + 0.5))) * smooth(ramp(t, d0, d0 + 0.1)))
    baker.add(kit.mat_input(lg["hair"].data.materials[0], "Strength"), "default_value",
              lambda t: 0.35 * smooth(ramp(t, d0 + 0.2, d1 + 0.2)))
    # La baldosa sube desde la oscuridad: una luz de recorte la separa del fondo.
    kit.area_light("S1Clave", (-1.6, -0.8, 2.6), (0.0, 0.5, 1.2), 55, size=1.6, color="#F4F1EA")
    kit.area_light("S1Recorte", (1.4, 2.2, 1.9), (0.0, 0.9, 1.2), 70, size=1.0, color="#C9D1FF")
    kit.area_light("S1Relleno", (0.6, -1.8, 0.7), (0.0, 0.5, 1.2), 6, size=2.0, color="#E8ECFF")

    # Línea: de la punta del cursor al centro superior del contorno.
    s = lg["s"]
    top = Vector((0.0, LOGO1_Y - 0.004 * 0.62, 1.20 + 12.5 * s + (32 - 28.5) * s))
    a_pts = [tip_rest + Vector((0.004, -0.004, -0.006)), tip_rest + Vector((0.05, 0.25, 0.10)), (top + Vector((0.0, -0.25, 0.10))), top]
    la = light_line("S1Linea", a_pts, strength=8.0)
    line_window(baker, la, lambda t: ease_in_out(ramp(t, d0 - 0.02, d0 + 0.12)), lambda t: ease_out(ramp(t, d0 - 0.10, d0 - 0.01), 2))
    head_light("S1LuzCabeza", baker, lambda t: curve_point(la["core"], ease_out(ramp(t, d0 - 0.10, d0 - 0.01), 2)),
               lambda t: 1.2 * smooth(ramp(t, d0 - 0.16, d0 - 0.1)) * (1 - smooth(ramp(t, d0 + 0.05, d0 + 0.2))))

    # --- cámara
    rig.key(0.00, pos=(0.0, -1.06, 1.20), target=(0.0, 6.0, 1.20), focal=50, fstop=2.8, roll=0.0)
    rig.key(0.35, pos=(0.0, -1.0, 1.20), target=(0.0, 6.0, 1.20), focal=50, fstop=2.8)
    rig.key(0.82, pos=(0.0, -1.0, 1.20), target=(-0.02, 3.0, 1.21), focal=50, fstop=2.8, roll=0.0)
    rig.key(1.45, pos=(0.10, -2.05, 1.30), target=(0.0, LOGO1_Y, 1.18), focal=46, fstop=3.2, roll=math.radians(-1.2))
    rig.key(0.0, focus=1.0)
    rig.key(0.45, focus=1.0)
    rig.key(0.95, focus=1.05)
    rig.key(1.40, focus=2.7)
    rig.shake(t_click, (0.15, -1.0, 0.1), amp=0.028, freq=6.0, damp=9.0)

    return {"logo": lg, "cursor": c}
