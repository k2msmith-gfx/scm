//! **The terminal pane** (docs/editor-crate-plan.md, E5): a program in a
//! pseudo-terminal — a REPL (`scm`, `python3`, `ghci`), a shell, a build —
//! drawn into the framebuffer one glyph per cell.
//!
//! alacritty's terminal library does the terminal: it opens the PTY, runs the
//! child, keeps a reader thread that parses what it prints into a screen grid
//! with scrollback, and reports what happened through an event listener. The
//! plan named two crates (`portable-pty` for the PTY, `alacritty_terminal`
//! for the grid); the second turned out to carry its own PTY and the reader
//! loop that ties the two together, so it is the one.
//!
//! Each terminal is keyed by the PANE that shows it, like every widget here.
//! The registry is the UI thread's; the reader thread touches only the
//! shared grid, and wakes the host through [`set_waker`] — a pane's dirty
//! flag is the UI thread's, so a terminal that printed something must ask
//! the window to come round, not set the flag itself.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

use crate::framebuffer::{Framebuffer, Rect};
use crate::text;

const PAD: f32 = 6.0;

/// How the host is told a terminal has something new to show — from the
/// reader thread, so it must reach the window's event loop (winit's proxy).
static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Install the host's wake-up. The first one stays.
pub fn set_waker(wake: Box<dyn Fn() + Send + Sync>) {
    let _ = WAKER.set(wake);
}

fn wake() {
    if let Some(w) = WAKER.get() {
        w();
    }
}

/// What the reader thread tells us: new output, an answer the terminal owes
/// the child (a cursor-position report), the child's exit.
#[derive(Clone)]
struct Listener {
    sender: Arc<Mutex<Option<EventLoopSender>>>,
    exited: Arc<AtomicBool>,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                if let Some(s) = self.sender.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    let _ = s.send(Msg::Input(text.into_bytes().into()));
                }
            }
            Event::Title(t) => *self.title.lock().unwrap_or_else(|e| e.into_inner()) = Some(t),
            Event::ChildExit(_) | Event::Exit => {
                self.exited.store(true, Ordering::SeqCst);
                wake();
            }
            Event::Wakeup => wake(),
            _ => {}
        }
    }
}

/// A grid size, as alacritty asks for one.
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    exited: Arc<AtomicBool>,
    title: Arc<Mutex<Option<String>>>,
    /// The grid size last sent to the child, and the cell size in pixels.
    size: (usize, usize),
    program: String,
}

thread_local! {
    static TERMS: RefCell<HashMap<u32, Terminal>> = RefCell::new(HashMap::new());
}

/// Run `program` with `args` in a terminal shown by pane `id`, replacing
/// one already there. `cwd` is where it starts (the editor's own, if none).
pub fn open(id: u32, program: &str, args: &[String], cwd: Option<&std::path::Path>) -> Result<(), String> {
    close(id);
    let (cols, rows) = (80, 24);
    let opts = tty::Options {
        shell: Some(tty::Shell::new(program.to_string(), args.to_vec())),
        working_directory: cwd.map(|p| p.to_path_buf()),
        drain_on_exit: true,
        env: HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]),
        #[cfg(target_os = "windows")]
        escape_args: true,
    };
    let window = WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: 8, cell_height: 16 };
    let pty = tty::new(&opts, window, id as u64).map_err(|e| format!("{program}: {e}"))?;
    let listener = Listener {
        sender: Arc::new(Mutex::new(None)),
        exited: Arc::new(AtomicBool::new(false)),
        title: Arc::new(Mutex::new(None)),
    };
    let term = Arc::new(FairMutex::new(Term::new(Config::default(), &Size { cols, rows }, listener.clone())));
    let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, true, false).map_err(|e| e.to_string())?;
    let sender = event_loop.channel();
    *listener.sender.lock().unwrap_or_else(|e| e.into_inner()) = Some(event_loop.channel());
    event_loop.spawn();
    TERMS.with(|t| {
        t.borrow_mut().insert(
            id,
            Terminal {
                term,
                sender,
                exited: listener.exited,
                title: listener.title,
                size: (cols, rows),
                program: program.to_string(),
            },
        )
    });
    crate::host::mark_dirty();
    Ok(())
}

