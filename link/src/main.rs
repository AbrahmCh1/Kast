//! kast-link — ver (y, en próximas fases, controlar) la pantalla de otro equipo por la red local.
//!
//!   kast-link list                                  monitores de este equipo
//!   kast-link host [-m N] [--fps 60] [--mbps 20] [--port 47800] [--bind 0.0.0.0]
//!   kast-link view <equipo>[:puerto] --key <clave> [-m N]
//!   kast-link key [--new]                           mostrar / regenerar la clave de este equipo
mod capture;
mod color;
mod mf;
mod net;
mod viewer;

use net::{Secure, msg};
use std::error::Error;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Res<T> = Result<T, Box<dyn Error>>;

const DEFAULT_PORT: u16 = 47800;
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    unsafe {
        use windows::Win32::UI::HiDpi::*;
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = windows::Win32::System::Console::SetConsoleOutputCP(65001);
    }
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
        let mut pos = Vec::new();
        let mut flags = Vec::new();
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            if a.starts_with('-') {
                let v = if a == "--new" { String::new() } else { it.next().unwrap_or_default() };
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
        Some("list") => list(),
        Some("host") => host(&o),
        Some("view") => view(&o),
        Some("key") => {
            let k = load_key(o.get(&["--new"]).is_some())?;
            println!("{}", net::key_hex(&k));
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
            println!("\nEn el visor: Esc cierra, F11 alterna pantalla completa.");
            Ok(())
        }
    }
}

fn key_path() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("Kast").join("link.key")
}

/// Clave del anfitrión: se genera una vez y se guarda en %APPDATA%\Kast\link.key.
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

fn list() -> Res<()> {
    for (i, o) in capture::outputs()?.iter().enumerate() {
        println!("{}  {}  {}x{}  en {},{}", i + 1, o.name.trim_start_matches(r"\\.\"), o.w, o.h, o.x, o.y);
    }
    Ok(())
}

fn now_us() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0) }

// ---------------------------------------------------------------- anfitrión

fn host(o: &Opts) -> Res<()> {
    let key = load_key(false)?;
    let n: usize = o.num(&["-m", "--monitor"], 1);
    let fps: u32 = o.num(&["--fps"], 60).clamp(1, 240);
    let mbps: u32 = o.num(&["--mbps"], 20).clamp(1, 500);
    let port: u16 = o.num(&["--port"], DEFAULT_PORT);
    let bind = o.get(&["--bind"]).unwrap_or("0.0.0.0").to_string();
    let outs = capture::outputs()?;
    let out = outs.get(n.wrapping_sub(1)).ok_or_else(|| format!("monitor {n} no existe (hay {})", outs.len()))?;
    let listener = TcpListener::bind((bind.as_str(), port))?;
    mf::startup()?;
    println!("kast-link {VERSION}: compartiendo el monitor {n} ({}x{}) en {bind}:{port}, {fps} fps, {mbps} Mbps", out.w, out.h);
    println!("clave: {}", net::key_hex(&key));
    loop {
        let (stream, addr) = listener.accept()?;
        println!("conexión de {addr}");
        match serve(stream, &key, out, fps, mbps) {
            Ok(()) => println!("visor desconectado"),
            Err(e) => println!("sesión terminada: {e}"),
        }
    }
}

