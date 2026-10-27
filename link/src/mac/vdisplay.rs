//! Pantallas virtuales en macOS con CGVirtualDisplay (CoreGraphics, macOS 11+; la misma API que usan Sidecar, DeskPad o
//! BetterDisplay). No es pública: se usa por nombre desde el runtime de Objective-C y, si no existe, no se ofrecen.
//! La pantalla existe mientras vive el objeto: se quita sola al terminar la sesión o si Kast se cierra.
use super::active_displays;
use crate::Res;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{MainThreadMarker, msg_send};
use objc2_core_foundation::CGSize;
use objc2_core_graphics::{CGBeginDisplayConfiguration, CGCompleteDisplayConfiguration, CGConfigureDisplayOrigin, CGConfigureOption,
                          CGDirectDisplayID, CGDisplayConfigRef, CGDisplayModelNumber, CGDisplayVendorNumber};
use objc2_foundation::{NSArray, NSString};
use std::ffi::CStr;
use std::time::{Duration, Instant};

/// El objeto CGVirtualDisplay: mientras se guarda, la pantalla existe.
struct Obj(#[allow(dead_code)] Retained<AnyObject>);
unsafe impl Send for Obj {}

/// Pantallas virtuales vivas de este proceso: id, píxeles reales, Retina y hueco (para el número de serie).
/// macOS no informa del modo de una pantalla virtual (CGDisplayCopyDisplayMode no devuelve nada), así que su tamaño
/// real se toma de aquí.
static LIVE: std::sync::Mutex<Vec<(CGDirectDisplayID, (u32, u32), bool, u32)>> = std::sync::Mutex::new(Vec::new());

/// Píxeles reales de una pantalla virtual de Kast (None si no es una de ellas).
pub fn pixels(id: CGDirectDisplayID) -> Option<(u32, u32)> {
    LIVE.lock().unwrap().iter().find(|v| v.0 == id).map(|v| v.1)
}

/// Número de serie según el tamaño (macOS recuerda por él el modo de cada pantalla: si se reutilizara con otro
/// tamaño, mantendría el anterior) y un hueco para tener varias del mismo tamaño a la vez.
fn serial(px: (u32, u32), hidpi: bool, slot: u32) -> u32 {
    (slot.min(7) << 25) | ((hidpi as u32) << 24) | ((px.0 / 2).min(4095) << 12) | (px.1 / 2).min(4095)
}

pub struct Virtual {
    obj: Option<Obj>,
    pub id: CGDirectDisplayID,
}

/// Al soltarla, la pantalla desaparece (y macOS lleva sus ventanas a las que quedan). Sin esperar: quien la suelta
/// puede tener bloqueado el estado del agente que el hilo principal necesita.
impl Drop for Virtual {
    fn drop(&mut self) {
        let Some(o) = self.obj.take() else { return };
        LIVE.lock().unwrap().retain(|v| v.0 != self.id);
        if MainThreadMarker::new().is_some() { drop(o) } else { dispatch2::DispatchQueue::main().exec_async(move || drop(o)); }
    }
}

pub const UNSUPPORTED: &str = "esta versión de macOS no permite crear pantallas virtuales";

pub fn supported() -> bool { AnyClass::get(c"CGVirtualDisplay").is_some() }

/// Pantalla de reemplazo de macOS: sin ningún monitor (p. ej. con el monitor puesto en otra entrada), macOS muestra una
/// pantalla propia (fabricante «unkn», modelo «virt») que quita en cuanto aparece otra.
pub fn placeholder(id: CGDirectDisplayID) -> bool {
    CGDisplayVendorNumber(id) == u32::from_be_bytes(*b"unkn") && CGDisplayModelNumber(id) == u32::from_be_bytes(*b"virt")
}

/// En el hilo principal (el agente tiene ahí el bucle de ventanas, que atiende la cola principal).
fn on_main<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    if MainThreadMarker::new().is_some() { return f(); }
    let mut out = None;
    dispatch2::DispatchQueue::main().exec_sync(|| out = Some(f()));
    out.expect("cola principal")
}

fn queue() -> &'static dispatch2::DispatchRetained<dispatch2::DispatchQueue> {
    struct Queue(dispatch2::DispatchRetained<dispatch2::DispatchQueue>);
    unsafe impl Send for Queue {}
    unsafe impl Sync for Queue {}
    static Q: std::sync::OnceLock<Queue> = std::sync::OnceLock::new();
    &Q.get_or_init(|| Queue(dispatch2::DispatchQueue::new("io.github.abrahmch1.kast-link.vdisplay", None))).0
}

fn class(name: &CStr) -> Result<&'static AnyClass, String> {
    AnyClass::get(name).ok_or_else(|| UNSUPPORTED.into())
}

