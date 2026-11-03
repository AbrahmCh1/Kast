"""Objetos de marca y de producto: el cursor (protagonista), el logo de Kast, monitores, teléfono, Mac mini y la
pantalla interactiva. Medidas en metros."""
import math
import bmesh
import bpy
from mathutils import Vector
from . import kit

LAVENDER = "#8197FF"
BLUE = "#4361EE"
VIOLET = "#A78BFA"
GREEN = "#6CCB5F"

# ---------------------------------------------------------------- materiales compartidos

_M = {}


def M(name):
    if name in _M:
        return _M[name]
    m = {
        "ceramic": lambda: kit.mat("Ceramica", "#F2F2F0", rough=0.28, coat=0.5, coat_rough=0.08, spec=0.5),
        "graphite": lambda: kit.mat("Grafito", "#1B1B1E", rough=0.32, coat=0.4, coat_rough=0.1),
        "anodized": lambda: kit.mat("Anodizado", "#3A3A40", metallic=1.0, rough=0.34, aniso=0.55),
        "aluminum": lambda: kit.mat("Aluminio", "#C9CBCF", metallic=1.0, rough=0.28, aniso=0.5),
        "bezel": lambda: kit.mat("Bisel", "#070708", rough=0.22, coat=0.5),
        "card": lambda: kit.mat("Tarjeta", "#26262A", rough=0.62, coat=0.15, coat_rough=0.3),
        "card_light": lambda: kit.mat("TarjetaClara", "#2E2E33", rough=0.55, coat=0.2, coat_rough=0.25),
        "floor": lambda: kit.mat("Suelo", "#0C0C0E", rough=0.30, coat=0.22, coat_rough=0.14, spec=0.3),
        "white_text": lambda: kit.mat("TextoBlanco", "#FFFFFF", rough=0.5, emit="#FFFFFF", strength=1.6),
        "sub_text": lambda: kit.mat("TextoGris", "#A9A9A9", rough=0.5, emit="#A9A9A9", strength=0.9),
        "glyph": lambda: kit.mat("Glifo", "#F4F4F2", rough=0.3, coat=0.3, emit="#FFFFFF", strength=0.55),
    }[name]()
    _M[name] = m
    return m


# ---------------------------------------------------------------- el cursor

ARROW = [(0, 0), (0, 16.6), (3.9, 12.9), (6.6, 19.3), (9.25, 18.15), (6.55, 11.8), (11.8, 11.8)]
UNIT = 0.0105          # 19,3 unidades = 0,203 m
STEM_ROOT = (5.2, 12.35)


def _offset(poly, d):
    """Desplaza un polígono hacia fuera `d` unidades (inglete)."""
    n = len(poly)
    # orientación
    area = sum(poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1] for i in range(n))
    sgn = 1 if area > 0 else -1
    out = []
    for i in range(n):
        p0, p1, p2 = Vector(poly[i - 1]), Vector(poly[i]), Vector(poly[(i + 1) % n])
        e1 = (p1 - p0).normalized()
        e2 = (p2 - p1).normalized()
        n1 = Vector((e1.y, -e1.x)) * sgn
        n2 = Vector((e2.y, -e2.x)) * sgn
        m = (n1 + n2)
        if m.length < 1e-6:
            m = n1
        m.normalize()
        k = d / max(0.35, m.dot(n1))
        out.append(tuple(p1 + m * k))
    return out


def _prism(name, poly, y0, y1):
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    f = [bm.verts.new((x * UNIT, y0, -z * UNIT)) for x, z in poly]
    b = [bm.verts.new((x * UNIT, y1, -z * UNIT)) for x, z in poly]
    bm.faces.new(f)
    bm.faces.new(b[::-1])
    n = len(poly)
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((f[j], f[i], b[i], b[j]))
    bm.normal_update()
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(me)
    bm.free()
    for p in me.polygons:
        p.use_smooth = True
    return me


