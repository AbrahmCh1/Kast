//! Canal TCP cifrado y autenticado en ambos sentidos con Noise XX
//! (X25519 + ChaCha20-Poly1305 + BLAKE2s). Cada equipo tiene una identidad (par de claves) propia;
//! la primera vez, el equipo que comparte debe aceptar al otro, y ambos muestran el mismo código
//! de 6 dígitos derivado del intercambio para descartar a alguien en medio.
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const MAX_PLAIN: usize = 65535 - 16;

/// Mensajes del protocolo (primer byte del texto plano).
pub mod msg {
    /// anfitrión -> visor: u32 ancho, u32 alto, u32 fps
    pub const HELLO: u8 = 1;
    /// anfitrión -> visor: u64 µs de captura, i32 x, i32 y, u8 cursor visible, u32 bytes H.264 que siguen
    pub const FRAME: u8 = 2;
    /// anfitrión -> visor: trozo de los datos H.264 del FRAME en curso
    pub const DATA: u8 = 3;
    /// anfitrión -> visor: sólo se movió el cursor (i32 x, i32 y, u8 visible)
    pub const CURSOR: u8 = 4;
    /// visor -> anfitrión: pide un fotograma clave (le falta la imagen inicial o el decodificador falló)
    pub const KEYFRAME: u8 = 13;
    /// visor -> anfitrión: teclado y ratón (ver input.rs)
    pub const INPUT: u8 = 20;
    /// ambos sentidos (sesión con control): portapapeles, u8 tipo (1 texto UTF-8, 2 PNG) + u32 bytes; siguen CLIP_DATA
    pub const CLIP: u8 = 30;
    pub const CLIP_DATA: u8 = 31;
    /// sesión con control: archivo, JSON {"id","name" (ruta relativa),"size","last"}. Del visor al anfitrión
    /// (archivos soltados en el visor) y del anfitrión al visor (archivos que se sacan arrastrando, ver DRAG_FILES)
    pub const FILE: u8 = 32;
    /// u32 id + trozo del archivo
    pub const FILE_DATA: u8 = 33;
    /// u32 id: archivo completo
    pub const FILE_END: u8 = 34;
    /// visor -> anfitrión (sesión con control): el ratón salió del visor con el botón pulsado; ¿se arrastran archivos?
    pub const DRAG_QUERY: u8 = 35;
    /// anfitrión -> visor: JSON {"items": [{"name","dir"}], "count", "bytes"} con lo que se arrastra (vacío si nada);
    /// si hay archivos, siguen FILE / FILE_DATA / FILE_END con su contenido
    pub const DRAG_FILES: u8 = 36;
    /// ambos sentidos, si los dos equipos lo anuncian ("acks" en REQUEST y en ACCEPT): u64 bytes de fondo (archivos y
    /// portapapeles) recibidos en esta conexión. Lo gestiona el propio canal (ver Pacer); no llega a la aplicación.
    pub const ACK: u8 = 37;
    /// Mensajes de fondo: se envían al ritmo que deja la red sin hacer cola (ver Pacer).
    pub fn is_bulk(kind: u8) -> bool { matches!(kind, CLIP | CLIP_DATA | FILE | FILE_DATA | FILE_END) }
    /// visor -> anfitrión: JSON {"name","platform","monitor","control"}
    pub const REQUEST: u8 = 10;
    /// anfitrión -> visor: petición aceptada, JSON {"control": bool, "note": texto} (siguen HELLO y los fotogramas)
    pub const ACCEPT: u8 = 11;
    /// anfitrión -> visor: petición rechazada (texto con el motivo)
    pub const REJECT: u8 = 12;
    /// anfitrión -> visor: demuestra que conoces la contraseña o el código, JSON {"kind": "password"|"pair", "salt": hex}
    pub const CHALLENGE: u8 = 14;
    /// visor -> anfitrión: JSON {"proof": hex} = MAC(Argon2id(secreto, salt), binding) (ver access.rs)
    pub const AUTH: u8 = 15;
}

fn err(e: impl std::fmt::Display) -> io::Error { io::Error::other(e.to_string()) }

