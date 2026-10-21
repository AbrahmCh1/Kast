//! Cliente de Kast Link para la app de iOS: descubre equipos, se conecta (con código y control) y entrega el
//! vídeo a Swift en AVCC. API en C (ver ios/Kast/kastlink.h); los eventos llegan a la app como JSON.
//!
//! Eventos (callback `ev`, desde hilos en segundo plano):
//!   {"ev":"devices","devices":[{"id","name","platform","addr","online","sharing","known","displays":[[w,h]…]}]}
//!   {"ev":"session","session":n,"state":"connecting|waiting|live|ended","peer","code","control","w","h","error","note"}
//!   {"ev":"clip","session":n,"text":"…"} o {"ev":"clip","session":n,"png":"base64"}
//!   {"ev":"toast","msg":"…","err":bool}
use crate::discovery::{self, Packet};
use crate::input::{Event, MAX_TEXT};
use crate::net::{Identity, Secure, msg};
use crate::store::{self, Saved};
use crate::{Res, h264};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub type EventCb = extern "C" fn(ctx: *mut c_void, json: *const c_char);
/// Fotograma H.264 en AVCC (NAL con longitud de 4 bytes). `sps`/`pps` sólo cuando llegan (fotogramas clave).
pub type FrameCb = extern "C" fn(ctx: *mut c_void, session: u64, avcc: *const u8, len: usize,
                                 sps: *const u8, sps_len: usize, pps: *const u8, pps_len: usize);

const ONLINE: Duration = Duration::from_secs(10);
const CHUNK: usize = 60_000;

struct Dev {
    name: String,
    platform: String,
    addr: SocketAddr,
    seen: Instant,
    displays: Vec<(u32, u32)>,
    sharing: bool,
}

struct Sess {
    sock: Option<TcpStream>,
    /// Con control: mensajes hacia el otro equipo (teclado, ratón, portapapeles).
    tx: Option<Sender<Vec<u8>>>,
}

struct Ctx(*mut c_void);
unsafe impl Send for Ctx {}
unsafe impl Sync for Ctx {}

struct Client {
    me: Identity,
    name: String,
    devices: Mutex<HashMap<String, Dev>>,
    saved: Mutex<Vec<Saved>>,
    sessions: Mutex<HashMap<u64, Sess>>,
    next: AtomicU64,
    udp: UdpSocket,
    ctx: Ctx,
    ev: EventCb,
    frame: FrameCb,
}

static CLIENT: OnceLock<Arc<Client>> = OnceLock::new();

impl Client {
    fn emit(&self, v: Value) {
        if let Ok(s) = std::ffi::CString::new(v.to_string()) { (self.ev)(self.ctx.0, s.as_ptr()); }
    }

    fn toast(&self, m: impl Into<String>, err: bool) { self.emit(json!({"ev": "toast", "msg": m.into(), "err": err})); }