def _stem_keys(o, angle_deg, squash):
    """Claves de forma: el tallo gira con retraso (izquierda / derecha) y el cuerpo se comprime al hacer clic."""
    me = o.data
    o.shape_key_add(name="Base")
    rx, rz = STEM_ROOT[0] * UNIT, -STEM_ROOT[1] * UNIT
    for kname, a in (("TalloIzq", angle_deg), ("TalloDer", -angle_deg)):
        k = o.shape_key_add(name=kname)
        ca, sa = math.cos(math.radians(a)), math.sin(math.radians(a))
        for i, v in enumerate(me.vertices):
            x, y, z = v.co
            w = kit.smooth((-(z) / UNIT - 11.0) / 3.0)   # 0 en la cabeza, 1 en el tallo
            if w <= 0:
                continue
            dx, dz = x - rx, z - rz
            nx, nz = rx + dx * ca - dz * sa, rz + dx * sa + dz * ca
            k.data[i].co = (x + (nx - x) * w, y, z + (nz - z) * w)
    k = o.shape_key_add(name="Aplastar")
    for i, v in enumerate(me.vertices):
        x, y, z = v.co
        # Desde la punta (origen): más ancho y más corto, conservando el volumen.
        k.data[i].co = (x * (1 + 0.16 * squash), y * (1 + 0.1 * squash), z * (1 - 0.18 * squash))


def cursor(name="Cursor"):
    """La flecha: cuerpo de cerámica blanca sobre una base grafito, filo de luz lavanda. Origen en la punta."""
    root = kit.empty(name)
    base_poly = _offset(ARROW, 1.05)
    base = kit.link(bpy.data.objects.new(name + "Base", _prism(name + "Base", base_poly, 0.0, 0.016)))
    body = kit.link(bpy.data.objects.new(name + "Cuerpo", _prism(name + "Cuerpo", ARROW, -0.008, 0.004)))
    for o, r in ((base, 0.0035), (body, 0.004)):
        bev = o.modifiers.new("Bisel", "BEVEL")
        bev.width = r
        bev.segments = 5
        bev.limit_method = "ANGLE"
        bev.angle_limit = math.radians(40)
        o.modifiers.new("Normales", "WEIGHTED_NORMAL").keep_sharp = True
        o.parent = root
        _stem_keys(o, 13, 1.0)
    edge_mat = rim_mat(name + "BaseM")
    kit.assign(base, edge_mat)
    kit.assign(body, M("ceramic"))
    edge = None
    # Barra de texto («I») para el parpadeo.
    ibeam = kit.empty(name + "Barra")
    ibeam.parent = root
    ip = [(-0.6, 0), (0.6, 0), (0.6, 18), (-0.6, 18)]
    stem = kit.link(bpy.data.objects.new(name + "BarraI", _prism(name + "BarraI", ip, -0.006, 0.006)))
    stem.parent = ibeam
    for zz in (0.0, 18.0):
        serif = [(-3.0, zz - 0.6), (3.0, zz - 0.6), (3.0, zz + 0.6), (-3.0, zz + 0.6)]
        s = kit.link(bpy.data.objects.new(name + "Serifa", _prism(name + "Serifa", serif, -0.006, 0.006)))
        s.parent = ibeam
        kit.assign(s, M("ceramic"))
    kit.assign(stem, M("ceramic"))
    ibeam.location = (5.0 * UNIT, 0, 0)
    ibeam.scale = (0.0, 1, 1)
    return {"root": root, "base": base, "body": body, "edge": edge, "edge_mat": edge_mat, "ibeam": ibeam}


def rim_mat(name):
    """Grafito con un filo de luz lavanda en los cantos (según la normal): sigue cualquier deformación.
    Su intensidad se anima con kit.mat_input(m, 'Emission Strength')."""
    m = kit.mat(name, "#1B1B1E", rough=0.32, coat=0.4, coat_rough=0.1, emit=LAVENDER, strength=0.0)
    nt = m.node_tree
    b = next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")
    geo = nt.nodes.new("ShaderNodeNewGeometry")
    vt = nt.nodes.new("ShaderNodeVectorTransform")
    vt.vector_type = "NORMAL"
    vt.convert_from = "WORLD"
    vt.convert_to = "OBJECT"
    sep = nt.nodes.new("ShaderNodeSeparateXYZ")
    ab = nt.nodes.new("ShaderNodeMath")
    ab.operation = "ABSOLUTE"
    inv = nt.nodes.new("ShaderNodeMath")
    inv.operation = "SUBTRACT"
    inv.inputs[0].default_value = 1.0
    pw = nt.nodes.new("ShaderNodeMath")
    pw.operation = "POWER"
    pw.inputs[1].default_value = 3.0
    mix = nt.nodes.new("ShaderNodeMix")
    mix.data_type = "RGBA"
    mix.inputs["A"].default_value = (0, 0, 0, 1)
    mix.inputs["B"].default_value = kit.srgb(LAVENDER)
    # Los cantos son las caras que no miran a ±Y (en coordenadas del objeto).
    nt.links.new(geo.outputs["Normal"], vt.inputs[0])
    nt.links.new(vt.outputs[0], sep.inputs[0])
    nt.links.new(sep.outputs["Y"], ab.inputs[0])
    nt.links.new(ab.outputs[0], inv.inputs[1])
    nt.links.new(inv.outputs[0], pw.inputs[0])
    nt.links.new(pw.outputs[0], mix.inputs["Factor"])
    nt.links.new(mix.outputs["Result"], b.inputs["Emission Color"])
    return m


