//! Agente de Kast Link: lo arranca la app de Kast (bandeja en Windows, barra de menús en macOS) y se
//! comunica con ella por stdin/stdout con una línea JSON por mensaje.
//!
//! App -> agente:
//!   {"cmd":"state"}                                   reenviar el estado completo
//!   {"cmd":"connect","id":"…","monitor":1,"show_on":0} ver el monitor 1 de un dispositivo (0 = ventana, n = monitor local n)
//!   {"cmd":"add","addr":"192.168.1.20"}               agregar un dispositivo por IP
//!   {"cmd":"forget","id":"…"}                          olvidar un dispositivo (y su confianza)
//!   {"cmd":"answer","req":3,"accept":true,"remember":true}
//!   {"cmd":"disconnect","session":5}
//!   {"cmd":"sharing","enabled":true}                   permitir que otros pidan ver mis pantallas
//! Agente -> app:
//!   {"t":"link","ev":"state",…}   estado completo (yo, dispositivos, peticiones, sesiones)
//!   {"t":"link","ev":"request",…} alguien pide ver mis pantallas (la app debe mostrarse)
//!   {"t":"link","ev":"toast","msg":"…","err":false}
use crate::discovery::{self, Beacon, Packet};
use crate::net::{self, Identity, Secure, id_of, msg};
use crate::store::{self, Saved};
use crate::{Res, platform, stream, viewer};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::event_loop::EventLoopProxy;

pub const DEFAULT_PORT: u16 = 47800;
const ONLINE: Duration = Duration::from_secs(7);
const FPS: u32 = 60;
const MBPS: u32 = 20;

struct Dev {
    name: String,
    platform: String,
    addr: Option<SocketAddr>,
    seen: Option<Instant>,
    displays: Vec<(u32, u32)>,
    sharing: bool,
}

struct Req {
    req: u64,
    name: String,
    platform: String,
    code: String,
    monitor: usize,
    answer: Sender<(bool, bool)>,
}

struct Sess {
    session: u64,
    /// "in": yo veo a otro; "out": otro me ve.
    dir: &'static str,
    peer: String,
    monitor: usize,
    state: &'static str,
    code: String,
    sock: Option<TcpStream>,
    /// Cortada a propósito (ventana cerrada o "Desconectar").
    stopped: bool,
}

#[derive(Default)]
struct State {
    devices: HashMap<String, Dev>,
    saved: Vec<Saved>,
    requests: Vec<Req>,
    sessions: Vec<Sess>,
}

pub struct Agent {
    me: Identity,
    id: String,
    name: String,
    port: u16,
    sharing: AtomicBool,
    next: AtomicU64,
    out: Mutex<std::io::Stdout>,
    st: Mutex<State>,
    displays: Mutex<Vec<(u32, u32)>>,
    proxy: EventLoopProxy<viewer::Event>,
    udp: UdpSocket,
}

fn settings_path() -> std::path::PathBuf { store::config_dir().join("link-settings.txt") }

impl Agent {
    fn emit(&self, v: Value) {
        let mut o = self.out.lock().unwrap();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    }

    fn toast(&self, msg: impl Into<String>, err: bool) {
        self.emit(json!({"t": "link", "ev": "toast", "msg": msg.into(), "err": err}));
    }

    fn id(&self) -> u64 { self.next.fetch_add(1, Ordering::Relaxed) + 1 }

    fn beacon(&self) -> Beacon {
        Beacon { id: self.id.clone(), name: self.name.clone(), platform: store::platform().into(), port: self.port,
                 displays: self.displays.lock().unwrap().clone(), sharing: self.sharing.load(Ordering::Relaxed) }
    }

