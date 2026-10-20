//! Conversión de color BT.709 (rango limitado) entre BGRA y NV12, en CPU.

/// BGRA (con `pitch` bytes por fila) -> NV12 compacto (Y de w*h + UV entrelazado de w*h/2). w y h pares.
pub fn bgra_to_nv12(src: &[u8], pitch: usize, w: usize, h: usize, dst: &mut [u8]) {
    let (y_plane, uv_plane) = dst.split_at_mut(w * h);
    for y in (0..h).step_by(2) {
        let r0 = &src[y * pitch..y * pitch + w * 4];
        let r1 = &src[(y + 1) * pitch..(y + 1) * pitch + w * 4];
        let (ya, yb) = y_plane[y * w..(y + 2) * w].split_at_mut(w);
        let uv = &mut uv_plane[(y / 2) * w..(y / 2) * w + w];
        for x in (0..w).step_by(2) {
            let mut rs = 0i32;
            let mut gs = 0i32;
            let mut bs = 0i32;
            for (row, out) in [(r0, &mut *ya), (r1, &mut *yb)] {
                for dx in 0..2 {
                    let p = (x + dx) * 4;
                    let (b, g, r) = (row[p] as i32, row[p + 1] as i32, row[p + 2] as i32);
                    out[x + dx] = (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8;
                    rs += r;
                    gs += g;
                    bs += b;
                }
            }
            let (r, g, b) = (rs >> 2, gs >> 2, bs >> 2);
            uv[x] = (((-26 * r - 87 * g + 113 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            uv[x + 1] = (((113 * r - 103 * g - 10 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
        }
    }
}

/// NV12 (Y con `pitch` bytes por fila; UV empieza en `uv_offset`) -> píxeles 0RGB de w*h.
pub fn nv12_to_xrgb(src: &[u8], pitch: usize, uv_offset: usize, w: usize, h: usize, dst: &mut [u32]) {
    for y in 0..h {
        let yrow = &src[y * pitch..y * pitch + w];
        let uvrow = &src[uv_offset + (y / 2) * pitch..uv_offset + (y / 2) * pitch + w];
        let out = &mut dst[y * w..(y + 1) * w];
        for x in 0..w {
            let c = 298 * (yrow[x] as i32 - 16);
            let d = uvrow[x & !1] as i32 - 128;
            let e = uvrow[(x & !1) + 1] as i32 - 128;
            let r = ((c + 459 * e + 128) >> 8).clamp(0, 255) as u32;
            let g = ((c - 55 * d - 136 * e + 128) >> 8).clamp(0, 255) as u32;
            let b = ((c + 541 * d + 128) >> 8).clamp(0, 255) as u32;
            out[x] = (r << 16) | (g << 8) | b;
        }
    }
}
