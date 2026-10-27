//! Anfitrión de Android: comparte la pantalla de una pantalla interactiva o tableta con quien se conecte, como el
//! agente de escritorio (agent.rs) pero sin ventanas. La captura (MediaProjection) y el codificador H.264
//! (MediaCodec) están en Kotlin y entregan aquí cada fotograma (`Host::frame`); el teclado y el ratón del visor
//! salen hacia el servicio de Accesibilidad de la app como eventos.
//!
//! Eventos hacia la app (además de los del cliente, ver mobile.rs):
//!   {"ev":"host",…}                         estado: ID, compartir, permisos, acceso, emparejamiento, dispositivos, sesiones
//!   {"ev":"request","req","name","platform","code","control"}   alguien pide ver la pantalla: aceptar o rechazar
//!   {"ev":"capture","on":bool}              hay (o ya no hay) alguien viendo: encender o apagar el codificador
//!   {"ev":"keyframe"}                       pedir un fotograma clave al codificador
//!   {"ev":"input","t":"move"|"button"|"wheel"|"key"|"text",…}  teclado y ratón (en píxeles de la imagen enviada;
//!                                           los movimientos sólo con un botón pulsado)
//!   {"ev":"clip_in","text"}                 portapapeles que llega del visor
//!   {"ev":"files","count","bytes","first"}  archivos recibidos en la carpeta de descargas
//! Comandos de la app (ver `Host::command`):
//!   {"cmd":"host_state"} · {"cmd":"share","enabled"} · {"cmd":"video","w","h","fps"} · {"cmd":"perms","capture","input"}
//!   {"cmd":"answer","req","accept","remember","control"} · {"cmd":"access_set","unknown","unattended","password",
//!   "unattended_control"} · {"cmd":"pair_start","control"} · {"cmd":"pair_cancel"} · {"cmd":"revoke","key"}
//!   {"cmd":"end","session"} · {"cmd":"downloads","dir"}
use crate::access::{self, Access, Limiter, PairCode};
use crate::discovery::{self, Beacon, Packet};
use crate::input::{self, Event};
use crate::net::{self, Identity, Secure, msg};
use crate::store::{self, Saved};
use crate::virt::Rect;
use crate::{Res, h264, mobile, now_us, share};
use serde_json::{Value, json};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PORT: u16 = 47800;
/// MediaCodec.BUFFER_FLAG_KEY_FRAME y BUFFER_FLAG_CODEC_CONFIG.
const KEY_FRAME: i32 = 1;
const CODEC_CONFIG: i32 = 2;
/// Fotogramas en cola por visor: si se llena (red lenta) se descarta hasta el siguiente fotograma clave.
const QUEUE: usize = 12;

type Frame = Arc<(Vec<u8>, u64)>;

struct Req {
    req: u64,
    name: String,
    platform: String,
    code: String,
    control: bool,
    /// (aceptar, recordar, permitir el control)
    answer: Sender<(bool, bool, bool)>,
}

struct Sess {
    session: u64,
    peer: String,
    platform: String,
    key: String,
    control: bool,
    /// Cómo entró: "trusted", "approved", "password" o "pair".
    via: &'static str,
    sock: Option<TcpStream>,
}

struct State {
    /// La app comparte la pantalla (el usuario lo activó y hay captura).
    sharing: bool,
    /// Servicio de Accesibilidad activo: se puede controlar.
    input: bool,
    access: Access,
    pairing: Option<PairCode>,
    limiter: Limiter,
    requests: Vec<Req>,
    sessions: Vec<Sess>,
    downloads: PathBuf,
}

struct Sub {
    session: u64,
    tx: SyncSender<Frame>,
    /// Espera un fotograma clave (al empezar o tras descartar fotogramas).
    need_key: bool,
}

struct Video {
    w: u32,
    h: u32,
    fps: u32,
    /// SPS y PPS del codificador (MediaCodec los entrega aparte, una vez).
    config: Vec<u8>,
    subs: Vec<Sub>,
    asked: Option<Instant>,
}

pub struct Host {
    me: Identity,
    id: String,
    name: String,
    port: u16,
    udp: UdpSocket,
    st: Mutex<State>,
    video: Mutex<Video>,
    next: AtomicU64,
    emit: fn(&Value),
}