    fn session_ev(&self, session: u64, state: &str, extra: Value) {
        let mut v = json!({"ev": "session", "session": session, "state": state});
        if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) { o.extend(e); }
        self.emit(v);
    }

    fn emit_devices(&self) {
        let devs = self.devices.lock().unwrap();
        let saved = self.saved.lock().unwrap();
        let known = |id: &str| saved.iter().any(|s| !s.key.is_empty() && crate::net::id_of(&crate::net::unhex(&s.key).unwrap_or_default()) == id);
        let mut list: Vec<Value> = devs.iter().map(|(id, d)| json!({
            "id": id, "name": d.name, "platform": d.platform, "addr": d.addr.to_string(),
            "online": d.seen.elapsed() < ONLINE, "sharing": d.sharing, "known": known(id),
            "displays": d.displays.iter().map(|(w, h)| json!([w, h])).collect::<Vec<_>>(),
        })).collect();
        // Agregados por IP que aún no respondieron.
        for s in saved.iter().filter(|s| s.manual && !devs.values().any(|d| d.addr.to_string() == s.addr || s.addr.starts_with(&format!("{}:", d.addr.ip())))) {
            list.push(json!({"id": format!("ip:{}", s.addr), "name": s.addr, "platform": "", "addr": s.addr, "online": false, "sharing": true, "known": false, "displays": []}));
        }
        list.sort_by_key(|v| (!v["online"].as_bool().unwrap_or(false), v["name"].as_str().unwrap_or("").to_lowercase()));
        drop((devs, saved));
        self.emit(json!({"ev": "devices", "devices": list}));
    }

    // ------------------------------------------------------------ descubrimiento

    /// Consultas directas a cada equipo de las redes locales (iOS no deja recibir difusión sin un permiso
    /// especial de Apple; los equipos de escritorio responden a la consulta con su anuncio).
    fn sweep(&self) {
        let q = discovery::query();
        let mut targets: Vec<Ipv4Addr> = Vec::new();
        if let Ok(ifs) = if_addrs::get_if_addrs() {
            for i in ifs {
                let if_addrs::IfAddr::V4(a) = i.addr else { continue };
                if a.ip.is_loopback() || !a.ip.is_private() { continue; }
                let mask = u32::from(a.netmask);
                let prefix = mask.leading_ones();
                // Redes grandes: sólo la /22 alrededor de este equipo (1022 consultas como mucho).
                let mask = if prefix < 22 { u32::MAX << 10 } else { mask };
                let net = u32::from(a.ip) & mask;
                let hosts = !mask;
                for h in 1..hosts {
                    let ip = Ipv4Addr::from(net | h);
                    if ip != a.ip && !targets.contains(&ip) { targets.push(ip); }
                }
                if let Some(b) = a.broadcast { let _ = self.udp.send_to(&q, (b, discovery::PORT)); }
            }
        }
        for ip in targets { let _ = self.udp.send_to(&q, (ip, discovery::PORT)); }
        // Los agregados a mano, por si están en otra red.
        for s in self.saved.lock().unwrap().iter().filter(|s| !s.addr.is_empty()) {
            if let Ok(a) = s.addr.parse::<SocketAddr>() { let _ = self.udp.send_to(&q, (a.ip(), discovery::PORT)); }
        }
    }

    fn receive_loop(&self) {
        let mut buf = [0u8; 2048];
        loop {
            let Ok((n, src)) = self.udp.recv_from(&mut buf) else { continue };
            let Some(Packet::Beacon(b)) = discovery::decode(&buf[..n]) else { continue };
            let addr = SocketAddr::new(src.ip(), b.port);
            let mut devs = self.devices.lock().unwrap();
            let fresh = devs.get(&b.id).map(|d| d.seen.elapsed() > ONLINE || d.addr != addr || d.name != b.name || d.displays != b.displays || d.sharing != b.sharing).unwrap_or(true);
            devs.insert(b.id, Dev { name: b.name, platform: b.platform, addr, seen: Instant::now(), displays: b.displays, sharing: b.sharing });
            drop(devs);
            if fresh { self.emit_devices(); }
        }
    }

    fn discovery_loop(&self) {
        let mut tick = 0u32;
        loop {
            self.sweep();
            // Cada tanto se avisa de los que dejaron de responder.
            if tick % 3 == 2 { self.emit_devices(); }
            tick += 1;
            std::thread::sleep(Duration::from_secs(4));
        }
    }

    fn add(&self, raw: &str) {
        let raw = raw.trim();
        let with_port = if raw.contains(':') { raw.to_string() } else { format!("{raw}:{}", 47800) };
        let Some(addr) = with_port.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
            return self.toast(format!("Dirección inválida: {raw}"), true);
        };
        let mut saved = self.saved.lock().unwrap();
        if !saved.iter().any(|s| s.addr == addr.to_string()) {
            saved.push(Saved { addr: addr.to_string(), manual: true, ..Default::default() });
            store::save_devices(&saved);
        }
        drop(saved);
        let _ = self.udp.send_to(&discovery::query(), (addr.ip(), discovery::PORT));
        self.emit_devices();
    }

    fn forget(&self, id: &str) {
        let mut saved = self.saved.lock().unwrap();
        let ip = id.strip_prefix("ip:");
        saved.retain(|s| match ip {
            Some(a) => s.addr != a,
            None => s.key.is_empty() || crate::net::id_of(&crate::net::unhex(&s.key).unwrap_or_default()) != id,
        });
        store::save_devices(&saved);
        drop(saved);
        self.devices.lock().unwrap().remove(id);
        self.emit_devices();
    }

    // ------------------------------------------------------------ sesión

    fn connect(self: &Arc<Self>, id: &str, monitor: usize, control: bool) -> u64 {
        let session = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (addr, name) = match id.strip_prefix("ip:") {
            Some(a) => (a.parse::<SocketAddr>().ok(), a.to_string()),
            None => match self.devices.lock().unwrap().get(id) { Some(d) => (Some(d.addr), d.name.clone()), None => (None, id.to_string()) },
        };
        self.sessions.lock().unwrap().insert(session, Sess { sock: None, tx: None });
        self.session_ev(session, "connecting", json!({"peer": name}));
        let (a, id) = (self.clone(), id.to_string());
        std::thread::spawn(move || {
            let r = match addr {
                Some(addr) => a.view(session, &id, addr, &name, monitor, control),
                None => Err(format!("{name} no está disponible").into()),
            };
            let stopped = a.sessions.lock().unwrap().remove(&session).is_none();
            let error = match r { Err(e) if !stopped => e.to_string(), _ => String::new() };
            a.session_ev(session, "ended", json!({"peer": name, "error": error}));
        });
        session
    }

    fn view(&self, session: u64, id: &str, addr: SocketAddr, name: &str, monitor: usize, control: bool) -> Res<()> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|_| format!("{name} no responde"))?;
        let mut ch = Secure::connect(stream, &self.me)?;
        store::check_peer(&self.saved.lock().unwrap(), id, &ch.peer)?;
        {
            let mut s = self.sessions.lock().unwrap();
            let Some(sess) = s.get_mut(&session) else { return Ok(()) }; // cancelada mientras conectaba
            sess.sock = Some(ch.shutdown_handle()?);
        }
        self.session_ev(session, "waiting", json!({"peer": name, "code": ch.code}));
        let req = json!({"name": self.name, "platform": "ios", "monitor": monitor, "control": control}).to_string();
        ch.send_typed(msg::REQUEST, req.as_bytes())?;
        let mut m = Vec::new();
        ch.recv(&mut m).map_err(|_| "conexión cerrada")?;
        match m.first() {
            Some(&msg::ACCEPT) => {}
            Some(&msg::REJECT) => return Err(String::from_utf8_lossy(&m[1..]).to_string().into()),
            _ => return Err("respuesta inesperada".into()),
        }
        let granted: Value = serde_json::from_slice(&m[1..]).unwrap_or(Value::Null);
        let got_control = control && granted.get("control").and_then(Value::as_bool).unwrap_or(false);
        let note = granted.get("note").and_then(Value::as_str).unwrap_or("").to_string();
        // Recordar el equipo (su identidad, para detectar cambios).
        {
            let mut saved = self.saved.lock().unwrap();
            let key = crate::net::hex(&ch.peer);
            let platform = self.devices.lock().unwrap().get(id).map(|d| d.platform.clone()).unwrap_or_default();
            match saved.iter_mut().find(|s| s.key == key || (s.key.is_empty() && s.addr == addr.to_string())) {
                Some(s) => { s.key = key; s.addr = addr.to_string(); s.name = name.to_string(); s.platform = platform; }
                None => saved.push(Saved { key, name: name.to_string(), platform, addr: addr.to_string(), ..Default::default() }),
            }
            store::save_devices(&saved);
        }
        ch.recv(&mut m).map_err(|_| format!("{name} cerró la conexión al empezar a transmitir"))?;
        if m.first() != Some(&msg::HELLO) {
            return Err(if m.first() == Some(&msg::REJECT) { String::from_utf8_lossy(&m[1..]).to_string() } else { "respuesta inesperada".into() }.into());
        }
        let b = &m[1..];
        let rd = |i: usize| b.get(i..i + 4).map(|x| u32::from_be_bytes(x.try_into().unwrap())).unwrap_or(0);
        let (w, h) = (rd(0), rd(4));
        if control_tx(self, session, &ch, got_control).is_err() { return Ok(()); }
        self.session_ev(session, "live", json!({"peer": name, "control": got_control, "w": w, "h": h, "note": note}));
        self.receive(session, &mut ch)
    }

    /// Vídeo y portapapeles del otro equipo hasta que se corte la conexión.
    fn receive(&self, session: u64, ch: &mut Secure) -> Res<()> {
        let (mut m, mut data) = (Vec::new(), Vec::new());
        let mut clip: Option<(u8, usize, Vec<u8>)> = None;
        loop {
            ch.recv(&mut m)?;
            match m.first() {
                Some(&msg::FRAME) if m.len() >= 22 => {
                    let len = u32::from_be_bytes(m[18..22].try_into()?) as usize;
                    if len > 64 << 20 { return Err("fotograma demasiado grande".into()); }
                    data.clear();
                    while data.len() < len {
                        ch.recv(&mut m)?;
                        match m.first() {
                            Some(&msg::DATA) => data.extend_from_slice(&m[1..]),
                            Some(&(msg::CLIP | msg::CLIP_DATA)) => self.on_clip(session, &m, &mut clip),
                            _ => return Err("flujo de vídeo corrupto".into()),
                        }
                    }
                    self.deliver(session, &data);
                }
                Some(&(msg::CLIP | msg::CLIP_DATA)) => self.on_clip(session, &m, &mut clip),
                _ => {}
            }
        }
    }

    /// Annex B -> AVCC + SPS/PPS para AVSampleBufferDisplayLayer (con el SPS ajustado a latencia mínima).
    fn deliver(&self, session: u64, annexb: &[u8]) {
        let fixed = h264::low_latency_annexb(annexb);
        let data = fixed.as_deref().unwrap_or(annexb);
        let nals = h264::nal_units(data);
        let sps = nals.iter().find(|n| h264::nal_type(n) == h264::NAL_SPS).copied().unwrap_or(&[]);
        let pps = nals.iter().find(|n| h264::nal_type(n) == h264::NAL_PPS).copied().unwrap_or(&[]);
        // Sin SPS/PPS ni delimitadores de acceso (9): sólo los cortes del fotograma.
        let avcc = h264::to_avcc(nals.iter().copied().filter(|n| !matches!(h264::nal_type(n), h264::NAL_SPS | h264::NAL_PPS | 9)));
        if avcc.is_empty() { return; }
        (self.frame)(self.ctx.0, session, avcc.as_ptr(), avcc.len(), sps.as_ptr(), sps.len(), pps.as_ptr(), pps.len());
    }

    fn on_clip(&self, session: u64, m: &[u8], rx: &mut Option<(u8, usize, Vec<u8>)>) {
        match m[0] {
            msg::CLIP if m.len() >= 6 => {
                let total = u32::from_be_bytes(m[2..6].try_into().unwrap()) as usize;
                *rx = (total <= 32 << 20).then(|| (m[1], total, Vec::with_capacity(total)));
            }
            msg::CLIP_DATA => { if let Some((_, _, d)) = rx { d.extend_from_slice(&m[1..]); } }
            _ => return,
        }
        if !matches!(rx, Some((_, total, d)) if d.len() >= *total) { return; }
        let (kind, _, d) = rx.take().unwrap();
        match kind {
            1 => self.emit(json!({"ev": "clip", "session": session, "text": String::from_utf8_lossy(&d)})),
            2 => self.emit(json!({"ev": "clip", "session": session, "png": base64(&d)})),
            _ => {}
        }
    }

    fn send(&self, session: u64, m: Vec<u8>) {
        if let Some(tx) = self.sessions.lock().unwrap().get(&session).and_then(|s| s.tx.as_ref()) { let _ = tx.send(m); }
    }

    fn send_clip(&self, session: u64, kind: u8, data: &[u8]) {
        if data.is_empty() || data.len() > 32 << 20 { return; }
        let mut head = vec![msg::CLIP, kind];
        head.extend_from_slice(&(data.len() as u32).to_be_bytes());
        self.send(session, head);
        for part in data.chunks(CHUNK) {
            let mut m = vec![msg::CLIP_DATA];
            m.extend_from_slice(part);
            self.send(session, m);
        }
    }

    fn disconnect(&self, session: u64) {
        if let Some(s) = self.sessions.lock().unwrap().remove(&session) {
            if let Some(sock) = s.sock { let _ = sock.shutdown(std::net::Shutdown::Both); }
        }
    }
}

