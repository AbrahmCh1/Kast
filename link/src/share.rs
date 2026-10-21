//! Durante una sesión con control: portapapeles compartido (en los dos sentidos) y envío de archivos
//! (del visor al equipo controlado, arrastrándolos a la ventana).
//! En iOS sólo se usan el formato de los mensajes y el envío de archivos: su portapapeles lo gestiona la app.
#![cfg_attr(target_os = "ios", allow(dead_code, unused_imports))]
use crate::net::msg;
use crate::Res;
#[cfg(not(target_os = "ios"))]
use crate::platform;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Contenido del portapapeles que se comparte.
#[derive(Hash)]
pub enum Clip {
    Text(String),
    /// Imagen en PNG (formato común a Windows y macOS).
    Png(Vec<u8>),
}

const MAX_CLIP: usize = 32 << 20;
/// Trozo de datos por mensaje (cabe en un mensaje Noise con su cabecera).
const CHUNK: usize = 60_000;

fn hash_of(c: &Clip) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    c.hash(&mut h);
    h.finish()
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn png_encode(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(out)
}

/// PNG -> (ancho, alto, RGBA de 8 bits).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn png_decode(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut d = png::Decoder::new(std::io::Cursor::new(data));
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((w, h, rgba))
}

// ---------------------------------------------------------------- portapapeles

#[cfg(not(target_os = "ios"))]
struct ClipState {
    /// Último número de secuencia visto (o puesto por nosotros: no se reenvía).
    seen: u64,
    /// Huella del último contenido enviado o recibido: evita rebotes entre los dos equipos.
    last: u64,
    /// Contenido que está llegando: tipo, tamaño total y datos.
    rx: Option<(u8, usize, Vec<u8>)>,
}

#[cfg(not(target_os = "ios"))]
/// Sincroniza el portapapeles con el otro equipo mientras exista.
pub struct ClipSync {
    st: Arc<Mutex<ClipState>>,
    stop: Arc<AtomicBool>,
}

#[cfg(not(target_os = "ios"))]
impl ClipSync {
    /// `send` envía un mensaje al otro equipo (false si la sesión terminó).
    pub fn start(send: impl Fn(Vec<u8>) -> bool + Send + 'static) -> Self {
        let st = Arc::new(Mutex::new(ClipState { seen: platform::clipboard::seq(), last: 0, rx: None }));
        let stop = Arc::new(AtomicBool::new(false));
        let (st2, stop2) = (st.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(300));
                let seq = platform::clipboard::seq();
                {
                    let mut s = st2.lock().unwrap();
                    if seq == s.seen { continue; }
                    s.seen = seq;
                }
                let Some(c) = platform::clipboard::get() else { continue };
                let (kind, data): (u8, &[u8]) = match &c { Clip::Text(t) => (1, t.as_bytes()), Clip::Png(p) => (2, p) };
                if data.is_empty() || data.len() > MAX_CLIP { continue; }
                {
                    let mut s = st2.lock().unwrap();
                    let h = hash_of(&c);
                    if h == s.last { continue; }
                    s.last = h;
                }
                let mut head = vec![msg::CLIP, kind];
                head.extend_from_slice(&(data.len() as u32).to_be_bytes());
                if !send(head) { return; }
                for part in data.chunks(CHUNK) {
                    let mut m = Vec::with_capacity(part.len() + 1);
                    m.push(msg::CLIP_DATA);
                    m.extend_from_slice(part);
                    if !send(m) { return; }
                }
            }
        });
        Self { st, stop }
    }

    /// Mensaje CLIP o CLIP_DATA del otro equipo: al completarse, se pone en el portapapeles de aquí.
    pub fn on_message(&self, m: &[u8]) {
        let mut s = self.st.lock().unwrap();
        match m.first() {
            Some(&msg::CLIP) if m.len() >= 6 => {
                let total = u32::from_be_bytes(m[2..6].try_into().unwrap()) as usize;
                s.rx = (total <= MAX_CLIP).then(|| (m[1], total, Vec::with_capacity(total)));
            }
            Some(&msg::CLIP_DATA) => {
                if let Some((_, total, data)) = &mut s.rx {
                    data.extend_from_slice(&m[1..]);
                    if data.len() > *total { s.rx = None; }
                }
            }
            _ => return,
        }
        if !matches!(&s.rx, Some((_, total, data)) if data.len() == *total) { return; }
        let (kind, _, data) = s.rx.take().unwrap();
        let clip = match kind {
            1 => Clip::Text(String::from_utf8_lossy(&data).into_owned()),
            2 => Clip::Png(data),
            _ => return,
        };
        s.last = hash_of(&clip);
        // Con el cerrojo tomado: el hilo que vigila no verá el cambio como propio del usuario.
        platform::clipboard::set(&clip);
        s.seen = platform::clipboard::seq();
    }
}

#[cfg(not(target_os = "ios"))]
impl Drop for ClipSync {
    fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); }
}

// ---------------------------------------------------------------- archivos