    fn emit_state(&self) {
        let st = self.st.lock().unwrap();
        let now = Instant::now();
        let mut ids: Vec<&String> = st.devices.keys().collect();
        ids.sort_by_key(|k| st.devices[*k].name.to_lowercase());
        let devices: Vec<Value> = ids.iter().map(|id| {
            let d = &st.devices[*id];
            let saved = st.saved.iter().find(|s| (!s.key.is_empty() && id_of(&net::unhex(&s.key).unwrap_or_default()) == **id)
                || (s.key.is_empty() && d.addr.map(|a| s.addr.starts_with(&a.ip().to_string())).unwrap_or(false)));
            json!({
                "id": id, "name": d.name, "platform": d.platform,
                "addr": d.addr.map(|a| a.to_string()).unwrap_or_default(),
                "online": d.seen.map(|t| now - t < ONLINE).unwrap_or(false),
                "trusted": saved.map(|s| s.trusted).unwrap_or(false),
                "known": saved.map(|s| !s.key.is_empty()).unwrap_or(false),
                "manual": saved.map(|s| s.manual).unwrap_or(false),
                "sharing": d.sharing,
                "displays": d.displays.iter().map(|(w, h)| json!([w, h])).collect::<Vec<_>>(),
            })
        }).collect();
        // Agregados por IP que aún no respondieron.
        let pending: Vec<Value> = st.saved.iter().filter(|s| s.manual && s.key.is_empty()
            && !st.devices.values().any(|d| d.addr.map(|a| s.addr.starts_with(&a.ip().to_string())).unwrap_or(false)))
            .map(|s| json!({"id": format!("ip:{}", s.addr), "name": s.addr, "platform": "", "addr": s.addr, "online": false,
                            "trusted": false, "known": false, "manual": true, "sharing": true, "displays": []}))
            .collect();
        let requests: Vec<Value> = st.requests.iter().map(|r| json!({"req": r.req, "name": r.name, "platform": r.platform, "code": r.code, "monitor": r.monitor})).collect();
        let sessions: Vec<Value> = st.sessions.iter().map(|s| json!({"session": s.session, "dir": s.dir, "peer": s.peer, "monitor": s.monitor, "state": s.state, "code": s.code})).collect();
        let me = json!({"id": self.id, "name": self.name, "platform": store::platform(), "port": self.port,
                        "sharing": self.sharing.load(Ordering::Relaxed),
                        "displays": self.displays.lock().unwrap().iter().map(|(w, h)| json!([w, h])).collect::<Vec<_>>()});
        drop(st);
        let all: Vec<Value> = devices.into_iter().chain(pending).collect();
        self.emit(json!({"t": "link", "ev": "state", "me": me, "devices": all, "requests": requests, "sessions": sessions}));
    }

    fn save(&self, st: &State) { store::save_devices(&st.saved); }

    // ------------------------------------------------------------ comandos de la app

    fn command(self: &Arc<Self>, line: &str) {
        let line = line.trim().trim_start_matches('\u{feff}');
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        match s("cmd").as_str() {
            "state" => self.emit_state(),
            "connect" => {
                let (id, monitor, show_on) = (s("id"), n("monitor").max(1) as usize, n("show_on") as usize);
                let a = self.clone();
                std::thread::spawn(move || a.connect(&id, monitor, show_on));
            }
            "add" => self.add(&s("addr")),
            "forget" => {
                let id = s("id");
                let mut st = self.st.lock().unwrap();
                let ip = id.strip_prefix("ip:").map(str::to_string);
                st.saved.retain(|d| match &ip {
                    Some(a) => d.addr != *a,
                    None => d.key.is_empty() || id_of(&net::unhex(&d.key).unwrap_or_default()) != id,
                });
                st.devices.remove(&id);
                self.save(&st);
                drop(st);
                self.emit_state();
            }
            "answer" => {
                let req = n("req");
                let mut st = self.st.lock().unwrap();
                if let Some(i) = st.requests.iter().position(|r| r.req == req) {
                    let r = st.requests.remove(i);
                    let _ = r.answer.send((b("accept"), b("remember")));
                }
                drop(st);
                self.emit_state();
            }
            "disconnect" => self.stop_session(n("session")),
            "sharing" => {
                self.sharing.store(b("enabled"), Ordering::Relaxed);
                let _ = std::fs::write(settings_path(), format!("sharing={}\n", b("enabled") as u8));
                self.emit_state();
                self.announce();
            }
            _ => {}
        }
    }