def cursor_shape(c, baker, stem_fn, squash_fn):
    """Anima las claves de forma de las dos capas del cursor: tallo (-1..1) y aplastamiento (0..1)."""
    for o in (c["base"], c["body"]):
        kb = o.data.shape_keys.key_blocks
        baker.add(kb["TalloIzq"], "value", lambda t: max(0.0, stem_fn(t)))
        baker.add(kb["TalloDer"], "value", lambda t: max(0.0, -stem_fn(t)))
        baker.add(kb["Aplastar"], "value", squash_fn)


# ---------------------------------------------------------------- logo de Kast (assets/kast.svg)

def logo(name="Logo", size=0.6):
    """La baldosa grafito con el monitor: contorno blanco, mitad derecha encendida y pie. Coordenadas del SVG
    (viewBox 64, baldosa de 58) escaladas a `size` metros. Devuelve las piezas para animar el trazado."""
    s = size / 58.0

    def P(x, y, yy=0.0):
        return ((x - 32) * s, yy, (32 - y) * s)

    root = kit.empty(name)
    tile = kit.slab(name + "Baldosa", 58 * s, 58 * s, 0.05 * size, 14 * s, bevel=0.012 * size, mats=kit.graphite_tile_mat(name + "Grafito"))
    tile.location = (0, 0.025 * size, 0)
    tile.parent = root
    # Filo blanco al 11 % (como el del SVG).
    hair = kit.curve(name + "Filo", [(x, -0.0005, z) for x, z in kit.rounded_rect_points(57.5 * s, 57.5 * s, 13.75 * s, 10)],
                     bevel=0.0012 * size / 0.6, res=2, closed=True, smooth=False, mats=kit.emission_mat(name + "FiloM", "#FFFFFF", 0.0))
    hair.parent = root
    gl = M("glyph")
    # Contorno del monitor: empieza arriba en el centro y recorre el rectángulo (para trazarlo con la línea).
    rr = kit.rounded_rect_points(36 * s, 25 * s, 6 * s, 10)
    cz = (32 - 28.5) * s
    pts = [(x, -0.004 * size, z + cz) for x, z in rr]
    # Girar la lista para que empiece en el centro superior.
    top = min(range(len(pts)), key=lambda i: abs(pts[i][0]) + abs(pts[i][2] - (cz + 12.5 * s)) * 3)
    pts = pts[top:] + pts[:top]
    outline = kit.curve(name + "Contorno", pts + [pts[0]], bevel=2 * s, res=6, smooth=False, mats=gl)
    outline.parent = root
    # Mitad derecha encendida (x 32..50, y 16..41), esquinas derechas redondeadas.
    half = []
    for i in range(11):
        a = math.radians(90 - 90 * i / 10)
        half.append((44 + 6 * math.cos(a), 22 - 6 * math.sin(a)))
    for i in range(11):
        a = math.radians(0 - 90 * i / 10)
        half.append((44 + 6 * math.cos(a), 35 - 6 * math.sin(a)))
    half = [(32, 16)] + half + [(32, 41)]
    me = bpy.data.meshes.new(name + "Mitad")
    bm = bmesh.new()
    f = [bm.verts.new(P(x, y, -0.005 * size)) for x, y in half]
    b = [bm.verts.new(P(x, y, 0.0)) for x, y in half]
    bm.faces.new(f[::-1])
    bm.faces.new(b)
    for i in range(len(half)):
        j = (i + 1) % len(half)
        bm.faces.new((f[i], f[j], b[j], b[i]))
    bm.normal_update()
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(me)
    bm.free()
    fill = kit.link(bpy.data.objects.new(name + "Mitad", me))
    kit.assign(fill, gl)
    fill.parent = root
    # La mitad crece desde el centro del monitor hacia la derecha: origen en x = 32.
    # Pie (24..40, y=49) con extremos redondeados.
    stand = kit.curve(name + "Pie", [P(24, 49, -0.004 * size), P(32, 49, -0.004 * size), P(40, 49, -0.004 * size)],
                      bevel=2 * s, res=6, smooth=False, mats=gl)
    stand.parent = root
    return {"root": root, "tile": tile, "hair": hair, "outline": outline, "fill": fill, "stand": stand, "s": s}


