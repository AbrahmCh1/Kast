//! Plataforma Windows: captura DXGI + H.264 de Media Foundation.
mod capture;
mod color;
mod input;
mod mf;

pub use input::Injector;

use crate::{DisplayInfo, Encoded, Res, now_us, pattern};
use std::time::{Duration, Instant};

pub fn init() {
    unsafe {
        use windows::Win32::UI::HiDpi::*;
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = windows::Win32::System::Console::SetConsoleOutputCP(65001);
    }
}

/// En Windows capturar la pantalla no requiere permiso.
pub fn capture_allowed() -> Option<bool> { None }

pub fn request_capture() {}

/// Tampoco controlar el teclado y el ratón (SendInput).
pub fn input_allowed() -> Option<bool> { None }

pub fn request_input() {}

pub fn displays() -> Res<Vec<DisplayInfo>> {
    Ok(capture::outputs()?
        .iter()
        .map(|o| DisplayInfo { name: o.name.trim_start_matches(r"\\.\").to_string(), x: o.x, y: o.y, w: o.w, h: o.h })
        .collect())
}

/// Captura + codificación de un monitor (o del patrón de prueba).
pub struct Source {
    cap: Option<capture::Capturer>,
    enc: mf::Encoder,
    w: usize,
    h: usize,
    frame: Vec<u8>,
    pitch: usize,
    nv12: Vec<u8>,
    period: Duration,
    next: Instant,
    pending: bool,
    refine: u8,
    n: u64,
}

impl Source {
    /// show_cursor no aplica: en Windows el cursor viaja aparte y el visor decide si lo dibuja.
    pub fn new(index: usize, fps: u32, mbps: u32, use_pattern: bool, _show_cursor: bool) -> Res<Self> {
        mf::startup()?;
        let (cap, frame, pitch, w, h) = if use_pattern {
            let (w, h) = (1280usize, 720usize);
            (None, vec![0u8; w * h * 4], w * 4, w, h)
        } else {
            let outs = capture::outputs()?;
            let out = outs.get(index.wrapping_sub(1)).ok_or_else(|| format!("monitor {index} no existe (hay {})", outs.len()))?;
            let mut cap = capture::Capturer::new(out)?;
            // El primer AcquireNextFrame siempre trae la imagen completa del escritorio.
            let mut first = None;
            for _ in 0..50 {
                if let Some(f) = cap.next(100)? { first = Some((f.data.to_vec(), f.pitch)); break; }
            }
            let (frame, pitch) = first.ok_or("no se pudo capturar el monitor")?;
            let (w, h) = ((cap.w & !1) as usize, (cap.h & !1) as usize);
            (Some(cap), frame, pitch, w, h)
        };
        let enc = mf::Encoder::new(w as u32, h as u32, fps, mbps * 1_000_000)?;
        let period = Duration::from_secs_f64(1.0 / fps as f64);
        Ok(Self { cap, enc, w, h, frame, pitch, nv12: vec![0; w * h * 3 / 2], period, next: Instant::now(), pending: true, refine: 0, n: 0 })
    }

    pub fn size(&self) -> (u32, u32) { (self.w as u32, self.h as u32) }

    pub fn cursor(&self) -> (i32, i32, bool) { self.cap.as_ref().map_or((0, 0, false), |c| c.cursor) }

    /// Espera al siguiente intervalo. Some = fotograma codificado; None = sin imagen nueva
    /// (el cursor puede haberse movido: ver `cursor()`).
    pub fn next(&mut self) -> Res<Option<Encoded>> {
        loop {
            match &mut self.cap {
                None => {
                    let wait = self.next.saturating_duration_since(Instant::now());
                    std::thread::sleep(wait);
                    pattern(self.n, self.w, self.h, &mut self.frame);
                    self.pending = true;
                }
                Some(cap) => {
                    let wait = self.next.saturating_duration_since(Instant::now()).as_millis() as u32;
                    if let Some(f) = cap.next(wait.max(1))? {
                        self.frame.clear();
                        self.frame.extend_from_slice(f.data);
                        self.pitch = f.pitch;
                        self.pending = true;
                        self.refine = 3; // al quedarse quieta, unos fotogramas extra afinan la nitidez
                    }
                    if Instant::now() < self.next { continue; }
                }
            }
            self.next += self.period;
            if Instant::now() > self.next + self.period * 4 { self.next = Instant::now() + self.period; }
            if !self.pending && self.refine == 0 { return Ok(None); }
            if !self.pending { self.refine -= 1; }
            self.pending = false;
            let t0 = Instant::now();
            let ts_us = now_us();
            color::bgra_to_nv12(&self.frame, self.pitch, self.w, self.h, &mut self.nv12);
            let data = self.enc.encode(&self.nv12)?;
            self.n += 1;
            if data.is_empty() { return Ok(None); }
            return Ok(Some(Encoded { data, ts_us, work_ms: t0.elapsed().as_secs_f32() * 1e3 }));
        }
    }
}

pub struct Decoder {
    dec: mf::Decoder,
    w: usize,
    h: usize,
}

impl Decoder {
    pub fn new(w: u32, h: u32, fps: u32) -> Res<Self> {
        mf::startup()?;
        Ok(Self { dec: mf::Decoder::new(w, h, fps)?, w: w as usize, h: h as usize })
    }

    /// Decodifica una unidad de acceso Annex B; escribe el último fotograma en `out` (0RGB) y
    /// devuelve cuántos salieron.
    pub fn decode(&mut self, data: &[u8], out: &mut [u32]) -> Res<usize> {
        // Flujos de otros codificadores (VideoToolbox) sin "0 fotogramas reordenados": se corrige el SPS
        // para que Media Foundation no retenga ~5 fotogramas.
        let fixed = crate::h264::low_latency_annexb(data);
        let (n, frame) = self.dec.decode(fixed.as_deref().unwrap_or(data))?;
        if let Some(f) = frame {
            color::nv12_to_xrgb(f.data, f.pitch, f.uv_offset, self.w, self.h, out);
        }
        Ok(n)
    }
}
