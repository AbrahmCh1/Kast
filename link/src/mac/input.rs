//! Teclado y ratón remotos en macOS con CGEvent. Requiere el permiso de Accesibilidad para Kast.
use crate::Res;
use crate::input::Event;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, kCFBooleanTrue};
use objc2_core_graphics::{CGDirectDisplayID, CGDisplayBounds, CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventType,
                          CGMouseButton, CGScrollEventUnit};
use objc2_core_foundation::CGPoint;
use std::collections::HashSet;
use std::ffi::c_void;
use std::time::{Duration, Instant};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}

/// Some(false) si falta el permiso de Accesibilidad (macOS lo exige para controlar teclado y ratón).
pub fn input_allowed() -> Option<bool> { Some(unsafe { AXIsProcessTrusted() }) }

/// Pide el permiso (aviso del sistema) y abre su panel en Ajustes del Sistema.
pub fn request_input() {
    // Kast se firma ad hoc: tras actualizarlo, macOS puede mostrarlo activado en Accesibilidad sin confiar
    // en la versión nueva. Se borra esa entrada caducada para que el aviso la vuelva a crear.
    if !unsafe { AXIsProcessTrusted() } {
        let _ = std::process::Command::new("tccutil").args(["reset", "Accessibility", "io.github.abrahmch1.kast"]).status();
    }
    let trusted = unsafe {
        let key = &*(kAXTrustedCheckOptionPrompt as *const CFString);
        let yes: &CFBoolean = kCFBooleanTrue.unwrap();
        let opts = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[yes]);
        AXIsProcessTrustedWithOptions(opts.as_opaque() as *const _ as *const c_void)
    };
    if !trusted {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

/// Código W3C -> código de tecla virtual de macOS (kVK_*), por posición física.
fn keycode(code: &str) -> Option<u16> {
    Some(match code {
        "KeyA" => 0x00, "KeyS" => 0x01, "KeyD" => 0x02, "KeyF" => 0x03, "KeyH" => 0x04, "KeyG" => 0x05, "KeyZ" => 0x06,
        "KeyX" => 0x07, "KeyC" => 0x08, "KeyV" => 0x09, "IntlBackslash" => 0x0A, "KeyB" => 0x0B, "KeyQ" => 0x0C,
        "KeyW" => 0x0D, "KeyE" => 0x0E, "KeyR" => 0x0F, "KeyY" => 0x10, "KeyT" => 0x11, "Digit1" => 0x12, "Digit2" => 0x13,
        "Digit3" => 0x14, "Digit4" => 0x15, "Digit6" => 0x16, "Digit5" => 0x17, "Equal" => 0x18, "Digit9" => 0x19,
        "Digit7" => 0x1A, "Minus" => 0x1B, "Digit8" => 0x1C, "Digit0" => 0x1D, "BracketRight" => 0x1E, "KeyO" => 0x1F,
        "KeyU" => 0x20, "BracketLeft" => 0x21, "KeyI" => 0x22, "KeyP" => 0x23, "Enter" => 0x24, "KeyL" => 0x25,
        "KeyJ" => 0x26, "Quote" => 0x27, "KeyK" => 0x28, "Semicolon" => 0x29, "Backslash" => 0x2A, "Comma" => 0x2B,
        "Slash" => 0x2C, "KeyN" => 0x2D, "KeyM" => 0x2E, "Period" => 0x2F, "Tab" => 0x30, "Space" => 0x31,
        "Backquote" => 0x32, "Backspace" => 0x33, "Escape" => 0x35, "SuperRight" | "MetaRight" => 0x36,
        "SuperLeft" | "MetaLeft" => 0x37, "ShiftLeft" => 0x38, "CapsLock" => 0x39, "AltLeft" => 0x3A, "ControlLeft" => 0x3B,
        "ShiftRight" => 0x3C, "AltRight" => 0x3D, "ControlRight" => 0x3E, "F17" => 0x40, "NumpadDecimal" => 0x41,
        "NumpadMultiply" => 0x43, "NumpadAdd" => 0x45, "NumLock" => 0x47, "NumpadDivide" => 0x4B, "NumpadEnter" => 0x4C,
        "NumpadSubtract" => 0x4E, "F18" => 0x4F, "F19" => 0x50, "NumpadEqual" => 0x51, "Numpad0" => 0x52, "Numpad1" => 0x53,
        "Numpad2" => 0x54, "Numpad3" => 0x55, "Numpad4" => 0x56, "Numpad5" => 0x57, "Numpad6" => 0x58, "Numpad7" => 0x59,
        "F20" => 0x5A, "Numpad8" => 0x5B, "Numpad9" => 0x5C, "IntlYen" => 0x5D, "IntlRo" => 0x5E, "NumpadComma" => 0x5F,
        "F5" => 0x60, "F6" => 0x61, "F7" => 0x62, "F3" => 0x63, "F8" => 0x64, "F9" => 0x65, "F11" => 0x67, "F13" => 0x69,
        "F16" => 0x6A, "F14" => 0x6B, "F10" => 0x6D, "ContextMenu" => 0x6E, "F12" => 0x6F, "F15" => 0x71, "Insert" => 0x72,
        "Help" => 0x72, "Home" => 0x73, "PageUp" => 0x74, "Delete" => 0x75, "F4" => 0x76, "End" => 0x77, "F2" => 0x78,
        "PageDown" => 0x79, "F1" => 0x7A, "ArrowLeft" => 0x7B, "ArrowRight" => 0x7C, "ArrowDown" => 0x7D, "ArrowUp" => 0x7E,
        _ => return None,
    })
}

/// Marca de modificador que activa la tecla (0 si no es modificador).
fn modifier_flag(code: &str) -> u64 {
    match code {
        "ShiftLeft" | "ShiftRight" => 0x0002_0000,
        "ControlLeft" | "ControlRight" => 0x0004_0000,
        "AltLeft" | "AltRight" => 0x0008_0000,
        "SuperLeft" | "SuperRight" | "MetaLeft" | "MetaRight" => 0x0010_0000,
        _ => 0,
    }
}

pub struct Injector {
    ox: f64,
    oy: f64,
    sx: f64,
    sy: f64,
    flags: u64,
    keys: HashSet<String>,
    buttons: [bool; 5],
    last: (f64, f64),
    /// Para el doble/triple clic: último botón pulsado, cuándo y cuántas veces seguidas.
    click: (u8, Instant, i64),
}

fn post(ev: Option<objc2_core_foundation::CFRetained<CGEvent>>) {
    if let Some(ev) = ev { CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&ev)); }
}

