//! Canal TCP cifrado y autenticado con Noise (NNpsk0: X25519 + ChaCha20-Poly1305 + BLAKE2s).
//! Sin la clave compartida no se completa el intercambio: nadie más en la red puede ver ni conectarse.
use std::io::{self, Read, Write};
use std::net::TcpStream;

const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const MAX_PLAIN: usize = 65535 - 16;

pub type Key = [u8; 32];

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
}

fn err(e: impl std::fmt::Display) -> io::Error { io::Error::other(e.to_string()) }

pub fn parse_key(hex: &str) -> Option<Key> {
    let hex = hex.trim();
    if hex.len() != 64 { return None; }
    let mut k = [0u8; 32];
    for i in 0..32 { k[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?; }
    Some(k)
}

pub fn key_hex(k: &Key) -> String { k.iter().map(|b| format!("{b:02x}")).collect() }

pub fn new_key() -> Key {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).expect("sin fuente de aleatoriedad");
    k
}

pub struct Secure {
    stream: TcpStream,
    noise: snow::TransportState,
    rbuf: Vec<u8>,
    wbuf: Vec<u8>,
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
    /// Lado que se conecta (visor).
    pub fn connect(mut stream: TcpStream, key: &Key) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let mut hs = snow::Builder::new(PATTERN.parse().map_err(err)?).psk(0, key).map_err(err)?.build_initiator().map_err(err)?;
        let mut buf = vec![0u8; 65535];
        let n = hs.write_message(&[], &mut buf).map_err(err)?;
        send_raw(&mut stream, &buf[..n])?;
        let mut msg = Vec::new();
        let n = recv_raw(&mut stream, &mut msg).map_err(|_| err("el anfitrión rechazó la conexión: ¿clave incorrecta?"))?;
        hs.read_message(&msg[..n], &mut buf).map_err(|_| err("clave incorrecta o anfitrión no válido"))?;
        Ok(Self { stream, noise: hs.into_transport_mode().map_err(err)?, rbuf: Vec::new(), wbuf: vec![0u8; 65535] })
    }

    /// Lado que acepta (anfitrión).
    pub fn accept(mut stream: TcpStream, key: &Key) -> io::Result<Self> {
        stream.set_nodelay(true)?;
        let mut hs = snow::Builder::new(PATTERN.parse().map_err(err)?).psk(0, key).map_err(err)?.build_responder().map_err(err)?;
        let mut buf = vec![0u8; 65535];
        let mut msg = Vec::new();
        let n = recv_raw(&mut stream, &mut msg)?;
        hs.read_message(&msg[..n], &mut buf).map_err(|_| err("clave incorrecta"))?;
        let n = hs.write_message(&[], &mut buf).map_err(err)?;
        send_raw(&mut stream, &buf[..n])?;
        Ok(Self { stream, noise: hs.into_transport_mode().map_err(err)?, rbuf: Vec::new(), wbuf: vec![0u8; 65535] })
    }

    pub fn send(&mut self, plain: &[u8]) -> io::Result<()> {
        let n = self.noise.write_message(plain, &mut self.wbuf).map_err(err)?;
        send_raw(&mut self.stream, &self.wbuf[..n])
    }

    /// Envía un tipo de mensaje + datos grandes partidos en trozos DATA.
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
        let r = self.noise.read_message(&raw[..n], out).map_err(err);
        self.rbuf = raw;
        let len = r?;
        out.truncate(len);
        Ok(len)
    }
}
