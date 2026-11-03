"""Herramientas del mundo 3D de Kast (Blender 5.2): materiales físicos, geometría con biseles reales, texto,
curvas de luz, curvas de animación y horneado de la animación fotograma a fotograma.

Toda la animación se escribe como funciones del tiempo en Python y se hornea en claves por fotograma: así cada
movimiento tiene la curva exacta que se diseñó (anticipación, muelles, inercia) y el desenfoque de movimiento se
calcula con el movimiento real entre fotogramas.
"""
import math
import bpy
import bmesh
from mathutils import Vector, Matrix, Quaternion

FONTS = {
    "regular": "C:/Windows/Fonts/segoeui.ttf",
    "semibold": "C:/Windows/Fonts/seguisb.ttf",
    "bold": "C:/Windows/Fonts/segoeuib.ttf",
    "light": "C:/Windows/Fonts/segoeuil.ttf",
}


def srgb(h):
    """#RRGGBB -> color lineal (RGBA)."""
    h = h.lstrip("#")
    c = [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    lin = [x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
    return (*lin, 1.0)


# ---------------------------------------------------------------- curvas de animación

def clamp(x, a=0.0, b=1.0):
    return a if x < a else b if x > b else x


def lerp(a, b, t):
    return a + (b - a) * t


def ramp(t, t0, t1):
    """0 antes de t0, 1 después de t1, lineal entre medias."""
    if t1 <= t0:
        return 1.0 if t >= t1 else 0.0
    return clamp((t - t0) / (t1 - t0))


def smooth(x):
    x = clamp(x)
    return x * x * (3 - 2 * x)


def smoother(x):
    x = clamp(x)
    return x * x * x * (x * (x * 6 - 15) + 10)


def ease_in(x, p=3):
    return clamp(x) ** p


def ease_out(x, p=3):
    return 1 - (1 - clamp(x)) ** p


def ease_in_out(x, p=3):
    x = clamp(x)
    return 0.5 * (2 * x) ** p if x < 0.5 else 1 - 0.5 * (2 - 2 * x) ** p


def expo_out(x):
    x = clamp(x)
    return 1.0 if x >= 1 else 1 - 2 ** (-10 * x)


def back_out(x, s=1.4):
    x = clamp(x) - 1
    return x * x * ((s + 1) * x + s) + 1


def spring(t, freq=4.0, damp=6.0):
    """Respuesta de un muelle amortiguado a un escalón: 0 en t=0, oscila y se asienta en 1."""
    if t <= 0:
        return 0.0
    return 1 - math.exp(-damp * t) * math.cos(2 * math.pi * freq * t)


def impulse(t, freq=6.0, damp=9.0):
    """Sacudida amortiguada tras un golpe en t=0 (empieza en 0, sube y se apaga)."""
    if t <= 0:
        return 0.0
    return math.exp(-damp * t) * math.sin(2 * math.pi * freq * t)


def noise1(t, seed=0.0, octaves=3):
    """Ruido suave y determinista (suma de senos con fases fijas) para movimiento secundario."""
    v = 0.0
    a = 1.0
    f = 1.0
    for o in range(octaves):
        v += a * math.sin(t * f * 2.17 + seed * 13.1 + o * 1.7) * math.sin(t * f * 0.73 + seed * 7.3 + o * 2.9)
        a *= 0.5
        f *= 2.03
    return v


def hermite_track(keys):
    """Interpolación C1 de claves (t, valor) con tangentes de Catmull-Rom en el tiempo: la velocidad es continua al
    pasar por cada clave; dos claves seguidas con el mismo valor dejan el objeto quieto entre ellas.
    `valor` puede ser un número o una tupla."""
    ts = [k[0] for k in keys]
    vs = [k[1] if isinstance(k[1], (tuple, list, Vector)) else (k[1],) for k in keys]
    n = len(keys)
    dim = len(vs[0])

    def tangent(i):
        if i == 0 or i == n - 1:
            return [0.0] * dim
        # Si la clave es una pausa (igual a la anterior o a la siguiente), velocidad cero.
        if vs[i] == vs[i - 1] or vs[i] == vs[i + 1]:
            return [0.0] * dim
        dt = ts[i + 1] - ts[i - 1]
        return [(vs[i + 1][d] - vs[i - 1][d]) / dt for d in range(dim)]

    tans = [tangent(i) for i in range(n)]

    def f(t):
        if t <= ts[0]:
            out = vs[0]
        elif t >= ts[-1]:
            out = vs[-1]
        else:
            i = max(j for j in range(n - 1) if ts[j] <= t)
            t0, t1 = ts[i], ts[i + 1]
            h = t1 - t0
            s = (t - t0) / h
            h00 = 2 * s ** 3 - 3 * s ** 2 + 1
            h10 = s ** 3 - 2 * s ** 2 + s
            h01 = -2 * s ** 3 + 3 * s ** 2
            h11 = s ** 3 - s ** 2
            out = [h00 * vs[i][d] + h10 * h * tans[i][d] + h01 * vs[i + 1][d] + h11 * h * tans[i + 1][d] for d in range(dim)]
        return out[0] if dim == 1 else tuple(out)

    return f


# ---------------------------------------------------------------- horneado

class Baker:
    """Acumula valores por fotograma y los escribe como F-Curves de una vez (rápido)."""

    def __init__(self, frames, fps):
        self.frames = frames
        self.fps = fps
        self.tracks = []

    def times(self):
        return [f / self.fps for f in range(self.frames)]

    def add(self, owner, path, fn, index=None, const=False):
        """owner: un ID (objeto, material, datos) o un struct de un ID (p. ej. un socket de un nodo).
        `fn(t)` da el valor; con index=None y valores tupla se hornea cada componente."""
        self.tracks.append((owner, path, fn, index, const))

    def bake(self):
        ts = self.times()
        for owner, path, fn, index, const in self.tracks:
            vals = [fn(t) for t in ts]
            multi = isinstance(vals[0], (tuple, list, Vector))
            comps = range(len(vals[0])) if multi else [index if index is not None else -1]
            id_data = owner.id_data
            full = owner.path_from_id(path) if owner != id_data else path
            if id_data.animation_data is None:
                id_data.animation_data_create()
            if id_data.animation_data.action is None:
                act = bpy.data.actions.new(id_data.name + "Accion")
                id_data.animation_data.action = act
            act = id_data.animation_data.action
            for ci, comp in enumerate(comps):
                idx = comp if comp is not None and comp >= 0 else 0
                fc = act.fcurve_ensure_for_datablock(id_data, full, index=idx) if hasattr(act, "fcurve_ensure_for_datablock") else None
                if fc is None:
                    fc = act.fcurves.new(full, index=idx)
                series = [v[ci] if multi else v for v in vals]
                # Se omiten las claves redundantes (valor constante): menos datos, mismo resultado.
                keep = [0]
                for i in range(1, len(series) - 1):
                    if not (abs(series[i] - series[i - 1]) < 1e-7 and abs(series[i + 1] - series[i]) < 1e-7):
                        keep.append(i)
                keep.append(len(series) - 1)
                fc.keyframe_points.add(len(keep))
                co = []
                for i in keep:
                    co += [float(i), float(series[i])]
                fc.keyframe_points.foreach_set("co", co)
                interp = 0 if const else 1  # CONSTANT=0, LINEAR=1
                fc.keyframe_points.foreach_set("interpolation", [interp] * len(keep))
                fc.update()


# ---------------------------------------------------------------- escena

def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def collection(name, parent=None):
    c = bpy.data.collections.new(name)
    (parent or bpy.context.scene.collection).children.link(c)
    return c


_current = [None]


def into(coll):
    _current[0] = coll


def link(obj):
    (_current[0] or bpy.context.scene.collection).objects.link(obj)
    return obj


def empty(name, loc=(0, 0, 0), parent=None):
    o = bpy.data.objects.new(name, None)
    o.location = loc
    if parent:
        o.parent = parent
    return link(o)


# ---------------------------------------------------------------- materiales

def _bsdf(m):
    return next(n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED")


def mat(name, base="#808080", metallic=0.0, rough=0.4, coat=0.0, coat_rough=0.05, emit=None, strength=0.0, aniso=0.0,
        spec=0.5, alpha=1.0):
    m = bpy.data.materials.new(name)
    b = _bsdf(m)
    b.inputs["Base Color"].default_value = srgb(base) if isinstance(base, str) else base
    b.inputs["Metallic"].default_value = metallic
    b.inputs["Roughness"].default_value = rough
    b.inputs["Coat Weight"].default_value = coat
    b.inputs["Coat Roughness"].default_value = coat_rough
    b.inputs["Specular IOR Level"].default_value = spec
    b.inputs["Anisotropic"].default_value = aniso
    if emit is not None:
        b.inputs["Emission Color"].default_value = srgb(emit) if isinstance(emit, str) else emit
        b.inputs["Emission Strength"].default_value = strength
    if alpha < 1:
        b.inputs["Alpha"].default_value = alpha
    return m


def emission_mat(name, color="#FFFFFF", strength=1.0):
    """Emisión pura (líneas de luz, filos). Strength se anima con mat_input(m, 'Strength')."""
    m = bpy.data.materials.new(name)
    nt = m.node_tree
    for n in list(nt.nodes):
        nt.nodes.remove(n)
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Color"].default_value = srgb(color) if isinstance(color, str) else color
    em.inputs["Strength"].default_value = strength
    nt.links.new(em.outputs[0], out.inputs["Surface"])
    return m


def mat_input(m, name):
    """Socket de entrada animable del nodo principal (Principled o Emission)."""
    for n in m.node_tree.nodes:
        if n.type in ("BSDF_PRINCIPLED", "EMISSION") and name in n.inputs:
            return n.inputs[name]
    raise KeyError(name)


def screen_mat(name, image_path, strength=1.0):
    """Pantalla: cristal con reflejo amplio y la imagen como emisión (su brillo se anima)."""
    m = mat(name, base="#050506", rough=0.18, coat=1.0, coat_rough=0.03, spec=0.5)
    nt = m.node_tree
    b = _bsdf(m)
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images.load(image_path, check_existing=True)
    tex.interpolation = "Cubic"
    tex.extension = "CLIP"
    nt.links.new(tex.outputs["Color"], b.inputs["Emission Color"])
    b.inputs["Emission Strength"].default_value = strength
    return m


def graphite_tile_mat(name):
    """El grafito del logo: degradado vertical #3A3A40 -> #161618, satinado con capa transparente."""
    m = mat(name, base="#2a2a2f", metallic=0.0, rough=0.38, coat=0.6, coat_rough=0.12)
    nt = m.node_tree
    b = _bsdf(m)
    tc = nt.nodes.new("ShaderNodeTexCoord")
    sep = nt.nodes.new("ShaderNodeSeparateXYZ")
    mix = nt.nodes.new("ShaderNodeMix")
    mix.data_type = "RGBA"
    nt.links.new(tc.outputs["Generated"], sep.inputs[0])
    nt.links.new(sep.outputs["Z"], mix.inputs["Factor"])
    mix.inputs["A"].default_value = srgb("#161618")
    mix.inputs["B"].default_value = srgb("#3A3A40")
    nt.links.new(mix.outputs["Result"], b.inputs["Base Color"])
    return m


def assign(obj, m, slot=None):
    if obj.data is None:
        return
    if slot is None:
        obj.data.materials.clear()
        obj.data.materials.append(m)
    else:
        while len(obj.data.materials) <= slot:
            obj.data.materials.append(m)
        obj.data.materials[slot] = m


# ---------------------------------------------------------------- geometría

def box(name, size, radius=0.0, segments=6, loc=(0, 0, 0), mats=None):
    """Caja con bisel real (modificador) y normales ponderadas: bordes perfectos a cualquier distancia."""
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    for v in bm.verts:
        v.co = Vector((v.co.x * size[0], v.co.y * size[1], v.co.z * size[2]))
    bm.to_mesh(me)
    bm.free()
    o = link(bpy.data.objects.new(name, me))
    o.location = loc
    if radius > 0:
        bev = o.modifiers.new("Bisel", "BEVEL")
        bev.width = min(radius, min(size) / 2 - 1e-4)
        bev.segments = segments
        bev.limit_method = "ANGLE"
        bev.harden_normals = False
        o.modifiers.new("Normales", "WEIGHTED_NORMAL").keep_sharp = True
    for p in me.polygons:
        p.use_smooth = True
    if mats:
        assign(o, mats)
    return o


def rounded_rect_points(w, h, r, seg=8):
    """Contorno de un rectángulo redondeado (plano XZ, centrado), sentido antihorario visto desde -Y."""
    r = min(r, w / 2, h / 2)
    pts = []
    corners = [(w / 2 - r, h / 2 - r, 0), (-w / 2 + r, h / 2 - r, 90), (-w / 2 + r, -h / 2 + r, 180), (w / 2 - r, -h / 2 + r, 270)]
    for cx, cz, a0 in corners:
        for i in range(seg + 1):
            a = math.radians(a0 + 90 * i / seg)
            pts.append((cx + r * math.cos(a), cz + r * math.sin(a)))
    return pts


def slab(name, w, h, depth, r, bevel=0.0, loc=(0, 0, 0), mats=None, seg=10):
    """Placa redondeada (tarjeta, pantalla, baldosa) en el plano XZ, con espesor en Y y canto biselado."""
    pts = rounded_rect_points(w, h, r, seg)
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    front = [bm.verts.new((x, -depth / 2, z)) for x, z in pts]
    back = [bm.verts.new((x, depth / 2, z)) for x, z in pts]
    bm.faces.new(front[::-1])
    bm.faces.new(back)
    n = len(pts)
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((front[i], front[j], back[j], back[i]))
    bm.normal_update()
    bm.to_mesh(me)
    bm.free()
    o = link(bpy.data.objects.new(name, me))
    o.location = loc
    if bevel > 0:
        bev = o.modifiers.new("Bisel", "BEVEL")
        bev.width = bevel
        bev.segments = 5
        bev.limit_method = "ANGLE"
        bev.angle_limit = math.radians(50)
        o.modifiers.new("Normales", "WEIGHTED_NORMAL").keep_sharp = True
    for p in me.polygons:
        p.use_smooth = True
    if mats:
        assign(o, mats)
    return o


def plane(name, w, h, loc=(0, 0, 0), mats=None):
    """Plano XZ mirando a -Y con UV 0..1 (pantallas, imágenes)."""
    me = bpy.data.meshes.new(name)
    verts = [(-w / 2, 0, -h / 2), (w / 2, 0, -h / 2), (w / 2, 0, h / 2), (-w / 2, 0, h / 2)]
    me.from_pydata(verts, [], [(0, 1, 2, 3)])
    uv = me.uv_layers.new()
    for i, c in enumerate([(0, 0), (1, 0), (1, 1), (0, 1)]):
        uv.data[i].uv = c
    o = link(bpy.data.objects.new(name, me))
    o.location = loc
    if mats:
        assign(o, mats)
    return o


def disc(name, radius, loc=(0, 0, 0), mats=None, seg=96):
    """Disco horizontal (escenario) en el plano XY."""
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_circle(bm, cap_ends=True, radius=radius, segments=seg)
    bm.to_mesh(me)
    bm.free()
    o = link(bpy.data.objects.new(name, me))
    o.location = loc
    if mats:
        assign(o, mats)
    return o


def curve(name, pts, bevel=0.004, res=6, closed=False, mats=None, smooth=True, fill_caps=True, loc=(0, 0, 0)):
    """Curva 3D con grosor (línea de luz, trazos). Bezier con asas automáticas si `smooth`."""
    cu = bpy.data.curves.new(name, "CURVE")
    cu.dimensions = "3D"
    cu.bevel_depth = bevel
    cu.bevel_resolution = res
    cu.use_fill_caps = fill_caps
    cu.resolution_u = 24
    cu.bevel_factor_mapping_start = "SPLINE"
    cu.bevel_factor_mapping_end = "SPLINE"
    if smooth:
        sp = cu.splines.new("BEZIER")
        sp.bezier_points.add(len(pts) - 1)
        for bp, p in zip(sp.bezier_points, pts):
            bp.co = p
            bp.handle_left_type = bp.handle_right_type = "AUTO"
    else:
        sp = cu.splines.new("POLY")
        sp.points.add(len(pts) - 1)
        for pt, p in zip(sp.points, pts):
            pt.co = (*p, 1.0)
    sp.use_cyclic_u = closed
    o = link(bpy.data.objects.new(name, cu))
    o.location = loc
    if mats:
        assign(o, mats)
    return o


def text(name, body, font="semibold", size=0.1, align="CENTER", valign="CENTER", extrude=0.0, mats=None, loc=(0, 0, 0),
         spacing=1.0):
    cu = bpy.data.curves.new(name, "FONT")
    cu.body = body
    cu.font = bpy.data.fonts.load(FONTS[font], check_existing=True)
    cu.size = size
    cu.align_x = align
    cu.align_y = valign
    cu.extrude = extrude
    cu.space_character = spacing
    cu.resolution_u = 8
    o = link(bpy.data.objects.new(name, cu))
    o.location = loc
    o.rotation_euler = (math.radians(90), 0, 0)   # de pie, mirando a -Y
    if mats:
        assign(o, mats)
    return o


def area_light(name, loc, target, power, size=1.0, color="#FFFFFF", size_y=None, spread=180):
    ld = bpy.data.lights.new(name, "AREA")
    ld.energy = power
    ld.color = srgb(color)[:3]
    ld.shape = "RECTANGLE" if size_y else "SQUARE"
    ld.size = size
    if size_y:
        ld.size_y = size_y
    ld.spread = math.radians(spread)
    o = link(bpy.data.objects.new(name, ld))
    o.location = loc
    o.visible_glossy = False
    o.visible_camera = False
    o.visible_transmission = False
    look_at(o, target)
    return o


def look_at(obj, target, roll=0.0):
    d = Vector(target) - Vector(obj.location)
    q = d.to_track_quat("-Z", "Y")
    if roll:
        q = q @ Quaternion((0, 0, 1), roll)
    obj.rotation_mode = "QUATERNION"
    obj.rotation_quaternion = q


def look_quat(eye, target, roll=0.0):
    d = Vector(target) - Vector(eye)
    q = d.to_track_quat("-Z", "Y")
    if roll:
        q = q @ Quaternion((0, 0, 1), roll)
    return q


# ---------------------------------------------------------------- pantallas con más control

def _img(nt, path):
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images.load(path, check_existing=True)
    tex.interpolation = "Cubic"
    tex.extension = "CLIP"
    return tex


def screen_mat_tint(name, image_path, strength=1.0):
    """Pantalla cuya imagen se multiplica por un color (tono y brillo de cada monitor al calibrar).
    Devuelve (material, socket del tono, socket de la intensidad)."""
    m = mat(name, base="#050506", rough=0.18, coat=1.0, coat_rough=0.03, spec=0.5)
    nt = m.node_tree
    b = _bsdf(m)
    tex = _img(nt, image_path)
    mix = nt.nodes.new("ShaderNodeMix")
    mix.data_type = "RGBA"
    mix.blend_type = "MULTIPLY"
    mix.inputs["Factor"].default_value = 1.0
    nt.links.new(tex.outputs["Color"], mix.inputs["A"])
    mix.inputs["B"].default_value = (1, 1, 1, 1)
    nt.links.new(mix.outputs["Result"], b.inputs["Emission Color"])
    b.inputs["Emission Strength"].default_value = strength
    return m, mix.inputs["B"], b.inputs["Emission Strength"]


def screen_mat_wipe(name, path_a, path_b, strength=1.0, tint_b=None):
    """Pantalla que pasa de la imagen A a la B con un barrido de izquierda a derecha y un filo de luz.
    Devuelve (material, socket del barrido 0..1, socket de la intensidad)."""
    m = mat(name, base="#050506", rough=0.18, coat=1.0, coat_rough=0.03, spec=0.5)
    nt = m.node_tree
    b = _bsdf(m)
    ta, tb = _img(nt, path_a), _img(nt, path_b)
    uv = nt.nodes.new("ShaderNodeTexCoord")
    sep = nt.nodes.new("ShaderNodeSeparateXYZ")
    nt.links.new(uv.outputs["UV"], sep.inputs[0])
    val = nt.nodes.new("ShaderNodeValue")
    # factor = smoothstep(x, wipe-0.02, wipe): 1 a la izquierda del frente (ya es B)
    mr = nt.nodes.new("ShaderNodeMapRange")
    mr.interpolation_type = "SMOOTHSTEP"
    nt.links.new(sep.outputs["X"], mr.inputs["Value"])
    sub = nt.nodes.new("ShaderNodeMath")
    sub.operation = "SUBTRACT"
    nt.links.new(val.outputs[0], sub.inputs[0])
    sub.inputs[1].default_value = 0.03
    nt.links.new(sub.outputs[0], mr.inputs["From Min"])
    nt.links.new(val.outputs[0], mr.inputs["From Max"])
    mr.inputs["To Min"].default_value = 1.0
    mr.inputs["To Max"].default_value = 0.0
    mix = nt.nodes.new("ShaderNodeMix")
    mix.data_type = "RGBA"
    nt.links.new(mr.outputs["Result"], mix.inputs["Factor"])
    nt.links.new(ta.outputs["Color"], mix.inputs["A"])
    src_b = tb.outputs["Color"]
    if tint_b:
        tm = nt.nodes.new("ShaderNodeMix")
        tm.data_type = "RGBA"
        tm.blend_type = "MULTIPLY"
        tm.inputs["Factor"].default_value = 1.0
        nt.links.new(tb.outputs["Color"], tm.inputs["A"])
        tm.inputs["B"].default_value = srgb(tint_b)
        src_b = tm.outputs["Result"]
    nt.links.new(src_b, mix.inputs["B"])
    # Filo de luz en el frente del barrido.
    edge = nt.nodes.new("ShaderNodeMath")
    edge.operation = "PINGPONG" if False else "MULTIPLY"
    e1 = nt.nodes.new("ShaderNodeMath")
    e1.operation = "SUBTRACT"
    nt.links.new(sep.outputs["X"], e1.inputs[0])
    nt.links.new(val.outputs[0], e1.inputs[1])
    e2 = nt.nodes.new("ShaderNodeMath")
    e2.operation = "ABSOLUTE"
    nt.links.new(e1.outputs[0], e2.inputs[0])
    e3 = nt.nodes.new("ShaderNodeMapRange")
    nt.links.new(e2.outputs[0], e3.inputs["Value"])
    e3.inputs["From Min"].default_value = 0.0
    e3.inputs["From Max"].default_value = 0.012
    e3.inputs["To Min"].default_value = 3.0
    e3.inputs["To Max"].default_value = 0.0
    add = nt.nodes.new("ShaderNodeMix")
    add.data_type = "RGBA"
    add.blend_type = "ADD"
    nt.links.new(e3.outputs["Result"], add.inputs["Factor"])
    nt.links.new(mix.outputs["Result"], add.inputs["A"])
    add.inputs["B"].default_value = srgb("#C9D1FF")
    nt.links.new(add.outputs["Result"], b.inputs["Emission Color"])
    b.inputs["Emission Strength"].default_value = strength
    val.outputs[0].default_value = 0.0
    return m, val.outputs[0], b.inputs["Emission Strength"]


def image_plane(name, image_path, w, h, strength=1.0, color=None, loc=(0, 0, 0)):
    """Imagen con transparencia que emite luz (iconos, rótulos). Devuelve (objeto, socket de intensidad)."""
    m = bpy.data.materials.new(name + "M")
    nt = m.node_tree
    for n in list(nt.nodes):
        nt.nodes.remove(n)
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    tex = _img(nt, image_path)
    em = nt.nodes.new("ShaderNodeEmission")
    if color:
        mul = nt.nodes.new("ShaderNodeMix")
        mul.data_type = "RGBA"
        mul.blend_type = "MULTIPLY"
        mul.inputs["Factor"].default_value = 1.0
        nt.links.new(tex.outputs["Color"], mul.inputs["A"])
        mul.inputs["B"].default_value = srgb(color)
        nt.links.new(mul.outputs["Result"], em.inputs["Color"])
    else:
        nt.links.new(tex.outputs["Color"], em.inputs["Color"])
    em.inputs["Strength"].default_value = strength
    tr = nt.nodes.new("ShaderNodeBsdfTransparent")
    mx = nt.nodes.new("ShaderNodeMixShader")
    nt.links.new(tex.outputs["Alpha"], mx.inputs[0])
    nt.links.new(tr.outputs[0], mx.inputs[1])
    nt.links.new(em.outputs[0], mx.inputs[2])
    nt.links.new(mx.outputs[0], out.inputs["Surface"])
    o = plane(name, w, h, loc=loc, mats=m)
    o.visible_shadow = False
    return o, em.inputs["Strength"]


def flip_counter(name, baker, values, size, mats, loc, cell=None, flip=0.09):
    """Contador mecánico de ancho fijo: cada carácter en su celda; al cambiar el valor, el carácter viejo gira hacia
    atrás y el nuevo entra girando desde abajo, con un rebote mínimo. values = [(t, "1920 × 1080"), ...]."""
    cell = cell or size * 0.62
    n = max(len(v) for _, v in values)
    root = empty(name, loc=loc)
    width = n * cell
    for i in range(n):
        x = -width / 2 + cell * (i + 0.5)
        # Glifos de esta celda a lo largo del tiempo: [(t_on, t_off, char)]
        spans = []
        for k, (t, v) in enumerate(values):
            ch = v.ljust(n)[i]
            t_next = values[k + 1][0] if k + 1 < len(values) else 1e9
            if spans and spans[-1][2] == ch:
                spans[-1] = (spans[-1][0], t_next, ch)
            else:
                spans.append((t, t_next, ch))
        for j, (t_on, t_off, ch) in enumerate(spans):
            if ch == " ":
                continue
            o = text(f"{name}_{i}_{j}", ch, font="semibold", size=size, mats=mats)
            o.parent = root
            o.location = (x, 0, 0)
            first = j == 0
            o.rotation_mode = "XYZ"

            def rot(t, t_on=t_on, t_off=t_off, first=first):
                a = 0.0
                if not first and t < t_on + flip:
                    u = ramp(t, t_on, t_on + flip)
                    a = math.radians(-90) * (1 - back_out(u, 1.6))
                if t > t_off:
                    u = ramp(t, t_off - flip * 0.5, t_off + flip * 0.5)
                    a = math.radians(90) * ease_in(u, 2)
                return (math.radians(90) + a, 0.0, 0.0)

            def vis(t, t_on=t_on, t_off=t_off, first=first):
                on = (first or t >= t_on) and t <= t_off + flip * 0.5
                return (1.0, 1.0, 1.0) if on else (0.0, 0.0, 0.0)

            baker.add(o, "rotation_euler", rot)
            baker.add(o, "scale", vis, const=True)
    return root