/// Crea una pantalla de `px` píxeles reales (con la escala del monitor del visor desde 1,5, Retina: la mitad en puntos)
/// y espera a que aparezca.
pub fn create(name: &str, px: (u32, u32), scale: f64) -> Res<Virtual> {
    let hidpi = scale >= 1.5;
    let (pw, ph) = (px.0.clamp(640, 7680) & !1, px.1.clamp(480, 4320) & !1);
    let slot = {
        let live = LIVE.lock().unwrap();
        (0..).find(|s| !live.iter().any(|v| v.1 == (pw, ph) && v.2 == hidpi && v.3 == *s)).unwrap_or(0)
    };
    let serial = serial((pw, ph), hidpi, slot);
    let name = name.to_string();
    let made = on_main(move || -> Result<(Obj, CGDirectDisplayID), String> {
        unsafe {
            let desc: Retained<AnyObject> = msg_send![class(c"CGVirtualDisplayDescriptor")?, new];
            // Las respuestas de macOS (modo aplicado, pantalla quitada) llegan por una cola propia: no dependen de
            // que el hilo principal esté libre.
            let queue = &**queue() as *const dispatch2::DispatchQueue as *const AnyObject;
            let _: () = msg_send![&*desc, setQueue: queue];
            let _: () = msg_send![&*desc, setName: &*NSString::from_str(&name)];
            let _: () = msg_send![&*desc, setMaxPixelsWide: pw];
            let _: () = msg_send![&*desc, setMaxPixelsHigh: ph];
            // Tamaño físico de un monitor normal (~109 ppp) o Retina (~218 ppp): macOS decide con él la escala.
            let mm = 25.4 / if hidpi { 218.0 } else { 109.0 };
            let _: () = msg_send![&*desc, setSizeInMillimeters: CGSize { width: pw as f64 * mm, height: ph as f64 * mm }];
            let _: () = msg_send![&*desc, setVendorID: 0x4B4Cu32];
            let _: () = msg_send![&*desc, setProductID: 0x0001u32];
            let _: () = msg_send![&*desc, setSerialNum: serial];
            let alloc: Allocated<AnyObject> = msg_send![class(c"CGVirtualDisplay")?, alloc];
            let disp: Option<Retained<AnyObject>> = msg_send![alloc, initWithDescriptor: &*desc];
            let disp = disp.ok_or("macOS no creó la pantalla virtual")?;
            // Un solo modo: el tamaño del monitor del visor (Retina: en puntos, con el doble de píxeles).
            let settings: Retained<AnyObject> = msg_send![class(c"CGVirtualDisplaySettings")?, new];
            let _: () = msg_send![&*settings, setHiDPI: hidpi as u32];
            let (mw, mh) = if hidpi { (pw / 2, ph / 2) } else { (pw, ph) };
            let alloc: Allocated<AnyObject> = msg_send![class(c"CGVirtualDisplayMode")?, alloc];
            let mode: Retained<AnyObject> = msg_send![alloc, initWithWidth: mw as usize, height: mh as usize, refreshRate: 60.0f64];
            let modes = NSArray::from_retained_slice(&[mode]);
            let _: () = msg_send![&*settings, setModes: &*modes];
            let ok: Bool = msg_send![&*disp, applySettings: &*settings];
            if !ok.as_bool() { return Err(format!("macOS no aceptó el modo {pw}×{ph} para la pantalla virtual")); }
            let id: CGDirectDisplayID = msg_send![&*disp, displayID];
            Ok((Obj(disp), id))
        }
    });
    let (obj, id) = made?;
    LIVE.lock().unwrap().push((id, (pw, ph), hidpi, slot));
    let v = Virtual { obj: Some(obj), id };
    // Tarda un momento en sumarse al escritorio.
    let t = Instant::now();
    while !active_displays().contains(&id) {
        if t.elapsed() > Duration::from_secs(5) { return Err("la pantalla virtual no apareció".into()); }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(v)
}

/// Coloca pantallas (id, x, y) en el escritorio, en puntos con origen en la principal. Sólo para esta sesión de
/// macOS: no se guarda como disposición del usuario.
pub fn arrange(origins: &[(CGDirectDisplayID, i32, i32)]) -> Res<()> {
    if origins.is_empty() { return Ok(()); }
    let origins = origins.to_vec();
    let err = on_main(move || unsafe {
        let mut cfg: CGDisplayConfigRef = std::ptr::null_mut();
        let e = CGBeginDisplayConfiguration(&mut cfg);
        if e.0 != 0 { return e.0; }
        for (id, x, y) in origins { CGConfigureDisplayOrigin(cfg, id, x, y); }
        CGCompleteDisplayConfiguration(cfg, CGConfigureOption::ForSession).0
    });
    if err != 0 { return Err(format!("macOS no pudo colocar las pantallas virtuales ({err})").into()); }
    Ok(())
}
