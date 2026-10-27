//! kast-link — ver la pantalla de otro equipo (Windows o macOS) por la red local, cifrado.
//! Lo normal es usarlo desde la app de Kast, que arranca `kast-link agent` en segundo plano.
//!
//!   kast-link agent [--port 47800]                  servicio que usa la app (stdin/stdout JSON)
//!   kast-link list                                  monitores de este equipo
//!   kast-link id                                    nombre e identidad de este equipo
//!   kast-link record [-m N | --pattern] [--frames 120] <archivo>   grabar un flujo H.264 (pruebas)
//!   kast-link play <archivo> [--dump imagen.bmp]                   decodificar un flujo grabado (pruebas)
//!   kast-link probe <ip:puerto> [--home dir] [--secs 10] [--text t] [--clip t] [--wheel x,y,dy] [--password p | --pair código]
//!                                                   cliente de iOS desde la terminal (pruebas)
//!   kast-link vdisplay <ancho> <alto> [--scale 200] [--at x,y] [--secs 5]   crear un momento una pantalla virtual (pruebas)
use kast_link::{Res, agent, mobile, platform, store, stream};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RECORD_MAGIC: &[u8; 4] = b"KLNK";

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
            let ids = platform::display_ids();
            for (i, d) in platform::displays()?.iter().enumerate() {
                let stand_in = ids.get(i).is_some_and(|id| platform::vdisplay::placeholder(*id));
                println!("{}  {}  {}x{}  en {},{}{}", i + 1, d.name, d.w, d.h, d.x, d.y, if stand_in { "  (reemplazo de macOS: no hay monitor)" } else { "" });
            }
            Ok(())
        }
        Some("id") => {
            println!("{}  {}  ({})", store::device_name(), store::identity().id(), store::platform());
            Ok(())
        }
        Some("record") => record(&o),
        Some("play") => play(&o),
        Some("probe") => probe(&o),
        Some("vdisplay") => vdisplay(&o),
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

// ---------------------------------------------------------------- pruebas: pantalla virtual

fn print_displays() {
    let (px, mons) = (platform::displays().unwrap_or_default(), platform::local_monitors());
    for (i, (id, b)) in platform::display_ids().into_iter().zip(platform::display_bounds()).enumerate() {
        let (w, h) = px.get(i).map(|d| (d.w, d.h)).unwrap_or_default();
        let scale = mons.get(i).map_or(1.0, |m| m.scale);
        println!("  {}  id {id}  en {},{}  {}x{} (escritorio)  {w}x{h} píxeles  escala {}%", i + 1, b.x, b.y, b.w, b.h, (scale * 100.0).round());
    }
}

/// Espera atendiendo el bucle principal (en macOS, los cambios de pantallas llegan por él; en el agente lo atiende
/// el bucle de ventanas).
fn pump(d: Duration) {
    #[cfg(target_os = "macos")]
    unsafe { objc2_core_foundation::CFRunLoop::run_in_mode(objc2_core_foundation::kCFRunLoopDefaultMode, d.as_secs_f64(), false); }
    #[cfg(not(target_os = "macos"))]
    std::thread::sleep(d);
}

fn vdisplay(o: &Opts) -> Res<()> {
    let w: u32 = o.pos.get(1).and_then(|v| v.parse().ok()).ok_or("falta el ancho")?;
    let h: u32 = o.pos.get(2).and_then(|v| v.parse().ok()).ok_or("falta el alto")?;
    println!("antes:");
    print_displays();
    let v = platform::vdisplay::create("Kast (prueba)", (w, h), o.num(&["--scale"], 100.0) / 100.0)?;
    if let Some((x, y)) = o.get(&["--at"]).and_then(|s| s.split_once(',')) {
        pump(Duration::from_millis(500));
        platform::vdisplay::arrange(&[(v.id, x.trim().parse()?, y.trim().parse()?)])?;
    }
    pump(Duration::from_millis(1000));
    println!("con la pantalla virtual {}:", v.id);
    print_displays();
    pump(Duration::from_secs(o.num(&["--secs"], 5)));
    drop(v);
    pump(Duration::from_secs(o.num(&["--after"], 1)));
    println!("después:");
    print_displays();
    Ok(())
}

// ---------------------------------------------------------------- pruebas: cliente de iOS

static FRAMES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static KEYFRAMES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static LIVE: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);
static ENDED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

extern "C" fn probe_event(_: *mut std::ffi::c_void, json: *const std::ffi::c_char) {
    let s = unsafe { std::ffi::CStr::from_ptr(json) }.to_string_lossy().into_owned();
    if s.contains(r#""ev":"devices""#) { return; }
    println!("{s}");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
        if v["state"] == "live" && v["control"] == true { *LIVE.lock().unwrap() = v["session"].as_u64(); }
        if v["state"] == "ended" { *ENDED.lock().unwrap() = Some(v["error"].as_str().unwrap_or("").to_string()); }
    }
}

