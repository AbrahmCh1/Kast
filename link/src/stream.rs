//! Envío y recepción del vídeo de una sesión ya aceptada.
use crate::net::{Secure, msg};
use crate::{Res, now_us, platform, viewer};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub fn hello_bytes(w: u32, h: u32, fps: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(12);
    for x in [w, h, fps] { v.extend_from_slice(&x.to_be_bytes()); }
    v
}

pub fn parse_hello(b: &[u8]) -> Option<(usize, usize, u32)> {
    if b.len() < 12 { return None; }
    let rd = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
    let (w, h, fps) = (rd(0) as usize, rd(4) as usize, rd(8));
    (w > 0 && h > 0 && w <= 8192 && h <= 8192).then_some((w, h, fps))
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
        if el < 5.0 { return; }
        let f = self.frames.max(1) as f64;
        let lat = if latency { format!("  latencia {:5.1} ms", self.lat / f) } else { String::new() };
        eprintln!("{:5.1} fps  {:6.2} Mbps  {label} {:4.1} ms{lat}", self.frames as f64 / el, self.bytes as f64 * 8.0 / el / 1e6, self.work / f);
        *self = Self::new();
    }
}

/// Prepara la captura del monitor (antes de aceptar la conexión, para poder explicar si falla).
pub fn open(monitor: usize, fps: u32, mbps: u32, show_cursor: bool) -> Res<platform::Source> {
    platform::Source::new(monitor, fps, mbps, false, show_cursor)
}

/// Anfitrión: transmite el monitor hasta que se corte la conexión.
pub fn serve(ch: &mut Secure, mut src: platform::Source, fps: u32) -> Res<()> {
    let (w, h) = src.size();
    ch.send_typed(msg::HELLO, &hello_bytes(w, h, fps))?;
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

/// Visor: decodifica los fotogramas en `shared` y avisa con `redraw` tras cada uno.
pub fn receive(ch: &mut Secure, shared: &Arc<Mutex<viewer::Shared>>, w: usize, h: usize, fps: u32, redraw: &dyn Fn()) -> Res<()> {
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
                    redraw();
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
                redraw();
            }
            _ => {}
        }
        st.tick("decodificación", true);
    }
}
