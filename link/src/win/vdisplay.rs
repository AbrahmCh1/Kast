//! Pantallas virtuales en Windows: hace falta un controlador de pantalla indirecta (IddCx) instalado en el equipo,
//! así que de momento no se ofrecen (el visor lo sabe por el anuncio del equipo).
use crate::Res;

pub struct Virtual {
    pub id: u32,
}

pub fn supported() -> bool { false }

pub fn create(_name: &str, _px: (u32, u32), _hidpi: bool) -> Res<Virtual> {
    Err("Windows todavía no permite crear pantallas virtuales desde Kast".into())
}

pub fn arrange(_origins: &[(u32, i32, i32)]) -> Res<()> { Ok(()) }
