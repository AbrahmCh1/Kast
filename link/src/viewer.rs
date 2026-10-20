//! Ventanas del visor (una por sesión): muestran el último fotograma decodificado, en ventana o a
//! pantalla completa sobre el monitor elegido (numerados de izquierda a derecha, como en Kast).
use crate::input::Event as Input;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

pub struct Shared {
    pub frame: Vec<u32>,
    pub w: usize,
    pub h: usize,
    pub cursor: (i32, i32, bool),
}

pub enum Event {
    /// Abre la ventana de una sesión. `monitor`: None = ventana, Some(n) = pantalla completa en el monitor n.
    /// `input`: con control, por aquí salen el teclado y el ratón hacia el otro equipo.
    Open { session: u64, shared: Arc<Mutex<Shared>>, monitor: Option<usize>, title: String, input: Option<Sender<Vec<u8>>> },
    Redraw(u64),
    /// La sesión terminó (por la red o porque se cerró): se cierra su ventana.
    Close(u64),
}

struct Win {
    session: u64,
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    shared: Arc<Mutex<Shared>>,
    input: Option<Sender<Vec<u8>>>,
    /// Última imagen dibujada: escala y desplazamiento, para traducir el ratón a píxeles remotos.
    layout: (f64, f64, f64, usize, usize),
    mods: ModifiersState,
    keys: HashSet<String>,
    buttons: [bool; 5],
    pos: Option<(i32, i32)>,
}

// Flecha del cursor: '#' borde negro, '.' relleno blanco.
const ARROW: [&str; 17] = [
    "#", "##", "#.#", "#..#", "#...#", "#....#", "#.....#", "#......#", "#.......#", "#........#",
    "#.....####", "#..#..#", "#.# #..#", "##  #..#", "#    #..#", "     #..#", "      ##",
];

impl Win {
    fn send(&self, e: Input) {
        if let Some(tx) = &self.input { let _ = tx.send(e.encode()); }
    }

    /// Suelta todo lo pulsado (al perder el foco o cerrar) para que nada quede atascado en el otro equipo.
    fn release_all(&mut self) {
        for code in std::mem::take(&mut self.keys) { self.send(Input::Key { down: false, code }); }
        let (x, y) = self.pos.unwrap_or((0, 0));
        for b in 0..5 {
            if self.buttons[b] { self.buttons[b] = false; self.send(Input::Button { button: b as u8, down: false, x, y }); }
        }
    }

    /// Posición de la ventana -> píxel de la imagen remota (None si cae en las franjas negras).
    fn to_remote(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        let (scale, ox, oy, w, h) = self.layout;
        if scale <= 0.0 { return None; }
        let (rx, ry) = ((x - ox) / scale, (y - oy) / scale);
        (rx >= 0.0 && ry >= 0.0 && rx < w as f64 && ry < h as f64).then(|| (rx as i32, ry as i32))
    }

    /// Eventos de teclado y ratón con control. Devuelve true si la ventana debe cerrarse.
    fn input_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::Focused(false) => self.release_all(),
            WindowEvent::KeyboardInput { event: KeyEvent { physical_key: PhysicalKey::Code(code), state, .. }, .. } => {
                let down = *state == ElementState::Pressed;
                // Atajos locales: Ctrl+Alt+Shift+F pantalla completa, Ctrl+Alt+Shift+Q salir.
                let local = self.mods.control_key() && self.mods.alt_key() && self.mods.shift_key();
                if local && down && *code == KeyCode::KeyF {
                    let full = self.window.fullscreen().is_some();
                    self.window.set_fullscreen(if full { None } else { Some(Fullscreen::Borderless(self.window.current_monitor())) });
                    return false;
                }
                if local && down && *code == KeyCode::KeyQ { self.release_all(); return true; }
                let name = format!("{code:?}");
                if down { self.keys.insert(name.clone()); } else if !self.keys.remove(&name) { return false; }
                self.send(Input::Key { down, code: name });
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(p) = self.to_remote(position.x, position.y) {
                    if self.pos != Some(p) { self.pos = Some(p); self.send(Input::Move { x: p.0, y: p.1 }); }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => 0, MouseButton::Right => 1, MouseButton::Middle => 2,
                    MouseButton::Back => 3, MouseButton::Forward => 4, _ => return false,
                };
                let down = *state == ElementState::Pressed;
                let Some((x, y)) = self.pos else { return false };
                if !down && !self.buttons[b] { return false; }
                self.buttons[b] = down;
                self.send(Input::Button { button: b as u8, down, x, y });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => ((x * 120.0) as i32, (y * 120.0) as i32),
                    MouseScrollDelta::PixelDelta(p) => ((p.x * 3.0) as i32, (p.y * 3.0) as i32),
                };
                if dx != 0 || dy != 0 { self.send(Input::Wheel { dx, dy }); }
            }
            _ => {}
        }
        false
    }

    fn draw(&mut self) {
        let size = self.window.inner_size();
        let (Some(ww), Some(wh)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if self.surface.resize(ww, wh).is_err() { return; }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        let (ww, wh) = (size.width as usize, size.height as usize);
        buf.fill(0);
        let s = self.shared.lock().unwrap();
        if s.w > 0 && s.h > 0 {
            // Escala manteniendo la proporción (vecino más próximo; 1:1 si coincide la resolución).
            let scale = (ww as f64 / s.w as f64).min(wh as f64 / s.h as f64);
            let dw = ((s.w as f64 * scale) as usize).max(1).min(ww);
            let dh = ((s.h as f64 * scale) as usize).max(1).min(wh);
            let (ox, oy) = ((ww - dw) / 2, (wh - dh) / 2);
            self.layout = (scale, ox as f64, oy as f64, s.w, s.h);
            if dw == s.w && dh == s.h {
                for y in 0..dh {
                    buf[(oy + y) * ww + ox..(oy + y) * ww + ox + dw].copy_from_slice(&s.frame[y * s.w..(y + 1) * s.w]);
                }
            } else {
                let xmap: Vec<usize> = (0..dw).map(|x| x * s.w / dw).collect();
                for y in 0..dh {
                    let src = &s.frame[(y * s.h / dh) * s.w..];
                    let dst = &mut buf[(oy + y) * ww + ox..(oy + y) * ww + ox + dw];
                    for (d, &sx) in dst.iter_mut().zip(&xmap) { *d = src[sx]; }
                }
            }
            // Con control se usa el cursor propio (sin retraso): no se dibuja el remoto.
            let (cx, cy, visible) = s.cursor;
            if visible && self.input.is_none() {
                let bx = ox as f64 + cx as f64 * scale;
                let by = oy as f64 + cy as f64 * scale;
                for (ry, row) in ARROW.iter().enumerate() {
                    for (rx, ch) in row.chars().enumerate() {
                        let color = match ch { '#' => 0x000000, '.' => 0xFFFFFF, _ => continue };
                        let (px, py) = (bx as isize + rx as isize, by as isize + ry as isize);
                        if px >= 0 && py >= 0 && (px as usize) < ww && (py as usize) < wh {
                            buf[py as usize * ww + px as usize] = color;
                        }
                    }
                }
            }
        }
        drop(s);
        let _ = buf.present();
    }
}

