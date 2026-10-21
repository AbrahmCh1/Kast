//! Agente de Kast Link: lo arranca la app de Kast (bandeja en Windows, barra de menús en macOS) y se
//! comunica con ella por stdin/stdout con una línea JSON por mensaje.
//!
//! App -> agente:
//!   {"cmd":"state"}                                   reenviar el estado completo
//!   {"cmd":"connect","id":"…","monitor":1,"show_on":0,"control":true} ver (y controlar) el monitor 1 de un dispositivo (0 = ventana, n = monitor local n)
//!   {"cmd":"add","addr":"192.168.1.20"}               agregar un dispositivo por IP
//!   {"cmd":"forget","id":"…"}                          olvidar un dispositivo (y su confianza)
//!   {"cmd":"answer","req":3,"accept":true,"remember":true,"control":true}
//!   {"cmd":"disconnect","session":5}
//!   {"cmd":"sharing","enabled":true}                   permitir que otros pidan ver mis pantallas
//!   {"cmd":"layout_save","path":"…\\X.link"}          guardar con una disposición las sesiones a pantalla completa
//!   {"cmd":"layout_load","path":"…\\X.link"}          restaurarlas (mantiene las que ya coinciden)
//!   {"cmd":"send_files","session":5,"paths":["…"]}    enviar archivos o carpetas a la sesión (con control)
//! Agente -> app:
//!   {"t":"link","ev":"state",…}   estado completo (yo, dispositivos, peticiones, sesiones)
//!   {"t":"link","ev":"request",…} alguien pide ver mis pantallas (la app debe mostrarse)
//!   {"t":"link","ev":"toast","msg":"…","err":false}
use crate::discovery::{self, Beacon, Packet};
use crate::net::{self, Identity, Secure, id_of, msg};
use crate::store::{self, Saved};
use crate::{Res, input, platform, share, stream, viewer};
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
    /// Otras direcciones desde las que llegó su anuncio (varias interfaces): se prueban si `addr` no responde.
    alts: Vec<SocketAddr>,
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
    /// Pide también controlar teclado y ratón.
    control: bool,
    answer: Sender<Answer>,
}

struct Answer {
    accept: bool,
    remember: bool,
    control: bool,
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
    /// Con control de teclado y ratón.
    control: bool,
    /// "in": dispositivo (id), monitor local (0 = ventana) y si se pidió control, para las disposiciones.
    dev: String,
    show_on: usize,
    want_control: bool,
    /// "in" con control: envía archivos al otro equipo.
    files: Option<viewer::FileDrop>,
}

/// Sesión guardada con una disposición: id, nombre, su pantalla, mi monitor y control.
struct LayoutSess {
    dev: String,
    name: String,
    monitor: usize,
    show_on: usize,
    control: bool,
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
        let requests: Vec<Value> = st.requests.iter().map(|r| json!({"req": r.req, "name": r.name, "platform": r.platform, "code": r.code, "monitor": r.monitor, "control": r.control})).collect();
        let sessions: Vec<Value> = st.sessions.iter().map(|s| json!({"session": s.session, "dir": s.dir, "peer": s.peer, "monitor": s.monitor, "state": s.state, "code": s.code, "control": s.control, "show_on": s.show_on})).collect();
        let me = json!({"id": self.id, "name": self.name, "platform": store::platform(), "port": self.port,
                        "sharing": self.sharing.load(Ordering::Relaxed), "capture": platform::capture_allowed(), "input": platform::input_allowed(),
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
                let control = v.get("control").and_then(Value::as_bool).unwrap_or(true);
                let a = self.clone();
                std::thread::spawn(move || a.connect(&id, monitor, show_on, control));
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
                    let _ = r.answer.send(Answer { accept: b("accept"), remember: b("remember"), control: b("control") });
                }
                drop(st);
                self.emit_state();
            }
            "disconnect" => self.stop_session(n("session")),
            "layout_save" => self.layout_save(&s("path")),
            "send_files" => {
                let paths: Vec<std::path::PathBuf> = v.get("paths").and_then(Value::as_array).into_iter().flatten()
                    .filter_map(Value::as_str).map(Into::into).collect();
                let files = self.st.lock().unwrap().sessions.iter().find(|x| x.session == n("session")).and_then(|x| x.files.clone());
                match files {
                    Some(f) => f(paths),
                    None => self.toast("Para enviar archivos hace falta una sesión con control", true),
                }
            }
            "layout_load" => {
                let (a, path) = (self.clone(), s("path"));
                std::thread::spawn(move || a.layout_load(&path));
            }
            "request_input" => {
                platform::request_input();
                self.toast("Activa Kast en «Accesibilidad» para que otros puedan controlar este equipo", false);
            }
            "request_capture" => {
                platform::request_capture();
                self.toast("Activa Kast en «Grabación de pantalla» y después cierra y vuelve a abrir Kast", false);
            }
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