/// Archivos a enviar: (ruta local, nombre relativo con '/'). Las carpetas se recorren enteras.
fn collect(paths: &[PathBuf]) -> Vec<(PathBuf, String, u64)> {
    fn walk(p: &Path, rel: String, out: &mut Vec<(PathBuf, String, u64)>) {
        let Ok(md) = std::fs::metadata(p) else { return };
        if md.is_dir() {
            let Ok(rd) = std::fs::read_dir(p) else { return };
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries { walk(&e.path(), format!("{rel}/{}", e.file_name().to_string_lossy()), out); }
        } else if md.is_file() {
            out.push((p.to_path_buf(), rel, md.len()));
        }
    }
    let mut out = Vec::new();
    for p in paths {
        if let Some(n) = p.file_name() { walk(p, n.to_string_lossy().into_owned(), &mut out); }
    }
    out
}

/// Envía archivos y carpetas. `progress` recibe el texto de estado (None al terminar).
/// Devuelve (archivos, bytes) enviados.
pub fn send_files(paths: &[PathBuf], send: &mut dyn FnMut(&[u8]) -> std::io::Result<()>, progress: &dyn Fn(Option<String>)) -> Res<(usize, u64)> {
    let files = collect(paths);
    if files.is_empty() { return Err("no hay archivos que enviar".into()); }
    let total: u64 = files.iter().map(|f| f.2).sum();
    let (mut sent, mut shown) = (0u64, Instant::now() - Duration::from_secs(1));
    let mut buf = vec![0u8; CHUNK];
    for (i, (path, rel, size)) in files.iter().enumerate() {
        let id = i as u32;
        let mut head = vec![msg::FILE];
        head.extend_from_slice(json!({"id": id, "name": rel, "size": size, "last": i + 1 == files.len()}).to_string().as_bytes());
        send(&head)?;
        let mut f = std::fs::File::open(path).map_err(|e| format!("{rel}: {e}"))?;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 { break; }
            let mut m = Vec::with_capacity(n + 5);
            m.push(msg::FILE_DATA);
            m.extend_from_slice(&id.to_be_bytes());
            m.extend_from_slice(&buf[..n]);
            send(&m)?;
            sent += n as u64;
            if shown.elapsed() > Duration::from_millis(250) {
                shown = Instant::now();
                let pct = if total > 0 { sent * 100 / total } else { 100 };
                let what = if files.len() == 1 { rel.clone() } else { format!("{} de {} archivos", i + 1, files.len()) };
                progress(Some(format!("Enviando {what} · {pct} %")));
            }
        }
        let mut end = vec![msg::FILE_END];
        end.extend_from_slice(&id.to_be_bytes());
        send(&end)?;
    }
    progress(None);
    Ok((files.len(), total))
}

/// Archivos recibidos de una tanda completa.
pub struct Received {
    pub count: usize,
    pub bytes: u64,
    /// Lo primero que se recibió (archivo o carpeta), para mostrarlo.
    pub first: PathBuf,
}

/// Recibe archivos en la carpeta de descargas (sin sobrescribir: "nombre (2).ext").
pub struct FileRecv {
    dir: PathBuf,
    open: HashMap<u32, (std::fs::File, PathBuf, bool)>,
    /// Primer componente de la ruta recibida -> nombre libre elegido en esta tanda.
    roots: HashMap<String, String>,
    first: Option<PathBuf>,
    count: usize,
    bytes: u64,
}

/// Componente de ruta seguro: sin separadores, "..", unidades ni caracteres que Windows no admite.
fn clean(part: &str) -> Option<String> {
    let s: String = part.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    (!s.is_empty() && s != "." && s != "..").then_some(s)
}

fn unique(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() { return name.to_string(); }
    let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name, "") };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|c| !dir.join(c).exists()).unwrap()
}

#[cfg(not(target_os = "ios"))]
impl FileRecv {
    pub fn new() -> Self {
        Self { dir: downloads_dir(), open: HashMap::new(), roots: HashMap::new(), first: None, count: 0, bytes: 0 }
    }

