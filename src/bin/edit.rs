//! **`edit`, the standalone program** (docs/editor-crate-plan.md, E4): the
//! modeler's editor and its Emacs-shaped shell, with an in-process Scheme
//! image and nothing else — no 3D, no scene. A winit window around
//! `edit::app` (the panes) and `edit::scm_binding::standalone` (the image).
//!
//!     edit [FILE …]                 open each file in a buffer
//!                                   (M-x term runs a program in the lower pane)
//!     edit --screenshot OUT [FILE]  draw one frame to OUT.png, no window
//!
//! `~/.config/edit/init.scm` is loaded at startup, after the shell, when it
//! exists.

use std::num::NonZeroU32;
use std::rc::Rc;

use edit::app;
use edit::framebuffer::Framebuffer;
use edit::scm_binding::standalone::Session;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Window, WindowId};

fn init_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config/edit/init.scm"))
}

struct Display {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    _context: softbuffer::Context<Rc<Window>>,
}

struct App {
    session: Session,
    display: Option<Display>,
    framebuffer: Framebuffer,
    modifiers: ModifiersState,
    cursor: Option<(f64, f64)>,
    /// The button held, for a drag.
    held: Option<MouseButton>,
    /// When the right button's marking menu reveals itself.
    pie_deadline: Option<std::time::Instant>,
}

impl App {
    fn redraw(&self) {
        if let Some(d) = &self.display {
            d.window.request_redraw();
        }
    }

    /// After anything that may have changed the screen.
    fn settle(&self, event_loop: &ActiveEventLoop) {
        if app::quit_requested() {
            event_loop.exit();
        } else if app::take_dirty() {
            app::mark_dirty(); // the paint takes it
            self.redraw();
        }
    }

    fn key(&mut self, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if event.logical_key == Key::Named(NamedKey::Escape) {
            if !self.session.escape() {
                self.session.key("escape");
            }
            return;
        }
        let m = self.modifiers;
        // A typing pane gets printable keys as TEXT, which honours shift and
        // the layout; chords travel as descriptors, as in the modeler.
        if app::typing().is_some() && !m.control_key() && !m.alt_key() && !m.super_key() {
            if let Some(txt) = event.text.as_deref() {
                if !txt.is_empty() && txt.chars().all(|c| !c.is_control()) {
                    self.session.text(txt);
                    return;
                }
            }
        }
        if let Some(desc) = edit::keys::descriptor(&event.key_without_modifiers(), m) {
            self.session.key(&desc);
        }
    }
}

