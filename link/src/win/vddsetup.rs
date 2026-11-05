//! Instalar y quitar el controlador de pantallas virtuales de Kast (vdd/ en el repositorio).
//!
//!   kast-link vdd install [--from carpeta]    con permiso de administrador (lo pide Windows)
//!   kast-link vdd uninstall
//!   kast-link vdd status
//!
//! La carpeta (por defecto, `vdd` junto a kast-link.exe) tiene el paquete firmado: KastDisplay.inf, KastDisplay.dll,
//! kastdisplay.cat y el certificado público de Kast, kast-vdd.cer. Instalar hace que el equipo confíe en ese certificado
//! para software de Kast (Entidades de certificación raíz de confianza y Editores de confianza; sólo sirve para firmar
//! código), añade el paquete al almacén de controladores y crea el adaptador «Kast Virtual Display». Quitar deshace las
//! tres cosas.
use super::vdisplay;
use crate::Res;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Foundation::{CloseHandle, HWND, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{HKEY, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegGetValueW};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{IsUserAnAdmin, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{BOOL, PCWSTR, w};

const HWID: &str = r"Root\KastDisplay";
const CERT_NAME: &str = "Kast Virtual Display";

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

/// Carpeta del paquete del controlador por defecto: `vdd` junto a kast-link.exe.
pub fn default_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("vdd"))).unwrap_or_else(|| PathBuf::from("vdd"))
}

pub fn status() -> String {
    let kast = vdisplay::kast_installed();
    let devices = devices().len();
    format!("controlador de Kast: {}{}", if kast { "instalado y activo" } else if devices > 0 { "instalado, pero sin arrancar" } else { "no instalado" },
            if vdisplay::supported() && !kast { " (se usa el de Parsec)" } else { "" })
}

/// Instala el controlador. Sin permiso de administrador, se vuelve a lanzar pidiéndolo (Windows muestra su aviso) y
/// espera a que termine.
pub fn install(dir: &Path) -> Res<()> {
    let dir = std::fs::canonicalize(dir).map_err(|e| format!("no encuentro el paquete del controlador en {}: {e}", dir.display()))?;
    let inf = dir.join("KastDisplay.inf");
    let cer = dir.join("kast-vdd.cer");
    for f in [&inf, &cer, &dir.join("KastDisplay.dll"), &dir.join("kastdisplay.cat")] {
        if !f.exists() { return Err(format!("falta {} (¿paquete firmado?)", f.display()).into()); }
    }
    if !unsafe { IsUserAnAdmin() }.as_bool() {
        return elevated(&["vdd", "install", "--from", &dir.to_string_lossy()]);
    }
    // 1. Confiar en el certificado de Kast (sólo firma de código).
    for store in ["Root", "TrustedPublisher"] {
        run("certutil", &["-f", "-addstore", store, &cer.to_string_lossy()])?;
    }
    // 2. El paquete, al almacén de controladores (y a los adaptadores que ya existan, si es una actualización).
    run("pnputil", &["/add-driver", &inf.to_string_lossy(), "/install"])?;
    // 3. El adaptador (un dispositivo raíz, como los de Parsec o los de ejemplo de Microsoft).
    if devices().is_empty() { create_device()?; }
    let inf_w = wide(&inf.to_string_lossy());
    let mut reboot = BOOL(0);
    unsafe { UpdateDriverForPlugAndPlayDevicesW(None, PCWSTR(wide(HWID).as_ptr()), PCWSTR(inf_w.as_ptr()), INSTALLFLAG_FORCE, Some(&mut reboot)) }
        .map_err(|e| format!("Windows no instaló el controlador en el adaptador: {e}"))?;
    // Arranca en un momento.
    vdisplay::forget_support();
    let t = std::time::Instant::now();
    while !vdisplay::kast_installed() {
        if t.elapsed() > std::time::Duration::from_secs(20) {
            return Err(format!("el controlador se instaló pero no arrancó: {}", why_not_started(reboot.as_bool())).into());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Ok(())
}

/// Por qué no arrancó el adaptador, con el código de problema del Administrador de dispositivos.
fn why_not_started(reboot: bool) -> String {
    let mut problems = devices().into_iter().filter_map(|d| {
        let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS(0), CM_PROB(0));
        (unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, d, 0) } == CR_SUCCESS).then_some((status, problem))
    });
    let Some((status, problem)) = problems.next() else {
        return "Windows no creó el adaptador".into();
    };
    if status.0 & DN_HAS_PROBLEM.0 == 0 {
        return if status.0 & DN_STARTED.0 != 0 { "arrancó, pero no se encuentra su interfaz".into() }
               else if reboot { "hay que reiniciar el equipo para terminar".into() }
               else { "sigue sin arrancar".into() };
    }
    let what = match problem {
        CM_PROB_NEED_RESTART => "hay que reiniciar el equipo para terminar",
        CM_PROB_DISABLED => "el adaptador está deshabilitado",
        CM_PROB_FAILED_INSTALL | CM_PROB_REINSTALL => "Windows no le instaló el controlador",
        CM_PROB_UNSIGNED_DRIVER => "Windows no aceptó su firma",
        CM_PROB_FAILED_ADD | CM_PROB_FAILED_DRIVER_ENTRY => "el controlador no se pudo cargar",
        CM_PROB_FAILED_START => "Windows no lo pudo iniciar",
        CM_PROB_FAILED_POST_START => "el controlador informó de un problema",
        _ => "Windows informa de un problema",
    };
    format!("{what} (código {} en el Administrador de dispositivos)", problem.0)
}

