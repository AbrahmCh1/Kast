//! Pantallas virtuales en Windows con el controlador de pantalla virtual de Parsec (Parsec VDD, un controlador IddCx
//! firmado que viene con Parsec o suelto: github.com/nomi-san/parsec-vdd). Windows no trae ninguno y uno propio tendría
//! que firmarlo Microsoft, así que si no está instalado no se ofrecen (el visor lo sabe por el anuncio del equipo).
//! El controlador quita sus pantallas si nadie le avisa cada ~100 ms: mientras haya alguna de Kast, un hilo le avisa,
//! y si Kast se cierra o falla desaparecen solas.
use super::capture;
use crate::Res;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Devices::DeviceAndDriverInstallation::{CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_ListW,
                                                         CM_Get_Device_Interface_List_SizeW, CR_SUCCESS};
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, LUID};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::{GUID, PCWSTR};

/// Interfaz del adaptador de Parsec VDD.
const ADAPTER: GUID = GUID::from_u128(0x00b41627_04c4_429e_a26e_0265cf50c8fa);
const ADD: u32 = 0x0022e004;
const REMOVE: u32 = 0x0022a008;
const UPDATE: u32 = 0x0022a00c;

pub const UNSUPPORTED: &str = "falta el controlador de pantalla virtual de Parsec (Parsec VDD) en ese equipo";

/// El controlador abierto y los números (suyos) de las pantallas de Kast que siguen vivas.
struct Dev {
    h: HANDLE,
    live: Vec<u16>,
}
unsafe impl Send for Dev {}

static DEV: Mutex<Option<Dev>> = Mutex::new(None);
/// Se crean de una en una: la nueva es la pantalla que aparece después de pedirla.
static CREATE: Mutex<()> = Mutex::new(());

pub struct Virtual {
    /// Número de Windows de la pantalla (\\.\DISPLAYn), el id de Kast en Windows.
    pub id: u32,
    index: u16,
    /// Escala que tenía antes de ponerle la del visor (Windows la recuerda para la siguiente que use ese número).
    restore: Option<i32>,
}

/// Al soltarla, la pantalla desaparece y Windows lleva sus ventanas a las que quedan.
impl Drop for Virtual {
    fn drop(&mut self) {
        if let Some(rel) = self.restore { let _ = set_scale_rel(self.id, rel); }
        let mut dev = DEV.lock().unwrap();
        if let Some(d) = dev.as_mut() {
            ioctl(d.h, REMOVE, &self.index.to_be_bytes());
            ioctl(d.h, UPDATE, &[]);
            d.live.retain(|i| *i != self.index);
        }
    }
}

/// Ruta del controlador (None si no está instalado o está desactivado).
fn device_path() -> Option<Vec<u16>> {
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &ADAPTER, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS || len < 2 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&ADAPTER, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS { return None; }
        // Una lista de cadenas terminada en dos ceros: la primera.
        let end = buf.iter().position(|&c| c == 0)?;
        (end > 0).then(|| buf[..=end].to_vec())
    }
}

pub fn supported() -> bool {
    // Se consulta en cada anuncio y cada estado: se recuerda unos segundos.
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if let Some((t, ok)) = *last && t.elapsed() < Duration::from_secs(5) { return ok; }
    let ok = device_path().is_some();
    *last = Some((Instant::now(), ok));
    ok
}

/// Una orden al controlador (entrada de 32 bytes, salida de 4). None si no respondió.
fn ioctl(h: HANDLE, code: u32, data: &[u8]) -> Option<u32> {
    let mut inb = [0u8; 32];
    inb[..data.len()].copy_from_slice(data);
    let mut out = 0u32;
    unsafe {
        let ev = CreateEventW(None, true, false, PCWSTR::null()).ok()?;
        let mut ov = OVERLAPPED { hEvent: ev, ..Default::default() };
        let started = match DeviceIoControl(h, code, Some(inb.as_ptr().cast()), 32, Some((&raw mut out).cast()), 4, None, Some(&mut ov)) {
            Ok(()) => true,
            Err(e) => e.code() == ERROR_IO_PENDING.to_hresult(),
        };
        let mut n = 0;
        let ok = started && (GetOverlappedResultEx(h, &ov, &mut n, 5000, false).is_ok() || {
            // Sin respuesta: se cancela y se espera a que termine (usa `out` y `ov`).
            let _ = CancelIoEx(h, Some(&ov));
            let _ = GetOverlappedResult(h, &ov, &mut n, true);
            false
        });
        let _ = CloseHandle(ev);
        ok.then_some(out)
    }
}

