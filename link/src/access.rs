//! Quién puede entrar en este equipo y cómo: política para dispositivos desconocidos, contraseña de acceso
//! desatendido, códigos de emparejamiento de un solo uso y límite de intentos.
//!
//! La contraseña y el código nunca viajan: el anfitrión envía una sal (CHALLENGE) y el visor responde con
//! MAC(Argon2id(secreto, sal), binding), donde `binding` es el hash del intercambio Noise de *esta* conexión
//! (net::Secure). Una prueba capturada o reenviada no sirve en otra conexión. La contraseña se guarda sólo
//! como (sal, Argon2id), nunca en claro.
use crate::store;
use blake2::Blake2sMac256;
use blake2::digest::{KeyInit, Mac};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const PASSWORD: &str = "password";
pub const PAIR: &str = "pair";
const MIN_PASSWORD: usize = 8;
/// Caracteres del código (sin 0/O, 1/I/L ni U): 8 caracteres ≈ 39 bits.
const CODE_CHARS: &[u8] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_TTL: Duration = Duration::from_secs(10 * 60);
const CODE_TRIES: u32 = 5;
const WINDOW: Duration = Duration::from_secs(10 * 60);
const LOCKOUT: Duration = Duration::from_secs(5 * 60);
const PEER_FAILS: usize = 5;
const GLOBAL_FAILS: usize = 20;

/// Clave derivada del secreto (Argon2id, 19 MiB, 2 pasadas): lenta a propósito contra ataques de diccionario.
pub fn derive(secret: &str, salt: &[u8]) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(19 * 1024, 2, 1, Some(32)).expect("parámetros de Argon2");
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(secret.as_bytes(), salt, &mut out)
        .expect("Argon2");
    out
}

fn mac(key: &[u8], binding: &[u8]) -> Blake2sMac256 {
    let mut m = <Blake2sMac256 as KeyInit>::new_from_slice(key).expect("clave de 32 bytes");
    m.update(b"kast-link auth v1");
    m.update(binding);
    m
}

/// Prueba que envía el visor.
pub fn proof(key: &[u8], binding: &[u8]) -> Vec<u8> { mac(key, binding).finalize().into_bytes().to_vec() }

/// Comprobación en tiempo constante.
pub fn verify(key: &[u8], binding: &[u8], proof: &[u8]) -> bool { mac(key, binding).verify_slice(proof).is_ok() }

/// Código tal como se escribe: sin guiones ni espacios y en mayúsculas.
pub fn normalize_code(s: &str) -> String { s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect() }

fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("aleatorios del sistema");
    b
}

// ---------------------------------------------------------------- lado del visor

/// Respuesta (mensaje AUTH completo) al CHALLENGE del anfitrión, sin enviar nunca el secreto.
pub fn answer_challenge(challenge: &[u8], secret: &str, binding: &[u8]) -> Result<Vec<u8>, String> {
    let v: Value = serde_json::from_slice(challenge).map_err(|_| "reto inválido")?;
    let salt = v["salt"].as_str().and_then(crate::net::unhex).filter(|s| !s.is_empty()).ok_or("reto inválido")?;
    let secret = if v["kind"] == PAIR { normalize_code(secret) } else { secret.to_string() };
    if secret.is_empty() {
        return Err(if v["kind"] == PAIR { "hace falta el código de emparejamiento" } else { "hace falta la contraseña" }.into());
    }
    let mut m = vec![crate::net::msg::AUTH];
    m.extend_from_slice(json!({"proof": crate::net::hex(&proof(&derive(&secret, &salt), binding))}).to_string().as_bytes());
    Ok(m)
}

/// Motivo de un REJECT y si el equipo pide contraseña o código (JSON {"msg","need_auth"}; antes era texto).
pub fn reject_reason(body: &[u8]) -> (String, bool) {
    match serde_json::from_slice::<Value>(body) {
        Ok(v) if v.is_object() => (v["msg"].as_str().unwrap_or("conexión rechazada").to_string(), v["need_auth"].as_bool().unwrap_or(false)),
        _ => (String::from_utf8_lossy(body).into_owned(), false),
    }
}

/// Cuerpo de un REJECT: texto (compatible con versiones anteriores) o, si pide contraseña o código, JSON.
pub fn reject_body(msg: &str, need_auth: bool) -> Vec<u8> {
    if need_auth { json!({"msg": msg, "need_auth": true}).to_string().into_bytes() } else { msg.as_bytes().to_vec() }
}

