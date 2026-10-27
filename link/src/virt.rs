//! Pantallas virtuales: dónde colocarlas en el otro equipo para que queden como tus monitores. El visor dice en qué
//! monitor suyo se verá cada pantalla (su rectángulo, en las unidades de su escritorio) y el anfitrión pega cada
//! pantalla virtual a la que tiene al lado en el visor, por el mismo borde y a la misma altura (o posición)
//! proporcional, para que el ratón pase de una a otra como pasa de un monitor a otro.
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self { Self { x, y, w, h } }
    fn right(&self) -> f64 { self.x + self.w }
    fn bottom(&self) -> f64 { self.y + self.h }
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    /// `[x, y, ancho, alto]`.
    pub fn from_json(v: &Value) -> Option<Self> {
        let a = v.as_array()?;
        let n = |i: usize| a.get(i).and_then(Value::as_f64);
        let r = Self::new(n(0)?, n(1)?, n(2)?, n(3)?);
        (r.w > 0.0 && r.h > 0.0).then_some(r)
    }

    pub fn json(&self) -> Value { json!([self.x, self.y, self.w, self.h]) }
}

#[derive(Clone, Copy)]
enum Side { Right, Left, Below, Above }

/// Si `v` está pegado a `p` en el visor: por qué lado y dónde queda en el anfitrión (`hp`: `p` en el anfitrión;
/// `size`: tamaño de `v` en el anfitrión).
fn beside(p: &Rect, hp: &Rect, v: &Rect, size: (f64, f64)) -> Option<(Side, f64, f64)> {
    let tol = (p.w.min(p.h).min(v.w).min(v.h) * 0.02).max(2.0);
    let over_y = p.bottom().min(v.bottom()) - p.y.max(v.y);
    let over_x = p.right().min(v.right()) - p.x.max(v.x);
    // Desplazamiento a lo largo del borde compartido, en proporción a la pantalla de al lado.
    let along_y = hp.y + (v.y - p.y) * hp.h / p.h;
    let along_x = hp.x + (v.x - p.x) * hp.w / p.w;
    if (v.x - p.right()).abs() <= tol && over_y > 0.0 { return Some((Side::Right, hp.right(), along_y)); }
    if (v.right() - p.x).abs() <= tol && over_y > 0.0 { return Some((Side::Left, hp.x - size.0, along_y)); }
    if (v.y - p.bottom()).abs() <= tol && over_x > 0.0 { return Some((Side::Below, along_x, hp.bottom())); }
    if (v.bottom() - p.y).abs() <= tol && over_x > 0.0 { return Some((Side::Above, along_x, hp.y - size.1)); }
    None
}

/// Origen en el anfitrión de cada pantalla virtual.
/// - `fixed`: sus pantallas reales que se verán en monitores del visor: (rectángulo en el visor, en el anfitrión).
/// - `virt`: las virtuales: (rectángulo en el visor, tamaño en el anfitrión).
/// - `others`: sus demás pantallas, que no se pueden tapar (si estorban, la virtual se aparta en la misma dirección).
///
/// Se colocan en orden de cercanía: primero las pegadas a una real, después las pegadas a esas, etc. Las que no
/// tocan ninguna se colocan en proporción a la primera real (o, sin ninguna, a la derecha de todo).
pub fn place(fixed: &[(Rect, Rect)], virt: &[(Rect, (f64, f64))], others: &[Rect]) -> Vec<(i32, i32)> {
    let mut placed: Vec<(Rect, Rect)> = fixed.to_vec();
    let mut taken: Vec<Rect> = others.iter().copied().chain(fixed.iter().map(|f| f.1)).collect();
    let mut out: Vec<Option<(i32, i32)>> = vec![None; virt.len()];
    loop {
        let mut progress = false;
        for i in 0..virt.len() {
            if out[i].is_some() { continue; }
            let (v, size) = virt[i];
            if let Some((side, x, y)) = placed.iter().find_map(|(p, hp)| beside(p, hp, &v, size)) {
                let r = settle(side, (x, y), size, &taken);
                out[i] = Some((r.x as i32, r.y as i32));
                placed.push((v, r));
                taken.push(r);
                progress = true;
            }
        }
        if !progress { break; }
    }
    for i in 0..virt.len() {
        if out[i].is_some() { continue; }
        let at = match fixed.first() {
            Some((p, hp)) => (hp.x + (virt[i].0.x - p.x) * hp.w / p.w, hp.y + (virt[i].0.y - p.y) * hp.h / p.h),
            None => (taken.iter().map(Rect::right).fold(0.0, f64::max), 0.0),
        };
        let r = settle(Side::Right, at, virt[i].1, &taken);
        out[i] = Some((r.x as i32, r.y as i32));
        taken.push(r);
    }
    out.into_iter().map(Option::unwrap).collect()
}