    /// Mensaje FILE, FILE_DATA o FILE_END. Devuelve la tanda cuando llega su último archivo.
    pub fn on_message(&mut self, m: &[u8]) -> Res<Option<Received>> {
        match m.first() {
            Some(&msg::FILE) => {
                let v: Value = serde_json::from_slice(&m[1..])?;
                let id = v.get("id").and_then(Value::as_u64).ok_or("archivo sin id")? as u32;
                let last = v.get("last").and_then(Value::as_bool).unwrap_or(true);
                let parts: Vec<String> = v.get("name").and_then(Value::as_str).unwrap_or("").split('/').filter_map(clean).collect();
                let (root, rest) = parts.split_first().ok_or("nombre de archivo inválido")?;
                let _ = std::fs::create_dir_all(&self.dir);
                let dir = self.dir.clone();
                let root = self.roots.entry(root.clone()).or_insert_with(|| unique(&dir, root)).clone();
                let mut path = self.dir.join(&root);
                for p in rest { path.push(p); }
                if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
                let f = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                self.first.get_or_insert_with(|| self.dir.join(&root));
                self.open.insert(id, (f, path, last));
            }
            Some(&msg::FILE_DATA) if m.len() >= 5 => {
                let id = u32::from_be_bytes(m[1..5].try_into()?);
                if let Some((f, _, _)) = self.open.get_mut(&id) {
                    f.write_all(&m[5..])?;
                    self.bytes += (m.len() - 5) as u64;
                }
            }
            Some(&msg::FILE_END) if m.len() >= 5 => {
                let id = u32::from_be_bytes(m[1..5].try_into()?);
                if let Some((f, _, last)) = self.open.remove(&id) {
                    drop(f);
                    self.count += 1;
                    if last {
                        let done = Received { count: self.count, bytes: self.bytes, first: self.first.take().unwrap_or_else(|| self.dir.clone()) };
                        self.roots.clear();
                        self.count = 0;
                        self.bytes = 0;
                        return Ok(Some(done));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }
}

/// Carpeta de descargas (KAST_LINK_DOWNLOADS para pruebas).
#[cfg(not(target_os = "ios"))]
pub fn downloads_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("KAST_LINK_DOWNLOADS") { return d.into(); }
    platform::downloads_dir()
}

/// Muestra lo recibido en el Explorador / Finder (no en las pruebas).
#[cfg(not(target_os = "ios"))]
pub fn reveal(p: &Path) {
    if std::env::var_os("KAST_LINK_DOWNLOADS").is_some() { return; }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
    }
    #[cfg(not(windows))]
    { let _ = std::process::Command::new("open").arg("-R").arg(p).spawn(); }
}

pub fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_ida_y_vuelta() {
        let rgba: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let png = png_encode(4, 3, &rgba).unwrap();
        assert_eq!(png_decode(&png).unwrap(), (4, 3, rgba));
    }

    #[cfg(not(target_os = "ios"))]
    /// Portapapeles real del sistema: sólo con KAST_TEST_CLIPBOARD=1 (en la CI; aquí sobrescribiría el del usuario).
    #[test]
    fn portapapeles_real() {
        if std::env::var_os("KAST_TEST_CLIPBOARD").is_none() { return; }
        let antes = platform::clipboard::seq();
        platform::clipboard::set(&Clip::Text("Kast ñ\nlínea 2".into()));
        assert_ne!(platform::clipboard::seq(), antes);
        match platform::clipboard::get() {
            // Windows guarda los saltos de línea como \r\n.
            Some(Clip::Text(t)) => assert_eq!(t.replace("\r\n", "\n"), "Kast ñ\nlínea 2"),
            _ => panic!("no se leyó el texto"),
        }
        let rgba: Vec<u8> = (0..8 * 6).flat_map(|i| [i as u8 * 5, 100, 200, 255]).collect();
        let png = png_encode(8, 6, &rgba).unwrap();
        platform::clipboard::set(&Clip::Png(png));
        match platform::clipboard::get() {
            Some(Clip::Png(p)) => assert_eq!(png_decode(&p).unwrap(), (8, 6, rgba)),
            _ => panic!("no se leyó la imagen"),
        }
    }

    #[test]
    fn nombres_seguros() {
        assert_eq!(clean("a:b?.txt").as_deref(), Some("a_b_.txt"));
        assert_eq!(clean(".."), None);
        assert_eq!(clean("  "), None);
        assert_eq!(clean("nota. "), Some("nota".into()));
    }

    #[cfg(not(target_os = "ios"))]
    #[test]
    fn archivos_ida_y_vuelta() {
        let base = std::env::temp_dir().join(format!("kast-share-{}", std::process::id()));
        let (src, dst) = (base.join("src"), base.join("dst"));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(src.join("carpeta/sub")).unwrap();
        std::fs::write(src.join("carpeta/a.txt"), b"hola").unwrap();
        std::fs::write(src.join("carpeta/sub/b.bin"), vec![7u8; 200_000]).unwrap();
        std::fs::create_dir_all(dst.join("carpeta")).unwrap(); // ya existe: se recibe como "carpeta (2)"
        let mut rx = FileRecv { dir: dst.clone(), open: HashMap::new(), roots: HashMap::new(), first: None, count: 0, bytes: 0 };
        let mut done = None;
        let (n, bytes) = send_files(&[src.join("carpeta")], &mut |m| { if let Some(d) = rx.on_message(m).unwrap() { done = Some(d); } Ok(()) }, &|_| {}).unwrap();
        assert_eq!((n, bytes), (2, 200_004));
        let d = done.unwrap();
        assert_eq!((d.count, d.bytes), (2, 200_004));
        assert_eq!(d.first, dst.join("carpeta (2)"));
        assert_eq!(std::fs::read(dst.join("carpeta (2)/a.txt")).unwrap(), b"hola");
        assert_eq!(std::fs::read(dst.join("carpeta (2)/sub/b.bin")).unwrap().len(), 200_000);
        let _ = std::fs::remove_dir_all(&base);
    }
}
