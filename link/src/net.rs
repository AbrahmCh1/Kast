//! Canal TCP cifrado y autenticado en ambos sentidos con Noise XX
//! (X25519 + ChaCha20-Poly1305 + BLAKE2s). Cada equipo tiene una identidad (par de claves) propia;
//! la primera vez, el equipo que comparte debe aceptar al otro, y ambos muestran el mismo código
//! de 6 dígitos derivado del intercambio para descartar a alguien en medio.
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

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
    /// visor -> anfitrión (sesión con control): archivo, JSON {"id","name" (ruta relativa),"size","last"}
    pub const FILE: u8 = 32;
    /// u32 id + trozo del archivo
    pub const FILE_DATA: u8 = 33;
    /// u32 id: archivo completo
    pub const FILE_END: u8 = 34;
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

/// Canal cifrado. Se puede clonar (	ry_clone) para enviar y recibir desde hilos distintos.
pub struct Secure {
    stream: TcpStream,
    noise: Arc<Mutex<snow::TransportState>>,
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
        Ok(Self { stream, noise: Arc::new(Mutex::new(hs.into_transport_mode().map_err(err)?)), rbuf: Vec::new(), wbuf: vec![0u8; 65535], peer, code, binding })
    }

    /// Lado que se conecta (visor).
    pub fn connect(mut stream: TcpStream, me: &Identity) -> io::Result<Self> {
        stream.set_nodelay(true)?;
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
        Ok(Self { stream: self.stream.try_clone()?, noise: self.noise.clone(), rbuf: Vec::new(), wbuf: vec![0u8; 65535], peer: self.peer.clone(), code: self.code.clone(), binding: self.binding.clone() })
    }

    pub fn send(&mut self, plain: &[u8]) -> io::Result<()> {
        // Cifrar y escribir bajo el mismo cerrojo mantiene el orden de los nonces en el cable.
        let mut noise = self.noise.lock().unwrap();
        let n = noise.write_message(plain, &mut self.wbuf).map_err(err)?;
        send_raw(&mut self.stream, &self.wbuf[..n])
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
        let mut raw = std::mem::take(&mut self.rbuf);
        let n = recv_raw(&mut self.stream, &mut raw)?;
        out.resize(65535, 0);
        let r = self.noise.lock().unwrap().read_message(&raw[..n], out).map_err(err);
        self.rbuf = raw;
        let len = r?;
        out.truncate(len);
        Ok(len)
    }
}
