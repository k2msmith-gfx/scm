//! **`edit`, the standalone program** (docs/editor-crate-plan.md, E4): the
//! modeler's editor and its Emacs-shaped shell, with an in-process Scheme
//! image and nothing else — no 3D, no scene. A winit window around
//! `edit::app` (the panes) and `edit::scm_binding::standalone` (the image).
//!
//!     edit [FILE …]                 open each file in a buffer
//!                                   (M-x term runs a program in the lower pane)
//!     edit --screenshot OUT [--scale N] [FILE]
//!                                   draw one frame to OUT.png, no window
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

/// A film script's hold on the program (`edit::film`): `EDIT_FILM_PORT`
/// turns it on. The frame is the film's size, not the window's, and the
/// window stays hidden: what a film records is the framebuffer, and a size
/// that followed the screen would make every take a different film. The
/// caption has a BAND of its own under the editor, so it can never cover
/// what is being shown.
struct Film {
    /// The whole frame, in physical pixels, band included.
    size: (u32, u32),
    scale: f32,
    band: u32,
    caption: Option<String>,
    /// Forms from the script's connection, each with where its answer goes.
    requests: std::sync::mpsc::Receiver<(String, std::sync::mpsc::Sender<String>)>,
}

impl Film {
    fn from_env(port: u16) -> Film {
        let size = std::env::var("EDIT_FILM_SIZE")
            .ok()
            .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
            .unwrap_or((1600, 1000));
        let scale: f32 = std::env::var("EDIT_FILM_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let band = (64.0 * scale).round() as u32;
        let (tx, requests) = std::sync::mpsc::channel();
        std::thread::spawn(move || serve(port, tx));
        Film { size, scale, band, caption: None, requests }
    }
}

/// The script's side: forms end at a blank line, as the modeler's eval
/// channel takes them; each answer is followed by the `edit>` prompt. The
/// form runs on the UI thread, woken to take it.
fn serve(port: u16, tx: std::sync::mpsc::Sender<(String, std::sync::mpsc::Sender<String>)>) {
    use std::io::{Read, Write};
    let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) else {
        eprintln!("edit: film: cannot listen on {port}");
        return;
    };
    for stream in listener.incoming().flatten() {
        let mut stream = stream;
        let _ = stream.write_all(b"edit> ");
        let mut pending = String::new();
        let mut buf = [0u8; 1 << 16];
        while let Ok(n) = stream.read(&mut buf) {
            if n == 0 {
                break;
            }
            pending.push_str(&String::from_utf8_lossy(&buf[..n]));
            while let Some(at) = pending.find("\n\n") {
                let form: String = pending.drain(..at + 2).collect();
                let (rtx, rrx) = std::sync::mpsc::channel();
                if tx.send((form, rtx)).is_err() {
                    return;
                }
                edit::wake::wake();
                let answer = rrx.recv().unwrap_or_else(|_| "error: the editor has gone".into());
                if stream.write_all(format!("{answer}\nedit> ").as_bytes()).is_err() {
                    break;
                }
            }
        }
    }
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
    /// The divider being dragged: the panel's bar, or the file tree's edge.
    dragging_divider: Option<app::Divider>,
    /// The pane a mouse selection is being dragged in (the REPL's
    /// transcript or the terminal), from press to release.
    selecting: Option<u32>,
    film: Option<Film>,
    /// Which divider the pointer shows the resize arrow for (set only on
    /// change).
    resize_cursor: Option<app::Divider>,
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
        if app::typing().is_some() && !m.control_key() && !m.alt_key() && !m.super_key()
            && let Some(txt) = event.text.as_deref()
                && !txt.is_empty() && txt.chars().all(|c| !c.is_control()) {
                    self.session.text(txt);
                    return;
                }
        if let Some(desc) = edit::keys::descriptor(&event.key_without_modifiers(), m) {
            self.session.key(&desc);
        }
    }

    /// The film script's forms, each run and answered — what they asked of
    /// the keyboard, the pointer and the camera applied first, through the
    /// same methods the window's own events take.
    fn film_requests(&mut self, event_loop: &ActiveEventLoop) {
        loop {
            let Some((form, reply)) = self.film.as_ref().and_then(|f| f.requests.try_recv().ok()) else { return };
            let answer = match self.session.eval_to_string(&form) {
                Ok(v) => format!("=> {v}"),
                Err(e) => format!("error: {e}"),
            };
            for op in edit::film::take() {
                self.film_op(event_loop, op);
            }
            let _ = reply.send(answer);
        }
    }

    fn film_op(&mut self, event_loop: &ActiveEventLoop, op: edit::film::Op) {
        use edit::film::Op;
        match op {
            Op::Key(desc) => {
                let m = self.modifiers;
                if desc == "escape" {
                    if !self.session.escape() {
                        self.session.key("escape");
                    }
                } else if app::typing().is_some()
                    && !m.control_key()
                    && !m.alt_key()
                    && !m.super_key()
                    && desc.chars().count() == 1
                    && !desc.chars().any(char::is_control)
                {
                    self.session.text(&desc);
                } else {
                    self.session.key(&desc);
                }
                self.settle(event_loop);
            }
            Op::Mouse(what, a, b) => {
                let button = if a >= 2.0 { MouseButton::Right } else { MouseButton::Left };
                let set = |m: &mut ModifiersState, bit: ModifiersState| m.set(bit, a != 0.0);
                match what.as_str() {
                    "move" => self.cursor_moved(event_loop, a, b),
                    "press" => self.mouse_button(event_loop, ElementState::Pressed, button),
                    "release" => self.mouse_button(event_loop, ElementState::Released, button),
                    "wheel" => self.wheel(event_loop, a),
                    "shift" => set(&mut self.modifiers, ModifiersState::SHIFT),
                    "ctrl" => set(&mut self.modifiers, ModifiersState::CONTROL),
                    "alt" => set(&mut self.modifiers, ModifiersState::ALT),
                    _ => {}
                }
            }
            Op::Shot(path) => {
                self.paint_film();
                if let Err(e) = self.framebuffer.save_png(&path) {
                    eprintln!("edit: film: {path}: {e}");
                }
            }
            Op::Caption(text) => {
                if let Some(f) = self.film.as_mut() {
                    f.caption = text;
                }
            }
        }
    }

    /// The film's frame: the editor above, the caption's band below.
    fn paint_film(&mut self) {
        let Some(film) = self.film.as_ref() else { return };
        let (w, h) = film.size;
        if self.framebuffer.width != w || self.framebuffer.height != h {
            self.framebuffer.resize(w, h);
        }
        app::take_dirty();
        app::draw(&mut self.framebuffer);
        let band = edit::framebuffer::Rect { x: 0, y: h - film.band, w, h: film.band };
        self.framebuffer.fill_rect(band, 0x0016_181d);
        // A hairline in the accent, so the band reads as the film's, not the editor's.
        self.framebuffer.fill_rect(edit::framebuffer::Rect { x: 0, y: band.y, w, h: (2.0 * film.scale) as u32 }, 0x0061_afef);
        if let Some(text) = &film.caption {
            let px = 22.0 * film.scale;
            let y = edit::text::label_center_top(px, band.y as f32, band.h as f32);
            edit::text::draw_label(&mut self.framebuffer, (24.0 * film.scale) as i32, y as i32, px, 0x00f5_f7fa, 1.0, text);
        }
    }

    /// A mouse button — the window's, or a film's (`edit::film`).
    fn mouse_button(&mut self, event_loop: &ActiveEventLoop, state: ElementState, button: MouseButton) {
        let Some((x, y)) = self.cursor else { return };
        let (shift, ctrl) = (self.modifiers.shift_key(), self.modifiers.control_key());
        self.held = (state == ElementState::Pressed).then_some(button);
        match (state, button) {
            // The divider and the strip's icons are chrome, not panes.
            (ElementState::Pressed, MouseButton::Left) if app::divider_at(x, y).is_some() => {
                self.held = None;
                self.dragging_divider = app::divider_at(x, y);
            }
            (ElementState::Released, MouseButton::Left) if self.dragging_divider.is_some() => {
                self.dragging_divider = None;
            }
            (ElementState::Pressed, MouseButton::Left) if app::icon_at(x, y).is_some() => {
                self.held = None;
                self.session.call(match app::icon_at(x, y) {
                    Some(app::Icon::Terminal) => "(toggle-panel :terminal)",
                    Some(app::Icon::Files) => "(toggle-files)",
                    Some(app::Icon::Agent) => "(agent-icon-click)",
                    _ => "(toggle-panel :repl)",
                });
            }
            (ElementState::Pressed, MouseButton::Left) => {
                // A press in the REPL or the terminal anchors a
                // selection (a drag extends it); the click still
                // reaches the shell, which focuses the pane.
                self.selecting = app::begin_select(x, y);
                self.session.click(x, y, shift, ctrl);
            }
            (ElementState::Released, MouseButton::Left) => {
                self.selecting = None;
                self.session.release(x, y);
            }
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

    /// The pointer moved to (X, Y), physical pixels.
    fn cursor_moved(&mut self, event_loop: &ActiveEventLoop, x: f64, y: f64) {
        if let Some(d) = self.dragging_divider {
            app::drag_divider(d, x, y);
            self.cursor = Some((x, y));
            self.redraw();
            return;
        }
        // An icon's tip shows while the pointer is on it.
        if app::hover_icon(x, y) {
            self.redraw();
        }
        let over = app::divider_at(x, y);
        if over != self.resize_cursor {
            self.resize_cursor = over;
            if let Some(d) = &self.display {
                d.window.set_cursor(match over {
                    Some(app::Divider::Panel) => winit::window::CursorIcon::NsResize,
                    Some(app::Divider::Files | app::Divider::Agent) => winit::window::CursorIcon::EwResize,
                    None => winit::window::CursorIcon::Default,
                });
            }
        }
        match (self.held, self.cursor) {
            (Some(MouseButton::Left), _) if self.selecting.is_some() => {
                app::extend_select(self.selecting.unwrap_or_default(), x, y);
                app::mark_dirty();
            }
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

    /// The wheel turned D notches.
    fn wheel(&mut self, event_loop: &ActiveEventLoop, d: f64) {
        if let (Some((x, y)), true) = (self.cursor, d != 0.0) {
            let (alt, shift) = (self.modifiers.alt_key(), self.modifiers.shift_key());
            self.session.wheel(d, x, y, alt, shift);
            self.settle(event_loop);
        }
    }
}

impl ApplicationHandler for App {
    /// A terminal printed something, or a remote Scheme answered an async
    /// send — from a reader thread (`edit::wake`): deliver the answers,
    /// then redraw.
    fn user_event(&mut self, event_loop: &ActiveEventLoop, _wake: ()) {
        self.film_requests(event_loop);
        self.session.poll();
        app::mark_dirty();
        self.redraw();
        self.settle(event_loop);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.display.is_some() {
            return;
        }
        let screen = event_loop.primary_monitor().map(|m| {
            let (size, scale) = (m.size(), m.scale_factor());
            (size.width as f64 / scale, size.height as f64 / scale)
        });
        let (w, h) = app::start_window_size(screen);
        let attrs = Window::default_attributes()
            .with_title("edit")
            .with_inner_size(winit::dpi::LogicalSize::new(w, h))
            .with_maximized(app::START_MAXIMIZED && self.film.is_none())
            .with_visible(self.film.is_none());
        let window = Rc::new(event_loop.create_window(attrs).expect("a window"));
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
        let size = window.inner_size();
        if self.film.is_none() {
            app::resize(size.width, size.height, window.scale_factor() as f32);
        }
        self.display = Some(Display { window, surface, _context: context });
        self.redraw();
        // A script that connected while the editor booted woke nothing — the
        // waker is set after the boot — so its first form waits here.
        self.film_requests(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            // A film's frame is its own size, whatever the window's.
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } if self.film.is_some() => {}
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
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(event_loop, state, button),
            WindowEvent::CursorLeft { .. } => {
                if app::hover_icon(-1.0, -1.0) {
                    self.redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(event_loop, position.x, position.y),
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.wheel(event_loop, d);
            }
            WindowEvent::RedrawRequested if self.film.is_some() => {}
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
        self.film_requests(event_loop);
        if let Some(at) = self.pie_deadline
            && std::time::Instant::now() >= at {
                self.pie_deadline = None;
                self.session.menu_hold();
                self.settle(event_loop);
            }
        // A playing movie's next frame is a deadline too: show this one,
        // and sleep until that one is due.
        let (_, movie) = edit::editorpane::video_tick();
        self.settle(event_loop);
        event_loop.set_control_flow(match [self.pie_deadline, movie].into_iter().flatten().min() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let screenshot = args.iter().position(|a| a == "--screenshot").map(|i| {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "edit.png".into());
        args.drain(i..(i + 2).min(args.len()));
        out
    });
    // `--scale N` (screenshot only): draw as a display at N× does — 2 for
    // retina — so a frame shows what the window shows.
    let scale = args.iter().position(|a| a == "--scale").map(|i| {
        let n = args.get(i + 1).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
        args.drain(i..(i + 2).min(args.len()));
        n
    });
    let init = init_path();
    if let Some(out) = screenshot {
        // One frame, no window: what a test or a README picture wants.
        let scale = scale.unwrap_or(1.0);
        let (w, h) = ((1200.0 * scale) as u32, (800.0 * scale) as u32);
        app::resize(w, h, scale);
        let session = Session::boot(init.as_deref(), &args);
        if let Err(e) = session {
            eprintln!("edit: {e}");
            std::process::exit(1);
        }
        // A terminal's first output arrives on its own thread: give it a
        // moment, so the frame shows the shell's prompt and not a blank.
        if app::terminal_showing() {
            std::thread::sleep(std::time::Duration::from_millis(2000));
        }
        let mut fb = Framebuffer::new(w, h);
        app::draw(&mut fb);
        if let Err(e) = fb.save_png(&out) {
            eprintln!("edit: {out}: {e}");
            std::process::exit(1);
        }
        return;
    }
    // A film (`edit::film`): its frame size, and its script's port.
    let film = std::env::var("EDIT_FILM_PORT").ok().and_then(|p| p.parse::<u16>().ok()).map(Film::from_env);
    // The layout needs a size before the boot echoes into the panes; the
    // window's own arrives with `resumed`, a film's is its own.
    match &film {
        Some(f) => app::resize(f.size.0, f.size.1 - f.band, f.scale),
        None => app::resize(1000, 760, 1.0),
    }
    let session = match Session::boot(init.as_deref(), &args) {
        Ok(mut s) => {
            if film.is_some()
                && let Err(e) = s.film()
            {
                eprintln!("edit: film: {e}");
            }
            s
        }
        Err(e) => {
            eprintln!("edit: {e}");
            std::process::exit(1);
        }
    };
    let event_loop = EventLoop::<()>::with_user_event().build().expect("an event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    // A terminal's output, and a remote Scheme's answers, arrive on reader
    // threads; the window must be woken to draw and deliver them.
    let proxy = std::sync::Mutex::new(event_loop.create_proxy());
    edit::wake::set_waker(Box::new(move || {
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
        dragging_divider: None,
        selecting: None,
        resize_cursor: None,
        film,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("edit: {e}");
    }
    // The Schemes this editor started end with it.
    edit::scm_binding::remote::close_all();
}
