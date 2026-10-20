//! kast-link — ver (y, en próximas fases, controlar) la pantalla de otro equipo por la red local.
//!
//!   kast-link list                                  monitores de este equipo
//!   kast-link host [-m N] [--fps 60] [--mbps 20] [--port 47800] [--bind 0.0.0.0]
//!   kast-link view <equipo>[:puerto] --key <clave> [-m N]
//!   kast-link key [--new]                           mostrar / regenerar la clave de este equipo
//!   kast-link record [-m N | --pattern] [--frames 120] <archivo>   grabar un flujo H.264 (pruebas)
//!   kast-link play <archivo> [--dump imagen.bmp]                   decodificar un flujo grabado (pruebas)
mod h264;
mod net;
mod viewer;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac as platform;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as platform;

use net::{Secure, msg};
use std::error::Error;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub type Res<T> = Result<T, Box<dyn Error>>;

const DEFAULT_PORT: u16 = 47800;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const RECORD_MAGIC: &[u8; 4] = b"KLNK";

/// Monitor de este equipo (numerados de izquierda a derecha, como en Kast).
pub struct DisplayInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Fotograma codificado en H.264 Annex B.
pub struct Encoded {
    pub data: Vec<u8>,
    /// Momento de la captura (µs desde 1970), para medir la latencia.
    pub ts_us: u64,
    /// Tiempo de captura + codificación en este equipo.
    pub work_ms: f32,
}

pub fn now_us() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0) }

/// Patrón de prueba sintético (BGRA): barras de color en movimiento y un cuadrado que rebota.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn pattern(n: u64, w: usize, h: usize, bgra: &mut [u8]) {
    const BARS: [[u8; 3]; 8] = [[255, 255, 255], [0, 255, 255], [255, 255, 0], [0, 255, 0], [255, 0, 255], [0, 0, 255], [255, 0, 0], [40, 40, 40]];
    let shift = (n as usize * 4) % w;
    let (sq, sx, sy) = (h / 6, (n as usize * 7) % (w - h / 6), (n as usize * 5) % (h - h / 6));
    for y in 0..h {
        for x in 0..w {
            let c = if x >= sx && x < sx + sq && y >= sy && y < sy + sq { [0, 0, 0] } else { BARS[((x + shift) % w) * 8 / w] };
            let p = (y * w + x) * 4;
            bgra[p..p + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
}

fn main() {
    platform::init();
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

struct Opts {
    pos: Vec<String>,
    flags: Vec<(String, String)>,
}

impl Opts {
    fn parse() -> Self {
        const SWITCHES: [&str; 2] = ["--new", "--pattern"];
        let mut pos = Vec::new();
        let mut flags = Vec::new();
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            if a.starts_with('-') {
                let v = if SWITCHES.contains(&a.as_str()) { String::new() } else { it.next().unwrap_or_default() };
                flags.push((a, v));
            } else {
                pos.push(a);
            }
        }
        Self { pos, flags }
    }
    fn get(&self, names: &[&str]) -> Option<&str> {
        self.flags.iter().rev().find(|(k, _)| names.contains(&k.as_str())).map(|(_, v)| v.as_str())
    }
    fn num<T: std::str::FromStr>(&self, names: &[&str], default: T) -> T {
        self.get(names).and_then(|v| v.parse().ok()).unwrap_or(default)
    }
}

fn run() -> Res<()> {
    let o = Opts::parse();
    match o.pos.first().map(String::as_str) {
        Some("list") => {
            for (i, d) in platform::displays()?.iter().enumerate() {
                println!("{}  {}  {}x{}  en {},{}", i + 1, d.name, d.w, d.h, d.x, d.y);
            }
            Ok(())
        }
        Some("host") => host(&o),
        Some("view") => view(&o),
        Some("record") => record(&o),
        Some("play") => play(&o),
        Some("key") => {
            println!("{}", net::key_hex(&load_key(o.get(&["--new"]).is_some())?));
            Ok(())
        }
        Some("version") | Some("--version") => { println!("kast-link {VERSION}"); Ok(()) }
        _ => {
            println!("kast-link {VERSION} — pantalla de otro equipo por la red local (cifrado)\n");
            println!("  kast-link list                                   monitores de este equipo");
            println!("  kast-link host [-m N] [--fps 60] [--mbps 20] [--port {DEFAULT_PORT}] [--bind 0.0.0.0]");
            println!("                                                   compartir el monitor N (1 = el de más a la izquierda)");
            println!("  kast-link view <equipo>[:puerto] --key <clave> [-m N]");
            println!("                                                   ver; con -m N, a pantalla completa en el monitor N");
            println!("  kast-link key [--new]                            clave de este equipo (la pide el visor)");
            println!("  kast-link record [-m N | --pattern] [--frames 120] <archivo>   grabar un flujo (pruebas)");
            println!("  kast-link play <archivo> [--dump imagen.bmp]     decodificar un flujo grabado (pruebas)");
            println!("\nEn el visor: Esc cierra, F11 alterna pantalla completa.");
            Ok(())
        }
    }
}

fn key_path() -> std::path::PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_else(|| ".".into()).join("Kast");
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| ".".into())
        .join("Library/Application Support/Kast");
    base.join("link.key")
}