// ---------------------------------------------------------------- configuración persistente

/// Configuración de acceso (link-access.json junto a la configuración de Kast).
#[derive(Clone)]
pub struct Access {
    /// Peticiones de dispositivos desconocidos: false = preguntar (por defecto), true = rechazar
    /// (sólo entran los emparejados o con la contraseña).
    pub reject_unknown: bool,
    /// Contraseña de acceso desatendido: (sal, Argon2id).
    password: Option<(Vec<u8>, Vec<u8>)>,
    /// Lo que permite la contraseña: ver y controlar (true) o sólo ver.
    pub password_control: bool,
}

impl Default for Access {
    fn default() -> Self { Self { reject_unknown: false, password: None, password_control: true } }
}

fn path() -> std::path::PathBuf { store::config_dir().join("link-access.json") }

impl Access {
    pub fn load() -> Self {
        let Ok(s) = std::fs::read_to_string(path()) else { return Self::default() };
        let v: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
        let hex = |k: &str| v["password"][k].as_str().and_then(crate::net::unhex);
        Self {
            reject_unknown: v["unknown"] == "reject",
            password: hex("salt").zip(hex("key")).filter(|(s, k)| !s.is_empty() && k.len() == 32),
            password_control: v["password"]["control"].as_bool().unwrap_or(true),
        }
    }

    pub fn save(&self) {
        let pw = self.password.as_ref().map(|(s, k)| json!({"salt": crate::net::hex(s), "key": crate::net::hex(k), "control": self.password_control}));
        let v = json!({"unknown": if self.reject_unknown { "reject" } else { "ask" }, "password": pw});
        let _ = std::fs::create_dir_all(store::config_dir());
        let p = path();
        let _ = std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default());
        store::restrict(&p);
    }

    pub fn unattended(&self) -> bool { self.password.is_some() }

    /// Activa o cambia la contraseña de acceso desatendido.
    pub fn set_password(&mut self, pw: &str) -> Result<(), String> {
        if pw.chars().count() < MIN_PASSWORD { return Err(format!("La contraseña debe tener al menos {MIN_PASSWORD} caracteres")); }
        let salt = random(16);
        let key = derive(pw, &salt).to_vec();
        self.password = Some((salt, key));
        Ok(())
    }

    pub fn clear_password(&mut self) { self.password = None; }

    /// (sal, clave) de la contraseña, si está activada.
    pub fn password_key(&self) -> Option<(Vec<u8>, Vec<u8>)> { self.password.clone() }

    /// Para el estado de la app (sin la contraseña).
    pub fn to_json(&self) -> Value {
        json!({"unknown": if self.reject_unknown { "reject" } else { "ask" }, "unattended": self.unattended(), "unattended_control": self.password_control})
    }
}

// ---------------------------------------------------------------- código de emparejamiento

/// Código de un solo uso para dar acceso permanente a un dispositivo (sólo en memoria).
pub struct PairCode {
    /// Código para mostrar ("K7QM-4ZP2").
    pub code: String,
    salt: Vec<u8>,
    key: [u8; 32],
    /// Permiso del dispositivo emparejado: ver y controlar (true) o sólo ver.
    pub control: bool,
    expires: Instant,
    tries: u32,
}

impl PairCode {
    pub fn new(control: bool) -> Self {
        let raw: String = random(8).iter().map(|b| CODE_CHARS[*b as usize % CODE_CHARS.len()] as char).collect();
        let salt = random(16);
        let key = derive(&raw, &salt);
        Self { code: format!("{}-{}", &raw[..4], &raw[4..]), salt, key, control, expires: Instant::now() + CODE_TTL, tries: 0 }
    }

    pub fn expired(&self) -> bool { Instant::now() >= self.expires || self.tries >= CODE_TRIES }
    pub fn secs_left(&self) -> u64 { self.expires.saturating_duration_since(Instant::now()).as_secs() }
    pub fn salt(&self) -> &[u8] { &self.salt }

    /// Comprueba una prueba; cada fallo gasta un intento.
    pub fn check(&mut self, binding: &[u8], proof: &[u8]) -> bool {
        if self.expired() { return false; }
        let ok = verify(&self.key, binding, proof);
        if !ok { self.tries += 1; }
        ok
    }
}

// ---------------------------------------------------------------- límite de intentos

