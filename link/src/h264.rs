//! Utilidades H.264 independientes de la plataforma.
//! En la red viaja siempre Annex B (códigos de inicio 00 00 00 01), que es lo que producen y consumen
//! Media Foundation en Windows. VideoToolbox en macOS usa AVCC (longitud de 4 bytes por unidad NAL)
//! y los parámetros SPS/PPS aparte, así que allí se convierte en ambos sentidos.

pub const NAL_IDR: u8 = 5;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const NAL_SPS: u8 = 7;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const NAL_PPS: u8 = 8;

pub fn nal_type(nal: &[u8]) -> u8 { nal.first().map_or(0, |b| b & 0x1F) }

/// Separa un flujo Annex B en unidades NAL (sin los códigos de inicio).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() { starts[k + 1] - 3 } else { data.len() };
        // El código de inicio de 4 bytes deja un 0 extra al final de la unidad anterior.
        while e > s && data[e - 1] == 0 && k + 1 < starts.len() { e -= 1; }
        if e > s { out.push(&data[s..e]); }
    }
    out
}

pub fn push_annexb(out: &mut Vec<u8>, nal: &[u8]) {
    out.extend_from_slice(&[0, 0, 0, 1]);
    out.extend_from_slice(nal);
}

/// AVCC (longitudes de 4 bytes) -> Annex B. Devuelve si contiene un fotograma IDR.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn avcc_to_annexb(avcc: &[u8], out: &mut Vec<u8>) -> bool {
    let mut idr = false;
    let mut i = 0;
    while i + 4 <= avcc.len() {
        let n = u32::from_be_bytes(avcc[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        if i + n > avcc.len() { break; }
        let nal = &avcc[i..i + n];
        idr |= nal_type(nal) == NAL_IDR;
        push_annexb(out, nal);
        i += n;
    }
    idr
}

/// Unidades NAL -> AVCC (longitudes de 4 bytes).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn to_avcc<'a>(nals: impl Iterator<Item = &'a [u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    for n in nals {
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
    out
}

// ---------------------------------------------------------------- reescritura de SPS
//
// Si el SPS no declara `max_num_reorder_frames = 0` (VideoToolbox no lo hace), el decodificador de
// Windows retiene tantos fotogramas como permita el nivel H.264 antes de entregar el primero (~5),
// aunque esté en modo de baja latencia. Se añade/ajusta `bitstream_restriction` en la VUI del SPS.

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let b = *self.d.get(self.pos / 8)?;
        let v = (b >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(v as u32)
    }
    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n { v = (v << 1) | self.bit()?; }
        Some(v)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 { return None; }
        }
        Some((1u32 << zeros) - 1 + self.bits(zeros)?)
    }
    fn se(&mut self) -> Option<i32> {
        let k = self.ue()? as i64;
        Some(if k % 2 == 1 { ((k + 1) / 2) as i32 } else { -((k / 2) as i32) })
    }
}

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    n: usize,
}

impl BitWriter {
    fn bit(&mut self, b: u32) {
        if self.n % 8 == 0 { self.out.push(0); }
        if b & 1 != 0 { *self.out.last_mut().unwrap() |= 1 << (7 - self.n % 8); }
        self.n += 1;
    }
    #[cfg(test)]
    fn bits(&mut self, v: u32, n: u32) { for i in (0..n).rev() { self.bit((v >> i) & 1); } }
    fn ue(&mut self, v: u32) {
        let x = v as u64 + 1;
        let len = 64 - x.leading_zeros();
        for _ in 0..len - 1 { self.bit(0); }
        for i in (0..len).rev() { self.bit(((x >> i) & 1) as u32); }
    }
    /// Copia bits [from, to) de otro flujo.
    fn copy(&mut self, src: &[u8], from: usize, to: usize) {
        for p in from..to { self.bit(((src[p / 8] >> (7 - p % 8)) & 1) as u32); }
    }
}

fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in nal {
        if zeros >= 2 && b == 3 { zeros = 0; continue; }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len() + 8);
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 { out.push(3); zeros = 0; }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

fn skip_hrd(r: &mut Bits) -> Option<()> {
    let cpb_cnt = r.ue()? + 1;
    r.bits(8)?; // bit_rate_scale + cpb_size_scale
    for _ in 0..cpb_cnt { r.ue()?; r.ue()?; r.bit()?; }
    r.bits(20)?; // 4 campos de 5 bits
    Some(())
}