/// Clave del anfitrión: se genera una vez y se guarda junto a la configuración de Kast.
fn load_key(regenerate: bool) -> Res<net::Key> {
    let p = key_path();
    if !regenerate {
        if let Some(k) = std::fs::read_to_string(&p).ok().and_then(|s| net::parse_key(&s)) {
            return Ok(k);
        }
    }
    let k = net::new_key();
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, net::key_hex(&k))?;
    Ok(k)
}

fn hello_bytes(w: u32, h: u32, fps: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(12);
    for x in [w, h, fps] { v.extend_from_slice(&x.to_be_bytes()); }
    v
}

fn parse_hello(b: &[u8]) -> Option<(usize, usize, u32)> {
    if b.len() < 12 { return None; }
    let rd = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
    Some((rd(0) as usize, rd(4) as usize, rd(8)))
}

struct Stats {
    t: Instant,
    frames: u32,
    bytes: usize,
    work: f64,
    lat: f64,
}

impl Stats {
    fn new() -> Self { Self { t: Instant::now(), frames: 0, bytes: 0, work: 0.0, lat: 0.0 } }
    fn tick(&mut self, label: &str, latency: bool) {
        let el = self.t.elapsed().as_secs_f64();
        if el < 2.0 { return; }
        let f = self.frames.max(1) as f64;
        let lat = if latency { format!("  latencia {:5.1} ms", self.lat / f) } else { String::new() };
        println!("{:5.1} fps  {:6.2} Mbps  {label} {:4.1} ms{lat}", self.frames as f64 / el, self.bytes as f64 * 8.0 / el / 1e6, self.work / f);
        *self = Self::new();
    }
}

// ---------------------------------------------------------------- anfitrión

fn host(o: &Opts) -> Res<()> {
    let key = load_key(false)?;
    let n: usize = o.num(&["-m", "--monitor"], 1);
    let fps: u32 = o.num(&["--fps"], 60).clamp(1, 240);
    let mbps: u32 = o.num(&["--mbps"], 20).clamp(1, 500);
    let port: u16 = o.num(&["--port"], DEFAULT_PORT);
    let bind = o.get(&["--bind"]).unwrap_or("0.0.0.0").to_string();
    let listener = TcpListener::bind((bind.as_str(), port))?;
    println!("kast-link {VERSION}: compartiendo el monitor {n} en {bind}:{port}, {fps} fps, {mbps} Mbps");
    println!("clave: {}", net::key_hex(&key));
    loop {
        let (stream, addr) = listener.accept()?;
        println!("conexión de {addr}");
        match serve(stream, &key, n, fps, mbps) {
            Ok(()) => println!("visor desconectado"),
            Err(e) => println!("sesión terminada: {e}"),
        }
    }
}

fn serve(stream: TcpStream, key: &net::Key, monitor: usize, fps: u32, mbps: u32) -> Res<()> {
    let mut ch = Secure::accept(stream, key)?;
    let mut src = platform::Source::new(monitor, fps, mbps, false)?;
    let (w, h) = src.size();
    let mut hello = vec![msg::HELLO];
    hello.extend_from_slice(&hello_bytes(w, h, fps));
    ch.send(&hello)?;
    println!("transmitiendo {w}x{h}");
    let mut cursor = (i32::MIN, 0, false);
    let mut st = Stats::new();
    loop {
        match src.next()? {
            Some(f) => {
                cursor = src.cursor();
                let mut hdr = vec![msg::FRAME];
                hdr.extend_from_slice(&f.ts_us.to_be_bytes());
                hdr.extend_from_slice(&cursor.0.to_be_bytes());
                hdr.extend_from_slice(&cursor.1.to_be_bytes());
                hdr.push(cursor.2 as u8);
                hdr.extend_from_slice(&(f.data.len() as u32).to_be_bytes());
                ch.send(&hdr)?;
                ch.send_chunks(&f.data)?;
                st.frames += 1;
                st.bytes += f.data.len();
                st.work += f.work_ms as f64;
            }
            None if src.cursor() != cursor => {
                cursor = src.cursor();
                let mut m = vec![msg::CURSOR];
                m.extend_from_slice(&cursor.0.to_be_bytes());
                m.extend_from_slice(&cursor.1.to_be_bytes());
                m.push(cursor.2 as u8);
                ch.send(&m)?;
            }
            None => {}
        }
        st.tick("captura+codificación", false);
    }
}