struct App {
    wins: HashMap<WindowId, Win>,
    by_session: HashMap<u64, WindowId>,
    /// Se avisa cuando el usuario cierra la ventana de una sesión.
    on_closed: Box<dyn Fn(u64)>,
    /// Modo agente: el bucle sigue aunque no haya ventanas.
    keep_alive: bool,
}

impl App {
    fn close(&mut self, el: &ActiveEventLoop, id: WindowId) {
        if let Some(mut w) = self.wins.remove(&id) {
            w.release_all();
            self.by_session.remove(&w.session);
            (self.on_closed)(w.session);
        }
        if self.wins.is_empty() && !self.keep_alive { el.exit(); }
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, _el: &ActiveEventLoop) {}

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Con control, el teclado y el ratón van al otro equipo (salvo los atajos locales).
        if let Some(w) = self.wins.get_mut(&id) {
            if w.input.is_some() {
                if w.input_event(&event) { return self.close(el, id); }
                match event {
                    WindowEvent::KeyboardInput { .. } | WindowEvent::ModifiersChanged(_) => return,
                    _ => {}
                }
            }
        }
        match event {
            WindowEvent::CloseRequested => self.close(el, id),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key: Key::Named(NamedKey::Escape), state: ElementState::Pressed, .. }, .. } => self.close(el, id),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key: Key::Named(NamedKey::F11), state: ElementState::Pressed, .. }, .. } => {
                if let Some(w) = self.wins.get(&id) {
                    let full = w.window.fullscreen().is_some();
                    w.window.set_fullscreen(if full { None } else { Some(Fullscreen::Borderless(w.window.current_monitor())) });
                }
            }
            WindowEvent::Resized(_) => { if let Some(w) = self.wins.get(&id) { w.window.request_redraw(); } }
            WindowEvent::RedrawRequested => { if let Some(w) = self.wins.get_mut(&id) { w.draw(); } }
            _ => {}
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Event) {
        match event {
            Event::Open { session, shared, monitor, title, input } => {
                let mut attrs = Window::default_attributes().with_title(&title).with_inner_size(LogicalSize::new(1280.0, 720.0));
                if let Some(n) = monitor {
                    let mut mons: Vec<_> = el.available_monitors().collect();
                    mons.sort_by_key(|m| (m.position().x, m.position().y));
                    if let Some(m) = mons.into_iter().nth(n.saturating_sub(1)) {
                        attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(m))));
                    }
                }
                let Ok(window) = el.create_window(attrs) else { (self.on_closed)(session); return };
                let window = Rc::new(window);
                let Ok(ctx) = softbuffer::Context::new(window.clone()) else { return };
                let Ok(surface) = softbuffer::Surface::new(&ctx, window.clone()) else { return };
                window.focus_window();
                let id = window.id();
                self.by_session.insert(session, id);
                self.wins.insert(id, Win { session, window, surface, shared, input, layout: (0.0, 0.0, 0.0, 0, 0),
                                           mods: ModifiersState::empty(), keys: HashSet::new(), buttons: [false; 5], pos: None });
            }
            Event::Redraw(s) => {
                if let Some(w) = self.by_session.get(&s).and_then(|id| self.wins.get(id)) { w.window.request_redraw(); }
            }
            Event::Close(s) => {
                if let Some(id) = self.by_session.remove(&s) {
                    self.wins.remove(&id);
                    if self.wins.is_empty() && !self.keep_alive { el.exit(); }
                }
            }
        }
    }
}

pub fn event_loop(agent: bool) -> Result<EventLoop<Event>, winit::error::EventLoopError> {
    let mut b = EventLoop::<Event>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        if agent { b.with_activation_policy(ActivationPolicy::Accessory); }
    }
    let _ = agent;
    b.build()
}

/// Bucle de ventanas (hilo principal).
pub fn run(el: EventLoop<Event>, keep_alive: bool, on_closed: impl Fn(u64) + 'static) -> Result<(), winit::error::EventLoopError> {
    let mut app = App { wins: HashMap::new(), by_session: HashMap::new(), on_closed: Box::new(on_closed), keep_alive };
    el.run_app(&mut app)
}
