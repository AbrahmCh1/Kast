"""El mundo de Kast: ambiente, render, composición y la cámara única. Las estaciones (escenas) están en stations.py."""
import math
import bpy
from mathutils import Vector
from . import kit
from .kit import srgb

# Relación de aspecto: "h" 1920x1080, "v" 1080x1920.
ASPECT = {"h": (1920, 1080), "v": (1080, 1920)}


def setup_render(aspect="h", samples=64, scale=100, fps=60, frames=1272, engine="CYCLES"):
    sc = bpy.context.scene
    sc.render.engine = engine
    w, h = ASPECT[aspect]
    sc.render.resolution_x, sc.render.resolution_y = w, h
    sc.render.resolution_percentage = scale
    sc.render.fps = fps
    sc.frame_start, sc.frame_end = 0, frames - 1
    sc.render.use_motion_blur = True
    sc.render.motion_blur_shutter = 0.5
    sc.render.film_transparent = False
    sc.render.filter_size = 1.5
    if engine == "CYCLES":
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = "OPTIX"
        prefs.refresh_devices()
        for d in prefs.devices:
            d.use = d.type == "OPTIX"
        c = sc.cycles
        c.device = "GPU"
        c.samples = samples
        c.use_adaptive_sampling = True
        c.adaptive_threshold = 0.015
        c.use_denoising = True
        c.denoiser = "OPENIMAGEDENOISE"
        c.denoising_use_gpu = True
        c.denoising_input_passes = "RGB_ALBEDO_NORMAL"
        c.max_bounces = 8
        c.diffuse_bounces = 3
        c.glossy_bounces = 4
        c.transmission_bounces = 4
        c.transparent_max_bounces = 8
        c.caustics_reflective = False
        c.caustics_refractive = False
        c.sample_clamp_indirect = 4.0
        c.blur_glossy = 1.0
        c.use_auto_tile = False
    vs = sc.view_settings
    vs.view_transform = "AgX"
    for look in ("AgX - Medium High Contrast", "Medium High Contrast", "AgX - Base Contrast"):
        try:
            vs.look = look
            break
        except TypeError:
            pass
    vs.exposure = 0.3
    sc.display_settings.display_device = "sRGB"
    sc.render.image_settings.file_format = "PNG"
    sc.render.image_settings.color_depth = "16"
    sc.render.image_settings.color_mode = "RGB"
    vl = sc.view_layers[0]
    vl.use_pass_mist = True


def setup_world():
    w = bpy.data.worlds.new("Mundo")
    bpy.context.scene.world = w
    nt = w.node_tree
    bg = next(n for n in nt.nodes if n.type == "BACKGROUND")
    # Degradado vertical muy suave: casi negro abajo, grafito arriba (luz que rebota del ambiente).
    tc = nt.nodes.new("ShaderNodeTexCoord")
    sep = nt.nodes.new("ShaderNodeSeparateXYZ")
    mr = nt.nodes.new("ShaderNodeMapRange")
    mix = nt.nodes.new("ShaderNodeMix")
    mix.data_type = "RGBA"
    nt.links.new(tc.outputs["Generated"], sep.inputs[0])
    nt.links.new(sep.outputs["Z"], mr.inputs["Value"])
    mr.inputs["From Min"].default_value = -0.2
    mr.inputs["From Max"].default_value = 0.8
    nt.links.new(mr.outputs["Result"], mix.inputs["Factor"])
    mix.inputs["A"].default_value = srgb("#060607")
    mix.inputs["B"].default_value = srgb("#121216")
    nt.links.new(mix.outputs["Result"], bg.inputs["Color"])
    bg.inputs["Strength"].default_value = 1.0
    w.mist_settings.start = 3.0
    w.mist_settings.depth = 38.0
    w.mist_settings.falloff = "QUADRATIC"
    return w