impl Host {
    /// Escucha conexiones y se anuncia en la red. `emit` entrega los eventos a la app.
    pub fn start(name: String, emit: fn(&Value)) -> Res<Arc<Self>> {
        let listener = TcpListener::bind(("0.0.0.0", PORT)).or_else(|_| TcpListener::bind(("0.0.0.0", 0)))?;
        let port = listener.local_addr()?.port();
        let me = store::identity();
        let h = Arc::new(Self {
            id: me.id(), me, name, port, udp: discovery::open()?,
            st: Mutex::new(State { sharing: false, input: false, access: Access::load(), pairing: None, limiter: Limiter::default(),
                                   requests: Vec::new(), sessions: Vec::new(), downloads: store::config_dir().join("Descargas") }),
            video: Mutex::new(Video { w: 1920, h: 1080, fps: 30, config: Vec::new(), subs: Vec::new(), asked: None }),
            next: AtomicU64::new(0), emit,
        });
        store::log(&format!("anfitrión de Android en el puerto {port}"));
        { let h = h.clone(); std::thread::spawn(move || h.listen(listener)); }
        { let h = h.clone(); std::thread::spawn(move || h.receive_loop()); }
        { let h = h.clone(); std::thread::spawn(move || h.announce_loop()); }
        Ok(h)
    }

    fn toast(&self, m: impl Into<String>, err: bool) { (self.emit)(&json!({"ev": "toast", "msg": m.into(), "err": err})); }

