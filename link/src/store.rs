//! Datos persistentes de Kast Link, junto a la configuración de Kast:
//! la identidad del equipo y los dispositivos conocidos (de confianza y agregados por IP).
use crate::net::{self, Identity};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Carpeta y nombre fijados por la app (iOS: su carpeta de documentos y el nombre del dispositivo).
static DIR: OnceLock<PathBuf> = OnceLock::new();
static NAME: OnceLock<String> = OnceLock::new();

pub fn set_config(dir: PathBuf, name: String) {
    let _ = DIR.set(dir);
    let _ = NAME.set(name);
}

/// %APPDATA%\Kast en Windows, ~/Library/Application Support/Kast en macOS (KAST_LINK_HOME para pruebas).
pub fn config_dir() -> PathBuf {
    if let Some(d) = DIR.get() { return d.clone(); }
    if let Some(d) = std::env::var_os("KAST_LINK_HOME") { return d.into(); }
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into()).join("Kast");
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| ".".into()).join("Library/Application Support/Kast");
    base
}

pub fn identity() -> Identity {
    let path = config_dir().join("link-identity.key");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let mut l = s.lines();
        if let (Some(p), Some(q)) = (l.next().and_then(net::unhex), l.next().and_then(net::unhex)) {
            if p.len() == 32 && q.len() == 32 { return Identity { private: p, public: q }; }
        }
    }
    let id = Identity::generate();
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(&path, format!("{}\n{}\n", net::hex(&id.private), net::hex(&id.public)));
    restrict(&path);
    id
}

#[cfg(unix)]
fn restrict(p: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict(_: &std::path::Path) {}

/// Nombre visible del equipo.
pub fn device_name() -> String {
    if let Some(n) = NAME.get() { return n.clone(); }
    #[cfg(windows)]
    { std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into()) }
    #[cfg(not(windows))]
    {
        std::process::Command::new("scutil").args(["--get", "ComputerName"]).output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()).unwrap_or_else(|| "Mac".into())
    }
}

pub fn platform() -> &'static str { if cfg!(windows) { "windows" } else if cfg!(target_os = "ios") { "ios" } else { "mac" } }

/// Registro de diagnóstico (link.log junto a la configuración; se reinicia con cada arranque del agente).
pub fn log(msg: &str) {
    use std::io::Write;
    eprintln!("{msg}");
    let p = config_dir().join("link.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "[{:02}:{:02}:{:02} UTC] {msg}", secs / 3600 % 24, secs / 60 % 60, secs % 60);
    }
}

pub fn log_reset() {
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(config_dir().join("link.log"), format!("kast-link {} ({})\n", env!("CARGO_PKG_VERSION"), platform()));
}

/// Dispositivo recordado.
#[derive(Clone, Default)]
pub struct Saved {
    /// Clave pública (hex) si ya se conoce.
    pub key: String,
    pub name: String,
    pub platform: String,
    /// "ip:puerto" agregada a mano o última conocida.
    pub addr: String,
    /// Puede ver mis pantallas sin preguntar.
    pub trusted: bool,
    /// Agregado a mano (se le pregunta directamente aunque no haya difusión).
    pub manual: bool,
    /// Además de ver, puede controlar teclado y ratón sin preguntar.
    pub control: bool,
}

/// Comprueba la identidad de un equipo al conectar: debe ser quien dice ser (`id` del anuncio, salvo los
/// agregados por IP) y no puede haber cambiado desde la última vez (TOFU).
pub fn check_peer(saved: &[Saved], id: &str, peer: &[u8]) -> Result<(), &'static str> {
    let (peer_key, peer_id) = (net::hex(peer), net::id_of(peer));
    if !id.starts_with("ip:") && peer_id != id { return Err("la identidad del dispositivo no coincide"); }
    match saved.iter().find(|s| !s.key.is_empty() && net::id_of(&net::unhex(&s.key).unwrap_or_default()) == peer_id) {
        Some(s) if s.key != peer_key => Err("la identidad del dispositivo cambió: olvídalo y vuelve a agregarlo"),
        _ => Ok(()),
    }
}

fn devices_path() -> PathBuf { config_dir().join("link-devices.tsv") }

pub fn load_devices() -> Vec<Saved> {
    let Ok(s) = std::fs::read_to_string(devices_path()) else { return Vec::new() };
    s.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 6).then(|| Saved {
                key: f[0].into(), name: f[1].into(), platform: f[2].into(), addr: f[3].into(),
                trusted: f[4] == "1", manual: f[5] == "1", control: f.get(6) == Some(&"1"),
            })
        })
        .collect()
}

pub fn save_devices(list: &[Saved]) {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let body: String = list.iter()
        .map(|d| format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\n", d.key, clean(&d.name), d.platform, d.addr, d.trusted as u8, d.manual as u8, d.control as u8))
        .collect();
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(devices_path(), body);
}