pub fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// Identidad del equipo: par de claves X25519.
#[derive(Clone)]
pub struct Identity {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

impl Identity {
    pub fn generate() -> Self {
        let kp = snow::Builder::new(PATTERN.parse().unwrap()).generate_keypair().expect("claves");
        Self { private: kp.private, public: kp.public }
    }
    /// Identificador corto y estable del equipo (prefijo de la clave pública).
    pub fn id(&self) -> String { id_of(&self.public) }
}

pub fn id_of(public: &[u8]) -> String { hex(&public[..8.min(public.len())]) }

/// ID de 9 dígitos para mostrar y escribir ("482 913 057"), derivado del identificador (`id_of`).
pub fn short_id(id: &str) -> String {
    let n = unhex(id).filter(|b| b.len() >= 8).map(|b| u64::from_be_bytes(b[..8].try_into().unwrap()) % 1_000_000_000).unwrap_or(0);
    format!("{:03} {:03} {:03}", n / 1_000_000, n / 1000 % 1000, n % 1000)
}

/// 9 dígitos con el formato del ID ("482913057" -> "482 913 057").
pub fn short_id_fmt(d: &str) -> String {
    if d.len() == 9 { format!("{} {} {}", &d[..3], &d[3..6], &d[6..]) } else { d.to_string() }
}

/// Sólo los dígitos de un ID escrito por el usuario ("482-913 057" -> "482913057").
pub fn digits(s: &str) -> String { s.chars().filter(char::is_ascii_digit).collect() }

/// ¿Es un ID de 9 dígitos escrito por el usuario (sólo números, espacios o guiones)? Así no se confunde con el
/// identificador interno (hexadecimal), que al quitarle las letras podría dejar 9 dígitos por casualidad.
pub fn typed_short_id(s: &str) -> Option<String> {
    (s.chars().all(|c| c.is_ascii_digit() || c == ' ' || c == '-') && digits(s).len() == 9).then(|| digits(s))
}

/// Código de 6 dígitos que ambos lados muestran tras el intercambio.
fn pairing_code(hash: &[u8]) -> String {
    let n = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

/// Cómo terminó la espera de una petición (`Secure::wait_answer`).
pub enum Wait<T> {
    Answer(T),
    Timeout,
    /// Quien pedía canceló o se fue.
    Gone,
}

/// Cola que un envío de fondo puede añadir a la red (por encima del mínimo medido).
const TARGET: f64 = 0.025;
/// Ventana de bytes de fondo sin confirmar: inicial, mínima, máxima y lo que crece por ida y vuelta sin cola.
const INIT_W: f64 = 64_000.0;
const MIN_W: f64 = 32_000.0;
const MAX_W: f64 = 32e6;
const STEP: f64 = 16_000.0;

/// Ritmo de los envíos de fondo (archivos y portapapeles), al estilo de LEDBAT. Quien recibe confirma lo recibido
/// (msg::ACK) y quien envía mide cuánto tarda la confirmación: si tarda más que el mínimo visto, la red está
/// acumulando cola y la ventana de bytes sin confirmar se encoge; si no, crece hasta llenar la red. Así un archivo
/// grande no deja más de unos `TARGET` ms de cola y el vídeo, el teclado y el ratón (que van por la misma conexión y
/// la misma red) siguen llegando a tiempo. Sin confirmaciones (el otro equipo no las anunció) se envía sin ritmo.
struct Pacer {
    st: Mutex<Pace>,
    cv: Condvar,
}

struct Pace {
    /// Los dos equipos confirman.
    on: bool,
    sent: u64,
    acked: u64,
    window: f64,
    /// (bytes enviados hasta ese mensaje, cuándo): para medir la ida y vuelta al llegar su confirmación.
    marks: VecDeque<(u64, Instant)>,
    /// Ida y vuelta mínima de los dos últimos tramos de 10 s (la de la red sin cola).
    base: [Duration; 2],
    base_t: Instant,
    cut: Instant,
    /// Bytes de fondo recibidos, y el hilo que los confirma (para no frenar nunca al que recibe).
    got: u64,
    acks: Option<Sender<u64>>,
}

impl Default for Pacer {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            st: Mutex::new(Pace { on: false, sent: 0, acked: 0, window: INIT_W, marks: VecDeque::new(), base: [Duration::from_secs(60); 2],
                                  base_t: now, cut: now, got: 0, acks: None }),
            cv: Condvar::new(),
        }
    }
}