def setup_compositor(bloom=0.12, fog="#101014", fog_amount=0.55):
    sc = bpy.context.scene
    ng = bpy.data.node_groups.new("Composicion", "CompositorNodeTree")
    ng.interface.new_socket("Image", in_out="OUTPUT", socket_type="NodeSocketColor")
    sc.compositing_node_group = ng
    rl = ng.nodes.new("CompositorNodeRLayers")
    out = ng.nodes.new("NodeGroupOutput")
    # Bruma por profundidad (pase de niebla).
    fogmix = ng.nodes.new("ShaderNodeMix")
    fogmix.data_type = "RGBA"
    fogmix.blend_type = "MIX"
    mul = ng.nodes.new("ShaderNodeMath")
    mul.operation = "MULTIPLY"
    mul.inputs[1].default_value = fog_amount
    ng.links.new(rl.outputs["Mist"], mul.inputs[0])
    ng.links.new(mul.outputs[0], fogmix.inputs["Factor"])
    ng.links.new(rl.outputs["Image"], fogmix.inputs["A"])
    fogmix.inputs["B"].default_value = srgb(fog)
    # Bloom óptico contenido sobre lo que emite.
    glare = ng.nodes.new("CompositorNodeGlare")
    for v in ("Bloom", "BLOOM"):
        try:
            glare.inputs["Type"].default_value = v
            break
        except Exception:
            pass
    glare.inputs["Threshold"].default_value = 0.9
    glare.inputs["Strength"].default_value = bloom
    glare.inputs["Size"].default_value = 0.6 if glare.inputs["Size"].type == "VALUE" else glare.inputs["Size"].default_value
    ng.links.new(fogmix.outputs["Result"], glare.inputs["Image"])
    ng.links.new(glare.outputs["Image"], out.inputs[0])
    return ng


def floor():
    o = kit.plane("Suelo", 400, 400)
    o.rotation_euler = (math.radians(-90), 0, 0)   # horizontal, mirando arriba
    kit.assign(o, _floor_mat())
    return o


def _floor_mat():
    from .assets import M
    return M("floor")


def camera(name="Camara"):
    cd = bpy.data.cameras.new(name)
    cd.sensor_fit = "HORIZONTAL"
    cd.sensor_width = 36
    cd.clip_start = 0.02
    cd.clip_end = 400
    cd.dof.use_dof = True
    cd.dof.aperture_blades = 9
    cd.dof.aperture_rotation = math.radians(10)
    o = kit.link(bpy.data.objects.new(name, cd))
    bpy.context.scene.camera = o
    return o