/// La pantalla en (x, y) y, si tapa otra, apartada hacia `side` hasta que no tape ninguna.
fn settle(side: Side, (mut x, mut y): (f64, f64), (w, h): (f64, f64), taken: &[Rect]) -> Rect {
    for _ in 0..=taken.len() {
        let r = Rect::new(x.round(), y.round(), w, h);
        let Some(o) = taken.iter().find(|t| t.overlaps(&r)) else { break };
        match side {
            Side::Right => x = o.right(),
            Side::Left => x = o.x - w,
            Side::Below => y = o.bottom(),
            Side::Above => y = o.y - h,
        }
    }
    Rect::new(x.round(), y.round(), w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect { Rect::new(x, y, w, h) }

    /// Monitor 1440p del visor que muestra la pantalla de 1440×900 del Mac.
    fn mac() -> (Rect, Rect) { (r(0.0, 0.0, 2560.0, 1440.0), r(0.0, 0.0, 1440.0, 900.0)) }

    #[test]
    fn a_cada_lado() {
        // A la derecha, centrada en vertical: misma altura proporcional (360 de 1440 -> 225 de 900).
        assert_eq!(place(&[mac()], &[(r(2560.0, 360.0, 1920.0, 1080.0), (1920.0, 1080.0))], &[]), vec![(1440, 225)]);
        // A la izquierda: termina donde empieza la real.
        assert_eq!(place(&[mac()], &[(r(-1920.0, 0.0, 1920.0, 1080.0), (1920.0, 1080.0))], &[]), vec![(-1920, 0)]);
        // Debajo y encima, en proporción a lo ancho.
        assert_eq!(place(&[mac()], &[(r(1280.0, 1440.0, 1280.0, 720.0), (1280.0, 720.0))], &[]), vec![(720, 900)]);
        assert_eq!(place(&[mac()], &[(r(0.0, -1080.0, 1920.0, 1080.0), (960.0, 540.0))], &[]), vec![(0, -540)]);
    }

    #[test]
    fn en_cadena_y_sin_tocar() {
        // Tres monitores en fila: la real en medio, una virtual a cada lado y otra pegada a la de la derecha.
        let fixed = [(r(0.0, 0.0, 1920.0, 1080.0), r(0.0, 0.0, 1920.0, 1080.0))];
        let virt = [
            (r(3840.0, 0.0, 1920.0, 1080.0), (1920.0, 1080.0)),
            (r(1920.0, 0.0, 1920.0, 1080.0), (2560.0, 1440.0)),
            (r(-1920.0, 0.0, 1920.0, 1080.0), (1920.0, 1080.0)),
        ];
        assert_eq!(place(&fixed, &virt, &[]), vec![(4480, 0), (1920, 0), (-1920, 0)]);
        // Un monitor que no toca a ninguno: en proporción a la real.
        assert_eq!(place(&fixed, &[(r(0.0, 2160.0, 1920.0, 1080.0), (1920.0, 1080.0))], &[]), vec![(0, 2160)]);
    }

    #[test]
    fn no_tapa_otras_pantallas() {
        // El Mac tiene otra pantalla a la derecha que no se usa: la virtual se pone después.
        let otra = r(1440.0, 0.0, 1920.0, 1080.0);
        assert_eq!(place(&[mac()], &[(r(2560.0, 0.0, 1920.0, 1080.0), (1920.0, 1080.0))], &[otra]), vec![(3360, 0)]);
        // Sin pantallas reales en el visor: a la derecha de todas.
        assert_eq!(place(&[], &[(r(0.0, 0.0, 1920.0, 1080.0), (1920.0, 1080.0))], &[mac().1, otra]), vec![(3360, 0)]);
    }
}