/// Is a terminal shown by pane `id`?
pub fn is_open(id: u32) -> bool {
    TERMS.with(|t| t.borrow().contains_key(&id))
}

/// Has its program exited?
pub fn exited(id: u32) -> bool {
    TERMS.with(|t| t.borrow().get(&id).is_some_and(|t| t.exited.load(Ordering::SeqCst)))
}

/// Stop the program and forget the terminal.
pub fn close(id: u32) {
    if let Some(t) = TERMS.with(|t| t.borrow_mut().remove(&id)) {
        let _ = t.sender.send(Msg::Shutdown);
        crate::host::mark_dirty();
    }
}

/// Write `bytes` to the program, as if typed. A view scrolled back into the
/// history returns to the bottom, as a terminal does when you type.
pub fn send(id: u32, bytes: &[u8]) -> bool {
    TERMS.with(|t| {
        let terms = t.borrow();
        let Some(term) = terms.get(&id) else { return false };
        term.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        term.sender.send(Msg::Input(bytes.to_vec().into())).is_ok()
    })
}

/// A key, by the keymap's name for it (`"return"`, `"C-c"`, `"M-b"`), as
/// the bytes a terminal sends for it — in the cursor mode the program asked
/// for. `None` for a key a terminal has no bytes for.
pub fn key_bytes(desc: &str, app_cursor: bool) -> Option<Vec<u8>> {
    let (mut ctrl, mut meta) = (false, false);
    let mut rest = desc;
    loop {
        if let Some(r) = rest.strip_prefix("C-") {
            ctrl = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("M-") {
            meta = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("S-") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("s-") {
            // The command key is the host's, never the child's.
            let _ = r;
            return None;
        } else {
            break;
        }
    }
    let arrow = |c: char| -> Vec<u8> { if app_cursor { format!("\x1bO{c}") } else { format!("\x1b[{c}") }.into_bytes() };
    let mut out: Vec<u8> = match rest {
        "return" => b"\r".to_vec(),
        "tab" => b"\t".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "space" => b" ".to_vec(),
        "delete" => b"\x1b[3~".to_vec(),
        "up" => arrow('A'),
        "down" => arrow('B'),
        "right" => arrow('C'),
        "left" => arrow('D'),
        "home" => b"\x1b[H".to_vec(),
        "end" => b"\x1b[F".to_vec(),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        s if s.chars().count() == 1 => {
            let c = s.chars().next()?;
            if ctrl {
                // C-a … C-z, C-@, C-[ … C-_ : the control codes.
                let up = c.to_ascii_uppercase();
                if ('@'..='_').contains(&up) {
                    vec![(up as u8) & 0x1f]
                } else if c == ' ' {
                    vec![0]
                } else {
                    return None;
                }
            } else {
                s.as_bytes().to_vec()
            }
        }
        _ => return None,
    };
    if ctrl && rest == "space" {
        out = vec![0];
    }
    if meta {
        out.insert(0, 0x1b);
    }
    Some(out)
}

/// Send the key `desc` to the program. False when there is no terminal, or
/// no bytes for the key.
pub fn key(id: u32, desc: &str) -> bool {
    let app_cursor = TERMS.with(|t| {
        t.borrow().get(&id).map(|t| t.term.lock().mode().contains(TermMode::APP_CURSOR))
    });
    match (app_cursor, key_bytes(desc, app_cursor.unwrap_or(false))) {
        (Some(_), Some(bytes)) => send(id, &bytes),
        _ => false,
    }
}

/// Scroll the view through the history by `rows` (positive = back).
pub fn scroll(id: u32, rows: i32) {
    TERMS.with(|t| {
        if let Some(term) = t.borrow().get(&id) {
            term.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Delta(rows));
        }
    });
    crate::host::mark_dirty();
}

/// The screen as text, one string per row, trailing blanks trimmed — what
/// a test reads, and what a send's caller may search.
pub fn screen_text(id: u32) -> Vec<String> {
    TERMS.with(|t| {
        let terms = t.borrow();
        let Some(term) = terms.get(&id) else { return Vec::new() };
        let term = term.term.lock();
        let grid = term.grid();
        let (rows, cols) = (grid.screen_lines(), grid.columns());
        let mut out = vec![String::new(); rows];
        for cell in term.renderable_content().display_iter {
            let row = (cell.point.line.0 + term.grid().display_offset() as i32) as usize;
            if row < rows && cell.point.column.0 < cols && !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                out[row].push(cell.c);
            }
        }
        out.iter().map(|l| l.trim_end().to_string()).collect()
    })
}

