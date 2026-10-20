//! Ventanas del visor (una por sesión): muestran el último fotograma decodificado, en ventana o a
//! pantalla completa sobre el monitor elegido (numerados de izquierda a derecha, como en Kast).
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

pub struct Shared {
    pub frame: Vec<u32>,
    pub w: usize,
    pub h: usize,
    pub cursor: (i32, i32, bool),
}

pub enum Event {
    /// Abre la ventana de una sesión. `monitor`: None = ventana, Some(n) = pantalla completa en el monitor n.
    Open { session: u64, shared: Arc<Mutex<Shared>>, monitor: Option<usize>, title: String },
    Redraw(u64),
    /// La sesión terminó (por la red o porque se cerró): se cierra su ventana.
    Close(u64),
}

struct Win {
    session: u64,
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    shared: Arc<Mutex<Shared>>,
}

// Flecha del cursor: '#' borde negro, '.' relleno blanco.
const ARROW: [&str; 17] = [
    "#", "##", "#.#", "#..#", "#...#", "#....#", "#.....#", "#......#", "#.......#", "#........#",
    "#.....####", "#..#..#", "#.# #..#", "##  #..#", "#    #..#", "     #..#", "      ##",
];

impl Win {
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
            let (cx, cy, visible) = s.cursor;
            if visible {
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
        if let Some(w) = self.wins.remove(&id) {
            self.by_session.remove(&w.session);
            (self.on_closed)(w.session);
        }
        if self.wins.is_empty() && !self.keep_alive { el.exit(); }
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, _el: &ActiveEventLoop) {}

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
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
            Event::Open { session, shared, monitor, title } => {
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
                self.wins.insert(id, Win { session, window, surface, shared });
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