/// Avisa al controlador cada 50 ms mientras haya pantallas de Kast; luego lo cierra.
fn keep_alive() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(50));
        let mut dev = DEV.lock().unwrap();
        let Some(d) = dev.as_ref() else { return };
        if d.live.is_empty() {
            unsafe { let _ = CloseHandle(d.h); }
            *dev = None;
            return;
        }
        ioctl(d.h, UPDATE, &[]);
    });
}

/// Número de Windows de cada pantalla del escritorio (\\.\DISPLAYn).
fn numbers() -> Vec<u32> { super::display_ids() }

fn gdi_name(id: u32) -> Vec<u16> { format!(r"\\.\DISPLAY{id}").encode_utf16().chain([0]).collect() }

/// Crea una pantalla de `px` píxeles (o el modo más parecido que tenga el controlador) con la escala del monitor del
/// visor, y espera a que aparezca.
pub fn create(_name: &str, px: (u32, u32), scale: f64) -> Res<Virtual> {
    let _one = CREATE.lock().unwrap();
    let before = numbers();
    let index = {
        let mut dev = DEV.lock().unwrap();
        if dev.is_none() {
            let path = device_path().ok_or(UNSUPPORTED)?;
            let h = unsafe {
                CreateFileW(PCWSTR(path.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING,
                            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING | FILE_FLAG_OVERLAPPED | FILE_FLAG_WRITE_THROUGH, None)
            }.map_err(|e| format!("no se pudo abrir el controlador de pantalla virtual de Parsec: {e}"))?;
            *dev = Some(Dev { h, live: Vec::new() });
            keep_alive();
        }
        let d = dev.as_mut().unwrap();
        let index = ioctl(d.h, ADD, &[]).filter(|i| *i < 64).ok_or("el controlador de Parsec no creó la pantalla")? as u16;
        ioctl(d.h, UPDATE, &[]);
        d.live.push(index);
        index
    };
    // Desde aquí, si algo falla, al soltarla se quita.
    let mut v = Virtual { id: 0, index, restore: None };
    // Tarda un momento en sumarse al escritorio.
    let t = Instant::now();
    v.id = loop {
        if let Some(id) = numbers().into_iter().find(|n| !before.contains(n)) { break id; }
        if t.elapsed() > Duration::from_secs(8) { return Err("la pantalla virtual no apareció".into()); }
        std::thread::sleep(Duration::from_millis(50));
    };
    let name = gdi_name(v.id);
    // El modo: el que tenga del tamaño pedido (o el más parecido).
    let mut modes = Vec::new();
    let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    for i in 0.. {
        if !unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm) }.as_bool() { break; }
        modes.push((dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency));
    }
    let (w, h, hz) = pick(&modes, px).ok_or("la pantalla virtual no tiene modos")?;
    let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    (dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency) = (w, h, hz);
    // Sin CDS_UPDATEREGISTRY: sólo mientras exista, no se guarda como configuración de pantallas del usuario.
    let r = unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_TYPE(0), None) };
    if r != DISP_CHANGE_SUCCESSFUL { return Err(format!("Windows no aceptó el modo {w}×{h} para la pantalla virtual ({})", r.0).into()); }
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(3) {
        let found = capture::outputs().ok().and_then(|o| o.into_iter().find(|o| o.name == format!(r"\\.\DISPLAY{}", v.id)).map(|o| (o.w, o.h)));
        if found == Some((w, h)) { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    // La escala de Windows, como la del monitor del visor (si se puede).
    if let Some((cur, rel)) = scale_rel(v.id, scale) && cur != rel && set_scale_rel(v.id, rel) { v.restore = Some(cur); }
    Ok(v)
}

/// El modo más parecido a `want`: el mismo tamaño si lo hay; si no, la misma forma y un tamaño cercano. A 60 Hz si se
/// puede.
fn pick(modes: &[(u32, u32, u32)], want: (u32, u32)) -> Option<(u32, u32, u32)> {
    let ar = want.0 as f64 / want.1.max(1) as f64;
    let area = (want.0 as f64 * want.1 as f64).max(1.0);
    let cost = |&(w, h, hz): &(u32, u32, u32)| {
        (w as f64 / h.max(1) as f64 - ar).abs() * 4.0 + (w as f64 * h as f64 / area).ln().abs() + if hz == 60 { 0.0 } else { 0.001 }
    };
    modes.iter().filter(|m| m.0 > 0 && m.1 > 0).min_by(|a, b| cost(a).total_cmp(&cost(b))).copied()
}

/// La configuración de pantallas activa de Windows y, en ella, la de una pantalla (para las llamadas DisplayConfig).
struct Config {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    /// La ruta de la pantalla.
    i: usize,
}

impl Config {
    fn adapter(&self) -> LUID { self.paths[self.i].sourceInfo.adapterId }
    fn source(&self) -> u32 { self.paths[self.i].sourceInfo.id }
}

fn config(id: u32) -> Option<Config> {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).is_err() { return None; }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None).is_err() { return None; }
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        let want: Vec<u16> = format!(r"\\.\DISPLAY{id}").encode_utf16().collect();
        let i = paths.iter().position(|p| {
            let mut n = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            n.header = DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, size: size_of_val(&n) as u32,
                                                          adapterId: p.sourceInfo.adapterId, id: p.sourceInfo.id };
            DisplayConfigGetDeviceInfo(&mut n.header) == 0 && n.viewGdiDeviceName.starts_with(&want) && n.viewGdiDeviceName.get(want.len()) == Some(&0)
        })?;
        Some(Config { paths, modes, i })
    }
}