// ---------------------------------------------------------------- visor

fn view(o: &Opts) -> Res<()> {
    let target = o.pos.get(1).ok_or("uso: kast-link view <equipo>[:puerto] --key <clave>")?;
    let key = net::parse_key(o.get(&["--key", "-k"]).ok_or("falta --key (en el anfitrión: kast-link key)")?)
        .ok_or("clave inválida: deben ser 64 caracteres hexadecimales")?;
    let monitor = o.get(&["-m", "--monitor"]).and_then(|v| v.parse().ok());
    let addr_str = if target.contains(':') { target.clone() } else { format!("{target}:{DEFAULT_PORT}") };
    let addr = addr_str.to_socket_addrs()?.next().ok_or("dirección inválida")?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    let mut ch = Secure::connect(stream, &key)?;
    let mut m = Vec::new();
    ch.recv(&mut m)?;
    if m.first() != Some(&msg::HELLO) { return Err("respuesta inesperada del anfitrión".into()); }
    let (w, h, fps) = parse_hello(&m[1..]).ok_or("respuesta inesperada del anfitrión")?;
    println!("conectado a {addr}: {w}x{h} a {fps} fps (cifrado)");

    let shared = Arc::new(Mutex::new(viewer::Shared { frame: vec![0; w * h], w, h, cursor: (0, 0, false) }));
    let el = winit::event_loop::EventLoop::<viewer::Event>::with_user_event().build()?;
    let proxy = el.create_proxy();
    let s2 = shared.clone();
    std::thread::spawn(move || {
        let reason = match receive(ch, s2, &proxy, w, h, fps) { Ok(()) => "conexión cerrada".to_string(), Err(e) => e.to_string() };
        let _ = proxy.send_event(viewer::Event::Closed(reason));
    });
    if let Some(reason) = viewer::run(el, shared, monitor, format!("Kast Link — {addr}")) {
        println!("{reason}");
    }
    Ok(())
}

fn receive(mut ch: Secure, shared: Arc<Mutex<viewer::Shared>>, proxy: &winit::event_loop::EventLoopProxy<viewer::Event>,
           w: usize, h: usize, fps: u32) -> Res<()> {
    let mut dec = platform::Decoder::new(w as u32, h as u32, fps)?;
    let mut m = Vec::new();
    let mut data = Vec::new();
    let mut px = vec![0u32; w * h];
    let mut st = Stats::new();
    loop {
        ch.recv(&mut m)?;
        match m.first() {
            Some(&msg::FRAME) if m.len() >= 22 => {
                let ts = u64::from_be_bytes(m[1..9].try_into()?);
                let cx = i32::from_be_bytes(m[9..13].try_into()?);
                let cy = i32::from_be_bytes(m[13..17].try_into()?);
                let vis = m[17] != 0;
                let len = u32::from_be_bytes(m[18..22].try_into()?) as usize;
                if len > 64 << 20 { return Err("fotograma demasiado grande".into()); }
                data.clear();
                while data.len() < len {
                    ch.recv(&mut m)?;
                    if m.first() != Some(&msg::DATA) { return Err("flujo de vídeo corrupto".into()); }
                    data.extend_from_slice(&m[1..]);
                }
                let t0 = Instant::now();
                if dec.decode(&data, &mut px)? > 0 {
                    let mut s = shared.lock().unwrap();
                    std::mem::swap(&mut s.frame, &mut px);
                    s.cursor = (cx, cy, vis);
                    drop(s);
                    let _ = proxy.send_event(viewer::Event::Redraw);
                    st.frames += 1;
                    st.work += t0.elapsed().as_secs_f64() * 1e3;
                    st.lat += now_us().saturating_sub(ts) as f64 / 1000.0;
                }
                st.bytes += len;
            }
            Some(&msg::CURSOR) if m.len() >= 10 => {
                let cx = i32::from_be_bytes(m[1..5].try_into()?);
                let cy = i32::from_be_bytes(m[5..9].try_into()?);
                shared.lock().unwrap().cursor = (cx, cy, m[9] != 0);
                let _ = proxy.send_event(viewer::Event::Redraw);
            }
            _ => {}
        }
        st.tick("decodificación", true);
    }
}