fn serve(stream: TcpStream, key: &net::Key, out: &capture::Output, fps: u32, mbps: u32) -> Res<()> {
    let mut ch = Secure::accept(stream, key)?;
    let mut cap = capture::Capturer::new(out)?;
    // El primer AcquireNextFrame siempre trae la imagen completa del escritorio.
    let mut first = None;
    for _ in 0..50 {
        if let Some(f) = cap.next(100)? { first = Some((f.data.to_vec(), f.pitch)); break; }
    }
    let (mut last, mut pitch) = first.ok_or("no se pudo capturar el monitor")?;
    let (w, h) = ((cap.w & !1) as usize, (cap.h & !1) as usize);
    let mut hello = vec![msg::HELLO];
    for v in [w as u32, h as u32, fps] { hello.extend_from_slice(&v.to_be_bytes()); }
    ch.send(&hello)?;

    let mut enc = mf::Encoder::new(w as u32, h as u32, fps, mbps * 1_000_000)?;
    let mut nv12 = vec![0u8; w * h * 3 / 2];
    let period = Duration::from_secs_f64(1.0 / fps as f64);
    let mut next = Instant::now();
    let mut cursor = (i32::MIN, 0, false);
    let mut pending = true;   // hay imagen nueva sin codificar
    let mut refine = 0;       // fotogramas extra al quedarse quieta la pantalla (mejoran la nitidez)
    let (mut st_t, mut st_frames, mut st_bytes, mut st_conv, mut st_enc) = (Instant::now(), 0u32, 0usize, 0f64, 0f64);

    loop {
        let wait = next.saturating_duration_since(Instant::now()).as_millis() as u32;
        if let Some(f) = cap.next(wait.max(1))? {
            last.clear();
            last.extend_from_slice(f.data);
            pitch = f.pitch;
            pending = true;
            refine = 3;
        }
        if Instant::now() < next { continue; }
        next += period;
        if Instant::now() > next + period * 4 { next = Instant::now() + period; }

        if pending || refine > 0 {
            if !pending { refine -= 1; }
            pending = false;
            let t0 = Instant::now();
            color::bgra_to_nv12(&last, pitch, w, h, &mut nv12);
            let t1 = Instant::now();
            let data = enc.encode(&nv12)?;
            let t2 = Instant::now();
            if !data.is_empty() {
                let mut hdr = vec![msg::FRAME];
                hdr.extend_from_slice(&now_us().to_be_bytes());
                hdr.extend_from_slice(&cap.cursor.0.to_be_bytes());
                hdr.extend_from_slice(&cap.cursor.1.to_be_bytes());
                hdr.push(cap.cursor.2 as u8);
                hdr.extend_from_slice(&(data.len() as u32).to_be_bytes());
                ch.send(&hdr)?;
                ch.send_chunks(&data)?;
                cursor = cap.cursor;
                st_frames += 1;
                st_bytes += data.len();
                st_conv += (t1 - t0).as_secs_f64();
                st_enc += (t2 - t1).as_secs_f64();
            }
        } else if cap.cursor != cursor {
            cursor = cap.cursor;
            let mut m = vec![msg::CURSOR];
            m.extend_from_slice(&cursor.0.to_be_bytes());
            m.extend_from_slice(&cursor.1.to_be_bytes());
            m.push(cursor.2 as u8);
            ch.send(&m)?;
        }

        let el = st_t.elapsed().as_secs_f64();
        if el >= 2.0 {
            let f = st_frames.max(1) as f64;
            println!("{:5.1} fps  {:6.2} Mbps  conversión {:4.1} ms  codificación {:4.1} ms",
                     st_frames as f64 / el, st_bytes as f64 * 8.0 / el / 1e6, st_conv / f * 1e3, st_enc / f * 1e3);
            (st_t, st_frames, st_bytes, st_conv, st_enc) = (Instant::now(), 0, 0, 0.0, 0.0);
        }
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
    if m.len() < 13 || m[0] != msg::HELLO { return Err("respuesta inesperada del anfitrión".into()); }
    let rd = |i: usize| u32::from_be_bytes(m[i..i + 4].try_into().unwrap());
    let (w, h, fps) = (rd(1) as usize, rd(5) as usize, rd(9));
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
    mf::startup()?;
    let mut dec = mf::Decoder::new(w as u32, h as u32, fps)?;
    let mut m = Vec::new();
    let mut data = Vec::new();
    let mut px = vec![0u32; w * h];
    let (mut st_t, mut st_frames, mut st_bytes, mut st_dec, mut st_lat) = (Instant::now(), 0u32, 0usize, 0f64, 0f64);
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
                if let Some(f) = dec.decode(&data)? {
                    color::nv12_to_xrgb(f.data, f.pitch, f.uv_offset, w, h, &mut px);
                    let mut s = shared.lock().unwrap();
                    std::mem::swap(&mut s.frame, &mut px);
                    s.cursor = (cx, cy, vis);
                    drop(s);
                    let _ = proxy.send_event(viewer::Event::Redraw);
                    st_frames += 1;
                    st_dec += t0.elapsed().as_secs_f64();
                    st_lat += now_us().saturating_sub(ts) as f64 / 1000.0;
                }
                st_bytes += len;
            }
            Some(&msg::CURSOR) if m.len() >= 10 => {
                let cx = i32::from_be_bytes(m[1..5].try_into()?);
                let cy = i32::from_be_bytes(m[5..9].try_into()?);
                shared.lock().unwrap().cursor = (cx, cy, m[9] != 0);
                let _ = proxy.send_event(viewer::Event::Redraw);
            }
            _ => {}
        }
        let el = st_t.elapsed().as_secs_f64();
        if el >= 2.0 {
            let f = st_frames.max(1) as f64;
            println!("{:5.1} fps  {:6.2} Mbps  decodificación {:4.1} ms  latencia {:5.1} ms",
                     st_frames as f64 / el, st_bytes as f64 * 8.0 / el / 1e6, st_dec / f * 1e3, st_lat / f);
            (st_t, st_frames, st_bytes, st_dec, st_lat) = (Instant::now(), 0, 0, 0.0, 0.0);
        }
    }
}
