//! Ventana del visor: muestra el último fotograma decodificado, en ventana o a pantalla completa
//! sobre el monitor elegido (numerados de izquierda a derecha, como en Kast).
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
    Redraw,
    Closed(String),
}

struct App {
    shared: Arc<Mutex<Shared>>,
    monitor: Option<usize>,
    title: String,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    error: Option<String>,
}

// Flecha del cursor: '#' borde negro, '.' relleno blanco.
const ARROW: [&str; 17] = [
    "#", "##", "#.#", "#..#", "#...#", "#....#", "#.....#", "#......#", "#.......#", "#........#",
    "#.....####", "#..#..#", "#.# #..#", "##  #..#", "#    #..#", "     #..#", "      ##",
];

impl App {
    fn draw(&mut self) {
        let (Some(win), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let size = win.inner_size();
        let (Some(ww), Some(wh)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if surface.resize(ww, wh).is_err() { return; }
        let Ok(mut buf) = surface.buffer_mut() else { return };
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

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() { return; }
        let mut attrs = Window::default_attributes().with_title(&self.title).with_inner_size(LogicalSize::new(1280.0, 720.0));
        if let Some(n) = self.monitor {
            let mut mons: Vec<_> = el.available_monitors().collect();
            mons.sort_by_key(|m| (m.position().x, m.position().y));
            match mons.into_iter().nth(n.saturating_sub(1)) {
                Some(m) => attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(m)))),
                None => eprintln!("monitor {n} no existe; se abre en ventana"),
            }
        }
        let win = match el.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => { self.error = Some(e.to_string()); el.exit(); return; }
        };
        let ctx = softbuffer::Context::new(win.clone()).expect("softbuffer");
        self.surface = Some(softbuffer::Surface::new(&ctx, win.clone()).expect("softbuffer"));
        self.window = Some(win);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key: Key::Named(NamedKey::Escape), state: ElementState::Pressed, .. }, .. } => el.exit(),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key: Key::Named(NamedKey::F11), state: ElementState::Pressed, .. }, .. } => {
                if let Some(w) = &self.window {
                    let full = w.fullscreen().is_some();
                    w.set_fullscreen(if full { None } else { Some(Fullscreen::Borderless(w.current_monitor())) });
                }
            }
            WindowEvent::Resized(_) => { if let Some(w) = &self.window { w.request_redraw(); } }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Event) {
        match event {
            Event::Redraw => { if let Some(w) = &self.window { w.request_redraw(); } }
            Event::Closed(msg) => { self.error = Some(msg); el.exit(); }
        }
    }
}

/// Bucle de la ventana (hilo principal). Devuelve el motivo de cierre si lo hubo.
pub fn run(el: EventLoop<Event>, shared: Arc<Mutex<Shared>>, monitor: Option<usize>, title: String) -> Option<String> {
    let mut app = App { shared, monitor, title, window: None, surface: None, error: None };
    if let Err(e) = el.run_app(&mut app) { return Some(e.to_string()); }
    app.error
}
