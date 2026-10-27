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
//!   {"cmd":"drag_query","session":5}                  como sacar el ratón del visor con el botón pulsado (pruebas)
//!   connect admite "screens":[{"monitor":1,"show_on":0},…] para ver varias pantallas del equipo a la vez (una sola
//!   aprobación), y "auth":"password"|"pair" y "secret": contraseña de acceso desatendido o código de emparejamiento;
//!   "id" también puede ser el ID de 9 dígitos de un equipo de la red. Una pantalla {"virtual":true,"show_on":n} pide
//!   al equipo una pantalla virtual nueva del tamaño de mi monitor n, colocada junto a las suyas como están mis
//!   monitores (ver virt.rs); con "show_on":0, en una ventana, del tamaño de mi monitor principal y a la derecha de lo
//!   que veo a pantalla completa; "at":[x,y,w,h], "px":[w,h] y "scale" sustituyen a los de mi monitor (pruebas)
//!   {"cmd":"access_set","unknown":"ask"|"reject","unattended":false,"password":"…","unattended_control":true}
//!   {"cmd":"pair_start","control":true} · {"cmd":"pair_cancel"}   código de emparejamiento de un solo uso
//!   {"cmd":"revoke","key":"…"}                         quitar el acceso a un dispositivo
//! Agente -> app:
//!   {"t":"link","ev":"state",…}   estado completo (yo, dispositivos, peticiones, sesiones)
//!   {"t":"link","ev":"request",…} alguien pide ver mis pantallas (la app debe mostrarse)
//!   {"t":"link","ev":"toast","msg":"…","err":false}
//!   {"t":"link","ev":"need_auth","id","name","monitor","show_on","control"}  el equipo pide contraseña o código
use crate::discovery::{self, Beacon, Packet};
use crate::net::{self, Identity, Secure, id_of, msg};
use crate::store::{self, Saved};
use crate::virt::{self, Rect};
use crate::{LocalMon, Res, access, input, platform, share, stream, viewer};
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
    /// Puede crear pantallas virtuales.
    vd: bool,
}

/// Pantalla que se quiere ver: una suya (`monitor`) o una virtual nueva (`virt`), y dónde mostrarla.
#[derive(Clone)]
struct Screen {
    monitor: usize,
    show_on: usize,
    virt: bool,
    /// Pruebas: rectángulo, píxeles y escala del monitor local (si no, los de mi monitor `show_on`).
    at: Option<Rect>,
    px: Option<(u32, u32)>,
    scale: Option<f64>,
}

impl Screen {
    fn real(monitor: usize, show_on: usize) -> Self { Self { monitor, show_on, virt: false, at: None, px: None, scale: None } }

    fn parse(x: &Value) -> Self {
        let virt = x.get("virtual").and_then(Value::as_bool).unwrap_or(false);
        Self {
            monitor: if virt { 0 } else { x.get("monitor").and_then(Value::as_u64).unwrap_or(1).max(1) as usize },
            show_on: x.get("show_on").and_then(Value::as_u64).unwrap_or(0) as usize,
            virt,
            at: x.get("at").and_then(Rect::from_json),
            px: x.get("px").and_then(|p| Some((p.get(0)?.as_u64()? as u32, p.get(1)?.as_u64()? as u32))),
            scale: x.get("scale").and_then(Value::as_f64),
        }
    }

    fn json(&self) -> Value { json!({"monitor": self.monitor, "show_on": self.show_on, "virtual": self.virt}) }

    /// Mi monitor donde se verá: rectángulo en mi escritorio, píxeles reales y escala.
    fn local(&self, mons: &[LocalMon]) -> Option<(Rect, (u32, u32), f64)> {
        let m = self.show_on.checked_sub(1).and_then(|i| mons.get(i));
        let at = self.at.or(m.map(|m| m.rect))?;
        let px = self.px.or(m.map(|m| m.px)).unwrap_or((at.w as u32, at.h as u32));
        Some((at, px, self.scale.or(m.map(|m| m.scale)).unwrap_or(1.0)))
    }
}

struct Req {
    req: u64,
    name: String,
    platform: String,
    code: String,
    monitor: usize,
    /// Todas las pantallas que pide ver (con una sola aprobación).
    monitors: Vec<usize>,
    /// Pide también controlar teclado y ratón.
    control: bool,
    /// Pantallas virtuales que pide crear.
    virt: usize,
    answer: Sender<Answer>,
}

/// Acceso concedido a un grupo de conexiones de un visor (varias pantallas a la vez, o cambiar de pantalla en iOS):
/// mientras haya alguna sesión del grupo abierta, las demás pantallas pedidas entran sin volver a preguntar.
/// Al cerrarse la última, se acaba (volver a entrar pide permiso, como siempre).
struct Grant {
    monitors: Vec<usize>,
    control: bool,
    via: &'static str,
    live: usize,
    /// Pantallas virtuales creadas para este visor: cada una se quita al cerrarse su sesión, y todas al acabar el grupo.
    virt: Vec<VEntry>,
}

struct VEntry {
    v: platform::vdisplay::Virtual,
    /// Sesiones que la están transmitiendo, y si alguna llegó a abrirse.
    streams: usize,
    used: bool,
}

/// Una sesión dentro de un grupo: al terminar, por donde sea, se descuenta. Si mostraba una pantalla virtual, la
/// pantalla se quita (macOS lleva sus ventanas a las demás); si era la última del grupo, se quitan todas.
struct GrantUse {
    a: Arc<Agent>,
    k: (String, String),
    virt_id: Option<u32>,
}

impl Drop for GrantUse {
    fn drop(&mut self) {
        let mut gone = false;
        {
            let mut st = self.a.st.lock().unwrap();
            if let Some(gr) = st.grants.get_mut(&self.k) {
                gr.live = gr.live.saturating_sub(1);
                if let Some(e) = gr.virt.iter_mut().find(|e| Some(e.v.id) == self.virt_id) { e.streams = e.streams.saturating_sub(1); }
                let n = gr.virt.len();
                gr.virt.retain(|e| !e.used || e.streams > 0);
                gone |= gr.virt.len() != n;
            }
            st.grants.retain(|_, gr| { gone |= !gr.valid() && !gr.virt.is_empty(); gr.valid() });
        }
        if gone {
            // El sistema la quita un instante después: entonces se anuncian mis pantallas.
            let a = self.a.clone();
            std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(1500)); a.displays_changed(); });
        }
    }
}

impl Grant {
    fn valid(&self) -> bool { self.live > 0 }
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
    /// Es una pantalla virtual creada para esta conexión.
    virt: bool,
    /// "in": dispositivo (id), monitor local (0 = ventana) y si se pidió control, para las disposiciones.
    dev: String,
    show_on: usize,
    want_control: bool,
    /// "in" con control: envía archivos al otro equipo.
    files: Option<viewer::FileDrop>,
    /// "in" con control: mensajes hacia el otro equipo (teclado, ratón, consultas de arrastre).
    input: Option<Sender<Vec<u8>>>,
    /// "out": cómo entró ("approved" aceptado en pantalla, "trusted" de confianza, "password" acceso desatendido,
    /// "pair" recién emparejado) y su clave pública (para cortar la sesión al quitarle el acceso).
    via: &'static str,
    peer_key: String,
}

/// Sesión guardada con una disposición: id, nombre, su pantalla (o una virtual), mi monitor y control.
struct LayoutSess {
    dev: String,
    name: String,
    monitor: usize,
    show_on: usize,
    control: bool,
    virt: bool,
}