/// Fallos recientes por dispositivo (clave pública) y en total: frena los ataques de diccionario.
#[derive(Default)]
pub struct Limiter {
    fails: HashMap<String, Vec<Instant>>,
    all: Vec<Instant>,
}

impl Limiter {
    fn prune(v: &mut Vec<Instant>) { v.retain(|t| t.elapsed() < WINDOW); }

    /// Si está bloqueado, cuánto falta.
    pub fn blocked(&mut self, peer: &str) -> Option<Duration> {
        let mut worst: Option<Duration> = None;
        let v = self.fails.entry(peer.to_string()).or_default();
        Self::prune(v);
        if v.len() >= PEER_FAILS { worst = Some(LOCKOUT.saturating_sub(v.last().unwrap().elapsed())); }
        Self::prune(&mut self.all);
        if self.all.len() >= GLOBAL_FAILS {
            let left = LOCKOUT.saturating_sub(self.all.last().unwrap().elapsed());
            worst = Some(worst.map_or(left, |w| w.max(left)));
        }
        worst.filter(|d| !d.is_zero())
    }

    pub fn fail(&mut self, peer: &str) {
        self.fails.entry(peer.to_string()).or_default().push(Instant::now());
        self.all.push(Instant::now());
    }

    pub fn ok(&mut self, peer: &str) { self.fails.remove(peer); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prueba_ligada_a_la_conexion() {
        let salt = random(16);
        let k = derive("correcta horse", &salt);
        let p = proof(&k, b"conexion-1");
        assert!(verify(&k, b"conexion-1", &p));
        // Reenviada en otra conexión, o con otra contraseña: no vale.
        assert!(!verify(&k, b"conexion-2", &p));
        assert!(!verify(&derive("incorrecta", &salt), b"conexion-1", &p));
    }

    #[test]
    fn codigo_de_un_solo_uso() {
        let mut c = PairCode::new(true);
        assert_eq!(c.code.len(), 9);
        let raw = normalize_code(&c.code.to_lowercase());
        let k = derive(&raw, c.salt());
        for _ in 0..CODE_TRIES - 1 { assert!(!c.check(b"b", b"mal")); }
        assert!(c.check(b"b", &proof(&k, b"b")));
        // Con todos los intentos gastados deja de valer, aunque la prueba sea buena.
        assert!(!c.check(b"b", b"mal"));
        assert!(c.expired());
        assert!(!c.check(b"b", &proof(&k, b"b")));
    }

    #[test]
    fn reto_y_respuesta() {
        let mut c = PairCode::new(false);
        let challenge = json!({"kind": PAIR, "salt": crate::net::hex(c.salt())}).to_string();
        // El código se escribe como sea (minúsculas, sin guion): vale igual.
        let typed = c.code.to_lowercase().replace('-', " ");
        let auth = answer_challenge(challenge.as_bytes(), &typed, b"enlace").unwrap();
        let v: Value = serde_json::from_slice(&auth[1..]).unwrap();
        assert!(c.check(b"enlace", &crate::net::unhex(v["proof"].as_str().unwrap()).unwrap()));
        assert!(answer_challenge(challenge.as_bytes(), " - ", b"enlace").is_err());
        assert_eq!(reject_reason(b"rechazada"), ("rechazada".into(), false));
        assert_eq!(reject_reason(&reject_body("hace falta", true)), ("hace falta".into(), true));
    }

    #[test]
    fn bloqueo_por_intentos() {
        let mut l = Limiter::default();
        for _ in 0..PEER_FAILS - 1 { l.fail("a"); }
        assert!(l.blocked("a").is_none());
        l.fail("a");
        assert!(l.blocked("a").is_some());
        assert!(l.blocked("b").is_none());
        l.ok("a");
        assert!(l.blocked("a").is_none());
    }

    #[test]
    fn contrasena_persistida() {
        let dir = std::env::temp_dir().join(format!("kast-access-{}", std::process::id()));
        store::set_config(dir.clone(), "prueba".into());
        let mut a = Access::default();
        assert!(a.set_password("corta").is_err());
        a.set_password("larga y buena").unwrap();
        a.reject_unknown = true;
        a.password_control = false;
        a.save();
        let b = Access::load();
        assert!(b.reject_unknown && b.unattended() && !b.password_control);
        let (salt, key) = b.password_key().unwrap();
        assert_eq!(derive("larga y buena", &salt).to_vec(), key);
        assert!(!std::fs::read_to_string(path()).unwrap().contains("larga"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