class CameraRig:
    """La cámara única. Claves de posición, punto de interés, focal, diafragma y foco, interpoladas con velocidad
    continua; encima, el movimiento secundario (respiración) y las sacudidas de los impactos."""

    # Versión vertical: la misma trayectoria recompuesta. (t, multiplicador de la focal, desplazamiento lateral y vertical
    # de la cámara y su punto de interés). Con el sensor por ancho, el encuadre vertical abarca lo mismo de lado a lado:
    # donde la composición es estrecha se cierra la focal y donde es ancha se desplaza hacia lo importante.
    VKEYS = [(0.0, 1.75, 0, 0), (0.95, 1.70, 0, 0), (1.45, 1.30, 0, 0), (1.70, 1.0, 0, 0), (2.60, 1.0, 0, 0),
             (2.90, 1.05, 0, 0.30), (4.50, 1.05, 0, 0.30), (4.90, 1.0, 0, 0.05), (5.20, 1.0, 0.08, 0.28), (6.90, 1.0, 0.05, 0.25),
             (7.30, 1.0, 0, 0), (7.70, 1.15, 0, 0), (9.40, 1.15, 0, 0), (9.60, 1.15, -0.10, 0.08), (10.30, 1.30, -0.18, 0.12),
             (11.00, 1.35, 0.32, 0.14), (11.30, 1.05, 0, 0), (14.30, 1.05, 0, 0), (14.90, 1.35, 0, 0), (16.30, 1.35, 0, 0),
             (16.95, 1.15, 0, 0), (18.50, 1.25, 0, 0.06), (21.20, 1.25, 0, 0.06)]

    # Confort: la trayectoria se suaviza en el tiempo con una gaussiana de este ancho (s) para que ningún barrido maree.
    # El filtro es simétrico: las claves y su sincronía con la voz y la música no se mueven, sólo se reparten los giros.
    COMFORT = 0.16

    def __init__(self, cam, aspect="h"):
        self.cam = cam
        self.aspect = aspect
        self.pos = []
        self.target = []
        self.focal = []
        self.fstop = []
        self.focus = []
        self.roll = []
        self.shakes = []      # (t, dirección, amplitud, frecuencia, amortiguación)
        self.breath = 0.0025  # metros

    def key(self, t, pos=None, target=None, focal=None, fstop=None, focus=None, roll=None):
        for lst, v in ((self.pos, pos), (self.target, target), (self.focal, focal), (self.fstop, fstop), (self.focus, focus),
                       (self.roll, roll)):
            if v is not None:
                lst.append((t, v))

    def shake(self, t, direction, amp=0.02, freq=7.0, damp=10.0):
        self.shakes.append((t, Vector(direction).normalized(), amp, freq, damp))

    def _smooth(self, baker, pos, tgt, foc, roll):
        """Suaviza la posición, la dirección de la mirada (y su distancia), la focal y el alabeo con una gaussiana de
        COMFORT s. Las sacudidas y la respiración se suman después, sin filtrar."""
        import numpy as np
        fps, n = baker.fps, baker.frames
        sig = self.COMFORT * fps
        r = int(4 * sig)
        ts = [(i - r) / fps for i in range(n + 2 * r)]      # con margen: fuera del anuncio las pistas se quedan quietas
        P = np.array([pos(t) for t in ts], float)
        D = np.array([tgt(t) for t in ts], float) - P
        dist = np.linalg.norm(D, axis=1)
        D /= dist[:, None]
        F = np.array([foc(t) for t in ts], float)
        R = np.array([roll(t) for t in ts], float)
        k = np.exp(-0.5 * (np.arange(-r, r + 1) / sig) ** 2)
        k /= k.sum()

        def conv(x):
            return np.convolve(x, k, mode="valid")

        Ps = np.stack([conv(P[:, j]) for j in range(3)], 1)
        Ds = np.stack([conv(D[:, j]) for j in range(3)], 1)
        Ds /= np.linalg.norm(Ds, axis=1)[:, None]
        Gs = Ps + Ds * conv(dist)[:, None]
        fr = np.arange(n)

        def track(a):
            if a.ndim == 1:
                return lambda t: float(np.interp(t * fps, fr, a))
            return lambda t: tuple(float(np.interp(t * fps, fr, a[:, j])) for j in range(a.shape[1]))

        return track(Ps), track(Gs), track(conv(F)), track(conv(R))

    def bake(self, baker):
        pos = kit.hermite_track(sorted(self.pos))
        tgt = kit.hermite_track(sorted(self.target))
        foc = kit.hermite_track(sorted(self.focal))
        fst = kit.hermite_track(sorted(self.fstop))
        roll = kit.hermite_track(sorted(self.roll)) if self.roll else (lambda t: 0.0)
        focus = kit.hermite_track(sorted(self.focus)) if self.focus else None
        cam = self.cam
        if self.aspect == "v":
            vk = kit.hermite_track([(k[0], (k[1], k[2], k[3])) for k in self.VKEYS])
            base_pos, base_tgt, base_foc = pos, tgt, foc

            def tgt(t):
                z, dx, dz = vk(t)
                return tuple(Vector(base_tgt(t)) + Vector((dx, 0, dz)))

            def pos(t):
                z, dx, dz = vk(t)
                return tuple(Vector(base_pos(t)) + Vector((dx, 0, dz)))

            def foc(t):
                return base_foc(t) * vk(t)[0]

        pos, tgt, foc, roll = self._smooth(baker, pos, tgt, foc, roll)

        def eye(t):
            p = Vector(pos(t))
            b = self.breath
            p += Vector((kit.noise1(t * 0.9, 1.0) * b, kit.noise1(t * 0.8, 2.0) * b * 0.6, kit.noise1(t * 0.7, 3.0) * b))
            for (t0, d, amp, fr, dm) in self.shakes:
                p += d * amp * kit.impulse(t - t0, fr, dm)
            return p

        def rot(t):
            q = kit.look_quat(eye(t), Vector(tgt(t)), roll(t))
            return (q.w, q.x, q.y, q.z)

        cam.rotation_mode = "QUATERNION"
        baker.add(cam, "location", lambda t: tuple(eye(t)))
        baker.add(cam, "rotation_quaternion", rot)
        baker.add(cam.data, "lens", foc)
        baker.add(cam.data.dof, "aperture_fstop", fst)
        # Foco: las claves de foco mandan mientras duran; después, el punto de interés (con un relevo suave).
        t_focus_end = max(k[0] for k in self.focus) if self.focus else -1.0

        def focus_d(t):
            auto = (Vector(tgt(t)) - eye(t)).length
            if focus is None or t > t_focus_end + 0.3:
                return auto
            if t <= t_focus_end:
                return focus(t)
            return kit.lerp(focus(t), auto, kit.smooth((t - t_focus_end) / 0.3))

        baker.add(cam.data.dof, "focus_distance", focus_d)