/// Un hilo envía al otro equipo lo que la app mande: petición de fotograma clave y, con control, teclado,
/// ratón y portapapeles (sin control el otro equipo sólo atiende la petición de fotograma clave).
fn control_tx(c: &Client, session: u64, ch: &Secure, _control: bool) -> Result<(), ()> {
    let mut s = c.sessions.lock().unwrap();
    let sess = s.get_mut(&session).ok_or(())?;
    let (tx, rx) = channel::<Vec<u8>>();
    let mut out = ch.try_clone().map_err(|_| ())?;
    std::thread::spawn(move || { for m in rx { if out.send(&m).is_err() { break; } } });
    sess.tx = Some(tx);
    Ok(())
}

fn base64(d: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

// ---------------------------------------------------------------- API en C

fn client() -> Option<&'static Arc<Client>> { CLIENT.get() }

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Arranca el cliente: `dir` para la identidad y los equipos recordados, `name` el nombre del dispositivo.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kl_start(dir: *const c_char, name: *const c_char, ctx: *mut c_void, ev: EventCb, frame: FrameCb) -> bool {
    if CLIENT.get().is_some() { return true; }
    let (dir, name) = unsafe { (cstr(dir), cstr(name)) };
    store::set_config(dir.into(), name.clone());
    let Ok(udp) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return false };
    let _ = udp.set_broadcast(true);
    let c = Arc::new(Client {
        me: store::identity(), name, devices: Mutex::new(HashMap::new()), saved: Mutex::new(store::load_devices()),
        sessions: Mutex::new(HashMap::new()), next: AtomicU64::new(0), udp, ctx: Ctx(ctx), ev, frame,
    });
    let _ = CLIENT.set(c.clone());
    { let c = c.clone(); std::thread::spawn(move || c.receive_loop()); }
    { let c = c.clone(); std::thread::spawn(move || c.discovery_loop()); }
    c.emit_devices();
    true
}

