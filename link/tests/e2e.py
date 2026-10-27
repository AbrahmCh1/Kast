#!/usr/bin/env python3
"""Prueba de punta a punta de Kast Link con dos agentes en el mismo equipo.

  python3 link/tests/e2e.py <ruta a kast-link> [captura.png]

A (puerto 47800) y B (47810, otra identidad) se descubren; A pide ver la pantalla 1 de B, B comprueba el
código y acepta con control, A recibe vídeo en vivo; después se desconecta y una segunda petición se rechaza.
Siguen el acceso (recordar, contraseña, emparejamiento), varias pantallas, archivos y pantallas virtuales.
"""
import json, os, queue, subprocess, sys, tempfile, threading, time

EXE = sys.argv[1]
SHOT = sys.argv[2] if len(sys.argv) > 2 else None
# Sacar archivos arrastrando (paso 15): el anfitrión dice que en su equipo se arrastra esta carpeta.
DRAG_SRC = os.path.join(tempfile.mkdtemp(prefix="kast-arrastre-"), "Arrastre")
os.environ["KAST_LINK_TEST_DRAG"] = DRAG_SRC


class Agent:
    def __init__(self, name, port, home):
        # Descargas en la carpeta de la prueba (y sin abrir el Explorador / Finder al recibir archivos).
        env = dict(os.environ, KAST_LINK_HOME=home, KAST_LINK_DOWNLOADS=os.path.join(home, "descargas"))
        self.name = name
        self.p = subprocess.Popen([EXE, "agent", "--port", str(port)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=open(os.path.join(home, "agent.err"), "w"), env=env, text=True, bufsize=1,
                                  encoding="utf-8")
        self.q = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            if line.strip():
                self.q.put(json.loads(line))

    def settled(self, cond, timeout=20):
        """Espera a que el estado *actual* (no uno antiguo de la cola) cumpla la condición."""
        end = time.time() + timeout
        while time.time() < end:
            while not self.q.empty():
                self.q.get_nowait()
            self.send(cmd="state")
            m = self.wait(lambda m: m.get("ev") == "state")
            if cond(m):
                return m
            time.sleep(0.2)
        raise SystemExit(f"tiempo agotado esperando el estado en {self.name}")

    def send(self, **cmd):
        self.p.stdin.write(json.dumps(cmd) + "\n")
        self.p.stdin.flush()

    def wait(self, cond, timeout=20):
        end = time.time() + timeout
        while time.time() < end:
            try:
                m = self.q.get(timeout=0.2)
            except queue.Empty:
                continue
            if cond(m):
                return m
        raise SystemExit(f"tiempo agotado esperando en {self.name}")


def state(cond):
    return lambda m: m.get("ev") == "state" and cond(m)


ha, hb = tempfile.mkdtemp(), tempfile.mkdtemp()
base = int(os.environ.get("E2E_PORT", "47800"))
A, B = Agent("A", base, ha), Agent("B", base + 10, hb)
try:
    me_b = A and B.wait(state(lambda m: True))["me"]
    print(f"B: {me_b['name']} ({me_b['platform']}), {len(me_b['displays'])} pantalla(s)")
    st = A.wait(state(lambda m: any(d["id"] == me_b["id"] and d["online"] for d in m["devices"])), 30)
    d = next(d for d in st["devices"] if d["id"] == me_b["id"])
    print(f"1) A descubre a B en {d['addr']}")

    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=True)
    req = B.wait(lambda m: m.get("ev") == "request")
    assert req.get("control") is True, "la petición no pide control"
    wait = A.wait(state(lambda m: any(s["state"] == "waiting" for s in m["sessions"])))
    code_a = next(s["code"] for s in wait["sessions"] if s["state"] == "waiting")
    assert req["code"] == code_a, f"los códigos no coinciden: {req['code']} / {code_a}"
    print(f"2) B recibe la petición; código {req['code']} igual en ambos lados")

    B.send(cmd="answer", req=req["req"], accept=True, remember=False, control=True)
    live = A.wait(state(lambda m: any(s["dir"] == "in" and s["state"] == "live" for s in m["sessions"])), 30)
    s_a = next(s for s in live["sessions"] if s["dir"] == "in")
    sid = s_a["session"]
    # Sin permiso de Accesibilidad (Mac de CI) el anfitrión pasa la sesión a solo vista.
    want = me_b.get("input") is not False
    assert s_a.get("control") == want, f"control negociado {s_a.get('control')}, esperado {want}"
    print(f"3) aceptada: A ve la pantalla de B en vivo ({'con control' if want else 'solo vista: B sin permiso de control'})")
    if SHOT:
        time.sleep(5)
        subprocess.run(["screencapture", "-x", SHOT])
        print(f"   captura en {SHOT}")

    A.send(cmd="disconnect", session=sid)
    A.settled(lambda m: not m["sessions"])
    B.settled(lambda m: not m["sessions"])
    print("4) desconectada en ambos lados")

    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0)
    req2 = B.wait(lambda m: m.get("ev") == "request")
    B.send(cmd="answer", req=req2["req"], accept=False, remember=False)
    toast = A.wait(lambda m: m.get("ev") == "toast" and m.get("err"))
    print(f"5) rechazada: A recibe «{toast['msg']}»")

    def live_session():
        st = A.wait(state(lambda m: any(s["dir"] == "in" and s["state"] == "live" for s in m["sessions"])), 30)
        return next(s for s in st["sessions"] if s["dir"] == "in")

    def hang_up(s):
        A.send(cmd="disconnect", session=s["session"])
        A.settled(lambda m: not m["sessions"])
        B.settled(lambda m: not m["sessions"])

    # B recuerda a A sólo para ver: después ve sin preguntar, pero si pide control vuelve a preguntar.
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False)
    req3 = B.wait(lambda m: m.get("ev") == "request")
    B.send(cmd="answer", req=req3["req"], accept=True, remember=True, control=False)
    hang_up(live_session())
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False)
    s = live_session()
    assert not s["control"]
    hang_up(s)
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=True)
    req4 = B.wait(lambda m: m.get("ev") == "request")
    assert req4["control"] is True
    B.send(cmd="answer", req=req4["req"], accept=True, remember=True, control=True)
    s = live_session()
    assert s["control"] == want
    hang_up(s)
    print("6) recordado sólo para ver: entra sin preguntar; al pedir control, vuelve a preguntar")

    # Disposiciones: guardar la sesión (monitor 9 no existe: se abre en ventana, sin tapar pantallas reales),
    # cortarla, restaurarla y después cargar una disposición sin sesiones.
    lay = os.path.join(ha, "prueba.link")
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=9, control=False)
    s = live_session()
    A.send(cmd="layout_save", path=lay)
    A.send(cmd="state")
    A.wait(state(lambda m: True))
    saved = open(lay, encoding="utf-8").read().split("	")
    assert saved[0] == me_b["id"] and saved[2:4] == ["1", "9"], saved
    hang_up(s)
    A.send(cmd="layout_load", path=lay)
    s = live_session()
    assert s["show_on"] == 9
    A.send(cmd="layout_load", path=lay)  # ya coincide: se mantiene
    time.sleep(2)
    A.send(cmd="state")
    st = A.wait(state(lambda m: True))
    assert [x["session"] for x in st["sessions"] if x["dir"] == "in"] == [s["session"]], st["sessions"]
    empty = os.path.join(ha, "vacia.link")
    open(empty, "w").close()
    A.send(cmd="layout_load", path=empty)
    A.wait(state(lambda m: not m["sessions"]))
    # Una disposición sin archivo de sesiones (guardada sin otros equipos o antes de esta función) también la cierra.
    A.send(cmd="layout_load", path=lay)
    live_session()
    A.send(cmd="layout_load", path=os.path.join(ha, "no-existe.link"))
    A.wait(state(lambda m: not m["sessions"]))
    print("7) disposiciones: guarda la sesión, la restaura, la mantiene si ya está y la cierra si la disposición no la tiene")

    # Archivos: una carpeta con subcarpeta, de A (visor con control) a las Descargas de B.
    src = os.path.join(ha, "para-enviar", "Fotos")
    os.makedirs(os.path.join(src, "sub"))
    with open(os.path.join(src, "nota.txt"), "w", encoding="utf-8") as f:
        f.write("hola desde A")
    big = os.urandom(300_000)
    with open(os.path.join(src, "sub", "datos.bin"), "wb") as f:
        f.write(big)
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=9, control=True)
    s = live_session()
    if s["control"]:
        A.send(cmd="send_files", session=s["session"], paths=[src])
        got = B.wait(lambda m: m.get("ev") == "toast" and "te envió" in m.get("msg", ""), 30)
        sent = A.wait(lambda m: m.get("ev") == "toast" and "Enviados" in m.get("msg", ""), 30)
        dl = os.path.join(hb, "descargas", "Fotos")
        assert open(os.path.join(dl, "nota.txt"), encoding="utf-8").read() == "hola desde A"
        assert open(os.path.join(dl, "sub", "datos.bin"), "rb").read() == big
        print(f"8) archivos: «{sent['msg']}» / «{got['msg']}»")
    else:
        print("8) archivos: omitido (B no permite el control en este equipo)")
    hang_up(s)

    # Cliente de iOS (mobile.rs, por la terminal): pide ver y controlar la pantalla de B como un iPhone.
    probe_home = tempfile.mkdtemp()
    probe = subprocess.Popen([EXE, "probe", f"127.0.0.1:{base + 10}", "--home", probe_home, "--secs", "15"],
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8")
    req5 = B.wait(lambda m: m.get("ev") == "request", 20)
    assert req5["platform"] == "ios" and req5["control"] is True, req5
    B.send(cmd="answer", req=req5["req"], accept=True, remember=False, control=True)
    out, _ = probe.communicate(timeout=40)
    assert probe.returncode == 0, out
    assert '"state":"live"' in out, out
    print(f"9) iOS: {[l for l in out.splitlines() if l.startswith('probe:')][0]} (petición «{req5['name']}», plataforma {req5['platform']})")
    B.wait(state(lambda m: not m["sessions"]), 20)

    A.send(cmd="state")
    me_a = A.wait(state(lambda m: True))["me"]
    is_a = lambda t: t["key"].startswith(me_a["id"])

    # Acceso desatendido: con la contraseña entra sin que nadie acepte; con otra, rechazo y aviso en B.
    B.send(cmd="access_set", password="clave de prueba 123", unattended_control=False)
    st = B.wait(state(lambda m: m["access"]["unattended"]))
    assert st["access"]["unattended_control"] is False
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False, auth="password", secret="clave de prueba 123")
    s = live_session()
    B.wait(state(lambda m: any(x["dir"] == "out" and x["via"] == "password" for x in m["sessions"])))
    hang_up(s)
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False, auth="password", secret="otra clave")
    A.wait(lambda m: m.get("ev") == "need_auth" and m.get("failed"))
    bad = A.wait(lambda m: m.get("ev") == "toast" and m.get("err") and "incorrecta" in m.get("msg", ""))
    B.wait(lambda m: m.get("ev") == "toast" and "Intento fallido" in m.get("msg", ""))
    print(f"10) desatendido: entra con la contraseña sin aprobación; con otra: «{bad['msg']}»")

    # Quitar el acceso y rechazar a los desconocidos: A ya no entra sin contraseña o código.
    B.send(cmd="state")
    st = B.wait(state(lambda m: True))
    ka = next(t["key"] for t in st["trusted"] if is_a(t))
    B.send(cmd="revoke", key=ka)
    B.wait(state(lambda m: not any(is_a(t) for t in m["trusted"])))
    B.send(cmd="access_set", unknown="reject")
    B.wait(state(lambda m: m["access"]["unknown"] == "reject"))
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False)
    A.wait(lambda m: m.get("ev") == "need_auth" and not m.get("failed"))
    why = A.wait(lambda m: m.get("ev") == "toast" and m.get("err"))
    print(f"11) acceso quitado y desconocidos rechazados: «{why['msg']}»")

    # Emparejamiento: código de un solo uso; después entra sin preguntar (también rechazando desconocidos).
    B.send(cmd="pair_start", control=True)
    code = B.wait(state(lambda m: m["pairing"]))["pairing"]["code"]
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False, auth="pair", secret=code.lower().replace("-", " "))
    s = live_session()
    B.wait(lambda m: m.get("ev") == "toast" and "emparejado" in m.get("msg", ""))
    st = B.wait(state(lambda m: not m["pairing"] and any(is_a(t) and t["control"] for t in m["trusted"])))
    hang_up(s)
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False)
    hang_up(live_session())
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0, control=False, auth="pair", secret=code)
    reused = A.wait(lambda m: m.get("ev") == "toast" and m.get("err"))
    print(f"12) emparejado con {code}: entra sin preguntar; el código ya usado: «{reused['msg']}»")

    # Límite de intentos (otra identidad: la del cliente de iOS): tras 5 fallos, bloqueo aunque acierte.
    lock_home = tempfile.mkdtemp()
    def try_password(pw):
        p = subprocess.run([EXE, "probe", f"127.0.0.1:{base + 10}", "--home", lock_home, "--secs", "20", "--password", pw],
                           capture_output=True, text=True, encoding="utf-8", timeout=60)
        return p.returncode, p.stdout + p.stderr
    for _ in range(5):
        rc, out = try_password("mala")
        assert rc != 0 and "incorrecta" in out, out
    rc, out = try_password("clave de prueba 123")
    assert rc != 0 and "Demasiados intentos" in out, out
    print("13) límite de intentos: tras 5 contraseñas incorrectas, bloqueado aunque acierte")
    B.send(cmd="access_set", unknown="ask", unattended=False)
    B.wait(state(lambda m: not m["access"]["unattended"] and m["access"]["unknown"] == "ask"))

    # Varias pantallas con una sola aprobación (si B tiene más de una) y, después, otra conexión del mismo grupo
    # entra sin volver a preguntar.
    B.send(cmd="revoke", key=ka)
    B.wait(state(lambda m: not any(is_a(t) for t in m["trusted"])))
    screens = [{"monitor": i + 1, "show_on": 9} for i in range(min(2, len(me_b["displays"])))]
    A.send(cmd="connect", id=me_b["id"], control=False, screens=screens)
    req6 = B.wait(lambda m: m.get("ev") == "request")
    assert req6["monitors"] == [s["monitor"] for s in screens], req6
    B.send(cmd="answer", req=req6["req"], accept=True, remember=False, control=False)
    st = A.wait(state(lambda m: sum(1 for x in m["sessions"] if x["dir"] == "in" and x["state"] == "live") == len(screens)), 30)
    assert not B.settled(lambda m: True)["requests"], "pidió permiso otra vez"
    for x in st["sessions"]:
        A.send(cmd="disconnect", session=x["session"])
    A.settled(lambda m: not m["sessions"])
    B.settled(lambda m: not m["sessions"])
    # Cerradas todas, el permiso del grupo se acaba: volver a entrar pide permiso otra vez.
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=9, control=False)
    req7 = B.wait(lambda m: m.get("ev") == "request")
    B.send(cmd="answer", req=req7["req"], accept=False)
    A.wait(lambda m: m.get("ev") == "toast" and m.get("err"))
    print(f"14) pantallas {[s['monitor'] for s in screens]} con una sola aprobación; al cerrarlas, volver a entrar pide permiso")

    # Sacar archivos arrastrando del visor: B dice que se arrastra una carpeta y A la recibe entera en su carpeta
    # temporal (la ventana del visor no arrastra nada porque nadie tiene el botón pulsado).
    os.makedirs(os.path.join(DRAG_SRC, "sub"))
    with open(os.path.join(DRAG_SRC, "a.txt"), "w", encoding="utf-8") as f:
        f.write("arrastrado desde B")
    blob = os.urandom(200_000)
    with open(os.path.join(DRAG_SRC, "sub", "b.bin"), "wb") as f:
        f.write(blob)
    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=9, control=True)
    req8 = B.wait(lambda m: m.get("ev") == "request")
    B.send(cmd="answer", req=req8["req"], accept=True, remember=False, control=True)
    s = live_session()
    if s["control"]:
        A.send(cmd="drag_query", session=s["session"])
        drag = A.wait(lambda m: m.get("ev") == "drag", 30)
        done = A.wait(lambda m: m.get("ev") == "drag_done", 30)
        assert done["ok"], done
        out = drag["paths"][0]
        assert os.path.basename(out) == "Arrastre", drag
        assert open(os.path.join(out, "a.txt"), encoding="utf-8").read() == "arrastrado desde B"
        assert open(os.path.join(out, "sub", "b.bin"), "rb").read() == blob
        print(f"15) sacar archivos arrastrando: la carpeta llega entera a {out}")
    else:
        print("15) sacar archivos arrastrando: omitido (B no permite el control en este equipo)")
    hang_up(s)

    # Pantallas virtuales: A pide la pantalla 1 de B en un monitor y una virtual nueva en el monitor de su derecha. Si B
    # puede (un Mac, o Windows con el controlador de Parsec), la crea con ese tamaño, A abre las dos con una sola
    # aprobación y al desconectar B la quita; si no, A recibe el motivo y ve sólo la 1.
    B_displays = B.settled(lambda m: True)["me"]["displays"]
    nb = len(B_displays)
    A.send(cmd="connect", id=me_b["id"], control=False, screens=[
        {"monitor": 1, "show_on": 9, "at": [0, 0, 1920, 1080]},
        {"virtual": True, "show_on": 9, "at": [1920, 0, 1280, 720], "px": [1280, 720], "scale": 1}])
    req9 = B.wait(lambda m: m.get("ev") == "request")
    assert req9.get("virt") == 1 and req9["monitors"] == [1], req9
    B.send(cmd="answer", req=req9["req"], accept=True, remember=False, control=False)
    lives = lambda m: [x for x in m["sessions"] if x["dir"] == "in" and x["state"] == "live"]
    got = A.wait(lambda m: (m.get("ev") == "toast" and m.get("err") and "virtual" in m["msg"]) or (m.get("ev") == "state" and len(lives(m)) == 2), 40)
    if got.get("ev") == "state":
        assert sum(1 for x in lives(got) if x["virt"]) == 1, got["sessions"]
        mb = B.settled(lambda m: len(m["me"]["displays"]) == nb + 1)["me"]
        assert [1280, 720] in mb["displays"], mb["displays"]
        assert any(x["virt"] for x in B.settled(lambda m: len(m["sessions"]) == 2)["sessions"])
        for x in lives(got):
            A.send(cmd="disconnect", session=x["session"])
        A.settled(lambda m: not m["sessions"])
        B.settled(lambda m: not m["sessions"])
        B.settled(lambda m: len(m["me"]["displays"]) == nb)
        print(f"16) pantalla virtual de 1280×720 creada en B, vista junto a su pantalla 1 y quitada al desconectar")

        # Con un solo monitor: la pantalla 1 de B y una virtual, las dos en ventanas. La virtual tiene el tamaño del
        # monitor principal de A y B la pone a la derecha de sus pantallas.
        A.send(cmd="connect", id=me_b["id"], control=False, screens=[{"monitor": 1, "show_on": 0}, {"virtual": True, "show_on": 0}])
        req10 = B.wait(lambda m: m.get("ev") == "request")
        assert req10.get("virt") == 1, req10
        B.send(cmd="answer", req=req10["req"], accept=True, remember=False, control=False)
        got = A.wait(lambda m: (m.get("ev") == "toast" and m.get("err")) or (m.get("ev") == "state" and len(lives(m)) == 2), 40)
        assert got.get("ev") == "state", got
        assert [x["show_on"] for x in lives(got)] == [0, 0] and sum(1 for x in lives(got) if x["virt"]) == 1, got["sessions"]
        mb = B.settled(lambda m: len(m["me"]["displays"]) == nb + 1)["me"]
        size = next(d for d in mb["displays"] if d not in B_displays or mb["displays"].count(d) > B_displays.count(d))
        for x in lives(got):
            A.send(cmd="disconnect", session=x["session"])
        A.settled(lambda m: not m["sessions"])
        B.settled(lambda m: len(m["me"]["displays"]) == nb)
        print(f"17) pantalla virtual en una ventana: {size[0]}×{size[1]} (el monitor principal de A), quitada al desconectar")
    else:
        if me_b.get("vd"):
            print(f"::warning::B no pudo crear la pantalla virtual en este equipo: {got['msg']}")
        hang_up(live_session())
        print(f"16) pantallas virtuales: A ve sólo la pantalla 1 y recibe «{got['msg']}»")
    print("OK")
finally:
    for a in (A, B):
        if a.p.poll() is not None:
            print(f"!!! el agente {a.name} terminó solo (código {a.p.returncode})")
        try:
            a.p.stdin.close()
            a.p.wait(timeout=5)
        except Exception:
            a.p.kill()
    for h in (ha, hb):
        err = open(os.path.join(h, "agent.err")).read().strip()
        if err:
            print(f"--- stderr ({h}):\n{err}")