impl Injector {
    pub fn new(monitor: usize, w: u32, h: u32) -> Res<Self> {
        let ids: Vec<CGDirectDisplayID> = super::active_displays();
        let id = *ids.get(monitor.wrapping_sub(1)).ok_or("monitor inexistente")?;
        // Coordenadas globales en puntos (no píxeles), con origen en la esquina superior izquierda del principal.
        let b = CGDisplayBounds(id);
        Ok(Self {
            ox: b.origin.x, oy: b.origin.y, sx: b.size.width / w.max(1) as f64, sy: b.size.height / h.max(1) as f64,
            flags: 0, keys: HashSet::new(), buttons: [false; 5], last: (b.origin.x, b.origin.y),
            click: (255, Instant::now(), 0),
        })
    }

    fn point(&mut self, x: i32, y: i32) -> CGPoint {
        self.last = (self.ox + x as f64 * self.sx, self.oy + y as f64 * self.sy);
        CGPoint { x: self.last.0, y: self.last.1 }
    }

    fn mouse(&self, kind: CGEventType, at: CGPoint, button: u32, clicks: i64) {
        let ev = CGEvent::new_mouse_event(None, kind, at, CGMouseButton(button));
        if let Some(e) = &ev {
            CGEvent::set_flags(Some(e), CGEventFlags(self.flags));
            if clicks > 0 { CGEvent::set_integer_value_field(Some(e), CGEventField::MouseEventClickState, clicks); }
        }
        post(ev);
    }

    pub fn apply(&mut self, e: Event) {
        match e {
            Event::Move { x, y } => {
                let at = self.point(x, y);
                // Con un botón pulsado, macOS espera eventos de arrastre.
                let (kind, b) = if self.buttons[0] { (CGEventType::LeftMouseDragged, 0) }
                    else if self.buttons[1] { (CGEventType::RightMouseDragged, 1) }
                    else if let Some(i) = (2..5).find(|&i| self.buttons[i]) { (CGEventType::OtherMouseDragged, i as u32) }
                    else { (CGEventType::MouseMoved, 0) };
                self.mouse(kind, at, b, 0);
            }
            Event::Button { button, down, x, y } => {
                let b = button as usize;
                if b >= 5 { return; }
                self.buttons[b] = down;
                let at = self.point(x, y);
                if down {
                    let quick = self.click.0 == button && self.click.1.elapsed() < Duration::from_millis(500);
                    self.click = (button, Instant::now(), if quick { self.click.2 + 1 } else { 1 });
                }
                let kind = match (button, down) {
                    (0, true) => CGEventType::LeftMouseDown,
                    (0, false) => CGEventType::LeftMouseUp,
                    (1, true) => CGEventType::RightMouseDown,
                    (1, false) => CGEventType::RightMouseUp,
                    (_, true) => CGEventType::OtherMouseDown,
                    (_, false) => CGEventType::OtherMouseUp,
                };
                self.mouse(kind, at, button as u32, self.click.2.max(1));
            }
            Event::Wheel { dx, dy } => {
                // 120 unidades = una muesca ≈ 40 píxeles.
                let ev = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 2, dy / 3, dx / 3, 0);
                post(ev);
            }
            Event::Key { down, code } => {
                if down { self.keys.insert(code.clone()); } else { self.keys.remove(&code); }
                let Some(kc) = keycode(&code) else { return };
                let m = modifier_flag(&code);
                if m != 0 {
                    if down { self.flags |= m } else { self.flags &= !m }
                }
                let ev = CGEvent::new_keyboard_event(None, kc, down);
                if let Some(e) = &ev {
                    if m != 0 { CGEvent::set_type(Some(e), CGEventType::FlagsChanged); }
                    CGEvent::set_flags(Some(e), CGEventFlags(self.flags));
                }
                post(ev);
            }
        }
    }
}

impl Drop for Injector {
    /// Al terminar la sesión no puede quedar nada pulsado.
    fn drop(&mut self) {
        for code in std::mem::take(&mut self.keys) { self.apply(Event::Key { down: false, code }); }
        let (x, y) = (((self.last.0 - self.ox) / self.sx) as i32, ((self.last.1 - self.oy) / self.sy) as i32);
        for b in 0..5 {
            if self.buttons[b] { self.apply(Event::Button { button: b as u8, down: false, x, y }); }
        }
    }
}