// Escala de una pantalla: Windows no lo publica, pero es lo que usa Configuración (DisplayConfig con los tipos -3 y -4).
// Se expresa en pasos respecto a la recomendada para ese monitor.
const DPI: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

#[repr(C)]
struct DpiGet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    min: i32,
    cur: i32,
    max: i32,
}

#[repr(C)]
struct DpiSet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    rel: i32,
}

/// Paso actual y paso para `scale` (lo más cerca posible, dentro de lo que admite).
fn scale_rel(id: u32, scale: f64) -> Option<(i32, i32)> {
    let c = config(id)?;
    let mut g = DpiGet { header: DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: DISPLAYCONFIG_DEVICE_INFO_TYPE(-3), size: size_of::<DpiGet>() as u32,
                                                                    adapterId: c.adapter(), id: c.source() }, min: 0, cur: 0, max: 0 };
    if unsafe { DisplayConfigGetDeviceInfo(&mut g.header) } != 0 { return None; }
    let rec = g.min.unsigned_abs() as usize; // la recomendada, en DPI
    let want = (scale * 100.0).round() as i64;
    let best = DPI.iter().enumerate().min_by_key(|(_, d)| (**d as i64 - want).abs())?.0 as i32 - rec as i32;
    Some((g.cur, best.clamp(g.min, g.max)))
}

fn set_scale_rel(id: u32, rel: i32) -> bool {
    let Some(c) = config(id) else { return false };
    let s = DpiSet { header: DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: DISPLAYCONFIG_DEVICE_INFO_TYPE(-4), size: size_of::<DpiSet>() as u32,
                                                                adapterId: c.adapter(), id: c.source() }, rel };
    unsafe { DisplayConfigSetDeviceInfo(&s.header) == 0 }
}

/// Coloca pantallas (número, x, y) en el escritorio, en píxeles con origen en la principal. Sólo mientras existan: no
/// se guarda en la configuración de pantallas del usuario.
pub fn arrange(origins: &[(u32, i32, i32)]) -> Res<()> {
    let Some(Config { paths, mut modes, .. }) = origins.first().and_then(|o| config(o.0)) else { return Ok(()) };
    for &(id, x, y) in origins {
        let Some(c) = config(id) else { continue };
        let idx = unsafe { paths[c.i].sourceInfo.Anonymous.modeInfoIdx } as usize;
        if let Some(m) = modes.get_mut(idx).filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) {
            m.Anonymous.sourceMode.position.x = x;
            m.Anonymous.sourceMode.position.y = y;
        }
    }
    let r = unsafe { SetDisplayConfig(Some(&paths), Some(&modes), SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES) };
    if r != 0 { return Err(format!("Windows no pudo colocar las pantallas virtuales ({r})").into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn elige_el_modo() {
        let modes = [(1280, 720, 60), (1920, 1080, 30), (1920, 1080, 60), (2560, 1440, 60), (2880, 1800, 60), (1600, 1200, 60)];
        assert_eq!(pick(&modes, (1920, 1080)), Some((1920, 1080, 60)));
        assert_eq!(pick(&modes, (2880, 1800)), Some((2880, 1800, 60)));
        // Sin el tamaño exacto: la misma forma y lo más cerca.
        assert_eq!(pick(&modes, (2560, 1600)), Some((2880, 1800, 60)));
        assert_eq!(pick(&modes, (1366, 768)), Some((1280, 720, 60)));
        assert_eq!(pick(&[], (1920, 1080)), None);
    }
}
