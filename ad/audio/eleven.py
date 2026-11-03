"""Locución con ElevenLabs. La clave se lee de la variable de entorno ELEVENLABS_API_KEY (también la del usuario de
Windows, guardada con setx) y nunca se imprime.

  python ad/audio/eleven.py voices [texto de búsqueda]     voces disponibles (propias y de la biblioteca en español)
  python ad/audio/eleven.py say <voice_id> <salida.mp3> [semilla] [estabilidad] [modelo]   genera la locución del guion
                                                                (modelo por defecto: eleven_v4)
"""
import json, os, sys, urllib.parse, urllib.request

API = "https://api.elevenlabs.io"

# Una sola interpretación continua; las pausas (…, punto y aparte) marcan la intención.
SCRIPT = ("Tres monitores. Dos equipos. "
          "Un brillo que por fin coincide. Una disposición para cada momento. "
          "Y, de repente, tu otro equipo… está en tu monitor. "
          "Windows, Mac, iPhone… hasta Android. "
          "Todas tus pantallas. Un solo lugar. "
          "Kast.")


def key():
    k = os.environ.get("ELEVENLABS_API_KEY")
    if not k and sys.platform == "win32":
        import winreg
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment") as h:
                k = winreg.QueryValueEx(h, "ELEVENLABS_API_KEY")[0]
        except OSError:
            k = None
    if not k:
        sys.exit("falta ELEVENLABS_API_KEY")
    return k.strip()


def call(path, body=None, accept="application/json"):
    req = urllib.request.Request(API + path, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"xi-api-key": key(), "Content-Type": "application/json", "Accept": accept})
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.read()
    except urllib.error.HTTPError as e:
        sys.exit(f"HTTP {e.code}: {e.read()[:400].decode(errors='replace')}")


def voices(q=""):
    mine = json.loads(call("/v2/voices?page_size=100" + (f"&search={urllib.parse.quote(q)}" if q else "")))
    for v in mine.get("voices", []):
        l = v.get("labels") or {}
        print("propia ", v["voice_id"], v["name"], l.get("gender", ""), l.get("age", ""), l.get("accent", ""), l.get("language", ""))
    qs = urllib.parse.urlencode({"page_size": 30, "language": "es", "gender": "female", "age": "young", **({"search": q} if q else {})})
    lib = json.loads(call("/v1/shared-voices?" + qs))
    for v in lib.get("voices", []):
        print("bibl.  ", v["voice_id"], v.get("name"), v.get("accent"), v.get("age"), v.get("use_case"), v.get("descriptive"),
              "|", (v.get("description") or "")[:90].replace("\n", " "))


def say(voice, out, seed=None, stability=0.5, model="eleven_v4"):
    body = {"text": SCRIPT, "model_id": model, "language_code": "es",
            "voice_settings": {"stability": float(stability), "similarity_boost": 0.8, "style": 0.35, "use_speaker_boost": True}}
    if seed is not None:
        body["seed"] = int(seed)
    # Con marcas de tiempo por carácter: para sincronizar la imagen con la voz.
    r = json.loads(call(f"/v1/text-to-speech/{voice}/with-timestamps?output_format=mp3_44100_192", body))
    import base64
    with open(out, "wb") as f:
        f.write(base64.b64decode(r["audio_base64"]))
    with open(os.path.splitext(out)[0] + ".json", "w", encoding="utf-8") as f:
        json.dump(r.get("alignment"), f, ensure_ascii=False)
    print("ok", out)


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "voices"
    if cmd == "voices":
        voices(" ".join(sys.argv[2:]))
    elif cmd == "say":
        say(sys.argv[2], sys.argv[3], *(sys.argv[4:7]))