    pub fn stop_session(&self, session: u64) {
        let mut st = self.st.lock().unwrap();
        if let Some(s) = st.sessions.iter_mut().find(|s| s.session == session) {
            s.stopped = true;
            if let Some(sock) = &s.sock { let _ = sock.shutdown(std::net::Shutdown::Both); }
        }
    }

    fn add(&self, raw: &str) {
        let raw = raw.trim();
        let with_port = if raw.contains(':') { raw.to_string() } else { format!("{raw}:{DEFAULT_PORT}") };
        let Some(addr) = with_port.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
            self.toast(format!("Dirección inválida: {raw}"), true);
            return;
        };
        let mut st = self.st.lock().unwrap();
        if !st.saved.iter().any(|d| d.addr == addr.to_string()) {
            st.saved.push(Saved { addr: addr.to_string(), manual: true, ..Default::default() });
            self.save(&st);
        }
        drop(st);
        let _ = self.udp.send_to(&discovery::query(), SocketAddr::new(addr.ip(), discovery::PORT));
        self.toast(format!("Buscando {}…", addr.ip()), false);
        self.emit_state();
    }

    fn set_session(&self, session: u64, f: impl FnOnce(&mut Sess)) {
        let mut st = self.st.lock().unwrap();
        if let Some(s) = st.sessions.iter_mut().find(|s| s.session == session) { f(s); }
        drop(st);
        self.emit_state();
    }

    fn end_session(&self, session: u64) {
        self.st.lock().unwrap().sessions.retain(|s| s.session != session);
        self.emit_state();
    }

    // ------------------------------------------------------------ yo veo a otro

    fn connect(self: &Arc<Self>, id: &str, monitor: usize, show_on: usize) {
        let (addr, name) = {
            let st = self.st.lock().unwrap();
            if let Some(a) = id.strip_prefix("ip:") {
                (a.parse::<SocketAddr>().ok(), a.to_string())
            } else {
                match st.devices.get(id) { Some(d) => (d.addr, d.name.clone()), None => (None, id.to_string()) }
            }
        };
        let Some(addr) = addr else { self.toast(format!("{name} no está disponible"), true); return };
        let session = self.id();
        self.st.lock().unwrap().sessions.push(Sess { session, dir: "in", peer: name.clone(), monitor, state: "connecting", code: String::new(), sock: None, stopped: false });
        self.emit_state();
        let r = self.view(session, id, addr, &name, monitor, show_on);
        let _ = self.proxy.send_event(viewer::Event::Close(session));
        self.end_session(session);
        match r {
            Ok(()) => {}
            Err(e) => self.toast(format!("{name}: {e}"), true),
        }
    }

    fn view(self: &Arc<Self>, session: u64, id: &str, addr: SocketAddr, name: &str, monitor: usize, show_on: usize) -> Res<()> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|_| "no responde")?;
        let mut ch = Secure::connect(stream, &self.me)?;
        let peer_key = net::hex(&ch.peer);
        let peer_id = id_of(&ch.peer);
        // El dispositivo debe ser quien dice ser (el anuncio y la identidad tienen que coincidir)…
        if !id.starts_with("ip:") && peer_id != id { return Err("la identidad del dispositivo no coincide".into()); }
        // …y no puede haber cambiado desde la última vez (TOFU).
        {
            let st = self.st.lock().unwrap();
            if let Some(s) = st.saved.iter().find(|s| !s.key.is_empty() && id_of(&net::unhex(&s.key).unwrap_or_default()) == peer_id) {
                if s.key != peer_key { return Err("la identidad del dispositivo cambió: olvídalo y vuelve a agregarlo".into()); }
            }
        }
        let sock = ch.shutdown_handle()?;
        let code = ch.code.clone();
        self.set_session(session, |s| { s.state = "waiting"; s.code = code; s.sock = Some(sock); });
        let req = json!({"name": self.name, "platform": store::platform(), "monitor": monitor}).to_string();
        ch.send_typed(msg::REQUEST, req.as_bytes())?;
        let mut m = Vec::new();
        ch.recv(&mut m).map_err(|_| "conexión cerrada")?;
        match m.first() {
            Some(&msg::ACCEPT) => {}
            Some(&msg::REJECT) => return Err(String::from_utf8_lossy(&m[1..]).to_string().into()),
            _ => return Err("respuesta inesperada".into()),
        }
        // Recordar el dispositivo (su identidad, para detectar cambios).
        {
            let mut st = self.st.lock().unwrap();
            let (plat, dname) = st.devices.get(&peer_id).map(|d| (d.platform.clone(), d.name.clone())).unwrap_or_default();
            let ip = addr.to_string();
            match st.saved.iter_mut().find(|s| s.key == peer_key || (s.key.is_empty() && s.addr == ip)) {
                Some(s) => { s.key = peer_key.clone(); s.addr = ip; if !dname.is_empty() { s.name = dname; s.platform = plat; } }
                None => st.saved.push(Saved { key: peer_key, name: dname, platform: plat, addr: ip, trusted: false, manual: false }),
            }
            self.save(&st);
        }
        ch.recv(&mut m)?;
        if m.first() != Some(&msg::HELLO) { return Err("respuesta inesperada".into()); }
        let (w, h, fps) = stream::parse_hello(&m[1..]).ok_or("respuesta inesperada")?;
        let shared = Arc::new(Mutex::new(viewer::Shared { frame: vec![0; w * h], w, h, cursor: (0, 0, false) }));
        let title = format!("Kast Link — {name} · pantalla {monitor}");
        let _ = self.proxy.send_event(viewer::Event::Open { session, shared: shared.clone(), monitor: (show_on > 0).then_some(show_on), title });
        self.set_session(session, |s| s.state = "live");
        let proxy = self.proxy.clone();
        let r = stream::receive(&mut ch, &shared, w, h, fps, &move || { let _ = proxy.send_event(viewer::Event::Redraw(session)); });
        // Cerrar la ventana o pulsar "Desconectar" corta la conexión a propósito: no es un error.
        let stopped = self.st.lock().unwrap().sessions.iter().any(|s| s.session == session && s.stopped);
        match r {
            Err(_) if stopped => Ok(()),
            // El otro equipo detuvo la transmisión (o se cerró Kast allí).
            Err(e) if e.downcast_ref::<std::io::Error>().is_some() => { self.toast(format!("{name} terminó la sesión"), false); Ok(()) }
            r => r,
        }
    }

    // ------------------------------------------------------------ otro me ve

    fn listen(self: &Arc<Self>, listener: TcpListener) {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let a = self.clone();
            std::thread::spawn(move || {
                let peer = stream.peer_addr().ok();
                if let Err(e) = a.host(stream, peer) { eprintln!("sesión entrante: {e}"); }
            });
        }
    }

    fn host(self: &Arc<Self>, stream: TcpStream, peer_addr: Option<SocketAddr>) -> Res<()> {
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut ch = Secure::accept(stream, &self.me)?;
        let mut m = Vec::new();
        ch.recv(&mut m)?;
        if m.first() != Some(&msg::REQUEST) { return Err("petición inválida".into()); }
        let v: Value = serde_json::from_slice(&m[1..]).unwrap_or(Value::Null);
        let name: String = v.get("name").and_then(Value::as_str).unwrap_or("Dispositivo").chars().take(64).collect();
        let plat: String = v.get("platform").and_then(Value::as_str).unwrap_or("").chars().take(16).collect();
        let monitor = v.get("monitor").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
        if !self.sharing.load(Ordering::Relaxed) {
            ch.send_typed(msg::REJECT, format!("{} no permite ver sus pantallas", self.name).as_bytes())?;
            return Ok(());
        }
        if monitor > self.displays.lock().unwrap().len().max(1) {
            ch.send_typed(msg::REJECT, format!("{} no tiene la pantalla {monitor}", self.name).as_bytes())?;
            return Ok(());
        }
        let key = net::hex(&ch.peer);
        let trusted = self.st.lock().unwrap().saved.iter().any(|s| s.key == key && s.trusted);
        if !trusted {
            // Hay que aceptar en este equipo: la app se muestra con la petición y el código.
            let req = self.id();
            let (tx, rx) = channel();
            let code = ch.code.clone();
            self.st.lock().unwrap().requests.push(Req { req, name: name.clone(), platform: plat.clone(), code: code.clone(), monitor, answer: tx });
            self.emit(json!({"t": "link", "ev": "request", "req": req, "name": name, "code": code, "monitor": monitor}));
            self.emit_state();
            let answer = rx.recv_timeout(Duration::from_secs(90));
            {
                let mut st = self.st.lock().unwrap();
                st.requests.retain(|r| r.req != req);
            }
            self.emit_state();
            let Ok((accept, remember)) = answer else {
                let _ = ch.send_typed(msg::REJECT, b"no se respondio a tiempo");
                return Ok(());
            };
            if !accept {
                ch.send_typed(msg::REJECT, format!("{} rechazó la conexión", self.name).as_bytes())?;
                return Ok(());
            }
            if remember {
                let mut st = self.st.lock().unwrap();
                let ip = peer_addr.map(|a| a.ip().to_string()).unwrap_or_default();
                match st.saved.iter_mut().find(|s| s.key == key) {
                    Some(s) => { s.trusted = true; s.name = name.clone(); s.platform = plat.clone(); }
                    None => st.saved.push(Saved { key: key.clone(), name: name.clone(), platform: plat.clone(), addr: ip, trusted: true, manual: false }),
                }
                self.save(&st);
            }
        }
        ch.send_typed(msg::ACCEPT, &[])?;
        let session = self.id();
        let sock = ch.shutdown_handle().ok();
        self.st.lock().unwrap().sessions.push(Sess { session, dir: "out", peer: name.clone(), monitor, state: "live", code: ch.code.clone(), sock, stopped: false });
        self.emit_state();
        let r = stream::serve(&mut ch, monitor, FPS, MBPS);
        self.end_session(session);
        if let Err(e) = r { eprintln!("sesión con {name} terminada: {e}"); }
        Ok(())
    }

    // ------------------------------------------------------------ descubrimiento

    fn announce(&self) {
        let b = discovery::encode(&self.beacon());
        for t in discovery::broadcast_targets() { let _ = self.udp.send_to(&b, t); }
    }

    fn discovery_loop(self: &Arc<Self>) {
        let mut tick = 0u64;
        loop {
            if tick % 5 == 0 {
                let d: Vec<(u32, u32)> = platform::displays().map(|v| v.iter().map(|d| (d.w, d.h)).collect()).unwrap_or_default();
                *self.displays.lock().unwrap() = d;
            }
            self.announce();
            // Los agregados por IP se consultan directamente.
            let manual: Vec<String> = self.st.lock().unwrap().saved.iter().filter(|s| s.manual || !s.addr.is_empty()).map(|s| s.addr.clone()).collect();
            for a in manual {
                if let Ok(sa) = a.parse::<SocketAddr>() { let _ = self.udp.send_to(&discovery::query(), SocketAddr::new(sa.ip(), discovery::PORT)); }
            }
            // Avisar cuando un dispositivo deja de responder.
            let changed = {
                let mut st = self.st.lock().unwrap();
                let mut changed = false;
                for d in st.devices.values_mut() {
                    if let Some(t) = d.seen {
                        if t.elapsed() > ONLINE { d.seen = None; changed = true; }
                    }
                }
                changed
            };
            if changed { self.emit_state(); }
            tick += 1;
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn receive_loop(self: &Arc<Self>) {
        let mut buf = [0u8; 2048];
        loop {
            let Ok((n, src)) = self.udp.recv_from(&mut buf) else { continue };
            match discovery::decode(&buf[..n]) {
                Some(Packet::Query) => { let _ = self.udp.send_to(&discovery::encode(&self.beacon()), src); }
                Some(Packet::Beacon(b)) if b.id != self.id => self.on_beacon(b, src.ip()),
                _ => {}
            }
        }
    }

    fn on_beacon(&self, b: Beacon, ip: IpAddr) {
        let mut addr = SocketAddr::new(ip, b.port);
        let mut st = self.st.lock().unwrap();
        // Con varias interfaces (VPN, adaptadores virtuales) el mismo anuncio llega desde varias IP:
        // se conserva la dirección mientras siga respondiendo, para no saltar de una a otra.
        if let Some(d) = st.devices.get(&b.id) {
            if let (Some(prev), Some(seen)) = (d.addr, d.seen) {
                if seen.elapsed() < ONLINE && prev.port() == b.port { addr = prev; }
            }
        }
        let fresh = match st.devices.get(&b.id) {
            Some(d) => d.seen.is_none() || d.addr != Some(addr) || d.name != b.name || d.displays != b.displays || d.sharing != b.sharing,
            None => true,
        };
        st.devices.insert(b.id.clone(), Dev { name: b.name, platform: b.platform, addr: Some(addr), seen: Some(Instant::now()), displays: b.displays, sharing: b.sharing });
        drop(st);
        if fresh { self.emit_state(); }
    }
}

/// Arranca el agente. Termina cuando la app cierra stdin.
pub fn run(port: u16) -> Res<()> {
    let el = viewer::event_loop(true)?;
    let proxy = el.create_proxy();
    let me = store::identity();
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("no se pudo abrir el puerto {port}: {e}"))?;
    let sharing = std::fs::read_to_string(settings_path()).map(|s| !s.contains("sharing=0")).unwrap_or(true);
    let saved = store::load_devices();
    let mut devices = HashMap::new();
    for s in &saved {
        if let Some(k) = net::unhex(&s.key) {
            devices.insert(id_of(&k), Dev { name: s.name.clone(), platform: s.platform.clone(), addr: s.addr.parse().ok(), seen: None, displays: vec![], sharing: true });
        }
    }
    let agent = Arc::new(Agent {
        id: me.id(),
        me,
        name: store::device_name(),
        port,
        sharing: AtomicBool::new(sharing),
        next: AtomicU64::new(0),
        out: Mutex::new(std::io::stdout()),
        st: Mutex::new(State { devices, saved, ..Default::default() }),
        displays: Mutex::new(platform::displays().map(|v| v.iter().map(|d| (d.w, d.h)).collect()).unwrap_or_default()),
        proxy,
        udp: discovery::open()?,
    });
    { let a = agent.clone(); std::thread::spawn(move || a.listen(listener)); }
    { let a = agent.clone(); std::thread::spawn(move || a.discovery_loop()); }
    { let a = agent.clone(); std::thread::spawn(move || a.receive_loop()); }
    {
        let a = agent.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                match line { Ok(l) => a.command(&l), Err(_) => break }
            }
            // La app se cerró: el agente también.
            std::process::exit(0);
        });
    }
    agent.emit_state();
    let a = agent.clone();
    viewer::run(el, true, move |s| a.stop_session(s))?;
    Ok(())
}
