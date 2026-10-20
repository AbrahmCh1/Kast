#!/usr/bin/env python3
"""Prueba de punta a punta de Kast Link con dos agentes en el mismo equipo.

  python3 link/tests/e2e.py <ruta a kast-link> [captura.png]

A (puerto 47800) y B (47810, otra identidad) se descubren; A pide ver la pantalla 1 de B, B comprueba el
código y acepta con control, A recibe vídeo en vivo; después se desconecta y una segunda petición se rechaza.
"""
import json, os, queue, subprocess, sys, tempfile, threading, time

EXE = sys.argv[1]
SHOT = sys.argv[2] if len(sys.argv) > 2 else None


class Agent:
    def __init__(self, name, port, home):
        env = dict(os.environ, KAST_LINK_HOME=home)
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
    A.wait(state(lambda m: not m["sessions"]))
    B.wait(state(lambda m: not m["sessions"]))
    print("4) desconectada en ambos lados")

    A.send(cmd="connect", id=me_b["id"], monitor=1, show_on=0)
    req2 = B.wait(lambda m: m.get("ev") == "request")
    B.send(cmd="answer", req=req2["req"], accept=False, remember=False)
    toast = A.wait(lambda m: m.get("ev") == "toast" and m.get("err"))
    print(f"5) rechazada: A recibe «{toast['msg']}»")
    print("OK")
finally:
    for a in (A, B):
        try:
            a.p.stdin.close()
            a.p.wait(timeout=5)
        except Exception:
            a.p.kill()
    for h in (ha, hb):
        err = open(os.path.join(h, "agent.err")).read().strip()
        if err:
            print(f"--- stderr ({h}):\n{err}")