// ---------------------------------------------------------------- pruebas: grabar / reproducir

/// Formato: "KLNK" + ancho, alto, fps (u32 BE) + repetido { u32 BE longitud, H.264 Annex B }.
fn record(o: &Opts) -> Res<()> {
    let path = o.pos.get(1).ok_or("uso: kast-link record [-m N | --pattern] [--frames 120] <archivo>")?;
    let frames: usize = o.num(&["--frames"], 120);
    let fps: u32 = o.num(&["--fps"], 30);
    let mut src = platform::Source::new(o.num(&["-m", "--monitor"], 1), fps, o.num(&["--mbps"], 8), o.get(&["--pattern"]).is_some())?;
    let (w, h) = src.size();
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(RECORD_MAGIC)?;
    f.write_all(&hello_bytes(w, h, fps))?;
    let (mut n, start) = (0, Instant::now());
    while n < frames {
        // Si la pantalla no cambia no hay fotogramas nuevos (sobre todo en macOS): se acepta una grabación parcial.
        if start.elapsed() > Duration::from_secs(30) {
            if n == 0 { return Err("no se capturó ningún fotograma en 30 s".into()); }
            println!("la pantalla apenas cambió: se detiene en {n} fotogramas");
            break;
        }
        if let Some(e) = src.next()? {
            f.write_all(&(e.data.len() as u32).to_be_bytes())?;
            f.write_all(&e.data)?;
            n += 1;
        }
    }
    f.flush()?;
    println!("grabados {n} fotogramas de {w}x{h} en {path}");
    Ok(())
}

fn play(o: &Opts) -> Res<()> {
    let path = o.pos.get(1).ok_or("uso: kast-link play <archivo> [--dump imagen.bmp]")?;
    let mut f = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut hdr = [0u8; 16];
    f.read_exact(&mut hdr)?;
    if &hdr[..4] != RECORD_MAGIC { return Err("no es un archivo de kast-link record".into()); }
    let (w, h, fps) = parse_hello(&hdr[4..]).unwrap();
    let mut dec = platform::Decoder::new(w as u32, h as u32, fps)?;
    let mut px = vec![0u32; w * h];
    let (mut total, mut decoded, mut data) = (0, 0, Vec::new());
    let t0 = Instant::now();
    loop {
        let mut len = [0u8; 4];
        if f.read_exact(&mut len).is_err() { break; }
        data.resize(u32::from_be_bytes(len) as usize, 0);
        f.read_exact(&mut data)?;
        total += 1;
        let got = dec.decode(&data, &mut px)?;
        decoded += got;
        if std::env::var_os("KAST_LINK_TRACE").is_some() { eprintln!("entrada {total}: {got} fotograma(s), {} bytes", data.len()); }
    }
    let ms = t0.elapsed().as_secs_f64() * 1e3 / decoded.max(1) as f64;
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for p in &px { r += (*p >> 16 & 0xFF) as u64; g += (*p >> 8 & 0xFF) as u64; b += (*p & 0xFF) as u64; }
    let n = (w * h).max(1) as u64;
    println!("{decoded}/{total} fotogramas decodificados ({w}x{h}, {ms:.1} ms cada uno); color medio del último: {} {} {}", r / n, g / n, b / n);
    if let Some(bmp) = o.get(&["--dump"]) {
        write_bmp(bmp, &px, w, h)?;
        println!("último fotograma guardado en {bmp}");
    }
    if decoded == 0 { return Err("no se decodificó ningún fotograma".into()); }
    Ok(())
}

fn write_bmp(path: &str, px: &[u32], w: usize, h: usize) -> Res<()> {
    let size = 54 + w * h * 4;
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    for v in [size as u32, 0, 54, 40, w as u32] { b.extend_from_slice(&v.to_le_bytes()); }
    b.extend_from_slice(&(-(h as i32)).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, (w * h * 4) as u32, 2835, 2835, 0, 0] { b.extend_from_slice(&v.to_le_bytes()); }
    for p in px { b.extend_from_slice(&(p | 0xFF00_0000).to_le_bytes()); }
    std::fs::write(path, b)?;
    Ok(())
}