/// Comandos en JSON: {"cmd":"refresh"}, {"cmd":"add","addr"}, {"cmd":"forget","id"}, {"cmd":"disconnect","session"}.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kl_command(json: *const c_char) {
    let Some(c) = client() else { return };
    let Ok(v) = serde_json::from_str::<Value>(&unsafe { cstr(json) }) else { return };
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match s("cmd").as_str() {
        "refresh" => { c.sweep(); c.emit_devices(); }
        "add" => c.add(&s("addr")),
        "forget" => c.forget(&s("id")),
        "disconnect" => c.disconnect(v.get("session").and_then(Value::as_u64).unwrap_or(0)),
        _ => {}
    }
}

/// Conecta con un equipo (`id` del anuncio o "ip:addr:puerto"); devuelve el número de sesión.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kl_connect(id: *const c_char, monitor: u32, control: bool) -> u64 {
    let Some(c) = client() else { return 0 };
    c.connect(&unsafe { cstr(id) }, monitor.max(1) as usize, control)
}

/// Pide al otro equipo un fotograma clave (al abrir la vista o si el decodificador falló).
#[unsafe(no_mangle)]
pub extern "C" fn kl_keyframe(session: u64) {
    if let Some(c) = client() { c.send(session, vec![msg::KEYFRAME]); }
}