    fn saved(&self) -> std::sync::MutexGuard<'static, Vec<Saved>> {
        static EMPTY: Mutex<Vec<Saved>> = Mutex::new(Vec::new());
        mobile::saved().unwrap_or(&EMPTY).lock().unwrap()
    }

    fn emit_state(&self) {
        let trusted: Vec<Value> = self.saved().iter().filter(|s| s.trusted && !s.key.is_empty()).map(|s| json!({
            "key": s.key, "name": s.name, "platform": s.platform, "control": s.control, "last_seen": s.last_seen,
            "short_id": net::short_id(&net::id_of(&net::unhex(&s.key).unwrap_or_default())),
        })).collect();
        let st = self.st.lock().unwrap();
        let (w, h) = { let v = self.video.lock().unwrap(); (v.w, v.h) };
        let v = json!({
            "ev": "host",
            "me": {"id": self.id, "short_id": net::short_id(&self.id), "name": self.name, "port": self.port, "w": w, "h": h},
            "sharing": st.sharing, "input": st.input, "access": st.access.to_json(),
            "pairing": st.pairing.as_ref().filter(|p| !p.expired()).map(|p| json!({"code": p.code, "secs": p.secs_left(), "control": p.control})),
            "trusted": trusted,
            "sessions": st.sessions.iter().map(|s| json!({"session": s.session, "peer": s.peer, "platform": s.platform, "control": s.control, "via": s.via})).collect::<Vec<_>>(),
            "requests": st.requests.iter().map(|r| json!({"req": r.req, "name": r.name, "platform": r.platform, "code": r.code, "control": r.control})).collect::<Vec<_>>(),
        });
        drop(st);
        (self.emit)(&v);
    }

    // ------------------------------------------------------------ comandos de la app

    /// Devuelve false si no es un comando del anfitrión (lo atiende el cliente).
    pub fn command(&self, v: &Value) -> bool {
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        match v.get("cmd").and_then(Value::as_str).unwrap_or("") {
            "host_state" => {}
            "share" => {
                let on = b("enabled");
                let mut st = self.st.lock().unwrap();
                if st.sharing != on { store::log(if on { "compartiendo la pantalla" } else { "se dejó de compartir la pantalla" }); }
                st.sharing = on;
                if !on { cut(&st.sessions, |_| true); }
                drop(st);
                self.announce();
            }
            "video" => {
                let mut vd = self.video.lock().unwrap();
                vd.w = (n("w") as u32).clamp(16, 8192);
                vd.h = (n("h") as u32).clamp(16, 8192);
                vd.fps = (n("fps") as u32).clamp(1, 120);
                vd.config.clear();
            }
            "perms" => {
                let mut st = self.st.lock().unwrap();
                st.input = b("input");
                // Sin control ya no se puede atender a quien controlaba: se corta (vuelve a entrar sólo para ver).
                if !st.input { cut(&st.sessions, |s| s.control); }
            }
            "answer" => {
                let st = self.st.lock().unwrap();
                if let Some(r) = st.requests.iter().find(|r| r.req == n("req")) { let _ = r.answer.send((b("accept"), b("remember"), b("control"))); }
                return true;
            }
            "access_set" => self.access_set(v),
            "pair_start" => {
                let code = PairCode::new(b("control"));
                store::log("código de emparejamiento creado");
                self.st.lock().unwrap().pairing = Some(code);
            }
            "pair_cancel" => self.st.lock().unwrap().pairing = None,
            "revoke" => {
                let key = v.get("key").and_then(Value::as_str).unwrap_or("").to_string();
                {
                    let mut saved = self.saved();
                    for s in saved.iter_mut().filter(|s| s.key == key) { s.trusted = false; s.control = false; }
                    store::save_devices(&saved);
                }
                cut(&self.st.lock().unwrap().sessions, |s| s.key == key);
            }
            "end" => cut(&self.st.lock().unwrap().sessions, |s| s.session == n("session")),
            "downloads" => {
                if let Some(d) = v.get("dir").and_then(Value::as_str).filter(|d| !d.is_empty()) { self.st.lock().unwrap().downloads = d.into(); }
                return true;
            }
            _ => return false,
        }
        self.emit_state();
        true
    }

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
        if cut_password { cut(&st.sessions, |s| s.via == "password"); }
    }

    // ------------------------------------------------------------ vídeo

    /// Fotograma del codificador (MediaCodec): `flags` de MediaCodec.BufferInfo.
    pub fn frame(&self, data: &[u8], flags: i32) {
        let mut v = self.video.lock().unwrap();
        if flags & CODEC_CONFIG != 0 {
            v.config = data.to_vec();
            return;
        }
        let key = flags & KEY_FRAME != 0;
        // Cada fotograma clave lleva SPS y PPS: así cualquier visor puede empezar (o recuperarse) en él.
        let has_sps = key && h264::nal_units(data).iter().any(|n| h264::nal_type(n) == h264::NAL_SPS);
        let mut buf = Vec::with_capacity(data.len() + v.config.len());
        if key && !has_sps { buf.extend_from_slice(&v.config); }
        buf.extend_from_slice(data);
        let f: Frame = Arc::new((buf, now_us()));
        let mut want_key = false;
        v.subs.retain_mut(|s| {
            if s.need_key && !key { return true; }
            match s.tx.try_send(f.clone()) {
                Ok(()) => { s.need_key = false; true }
                Err(TrySendError::Full(_)) => { s.need_key = true; want_key = true; true }
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
        drop(v);
        if want_key { self.keyframe(); }
    }

    /// Pide un fotograma clave al codificador (como mucho cada 300 ms).
    fn keyframe(&self) {
        let mut v = self.video.lock().unwrap();
        if v.asked.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) { return; }
        v.asked = Some(Instant::now());
        drop(v);
        (self.emit)(&json!({"ev": "keyframe"}));
    }

    fn subscribe(&self, session: u64) -> Receiver<Frame> {
        let (tx, rx) = sync_channel(QUEUE);
        let mut v = self.video.lock().unwrap();
        let first = v.subs.is_empty();
        v.subs.push(Sub { session, tx, need_key: true });
        v.asked = None;
        drop(v);
        if first { (self.emit)(&json!({"ev": "capture", "on": true})); }
        self.keyframe();
        rx
    }

    fn unsubscribe(&self, session: u64) {
        let mut v = self.video.lock().unwrap();
        v.subs.retain(|s| s.session != session);
        let last = v.subs.is_empty();
        drop(v);
        if last { (self.emit)(&json!({"ev": "capture", "on": false})); }
    }

    // ------------------------------------------------------------ red

    fn beacon(&self) -> Beacon {
        let (w, h) = { let v = self.video.lock().unwrap(); (v.w, v.h) };
        Beacon { id: self.id.clone(), name: self.name.clone(), platform: "android".into(), port: self.port, displays: vec![(w, h)],
                 sharing: self.st.lock().unwrap().sharing, vd: false }
    }

    fn announce(&self) {
        let b = discovery::encode(&self.beacon());
        for t in discovery::broadcast_targets() { let _ = self.udp.send_to(&b, t); }
    }

    fn announce_loop(&self) {
        loop {
            self.announce();
            // Código de emparejamiento caducado.
            let expired = {
                let mut st = self.st.lock().unwrap();
                let e = st.pairing.as_ref().is_some_and(|p| p.expired());
                if e { st.pairing = None; }
                e
            };
            if expired { self.emit_state(); }
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    fn receive_loop(&self) {
        let mut buf = [0u8; 2048];
        loop {
            let Ok((n, src)) = self.udp.recv_from(&mut buf) else { continue };
            if let Some(Packet::Query) = discovery::decode(&buf[..n]) {
                let _ = self.udp.send_to(&discovery::encode(&self.beacon()), src);
            }
        }
    }

    fn listen(self: &Arc<Self>, listener: TcpListener) {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let h = self.clone();
            std::thread::spawn(move || {
                let peer = stream.peer_addr().ok();
                if let Err(e) = h.host(stream, peer) { store::log(&format!("sesión entrante: {e}")); }
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
        let monitor = v.get("monitor").and_then(Value::as_u64).unwrap_or(1).max(1);
        let wants_control = v.get("control").and_then(Value::as_bool).unwrap_or(false);
        let auth = v.get("auth").and_then(Value::as_str).unwrap_or("").to_string();
        if !self.st.lock().unwrap().sharing {
            let why = format!("{} no está compartiendo su pantalla: en ella, abre Kast y pulsa «Compartir esta pantalla»", self.name);
            ch.send_typed(msg::REJECT, why.as_bytes())?;
            return Ok(());
        }
        if monitor > 1 {
            ch.send_typed(msg::REJECT, format!("{} no tiene la pantalla {monitor}", self.name).as_bytes())?;
            return Ok(());
        }
        let key = net::hex(&ch.peer);
        let (trusted, mut control) = self.saved().iter().find(|s| s.key == key && s.trusted).map(|s| (true, s.control)).unwrap_or((false, false));
        let mut via = "trusted";
        if auth == access::PASSWORD || auth == access::PAIR {
            // Contraseña de acceso desatendido o código de emparejamiento: entra sin que nadie acepte.
            match self.authenticate(&mut ch, &auth, &key, &name, &plat, peer_addr) {
                Ok(allow) => { control = allow; via = if auth == access::PAIR { "pair" } else { "password" }; }
                Err(e) => {
                    ch.send_typed(msg::REJECT, &access::reject_body(&e, true))?;
                    return Ok(());
                }
            }
        } else if !trusted && self.st.lock().unwrap().access.reject_unknown {
            store::log(&format!("{name} pidió ver la pantalla: rechazado (sólo dispositivos emparejados o con contraseña)"));
            let why = format!("{} sólo acepta dispositivos emparejados o con la contraseña de acceso", self.name);
            ch.send_typed(msg::REJECT, &access::reject_body(&why, true))?;
            return Ok(());
        } else if !trusted || (wants_control && !control) {
            // Hay que aceptar en esta pantalla: la app muestra la petición con el código.
            via = "approved";
            let req = self.next.fetch_add(1, Ordering::Relaxed) + 1;
            let (tx, rx) = channel();
            self.st.lock().unwrap().requests.push(Req { req, name: name.clone(), platform: plat.clone(), code: ch.code.clone(), control: wants_control, answer: tx });
            (self.emit)(&json!({"ev": "request", "req": req, "name": name, "platform": plat, "code": ch.code, "control": wants_control}));
            self.emit_state();
            let answer = rx.recv_timeout(Duration::from_secs(90));
            self.st.lock().unwrap().requests.retain(|r| r.req != req);
            self.emit_state();
            let Ok((accept, remember, allow)) = answer else {
                let _ = ch.send_typed(msg::REJECT, b"no se respondio a tiempo");
                return Ok(());
            };
            if !accept {
                ch.send_typed(msg::REJECT, format!("{} rechazó la conexión", self.name).as_bytes())?;
                return Ok(());
            }
            control = allow;
            if remember {
                let mut saved = self.saved();
                let ip = peer_addr.map(|a| a.ip().to_string()).unwrap_or_default();
                match saved.iter_mut().find(|s| s.key == key) {
                    Some(s) => { s.trusted = true; s.control = allow; s.name = name.clone(); s.platform = plat.clone(); }
                    None => saved.push(Saved { key: key.clone(), name: name.clone(), platform: plat.clone(), addr: ip, trusted: true, manual: false, control: allow, last_seen: 0 }),
                }
                store::save_devices(&saved);
            }
        }
        // Control: pedido, concedido y con el servicio de Accesibilidad de Kast activo.
        let mut note = String::new();
        let control = wants_control && control && if self.st.lock().unwrap().input { true } else {
            note = format!("sólo vista: en {}, activa Kast en Ajustes → Accesibilidad para poder controlarla", self.name);
            store::log(&format!("{name} pidió control, pero el servicio de Accesibilidad de Kast no está activo"));
            self.toast(format!("{name} quiere controlar esta pantalla: activa Kast en Ajustes → Accesibilidad"), true);
            false
        };
        let (w, h, fps) = { let v = self.video.lock().unwrap(); (v.w, v.h, v.fps) };
        let acc = json!({"control": control, "note": note, "displays": [[w, h]], "monitor": 1, "bounds": Rect::new(0.0, 0.0, w as f64, h as f64).json()});
        ch.send_typed(msg::ACCEPT, acc.to_string().as_bytes())?;
        {
            let mut saved = self.saved();
            if let Some(d) = saved.iter_mut().find(|d| d.key == key && d.trusted) {
                d.last_seen = store::now_secs();
                store::save_devices(&saved);
            }
        }
        if via == "password" {
            self.toast(format!("{name} entró con la contraseña de acceso desatendido ({})", if control { "controla" } else { "sólo ve" }), false);
        }
        let session = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let sock = ch.shutdown_handle().ok();
        if let Some(s) = &sock { s.set_read_timeout(None)?; }
        self.st.lock().unwrap().sessions.push(Sess { session, peer: name.clone(), platform: plat.clone(), key: key.clone(), control, via, sock });
        self.emit_state();
        store::log(&format!("{name} empieza a {} la pantalla", if control { "controlar" } else { "ver" }));
        let frames = self.subscribe(session);
        let closed = Arc::new(AtomicBool::new(false));
        {
            let (mut rx, h, cl, peer) = (ch.try_clone()?, self.clone(), closed.clone(), name.clone());
            std::thread::spawn(move || {
                h.viewer_input(&mut rx, session, control, &plat, &peer);
                cl.store(true, Ordering::Relaxed);
            });
        }
        let r = self.serve(&mut ch, &frames, w, h, fps, &closed);
        self.unsubscribe(session);
        {
            let mut st = self.st.lock().unwrap();
            st.sessions.retain(|s| s.session != session);
        }
        // Que no quede un botón pulsado en la pantalla.
        if control { (self.emit)(&json!({"ev": "input", "session": session, "t": "end"})); }
        self.emit_state();
        store::log(&format!("sesión con {name} terminada{}", r.err().map(|e| format!(": {e}")).unwrap_or_default()));
        Ok(())
    }

    /// Envía el vídeo hasta que se corte la conexión.
    fn serve(&self, ch: &mut Secure, frames: &Receiver<Frame>, w: u32, h: u32, fps: u32, closed: &AtomicBool) -> Res<()> {
        let mut hello = Vec::with_capacity(12);
        for x in [w, h, fps] { hello.extend_from_slice(&x.to_be_bytes()); }
        ch.send_typed(msg::HELLO, &hello)?;
        loop {
            if closed.load(Ordering::Relaxed) { return Err("el visor cerró la conexión".into()); }
            let f = match frames.recv_timeout(Duration::from_millis(500)) {
                Ok(f) => f,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err("se dejó de compartir la pantalla".into()),
            };
            let (data, ts) = (&f.0, f.1);
            let mut hdr = vec![msg::FRAME];
            hdr.extend_from_slice(&ts.to_be_bytes());
            // Android no tiene cursor: el visor usa el suyo.
            hdr.extend_from_slice(&0i32.to_be_bytes());
            hdr.extend_from_slice(&0i32.to_be_bytes());
            hdr.push(0);
            hdr.extend_from_slice(&(data.len() as u32).to_be_bytes());
            ch.send(&hdr)?;
            ch.send_chunks(data)?;
        }
    }

    /// Lo que manda el visor: petición de fotograma clave y, con control, teclado, ratón, portapapeles y archivos.
    fn viewer_input(&self, rx: &mut Secure, session: u64, control: bool, viewer: &str, peer: &str) {
        let mut m = Vec::new();
        let mut clip: Option<(u8, usize, Vec<u8>)> = None;
        let mut files: Option<share::FileRecv> = None;
        // Posición del ratón y botones pulsados (los movimientos sólo se entregan arrastrando).
        let (mut pos, mut buttons) = ((0, 0), 0u8);
        while rx.recv(&mut m).is_ok() {
            match m.first() {
                Some(&msg::KEYFRAME) => self.keyframe(),
                Some(&msg::INPUT) if control => {
                    let Some(e) = Event::decode(&m[1..]) else { continue };
                    let v = match e {
                        Event::Move { x, y } => {
                            pos = (x, y);
                            if buttons == 0 { continue; }
                            json!({"t": "move", "x": x, "y": y})
                        }
                        Event::Button { button, down, x, y } => {
                            pos = (x, y);
                            if button < 8 { if down { buttons |= 1 << button } else { buttons &= !(1 << button) } }
                            json!({"t": "button", "b": button, "down": down, "x": x, "y": y})
                        }
                        Event::Wheel { dx, dy } => json!({"t": "wheel", "dx": dx, "dy": dy, "x": pos.0, "y": pos.1}),
                        Event::Key { down, code } => json!({"t": "key", "code": input::map_modifier(&code, viewer, "android"), "down": down}),
                        Event::Text { text } => json!({"t": "text", "text": text}),
                    };
                    let mut v = v;
                    v["ev"] = "input".into();
                    v["session"] = session.into();
                    (self.emit)(&v);
                }
                Some(&(msg::CLIP | msg::CLIP_DATA)) if control => {
                    if let Some(text) = on_clip(&m, &mut clip) { (self.emit)(&json!({"ev": "clip_in", "text": text})); }
                }
                Some(&(msg::FILE | msg::FILE_DATA | msg::FILE_END)) if control => {
                    let dir = self.st.lock().unwrap().downloads.clone();
                    let rcv = files.get_or_insert_with(|| share::FileRecv::in_dir(dir));
                    match rcv.on_message(&m) {
                        Ok(Some(r)) => {
                            let what = format!("{} archivo{} ({})", r.count, if r.count == 1 { "" } else { "s" }, share::human(r.bytes));
                            store::log(&format!("recibidos de {peer}: {what} en {}", r.first.display()));
                            self.toast(format!("{peer} te envió {what} a Descargas"), false);
                            (self.emit)(&json!({"ev": "files", "count": r.count, "bytes": r.bytes, "first": r.first.to_string_lossy()}));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            store::log(&format!("recibir archivos de {peer}: {e}"));
                            self.toast(format!("No se pudo recibir un archivo de {peer}: {e}"), true);
                            files = None;
                        }
                    }
                }
                _ => {}
            }
        }
    }

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
            drop(st);
            let ip = peer_addr.map(|a| a.ip().to_string()).unwrap_or_default();
            let mut saved = self.saved();
            match saved.iter_mut().find(|s| s.key == key) {
                Some(s) => { s.trusted = true; s.control = code.control; s.name = name.to_string(); s.platform = plat.to_string(); }
                None => saved.push(Saved { key: key.to_string(), name: name.to_string(), platform: plat.to_string(), addr: ip,
                                           trusted: true, manual: false, control: code.control, last_seen: 0 }),
            }
            store::save_devices(&saved);
            code.control
        };
        if kind == access::PAIR {
            store::log(&format!("{name} se emparejó con este equipo"));
            self.toast(format!("{name} quedó emparejado: ya puede entrar sin preguntar"), false);
        } else {
            store::log(&format!("{name} entró con la contraseña de acceso desatendido"));
        }
        self.emit_state();
        Ok(control)
    }
}

/// Corta las sesiones que cumplan `f` (su hilo termina al cerrarse la conexión).
fn cut(sessions: &[Sess], f: impl Fn(&Sess) -> bool) {
    for s in sessions.iter().filter(|s| f(s)) {
        if let Some(sock) = &s.sock { let _ = sock.shutdown(std::net::Shutdown::Both); }
    }
}

/// Portapapeles que llega en trozos (CLIP + CLIP_DATA); devuelve el texto completo (las imágenes se ignoran).
fn on_clip(m: &[u8], rx: &mut Option<(u8, usize, Vec<u8>)>) -> Option<String> {
    match m[0] {
        msg::CLIP if m.len() >= 6 => {
            let total = u32::from_be_bytes(m[2..6].try_into().unwrap()) as usize;
            *rx = (total <= 32 << 20).then(|| (m[1], total, Vec::with_capacity(total)));
        }
        msg::CLIP_DATA => { if let Some((_, _, d)) = rx { d.extend_from_slice(&m[1..]); } }
        _ => return None,
    }
    if !matches!(rx, Some((_, total, d)) if d.len() >= *total) { return None; }
    let (kind, _, d) = rx.take()?;
    (kind == 1).then(|| String::from_utf8_lossy(&d).into_owned())
}