/// Quita el adaptador, el paquete y la confianza en el certificado.
pub fn uninstall() -> Res<()> {
    if !unsafe { IsUserAnAdmin() }.as_bool() { return elevated(&["vdd", "uninstall"]); }
    let mut infs = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_DISPLAY), PCWSTR::null(), None, SETUP_DI_GET_CLASS_DEVS_FLAGS(0))
            .map_err(|e| format!("SetupDiGetClassDevs: {e}"))?;
        for info in device_infos(set) {
            if !is_kast(set, &info) { continue; }
            if let Some(inf) = driver_inf(set, &info) { infs.push(inf); }
            SetupDiCallClassInstaller(DIF_REMOVE, set, Some(&info)).map_err(|e| format!("no se pudo quitar el adaptador: {e}"))?;
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    infs.sort();
    infs.dedup();
    for inf in infs { run("pnputil", &["/delete-driver", &inf, "/uninstall", "/force"])?; }
    for store in ["Root", "TrustedPublisher"] { let _ = run("certutil", &["-delstore", store, CERT_NAME]); }
    vdisplay::forget_support();
    Ok(())
}

/// Lanza kast-link con los mismos argumentos y permiso de administrador, y espera su resultado (el mensaje de error, si
/// lo hay, lo deja en un archivo).
fn elevated(args: &[&str]) -> Res<()> {
    let exe = std::env::current_exe()?;
    let result = std::env::temp_dir().join(format!("kast-vdd-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&result);
    let params: Vec<String> = args.iter().map(|a| format!("\"{a}\"")).chain([format!("--result \"{}\"", result.display())]).collect();
    let (exe_w, params_w) = (wide(&exe.to_string_lossy()), wide(&params.join(" ")));
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe_w.as_ptr()),
        lpParameters: PCWSTR(params_w.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei) }.map_err(|_| "hace falta el permiso de administrador para instalar el controlador")?;
    let mut code = 1u32;
    unsafe {
        if WaitForSingleObject(sei.hProcess, INFINITE) == WAIT_OBJECT_0 { let _ = GetExitCodeProcess(sei.hProcess, &mut code); }
        let _ = CloseHandle(sei.hProcess);
    }
    vdisplay::forget_support();
    let msg = std::fs::read_to_string(&result).unwrap_or_default();
    let _ = std::fs::remove_file(&result);
    if code == 0 { Ok(()) } else { Err(if msg.trim().is_empty() { format!("falló (código {code})") } else { msg.trim().to_string() }.into()) }
}