    // ------------------------------------------------------------ disposiciones

    /// Guarda las sesiones a pantalla completa en un monitor (las de ventana no forman parte de la disposición).
    fn layout_save(&self, path: &str) {
        let st = self.st.lock().unwrap();
        let body: String = st.sessions.iter().filter(|s| s.dir == "in" && s.show_on > 0 && !s.stopped)
            .map(|s| format!("{}\t{}\t{}\t{}\t{}\n", s.dev, s.peer.replace(['\t', '\n', '\r'], " "), s.monitor, s.show_on, s.want_control as u8))
            .collect();
        drop(st);
        if let Err(e) = std::fs::write(path, body) { store::log(&format!("disposición {path}: {e}")); }
    }

    /// Deja exactamente las sesiones guardadas: mantiene (y recoloca) las que ya están, corta las demás
    /// de pantalla completa y conecta las que faltan. Sin archivo (disposición sin sesiones o anterior a esta
    /// función) es una disposición sin pantallas de otros equipos: se cierran las de pantalla completa.
    fn layout_load(self: &Arc<Self>, path: &str) {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let want: Vec<LayoutSess> = text.lines().filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 5).then(|| LayoutSess {
                dev: f[0].into(), name: f[1].into(), monitor: f[2].parse().unwrap_or(1), show_on: f[3].parse().unwrap_or(0), control: f[4] == "1",
            })
        }).filter(|w| !w.dev.is_empty() && w.show_on > 0).collect();
        // Que Windows termine de aplicar la nueva disposición de pantallas antes de colocar ventanas.
        std::thread::sleep(Duration::from_millis(800));
        let same = |s: &Sess, w: &LayoutSess| s.dev == w.dev && s.monitor == w.monitor && s.show_on == w.show_on;
        let (keep, stop, missing) = {
            let st = self.st.lock().unwrap();
            let live: Vec<&Sess> = st.sessions.iter().filter(|s| s.dir == "in" && !s.stopped).collect();
            let keep: Vec<(u64, usize)> = live.iter().filter(|s| want.iter().any(|w| same(s, w))).map(|s| (s.session, s.show_on)).collect();
            let stop: Vec<u64> = live.iter().filter(|s| s.show_on > 0 && !want.iter().any(|w| same(s, w))).map(|s| s.session).collect();
            let missing: Vec<&LayoutSess> = want.iter().filter(|w| !live.iter().any(|s| same(s, w))).collect();
            (keep, stop, missing.into_iter().map(|w| LayoutSess { dev: w.dev.clone(), name: w.name.clone(), ..*w }).collect::<Vec<_>>())
        };
        for s in stop { self.stop_session(s); }
        for (session, monitor) in keep { let _ = self.proxy.send_event(viewer::Event::Place { session, monitor }); }
        for w in missing {
            let a = self.clone();
            std::thread::spawn(move || {
                // Recién arrancado (o con la red cambiando), el dispositivo puede tardar en anunciarse.
                if !a.wait_online(&w.dev, Duration::from_secs(20)) {
                    a.toast(format!("{} no está en la red: no se pudo abrir su pantalla en tu monitor {}", w.name, w.show_on), true);
                    return;
                }
                a.connect(&w.dev, w.monitor, w.show_on, w.control);
            });
        }
    }

    fn wait_online(&self, id: &str, max: Duration) -> bool {
        if id.starts_with("ip:") { return true; }
        let t = Instant::now();
        loop {
            let online = self.st.lock().unwrap().devices.get(id).and_then(|d| d.seen).map(|s| s.elapsed() < ONLINE).unwrap_or(false);
            if online { return true; }
            if t.elapsed() > max { return false; }
            std::thread::sleep(Duration::from_millis(300));
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

    fn connect(self: &Arc<Self>, id: &str, monitor: usize, show_on: usize, control: bool) {
        let (addrs, name) = {
            let st = self.st.lock().unwrap();
            if let Some(a) = id.strip_prefix("ip:") {
                (a.parse::<SocketAddr>().ok().into_iter().collect::<Vec<_>>(), a.to_string())
            } else {
                match st.devices.get(id) {
                    Some(d) => {
                        let mut v: Vec<SocketAddr> = d.addr.into_iter().collect();
                        v.extend(d.alts.iter().filter(|a| Some(**a) != d.addr));
                        (v, d.name.clone())
                    }
                    None => (vec![], id.to_string()),
                }
            }
        };
        if addrs.is_empty() { self.toast(format!("{name} no está disponible"), true); return }
        let session = self.id();
        self.st.lock().unwrap().sessions.push(Sess { session, dir: "in", peer: name.clone(), monitor, state: "connecting", code: String::new(), sock: None,
                                                     stopped: false, control, dev: id.to_string(), show_on, want_control: control, files: None });
        self.emit_state();
        let r = self.view(session, id, &addrs, &name, monitor, show_on, control);
        let _ = self.proxy.send_event(viewer::Event::Close(session));
        self.end_session(session);
        match r {
            Ok(()) => {}
            Err(e) => { store::log(&format!("ver {name}: {e}")); self.toast(format!("{name}: {e}"), true) }
        }
    }

    fn view(self: &Arc<Self>, session: u64, id: &str, addrs: &[SocketAddr], name: &str, monitor: usize, show_on: usize, control: bool) -> Res<()> {
        // La dirección conocida primero; si no responde, las demás desde las que se anunció.
        let (stream, addr) = addrs.iter().enumerate()
            .find_map(|(i, a)| TcpStream::connect_timeout(a, Duration::from_secs(if i == 0 { 4 } else { 2 })).ok().map(|s| (s, *a)))
            .ok_or("no responde")?;
        if addr != addrs[0] {
            store::log(&format!("{name}: {} no responde, conectado por {addr}", addrs[0]));
            if let Some(d) = self.st.lock().unwrap().devices.get_mut(id) { d.addr = Some(addr); }
        }
        let mut ch = Secure::connect(stream, &self.me)?;
        let peer_key = net::hex(&ch.peer);
        let peer_id = id_of(&ch.peer);
        store::check_peer(&self.st.lock().unwrap().saved, id, &ch.peer)?;
        let sock = ch.shutdown_handle()?;
        let code = ch.code.clone();
        self.set_session(session, |s| { s.state = "waiting"; s.code = code; s.sock = Some(sock); });
        let req = json!({"name": self.name, "platform": store::platform(), "monitor": monitor, "control": control}).to_string();
        ch.send_typed(msg::REQUEST, req.as_bytes())?;
        let mut m = Vec::new();
        ch.recv(&mut m).map_err(|_| "conexión cerrada")?;
        match m.first() {
            Some(&msg::ACCEPT) => {}
            Some(&msg::REJECT) => return Err(String::from_utf8_lossy(&m[1..]).to_string().into()),
            _ => return Err("respuesta inesperada".into()),
        }
        // ¿Concedió el control? Si no, la sesión es sólo de vista (con el motivo, si lo hay).
        let granted: Value = serde_json::from_slice(&m[1..]).unwrap_or(Value::Null);
        let note = granted.get("note").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string)
            .or_else(|| (control && !granted.get("control").and_then(Value::as_bool).unwrap_or(false)).then(|| "aceptó sólo ver, sin control".into()));
        if let Some(note) = note {
            store::log(&format!("{name}: {note}"));
            self.toast(format!("{name}: {note}"), true);
        }
        let control = control && granted.get("control").and_then(Value::as_bool).unwrap_or(false);
        self.set_session(session, |s| s.control = control);
        // Recordar el dispositivo (su identidad, para detectar cambios).
        {
            let mut st = self.st.lock().unwrap();
            let (plat, dname) = st.devices.get(&peer_id).map(|d| (d.platform.clone(), d.name.clone())).unwrap_or_default();
            let ip = addr.to_string();
            match st.saved.iter_mut().find(|s| s.key == peer_key || (s.key.is_empty() && s.addr == ip)) {
                Some(s) => { s.key = peer_key.clone(); s.addr = ip; if !dname.is_empty() { s.name = dname; s.platform = plat; } }
                None => st.saved.push(Saved { key: peer_key, name: dname, platform: plat, addr: ip, ..Default::default() }),
            }
            self.save(&st);
        }
        ch.recv(&mut m).map_err(|_| format!("{name} cerró la conexión al empezar a transmitir (revisa Kast → Dispositivos en ese equipo)"))?;
        match m.first() {
            Some(&msg::HELLO) => {}
            Some(&msg::REJECT) => return Err(String::from_utf8_lossy(&m[1..]).to_string().into()),
            _ => return Err("respuesta inesperada".into()),
        }
        let (w, h, fps) = stream::parse_hello(&m[1..]).ok_or("respuesta inesperada")?;
        let shared = Arc::new(Mutex::new(viewer::Shared { frame: vec![0; w * h], w, h, cursor: (0, 0, false) }));
        let title = if control {
            format!("Kast Link — {name} · pantalla {monitor} · control (Ctrl+Alt+Shift+F pantalla completa, Ctrl+Alt+Shift+Q salir)")
        } else {
            format!("Kast Link — {name} · pantalla {monitor}")
        };
        // Con control, la ventana envía teclado y ratón por un hilo propio (no bloquea la ventana), el
        // portapapeles se comparte y se pueden arrastrar archivos a la ventana.
        let (input, files, clip) = if control {
            let (tx, rx) = channel::<Vec<u8>>();
            let mut out = ch.try_clone()?;
            std::thread::spawn(move || { for m in rx { if out.send(&m).is_err() { break; } } });
            let ctx = tx.clone();
            let clip = share::ClipSync::start(move |m| ctx.send(m).is_ok());
            (Some(tx), Some(self.file_drop(session, name, ch.try_clone()?)), Some(clip))
        } else {
            (None, None, None)
        };
        self.set_session(session, |s| s.files = files.clone());
        let _ = self.proxy.send_event(viewer::Event::Open { session, shared: shared.clone(), monitor: (show_on > 0).then_some(show_on), title, input, files });
        self.set_session(session, |s| s.state = "live");
        let proxy = self.proxy.clone();
        let r = stream::receive(&mut ch, &shared, w, h, fps, &move || { let _ = proxy.send_event(viewer::Event::Redraw(session)); },
                                &mut |m| if let Some(c) = &clip { c.on_message(m) });
        // Cerrar la ventana o pulsar "Desconectar" corta la conexión a propósito: no es un error.
        let stopped = self.st.lock().unwrap().sessions.iter().any(|s| s.session == session && s.stopped);
        match r {
            Err(_) if stopped => Ok(()),
            // El otro equipo detuvo la transmisión (o se cerró Kast allí).
            Err(e) if e.downcast_ref::<std::io::Error>().is_some() => { self.toast(format!("{name} terminó la sesión"), false); Ok(()) }
            r => r,
        }
    }

    /// Envío de archivos de una sesión: en su propio hilo (los datos van intercalados con el teclado y el ratón)
    /// y con el progreso en el título del visor.
    fn file_drop(self: &Arc<Self>, session: u64, peer: &str, ch: Secure) -> viewer::FileDrop {
        let ch = Arc::new(Mutex::new(ch));
        let busy = Arc::new(AtomicBool::new(false));
        let (a, peer) = (self.clone(), peer.to_string());
        Arc::new(move |paths: Vec<std::path::PathBuf>| {
            if paths.is_empty() { return; }
            if busy.swap(true, Ordering::SeqCst) {
                a.toast(format!("Espera a que termine el envío en curso a {peer}"), true);
                return;
            }
            let (a, ch, busy, peer) = (a.clone(), ch.clone(), busy.clone(), peer.clone());
            std::thread::spawn(move || {
                let proxy = a.proxy.clone();
                let status = |t: Option<String>| { let _ = proxy.send_event(viewer::Event::Status { session, text: t }); };
                match share::send_files(&paths, &mut |m| ch.lock().unwrap().send(m), &status) {
                    Ok((n, bytes)) => {
                        let what = format!("{n} archivo{} ({})", if n == 1 { "" } else { "s" }, share::human(bytes));
                        store::log(&format!("enviados a {peer}: {what}"));
                        a.toast(format!("Enviados a {peer} (Descargas): {what}"), false);
                        status(Some(format!("enviados {what} a Descargas ✓")));
                        std::thread::sleep(Duration::from_secs(4));
                        status(None);
                    }
                    Err(e) => {
                        status(None);
                        store::log(&format!("enviar archivos a {peer}: {e}"));
                        a.toast(format!("No se pudieron enviar los archivos a {peer}: {e}"), true);
                    }
                }
                busy.store(false, Ordering::SeqCst);
            });
        })
    }

    // ------------------------------------------------------------ otro me ve

    fn listen(self: &Arc<Self>, listener: TcpListener) {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let a = self.clone();
            std::thread::spawn(move || {
                let peer = stream.peer_addr().ok();
                if let Err(e) = a.host(stream, peer) { store::log(&format!("sesión entrante: {e}")); }
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
        let wants_control = v.get("control").and_then(Value::as_bool).unwrap_or(false);
        if !self.sharing.load(Ordering::Relaxed) {
            ch.send_typed(msg::REJECT, format!("{} no permite ver sus pantallas", self.name).as_bytes())?;
            return Ok(());
        }
        // macOS: sin el permiso de Grabación de pantalla no se puede compartir. Se avisa en los dos lados.
        if platform::capture_allowed() == Some(false) {
            store::log(&format!("{name} pidió ver la pantalla {monitor}: falta el permiso de Grabación de pantalla"));
            self.emit(json!({"t": "link", "ev": "show"}));
            self.toast(format!("{name} quiso ver tu pantalla, pero Kast no tiene permiso de Grabación de pantalla: pulsa «Dar permiso»"), true);
            ch.send_typed(msg::REJECT, format!("{} no ha dado a Kast el permiso de Grabación de pantalla", self.name).as_bytes())?;
            return Ok(());
        }
        if monitor > self.displays.lock().unwrap().len().max(1) {
            ch.send_typed(msg::REJECT, format!("{} no tiene la pantalla {monitor}", self.name).as_bytes())?;
            return Ok(());
        }
        let key = net::hex(&ch.peer);
        let (trusted, mut control) = self.st.lock().unwrap().saved.iter().find(|s| s.key == key && s.trusted)
            .map(|s| (true, s.control)).unwrap_or((false, false));
        // Se pregunta si no es de confianza o si pide un control que no tenía concedido (p. ej. recordado sólo para ver).
        if !trusted || (wants_control && !control) {
            // Hay que aceptar en este equipo: la app se muestra con la petición y el código.
            let req = self.id();
            let (tx, rx) = channel();
            let code = ch.code.clone();
            self.st.lock().unwrap().requests.push(Req { req, name: name.clone(), platform: plat.clone(), code: code.clone(), monitor, control: wants_control, answer: tx });
            self.emit(json!({"t": "link", "ev": "request", "req": req, "name": name, "platform": plat, "code": code, "monitor": monitor, "control": wants_control}));
            self.emit_state();
            let answer = rx.recv_timeout(Duration::from_secs(90));
            {
                let mut st = self.st.lock().unwrap();
                st.requests.retain(|r| r.req != req);
            }
            self.emit_state();
            let Ok(Answer { accept, remember, control: allow }) = answer else {
                let _ = ch.send_typed(msg::REJECT, b"no se respondio a tiempo");
                return Ok(());
            };
            if !accept {
                ch.send_typed(msg::REJECT, format!("{} rechazó la conexión", self.name).as_bytes())?;
                return Ok(());
            }
            control = allow;
            if remember {
                let mut st = self.st.lock().unwrap();
                let ip = peer_addr.map(|a| a.ip().to_string()).unwrap_or_default();
                match st.saved.iter_mut().find(|s| s.key == key) {
                    Some(s) => { s.trusted = true; s.control = allow; s.name = name.clone(); s.platform = plat.clone(); }
                    None => st.saved.push(Saved { key: key.clone(), name: name.clone(), platform: plat.clone(), addr: ip, trusted: true, manual: false, control: allow }),
                }
                self.save(&st);
            }
        }
        // Control de teclado y ratón: pedido, concedido y, en macOS, con el permiso de Accesibilidad.
        let mut note = String::new();
        let control = wants_control && control && if platform::input_allowed() == Some(false) {
            note = format!("sólo vista: en {}, Kast no tiene el permiso de Accesibilidad (allí: Kast → Dispositivos → Dar permiso)", self.name);
            store::log(&format!("{name} pidió control, pero falta el permiso de Accesibilidad"));
            self.toast(format!("{name} quiere controlar tu pantalla, pero Kast no tiene permiso de Accesibilidad: pulsa «Dar permiso»"), true);
            self.emit(json!({"t": "link", "ev": "show"}));
            false
        } else { true };
        // La captura se prepara antes de confirmar: si falla, el otro equipo recibe el motivo.
        let src = match stream::open(monitor, FPS, MBPS, !control) {
            Ok(s) => s,
            Err(e) => {
                store::log(&format!("no se pudo capturar la pantalla {monitor} para {name}: {e}"));
                self.toast(format!("No se pudo compartir tu pantalla {monitor}: {e}"), true);
                ch.send_typed(msg::REJECT, format!("{} no pudo capturar su pantalla: {e}", self.name).as_bytes())?;
                return Ok(());
            }
        };
        ch.send_typed(msg::ACCEPT, json!({"control": control, "note": note}).to_string().as_bytes())?;
        let session = self.id();
        let sock = ch.shutdown_handle().ok();
        if let Some(s) = &sock { s.set_read_timeout(None)?; }
        self.st.lock().unwrap().sessions.push(Sess { session, dir: "out", peer: name.clone(), monitor, state: "live", code: ch.code.clone(), sock,
                                                     stopped: false, control, dev: String::new(), show_on: 0, want_control: control, files: None });
        self.emit_state();
        store::log(&format!("{name} empieza a {} la pantalla {monitor}", if control { "controlar" } else { "ver" }));
        // Con control: teclado y ratón del visor inyectados aquí, portapapeles compartido y archivos recibidos.
        let _clip = if control {
            let (w, h) = src.size();
            // Pruebas (KAST_LINK_TEST): la entrada se anuncia en vez de inyectarse en este equipo.
            let test = std::env::var_os("KAST_LINK_TEST").is_some();
            let mut inj = if test { None } else { Some(platform::Injector::new(monitor, w, h)?) };
            let mut rx = ch.try_clone()?;
            let tx = Mutex::new(ch.try_clone()?);
            // En pruebas no se vigila el portapapeles real de este equipo.
            let clip = (!test).then(|| Arc::new(share::ClipSync::start(move |m| tx.lock().unwrap().send(&m).is_ok())));
            let (viewer, host) = (plat.clone(), store::platform());
            let (clip2, a, peer) = (clip.clone(), self.clone(), name.clone());
            std::thread::spawn(move || {
                let mut m = Vec::new();
                let mut files = share::FileRecv::new();
                while rx.recv(&mut m).is_ok() {
                    match m.first() {
                        Some(&msg::INPUT) => {
                            if let Some(mut e) = input::Event::decode(&m[1..]) {
                                if let input::Event::Key { code, .. } = &mut e { *code = input::map_modifier(code, &viewer, host); }
                                match &mut inj {
                                    Some(i) => i.apply(e),
                                    None => a.emit(json!({"t": "link", "ev": "input", "input": input_json(&e)})),
                                }
                            }
                        }
                        Some(&(msg::CLIP | msg::CLIP_DATA)) => match &clip2 {
                            Some(c) => c.on_message(&m),
                            None => a.emit(json!({"t": "link", "ev": "input", "input": {"clip": m[0] == msg::CLIP, "bytes": m.len() - 1}})),
                        },
                        Some(&(msg::FILE | msg::FILE_DATA | msg::FILE_END)) => match files.on_message(&m) {
                            Ok(Some(r)) => {
                                let what = format!("{} archivo{} ({})", r.count, if r.count == 1 { "" } else { "s" }, share::human(r.bytes));
                                store::log(&format!("recibidos de {peer}: {what} en {}", r.first.display()));
                                a.toast(format!("{peer} te envió {what} a Descargas"), false);
                                share::reveal(&r.first);
                            }
                            Ok(None) => {}
                            Err(e) => {
                                store::log(&format!("recibir archivos de {peer}: {e}"));
                                a.toast(format!("No se pudo recibir un archivo de {peer}: {e}"), true);
                                files = share::FileRecv::new();
                            }
                        },
                        _ => {}
                    }
                }
            });
            clip
        } else {
            None
        };
        let r = stream::serve(&mut ch, src, FPS);
        self.end_session(session);
        store::log(&format!("sesión con {name} terminada{}", r.err().map(|e| format!(": {e}")).unwrap_or_default()));
        Ok(())
    }

    // ------------------------------------------------------------ descubrimiento

    fn announce(&self) {
        let b = discovery::encode(&self.beacon());
        for t in discovery::broadcast_targets() { let _ = self.udp.send_to(&b, t); }
    }

    fn discovery_loop(self: &Arc<Self>) {
        let mut tick = 0u64;
        let mut perms = (platform::capture_allowed(), platform::input_allowed());
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
            // Permisos de macOS concedidos (o retirados) desde Ajustes del Sistema.
            let now = (platform::capture_allowed(), platform::input_allowed());
            if changed || now != perms { perms = now; self.emit_state(); }
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
        let here = SocketAddr::new(ip, b.port);
        let mut alts = st.devices.get(&b.id).map(|d| d.alts.clone()).unwrap_or_default();
        alts.retain(|a| *a != here && a.port() == b.port);
        alts.insert(0, here);
        alts.truncate(4);
        let fresh = match st.devices.get(&b.id) {
            Some(d) => d.seen.is_none() || d.addr != Some(addr) || d.name != b.name || d.displays != b.displays || d.sharing != b.sharing,
            None => true,
        };
        st.devices.insert(b.id.clone(), Dev { name: b.name, platform: b.platform, addr: Some(addr), alts, seen: Some(Instant::now()), displays: b.displays, sharing: b.sharing });
        drop(st);
        if fresh { self.emit_state(); }
    }
}

/// Arranca el agente. Termina cuando la app cierra stdin.
pub fn run(port: u16) -> Res<()> {
    store::log_reset();
    let el = viewer::event_loop(true)?;
    let proxy = el.create_proxy();
    let me = store::identity();
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("no se pudo abrir el puerto {port}: {e}"))?;
    let sharing = std::fs::read_to_string(settings_path()).map(|s| !s.contains("sharing=0")).unwrap_or(true);
    let saved = store::load_devices();
    let mut devices = HashMap::new();
    for s in &saved {
        if let Some(k) = net::unhex(&s.key) {
            devices.insert(id_of(&k), Dev { name: s.name.clone(), platform: s.platform.clone(), addr: s.addr.parse().ok(), alts: vec![], seen: None, displays: vec![], sharing: true });
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

/// Evento de entrada en JSON (modo de pruebas KAST_LINK_TEST).
fn input_json(e: &input::Event) -> Value {
    match e {
        input::Event::Move { x, y } => json!({"move": [x, y]}),
        input::Event::Button { button, down, x, y } => json!({"button": button, "down": down, "at": [x, y]}),
        input::Event::Wheel { dx, dy } => json!({"wheel": [dx, dy]}),
        input::Event::Key { down, code } => json!({"key": code, "down": down}),
        input::Event::Text { text } => json!({"text": text}),
    }
}
