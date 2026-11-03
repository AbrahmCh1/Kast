"""Construye el anuncio en Blender y, si se pide, renderiza fotogramas.

  blender -b --factory-startup -P ad/render/build.py -- --aspect h [--samples 64] [--scale 50] [--frames 24,40] \
      [--range 0-1271] [--out ad/out/h/f_] [--save ad/out/kast_h.blend] [--engine CYCLES|BLENDER_EEVEE] [--upto 9]
"""
import argparse
import os
import sys
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, ROOT)
os.chdir(ROOT)

import bpy  # noqa: E402
import importlib  # noqa: E402
import ad.timeline as T  # noqa: E402
from ad.render import kit, assets, world, stations, stations2  # noqa: E402

for m in (kit, assets, world, stations, stations2):
    importlib.reload(m)


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser()
    p.add_argument("--aspect", default="h")
    p.add_argument("--samples", type=int, default=64)
    p.add_argument("--scale", type=int, default=100)
    p.add_argument("--frames", default="")
    p.add_argument("--range", default="")
    p.add_argument("--step", type=int, default=1)
    p.add_argument("--out", default="")
    p.add_argument("--save", default="")
    p.add_argument("--engine", default="CYCLES")
    p.add_argument("--upto", type=int, default=99)
    return p.parse_args(argv)


def build(a):
    kit.reset()
    world.setup_render(a.aspect, a.samples, a.scale, T.FPS, T.FRAMES, a.engine)
    world.setup_world()
    world.setup_compositor()
    kit.into(kit.collection("Mundo"))
    world.floor()
    cam = world.camera()
    rig = world.CameraRig(cam, a.aspect)
    baker = kit.Baker(T.FRAMES, T.FPS)
    ctx = {"aspect": a.aspect}
    scenes = [getattr(stations, f"scene{i}", None) or getattr(stations2, f"scene{i}") for i in range(1, 11)]
    for i, fn in enumerate(scenes, 1):
        if i > a.upto:
            break
        ctx[f"s{i}"] = fn(baker, rig, a.aspect) if fn.__code__.co_argcount == 3 else fn(baker, rig, a.aspect, ctx)
    rig.bake(baker)
    t0 = time.time()
    baker.bake()
    hide_dark_rings()
    print(f"horneado: {len(baker.tracks)} pistas en {time.time() - t0:.1f} s")


def hide_dark_rings():
    """Ondas y anillos apagados: se ocultan (si no, se verían como aros negros en el suelo)."""
    for o in bpy.data.objects:
        if o.type != "CURVE" or not any(k in o.name for k in ("Onda", "Aterriza", "Toque")) or not o.data.materials:
            continue
        nt = o.data.materials[0].node_tree
        ad = nt.animation_data
        fc = next((f for f in (ad.action.fcurves if ad and ad.action and hasattr(ad.action, "fcurves") else []) if "Strength" in f.data_path or "inputs[1]" in f.data_path), None) if ad else None
        if fc is None and ad and ad.action:
            for layer in getattr(ad.action, "layers", []):
                for strip in layer.strips:
                    for bag in strip.channelbags:
                        fc = next((f for f in bag.fcurves if "inputs" in f.data_path), fc)
        if fc is None:
            continue
        vals = [fc.evaluate(f) for f in range(T.FRAMES)]
        act = o.animation_data.action if o.animation_data and o.animation_data.action else None
        if o.animation_data is None:
            o.animation_data_create()
        if act is None:
            act = bpy.data.actions.new(o.name + "Accion")
            o.animation_data.action = act
        hf = act.fcurve_ensure_for_datablock(o, "hide_render")
        keys = [0]
        for i in range(1, T.FRAMES):
            if (vals[i] < 0.01) != (vals[i - 1] < 0.01):
                keys += [i - 1, i]
        keys.append(T.FRAMES - 1)
        keys = sorted(set(keys))
        hf.keyframe_points.add(len(keys))
        co = []
        for i in keys:
            co += [float(i), 1.0 if vals[i] < 0.01 else 0.0]
        hf.keyframe_points.foreach_set("co", co)
        hf.keyframe_points.foreach_set("interpolation", [0] * len(keys))
        hf.update()


def render(a):
    sc = bpy.context.scene
    frames = []
    if a.frames:
        for x in a.frames.split(","):
            x = x.strip()
            frames.append(int(round(float(x[:-1]) * T.FPS)) if x.endswith("s") else int(x))
    if a.range:
        f0, f1 = (int(v) for v in a.range.split("-"))
        frames += list(range(f0, f1 + 1, a.step))
    for f in frames:
        sc.frame_set(f)
        sc.render.filepath = os.path.abspath(f"{a.out}{f:04d}.png")
        if os.path.exists(sc.render.filepath) and os.environ.get("KAST_SKIP_EXISTING"):
            continue
        t0 = time.time()
        bpy.ops.render.render(write_still=True)
        print(f"fotograma {f} ({f / T.FPS:.2f} s): {time.time() - t0:.1f} s", flush=True)


if __name__ == "__main__":
    a = args()
    build(a)
    if a.save:
        os.makedirs(os.path.dirname(os.path.abspath(a.save)), exist_ok=True)
        bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(a.save))
    if a.out:
        os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
        render(a)