impl Pacer {
    /// Espera a que quepa otro mensaje en la ventana.
    fn wait(&self) {
        let mut st = self.st.lock().unwrap();
        let (mut last, mut since) = (st.acked, Instant::now());
        while st.on && (st.sent - st.acked) as f64 >= st.window {
            st = self.cv.wait_timeout(st, Duration::from_millis(100)).unwrap().0;
            if st.acked != last {
                (last, since) = (st.acked, Instant::now());
            } else if since.elapsed() > Duration::from_secs(10) {
                // Sin confirmaciones tanto tiempo no debería pasar: mejor seguir que quedarse parado.
                break;
            }
        }
    }

    fn sent(&self, len: usize) {
        let mut st = self.st.lock().unwrap();
        st.sent += len as u64;
        if st.on {
            let mark = (st.sent, Instant::now());
            st.marks.push_back(mark);
        }
    }

    fn acked(&self, total: u64) {
        let mut st = self.st.lock().unwrap();
        if total <= st.acked { return; }
        let now = Instant::now();
        let newly = (total - st.acked) as f64;
        st.acked = total.min(st.sent);
        let mut rtt = None;
        while let Some(&(end, t)) = st.marks.front() {
            if end > total { break; }
            rtt = Some(now - t);
            st.marks.pop_front();
        }
        if let Some(rtt) = rtt {
            if now - st.base_t > Duration::from_secs(10) {
                st.base = [st.base[1], rtt];
                st.base_t = now;
            } else {
                st.base[1] = st.base[1].min(rtt);
            }
            let q = rtt.saturating_sub(st.base[0].min(st.base[1])).as_secs_f64();
            let off = ((TARGET - q) / TARGET).clamp(-1.0, 1.0);
            st.window += off * newly * STEP / st.window;
            // Mucha cola de golpe (el vídeo pidió más, la wifi bajó…): a la mitad, como mucho una vez por ida y vuelta.
            if q > 2.0 * TARGET && now - st.cut > rtt {
                st.window *= 0.5;
                st.cut = now;
            }
            st.window = st.window.clamp(MIN_W, MAX_W);
        }
        self.cv.notify_all();
    }
}

/// Canal cifrado. Se puede clonar (try_clone) para enviar y recibir desde hilos distintos.
pub struct Secure {
    stream: TcpStream,
    /// Enviar y recibir llevan cada uno su contador (el nonce de Noise) con su cerrojo: recibir nunca espera a un envío
    /// que está bloqueado porque la red no da abasto (si no, enviando un archivo mientras llega el vídeo, los dos
    /// equipos se quedaban esperándose el uno al otro). En el cable es lo mismo que el modo con estado.
    noise: Arc<snow::StatelessTransportState>,
    tx: Arc<Mutex<u64>>,
    rx: Arc<Mutex<u64>>,
    /// Envíos normales (teclado, ratón, vídeo…) esperando turno: los de fondo (archivos) les ceden el paso.
    urgent: Arc<AtomicUsize>,
    /// Ritmo de los envíos de fondo y confirmación de lo recibido.
    pace: Arc<Pacer>,
    /// Lo tienen todas las copias de la aplicación: cuando no queda ninguna, el hilo de confirmaciones termina.
    life: Arc<()>,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
    /// Clave pública del otro equipo.
    pub peer: Vec<u8>,
    /// Código de verificación de esta conexión.
    pub code: String,
    /// Hash del intercambio Noise: único de esta conexión. Las pruebas de contraseña y de código se ligan a él,
    /// así que no sirven en otra conexión (p. ej. de alguien que reenvíe el tráfico entre los dos equipos).
    pub binding: Vec<u8>,
}