/// Devuelve el SPS (unidad NAL completa, con cabecera) declarando cero fotogramas reordenados.
/// `None` si no se puede analizar (se usa el original).
pub fn sps_low_latency(nal: &[u8]) -> Option<Vec<u8>> {
    if nal_type(nal) != NAL_SPS || nal.len() < 4 { return None; }
    let rbsp = unescape(&nal[1..]);
    let mut r = Bits { d: &rbsp, pos: 0 };
    let profile = r.bits(8)?;
    r.bits(16)?; // restricciones + nivel
    r.ue()?;     // seq_parameter_set_id
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        if r.ue()? == 3 { r.bit()?; }
        r.ue()?;
        r.ue()?;
        r.bit()?;
        if r.bit()? == 1 { return None; } // matrices de escalado: no se tocan
    }
    r.ue()?; // log2_max_frame_num_minus4
    match r.ue()? {
        0 => { r.ue()?; }
        1 => {
            r.bit()?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? { r.se()?; }
        }
        _ => {}
    }
    let max_ref = r.ue()?;
    r.bit()?;
    r.ue()?;
    r.ue()?;
    if r.bit()? == 0 { r.bit()?; }
    r.bit()?;
    if r.bit()? == 1 { for _ in 0..4 { r.ue()?; } }
    let vui_flag_pos = r.pos;

    let mut w = BitWriter::default();
    w.copy(&rbsp, 0, vui_flag_pos);
    w.bit(1); // vui_parameters_present_flag
    if r.bit()? == 1 {
        // Se conserva la VUI original hasta bitstream_restriction_flag.
        let start = r.pos;
        if r.bit()? == 1 && r.bits(8)? == 255 { r.bits(32)?; }
        if r.bit()? == 1 { r.bit()?; }
        if r.bit()? == 1 {
            r.bits(4)?;
            if r.bit()? == 1 { r.bits(24)?; }
        }
        if r.bit()? == 1 { r.ue()?; r.ue()?; }
        if r.bit()? == 1 { r.bits(32)?; r.bits(32)?; r.bit()?; }
        let nal_hrd = r.bit()? == 1;
        if nal_hrd { skip_hrd(&mut r)?; }
        let vcl_hrd = r.bit()? == 1;
        if vcl_hrd { skip_hrd(&mut r)?; }
        if nal_hrd || vcl_hrd { r.bit()?; }
        r.bit()?; // pic_struct_present_flag
        w.copy(&rbsp, start, r.pos);
    } else {
        // VUI mínima: aspecto, overscan, señal de vídeo, croma, tiempos, HRD NAL, HRD VCL y pic_struct ausentes.
        for _ in 0..8 { w.bit(0); }
    }
    w.bit(1); // bitstream_restriction_flag
    w.bit(1); // motion_vectors_over_pic_boundaries_flag
    w.ue(2);  // max_bytes_per_pic_denom
    w.ue(1);  // max_bits_per_mb_denom
    w.ue(16); // log2_max_mv_length_horizontal
    w.ue(16); // log2_max_mv_length_vertical
    w.ue(0);  // max_num_reorder_frames
    w.ue(max_ref.max(1)); // max_dec_frame_buffering
    w.bit(1); // rbsp_stop_one_bit
    while w.n % 8 != 0 { w.bit(0); }

    let mut out = vec![nal[0]];
    out.extend_from_slice(&escape(&w.out));
    Some(out)
}

/// Reescribe los SPS de una unidad de acceso Annex B (ver `sps_low_latency`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn low_latency_annexb(data: &[u8]) -> Option<Vec<u8>> {
    let nals = nal_units(data);
    if !nals.iter().any(|n| nal_type(n) == NAL_SPS) { return None; }
    let mut out = Vec::with_capacity(data.len() + 16);
    for n in nals {
        match sps_low_latency(n) {
            Some(s) => push_annexb(&mut out, &s),
            None => push_annexb(&mut out, n),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let sps = [0x67u8, 0x42, 0xC0, 0x1F];
        let idr = [0x65u8, 0x88, 0x84, 0x00, 0x00];
        let mut annexb = Vec::new();
        push_annexb(&mut annexb, &sps);
        push_annexb(&mut annexb, &idr);
        let nals = nal_units(&annexb);
        assert_eq!(nals, vec![&sps[..], &idr[..]]);
        let avcc = to_avcc(nals.into_iter());
        let mut back = Vec::new();
        assert!(avcc_to_annexb(&avcc, &mut back));
        assert_eq!(back, annexb);
    }

    #[test]
    fn exp_golomb() {
        let mut w = BitWriter::default();
        for v in [0, 1, 2, 7, 16, 255] { w.ue(v); }
        let mut r = Bits { d: &w.out, pos: 0 };
        for v in [0, 1, 2, 7, 16, 255] { assert_eq!(r.ue(), Some(v)); }
    }

    #[test]
    fn sps_rewrite_is_stable() {
        // SPS Baseline 1280x720 sin VUI (generado con el escritor de bits).
        let mut w = BitWriter::default();
        w.bits(66, 8); w.bits(0xC0, 8); w.bits(31, 8); w.ue(0); // perfil, restricciones, nivel, id
        w.ue(0); w.ue(2); // log2_max_frame_num, poc tipo 2
        w.ue(1); w.bit(0); w.ue(79); w.ue(44); w.bit(1); w.bit(1); w.bit(0); // refs, gaps, 80x45 MB, frame_mbs, 8x8, sin recorte
        w.bit(0); // sin VUI
        w.bit(1); while w.n % 8 != 0 { w.bit(0); }
        let mut sps = vec![0x67];
        sps.extend_from_slice(&escape(&w.out));
        let fixed = sps_low_latency(&sps).expect("analizable");
        assert!(fixed.len() > sps.len());
        // Reescribir otra vez conserva la VUI ya añadida y produce lo mismo.
        assert_eq!(sps_low_latency(&fixed).unwrap(), fixed);
    }
}