/// Lo que dejó la instancia con permiso de administrador (ver `elevated`).
pub fn report(result: Option<&str>, r: &Res<()>) {
    if let (Some(path), Err(e)) = (result, r) { let _ = std::fs::write(path, e.to_string()); }
}

fn run(cmd: &str, args: &[&str]) -> Res<()> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() { return Ok(()); }
    let text = String::from_utf8_lossy(&out.stdout).lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").to_string();
    Err(format!("{cmd} {} falló: {text}", args.first().unwrap_or(&"")).into())
}

unsafe fn device_infos(set: HDEVINFO) -> Vec<SP_DEVINFO_DATA> {
    let mut v = Vec::new();
    for i in 0.. {
        let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        if unsafe { SetupDiEnumDeviceInfo(set, i, &mut info) }.is_err() { break; }
        v.push(info);
    }
    v
}

/// ¿Es un adaptador de Kast? (Su identificador de hardware.)
unsafe fn is_kast(set: HDEVINFO, info: &SP_DEVINFO_DATA) -> bool {
    let mut buf = [0u8; 1024];
    let mut size = 0u32;
    if unsafe { SetupDiGetDeviceRegistryPropertyW(set, info, SPDRP_HARDWAREID, None, Some(&mut buf), Some(&mut size)) }.is_err() { return false; }
    let ws: Vec<u16> = buf[..size as usize].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    String::from_utf16_lossy(&ws).split('\0').any(|id| id.eq_ignore_ascii_case(HWID))
}

/// Adaptadores de Kast (presentes o no), por su nodo de dispositivo.
fn devices() -> Vec<u32> {
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_DISPLAY), PCWSTR::null(), None, SETUP_DI_GET_CLASS_DEVS_FLAGS(0)) else { return Vec::new() };
        let n = device_infos(set).iter().filter(|i| is_kast(set, i)).map(|i| i.DevInst).collect();
        let _ = SetupDiDestroyDeviceInfoList(set);
        n
    }
}

/// El .inf publicado (oemNN.inf) del controlador de un adaptador.
unsafe fn driver_inf(set: HDEVINFO, info: &SP_DEVINFO_DATA) -> Option<String> {
    let key: HKEY = unsafe { SetupDiOpenDevRegKey(set, info, DICS_FLAG_GLOBAL.0, 0, DIREG_DRV, KEY_READ.0) }.ok()?;
    let mut buf = [0u16; 260];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(key, PCWSTR::null(), w!("InfPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) };
    unsafe { let _ = RegCloseKey(key); }
    r.is_ok().then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
}

/// Crea el adaptador raíz «Kast Virtual Display» (lo que hace `devcon install`).
fn create_device() -> Res<()> {
    unsafe {
        let set = SetupDiCreateDeviceInfoList(Some(&GUID_DEVCLASS_DISPLAY), Some(HWND::default())).map_err(|e| format!("SetupDiCreateDeviceInfoList: {e}"))?;
        let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        let r = (|| -> Res<()> {
            SetupDiCreateDeviceInfoW(set, w!("Display"), &GUID_DEVCLASS_DISPLAY, PCWSTR::null(), None, DICD_GENERATE_ID, Some(&mut info))
                .map_err(|e| format!("SetupDiCreateDeviceInfo: {e}"))?;
            // Identificador de hardware: lista de cadenas terminada en dos ceros.
            let ids: Vec<u8> = HWID.encode_utf16().chain([0, 0]).flat_map(u16::to_le_bytes).collect();
            SetupDiSetDeviceRegistryPropertyW(set, &mut info, SPDRP_HARDWAREID, Some(&ids)).map_err(|e| format!("SetupDiSetDeviceRegistryProperty: {e}"))?;
            SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set, Some(&info)).map_err(|e| format!("no se pudo crear el adaptador: {e}"))?;
            Ok(())
        })();
        let _ = SetupDiDestroyDeviceInfoList(set);
        r
    }
}
