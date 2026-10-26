//! Portapapeles de Windows: texto (CF_UNICODETEXT) e imágenes (formato "PNG" registrado, o CF_DIB convertido).
use crate::share::{Clip, png_decode, png_encode};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE};
use windows::core::w;

/// Cambia cada vez que alguien escribe en el portapapeles.
pub fn seq() -> u64 { unsafe { GetClipboardSequenceNumber() as u64 } }

/// Sacar archivos de Windows arrastrándolos hacia otro equipo aún no está disponible (no hay un portapapeles de
/// arrastre que consultar): nunca hay archivos.
pub fn drag_seq() -> i64 { 0 }
pub fn dragged_files(_since: i64) -> Vec<std::path::PathBuf> { Vec::new() }

fn png_format() -> u32 { unsafe { RegisterClipboardFormatW(w!("PNG")) } }

/// Abre el portapapeles (otra app puede tenerlo abierto un momento: se reintenta).
fn open(owner: Option<HWND>) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(owner) }.is_ok() { return true; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

/// Copia del bloque de memoria de un formato (con el portapapeles abierto).
fn read(format: u32) -> Option<Vec<u8>> {
    unsafe {
        IsClipboardFormatAvailable(format).ok()?;
        let h = GetClipboardData(format).ok()?;
        let g = HGLOBAL(h.0);
        let p = GlobalLock(g) as *const u8;
        if p.is_null() { return None; }
        let v = std::slice::from_raw_parts(p, GlobalSize(g)).to_vec();
        let _ = GlobalUnlock(g);
        Some(v)
    }
}

pub fn get() -> Option<Clip> {
    if !open(None) { return None; }
    let r = (|| {
        if let Some(b) = read(CF_UNICODETEXT.0 as u32) {
            let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            return Some(Clip::Text(String::from_utf16_lossy(&w)));
        }
        if let Some(png) = read(png_format()) { return Some(Clip::Png(png)); }
        read(CF_DIB.0 as u32).and_then(|d| dib_to_png(&d)).map(Clip::Png)
    })();
    unsafe { let _ = CloseClipboard(); }
    r
}

fn put(format: u32, data: &[u8]) {
    unsafe {
        let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, data.len()) else { return };
        let p = GlobalLock(g) as *mut u8;
        if p.is_null() { return; }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        let _ = GlobalUnlock(g);
        // El sistema se queda con la memoria si SetClipboardData funciona.
        let _ = SetClipboardData(format, Some(HANDLE(g.0)));
    }
}

pub fn set(c: &Clip) {
    // Para escribir hace falta una ventana propietaria (con NULL, SetClipboardData puede fallar).
    let Ok(owner) = (unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None) }) else { return };
    if open(Some(owner)) {
        unsafe { let _ = EmptyClipboard(); }
        match c {
            Clip::Text(t) => {
                let t = t.replace("\r\n", "\n").replace('\n', "\r\n");
                let mut b: Vec<u8> = t.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
                b.shrink_to_fit();
                put(CF_UNICODETEXT.0 as u32, &b);
            }
            Clip::Png(png) => {
                put(png_format(), png);
                if let Some(dib) = png_to_dib(png) { put(CF_DIB.0 as u32, &dib); }
            }
        }
        unsafe { let _ = CloseClipboard(); }
    }
    unsafe { let _ = DestroyWindow(owner); }
}

/// CF_DIB (BITMAPINFOHEADER + píxeles, 24 o 32 bits) -> PNG.
fn dib_to_png(d: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| d.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let hsize = u32_at(0)? as usize;
    let w = u32_at(4)? as i32;
    let h = u32_at(8)? as i32;
    let bpp = u16::from_le_bytes(d.get(14..16)?.try_into().ok()?);
    let comp = u32_at(16)?;
    if w <= 0 || h == 0 || !(bpp == 24 || bpp == 32) || !(comp == 0 || comp == 3) { return None; }
    let (w, top_down, h) = (w as usize, h < 0, h.unsigned_abs() as usize);
    // BI_BITFIELDS con cabecera de 40 bytes: siguen tres máscaras.
    let off = hsize + if comp == 3 && hsize == 40 { 12 } else { 0 };
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    let px = d.get(off..off + stride * h)?;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &px[(if top_down { y } else { h - 1 - y }) * stride..];
        for x in 0..w {
            let p = &row[x * (bpp as usize / 8)..];
            rgba.extend_from_slice(&[p[2], p[1], p[0], if bpp == 32 { p[3] } else { 255 }]);
        }
    }
    // Muchas apps dejan el canal alfa a 0 en imágenes opacas.
    if bpp == 32 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
        for p in rgba.chunks_exact_mut(4) { p[3] = 255; }
    }
    png_encode(w as u32, h as u32, &rgba)
}

/// PNG -> CF_DIB de 32 bits (de abajo arriba, BI_RGB).
fn png_to_dib(png: &[u8]) -> Option<Vec<u8>> {
    let (w, h, rgba) = png_decode(png)?;
    let (w, h) = (w as usize, h as usize);
    let mut d = Vec::with_capacity(40 + w * h * 4);
    d.extend_from_slice(&40u32.to_le_bytes());
    d.extend_from_slice(&(w as i32).to_le_bytes());
    d.extend_from_slice(&(h as i32).to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&32u16.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    d.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    d.extend_from_slice(&[0u8; 16]); // resolución y paleta
    for y in (0..h).rev() {
        for p in rgba[y * w * 4..(y + 1) * w * 4].chunks_exact(4) { d.extend_from_slice(&[p[2], p[1], p[0], p[3]]); }
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_ida_y_vuelta() {
        let rgba: Vec<u8> = (0..5 * 3).flat_map(|i| [i as u8 * 10, 20, 30, 255]).collect();
        let png = png_encode(5, 3, &rgba).unwrap();
        let dib = png_to_dib(&png).unwrap();
        assert_eq!(png_decode(&dib_to_png(&dib).unwrap()).unwrap(), (5, 3, rgba));
    }

    #[test]
    fn dib_24_bits_de_arriba_abajo() {
        // 2x2, 24 bits, altura negativa (de arriba abajo), filas alineadas a 4 bytes.
        let mut d = vec![0u8; 40];
        d[0..4].copy_from_slice(&40u32.to_le_bytes());
        d[4..8].copy_from_slice(&2i32.to_le_bytes());
        d[8..12].copy_from_slice(&(-2i32).to_le_bytes());
        d[14..16].copy_from_slice(&24u16.to_le_bytes());
        d.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]); // rojo, verde (BGR) + relleno
        d.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]); // azul, blanco + relleno
        let (w, h, px) = png_decode(&dib_to_png(&d).unwrap()).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(px, [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]);
    }
}