/// The program a terminal runs, and its title if it set one.
pub fn describe(id: u32) -> Option<(String, Option<String>)> {
    TERMS.with(|t| {
        t.borrow()
            .get(&id)
            .map(|t| (t.program.clone(), t.title.lock().unwrap_or_else(|e| e.into_inner()).clone()))
    })
}

/// The xterm palette: the 16 named colours, the 6×6×6 cube, the grey ramp.
fn indexed(i: u8) -> u32 {
    const BASE: [u32; 16] = [
        0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6,
        0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
    ];
    match i {
        0..=15 => BASE[i as usize],
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v as u32 };
            (level(n / 36) << 16) | (level((n / 6) % 6) << 8) | level(n % 6)
        }
        _ => {
            let g = 8 + 10 * (i as u32 - 232);
            (g << 16) | (g << 8) | g
        }
    }
}

fn rgb(c: Rgb) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

/// A cell colour as ink: what the program set, else the theme's.
fn ink(c: Color, colors: &alacritty_terminal::term::color::Colors, theme: &crate::theme::Theme) -> u32 {
    match c {
        Color::Spec(c) => rgb(c),
        Color::Indexed(i) => colors[i as usize].map_or_else(|| indexed(i), rgb),
        Color::Named(n) => {
            if let Some(c) = colors[n as usize] {
                return rgb(c);
            }
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => theme.text_ink,
                NamedColor::DimForeground => theme.text_dim,
                NamedColor::Background => theme.editor_bg,
                NamedColor::Cursor => theme.text_accent,
                n if (n as usize) < 16 => indexed(n as u8),
                n => {
                    // The dim colours: the normal one, darker.
                    let base = indexed((n as usize - NamedColor::DimBlack as usize) as u8);
                    let half = |v: u32| (v >> 1) & 0x7f7f7f;
                    half(base) + half(half(base))
                }
            }
        }
    }
}