fn send_raw(s: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    s.write_all(&(data.len() as u16).to_be_bytes())?;
    s.write_all(data)
}

fn recv_raw(s: &mut TcpStream, buf: &mut Vec<u8>) -> io::Result<usize> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len)?;
    let n = u16::from_be_bytes(len) as usize;
    buf.resize(n, 0);
    s.read_exact(&mut buf[..n])?;
    Ok(n)
}

impl Secure {
    fn finish(stream: TcpStream, hs: snow::HandshakeState) -> io::Result<Self> {
        let peer = hs.get_remote_static().ok_or_else(|| err("el otro equipo no envió su identidad"))?.to_vec();
        let code = pairing_code(hs.get_handshake_hash());
        let binding = hs.get_handshake_hash().to_vec();
        Ok(Self { stream, noise: Arc::new(hs.into_stateless_transport_mode().map_err(err)?), tx: Arc::default(), rx: Arc::default(),
                  urgent: Arc::default(), pace: Arc::default(), life: Arc::default(), rbuf: Vec::new(), wbuf: vec![0u8; 65535], peer, code,
                  binding })
    }

    /// Lado que se conecta (visor).
    pub fn connect(mut stream: TcpStream, me: &Identity) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        // Lo que el visor envía (teclado y ratón, pero también archivos) no se acumula en el sistema: así el ratón no
        // queda detrás de megas de un archivo esperando a salir.
        let _ = socket2::SockRef::from(&stream).set_send_buffer_size(256 << 10);
        let mut hs = snow::Builder::new(PATTERN.parse().map_err(err)?).local_private_key(&me.private).map_err(err)?
            .build_initiator().map_err(err)?;
        let mut buf = vec![0u8; 65535];
        let mut msg = Vec::new();
        let n = hs.write_message(&[], &mut buf).map_err(err)?;
        send_raw(&mut stream, &buf[..n])?;
        let n = recv_raw(&mut stream, &mut msg).map_err(|_| err("el equipo no respondió al intercambio cifrado"))?;
        hs.read_message(&msg[..n], &mut buf).map_err(|_| err("intercambio cifrado inválido"))?;
        let n = hs.write_message(&[], &mut buf).map_err(err)?;
        send_raw(&mut stream, &buf[..n])?;
        Self::finish(stream, hs)
    }

    /// Lado que acepta (anfitrión).
    pub fn accept(mut stream: TcpStream, me: &Identity) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let mut hs = snow::Builder::new(PATTERN.parse().map_err(err)?).local_private_key(&me.private).map_err(err)?
            .build_responder().map_err(err)?;
        let mut buf = vec![0u8; 65535];
        let mut msg = Vec::new();
        let n = recv_raw(&mut stream, &mut msg)?;
        hs.read_message(&msg[..n], &mut buf).map_err(|_| err("intercambio cifrado inválido"))?;
        let n = hs.write_message(&[], &mut buf).map_err(err)?;
        send_raw(&mut stream, &buf[..n])?;
        let n = recv_raw(&mut stream, &mut msg)?;
        hs.read_message(&msg[..n], &mut buf).map_err(|_| err("intercambio cifrado inválido"))?;
        Self::finish(stream, hs)
    }

    /// Copia del socket para poder cortar la conexión desde otro hilo.
    pub fn shutdown_handle(&self) -> io::Result<TcpStream> { self.stream.try_clone() }

    /// Otra referencia al mismo canal (para un segundo hilo).
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self { stream: self.stream.try_clone()?, noise: self.noise.clone(), tx: self.tx.clone(), rx: self.rx.clone(), urgent: self.urgent.clone(),
                  pace: self.pace.clone(), life: self.life.clone(), rbuf: Vec::new(), wbuf: vec![0u8; 65535], peer: self.peer.clone(),
                  code: self.code.clone(), binding: self.binding.clone() })
    }

    /// Los dos equipos confirman lo recibido de fondo (lo anunciaron al conectar): desde ahora, archivos y
    /// portapapeles van al ritmo de la red (ver Pacer).
    pub fn enable_acks(&self) { self.pace.st.lock().unwrap().on = true; }

    /// Confirma lo recibido de fondo desde un hilo propio: quien recibe nunca espera a poder enviar.
    fn ack(&self, got: u64) {
        let mut st = self.pace.st.lock().unwrap();
        if st.acks.is_none() {
            let Ok(mut ch) = self.try_clone() else { return };
            // Su copia no cuenta como de la aplicación: termina cuando la aplicación suelta todas las suyas.
            ch.life = Arc::new(());
            let life: Weak<()> = Arc::downgrade(&self.life);
            let (tx, rx) = channel::<u64>();
            std::thread::spawn(move || loop {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(mut n) => {
                        while let Ok(m) = rx.try_recv() { n = n.max(m); }
                        let mut m = vec![msg::ACK];
                        m.extend_from_slice(&n.to_be_bytes());
                        if ch.send(&m).is_err() { break; }
                    }
                    Err(RecvTimeoutError::Timeout) if life.strong_count() > 0 => {}
                    Err(_) => break,
                }
            });
            st.acks = Some(tx);
        }
        if let Some(tx) = &st.acks { let _ = tx.send(got); }
    }

    /// Espera la respuesta a una petición que alguien tiene que aceptar en este equipo (hasta `max`) vigilando la
    /// conexión: mientras espera, quien pide no envía nada, así que si la cierra es que canceló (o se fue) y la
    /// petición ya no tiene que seguir en pantalla.
    pub fn wait_answer<T>(&self, rx: &Receiver<T>, max: Duration) -> Wait<T> {
        let t = Instant::now();
        let _ = self.stream.set_nonblocking(true);
        let r = loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(a) => break Wait::Answer(a),
                Err(RecvTimeoutError::Disconnected) => break Wait::Timeout,
                Err(RecvTimeoutError::Timeout) => {}
            }
            match self.stream.peek(&mut [0u8; 1]) {
                Ok(0) => break Wait::Gone,
                Err(e) if e.kind() != io::ErrorKind::WouldBlock && e.kind() != io::ErrorKind::Interrupted => break Wait::Gone,
                _ => {}
            }
            if t.elapsed() > max { break Wait::Timeout; }
        };
        let _ = self.stream.set_nonblocking(false);
        r
    }

    fn write(&mut self, plain: &[u8]) -> io::Result<()> {
        // Cifrar y escribir bajo el mismo cerrojo mantiene el orden de los nonces en el cable.
        let mut nonce = self.tx.lock().unwrap();
        let n = self.noise.write_message(*nonce, plain, &mut self.wbuf).map_err(err)?;
        *nonce += 1;
        // Lo de fondo se cuenta igual que lo cuenta quien recibe, vaya o no por send_bulk.
        if plain.first().is_some_and(|k| msg::is_bulk(*k)) { self.pace.sent(plain.len()); }
        send_raw(&mut self.stream, &self.wbuf[..n])
    }

    pub fn send(&mut self, plain: &[u8]) -> io::Result<()> {
        self.urgent.fetch_add(1, Ordering::SeqCst);
        let r = self.write(plain);
        self.urgent.fetch_sub(1, Ordering::SeqCst);
        r
    }

    /// Envío de fondo (archivos, portapapeles): al ritmo que deja la red sin hacer cola (ver Pacer) y cediendo el paso
    /// a los envíos normales pendientes, para no retrasar el teclado y el ratón (ni el vídeo).
    pub fn send_bulk(&mut self, plain: &[u8]) -> io::Result<()> {
        self.pace.wait();
        while self.urgent.load(Ordering::SeqCst) > 0 { std::thread::sleep(Duration::from_millis(1)); }
        self.write(plain)
    }

    pub fn send_typed(&mut self, kind: u8, body: &[u8]) -> io::Result<()> {
        let mut m = Vec::with_capacity(body.len() + 1);
        m.push(kind);
        m.extend_from_slice(body);
        self.send(&m)
    }

    /// Envía datos grandes partidos en trozos DATA.
    pub fn send_chunks(&mut self, data: &[u8]) -> io::Result<()> {
        let mut part = Vec::with_capacity(MAX_PLAIN);
        for chunk in data.chunks(MAX_PLAIN - 1) {
            part.clear();
            part.push(msg::DATA);
            part.extend_from_slice(chunk);
            self.send(&part)?;
        }
        Ok(())
    }

    pub fn recv(&mut self, out: &mut Vec<u8>) -> io::Result<usize> {
        // Un mensaje entero (leer y descifrar) bajo el cerrojo de recibir: el orden de los nonces es el del cable.
        let mut nonce = self.rx.lock().unwrap();
        loop {
            let mut raw = std::mem::take(&mut self.rbuf);
            let r = recv_raw(&mut self.stream, &mut raw).and_then(|n| {
                out.resize(65535, 0);
                self.noise.read_message(*nonce, &raw[..n], out).map_err(err)
            });
            self.rbuf = raw;
            let len = r?;
            *nonce += 1;
            out.truncate(len);
            match out.first() {
                // Confirmación de lo que enviamos de fondo: es del canal, no de la aplicación.
                Some(&msg::ACK) if len >= 9 => { self.pace.acked(u64::from_be_bytes(out[1..9].try_into().unwrap())); continue; }
                Some(&k) if msg::is_bulk(k) => {
                    let (on, got) = {
                        let mut st = self.pace.st.lock().unwrap();
                        st.got += len as u64;
                        (st.on, st.got)
                    };
                    if on { self.ack(got); }
                }
                _ => {}
            }
            return Ok(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Los dos equipos se envían mucho a la vez (un archivo en un sentido y el vídeo en el otro) por una red lenta (el
    /// anfitrión tarda en empezar a leer): ninguno se queda esperando al otro.
    #[test]
    fn enviar_y_recibir_a_la_vez() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let host = std::thread::spawn(move || Secure::accept(l.accept().unwrap().0, &Identity::generate()).unwrap());
        let viewer = Secure::connect(TcpStream::connect(addr).unwrap(), &Identity::generate()).unwrap();
        let host = host.join().unwrap();
        // Búferes pequeños: se llenan enseguida.
        for s in [&viewer.stream, &host.stream] {
            let r = socket2::SockRef::from(s);
            r.set_send_buffer_size(16 << 10).unwrap();
            r.set_recv_buffer_size(16 << 10).unwrap();
        }
        const N: usize = 300; // mensajes de 60 KB en cada sentido
        let (done, all_done) = std::sync::mpsc::channel();
        for (ch, viewer_side) in [(viewer, true), (host, false)] {
            let (mut snd, mut rcv, done) = (ch.try_clone().unwrap(), ch, done.clone());
            std::thread::spawn(move || {
                let big = vec![7u8; 60_000];
                for _ in 0..N { if viewer_side { snd.send_bulk(&big).unwrap() } else { snd.send(&big).unwrap() } }
            });
            std::thread::spawn(move || {
                if !viewer_side { std::thread::sleep(Duration::from_millis(500)); }
                let mut m = Vec::new();
                for _ in 0..N { rcv.recv(&mut m).unwrap(); assert_eq!(m.len(), 60_000); }
                done.send(()).unwrap();
            });
        }
        for _ in 0..2 { all_done.recv_timeout(Duration::from_secs(20)).expect("los dos equipos se quedaron esperándose"); }
    }

    /// Reenvía `from` -> `to` a `rate` bytes/s con una cola sin límite: una red lenta y cargada (como una wifi).
    fn slow_pipe(mut from: TcpStream, mut to: TcpStream, rate: f64) {
        let (tx, rx) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 8192];
            while let Ok(n) = from.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() { break; }
            }
        });
        std::thread::spawn(move || {
            let mut next = Instant::now();
            for b in rx {
                // Lo que se pasó de largo al dormir se recupera (si no, la red iría más lenta de lo dicho), salvo tras
                // una pausa sin nada que enviar.
                next = next.max(Instant::now() - Duration::from_millis(20)) + Duration::from_secs_f64(b.len() as f64 / rate);
                if let Some(d) = next.checked_duration_since(Instant::now()) { std::thread::sleep(d); }
                if to.write_all(&b).is_err() { break; }
            }
        });
    }

    /// El anfitrión envía vídeo (2 KB cada 16 ms) y a la vez un archivo de 6 MB por una red de 4 MB/s. Devuelve el
    /// retraso del vídeo (percentil 95) y lo que tardó el archivo.
    fn video_while_sending_file(acks: bool) -> (Duration, Duration) {
        let host_l = TcpListener::bind("127.0.0.1:0").unwrap();
        let link_l = TcpListener::bind("127.0.0.1:0").unwrap();
        let (host_addr, link_addr) = (host_l.local_addr().unwrap(), link_l.local_addr().unwrap());
        std::thread::spawn(move || {
            let a = link_l.accept().unwrap().0;
            let b = TcpStream::connect(host_addr).unwrap();
            slow_pipe(a.try_clone().unwrap(), b.try_clone().unwrap(), 4e6);
            slow_pipe(b, a, 4e6);
        });
        let host = std::thread::spawn(move || Secure::accept(host_l.accept().unwrap().0, &Identity::generate()).unwrap());
        let mut viewer = Secure::connect(TcpStream::connect(link_addr).unwrap(), &Identity::generate()).unwrap();
        let host = host.join().unwrap();
        if acks {
            viewer.enable_acks();
            host.enable_acks();
        }
        // El anfitrión también lee (en Kast, el hilo del teclado y el ratón): por ahí le llegan las confirmaciones.
        let mut rd = host.try_clone().unwrap();
        std::thread::spawn(move || { let mut m = Vec::new(); while rd.recv(&mut m).is_ok() {} });
        let t0 = Instant::now();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (mut video, st) = (host.try_clone().unwrap(), stop.clone());
        std::thread::spawn(move || while !st.load(Ordering::Relaxed) {
            let mut m = vec![0u8; 2000];
            m[0] = msg::CURSOR;
            m[1..9].copy_from_slice(&(t0.elapsed().as_micros() as u64).to_be_bytes());
            if video.send(&m).is_err() { break; }
            std::thread::sleep(Duration::from_millis(16));
        });
        const N: usize = 375; // trozos de 16 KB: 6 MB
        let mut file = host.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut m = vec![0u8; 16_000];
            m[0] = msg::FILE_DATA;
            for _ in 0..N { if file.send_bulk(&m).is_err() { break; } }
        });
        // Hasta que llegue el vídeo enviado después de que terminara el archivo: también cuenta el que quedó detrás.
        let (mut delays, mut got, mut took, mut m) = (Vec::new(), 0, None, Vec::new());
        loop {
            viewer.recv(&mut m).unwrap();
            match m[0] {
                msg::CURSOR => {
                    let sent = Duration::from_micros(u64::from_be_bytes(m[1..9].try_into().unwrap()));
                    delays.push(t0.elapsed() - sent);
                    if took.is_some_and(|t| sent > t) { break; }
                }
                msg::FILE_DATA => {
                    got += 1;
                    if got == N { took = Some(t0.elapsed()); }
                }
                _ => {}
            }
        }
        let took = took.unwrap();
        stop.store(true, Ordering::Relaxed);
        delays.sort();
        (delays[delays.len() * 95 / 100], took)
    }

    /// Un archivo grande por una red lenta no frena el vídeo: sin ritmo, el vídeo espera detrás de megas de archivo;
    /// con confirmaciones, la cola se queda en unos milisegundos y el archivo sigue usando casi toda la red.
    #[test]
    fn archivo_grande_sin_frenar_el_video() {
        let (before, _) = video_while_sending_file(false);
        let (after, took) = video_while_sending_file(true);
        eprintln!("retraso del vídeo (p95): sin ritmo {before:?}, con ritmo {after:?}; archivo en {took:?}");
        assert!(before > Duration::from_millis(500), "la prueba no reproduce la red cargada ({before:?})");
        assert!(after < Duration::from_millis(150), "el vídeo sigue esperando detrás del archivo ({after:?})");
        assert!(took < Duration::from_secs(3), "el archivo va demasiado lento ({took:?}; a toda velocidad, 1,5 s)");
    }
}