#[derive(Default)]
struct State {
    devices: HashMap<String, Dev>,
    saved: Vec<Saved>,
    requests: Vec<Req>,
    sessions: Vec<Sess>,
    /// Quién puede entrar en este equipo (access.rs).
    access: access::Access,
    /// Código de emparejamiento activo (de un solo uso).
    pairing: Option<access::PairCode>,
    limiter: access::Limiter,
    /// Anfitrión: (clave del visor, grupo) -> acceso concedido.
    grants: HashMap<(String, String), Grant>,
    /// Visor: grupo de conexiones por dispositivo (se reutiliza para abrir o cambiar de pantalla sin preguntar).
    groups: HashMap<String, String>,
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
                 displays: self.displays.lock().unwrap().clone(), sharing: self.sharing.load(Ordering::Relaxed),
                 vd: platform::vdisplay::supported() }
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
                "vd": d.vd,
                "short_id": net::short_id(id),
                "displays": d.displays.iter().map(|(w, h)| json!([w, h])).collect::<Vec<_>>(),
            })
        }).collect();
        // Agregados por IP que aún no respondieron.
        let pending: Vec<Value> = st.saved.iter().filter(|s| s.manual && s.key.is_empty()
            && !st.devices.values().any(|d| d.addr.map(|a| s.addr.starts_with(&a.ip().to_string())).unwrap_or(false)))
            .map(|s| json!({"id": format!("ip:{}", s.addr), "name": s.addr, "platform": "", "addr": s.addr, "online": false,
                            "trusted": false, "known": false, "manual": true, "sharing": true, "displays": []}))
            .collect();
        let requests: Vec<Value> = st.requests.iter().map(|r| json!({"req": r.req, "name": r.name, "platform": r.platform, "code": r.code, "monitor": r.monitor,
                                                                   "monitors": r.monitors, "control": r.control, "virt": r.virt})).collect();
        let sessions: Vec<Value> = st.sessions.iter().map(|s| json!({"session": s.session, "dir": s.dir, "peer": s.peer, "monitor": s.monitor, "state": s.state, "code": s.code, "control": s.control, "show_on": s.show_on, "via": s.via, "virt": s.virt})).collect();
        // Dispositivos con acceso a este equipo (emparejados o recordados).
        let trusted: Vec<Value> = st.saved.iter().filter(|s| s.trusted && !s.key.is_empty())
            .map(|s| json!({"key": s.key, "name": s.name, "platform": s.platform, "control": s.control, "last_seen": s.last_seen,
                            "short_id": net::short_id(&id_of(&net::unhex(&s.key).unwrap_or_default()))}))
            .collect();
        let access = st.access.to_json();
        let pairing = st.pairing.as_ref().filter(|p| !p.expired()).map(|p| json!({"code": p.code, "control": p.control, "secs": p.secs_left()}));
        let me = json!({"id": self.id, "short_id": net::short_id(&self.id), "name": self.name, "platform": store::platform(), "port": self.port,
                        "sharing": self.sharing.load(Ordering::Relaxed), "capture": platform::capture_allowed(), "input": platform::input_allowed(),
                        "vd": platform::vdisplay::supported(),
                        "displays": self.displays.lock().unwrap().iter().map(|(w, h)| json!([w, h])).collect::<Vec<_>>()});
        drop(st);
        let all: Vec<Value> = devices.into_iter().chain(pending).collect();
        self.emit(json!({"t": "link", "ev": "state", "me": me, "devices": all, "requests": requests, "sessions": sessions,
                         "access": access, "pairing": pairing, "trusted": trusted}));
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
                let (mut id, monitor, show_on) = (s("id"), n("monitor").max(1) as usize, n("show_on") as usize);
                let control = v.get("control").and_then(Value::as_bool).unwrap_or(true);
                let auth = Some((s("auth"), s("secret"))).filter(|(k, _)| k == access::PASSWORD || k == access::PAIR);
                // ID de 9 dígitos: se busca entre los equipos de la red.
                if let Some(digits) = net::typed_short_id(&id) {
                    let found = self.st.lock().unwrap().devices.keys().find(|k| net::digits(&net::short_id(k)) == digits).cloned();
                    match found {
                        Some(k) => id = k,
                        None => return self.toast(format!("No encuentro el equipo con ID {} en tu red (¿tiene Kast abierto?)", net::short_id_fmt(&digits)), true),
                    }
                }
                // Varias pantallas: la primera pide permiso (por todas) y las demás se abren al aceptarse. Las reales
                // van primero: las virtuales se crean al aceptar y se colocan junto a ellas.
                let mut screens: Vec<Screen> = v.get("screens").and_then(Value::as_array).into_iter().flatten().map(Screen::parse).take(16).collect();
                let mut seen = std::collections::HashSet::new();
                screens.retain(|x| x.virt || seen.insert(x.monitor));
                if screens.is_empty() { screens.push(Screen::real(monitor, show_on)); }
                screens.sort_by_key(|x| x.virt);
                if screens[0].virt {
                    return self.toast("Para añadir pantallas virtuales, elige también una de sus pantallas reales: se colocan junto a ella", true);
                }
                let first = screens.remove(0);
                let a = self.clone();
                std::thread::spawn(move || a.connect(&id, first, control, auth, screens));
            }
            "access_set" => self.access_set(&v),
            "pair_start" => {
                let code = access::PairCode::new(b("control"));
                store::log(&format!("código de emparejamiento creado ({})", if code.control { "ver y controlar" } else { "sólo ver" }));
                self.st.lock().unwrap().pairing = Some(code);
                self.emit_state();
            }
            "pair_cancel" => {
                self.st.lock().unwrap().pairing = None;
                self.emit_state();
            }
            "revoke" => {
                let key = s("key");
                let mut st = self.st.lock().unwrap();
                let mut name = String::new();
                if let Some(d) = st.saved.iter_mut().find(|d| d.key == key) {
                    // Se conserva la clave (para detectar si cambia), pero ya no tiene acceso.
                    d.trusted = false;
                    d.control = false;
                    name = d.name.clone();
                }
                self.save(&st);
                st.grants.retain(|(k, _), _| *k != key);
                for x in st.sessions.iter_mut().filter(|x| x.dir == "out" && x.peer_key == key) {
                    x.stopped = true;
                    if let Some(sock) = &x.sock { let _ = sock.shutdown(std::net::Shutdown::Both); }
                }
                drop(st);
                store::log(&format!("acceso quitado a {name}"));
                self.toast(format!("{name} ya no tiene acceso a este equipo"), false);
                self.emit_state();
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
            // Como sacar el ratón del visor con el botón pulsado (para las pruebas): ¿se arrastran archivos allí?
            "drag_query" => {
                let tx = self.st.lock().unwrap().sessions.iter().find(|x| x.session == n("session")).and_then(|x| x.input.clone());
                if let Some(tx) = tx { let _ = tx.send(vec![msg::DRAG_QUERY]); }
            }
            "send_files" => {
                let paths: Vec<std::path::PathBuf> = v.get("paths").and_then(Value::as_array).into_iter().flatten()
                    .filter_map(Value::as_str).map(Into::into).collect();
                let files = self.st.lock().unwrap().sessions.iter().find(|x| x.session == n("session")).and_then(|x| x.files.clone());
                match files {
                    Some(f) => f(paths),
                    None => self.toast("Para enviar archivos hace falta una sesión con control", true),
                }
            }
            // Pruebas (KAST_LINK_TEST): mover el ratón como si viniera de la ventana de la sesión.
            "test_input" if std::env::var_os("KAST_LINK_TEST").is_some() => {
                let input = self.st.lock().unwrap().sessions.iter().find(|x| x.session == n("session")).and_then(|x| x.input.clone());
                if let Some(i) = input { let _ = i.send(input::Event::Move { x: n("x") as i32, y: n("y") as i32 }.encode()); }
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
            .map(|s| format!("{}\t{}\t{}\t{}\t{}\t{}\n", s.dev, s.peer.replace(['\t', '\n', '\r'], " "), if s.virt { 0 } else { s.monitor }, s.show_on,
                             s.want_control as u8, if s.virt { "virtual" } else { "" }))
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
                virt: f.get(5) == Some(&"virtual"),
            })
        }).filter(|w| !w.dev.is_empty() && w.show_on > 0).collect();
        // Que Windows termine de aplicar la nueva disposición de pantallas antes de colocar ventanas.
        std::thread::sleep(Duration::from_millis(800));
        let same = |s: &Sess, w: &LayoutSess| s.dev == w.dev && s.show_on == w.show_on && s.virt == w.virt && (w.virt || s.monitor == w.monitor);
        let (keep, stop, missing) = {
            let st = self.st.lock().unwrap();
            let live: Vec<&Sess> = st.sessions.iter().filter(|s| s.dir == "in" && !s.stopped).collect();
            let missing: Vec<&LayoutSess> = want.iter().filter(|w| !live.iter().any(|s| same(s, w))).collect();
            // Si falta una virtual de un equipo, se vuelven a abrir todas sus pantallas: las virtuales se crean y se
            // colocan junto a las reales al conectar.
            let redo: Vec<&str> = missing.iter().filter(|w| w.virt).map(|w| w.dev.as_str()).collect();
            let keep: Vec<(u64, usize)> = live.iter().filter(|s| !redo.contains(&s.dev.as_str()) && want.iter().any(|w| same(s, w)))
                .map(|s| (s.session, s.show_on)).collect();
            let stop: Vec<u64> = live.iter().filter(|s| s.show_on > 0 && !keep.iter().any(|k| k.0 == s.session)).map(|s| s.session).collect();
            let missing: Vec<LayoutSess> = want.iter().filter(|w| redo.contains(&w.dev.as_str()) || !live.iter().any(|s| same(s, w)))
                .map(|w| LayoutSess { dev: w.dev.clone(), name: w.name.clone(), ..*w }).collect();
            (keep, stop, missing)
        };
        for s in stop { self.stop_session(s); }
        for (session, monitor) in keep { let _ = self.proxy.send_event(viewer::Event::Place { session, monitor }); }
        // Las que faltan, por equipo: una sola conexión pide todas sus pantallas (y crea las virtuales).
        let mut devs: Vec<&str> = missing.iter().map(|w| w.dev.as_str()).collect();
        devs.sort_unstable();
        devs.dedup();
        for dev in devs {
            let mut ws: Vec<&LayoutSess> = missing.iter().filter(|w| w.dev == dev).collect();
            ws.sort_by_key(|w| w.virt);
            let mut screens: Vec<Screen> = ws.iter().map(|w| Screen { virt: w.virt, ..Screen::real(w.monitor.max(1), w.show_on) }).collect();
            let (name, show_on, control) = (ws[0].name.clone(), ws[0].show_on, ws[0].control);
            if screens[0].virt { self.toast(format!("{name}: la disposición sólo tiene pantallas virtuales; falta una de sus pantallas reales"), true); continue; }
            let first = screens.remove(0);
            let (a, dev) = (self.clone(), dev.to_string());
            std::thread::spawn(move || {
                // Recién arrancado (o con la red cambiando), el dispositivo puede tardar en anunciarse.
                if !a.wait_online(&dev, Duration::from_secs(20)) {
                    a.toast(format!("{name} no está en la red: no se pudo abrir su pantalla en tu monitor {show_on}"), true);
                    return;
                }
                a.connect(&dev, first, control, None, screens);
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

    /// `more`: otras pantallas que se abren en cuanto el equipo acepte la primera (las virtuales las crea al aceptar).
    fn connect(self: &Arc<Self>, id: &str, screen: Screen, control: bool, auth: Option<(String, String)>, more: Vec<Screen>) {
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
        self.st.lock().unwrap().sessions.push(Sess { session, dir: "in", peer: name.clone(), monitor: screen.monitor, state: "connecting", code: String::new(),
                                                     sock: None, stopped: false, control, virt: screen.virt, dev: id.to_string(), show_on: screen.show_on,
                                                     want_control: control, files: None, input: None, via: "", peer_key: String::new() });
        self.emit_state();
        let r = self.view(session, id, &addrs, &name, &screen, control, auth.as_ref(), &more);
        let _ = self.proxy.send_event(viewer::Event::Close(session));
        self.end_session(session);
        match r {
            Ok(()) => {}
            Err(e) => { store::log(&format!("ver {name}: {e}")); self.toast(format!("{name}: {e}"), true) }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn view(self: &Arc<Self>, session: u64, id: &str, addrs: &[SocketAddr], name: &str, screen: &Screen, control: bool,
            auth: Option<&(String, String)>, more: &[Screen]) -> Res<()> {
        let (mut monitor, show_on) = (screen.monitor, screen.show_on);
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
        // Grupo de este dispositivo: las demás pantallas (ahora o más tarde) entran con el mismo permiso.
        let group = self.st.lock().unwrap().groups.entry(id.to_string())
            .or_insert_with(|| { let mut b = [0u8; 12]; let _ = getrandom::fill(&mut b); net::hex(&b) }).clone();
        let monitors: Vec<usize> = std::iter::once(monitor).chain(more.iter().filter(|x| !x.virt).map(|x| x.monitor)).collect();
        let nvirt = more.iter().filter(|x| x.virt).count();
        let layout = (nvirt > 0).then(|| layout_plan(screen, more));
        let req = json!({"name": self.name, "platform": store::platform(), "monitor": monitor, "monitors": monitors, "group": group,
                         "control": control, "auth": auth.map(|(k, _)| k.as_str()), "virt": nvirt, "layout": layout}).to_string();
        ch.send_typed(msg::REQUEST, req.as_bytes())?;
        let mut m = Vec::new();
        loop {
            ch.recv(&mut m).map_err(|_| "conexión cerrada")?;
            match m.first() {
                Some(&msg::ACCEPT) => break,
                // Contraseña o código: se demuestra que se conoce sin enviarlo (access.rs).
                Some(&msg::CHALLENGE) => {
                    let secret = auth.map(|(_, s)| s.as_str()).unwrap_or("");
                    ch.send(&access::answer_challenge(&m[1..], secret, &ch.binding)?)?;
                }
                Some(&msg::REJECT) => {
                    let (reason, need_auth) = access::reject_reason(&m[1..]);
                    if need_auth {
                        let screens: Vec<Value> = std::iter::once(screen).chain(more).map(Screen::json).collect();
                        self.emit(json!({"t": "link", "ev": "need_auth", "id": id, "name": name, "monitor": monitor, "show_on": show_on,
                                         "screens": screens, "control": control, "failed": auth.is_some()}));
                    }
                    return Err(reason.into());
                }
                _ => return Err("respuesta inesperada".into()),
            }
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
        // Al crear las virtuales, el equipo pudo renumerar sus pantallas (se cuentan de izquierda a derecha).
        if let Some(m) = granted.get("monitor").and_then(Value::as_u64).filter(|m| *m > 0) { monitor = m as usize; }
        self.set_session(session, |s| { s.control = control; s.monitor = monitor; });
        let remap: Vec<(usize, usize)> = granted.get("remap").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|r| Some((r.get(0)?.as_u64()? as usize, r.get(1)?.as_u64()? as usize))).collect();
        let mut made = granted.get("virtual").and_then(Value::as_array).into_iter().flatten().map(|x| x.as_u64().map(|m| m as usize));
        if nvirt > 0 {
            let ok = granted.get("virtual").and_then(Value::as_array).map(|a| a.iter().filter(|x| x.is_u64()).count()).unwrap_or(0);
            let why = granted.get("virtual_error").and_then(Value::as_str).map(str::to_string)
                .or_else(|| granted.get("virtual").is_none().then(|| "su versión de Kast no lo permite".into()));
            if ok < nvirt {
                let what = if ok == 0 { "las pantallas virtuales".to_string() } else { format!("{} de {nvirt} pantallas virtuales", nvirt - ok) };
                self.toast(format!("{name} no pudo crear {what}{}", why.map(|w| format!(": {w}")).unwrap_or_default()), true);
            }
        }
        // Aceptada: se abren las demás pantallas pedidas (entran con el permiso del grupo, sin volver a preguntar).
        for x in more {
            let mut x = x.clone();
            if x.virt {
                match made.next().flatten() { Some(m) => x.monitor = m, None => continue }
            } else if let Some(&(_, m)) = remap.iter().find(|r| r.0 == x.monitor) {
                x.monitor = m;
            }
            let (a, id) = (self.clone(), id.to_string());
            std::thread::spawn(move || a.connect(&id, x, control, None, Vec::new()));
        }
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
            Some(&msg::REJECT) => return Err(access::reject_reason(&m[1..]).0.into()),
            _ => return Err("respuesta inesperada".into()),
        }
        let (w, h, fps) = stream::parse_hello(&m[1..]).ok_or("respuesta inesperada")?;
        let shared = Arc::new(Mutex::new(viewer::Shared { frame: vec![0; w * h], w, h, cursor: (0, 0, false) }));
        let what = if screen.virt { "pantalla virtual".to_string() } else { format!("pantalla {monitor}") };
        let title = if control {
            format!("Kast Link — {name} · {what} · control (Ctrl+Alt+Shift+F pantalla completa, Ctrl+Alt+Shift+Q salir)")
        } else {
            format!("Kast Link — {name} · {what}")
        };
        // Dónde está esa pantalla en el otro equipo: arrastrar de una ventana a otra del mismo equipo sigue allí.
        let geo = granted.get("bounds").and_then(Rect::from_json).map(|bounds| viewer::Geo { group: id.to_string(), bounds });
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
        self.set_session(session, |s| { s.files = files.clone(); s.input = input.clone(); });
        let _ = self.proxy.send_event(viewer::Event::Open { session, shared: shared.clone(), monitor: (show_on > 0).then_some(show_on), title, input, files, geo });
        self.set_session(session, |s| s.state = "live");
        let proxy = self.proxy.clone();
        let mut drag = DragIn { a: self.clone(), session, peer: name.to_string(), recv: None, done: None };
        let r = stream::receive(&mut ch, &shared, w, h, fps, &move || { let _ = proxy.send_event(viewer::Event::Redraw(session)); },
                                &mut |m| match m.first() {
                                    Some(&(msg::DRAG_FILES | msg::FILE | msg::FILE_DATA | msg::FILE_END)) => drag.on_message(m),
                                    _ => if let Some(c) = &clip { c.on_message(m) },
                                });
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
                // De fondo: el teclado y el ratón de esta ventana pasan antes que los trozos del archivo.
                match share::send_files(&paths, &mut |m| ch.lock().unwrap().send_bulk(m), &status) {
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

    /// El visor sacó el ratón de su ventana con el botón pulsado. Si aquí se están arrastrando archivos (el arrastre
    /// empezó con ese clic), se le dice cuáles y se le envían para que los suelte en su equipo.
    fn drag_out(self: &Arc<Self>, left: bool, since: i64, ch: &Arc<Mutex<Secure>>, peer: &str) {
        let (a, ch, peer) = (self.clone(), ch.clone(), peer.to_string());
        // En su propio hilo: leer los archivos puede hacer que macOS pida permiso (Escritorio, Documentos…), y
        // mientras tanto el teclado y el ratón del visor tienen que seguir llegando para poder aceptarlo.
        std::thread::spawn(move || {
            let paths: Vec<std::path::PathBuf> = match std::env::var_os("KAST_LINK_TEST_DRAG") {
                Some(p) => vec![p.into()],
                None if left => platform::clipboard::dragged_files(since),
                None => Vec::new(),
            };
            let items: Vec<Value> = paths.iter().filter_map(|p| Some(json!({"name": p.file_name()?.to_string_lossy(), "dir": p.is_dir()}))).collect();
            let (count, bytes) = share::summary(&paths);
            let mut m = vec![msg::DRAG_FILES];
            m.extend_from_slice(json!({"items": items, "count": count, "bytes": bytes}).to_string().as_bytes());
            if ch.lock().unwrap().send(&m).is_err() || count == 0 { return; }
            match share::send_files(&paths, &mut |m| ch.lock().unwrap().send_bulk(m), &|_| {}) {
                Ok((n, bytes)) => store::log(&format!("{peer} saca arrastrando {n} archivo{} ({})", if n == 1 { "" } else { "s" }, share::human(bytes))),
                Err(e) => {
                    store::log(&format!("sacar archivos arrastrando hacia {peer}: {e}"));
                    a.toast(format!("No se pudieron enviar a {peer} los archivos arrastrados: {e}"), true);
                }
            }
        });
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
        let mut monitor = v.get("monitor").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
        let wants_control = v.get("control").and_then(Value::as_bool).unwrap_or(false);
        let auth = v.get("auth").and_then(Value::as_str).unwrap_or("").to_string();
        let group = v.get("group").and_then(Value::as_str).filter(|g| !g.is_empty() && g.len() <= 64).map(str::to_string);
        // Pantallas virtuales que pide (sólo dentro de un grupo, que es quien las mantiene vivas).
        let plan = v.get("layout").filter(|l| group.is_some() && l.get("virtual").and_then(Value::as_array).is_some_and(|a| !a.is_empty())).cloned();
        let nvirt = plan.as_ref().and_then(|l| l["virtual"].as_array()).map(|a| a.len().min(4)).unwrap_or(0);
        let ndisp = self.displays.lock().unwrap().len().max(1);
        let mut monitors: Vec<usize> = v.get("monitors").and_then(Value::as_array).into_iter().flatten()
            .filter_map(Value::as_u64).map(|m| m as usize).filter(|m| (1..=ndisp).contains(m)).take(16).collect();
        // La app de iOS pide todas (puede cambiar de pantalla durante la sesión).
        if v.get("all_monitors") == Some(&Value::Bool(true)) { monitors = (1..=ndisp).collect(); }
        if !monitors.contains(&monitor) { monitors.insert(0, monitor); }
        monitors.sort_unstable();
        monitors.dedup();
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
        let mut via = "trusted";
        // Otra pantalla de un grupo ya aceptado: entra con el mismo permiso, sin volver a preguntar (sólo si no
        // tiene ya acceso por sí mismo: un dispositivo de confianza conserva el suyo).
        let has_access = trusted && (!wants_control || control);
        let granted = group.as_ref().filter(|_| !has_access).and_then(|g| {
            let st = self.st.lock().unwrap();
            st.grants.get(&(key.clone(), g.clone())).filter(|gr| gr.valid() && gr.monitors.contains(&monitor)).map(|gr| (gr.control, gr.via))
        });
        if let Some((c, v)) = granted {
            control = c;
            via = v;
        } else if auth == access::PASSWORD || auth == access::PAIR {
            // Contraseña de acceso desatendido o código de emparejamiento: entra sin que nadie acepte.
            match self.authenticate(&mut ch, &auth, &key, &name, &plat, peer_addr) {
                Ok(allow) => { control = allow; via = if auth == access::PAIR { "pair" } else { "password" }; }
                Err(e) => {
                    ch.send_typed(msg::REJECT, &access::reject_body(&e, true))?;
                    return Ok(());
                }
            }
        } else if !trusted && self.st.lock().unwrap().access.reject_unknown {
            // Sólo entran los emparejados o con la contraseña: no se molesta a nadie con la petición.
            store::log(&format!("{name} pidió ver la pantalla {monitor}: rechazado (sólo dispositivos emparejados o con contraseña)"));
            let why = format!("{} sólo acepta dispositivos emparejados o con la contraseña de acceso", self.name);
            ch.send_typed(msg::REJECT, &access::reject_body(&why, true))?;
            return Ok(());
        } else if !trusted || (wants_control && !control) {
            // Se pregunta si no es de confianza o si pide un control que no tenía concedido (p. ej. recordado sólo para ver).
            via = "approved";
            // Hay que aceptar en este equipo: la app se muestra con la petición y el código.
            let req = self.id();
            let (tx, rx) = channel();
            let code = ch.code.clone();
            self.st.lock().unwrap().requests.push(Req { req, name: name.clone(), platform: plat.clone(), code: code.clone(), monitor,
                                                        monitors: monitors.clone(), control: wants_control, virt: nvirt, answer: tx });
            self.emit(json!({"t": "link", "ev": "request", "req": req, "name": name, "platform": plat, "code": code, "monitor": monitor,
                             "monitors": monitors, "control": wants_control, "virt": nvirt}));
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
                    None => st.saved.push(Saved { key: key.clone(), name: name.clone(), platform: plat.clone(), addr: ip, trusted: true, manual: false, control: allow, last_seen: 0 }),
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
        // Pantallas virtuales pedidas: se crean ahora, ya con permiso, colocadas como los monitores del visor. Al
        // crearlas puede cambiar el número de las pantallas (se cuentan de izquierda a derecha).
        let mut made = Vec::new();
        let mut vinfo = json!({});
        if let Some(plan) = &plan {
            match self.make_virtual(plan, &name) {
                Ok((list, remap, idx)) => {
                    let new = |m: usize| remap.iter().find(|r| r.0 == m).map(|r| r.1).unwrap_or(m);
                    monitor = new(monitor);
                    monitors = monitors.iter().map(|m| new(*m)).chain(idx.iter().flatten().copied()).collect();
                    store::log(&format!("{} pantalla{} virtual{} para {name}", list.len(), if list.len() == 1 { "" } else { "s" }, if list.len() == 1 { "" } else { "es" }));
                    vinfo = json!({"remap": remap, "virtual": idx});
                    made = list;
                }
                Err(e) => {
                    store::log(&format!("pantallas virtuales para {name}: {e}"));
                    vinfo = json!({"virtual_error": e.to_string()});
                }
            }
        }
        // ¿Es una de las pantallas virtuales creadas para este grupo?
        let my_id = platform::display_ids().get(monitor - 1).copied();
        let virt_id = group.as_ref().and_then(|g| {
            let st = self.st.lock().unwrap();
            let gr = st.grants.get(&(key.clone(), g.clone()))?;
            gr.virt.iter().find(|e| Some(e.v.id) == my_id).map(|e| e.v.id)
        });
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
        // Con sus pantallas: el visor puede ofrecer cambiar de una a otra aunque haya entrado por ID o por IP.
        let displays: Vec<Value> = self.displays.lock().unwrap().iter().map(|(w, h)| json!([w, h])).collect();
        let mut acc = json!({"control": control, "note": note, "displays": displays, "monitor": monitor,
                             "bounds": platform::display_bounds().get(monitor - 1).map(Rect::json)});
        if let (Some(a), Some(v)) = (acc.as_object_mut(), vinfo.as_object()) { a.extend(v.clone()); }
        // Permiso del grupo: las demás pantallas pedidas entran sin preguntar mientras dure (ver Grant). Se apunta antes
        // de confirmar, porque el visor abre las demás conexiones en cuanto recibe la confirmación.
        let _grant = group.as_ref().map(|g| {
            let mut st = self.st.lock().unwrap();
            let gr = st.grants.entry((key.clone(), g.clone())).or_insert(Grant { monitors: Vec::new(), control, via, live: 0, virt: Vec::new() });
            for m in &monitors { if !gr.monitors.contains(m) { gr.monitors.push(*m); } }
            gr.control = control;
            gr.live += 1;
            gr.virt.extend(made.drain(..).map(|v| VEntry { v, streams: 0, used: false }));
            if let Some(e) = gr.virt.iter_mut().find(|e| Some(e.v.id) == virt_id) { e.streams += 1; e.used = true; }
            GrantUse { a: self.clone(), k: (key.clone(), g.clone()), virt_id }
        });
        ch.send_typed(msg::ACCEPT, acc.to_string().as_bytes())?;
        // Último acceso de los dispositivos con acceso (se ve en «Dispositivos con acceso»).
        {
            let mut st = self.st.lock().unwrap();
            if let Some(d) = st.saved.iter_mut().find(|d| d.key == key && d.trusted) {
                d.last_seen = store::now_secs();
                self.save(&st);
            }
        }
        if via == "password" {
            self.toast(format!("{name} entró con la contraseña de acceso desatendido ({})", if control { "controla" } else { "sólo ve" }), false);
        }
        let session = self.id();
        let sock = ch.shutdown_handle().ok();
        if let Some(s) = &sock { s.set_read_timeout(None)?; }
        self.st.lock().unwrap().sessions.push(Sess { session, dir: "out", peer: name.clone(), monitor, state: "live", code: ch.code.clone(), sock,
                                                     stopped: false, control, virt: virt_id.is_some(), dev: String::new(), show_on: 0, want_control: control,
                                                     files: None, input: None, via, peer_key: key.clone() });
        self.emit_state();
        store::log(&format!("{name} empieza a {} la pantalla {monitor}{}", if control { "controlar" } else { "ver" }, if virt_id.is_some() { " (virtual)" } else { "" }));
        // El visor puede pedir un fotograma clave (p. ej. la app de iOS al abrir la sesión o si su decodificador falla).
        let keyframe = Arc::new(AtomicBool::new(false));
        // El hilo que recibe del visor avisa en cuanto la conexión se cierra (ver stream::serve).
        let closed = Arc::new(AtomicBool::new(false));
        // Con control: teclado y ratón del visor inyectados aquí, portapapeles compartido y archivos recibidos.
        let _clip = if control {
            let (w, h) = src.size();
            // Pruebas (KAST_LINK_TEST): la entrada se anuncia en vez de inyectarse en este equipo.
            let test = std::env::var_os("KAST_LINK_TEST").is_some();
            let mut inj = if test { None } else { Some(platform::Injector::new(monitor, w, h)?) };
            let mut rx = ch.try_clone()?;
            let tx = Mutex::new(ch.try_clone()?);
            let drag_tx = Arc::new(Mutex::new(ch.try_clone()?));
            // En pruebas no se vigila el portapapeles real de este equipo.
            let clip = (!test).then(|| Arc::new(share::ClipSync::start(move |m| tx.lock().unwrap().send(&m).is_ok())));
            let (viewer, host) = (plat.clone(), store::platform());
            let (clip2, a, peer, kf, cl) = (clip.clone(), self.clone(), name.clone(), keyframe.clone(), closed.clone());
            std::thread::spawn(move || {
                let mut m = Vec::new();
                let mut files = share::FileRecv::new();
                // Botón izquierdo pulsado desde el visor y el portapapeles de arrastre de ese momento: si cambia
                // después, ese clic empezó a arrastrar algo (ver drag_out).
                let (mut left, mut drag_since) = (false, 0i64);
                while rx.recv(&mut m).is_ok() {
                    match m.first() {
                        Some(&msg::KEYFRAME) => kf.store(true, Ordering::Relaxed),
                        Some(&msg::DRAG_QUERY) => a.drag_out(left, drag_since, &drag_tx, &peer),
                        Some(&msg::INPUT) => {
                            if let Some(mut e) = input::Event::decode(&m[1..]) {
                                if let input::Event::Button { button: 0, down, .. } = e {
                                    left = down;
                                    if down { drag_since = platform::clipboard::drag_seq(); }
                                }
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
                cl.store(true, Ordering::Relaxed);
            });
            clip
        } else {
            // Sólo vista: lo único que manda el visor es la petición de fotograma clave.
            let (mut rx, kf, cl) = (ch.try_clone()?, keyframe.clone(), closed.clone());
            std::thread::spawn(move || {
                let mut m = Vec::new();
                while rx.recv(&mut m).is_ok() {
                    if m.first() == Some(&msg::KEYFRAME) { kf.store(true, Ordering::Relaxed); }
                }
                cl.store(true, Ordering::Relaxed);
            });
            None
        };
        let r = stream::serve(&mut ch, src, FPS, &keyframe, &closed);
        self.end_session(session);
        store::log(&format!("sesión con {name} terminada{}", r.err().map(|e| format!(": {e}")).unwrap_or_default()));
        Ok(())
    }

    /// Crea las pantallas virtuales del plan del visor y las coloca junto a mis pantallas como están sus monitores.
    /// Devuelve las pantallas, cómo cambió la numeración de las que ya había (antes, después) y el número de cada
    /// virtual (None si no se pudo crear).
    #[allow(clippy::type_complexity)]
    fn make_virtual(&self, plan: &Value, viewer: &str) -> Res<(Vec<platform::vdisplay::Virtual>, Vec<(usize, usize)>, Vec<Option<usize>>)> {
        if !platform::vdisplay::supported() { return Err(platform::vdisplay::UNSUPPORTED.into()); }
        let ids_before = platform::display_ids();
        // Mis pantallas que verá en sus monitores: dónde (en su escritorio) y, si lo dice, píxeles y escala de ese monitor.
        let mut fixed: Vec<(u32, Rect, Option<((u32, u32), f64)>)> = plan["fixed"].as_array().into_iter().flatten()
            .filter_map(|f| {
                let id = *ids_before.get((f.get("monitor")?.as_u64()? as usize).checked_sub(1)?)?;
                let px = f.get("px").and_then(|p| Some((p.get(0)?.as_u64()? as u32, p.get(1)?.as_u64()? as u32)));
                Some((id, Rect::from_json(f.get("at")?)?, px.map(|px| (px, f.get("scale").and_then(Value::as_f64).unwrap_or(1.0)))))
            })
            .collect();
        // Escala del monitor del visor (los visores anteriores sólo dicen si es Retina).
        let wanted: Vec<Option<(Rect, (u32, u32), f64)>> = plan["virtual"].as_array().into_iter().flatten().take(4).map(|x| {
            let px = x.get("px")?;
            let scale = x.get("scale").and_then(Value::as_f64).unwrap_or(if x.get("hidpi")?.as_bool()? { 2.0 } else { 1.0 });
            Some((Rect::from_json(x.get("at")?)?, (px.get(0)?.as_u64()? as u32, px.get(1)?.as_u64()? as u32), scale))
        }).collect();
        let name = format!("Kast · {viewer}");
        // Un Mac sin monitor (p. ej. con el monitor puesto en otra entrada) sólo tiene una pantalla de reemplazo de macOS,
        // que desaparece en cuanto hay otra: antes se sustituye por una virtual (del tamaño del monitor del visor donde
        // se verá, si lo dice), para que siga teniendo su pantalla además de las nuevas.
        let (mut stand_ins, mut swap) = (Vec::new(), Vec::new());
        let sizes = platform::displays().unwrap_or_default();
        for (i, &id) in ids_before.iter().enumerate() {
            if !platform::vdisplay::placeholder(id) { continue; }
            let f = fixed.iter().position(|f| f.0 == id);
            let (px, scale) = f.and_then(|f| fixed[f].2).unwrap_or((sizes.get(i).map_or((1920, 1080), |d| (d.w, d.h)), 1.0));
            let v = match platform::vdisplay::create(&name, px, scale) {
                Ok(v) => v,
                Err(e) => { store::log(&format!("pantalla de {viewer} en lugar de la de reemplazo: {e}")); continue }
            };
            let t = Instant::now();
            while platform::display_ids().contains(&id) && t.elapsed() < Duration::from_secs(3) { std::thread::sleep(Duration::from_millis(50)); }
            // Si macOS no la quitó, no hace falta (y se suelta la nueva).
            if platform::display_ids().contains(&id) { continue; }
            if let Some(f) = f { fixed[f].0 = v.id; }
            swap.push((id, v.id));
            stand_ins.push(v);
        }
        let mut made: Vec<(usize, platform::vdisplay::Virtual)> = Vec::new();
        let mut last_err = None;
        for (k, w) in wanted.iter().enumerate() {
            let Some((_, px, scale)) = w else { continue };
            match platform::vdisplay::create(&name, *px, *scale) {
                Ok(v) => made.push((k, v)),
                Err(e) => { store::log(&format!("pantalla virtual {}×{} para {viewer}: {e}", px.0, px.1)); last_err = Some(e); }
            }
        }
        if made.is_empty() { return Err(last_err.unwrap_or_else(|| "faltan los datos de las pantallas virtuales".into())); }
        // Dónde van: junto a mis pantallas que el visor verá en sus monitores, sin tapar las demás.
        let (ids, bounds) = (platform::display_ids(), platform::display_bounds());
        let at = |id: u32| ids.iter().position(|x| *x == id).and_then(|i| bounds.get(i).copied());
        let virt_ids: Vec<u32> = made.iter().map(|(_, v)| v.id).collect();
        let pairs: Vec<(Rect, Rect)> = fixed.iter().filter_map(|(id, local, _)| Some((*local, at(*id)?))).collect();
        let others: Vec<Rect> = ids.iter().zip(&bounds).filter(|(id, _)| !virt_ids.contains(id) && !fixed.iter().any(|f| f.0 == **id))
            .map(|(_, b)| *b).collect();
        let virt_in: Vec<(Rect, (f64, f64))> = made.iter().map(|(k, v)| {
            let (local, px, scale) = wanted[*k].unwrap();
            // En un Mac, el escritorio va en puntos (Retina: la mitad de los píxeles); en Windows, en píxeles.
            let d = if cfg!(target_os = "macos") && scale >= 1.5 { 2.0 } else { 1.0 };
            let size = at(v.id).map(|b| (b.w, b.h)).unwrap_or((px.0 as f64 / d, px.1 as f64 / d));
            (local, size)
        }).collect();
        let origins = virt::place(&pairs, &virt_in, &others);
        let moves: Vec<(u32, i32, i32)> = made.iter().zip(&origins).map(|((_, v), &(x, y))| (v.id, x, y)).collect();
        platform::vdisplay::arrange(&moves)?;
        // Se aplica en un momento (el sistema puede ajustarlas un poco para que no queden huecos).
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(2) {
            let (ids, bounds) = (platform::display_ids(), platform::display_bounds());
            let placed = moves.iter().all(|(id, x, y)| ids.iter().position(|i| i == id).and_then(|i| bounds.get(i))
                .is_some_and(|b| b.x as i32 == *x && b.y as i32 == *y));
            if placed { break; }
            std::thread::sleep(Duration::from_millis(50));
        }
        let ids_after = platform::display_ids();
        let index = |id: u32| ids_after.iter().position(|x| *x == id).map(|i| i + 1);
        // (La de reemplazo pasa a ser la virtual que la sustituye.)
        let remap: Vec<(usize, usize)> = ids_before.iter().enumerate()
            .filter_map(|(i, id)| Some((i + 1, index(swap.iter().find(|s: &&(u32, u32)| s.0 == *id).map_or(*id, |s| s.1))?)))
            .collect();
        let mut idx = vec![None; wanted.len()];
        for (k, v) in &made { idx[*k] = index(v.id); }
        self.displays_changed();
        Ok((made.into_iter().map(|(_, v)| v).chain(stand_ins).collect(), remap, idx))
    }

    /// Mis pantallas cambiaron (se crearon o quitaron virtuales): se anuncian ya, sin esperar a la siguiente vuelta.
    fn displays_changed(&self) {
        let d: Vec<(u32, u32)> = platform::displays().map(|v| v.iter().map(|d| (d.w, d.h)).collect()).unwrap_or_default();
        *self.displays.lock().unwrap() = d;
        self.announce();
        self.emit_state();
    }

    /// Contraseña o código: reto ligado a esta conexión (access.rs). Devuelve si puede controlar.
    fn authenticate(&self, ch: &mut Secure, kind: &str, key: &str, name: &str, plat: &str, peer_addr: Option<SocketAddr>) -> Result<bool, String> {
        let what = if kind == access::PASSWORD { "contraseña" } else { "código de emparejamiento" };
        let (salt, pw_key) = {
            let mut st = self.st.lock().unwrap();
            if let Some(left) = st.limiter.blocked(key) {
                return Err(format!("Demasiados intentos fallidos: espera {} min", left.as_secs() / 60 + 1));
            }
            if kind == access::PASSWORD {
                let Some((salt, k)) = st.access.password_key() else { return Err(format!("{} no tiene activado el acceso desatendido", self.name)) };
                (salt, Some(k))
            } else {
                if st.pairing.as_ref().is_none_or(|p| p.expired()) {
                    st.pairing = None;
                    return Err(format!("{} no tiene un código de emparejamiento activo (o ya caducó)", self.name));
                }
                (st.pairing.as_ref().unwrap().salt().to_vec(), None)
            }
        };
        ch.send_typed(msg::CHALLENGE, json!({"kind": kind, "salt": net::hex(&salt)}).to_string().as_bytes()).map_err(|e| e.to_string())?;
        let mut m = Vec::new();
        ch.recv(&mut m).map_err(|_| format!("{name} no respondió al reto"))?;
        let proof = if m.first() == Some(&msg::AUTH) {
            serde_json::from_slice::<Value>(&m[1..]).ok().and_then(|v| v["proof"].as_str().and_then(net::unhex)).unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut st = self.st.lock().unwrap();
        let ok = match &pw_key {
            Some(k) => access::verify(k, &ch.binding, &proof),
            None => st.pairing.as_mut().is_some_and(|p| p.check(&ch.binding, &proof)),
        };
        if !ok {
            st.limiter.fail(key);
            if st.pairing.as_ref().is_some_and(|p| p.expired()) { st.pairing = None; }
            drop(st);
            store::log(&format!("{name}: {what} incorrecto/a"));
            self.toast(format!("Intento fallido de {what} desde {name}"), true);
            self.emit_state();
            return Err(if kind == access::PASSWORD { "Contraseña incorrecta".into() } else { "Código incorrecto o caducado".into() });
        }
        st.limiter.ok(key);
        let control = if kind == access::PASSWORD {
            st.access.password_control
        } else {
            // Un solo uso: el dispositivo queda emparejado (acceso permanente con el permiso del código).
            let code = st.pairing.take().unwrap();
            let ip = peer_addr.map(|a| a.ip().to_string()).unwrap_or_default();
            match st.saved.iter_mut().find(|s| s.key == key) {
                Some(s) => { s.trusted = true; s.control = code.control; s.name = name.to_string(); s.platform = plat.to_string(); }
                None => st.saved.push(Saved { key: key.to_string(), name: name.to_string(), platform: plat.to_string(), addr: ip,
                                              trusted: true, manual: false, control: code.control, last_seen: 0 }),
            }
            self.save(&st);
            code.control
        };
        drop(st);
        if kind == access::PAIR {
            store::log(&format!("{name} se emparejó con este equipo"));
            self.toast(format!("{name} quedó emparejado: ya puede entrar sin preguntar"), false);
        } else {
            store::log(&format!("{name} entró con la contraseña de acceso desatendido"));
        }
        self.emit_state();
        Ok(control)
    }

    /// Configuración de acceso desde la app.
    fn access_set(&self, v: &Value) {
        let mut st = self.st.lock().unwrap();
        let mut cut_password = false;
        if let Some(u) = v.get("unknown").and_then(Value::as_str) { st.access.reject_unknown = u == "reject"; }
        if let Some(c) = v.get("unattended_control").and_then(Value::as_bool) {
            cut_password |= st.access.password_control && !c;
            st.access.password_control = c;
        }
        if let Some(pw) = v.get("password").and_then(Value::as_str) {
            if let Err(e) = st.access.set_password(pw) {
                drop(st);
                return self.toast(e, true);
            }
            cut_password = true;
            store::log("contraseña de acceso desatendido establecida");
        }
        if v.get("unattended") == Some(&Value::Bool(false)) && st.access.unattended() {
            st.access.clear_password();
            cut_password = true;
            store::log("acceso desatendido desactivado");
        }
        st.access.save();
        // Al cambiar o quitar la contraseña (o su permiso) se cortan las sesiones que entraron con ella.
        if cut_password {
            st.grants.retain(|_, g| g.via != "password");
            for x in st.sessions.iter_mut().filter(|x| x.dir == "out" && x.via == "password") {
                x.stopped = true;
                if let Some(sock) = &x.sock { let _ = sock.shutdown(std::net::Shutdown::Both); }
            }
        }
        drop(st);
        self.emit_state();
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
            // Código de emparejamiento caducado.
            let expired = {
                let mut st = self.st.lock().unwrap();
                let e = st.pairing.as_ref().is_some_and(|p| p.expired());
                if e { st.pairing = None; }
                e
            };
            if changed || expired || now != perms { perms = now; self.emit_state(); }
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
            Some(d) => d.seen.is_none() || d.addr != Some(addr) || d.name != b.name || d.displays != b.displays || d.sharing != b.sharing || d.vd != b.vd,
            None => true,
        };
        st.devices.insert(b.id.clone(), Dev { name: b.name, platform: b.platform, addr: Some(addr), alts, seen: Some(Instant::now()), displays: b.displays,
                                              sharing: b.sharing, vd: b.vd });
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
            devices.insert(id_of(&k), Dev { name: s.name.clone(), platform: s.platform.clone(), addr: s.addr.parse().ok(), alts: vec![], seen: None, displays: vec![],
                                            sharing: true, vd: false });
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
        st: Mutex::new(State { devices, saved, access: access::Access::load(), ..Default::default() }),
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

/// Pantallas virtuales pedidas y dónde se verán, con mis monitores que mostrarán sus pantallas reales, para que el
/// otro equipo las coloque como están mis monitores (ver virt.rs). Una virtual en una ventana tiene el tamaño de mi
/// monitor principal y va a la derecha de lo que veo a pantalla completa (sin nada así, a la derecha de sus pantallas).
fn layout_plan(screen: &Screen, more: &[Screen]) -> Value { plan_with(screen, more, &platform::local_monitors()) }

fn plan_with(screen: &Screen, more: &[Screen], mons: &[LocalMon]) -> Value {
    let all: Vec<&Screen> = std::iter::once(screen).chain(more).collect();
    let fixed: Vec<Value> = all.iter().filter(|s| !s.virt)
        .filter_map(|s| s.local(mons).map(|(at, px, scale)| json!({"monitor": s.monitor, "at": at.json(), "px": [px.0, px.1], "scale": scale})))
        .collect();
    let mut used: Vec<Rect> = all.iter().filter_map(|s| s.local(mons)).map(|l| l.0).collect();
    let main = mons.iter().find(|m| m.rect.x == 0.0 && m.rect.y == 0.0).or(mons.first());
    let virt: Vec<Value> = more.iter().filter(|s| s.virt).map(|s| {
        let (at, px, scale) = s.local(mons).unwrap_or_else(|| {
            let (r, px, scale) = main.map_or((Rect::new(0.0, 0.0, 1920.0, 1080.0), (1920, 1080), 1.0), |m| (m.rect, m.px, m.scale));
            let at = used.iter().max_by(|a, b| (a.x + a.w).total_cmp(&(b.x + b.w))).map_or(r, |u| Rect::new(u.x + u.w, u.y, r.w, r.h));
            used.push(at);
            (at, s.px.unwrap_or(px), s.scale.unwrap_or(scale))
        });
        json!({"at": at.json(), "px": [px.0, px.1], "scale": scale, "hidpi": scale >= 1.5})
    }).collect();
    json!({"fixed": fixed, "virtual": virt})
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

/// Visor: archivos que se sacan arrastrando del otro equipo (ver msg::DRAG_FILES). Llegan a una carpeta temporal
/// donde ya existen desde el principio (vacíos), y la ventana arrastra con ellos hacia este equipo.
struct DragIn {
    a: Arc<Agent>,
    session: u64,
    peer: String,
    recv: Option<share::FileRecv>,
    done: Option<Arc<viewer::DragDone>>,
}

/// Si la sesión se corta a medias, el arrastre en curso se cancela (en vez de esperar archivos que ya no llegan).
impl Drop for DragIn {
    fn drop(&mut self) {
        if let Some(d) = self.done.take() { d.finish(false); }
    }
}

impl DragIn {
    fn on_message(&mut self, m: &[u8]) {
        if m[0] == msg::DRAG_FILES {
            let Ok(v) = serde_json::from_slice::<Value>(&m[1..]) else { return };
            let items: Vec<(String, bool)> = v.get("items").and_then(Value::as_array).into_iter().flatten()
                .filter_map(|it| Some((share::clean(it.get("name")?.as_str()?)?, it.get("dir").and_then(Value::as_bool).unwrap_or(false))))
                .collect();
            if items.is_empty() { return; }
            // Un arrastre anterior que no terminó de llegar ya no se va a usar.
            if let Some(d) = self.done.take() { d.finish(false); }
            let dir = match share::drag_dir() {
                Ok(d) => d,
                Err(e) => { self.a.toast(format!("No se pudieron recibir los archivos arrastrados: {e}"), true); return; }
            };
            let paths: Vec<std::path::PathBuf> = items.iter().map(|(name, is_dir)| {
                let p = dir.join(name);
                let _ = if *is_dir { std::fs::create_dir_all(&p) } else { std::fs::File::create(&p).map(|_| ()) };
                p
            }).collect();
            let done = Arc::new(viewer::DragDone::default());
            let count = v.get("count").and_then(Value::as_u64).unwrap_or(0);
            if count == 0 { done.finish(true); }
            self.recv = (count > 0).then(|| share::FileRecv::into_dir(dir));
            self.done = (count > 0).then(|| done.clone());
            self.a.emit(json!({"t": "link", "ev": "drag", "session": self.session, "paths": paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()}));
            let _ = self.a.proxy.send_event(viewer::Event::DragOut { session: self.session, paths, done });
            if count == 0 { self.a.emit(json!({"t": "link", "ev": "drag_done", "session": self.session, "ok": true})); }
            return;
        }
        let Some(r) = &mut self.recv else { return };
        let end = match r.on_message(m) {
            Ok(Some(got)) => {
                store::log(&format!("sacados de {} arrastrando: {} archivo{} ({})", self.peer, got.count, if got.count == 1 { "" } else { "s" }, share::human(got.bytes)));
                Some(true)
            }
            Ok(None) => None,
            Err(e) => {
                store::log(&format!("sacar archivos arrastrando de {}: {e}", self.peer));
                self.a.toast(format!("No se pudieron recibir los archivos arrastrados de {}: {e}", self.peer), true);
                Some(false)
            }
        };
        if let Some(ok) = end {
            if let Some(d) = self.done.take() { d.finish(ok); }
            self.recv = None;
            self.a.emit(json!({"t": "link", "ev": "drag_done", "session": self.session, "ok": ok}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: f64, y: f64) -> LocalMon { LocalMon { rect: Rect::new(x, y, 1920.0, 1080.0), px: (1920, 1080), scale: 1.0 } }
    fn virt(show_on: usize) -> Screen { Screen { virt: true, ..Screen::real(0, show_on) } }

    #[test]
    fn virtual_en_una_ventana() {
        // Un solo monitor: su pantalla 1 en él y una virtual en una ventana, de su tamaño y a su derecha.
        let p = plan_with(&Screen::real(1, 1), &[virt(0)], &[mon(0.0, 0.0)]);
        assert_eq!(p["virtual"][0]["at"], json!([1920.0, 0.0, 1920.0, 1080.0]));
        assert_eq!(p["virtual"][0]["px"], json!([1920, 1080]));
        // Una virtual a pantalla completa en el monitor de la derecha y otra en una ventana, a continuación.
        let p = plan_with(&Screen::real(1, 1), &[virt(2), virt(0)], &[mon(0.0, 0.0), mon(1920.0, 0.0)]);
        assert_eq!(p["virtual"][0]["at"], json!([1920.0, 0.0, 1920.0, 1080.0]));
        assert_eq!(p["virtual"][1]["at"], json!([3840.0, 0.0, 1920.0, 1080.0]));
        // Todo en ventanas: nada fijo (el otro equipo la pone a la derecha de sus pantallas), del tamaño del principal.
        let p = plan_with(&Screen::real(1, 0), &[virt(0)], &[mon(-1920.0, 0.0), mon(0.0, 0.0)]);
        assert!(p["fixed"].as_array().unwrap().is_empty());
        assert_eq!(p["virtual"][0]["at"], json!([0.0, 0.0, 1920.0, 1080.0]));
    }
}
