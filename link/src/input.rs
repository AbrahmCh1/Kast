//! Teclado y ratón remotos: eventos que el visor envía al equipo que comparte.
//! Las teclas viajan como su posición física en el estándar W3C (`KeyA`, `ShiftLeft`, `Digit1`…),
//! independiente del sistema; cada plataforma las traduce a sus propios códigos (win/input.rs, mac/input.rs).
//! Las coordenadas del ratón son píxeles de la imagen transmitida.

pub enum Event {
    Move { x: i32, y: i32 },
    /// Botón: 0 izquierdo, 1 derecho, 2 central, 3 atrás, 4 adelante.
    Button { button: u8, down: bool, x: i32, y: i32 },
    /// Rueda en 1/120 de "muesca" (120 = una línea), como en Windows.
    Wheel { dx: i32, dy: i32 },
    Key { down: bool, code: String },
    /// Texto que se escribe tal cual, sin depender de la distribución del teclado remoto (teclado de iOS).
    Text { text: String },
}

/// Máximo de bytes de texto por mensaje (la app trocea lo más largo).
pub const MAX_TEXT: usize = 4000;

const MOVE: u8 = 1;
const BUTTON: u8 = 2;
const WHEEL: u8 = 3;
const KEY: u8 = 4;
const TEXT: u8 = 5;

impl Event {
    /// Mensaje completo (con el tipo INPUT delante).
    pub fn encode(&self) -> Vec<u8> {
        let mut m = vec![crate::net::msg::INPUT];
        match self {
            Event::Move { x, y } => { m.push(MOVE); m.extend_from_slice(&x.to_be_bytes()); m.extend_from_slice(&y.to_be_bytes()); }
            Event::Button { button, down, x, y } => {
                m.extend_from_slice(&[BUTTON, *button, *down as u8]);
                m.extend_from_slice(&x.to_be_bytes());
                m.extend_from_slice(&y.to_be_bytes());
            }
            Event::Wheel { dx, dy } => { m.push(WHEEL); m.extend_from_slice(&dx.to_be_bytes()); m.extend_from_slice(&dy.to_be_bytes()); }
            Event::Key { down, code } => {
                let c = &code.as_bytes()[..code.len().min(32)];
                m.extend_from_slice(&[KEY, *down as u8, c.len() as u8]);
                m.extend_from_slice(c);
            }
            Event::Text { text } => {
                let mut end = text.len().min(MAX_TEXT);
                while !text.is_char_boundary(end) { end -= 1; }
                m.push(TEXT);
                m.extend_from_slice(&(end as u16).to_be_bytes());
                m.extend_from_slice(&text.as_bytes()[..end]);
            }
        }
        m
    }

    /// Decodifica el cuerpo de un mensaje INPUT (sin el tipo).
    pub fn decode(b: &[u8]) -> Option<Event> {
        let i32_at = |i: usize| b.get(i..i + 4).map(|s| i32::from_be_bytes(s.try_into().unwrap()));
        match *b.first()? {
            MOVE => Some(Event::Move { x: i32_at(1)?, y: i32_at(5)? }),
            BUTTON => Some(Event::Button { button: *b.get(1)?, down: *b.get(2)? != 0, x: i32_at(3)?, y: i32_at(7)? }),
            WHEEL => Some(Event::Wheel { dx: i32_at(1)?, dy: i32_at(5)? }),
            KEY => {
                let len = *b.get(2)? as usize;
                let code = std::str::from_utf8(b.get(3..3 + len)?).ok()?.to_string();
                Some(Event::Key { down: *b.get(1)? != 0, code })
            }
            TEXT => {
                let len = u16::from_be_bytes(b.get(1..3)?.try_into().ok()?) as usize;
                Some(Event::Text { text: std::str::from_utf8(b.get(3..3 + len)?).ok()?.to_string() })
            }
            _ => None,
        }
    }
}

/// Entre Windows y Mac se intercambian Ctrl y ⌘ para que los atajos se escriban igual
/// (Ctrl+C en Windows → ⌘C en Mac y al revés). iOS usa los modificadores de Apple, como macOS; Android, Ctrl,
/// como Windows.
pub fn map_modifier(code: &str, viewer: &str, host: &str) -> String {
    fn family(p: &str) -> &str { match p { "ios" => "mac", "android" => "windows", p => p } }
    if family(viewer) == family(host) || viewer.is_empty() { return code.to_string(); }
    match code {
        "ControlLeft" => "SuperLeft",
        "ControlRight" => "SuperRight",
        "SuperLeft" | "MetaLeft" => "ControlLeft",
        "SuperRight" | "MetaRight" => "ControlRight",
        c => c,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for e in [Event::Move { x: 10, y: -3 }, Event::Button { button: 1, down: true, x: 5, y: 6 }, Event::Wheel { dx: 0, dy: -120 },
                  Event::Key { down: false, code: "ShiftLeft".into() }, Event::Text { text: "Hola, ñandú 😀".into() }] {
            let m = e.encode();
            let d = Event::decode(&m[1..]).unwrap();
            assert_eq!(d.encode(), m);
        }
    }

    #[test]
    fn swaps_ctrl_and_cmd_between_systems() {
        assert_eq!(map_modifier("ControlLeft", "windows", "mac"), "SuperLeft");
        assert_eq!(map_modifier("SuperLeft", "mac", "windows"), "ControlLeft");
        assert_eq!(map_modifier("ControlLeft", "windows", "windows"), "ControlLeft");
        assert_eq!(map_modifier("KeyA", "windows", "mac"), "KeyA");
        // iOS tiene los modificadores de Apple: igual que un Mac.
        assert_eq!(map_modifier("SuperLeft", "ios", "mac"), "SuperLeft");
        assert_eq!(map_modifier("SuperLeft", "ios", "windows"), "ControlLeft");
        // Android usa Ctrl para los atajos, como Windows.
        assert_eq!(map_modifier("ControlLeft", "windows", "android"), "ControlLeft");
        assert_eq!(map_modifier("SuperLeft", "mac", "android"), "ControlLeft");
        assert_eq!(map_modifier("ControlLeft", "android", "mac"), "SuperLeft");
    }
}
