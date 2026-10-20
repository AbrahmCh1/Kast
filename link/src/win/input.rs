//! Teclado y ratón remotos en Windows con SendInput (códigos de escaneo: respeta la distribución
//! de teclado de este equipo). No puede actuar sobre ventanas de administrador ni sobre avisos de UAC.
use super::capture;
use crate::Res;
use crate::input::Event;
use std::collections::HashSet;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Código W3C -> (código de escaneo, tecla extendida).
fn scancode(code: &str) -> Option<(u16, bool)> {
    let letters = "QWERTYUIOP";
    let home = "ASDFGHJKL";
    let bottom = "ZXCVBNM";
    if let Some(c) = code.strip_prefix("Key").and_then(|c| c.chars().next()) {
        if let Some(i) = letters.find(c) { return Some((0x10 + i as u16, false)); }
        if let Some(i) = home.find(c) { return Some((0x1E + i as u16, false)); }
        if let Some(i) = bottom.find(c) { return Some((0x2C + i as u16, false)); }
    }
    if let Some(d) = code.strip_prefix("Digit").and_then(|d| d.parse::<u16>().ok()) {
        return Some((if d == 0 { 0x0B } else { 0x01 + d }, false));
    }
    if let Some(n) = code.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        return match n {
            1..=10 => Some((0x3A + n, false)),
            11 => Some((0x57, false)),
            12 => Some((0x58, false)),
            13..=23 => Some((0x64 + n - 13, false)),
            24 => Some((0x76, false)),
            _ => None,
        };
    }
    let (s, e) = match code {
        "Escape" => (0x01, false), "Minus" => (0x0C, false), "Equal" => (0x0D, false), "Backspace" => (0x0E, false),
        "Tab" => (0x0F, false), "BracketLeft" => (0x1A, false), "BracketRight" => (0x1B, false), "Enter" => (0x1C, false),
        "ControlLeft" => (0x1D, false), "Semicolon" => (0x27, false), "Quote" => (0x28, false), "Backquote" => (0x29, false),
        "ShiftLeft" => (0x2A, false), "Backslash" => (0x2B, false), "Comma" => (0x33, false), "Period" => (0x34, false),
        "Slash" => (0x35, false), "ShiftRight" => (0x36, false), "NumpadMultiply" => (0x37, false), "AltLeft" => (0x38, false),
        "Space" => (0x39, false), "CapsLock" => (0x3A, false), "ScrollLock" => (0x46, false),
        "Numpad7" => (0x47, false), "Numpad8" => (0x48, false), "Numpad9" => (0x49, false), "NumpadSubtract" => (0x4A, false),
        "Numpad4" => (0x4B, false), "Numpad5" => (0x4C, false), "Numpad6" => (0x4D, false), "NumpadAdd" => (0x4E, false),
        "Numpad1" => (0x4F, false), "Numpad2" => (0x50, false), "Numpad3" => (0x51, false), "Numpad0" => (0x52, false),
        "NumpadDecimal" => (0x53, false), "IntlBackslash" => (0x56, false), "NumpadEqual" => (0x59, false),
        "IntlRo" => (0x73, false), "IntlYen" => (0x7D, false), "NumpadComma" => (0x7E, false),
        "NumpadEnter" => (0x1C, true), "ControlRight" => (0x1D, true), "NumpadDivide" => (0x35, true),
        "PrintScreen" => (0x37, true), "AltRight" => (0x38, true), "Home" => (0x47, true), "ArrowUp" => (0x48, true),
        "PageUp" => (0x49, true), "ArrowLeft" => (0x4B, true), "ArrowRight" => (0x4D, true), "End" => (0x4F, true),
        "ArrowDown" => (0x50, true), "PageDown" => (0x51, true), "Insert" => (0x52, true), "Delete" => (0x53, true),
        "SuperLeft" | "MetaLeft" => (0x5B, true), "SuperRight" | "MetaRight" => (0x5C, true), "ContextMenu" => (0x5D, true),
        _ => return None,
    };
    Some((s, e))
}

fn send(inputs: &[INPUT]) { unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) }; }

fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as _, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn key(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

pub struct Injector {
    /// Esquina del monitor compartido en el escritorio virtual y escala imagen -> pantalla.
    ox: i32,
    oy: i32,
    sx: f64,
    sy: f64,
    keys: HashSet<String>,
    buttons: [bool; 5],
}

impl Injector {
    pub fn new(monitor: usize, w: u32, h: u32) -> Res<Self> {
        let outs = capture::outputs()?;
        let o = outs.get(monitor.wrapping_sub(1)).ok_or("monitor inexistente")?;
        Ok(Self { ox: o.x, oy: o.y, sx: o.w as f64 / w.max(1) as f64, sy: o.h as f64 / h.max(1) as f64, keys: HashSet::new(), buttons: [false; 5] })
    }

    /// Coordenadas absolutas normalizadas (0..65535) sobre todo el escritorio virtual.
    fn absolute(&self, x: i32, y: i32) -> (i32, i32) {
        let (vx, vy, vw, vh) = unsafe {
            (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN),
             GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2), GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2))
        };
        let px = self.ox as f64 + x as f64 * self.sx;
        let py = self.oy as f64 + y as f64 * self.sy;
        (((px - vx as f64) * 65535.0 / (vw - 1) as f64).round() as i32, ((py - vy as f64) * 65535.0 / (vh - 1) as f64).round() as i32)
    }

    fn move_to(&self, x: i32, y: i32) -> INPUT {
        let (nx, ny) = self.absolute(x, y);
        mouse(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)
    }

    pub fn apply(&mut self, e: Event) {
        match e {
            Event::Move { x, y } => send(&[self.move_to(x, y)]),
            Event::Button { button, down, x, y } => {
                let b = button as usize;
                if b >= 5 { return; }
                self.buttons[b] = down;
                let (flags, data) = match (button, down) {
                    (0, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                    (0, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (_, true) => (MOUSEEVENTF_XDOWN, if button == 3 { 1 } else { 2 }),
                    (_, false) => (MOUSEEVENTF_XUP, if button == 3 { 1 } else { 2 }),
                };
                send(&[self.move_to(x, y), mouse(0, 0, data, flags)]);
            }
            Event::Wheel { dx, dy } => {
                if dy != 0 { send(&[mouse(0, 0, dy, MOUSEEVENTF_WHEEL)]); }
                if dx != 0 { send(&[mouse(0, 0, dx, MOUSEEVENTF_HWHEEL)]); }
            }
            Event::Key { down, code } => {
                if down { self.keys.insert(code.clone()); } else { self.keys.remove(&code); }
                let up = if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP };
                // Bloq Num y Pausa se envían por tecla virtual: sus códigos de escaneo son ambiguos.
                match code.as_str() {
                    "NumLock" => return send(&[key(VK_NUMLOCK, 0, up)]),
                    "Pause" => return send(&[key(VK_PAUSE, 0, up)]),
                    _ => {}
                }
                if let Some((scan, ext)) = scancode(&code) {
                    let flags = KEYEVENTF_SCANCODE | up | if ext { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
                    send(&[key(VIRTUAL_KEY(0), scan, flags)]);
                }
            }
        }
    }
}

impl Drop for Injector {
    /// Al terminar la sesión no puede quedar nada pulsado.
    fn drop(&mut self) {
        for code in std::mem::take(&mut self.keys) { self.apply(Event::Key { down: false, code }); }
        for b in 0..5 {
            if self.buttons[b] { self.apply(Event::Button { button: b as u8, down: false, x: 0, y: 0 }); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scancode;

    #[test]
    fn scancodes() {
        assert_eq!(scancode("KeyA"), Some((0x1E, false)));
        assert_eq!(scancode("KeyQ"), Some((0x10, false)));
        assert_eq!(scancode("KeyM"), Some((0x32, false)));
        assert_eq!(scancode("Digit1"), Some((0x02, false)));
        assert_eq!(scancode("Digit0"), Some((0x0B, false)));
        assert_eq!(scancode("F1"), Some((0x3B, false)));
        assert_eq!(scancode("F10"), Some((0x44, false)));
        assert_eq!(scancode("F12"), Some((0x58, false)));
        assert_eq!(scancode("ArrowUp"), Some((0x48, true)));
        assert_eq!(scancode("SuperLeft"), Some((0x5B, true)));
    }
}