#[unsafe(no_mangle)]
pub extern "C" fn kl_mouse_move(session: u64, x: i32, y: i32) {
    if let Some(c) = client() { c.send(session, Event::Move { x, y }.encode()); }
}

/// Botón: 0 izquierdo, 1 derecho, 2 central.
#[unsafe(no_mangle)]
pub extern "C" fn kl_mouse_button(session: u64, button: u8, down: bool, x: i32, y: i32) {
    if let Some(c) = client() { c.send(session, Event::Button { button, down, x, y }.encode()); }
}

/// Rueda en 1/120 de muesca (120 = una línea).
#[unsafe(no_mangle)]
pub extern "C" fn kl_wheel(session: u64, dx: i32, dy: i32) {
    if let Some(c) = client() { c.send(session, Event::Wheel { dx, dy }.encode()); }
}

/// Tecla física por su nombre W3C (`KeyA`, `Escape`, `ControlLeft`…).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kl_key(session: u64, code: *const c_char, down: bool) {
    if let Some(c) = client() { c.send(session, Event::Key { down, code: unsafe { cstr(code) } }.encode()); }
}

/// Texto escrito con el teclado de iOS.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kl_text(session: u64, text: *const c_char) {
    let Some(c) = client() else { return };
    let text = unsafe { cstr(text) };
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let mut end = rest.len().min(MAX_TEXT);
        while !rest.is_char_boundary(end) { end -= 1; }
        c.send(session, Event::Text { text: rest[..end].to_string() }.encode());
        rest = &rest[end..];
    }
}

/// Envía el portapapeles de iOS al otro equipo (texto UTF-8 o PNG).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kl_clip_text(session: u64, text: *const c_char) {
    if let Some(c) = client() { c.send_clip(session, 1, unsafe { cstr(text) }.as_bytes()); }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kl_clip_png(session: u64, data: *const u8, len: usize) {
    if data.is_null() { return; }
    if let Some(c) = client() { c.send_clip(session, 2, unsafe { std::slice::from_raw_parts(data, len) }); }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_rfc4648() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(super::base64(i.as_bytes()), o);
        }
    }
}