extern "C" fn probe_frame(_: *mut std::ffi::c_void, _: u64, avcc: *const u8, len: usize, _: *const u8, _: usize, _: *const u8, _: usize) {
    use std::sync::atomic::Ordering;
    if len == 0 || avcc.is_null() { return; }
    FRAMES.fetch_add(1, Ordering::Relaxed);
    // Fotograma clave: lleva un corte IDR (NAL 5). SPS/PPS no siempre se repiten en los forzados.
    let d = unsafe { std::slice::from_raw_parts(avcc, len) };
    let mut i = 0;
    while i + 4 < d.len() {
        let n = u32::from_be_bytes(d[i..i + 4].try_into().unwrap()) as usize;
        if d[i + 4] & 0x1F == 5 { KEYFRAMES.fetch_add(1, Ordering::Relaxed); break; }
        i += 4 + n;
    }
}

/// Se conecta como la app de iOS (mobile.rs) y cuenta los fotogramas recibidos.
fn probe(o: &Opts) -> Res<()> {
    use std::ffi::CString;
    use std::sync::atomic::Ordering;
    let addr = o.pos.get(1).ok_or("uso: kast-link probe <ip:puerto> [--home dir] [--secs 10] [--text t] [--clip t]")?;
    let home = o.get(&["--home"]).map(String::from).unwrap_or_else(|| std::env::temp_dir().join("kast-probe").display().to_string());
    let secs: u64 = o.num(&["--secs"], 10);
    let (dir, name) = (CString::new(home)?, CString::new("Prueba iOS")?);
    if !unsafe { mobile::kl_start(dir.as_ptr(), name.as_ptr(), std::ptr::null_mut(), probe_event, probe_frame) } {
        return Err("no se pudo iniciar el cliente".into());
    }
    let id = CString::new(format!("ip:{addr}"))?;
    let monitor = o.num(&["-m", "--monitor"], 1);
    let session = match (o.get(&["--password"]), o.get(&["--pair"])) {
        (Some(p), _) => { let (k, s) = (CString::new("password")?, CString::new(p)?); unsafe { mobile::kl_connect_auth(id.as_ptr(), monitor, true, k.as_ptr(), s.as_ptr()) } }
        (_, Some(p)) => { let (k, s) = (CString::new("pair")?, CString::new(p)?); unsafe { mobile::kl_connect_auth(id.as_ptr(), monitor, true, k.as_ptr(), s.as_ptr()) } }
        _ => unsafe { mobile::kl_connect(id.as_ptr(), monitor, true) },
    };
    let t0 = Instant::now();
    let (mut sent, mut asked) = (false, 0);
    while t0.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(200));
        // La sesión terminó (rechazada, contraseña incorrecta…): no hay nada más que esperar.
        if let Some(e) = ENDED.lock().unwrap().clone() {
            return Err(if e.is_empty() { "la sesión terminó".into() } else { e.into() });
        }
        // Pide fotogramas clave como la app (con la pantalla quieta, macOS no manda otros por su cuenta).
        if t0.elapsed() > Duration::from_secs(4 + 4 * asked) && asked < 2 {
            mobile::kl_keyframe(session);
            asked += 1;
        }
        let live = *LIVE.lock().unwrap();
        if let (Some(s), false) = (live, sent) {
            if FRAMES.load(Ordering::Relaxed) > 0 {
                sent = true;
                if let Some(t) = o.get(&["--text"]) { let t = CString::new(t)?; unsafe { mobile::kl_text(s, t.as_ptr()) }; }
                if let Some(t) = o.get(&["--clip"]) { let t = CString::new(t)?; unsafe { mobile::kl_clip_text(s, t.as_ptr()) }; }
                // Rueda en un punto (p. ej. para comprobar el control de un Android: se convierte en un gesto).
                if let Some(w) = o.get(&["--wheel"]) {
                    let v: Vec<i32> = w.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if let [x, y, dy] = v[..] {
                        mobile::kl_mouse_move(s, x, y);
                        mobile::kl_wheel(s, 0, dy);
                    }
                }
            }
        }
    }
    let (n, k) = (FRAMES.load(Ordering::Relaxed), KEYFRAMES.load(Ordering::Relaxed));
    println!("probe: {n} fotogramas ({k} clave)");
    if n == 0 || k == 0 { return Err("no llegó vídeo utilizable".into()); }
    if k < 2 { return Err("no llegaron los fotogramas clave pedidos".into()); }
    Ok(())
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