# ---------------------------------------------------------------- monitores y dispositivos

def monitor(name, w=0.62, h=0.36, image=None, stand=True, portrait=False):
    """Monitor de aluminio anodizado: marco, pantalla de cristal (emisión animable) y pie de fundición.
    Origen en la base del pie (sobre el suelo). Devuelve el objeto raíz, la pantalla y su material."""
    root = kit.empty(name)
    head = kit.empty(name + "Cabeza", parent=root)
    neck_h = 0.16 if stand else 0.0
    head.location = (0, 0, neck_h + h / 2 + 0.02)
    body = kit.slab(name + "Cuerpo", w + 0.016, h + 0.016, 0.022, 0.012, bevel=0.004, mats=M("anodized"))
    body.parent = head
    body.location = (0, 0.008, 0)
    bez = kit.slab(name + "Marco", w + 0.006, h + 0.006, 0.004, 0.008, bevel=0.0015, mats=M("bezel"))
    bez.parent = head
    bez.location = (0, -0.004, 0)
    scr = kit.plane(name + "Pantalla", w, h)
    scr.parent = head
    scr.location = (0, -0.0065, 0)
    sm = kit.screen_mat(name + "PantallaM", image, 0.0) if image else kit.mat(name + "PantallaM", "#050506", rough=0.15, coat=1.0)
    kit.assign(scr, sm)
    led = kit.box(name + "LED", (0.006, 0.002, 0.002), loc=(w / 2 - 0.02, -0.006, -h / 2 - 0.006),
                  mats=kit.emission_mat(name + "LEDM", "#FFB347", 0.0))
    led.parent = head
    parts = {"root": root, "head": head, "screen": scr, "screen_mat": sm, "led": led}
    if stand:
        neck = kit.box(name + "Cuello", (0.045, 0.02, neck_h + 0.05), radius=0.006, loc=(0, 0.03, (neck_h + 0.05) / 2),
                       mats=M("anodized"))
        neck.parent = root
        foot = kit.slab(name + "Pie", 0.24, 0.16, 0.012, 0.03, bevel=0.003, mats=M("anodized"))
        foot.rotation_euler = (math.radians(90), 0, 0)
        foot.location = (0, 0.03, 0.006)
        foot.parent = root
        parts["neck"] = neck
        parts["foot"] = foot
    return parts


def phone(name, image=None, w=0.072, h=0.150):
    root = kit.empty(name)
    body = kit.slab(name + "Cuerpo", w, h, 0.008, 0.011, bevel=0.0025, mats=M("graphite"))
    body.parent = root
    scr = kit.plane(name + "Pantalla", w - 0.006, h - 0.006)
    scr.location = (0, -0.0042, 0)
    scr.parent = root
    sm = kit.screen_mat(name + "PantallaM", image, 0.0) if image else kit.mat(name + "PantallaM", "#050506", rough=0.15, coat=1.0)
    kit.assign(scr, sm)
    return {"root": root, "screen": scr, "screen_mat": sm}


def mac_mini(name):
    root = kit.empty(name)
    b = kit.box(name + "Cuerpo", (0.197, 0.197, 0.036), radius=0.012, loc=(0, 0, 0.018), mats=M("aluminum"))
    b.parent = root
    led = kit.box(name + "LED", (0.003, 0.002, 0.003), loc=(0.08, -0.099, 0.02), mats=kit.emission_mat(name + "LEDM", "#FFFFFF", 0.0))
    led.parent = root
    return {"root": root, "led": led}
