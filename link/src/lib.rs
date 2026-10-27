//! Núcleo de Kast Link: protocolo cifrado, descubrimiento, vídeo y entrada. Lo usan el binario
//! `kast-link` (agente de escritorio en Windows y macOS) y la app de iOS (biblioteca estática, ver mobile.rs).
pub mod access;
pub mod discovery;
pub mod h264;
pub mod input;
pub mod net;
pub mod share;
pub mod store;
pub mod virt;

#[cfg(not(target_os = "ios"))]
pub mod agent;
#[cfg(not(target_os = "ios"))]
pub mod stream;
#[cfg(not(target_os = "ios"))]
pub mod viewer;

/// Cliente de la app de iOS (Rust puro: también se compila en escritorio para probarlo con `kast-link probe`).
pub mod mobile;

#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_os = "macos")]
pub use mac as platform;
#[cfg(windows)]
pub mod win;
#[cfg(windows)]
pub use win as platform;

use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Res<T> = Result<T, Box<dyn Error>>;

/// Monitor de este equipo (numerados de izquierda a derecha, como en Kast).
pub struct DisplayInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Monitor de este equipo como destino de una pantalla remota: rectángulo en el escritorio (en las unidades del
/// sistema: píxeles en Windows, puntos en macOS), píxeles reales y escala (1.0, 1.5, 2.0…).
pub struct LocalMon {
    pub rect: virt::Rect,
    pub px: (u32, u32),
    pub scale: f64,
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
#[cfg_attr(not(windows), allow(dead_code))]
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