/// Draw pane `id`'s terminal into `rect`: the grid at the text size, the
/// cursor a block while the pane has the keyboard. A pane whose size changed
/// tells the program its new size (SIGWINCH) here, where the size is known.
pub fn draw(id: u32, fb: &mut Framebuffer, rect: Rect, focused: bool) {
    let rect = rect.clamped(fb.width, fb.height);
    let theme = crate::theme::get();
    fb.fill_rect(rect, theme.editor_bg);
    let scale = crate::host::ui_scale();
    let px = crate::textpane::font_px() * scale;
    let pad = PAD * scale;
    let (advance, row_h) = text::mono_metrics(px);
    let row_h = row_h.ceil();
    let cols = (((rect.w as f32 - 2.0 * pad) / advance).floor() as usize).max(2);
    let rows = (((rect.h as f32 - 2.0 * pad) / row_h).floor() as usize).max(1);
    let typing = crate::host::typing() == Some(id);
    TERMS.with(|t| {
        let mut terms = t.borrow_mut();
        let Some(term) = terms.get_mut(&id) else { return };
        if term.size != (cols, rows) {
            term.size = (cols, rows);
            term.term.lock().resize(Size { cols, rows });
            let window = WindowSize {
                num_lines: rows as u16,
                num_cols: cols as u16,
                cell_width: advance as u16,
                cell_height: row_h as u16,
            };
            let _ = term.sender.send(Msg::Resize(window));
        }
        let exited = term.exited.load(Ordering::SeqCst);
        let term = term.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let colors = content.colors;
        let cursor = content.cursor.point;
        let (x0, y0) = (rect.x as f32 + pad, rect.y as f32 + pad);
        let cell_at = |p: Point| (x0 + p.column.0 as f32 * advance, y0 + (p.line.0 + offset) as f32 * row_h);
        for cell in content.display_iter {
            let flags = cell.flags;
            if flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let (mut fg, mut bg) = (ink(cell.fg, colors, &theme), ink(cell.bg, colors, &theme));
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let (x, y) = cell_at(cell.point);
            if y + row_h > (rect.y + rect.h) as f32 {
                continue;
            }
            let w = if flags.contains(Flags::WIDE_CHAR) { 2.0 * advance } else { advance };
            if bg != theme.editor_bg {
                fb.fill_rect(Rect { x: x as u32, y: y as u32, w: w.ceil() as u32, h: row_h as u32 }, bg);
            }
            if cell.c != ' ' && !flags.contains(Flags::HIDDEN) {
                let mut buf = [0u8; 4];
                text::draw_mono_line(fb, x, y, px, fg, cell.c.encode_utf8(&mut buf));
            }
            if flags.intersects(Flags::ALL_UNDERLINES) {
                let uy = (y + row_h - scale.max(1.0)) as u32;
                fb.fill_rect(Rect { x: x as u32, y: uy, w: w.ceil() as u32, h: scale.max(1.0) as u32 }, fg);
            }
        }
        // The cursor: a translucent block over its cell while typing, an
        // outline when the keyboard is elsewhere, none once the program is gone.
        if !exited && term.mode().contains(TermMode::SHOW_CURSOR) {
            let (x, y) = cell_at(cursor);
            let r = Rect { x: x as u32, y: y as u32, w: advance.ceil() as u32, h: row_h as u32 };
            if typing || focused {
                let a = theme.text_accent;
                for py in r.y..(r.y + r.h).min(fb.height) {
                    for px_ in r.x..(r.x + r.w).min(fb.width) {
                        let i = (py * fb.width + px_) as usize;
                        fb.pixels[i] = text::blend(fb.pixels[i], a, if typing { 0.45 } else { 0.2 });
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_the_bytes_a_terminal_sends() {
        assert_eq!(key_bytes("return", false).unwrap(), b"\r");
        assert_eq!(key_bytes("C-c", false).unwrap(), [3]);
        assert_eq!(key_bytes("C-d", false).unwrap(), [4]);
        assert_eq!(key_bytes("M-b", false).unwrap(), b"\x1bb");
        assert_eq!(key_bytes("up", false).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("up", true).unwrap(), b"\x1bOA", "application cursor mode");
        assert_eq!(key_bytes("backspace", false).unwrap(), [0x7f]);
        assert!(key_bytes("s-c", false).is_none(), "the command key stays the host's");
        assert!(key_bytes("f5", false).is_none());
    }

    /// A real program in a real pseudo-terminal: what it prints lands on the
    /// screen, what is typed reaches it, and its exit is seen.
    #[test]
    fn a_program_runs_in_the_pane_and_its_output_lands_on_the_screen() {
        let id = 990;
        open(id, "/bin/sh", &[], None).expect("a shell");
        let wait = |want: &dyn Fn(&[String]) -> bool| {
            let t0 = std::time::Instant::now();
            while t0.elapsed() < std::time::Duration::from_secs(10) {
                if want(&screen_text(id)) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        };
        send(id, b"echo hel''lo-$((6*7))\r");
        assert!(wait(&|s| s.iter().any(|l| l == "hello-42")), "{:?}", screen_text(id));
        // Drawn: the pane paints, and the size it drew at reached the child.
        let mut fb = Framebuffer::new(400, 200);
        draw(id, &mut fb, Rect { x: 0, y: 0, w: 400, h: 200 }, true);
        let (cols, _) = TERMS.with(|t| t.borrow()[&id].size);
        send(id, b"tput cols\r");
        assert!(wait(&|s| s.iter().any(|l| l == &cols.to_string())), "cols {cols}: {:?}", screen_text(id));
        key(id, "C-d");
        let t0 = std::time::Instant::now();
        while !exited(id) && t0.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(exited(id), "C-d ended the shell");
        close(id);
        assert!(!is_open(id));
    }
}
