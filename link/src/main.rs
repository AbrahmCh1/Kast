//! kast-link — ver la pantalla de otro equipo (Windows o macOS) por la red local, cifrado.
//! Lo normal es usarlo desde la app de Kast, que arranca `kast-link agent` en segundo plano.
//!
//!   kast-link agent [--port 47800]                  servicio que usa la app (stdin/stdout JSON)
//!   kast-link list                                  monitores de este equipo
//!   kast-link id                                    nombre e identidad de este equipo
//!   kast-link record [-m N | --pattern] [--frames 120] <archivo>   grabar un flujo H.264 (pruebas)
//!   kast-link play <archivo> [--dump imagen.bmp]                   decodificar un flujo grabado (pruebas)
mod agent;
mod discovery;
mod h264;
mod input;
mod net;
mod share;
mod store;
mod stream;
mod viewer;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac as platform;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as platform;

use std::error::Error;
use std::io::{Read, Write};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub type Res<T> = Result<T, Box<dyn Error>>;

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
        const SWITCHES: [&str; 1] = ["--pattern"];
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
        Some("agent") => agent::run(o.num(&["--port"], agent::DEFAULT_PORT)),
        Some("list") => {
            for (i, d) in platform::displays()?.iter().enumerate() {
                println!("{}  {}  {}x{}  en {},{}", i + 1, d.name, d.w, d.h, d.x, d.y);
            }
            Ok(())
        }
        Some("id") => {
            println!("{}  {}  ({})", store::device_name(), store::identity().id(), store::platform());
            Ok(())
        }
        Some("record") => record(&o),
        Some("play") => play(&o),
        Some("version") | Some("--version") => { println!("kast-link {VERSION}"); Ok(()) }
        _ => {
            println!("kast-link {VERSION} — pantalla de otro equipo por la red local (cifrado)\n");
            println!("Se usa desde la app de Kast: sección Dispositivos.\n");
            println!("  kast-link agent [--port 47800]                   servicio que arranca la app");
            println!("  kast-link list                                   monitores de este equipo");
            println!("  kast-link id                                     nombre e identidad de este equipo");
            println!("  kast-link record [-m N | --pattern] [--frames 120] <archivo>   grabar un flujo (pruebas)");
            println!("  kast-link play <archivo> [--dump imagen.bmp]     decodificar un flujo grabado (pruebas)");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------- pruebas: grabar / reproducir

/// Formato: "KLNK" + ancho, alto, fps (u32 BE) + repetido { u32 BE longitud, H.264 Annex B }.
fn record(o: &Opts) -> Res<()> {
    let path = o.pos.get(1).ok_or("uso: kast-link record [-m N | --pattern] [--frames 120] <archivo>")?;
    let frames: usize = o.num(&["--frames"], 120);
    let fps: u32 = o.num(&["--fps"], 30);
    let mut src = platform::Source::new(o.num(&["-m", "--monitor"], 1), fps, o.num(&["--mbps"], 8), o.get(&["--pattern"]).is_some(), true)?;
    let (w, h) = src.size();
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(RECORD_MAGIC)?;
    f.write_all(&stream::hello_bytes(w, h, fps))?;
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
    let (w, h, fps) = stream::parse_hello(&hdr[4..]).ok_or("cabecera inválida")?;
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