impl ApplicationHandler for App {
    /// A terminal printed something, from its reader thread (E5).
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _wake: ()) {
        app::mark_dirty();
        self.redraw();
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.display.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("edit")
            .with_inner_size(winit::dpi::LogicalSize::new(1000.0, 760.0));
        let window = Rc::new(event_loop.create_window(attrs).expect("a window"));
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
        let size = window.inner_size();
        app::resize(size.width, size.height, window.scale_factor() as f32);
        self.display = Some(Display { window, surface, _context: context });
        self.redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let scale = self.display.as_ref().map_or(1.0, |d| d.window.scale_factor() as f32);
                app::resize(size.width, size.height, scale);
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(d) = &self.display {
                    let size = d.window.inner_size();
                    app::resize(size.width, size.height, scale_factor as f32);
                }
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                self.key(&event);
                self.settle(event_loop);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some((x, y)) = self.cursor else { return };
                let (shift, ctrl) = (self.modifiers.shift_key(), self.modifiers.control_key());
                self.held = (state == ElementState::Pressed).then_some(button);
                match (state, button) {
                    (ElementState::Pressed, MouseButton::Left) => self.session.click(x, y, shift, ctrl),
                    (ElementState::Released, MouseButton::Left) => self.session.release(x, y),
                    (ElementState::Pressed, MouseButton::Right) => {
                        self.session.menu_press(x, y, shift);
                        let ms = app::pie_delay_ms();
                        if ms == 0 {
                            self.session.menu_hold();
                        } else {
                            let at = std::time::Instant::now() + std::time::Duration::from_millis(ms);
                            self.pie_deadline = Some(at);
                            event_loop.set_control_flow(ControlFlow::WaitUntil(at));
                        }
                    }
                    (ElementState::Released, MouseButton::Right) => {
                        self.pie_deadline = None;
                        event_loop.set_control_flow(ControlFlow::Wait);
                        self.session.menu_release(x, y);
                    }
                    _ => {}
                }
                self.settle(event_loop);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x, position.y);
                match (self.held, self.cursor) {
                    (Some(MouseButton::Left), Some((lx, ly))) => {
                        let (alt, shift) = (self.modifiers.alt_key(), self.modifiers.shift_key());
                        self.session.drag(x - lx, y - ly, alt, shift);
                    }
                    (Some(MouseButton::Right), _) => self.session.menu_drag(x, y),
                    _ => self.session.cursor_move(x, y),
                }
                self.cursor = Some((x, y));
                self.settle(event_loop);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                if let (Some((x, y)), true) = (self.cursor, d != 0.0) {
                    let (alt, shift) = (self.modifiers.alt_key(), self.modifiers.shift_key());
                    self.session.wheel(d, x, y, alt, shift);
                    self.settle(event_loop);
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(display) = self.display.as_mut() else { return };
                let size = display.window.inner_size();
                let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
                if display.surface.resize(w, h).is_err() {
                    return;
                }
                if self.framebuffer.width != size.width || self.framebuffer.height != size.height {
                    self.framebuffer.resize(size.width, size.height);
                }
                app::take_dirty();
                app::draw(&mut self.framebuffer);
                if let Ok(mut buffer) = display.surface.buffer_mut() {
                    buffer.copy_from_slice(&self.framebuffer.pixels);
                    if let Err(e) = buffer.present() {
                        eprintln!("[edit] present failed: {e}");
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(at) = self.pie_deadline {
            if std::time::Instant::now() >= at {
                self.pie_deadline = None;
                event_loop.set_control_flow(ControlFlow::Wait);
                self.session.menu_hold();
                self.settle(event_loop);
            }
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let screenshot = args.iter().position(|a| a == "--screenshot").map(|i| {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "edit.png".into());
        args.drain(i..(i + 2).min(args.len()));
        out
    });
    let init = init_path();
    if let Some(out) = screenshot {
        // One frame, no window: what a test or a README picture wants.
        app::resize(1200, 800, 1.0);
        let session = Session::boot(init.as_deref(), &args);
        if let Err(e) = session {
            eprintln!("edit: {e}");
            std::process::exit(1);
        }
        let mut fb = Framebuffer::new(1200, 800);
        app::draw(&mut fb);
        if let Err(e) = fb.save_png(&out) {
            eprintln!("edit: {out}: {e}");
            std::process::exit(1);
        }
        return;
    }
    // The layout needs a size before the boot echoes into the panes; the
    // window's own arrives with `resumed`.
    app::resize(1000, 760, 1.0);
    let session = match Session::boot(init.as_deref(), &args) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("edit: {e}");
            std::process::exit(1);
        }
    };
    let event_loop = EventLoop::<()>::with_user_event().build().expect("an event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    // A terminal's output arrives on its reader thread; the window must be
    // woken to draw it.
    let proxy = std::sync::Mutex::new(event_loop.create_proxy());
    edit::termpane::set_waker(Box::new(move || {
        let _ = proxy.lock().unwrap_or_else(|e| e.into_inner()).send_event(());
    }));
    let mut app = App {
        session,
        display: None,
        framebuffer: Framebuffer::new(1, 1),
        modifiers: ModifiersState::empty(),
        cursor: None,
        held: None,
        pie_deadline: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("edit: {e}");
    }
}
