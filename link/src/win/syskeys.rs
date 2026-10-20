//! Atajos del sistema hacia el otro equipo: mientras un visor con control está en primer plano, un gancho
//! de teclado de bajo nivel intercepta antes que Windows las teclas Win, Alt+Tab, Alt+Esc, Alt+F4,
//! Alt+Espacio y Ctrl+Esc (también Ctrl+Mayús+Esc) y las envía al otro equipo en vez de actuar aquí.
//! Ctrl+Alt+Supr y Win+L no se pueden interceptar.
use crate::input::Event as Input;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::Sender;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

struct State {
    /// Ventanas de visor con control (HWND) -> canal de teclado y ratón de su sesión.
    wins: HashMap<isize, Sender<Vec<u8>>>,
    /// Teclas interceptadas que siguen pulsadas -> ventana a la que se enviaron (para enviar también su "suelta").
    held: HashMap<u32, isize>,
    hook: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Nombre de la tecla como lo envía el visor (winit `KeyCode`).
fn name(vk: u32) -> Option<&'static str> {
    Some(match VIRTUAL_KEY(vk as u16) {
        VK_LWIN => "SuperLeft",
        VK_RWIN => "SuperRight",
        VK_TAB => "Tab",
        VK_ESCAPE => "Escape",
        VK_F4 => "F4",
        VK_SPACE => "Space",
        _ => return None,
    })
}

/// Hay que llamarla desde el hilo de las ventanas (su bucle de mensajes es el que atiende el gancho).
pub fn register(hwnd: isize, tx: Sender<Vec<u8>>) {
    let mut g = STATE.lock().unwrap();
    let st = g.get_or_insert_with(|| State { wins: HashMap::new(), held: HashMap::new(), hook: false });
    st.wins.insert(hwnd, tx);
    if !st.hook {
        let module = unsafe { GetModuleHandleW(None) }.ok().map(|m| HINSTANCE(m.0));
        st.hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module, 0) }.is_ok();
    }
}

pub fn unregister(hwnd: isize) {
    release(hwnd);
    if let Some(st) = STATE.lock().unwrap().as_mut() { st.wins.remove(&hwnd); }
}

/// Suelta en el otro equipo las teclas interceptadas que siguen pulsadas (al perder el foco).
pub fn release(hwnd: isize) {
    let mut g = STATE.lock().unwrap();
    let Some(st) = g.as_mut() else { return };
    let keys: Vec<u32> = st.held.iter().filter(|(_, w)| **w == hwnd).map(|(vk, _)| *vk).collect();
    for vk in keys {
        st.held.remove(&vk);
        if let (Some(tx), Some(n)) = (st.wins.get(&hwnd), name(vk)) { let _ = tx.send(Input::Key { down: false, code: n.into() }.encode()); }
    }
}

/// true si la tecla se envía al otro equipo (y no debe llegar a Windows).
fn take(vk: u32, alt: bool, down: bool) -> bool {
    let Some(n) = name(vk) else { return false };
    let mut g = STATE.lock().unwrap();
    let Some(st) = g.as_mut() else { return false };
    let target = if down {
        let fg = unsafe { GetForegroundWindow() }.0 as isize;
        if !st.wins.contains_key(&fg) { return false; }
        let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0;
        let vkey = VIRTUAL_KEY(vk as u16);
        let system = matches!(vkey, VK_LWIN | VK_RWIN)
            || (alt && matches!(vkey, VK_TAB | VK_ESCAPE | VK_F4 | VK_SPACE))
            || (ctrl && vkey == VK_ESCAPE)
            // Repetición de una tecla ya interceptada.
            || st.held.contains_key(&vk);
        if !system { return false; }
        st.held.insert(vk, fg);
        fg
    } else {
        // Se suelta una tecla interceptada: va a la misma ventana aunque ya no esté en primer plano.
        match st.held.remove(&vk) { Some(w) => w, None => return false }
    };
    if let Some(tx) = st.wins.get(&target) { let _ = tx.send(Input::Key { down, code: n.into() }.encode()); }
    true
}

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        // Las pulsaciones inyectadas (p. ej. por otro programa) pasan sin tocar.
        if k.flags.0 & LLKHF_INJECTED.0 == 0 {
            let down = matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if take(k.vkCode, k.flags.0 & LLKHF_ALTDOWN.0 != 0, down) { return LRESULT(1); }
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}
