//! **The standalone program's Scheme session** (docs/editor-crate-plan.md,
//! E4): the kernel prims the shell asks a host for (00-base.scm's list),
//! answered from `crate::app`'s three panes, and the session that boots the
//! shell and takes the window's events. Here because it names the VM; the
//! window (`bin/edit.rs`) and the layout (`app.rs`) do not.

use scm::Val;

use super::{val_num, val_text};
use crate::app;

/// The host's Scheme: ray's helpers (`host/prelude.scm`) before the shell,
/// the program's hooks (`host/standalone.scm`) after it.
const HOST_PRELUDE: &str = "host/prelude";
const HOST_SETUP: &str = "host/standalone";

fn os_stat(path: &str) -> Val {
    let Ok(md) = std::fs::metadata(path) else { return Val::Bool(false) };
    let modified = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64());
    let entry = |k: &str, v: Val| Val::Pair(Box::new(Val::Keyword(k.into())), Box::new(v));
    Val::List(vec![
        entry("mode", Val::Keyword(if md.is_dir() { "directory" } else { "file" }.into())),
        entry("size", Val::Int(md.len() as i64)),
        entry("modified", Val::Float(modified)),
    ])
}

/// The kernel prims, as the modeler's `script/panes.rs` and ray's binding
/// answer them, over `app`'s panes and the file system.
fn install_host_prims(interp: &mut scm::Interp) -> Result<(), scm::EmbedError> {
    let id = |args: &[Val]| val_num(args, 0) as u32;
    interp.define_fn("%focused-pane", |_| Ok(Val::Int(app::focused() as i64)))?;
    interp.define_fn("%typing-pane", |_| Ok(app::typing().map_or(Val::Bool(false), |t| Val::Int(t as i64))))?;
    interp.define_fn("%focus-pane", move |args| {
        app::focus(id(args));
        Ok(Val::Bool(false))
    })?;
    interp.define_fn("%hold-typing", move |args| {
        app::hold_typing(id(args));
        Ok(Val::Bool(false))
    })?;
    interp.define_fn("%release-typing", |_| {
        app::release_typing();
        Ok(Val::Bool(false))
    })?;
    interp.define_fn("%pane-at", |args| {
        Ok(match app::pane_at(val_num(args, 0), val_num(args, 1)) {
            Some(id) => Val::List(vec![Val::Int(id as i64), Val::Str(app::kind_name(id).unwrap_or("text").into())]),
            None => Val::Bool(false),
        })
    })?;
    interp.define_fn("%pane-rect", move |args| {
        Ok(match app::rect(id(args)) {
            Some(r) => Val::List(vec![Val::Int(r.x as i64), Val::Int(r.y as i64), Val::Int(r.w as i64), Val::Int(r.h as i64)]),
            None => Val::Bool(false),
        })
    })?;
    interp.define_fn("%set-pie-delay", |args| {
        app::set_pie_delay(val_num(args, 0).max(0.0) as u64);
        Ok(Val::Bool(false))
    })?;
    // `(%show-panel :terminal / :repl / #f)` — what the bottom panel shows,
    // or hide it; `(%panel)` answers which, or #f.
    interp.define_fn("%show-panel", |args| {
        app::show_panel(match val_text(args, 0).as_deref() {
            Some("terminal") => Some(app::Panel::Terminal),
            Some("repl") => Some(app::Panel::Repl),
            _ => None,
        });
        Ok(Val::Bool(false))
    })?;
    interp.define_fn("%panel", |_| {
        Ok(match app::panel() {
            Some(app::Panel::Terminal) => Val::Keyword("terminal".into()),
            Some(app::Panel::Repl) => Val::Keyword("repl".into()),
            None => Val::Bool(false),
        })
    })?;
    // `(%show-files on)` / `(%files-shown?)` — the file tree's column.
    interp.define_fn("%show-files", |args| {
        app::show_files(!matches!(args.first(), Some(Val::Bool(false)) | None));
        Ok(Val::Bool(false))
    })?;
    interp.define_fn("%files-shown?", |_| Ok(Val::Bool(app::files_shown())))?;
    // `(%git-ignored dir)` — what git ignores under DIR, relative to it, an
    // ignored directory once with a trailing `/` (ray's prim of the same
    // name, which a standalone without ray answers itself); () outside git.
    interp.define_fn("%git-ignored", |args| {
        let Some(dir) = val_text(args, 0) else { return Ok(Val::List(Vec::new())) };
        let out = std::process::Command::new("git")
            .args(["-C", &dir, "ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"])
            .stderr(std::process::Stdio::null())
            .output();
        Ok(Val::List(match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(|p| Val::Str(p.to_string()))
                .collect(),
            _ => Vec::new(),
        }))
    })?;
    // `(%read-status text)` — complete, incomplete or invalid, by scm's
    // reader: whether the REPL's RET submits or continues the form.
    interp.define_fn("%read-status", |args| {
        Ok(Val::Sym(
            match scm::parse_status(&val_text(args, 0).unwrap_or_default()) {
                scm::ParseStatus::Complete => "complete",
                scm::ParseStatus::Incomplete => "incomplete",
                scm::ParseStatus::Invalid(_) => "invalid",
            }
            .into(),
        ))
    })?;
    interp.define_fn("%quit", |_| {
        app::request_quit();
        Ok(Val::Bool(false))
    })?;
    interp.define_fn("%os-clock", |_| {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        Ok(Val::Float(START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()))
    })?;
    interp.define_fn("%os-stat", |args| Ok(val_text(args, 0).map_or(Val::Bool(false), |p| os_stat(&p))))?;
    interp.define_fn("%os-dir", |args| {
        let Some(path) = val_text(args, 0) else { return Ok(Val::Bool(false)) };
        let Ok(entries) = std::fs::read_dir(&path) else { return Ok(Val::Bool(false)) };
        Ok(Val::List(entries.flatten().map(|e| Val::Str(e.file_name().to_string_lossy().into_owned())).collect()))
    })?;
    interp.define_fn("%os-realpath", |args| {
        Ok(val_text(args, 0)
            .and_then(|p| std::fs::canonicalize(p).ok())
            .map_or(Val::Bool(false), |p| Val::Str(p.to_string_lossy().into_owned())))
    })?;
    Ok(())
}

/// A string as a Scheme literal.
fn lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn bool_lit(b: bool) -> &'static str {
    if b { "#t" } else { "#f" }
}

/// The standalone editor's image: the shell, its hooks, the user's init.
pub struct Session {
    interp: scm::Interp,
}

impl Session {
    /// Boot: the editor's prims and the host's, ray's helpers, the shell,
    /// the program's hooks — then `init` (the user's file, if it exists) —
    /// then each of `files` opened as a buffer. An error in `init` or a
    /// file is reported in the transcript, not fatal: the editor is how
    /// you fix a broken init.
    pub fn boot(init: Option<&std::path::Path>, files: &[String]) -> Result<Session, String> {
        app::install_host();
        let mut interp = scm::Interp::new().map_err(|e| e.to_string())?;
        super::install(&mut interp, &mut |_| {}).map_err(|e| e.to_string())?;
        install_host_prims(&mut interp).map_err(|e| e.to_string())?;
        let load = |interp: &mut scm::Interp, name: &str| -> Result<(), String> {
            let (src, path) = super::shell_source(name)?;
            interp.eval_named(&src, &path).map(|_| ()).map_err(|e| format!("{name}: {e}"))
        };
        load(&mut interp, HOST_PRELUDE)?;
        for name in super::SHELL {
            load(&mut interp, name)?;
        }
        load(&mut interp, HOST_SETUP)?;
        let mut session = Session { interp };
        session.call("(edit-greeting)");
        if let Some(init) = init.filter(|p| p.exists()) {
            let path = init.to_string_lossy();
            session.call(&format!(
                "(guard (e (#t (*transcript-append* (string-append \"init: \" (if (string? e) e (*show-value* e))) :error))) \
                   (load {}))",
                lit(&path)
            ));
        }
        for f in files {
            session.call(&format!("(open-path {})", lit(f)));
        }
        let names: Vec<String> = files.iter().map(|f| lit(f)).collect();
        session.call(&format!("(edit-open-files (list {}))", names.join(" ")));
        session.call("(focus-scene-editor)");
        Ok(session)
    }

    /// Evaluate `src` in the image; an error goes to the transcript (a
    /// command's own errors reach the echo through run-command first).
    pub fn call(&mut self, src: &str) {
        if let Err(e) = self.interp.eval(src) {
            let line = format!("error: {e}");
            crate::textpane::append(app::TRANSCRIPT, crate::textpane::Style::Error, &line);
            app::mark_dirty();
        }
    }

    pub fn key(&mut self, desc: &str) {
        self.call(&format!("(on-key {})", lit(desc)));
    }

    pub fn text(&mut self, text: &str) {
        self.call(&format!("(on-text {})", lit(text)));
    }

    pub fn click(&mut self, x: f64, y: f64, shift: bool, ctrl: bool) {
        self.call(&format!("(on-click {x:.1} {y:.1} {} {})", bool_lit(shift), bool_lit(ctrl)));
    }

    pub fn drag(&mut self, dx: f64, dy: f64, alt: bool, shift: bool) {
        self.call(&format!("(on-mouse-drag :left {dx:.1} {dy:.1} {} {})", bool_lit(alt), bool_lit(shift)));
    }

    pub fn release(&mut self, x: f64, y: f64) {
        self.call(&format!("(on-release {x:.1} {y:.1})"));
    }

    pub fn wheel(&mut self, d: f64, x: f64, y: f64, alt: bool, shift: bool) {
        self.call(&format!("(on-wheel {d:.3} {x:.1} {y:.1} {} {})", bool_lit(alt), bool_lit(shift)));
    }

    pub fn cursor_move(&mut self, x: f64, y: f64) {
        self.call(&format!("(on-cursor-move {x:.1} {y:.1})"));
    }

    pub fn menu_press(&mut self, x: f64, y: f64, shift: bool) {
        self.call(&format!("(on-menu-press {x:.1} {y:.1} {})", bool_lit(shift)));
    }

    pub fn menu_hold(&mut self) {
        self.call("(on-menu-hold)");
    }

    pub fn menu_drag(&mut self, x: f64, y: f64) {
        self.call(&format!("(on-menu-drag {x:.1} {y:.1})"));
    }

    pub fn menu_release(&mut self, x: f64, y: f64) {
        self.call(&format!("(on-menu-release {x:.1} {y:.1})"));
    }

    /// Escape, by the modeler's ladder: an open completion closes, then a vi
    /// editor leaves its mode, then an open menu closes, then a typing pane
    /// lets go of the keyboard. Answers whether anything took it.
    pub fn escape(&mut self) -> bool {
        let typing = app::typing();
        // A terminal's program gets Escape, as every other key (D4): vi in a
        // terminal needs it. C-x o is how the keyboard leaves.
        if let Some(t) = typing.filter(|&t| crate::termpane::is_open(t)) {
            return crate::termpane::key(t, "escape");
        }
        if crate::textpane::overlay_active() {
            crate::textpane::clear_completions();
            self.call("(begin (editor-comp-cancel) (on-menu-cancel))");
            return true;
        }
        // A query-replace running stops, and a search's single keys end:
        // Escape is one of their quit keys, ahead of vi's own use of it (an
        // open ? menu closes first, above).
        if self.eval_to_string("(if (search-escape) 1 0)").as_deref() == Ok("1") {
            return true;
        }
        if typing == Some(app::EDITOR) && crate::editorpane::vi_escape(app::EDITOR) {
            return true;
        }
        if typing.is_some() && typing != Some(app::EDITOR) {
            self.call("(on-escape)");
            app::release_typing();
            self.call("(on-typing-released)");
            self.call("(focus-scene-editor)");
            return true;
        }
        false
    }

    /// Deliver the answers async sends are owed — what the window does when
    /// a connection's reader wakes it.
    pub fn poll(&mut self) {
        self.call("(remote-poll)");
    }

    /// Poll until every async send has its answer, for at most `secs` —
    /// what a test does instead of a window's loop.
    pub fn settle(&mut self, secs: f64) -> bool {
        let t0 = std::time::Instant::now();
        loop {
            self.poll();
            if self.eval_to_string("(zero? (hash-table-size *remote-callbacks*))").as_deref() == Ok("#t") {
                return true;
            }
            if t0.elapsed().as_secs_f64() > secs {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The value of `src`, written — for tests.
    pub fn eval_to_string(&mut self, src: &str) -> Result<String, String> {
        let v = self.interp.eval(src).map_err(|e| e.to_string())?;
        self.interp.write(&v).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // That host/prelude.scm's copies of ray's helpers are still ray's text
    // is checked in ray (the modeler's tests), which has both files.

    /// The centres of the strip's REPL and terminal icons, after a draw.
    fn icons() -> ((f64, f64), (f64, f64), (f64, f64)) {
        let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
        app::draw(&mut fb);
        let m = app::rect(app::MINIBUFFER).unwrap();
        // Right-aligned 28 px in (clear of the OS resize corner): 15 px
        // squares, 5 px apart, on the strip.
        let (right, y) = ((m.x + m.w) as f64 - 28.0, m.y as f64 + m.h as f64 / 2.0);
        // Left to right: the file tree's, the REPL's, the terminal's.
        ((right - 2.0 * (15.0 + 5.0) - 7.5, y), (right - 15.0 - 5.0 - 7.5, y), (right - 7.5, y))
    }

    /// The window's opening size (Kevin, 2026-11-14): 1920×1080 on the Mac's
    /// 2560×1440, kept inside a smaller screen, a default with none; and
    /// maximized only on Linux.
    #[test]
    fn the_window_opens_at_blenders_size_inside_the_screen() {
        assert_eq!(app::start_window_size(Some((2560.0, 1440.0))), (1920.0, 1080.0));
        assert_eq!(app::start_window_size(Some((1440.0, 900.0))), (1354.0, 810.0));
        assert_eq!(app::start_window_size(None), (1600.0, 900.0));
        assert_eq!(app::START_MAXIMIZED, cfg!(target_os = "linux"));
    }

    /// The program without its window: boot the shell over the three panes,
    /// type a form, send it, save the buffer, and draw.
    #[test]
    fn the_standalone_session_types_sends_saves_and_draws() {
        let dir = std::env::temp_dir().join(format!("edit-standalone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hello.scm");
        std::fs::write(&file, "(define greeting \"hi\")\n").unwrap();
        let file = file.to_string_lossy().replace('\\', "/");
        crate::editorpane::set_vi(false);
        app::resize(800, 600, 1.0);
        let mut s = Session::boot(None, std::slice::from_ref(&file)).expect("boot");
        // A Scheme file sends to a separate scm by default
        // (`files_default_to_a_separate_scm`); this test is about the
        // editor, so it keeps the file in the editor's image.
        s.call("(run-in-editor-image)");
        assert_eq!(app::typing(), Some(app::EDITOR), "the editor has the keyboard");
        assert_eq!(app::panel(), None, "no REPL or terminal at startup");
        assert!(!crate::termpane::is_open(app::TRANSCRIPT), "and no shell started");
        assert_eq!(s.eval_to_string("(%editor-text 1)").unwrap(), "\"(define greeting \\\"hi\\\")\\n\"");

        // Type at the end and send the form before point: C-x C-e.
        s.call("(%editor-act 1 \"buffer-end\")");
        for c in "(string-length greeting)".chars() {
            s.text(&c.to_string());
        }
        s.call("(eval-buffer)"); // greeting defined
        s.key("C-x");
        s.key("C-e");
        let transcript: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(transcript.iter().any(|l| l == "=> 2"), "{transcript:?}");

        // C-x C-s writes the file.
        s.key("C-x");
        s.key("C-s");
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(saved.ends_with("(string-length greeting)"), "{saved:?}");

        // M-x prompts with a cursor where the command goes.
        let theme = crate::theme::Theme::default();
        let accent_in = |r: crate::framebuffer::Rect| {
            let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
            app::draw(&mut fb);
            (r.y..r.y + r.h)
                .flat_map(|y| (r.x..r.x + r.w).map(move |x| (x, y)))
                .filter(|&(x, y)| fb.pixels[(y * 800 + x) as usize] == theme.text_accent)
                .count()
        };
        let mini = app::rect(app::MINIBUFFER).unwrap();
        let strip = crate::framebuffer::Rect { w: mini.w / 2, ..mini };
        let before = accent_in(strip);
        s.key("M-x");
        assert_eq!(app::typing(), Some(app::MINIBUFFER));
        assert!(accent_in(strip) > before + 20, "the prompt draws its cursor block");
        // Backspace edits the line; C-g aborts and gives the editor back.
        for c in "abc".chars() {
            s.text(&c.to_string());
        }
        s.key("backspace");
        assert_eq!(crate::textpane::input(app::MINIBUFFER).0, "ab");
        s.key("C-g");
        assert_eq!(app::typing(), Some(app::EDITOR), "C-g aborted the prompt");
        // C-x C-f prompts in the minibuffer; Escape gives the keyboard back.
        s.key("C-x");
        s.key("C-f");
        assert_eq!(app::typing(), Some(app::MINIBUFFER));
        assert!(s.escape());
        assert_eq!(app::typing(), Some(app::EDITOR));

        let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
        app::draw(&mut fb);
        let theme = crate::theme::Theme::default();
        assert!(fb.pixels.iter().filter(|&&p| p == theme.editor_bg).count() > 10_000, "the editor painted");

        // E5: a program in the lower pane takes the keyboard; what is typed
        // reaches it; C-x o goes back to the editor, and a buffer attached
        // to the terminal sends its forms there.
        s.call(r#"(term-open "/bin/sh")"#);
        assert_eq!(app::typing(), Some(app::TRANSCRIPT));
        assert_eq!(app::kind_name(app::TRANSCRIPT), Some("term"));
        for c in "echo typed-$((2+3))".chars() {
            s.text(&c.to_string());
        }
        s.key("return");
        let wait = |want: &str| {
            let t0 = std::time::Instant::now();
            while t0.elapsed() < std::time::Duration::from_secs(10) {
                if crate::termpane::screen_text(app::TRANSCRIPT).iter().any(|l| l == want) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        };
        assert!(wait("typed-5"), "{:?}", crate::termpane::screen_text(app::TRANSCRIPT));
        s.key("C-x");
        s.key("o");
        assert_eq!(app::typing(), Some(app::EDITOR), "C-x o left the terminal");
        s.call("(term-attach)");
        s.call(r#"(%editor-set-text 1 "echo sent-$((3*3))")"#);
        s.call("(eval-buffer)");
        assert!(wait("sent-9"), "{:?}", crate::termpane::screen_text(app::TRANSCRIPT));
        let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
        app::draw(&mut fb);
        // The strip's two icons: the terminal's, lit, hides the whole panel
        // (the program runs on) and shows it again; the λ swaps in the REPL.
        assert_eq!(app::panel(), Some(app::Panel::Terminal), "running an attached buffer shows its terminal");
        let (files_icon, repl_icon, term_icon) = icons();
        assert_eq!(app::icon_at(term_icon.0, term_icon.1), Some(app::Icon::Terminal));
        assert_eq!(app::icon_at(repl_icon.0, repl_icon.1), Some(app::Icon::Repl));
        assert_eq!(app::icon_at(files_icon.0, files_icon.1), Some(app::Icon::Files));
        // Hover help: on an icon its tip floats above it, and leaving clears it.
        let frame = || {
            let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
            app::draw(&mut fb);
            fb.pixels
        };
        let plain = frame();
        assert!(app::hover_icon(term_icon.0, term_icon.1), "the pointer arrived on an icon");
        assert_eq!(app::hovered_icon().map(app::Icon::tip), Some("terminal"));
        let tipped = frame();
        let changed: Vec<usize> = (0..plain.len()).filter(|&i| plain[i] != tipped[i]).collect();
        assert!(changed.len() > 200, "a tip is drawn ({} px)", changed.len());
        assert!(changed.iter().all(|&i| ((i / 800) as f64) < term_icon.1 - 7.0), "above the icon");
        assert!(app::hover_icon(-1.0, -1.0), "and left");
        assert_eq!(frame(), plain, "taking the tip with it");
        s.call("(toggle-panel :terminal)");
        assert_eq!(app::panel(), None, "hidden");
        assert!(app::rect(app::TRANSCRIPT).is_none(), "and the editor has the height");
        assert!(crate::termpane::is_open(app::TRANSCRIPT), "the program runs on");
        s.call("(toggle-panel :terminal)");
        assert_eq!(app::kind_name(app::TRANSCRIPT), Some("term"), "shown again");
        assert_eq!(app::typing(), Some(app::TRANSCRIPT));
        s.call("(toggle-panel :repl)");
        assert_eq!(app::panel(), Some(app::Panel::Repl), "the REPL instead");
        assert_eq!(app::kind_name(app::TRANSCRIPT), Some("text"));
        // The REPL: a complete form answers; an open one continues.
        for c in "(* 6 7)".chars() {
            s.text(&c.to_string());
        }
        s.key("return");
        for line in ["(+ 1", "2)"] {
            for c in line.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
        }
        let transcript: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(transcript.iter().any(|l| l == "=> 42"), "{transcript:?}");
        assert!(transcript.iter().any(|l| l == "=> 3"), "a form over two lines: {transcript:?}");
        // Guile's REPL command names, at scm's prompt.
        let type_line = |s: &mut Session, line: &str| {
            for c in line.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
        };
        for line in [
            r#"(define (area r) "Circle area." (* 3 r r))"#,
            ",help",
            ",describe area",
            ",a circle",
            ",expand1 (when a b)",
            ",t (area 2)",
            "(car 1)",
            ",bt",
            ",frobnicate",
        ] {
            type_line(&mut s, line);
        }
        let t: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        let has = |want: &str| t.iter().any(|l| l.contains(want));
        assert!(has(",describe NAME"), ",help lists the commands: {t:?}");
        assert!(has("(area r)") && has("Circle area."), ",describe: {t:?}");
        assert!(has("=> (area)"), ",a finds area by its docstring: {t:?}");
        assert!(has("=> (if a (begin b)"), ",expand1: {t:?}");
        assert!(has("=> 12") && t.iter().any(|l| l.starts_with(";; ") && l.ends_with(" s")), ",t: {t:?}");
        assert!(has("  in (car 1)"), ",bt shows the last error's call: {t:?}");
        assert!(has("unknown command ,frobnicate"), "{t:?}");
        s.call("(term-close)");
        assert_eq!(app::panel(), Some(app::Panel::Repl), "a closed terminal leaves the REPL");

        // The divider is the editor's modeline: grabbed on the bar (its
        // centre line, not the panel's edge below it), dragged up, the bar's
        // centre follows the pointer and the panel grows to meet it.
        let top = app::rect(app::TRANSCRIPT).unwrap().y as f64;
        let half = crate::editorpane::modeline_height(1.0) as f64 / 2.0;
        let mid = { let e = app::rect(app::EDITOR).unwrap(); e.x as f64 + e.w as f64 / 2.0 };
        assert_eq!(app::divider_at(mid, top - half), Some(app::Divider::Panel), "the bar's centre line");
        assert_eq!(app::divider_at(mid, top + 6.0), None, "not the panel's own text below it");
        app::drag_divider(app::Divider::Panel, mid, top - half - 100.0);
        let (ed, panel) = (app::rect(app::EDITOR).unwrap(), app::rect(app::TRANSCRIPT).unwrap());
        assert!((panel.y as f64 - (top - 100.0)).abs() <= 1.0, "the bar moved with the pointer");
        assert_eq!(ed.y + ed.h, panel.y, "the editor ends where the panel starts");

        // Menus: a quick right-click on the editor posts its sends; on the
        // transcript, its own.
        s.call("(focus-scene-editor)");
        let menu_at = |s: &mut Session, id: u32| {
            let r = app::rect(id).unwrap();
            let (x, y) = (r.x as f64 + 40.0, r.y as f64 + 20.0);
            s.menu_press(x, y, false);
            s.menu_release(x, y);
            let open = s.eval_to_string("(menu-open?)").unwrap();
            let items = s.eval_to_string("(and *menu* (map car (%state-ref *menu* :items)))").unwrap_or_default();
            s.call("(on-menu-cancel)");
            (open, items)
        };
        // Every item that has a key shows it, the sends' guarded lambdas
        // included (the shell labels them).
        let labels = s.eval_to_string("(%item-labels (hash-table-ref *menus* :editor))").unwrap();
        for key in ["C-M-x", "C-x C-e", "C-c C-b", "C-c C-r", "C-c C-d d", "s-c", "s-v"] {
            assert!(labels.contains(&format!("\\t{key}")), "{key} missing: {labels}");
        }
        let (open, items) = menu_at(&mut s, app::EDITOR);
        assert_eq!(open, "#t", "the editor's menu opened");
        assert!(items.contains("send defun"), "{items}");
        let (open, items) = menu_at(&mut s, app::TRANSCRIPT);
        assert_eq!(open, "#t");
        assert!(items.contains("terminal"), "{items}");

        s.key("C-x");
        s.key("C-c");
        assert!(app::quit_requested());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rust with rust-analyzer, through the shell as a user reaches it:
    /// a .rs file opens as Rust, TAB's pool has the crate's names, M-.
    /// jumps, describe shows the doc, the argument hint is echoed when the
    /// host is woken, a saved file's errors are underlined, the modeline
    /// names the server once it is ready, and C-x k lets the file go.
    /// `cargo test -p edit --lib rust_analyzer_through_the_shell -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn rust_analyzer_through_the_shell() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("edit-ra-shell-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        let src = "/// Frobs it.\nfn frobnicate(x: u32) -> u32 { x }\n\nfn main() {\n    let y: u32 = \"no\";\n    frobnicate(1);\n    frob\n}\n";
        let main = dir.join("src/main.rs");
        std::fs::write(&main, src).unwrap();
        let file = main.to_string_lossy().replace('\\', "/");
        crate::editorpane::set_vi(false);
        app::resize(800, 600, 1.0);
        let mut s = Session::boot(None, std::slice::from_ref(&file)).expect("boot");
        assert_eq!(s.eval_to_string("(%buffer-language 1 (%editor-active-buffer 1))").unwrap(), "\"rust\"");
        let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
        app::draw(&mut fb);
        // TAB's pool at the end of `    frob`, once the server has loaded.
        s.call("(%editor-goto-line 1 6) (%editor-act 1 \"end\")");
        let t0 = Instant::now();
        let mut pool = String::new();
        // The server's pool, not the buffer's words (\"Frobs\" is a word of
        // the doc comment) that answer while it loads.
        let served = |p: &str| p.contains("\"frobnicate\"") && !p.contains("\"Frobs\"");
        while t0.elapsed() < Duration::from_secs(120) && !served(&pool) {
            pool = s.eval_to_string("(editor-candidates)").unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("pool after {:?}: {}", t0.elapsed(), &pool[..pool.len().min(120)]);
        assert!(served(&pool), "{pool} / {}", s.eval_to_string("(%lsp-status 1)").unwrap());
        // Describe on the call; M-. to the definition.
        s.call("(editor-goto 1 5 6)");
        let th = Instant::now();
        let mut h = String::from("#f");
        while th.elapsed() < Duration::from_secs(60) && h == "#f" {
            h = s.eval_to_string("(%lsp-hover 1)").unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("hover after {:?}: {h}", th.elapsed());
        s.call("(editor-describe)");
        let transcript: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(transcript.iter().any(|l| l.contains("Frobs it.")), "{transcript:?}");
        s.call("(find-definition-at-point)");
        assert_eq!(s.eval_to_string("(%editor-cursor 1)").unwrap(), "(1 3)");
        // The argument hint, inside `frobnicate(|1)`, arrives on a wake.
        s.call("(go-back-from-definition) (editor-goto 1 5 15) (set! *editor-eldoc* #f) (editor-eldoc)");
        let t1 = Instant::now();
        let echoed = || crate::textpane::lines(app::MINIBUFFER).into_iter().any(|(_, l)| l.contains("fn frobnicate(x: u32)"));
        while t1.elapsed() < Duration::from_secs(10) && !echoed() {
            std::thread::sleep(Duration::from_millis(50));
            s.poll();
        }
        assert!(echoed(), "{:?}", crate::textpane::lines(app::MINIBUFFER));
        // Saved, cargo check's errors come back and are underlined.
        s.key("C-x");
        s.key("C-s");
        let t2 = Instant::now();
        let mut diags = String::new();
        while t2.elapsed() < Duration::from_secs(60) && !diags.contains("mismatched types") {
            std::thread::sleep(Duration::from_millis(200));
            app::draw(&mut fb);
            diags = s.eval_to_string("(%editor-diagnostics 1)").unwrap();
        }
        eprintln!("diagnostics: {diags}");
        assert!(diags.contains("mismatched types"), "{diags}");
        // The modeline's word, once the server has settled.
        let t3 = Instant::now();
        let mut mode = None;
        while t3.elapsed() < Duration::from_secs(60) && mode.as_deref() != Some("rust-analyzer") {
            std::thread::sleep(Duration::from_millis(200));
            mode = crate::lsp::mode(&file, "rust");
        }
        assert_eq!(mode.as_deref(), Some("rust-analyzer"));
        // C-x k: the server is told, and forgets the file.
        s.key("C-x");
        s.key("k");
        assert_eq!(crate::lsp::status(&file, "rust"), "rust-analyzer has not seen this buffer yet");
        assert_eq!(crate::lsp::mode(&file, "rust"), None);
        crate::lsp::stop(None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every row of the :help page names what its key runs. An anonymous
    /// binding printed #<procedure>, which the reading view showed as a bare
    /// `#` (Kevin, 2026-11-14: "C-a - #"), and labels filed by key alone let
    /// the minibuffer's C-a name the editor's.
    /// M-? lists the uses of the name at point across the indexed files —
    /// uses, not words — with each line, and RET goes to one (E8).
    #[test]
    fn find_references_lists_the_uses() {
        app::resize(800, 600, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-refs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.scm"), "(define (slab w) (box w))\n(slab 1)\n").unwrap();
        std::fs::write(dir.join("b.scm"), "(let ((slab 2)) slab)\n'slab\n(map slab xs)\n").unwrap();
        // As the standalone roots it: the real path (/var is /private/var).
        let d = std::fs::canonicalize(&dir).unwrap().to_string_lossy().replace('\\', "/");
        s.call(&format!("(%xref-index (list \"{d}\")) (open-path \"{d}/a.scm\") (editor-goto 1 1 2)"));
        assert_eq!(s.eval_to_string("(length (reference-sites \"slab\"))").unwrap(), "2", "uses, not words");
        s.key("M-S-/");
        let prompt = s.eval_to_string("(%state-ref *edit* :prompt)").unwrap();
        assert_eq!(prompt, "\"2 uses of slab: \"");
        let shown = s.eval_to_string("(%state-ref (%edit :comp) :cands)").unwrap();
        assert!(shown.contains("a.scm:2  (slab 1)") && shown.contains("b.scm:3  (map slab xs)"), "the list is up, each with its line: {shown}");
        let labels = s.eval_to_string("(map (lambda (st) (%list-ref/default st 1 #f)) (reference-sites \"slab\"))").unwrap();
        assert_eq!(labels, "(1 2)", "a.scm line 2, b.scm line 3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Scheme file is judged as it is typed (E8): a name nothing binds and
    /// a call with the wrong count are warnings, M-g n goes to them, the
    /// modeline counts them — and a fix clears them.
    #[test]
    fn a_scheme_file_warns_before_it_runs() {
        app::resize(800, 600, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-warn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("w.scm");
        std::fs::write(&f, "(define (cube x) (* x x x))\n(cube 1 2)\n(dispaly (cube 3))\n(let ((y 2)) (list y 'zz))\n").unwrap();
        let f = std::fs::canonicalize(&f).unwrap().to_string_lossy().replace('\\', "/");
        s.call(&format!("(open-path \"{f}\")"));
        let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
        app::draw(&mut fb);
        let warnings = |s: &mut Session| s.eval_to_string("(filter (lambda (d) (= (cadddr d) 2)) (%editor-diagnostics 1))").unwrap();
        let w = warnings(&mut s);
        assert!(w.contains("(1 1 \"cube takes 1 argument, given 2\" 2)"), "{w}");
        assert!(w.contains("(2 1 \"dispaly is not defined"), "{w}");
        assert_eq!(w.matches("is not defined").count(), 1, "y and 'zz are not warned of: {w}");
        s.call("(%editor-goto-line 1 0) (goto-diagnostic)");
        assert_eq!(s.eval_to_string("(%editor-cursor 1)").unwrap(), "(1 1)", "M-g n goes to the first");
        // Fixed, the warnings go.
        s.call("(%editor-set-text 1 \"(define (cube x) (* x x x))\\n(cube 1)\\n(display (cube 3))\\n\")");
        app::draw(&mut fb);
        assert_eq!(warnings(&mut s), "()");
        s.call("(set-editor-warnings #f)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_help_row_names_its_command() {
        app::resize(800, 600, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        let md = s.eval_to_string("(string-append (vi-help-markdown) (keybindings-markdown))").unwrap();
        let bad: Vec<&str> =
            md.split("\\n").filter(|l| l.contains("#<") || l.contains("unnamed")).collect();
        assert!(bad.is_empty(), "rows without a name: {bad:?}");
        assert!(md.contains("- `C-a` — **editor-line-start**"), "the editor's C-a: {md}");
        assert!(md.contains("- `C-a` — **edit-line-start**"), "the minibuffer's C-a");
        assert!(!md.contains("## Mouse"), "the standalone adds no gestures");
        // A host's sections that are not bindings (the modeler's mouse
        // gestures) close the page, the transcript's rule a heading.
        s.call(r#"(set! *show-keys-extra* (lambda () (list (cons "── mouse ──────" '(("S-drag" "add to the selection"))))))"#);
        let md = s.eval_to_string("(keybindings-markdown)").unwrap();
        assert!(md.contains("\\n## Mouse\\n\\n- `S-drag` — add to the selection\\n"), "{md}");
    }

    /// Where a send goes by default: a Scheme FILE to a separate scm, the
    /// scratch and the editor's own files to the editor's image — and with
    /// scm on the PATH, the file's sends really run over there.
    #[test]
    fn files_default_to_a_separate_scm() {
        app::resize(800, 600, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        let name = |s: &mut Session| {
            s.eval_to_string("(%alist-ref (backend-of (%editor-active-buffer 1)) :name #f)").unwrap()
        };
        assert_eq!(name(&mut s), "\"this image\"", "the scratch is the editor's");
        let shell_file = super::super::shell_dir().join("05-buffers.scm").to_string_lossy().replace('\\', "/");
        s.call(&format!("(open-path \"{shell_file}\")"));
        assert_eq!(name(&mut s), "\"this image\"", "the editor's own files are the editor's");

        let dir = std::env::temp_dir().join(format!("edit-scm-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prog.scm");
        std::fs::write(&file, "(define (cube x) (* x x x))\n(cube 3)\n").unwrap();
        let file = file.to_string_lossy().replace('\\', "/");
        s.call(&format!("(open-path \"{file}\")"));
        let scm_on_path =
            std::process::Command::new("scm").arg("--version").output().is_ok_and(|o| o.status.success());
        if !scm_on_path {
            // Without scm, a file's send fails and says so -- it does not run here.
            s.call("(set! *remote-program* \"no-such-scm-here\") (%editor-goto-line 1 0) (eval-defun)");
            assert_eq!(s.eval_to_string("(%env-ref 'cube #f)").unwrap(), "#f");
            eprintln!("scm is not on the PATH: the separate-process half skipped");
        } else {
            s.call("(%editor-goto-line 1 0) (eval-defun) (%editor-goto-line 1 1) (eval-defun)");
            assert!(s.settle(10.0), "scm answers");
            let t: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
            assert!(t.iter().any(|l| l == "=> 27"), "{t:?}");
            assert_eq!(s.eval_to_string("(%env-ref 'cube #f)").unwrap(), "#f", "not in the editor's image");
            assert!(name(&mut s).starts_with("\"scm at "), "{}", name(&mut s));
            assert_eq!(s.eval_to_string("(repl-prompt)").unwrap(), "\"scm λ > \"", "the REPL says where it is");
        }
        super::super::remote::close_all();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A ray file (the modeline's "Scheme (ray)") sends to a `ray --serve`
    /// of its checkout's: ray's verbs are there, and an include resolves
    /// beside the file, and M-x ray-restart starts it over. Needs a ray
    /// binary:
    /// `RAY_BIN=~/devel/ray/target/release/ray cargo test -p edit --lib a_ray_file_sends_to_ray -- --ignored`
    #[test]
    #[ignore]
    fn a_ray_file_sends_to_ray() {
        let ray = std::env::var("RAY_BIN").expect("RAY_BIN names a ray binary");
        app::resize(800, 600, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-ray-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("scripts/scm")).unwrap();
        std::fs::create_dir_all(dir.join("examples/parts")).unwrap();
        std::fs::write(dir.join("scripts/scm/prelude.scm"), "").unwrap();
        std::fs::write(dir.join("examples/parts/cube.scm"), "(define (cube x) (* x x x))\n").unwrap();
        let file = dir.join("examples/scene.scm");
        std::fs::write(&file, "(include \"parts/cube.scm\")\n(cube 2)\n(procedure? add-sphere)\n").unwrap();
        let file = file.to_string_lossy().replace('\\', "/");
        s.call(&format!("(set! *ray-program* \"{ray}\") (open-path \"{file}\")"));
        assert_eq!(s.eval_to_string("(%buffer-dialect 1 (%editor-active-buffer 1))").unwrap(), "\"ray\"");
        for line in 0..3 {
            s.call(&format!("(%editor-goto-line 1 {line}) (eval-defun)"));
            assert!(s.settle(30.0), "ray answers line {line}");
        }
        let t: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(t.iter().any(|l| l == "=> 8"), "the include resolved beside the file: {t:?}");
        assert!(t.iter().any(|l| l == "=> #t"), "ray's verbs are there: {t:?}");
        let name = s.eval_to_string("(%alist-ref (backend-of (%editor-active-buffer 1)) :name #f)").unwrap();
        assert!(name.starts_with("\"ray at "), "{name}");
        // Judged against RAY's image now that it runs (E8): its verbs are
        // bound, a typo of one is not, and what was sent counts.
        s.call("(%editor-set-text 1 \"(add-shpere \\\"s\\\")\\n(add-sphere \\\"s\\\")\\n(cube 2 3)\\n\")");
        let mut fb = crate::framebuffer::Framebuffer::new(800, 600);
        app::draw(&mut fb);
        let w = s.eval_to_string("(map caddr (%editor-diagnostics 1))").unwrap();
        assert!(w.contains("add-shpere is not defined"), "{w}");
        assert!(!w.contains("add-sphere is"), "ray's verb is bound: {w}");
        assert!(w.contains("cube takes 1 argument, given 2"), "what was sent counts: {w}");
        s.call("(%editor-set-text 1 \"(include \\\"parts/cube.scm\\\")\\n(cube 2)\\n\")");
        // M-x ray-restart: the next send meets a fresh ray, without cube.
        s.call("(ray-restart) (%editor-goto-line 1 1) (eval-defun)");
        assert!(s.settle(30.0), "a fresh ray answers");
        let t: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(
            t.iter().rev().take(3).any(|l| l == "error: unbound variable: cube"),
            "cube went with the old ray: {t:?}"
        );
        super::super::remote::close_all();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The file tree: rooted at the git checkout, folders first, what git
    /// ignores hidden; expand, collapse, collapse all; RET opens a file;
    /// the tree follows the buffer on screen; its menu shows its keys.
    #[test]
    fn the_file_tree_shows_the_checkout_and_opens_files() {
        let git = |dir: &std::path::Path, args: &[&str]| {
            std::process::Command::new("git").arg("-C").arg(dir).args(args).output().is_ok_and(|o| o.status.success())
        };
        let dir = std::env::temp_dir().join(format!("edit-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(dir.join("a.scm"), "(a)\n").unwrap();
        std::fs::write(dir.join("noise.log"), "x").unwrap();
        std::fs::write(dir.join("src/b.scm"), "(b)\n").unwrap();
        std::fs::write(dir.join("src/deep/c.scm"), "(c)\n").unwrap();
        std::fs::write(dir.join("target/out"), "x").unwrap();
        if !git(&dir, &["init", "-q"]) {
            eprintln!("git is not on the PATH: skipped");
            return;
        }
        let root = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
        let a = format!("{root}/a.scm");
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, std::slice::from_ref(&a)).expect("boot");
        let rows = |s: &mut Session| -> Vec<String> {
            let n: usize = s.eval_to_string("(%pane-row-count 4)").unwrap().parse().unwrap();
            (0..n)
                .map(|i| {
                    let t = s.eval_to_string(&format!("(%pane-row-text 4 {i})")).unwrap();
                    t.trim_matches('"').replace(app::is_icon_mark, "").split_whitespace().collect::<Vec<_>>().join(" ")
                })
                .collect()
        };
        assert_eq!(s.eval_to_string("*files-root*").unwrap(), format!("\"{root}\""), "rooted at the checkout");
        let r = rows(&mut s);
        let base = std::path::Path::new(&root).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(r, vec![format!("▾ {base}"), "▸ src".into(), "a.scm".into()], "folders first; target/ and *.log hidden");
        // Each row's icon by kind: a folder, braces for source.
        assert!(s.eval_to_string("(%pane-row-text 4 1)").unwrap().contains(app::FOLDER_MARK));
        assert!(s.eval_to_string("(%pane-row-text 4 2)").unwrap().contains(app::CODE_MARK));
        assert_eq!(s.eval_to_string(r#"(files-file-mark "x.PNG")"#).unwrap().trim_matches('"'), format!("{0}{0}", app::IMAGE_MARK));
        assert_eq!(s.eval_to_string(r#"(files-file-mark "notes.txt")"#).unwrap().trim_matches('"'), format!("{0}{0}", app::FILE_MARK));
        // a.scm, the opened file, is the selection.
        assert_eq!(s.eval_to_string("(files-selected)").unwrap(), format!("\"{a}\""));

        // Expand src, then src/deep; collapse all closes both.
        s.call("(files-select! 1) (files-expand)");
        assert_eq!(rows(&mut s)[1..4].to_vec(), vec!["▾ src", "▸ deep", "b.scm"]);
        s.call("(files-select! 2) (files-expand)");
        assert!(rows(&mut s).contains(&"c.scm".to_string()));
        s.call("(files-collapse-all)");
        assert_eq!(rows(&mut s).len(), 3);
        // Ignored files on demand.
        s.call("(files-toggle-ignored)");
        assert!(rows(&mut s).contains(&"noise.log".to_string()) && rows(&mut s).contains(&"▸ target".to_string()));
        s.call("(files-toggle-ignored)");

        // RET on a file opens it in the editor, keyboard there.
        s.call("(files-select! 1) (files-expand) (files-select! 3) (files-activate)");
        assert_eq!(s.eval_to_string("(buffer-path (%editor-active-buffer 1))").unwrap(), format!("\"{root}/src/b.scm\""));
        assert_eq!(app::typing(), Some(app::EDITOR));
        // The tree follows the buffer on screen: back to a.scm by name.
        s.call(&format!("(visit-buffer (buffer-for-file \"{a}\"))"));
        assert_eq!(s.eval_to_string("(files-selected)").unwrap(), format!("\"{a}\""));
        // Revealing a deep file opens the folders above it.
        s.call(&format!("(files-reveal \"{root}/src/deep/c.scm\")"));
        assert_eq!(s.eval_to_string("(files-selected)").unwrap(), format!("\"{root}/src/deep/c.scm\""));

        // The menu has the folder commands, with their keys.
        let labels = s.eval_to_string("(%item-labels (hash-table-ref *menus* :files))").unwrap();
        for want in ["expand\\t+", "collapse\\t-", "collapse all", "open\\tRET"] {
            assert!(labels.contains(want), "{want} in {labels}");
        }
        // The tree's own keys reach it when it has the focus.
        s.call("(files-focus) (files-select! 0)");
        s.key("down");
        assert_eq!(s.eval_to_string("*files-sel*").unwrap(), "1");

        // Its edge drags; its icon hides it, and the editor takes the width.
        let f = app::rect(app::FILES).unwrap();
        assert_eq!(app::divider_at((f.x + f.w) as f64, 100.0), Some(app::Divider::Files));
        app::drag_divider(app::Divider::Files, 300.0, 100.0);
        assert_eq!(app::rect(app::EDITOR).unwrap().x, 300);
        s.call("(toggle-files)");
        assert!(app::rect(app::FILES).is_none());
        assert_eq!(app::rect(app::EDITOR).unwrap().x, 0);
        s.call("(toggle-files)");
        let mut fb = crate::framebuffer::Framebuffer::new(1000, 700);
        app::draw(&mut fb);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pane's right-click menu WORKS over that pane: a click on an item
    /// runs it. Over the terminal, the file tree and the REPL the pane's own
    /// click handler took the click first, so the menu popped up and its
    /// items did nothing (Kevin, 2026-11-13).
    #[test]
    fn a_pane_menu_runs_its_items_over_the_pane() {
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        // Post a pane's menu with a quick right-click inside it, then click
        // its first row.
        let run_first_item = |s: &mut Session, id: u32, down: f64| -> bool {
            let r = app::rect(id).unwrap();
            let (x, y) = (r.x as f64 + 30.0, r.y as f64 + down);
            s.menu_press(x, y, false);
            s.menu_release(x, y);
            if s.eval_to_string("(menu-open?)").unwrap() != "#t" {
                return false;
            }
            for py in (0..700).step_by(3) {
                for px in (0..1000).step_by(10) {
                    let (px, py) = (px as f64, py as f64);
                    if s.eval_to_string(&format!("(%overlay-hit {px} {py})")).unwrap() == "0" {
                        s.click(px, py, false, false);
                        return s.eval_to_string("(menu-open?)").unwrap() == "#f";
                    }
                }
            }
            false
        };
        // The terminal's first item is "back to the editor". The panel made
        // tall, so a press near its top posts the menu DOWNWARD, over the
        // terminal itself — where the terminal's click handler took the click.
        s.call(r#"(term-open "/bin/sh")"#);
        let mid = { let e = app::rect(app::EDITOR).unwrap(); e.x as f64 + e.w as f64 / 2.0 };
        app::drag_divider(app::Divider::Panel, mid, 60.0);
        assert_eq!(app::typing(), Some(app::TRANSCRIPT));
        assert!(run_first_item(&mut s, app::TRANSCRIPT, 20.0), "the terminal's menu took the click");
        assert_eq!(app::typing(), Some(app::EDITOR), "and its item ran");
        s.call("(term-close)");
        // The REPL's first item is also "back to the editor".
        s.call("(repl)");
        assert_eq!(app::typing(), Some(app::TRANSCRIPT));
        assert!(run_first_item(&mut s, app::TRANSCRIPT, 20.0), "the REPL's menu took the click");
        assert_eq!(app::typing(), Some(app::EDITOR));
        // The file tree's first item opens the row the right-press selected:
        // a folder, which expands.
        s.call("(files-open-root \".\")");
        let before = s.eval_to_string("(%pane-row-count 4)").unwrap();
        // A right-press selects the row under it: two rows down, a folder.
        assert!(run_first_item(&mut s, app::FILES, 50.0), "the tree's menu took the click");
        assert_ne!(s.eval_to_string("(%pane-row-count 4)").unwrap(), before, "and its item ran");
    }

    /// Copy and paste at the REPL and in the terminal: a drag selects, Cmd-C
    /// copies the selection, Cmd-V pastes — into the REPL lines intact and
    /// nothing run until RET, into the terminal as the program asked. (The
    /// clipboard is a stand-in here, not the system's.)
    #[test]
    fn copy_and_paste_at_the_repl_and_in_the_terminal() {
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        s.call("(define clip #f) (set! %set-clipboard (lambda (t) (set! clip t))) (set! %clipboard (lambda () clip))");
        let typed = |s: &mut Session, line: &str| {
            for c in line.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
        };
        let draw = || {
            let mut fb = crate::framebuffer::Framebuffer::new(1000, 700);
            app::draw(&mut fb);
        };

        // The REPL: an answer in the transcript, selected with the mouse.
        s.call("(repl)");
        typed(&mut s, "(* 6 7)");
        draw();
        let rows = crate::textpane::laid_rows(app::TRANSCRIPT);
        let (y, _) = rows.iter().find(|(_, t)| t == "=> 42").expect("the answer is on screen").clone();
        let x0 = app::rect(app::TRANSCRIPT).unwrap().x as f64;
        assert_eq!(app::begin_select(x0 + 9.0, y as f64 + 2.0), Some(app::TRANSCRIPT));
        app::extend_select(app::TRANSCRIPT, x0 + 400.0, y as f64 + 2.0);
        s.key("s-c");
        assert_eq!(s.eval_to_string("clip").unwrap(), "\"=> 42\"", "Cmd-C copied the selection");

        // A multi-line paste waits as pending source; RET runs the form.
        s.call(r#"(set! clip "(+ 1\n 2)")"#);
        s.key("s-v");
        assert_eq!(s.eval_to_string("*repl-pending*").unwrap(), "\"(+ 1\\n\"", "the first line waits");
        s.key("return");
        let t: Vec<String> = crate::textpane::lines(app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(t.iter().any(|l| l == "=> 3"), "RET ran the pasted form: {t:?}");

        // The terminal: a line of output selected and copied; a command
        // pasted in and run.
        s.call(r#"(term-open "/bin/sh")"#);
        s.call(r#"(set! clip "echo pasted-$((6*7))")"#);
        s.key("s-v");
        s.key("return");
        let wait = |want: &str| {
            let t0 = std::time::Instant::now();
            while t0.elapsed() < std::time::Duration::from_secs(10) {
                if let Some(i) = crate::termpane::screen_text(app::TRANSCRIPT).iter().position(|l| l == want) {
                    return Some(i);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            None
        };
        let row = wait("pasted-42").expect("the pasted command ran");
        draw();
        let r = app::rect(app::TRANSCRIPT).unwrap();
        let row_h = (crate::textpane::font_px() * crate::editorpane::LEADING).round() as f64;
        let y = r.y as f64 + 6.0 + row as f64 * row_h + row_h / 2.0;
        assert_eq!(app::begin_select(r.x as f64 + 6.0 + 1.0, y), Some(app::TRANSCRIPT));
        app::extend_select(app::TRANSCRIPT, r.x as f64 + 6.0 + 9.0 * crate::text::mono_metrics(crate::textpane::font_px()).0 as f64 + 1.0, y);
        s.key("s-c");
        assert_eq!(s.eval_to_string("clip").unwrap(), "\"pasted-42\"", "Cmd-C copied the terminal's selection");
        s.call("(term-close)");
    }

    /// Search and replace (Kevin, 2026-11-14): C-s finds from the cursor and
    /// wraps; M-% steps through the matches — y replaces, n skips, . replaces
    /// and stops, ! does the rest at once, q quits; replace-all does them all.
    #[test]
    fn search_and_replace_step_through_or_all() {
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        let set = |s: &mut Session, text: &str| {
            s.call(&format!("(%editor-set-text 1 {}) (%editor-goto-line 1 0)", lit(text)));
        };
        let text = |_: &mut Session| crate::editorpane::text(app::EDITOR);
        let typed = |s: &mut Session, line: &str| {
            for c in line.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
        };
        s.call("(focus-scene-editor)");
        set(&mut s, "alpha beta\ngamma Beta\nbeta");

        // C-s: the first match from the cursor; then single keys — n next,
        // p previous — and the prompt names them.
        let cur = || crate::editorpane::cursor(app::EDITOR);
        let mini = || crate::textpane::lines(app::MINIBUFFER).last().map(|(_, l)| l.clone()).unwrap_or_default();
        s.key("C-s");
        typed(&mut s, "beta");
        assert_eq!(cur(), (0, 10), "first beta selected, cursor at its end");
        // (previous is [N] in vi's normal mode, [p] otherwise)
        for k in ["[n] next", "] previous", "[r] replace", "[a] replace all", "[q] done"] {
            assert!(mini().contains(k), "the prompt names {k}: {}", mini());
        }
        s.text("n");
        assert_eq!(cur(), (1, 10), "n: the next; no capital, so Beta matches");
        s.text("n");
        s.text("n");
        assert_eq!(cur(), (0, 10), "wrapped to the top");
        assert!(mini().starts_with("Wrapped"), "{}", mini());
        s.text("p");
        assert_eq!(cur(), (2, 4), "p: back round to the last");
        // q leaves the keys: n is the editor's own again.
        s.text("q");
        assert_eq!(s.eval_to_string("(if *search-mode* 1 0)").unwrap(), "0", "q ended the search's keys");
        set(&mut s, "alpha beta\ngamma Beta\nbeta");
        // Any other key ends the search and does its own thing.
        s.key("C-s");
        typed(&mut s, "");
        assert_eq!(s.eval_to_string("(if *search-mode* 1 0)").unwrap(), "1", "an empty answer is the last search");
        s.text("Z");
        assert_eq!(s.eval_to_string("(if *search-mode* 1 0)").unwrap(), "0");
        // r replaces what was searched for, one by one from the match on
        // screen; a replaces them all.
        set(&mut s, "alpha beta\ngamma Beta\nbeta");
        s.key("C-s");
        typed(&mut s, "");
        s.text("n");
        s.text("r");
        typed(&mut s, "Q");
        s.text("y");
        s.text("q");
        assert_eq!(crate::editorpane::text(app::EDITOR), "alpha beta\ngamma Q\nbeta", "r replaced from the match on screen");
        s.key("C-s");
        typed(&mut s, "");
        s.text("a");
        typed(&mut s, "W");
        assert_eq!(crate::editorpane::text(app::EDITOR), "alpha W\ngamma Q\nW", "a replaced every one");
        s.call("(%editor-undo 1)");
        assert_eq!(crate::editorpane::text(app::EDITOR), "alpha beta\ngamma Q\nbeta", "in one undo step");
        set(&mut s, "alpha beta\ngamma Beta\nbeta");

        // M-%: y, n, then . stops.
        s.call("(%editor-goto-line 1 0)");
        s.key("M-S-5");
        typed(&mut s, "beta");
        typed(&mut s, "B");
        let prompt = crate::textpane::lines(app::MINIBUFFER).last().map(|(_, l)| l.clone()).unwrap_or_default();
        assert!(prompt.contains("[y] replace") && prompt.contains("[q] quit") && prompt.contains("[?] menu"), "the prompt lists the keys: {prompt}");
        s.text("?");
        assert_eq!(s.eval_to_string("(menu-open?)").unwrap(), "#t", "? opens the answers as a menu");
        s.key("down");
        s.key("down");
        s.key("down");
        s.key("down");
        s.key("return");
        assert_eq!(s.eval_to_string("*query*").unwrap(), "#f", "the menu's Quit quits");
        s.call("(%editor-goto-line 1 0)");
        // Again: an empty answer is the last one, shown in the prompt.
        s.key("M-S-5");
        typed(&mut s, "");
        typed(&mut s, "");
        s.text("y");
        s.text("n");
        s.text(".");
        assert_eq!(text(&mut s), "alpha B\ngamma Beta\nB");
        assert_eq!(s.eval_to_string("*query*").unwrap(), "#f", "the run ended");

        // ! from the middle: this one, the rest, and round to the start.
        set(&mut s, "x x\nx x");
        s.call("(%editor-goto-line 1 1)"); // line 1, so the run wraps
        s.key("M-S-5");
        typed(&mut s, "x");
        typed(&mut s, "yy");
        s.text("n");
        s.text("a");
        assert_eq!(text(&mut s), "yy yy\nx yy", "skipped the first, ! did the rest round the end");
        s.call("(%editor-undo 1)");
        assert_eq!(text(&mut s), "x x\nx x", "! was one undo step");

        // q quits; so does Escape.
        s.key("M-S-5");
        typed(&mut s, "x");
        typed(&mut s, "z");
        s.text("q");
        assert_eq!(s.eval_to_string("*query*").unwrap(), "#f");
        s.key("M-S-5");
        typed(&mut s, "x");
        typed(&mut s, "z");
        assert!(s.escape());
        assert_eq!(s.eval_to_string("*query*").unwrap(), "#f");
        assert_eq!(text(&mut s), "x x\nx x", "quitting replaced nothing");

        // replace-all: the whole buffer, one step.
        s.call("(replace-all)");
        typed(&mut s, "x");
        typed(&mut s, "w");
        assert_eq!(text(&mut s), "w w\nw w");
    }

    /// vi's normal mode: / and ? open the same search, n and N repeat it,
    /// and :s / :%s substitute — c stepping through as M-% does.
    #[test]
    fn vi_slash_search_and_substitute() {
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        s.call("(focus-scene-editor) (%editor-set-vi 1 1)");
        let set = |s: &mut Session, text: &str| {
            s.call(&format!("(%editor-set-text 1 {}) (%editor-goto-line 1 0)", lit(text)));
        };
        let typed = |s: &mut Session, line: &str| {
            for c in line.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
        };
        let cur = || crate::editorpane::cursor(app::EDITOR);
        let text = || crate::editorpane::text(app::EDITOR);
        set(&mut s, "one two\ntwo three\ntwo");
        assert_eq!(s.eval_to_string("(%editor-vi-mode 1)").unwrap(), "\"NORMAL\"");

        s.text("/");
        assert_eq!(app::typing(), Some(app::MINIBUFFER), "/ opens the search prompt");
        typed(&mut s, "two");
        assert_eq!(cur(), (0, 7));
        s.text("n");
        assert_eq!(cur(), (1, 3), "n: the next");
        s.text("N");
        assert_eq!(cur(), (0, 7), "N: the previous");
        s.text("q");
        s.text("n");
        assert_eq!(cur(), (1, 3), "n after the search ended still repeats it, as vi's does");
        s.text("q");
        s.text("?");
        typed(&mut s, "");
        assert_eq!(cur(), (0, 7), "?: backward, an empty answer the last search");
        s.text("q");

        // :s on this line, :%s on all of them.
        let ex = |s: &mut Session, cmd: &str| s.call(&format!("(editor-ex-command {})", lit(cmd)));
        s.call("(%editor-goto-line 1 1)");
        ex(&mut s, ":s/two/2/");
        assert_eq!(text(), "one two\n2 three\ntwo", ":s is this line only");
        ex(&mut s, ":%s/two/II/g");
        assert_eq!(text(), "one II\n2 three\nII", ":%s is the whole buffer");
        ex(&mut s, ":%s#II#x#c");
        assert_eq!(s.eval_to_string("(if *query* 1 0)").unwrap(), "1", "c steps through, any delimiter");
        s.text("y");
        s.text("n");
        assert_eq!(text(), "one x\n2 three\nII");
        assert_eq!(s.eval_to_string("(if *query* 1 0)").unwrap(), "0");
    }

    /// vi's ':' opens the minibuffer; :q refuses over unsaved changes and :q!
    /// does not; :w, :wq, :N, :bn, :e!; anything else is an M-x command.
    #[test]
    fn vi_colon_commands() {
        app::resize(1000, 700, 1.0);
        let dir = std::env::temp_dir().join(format!("edit-ex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, "one\ntwo\nthree\n").unwrap();
        let f = std::fs::canonicalize(&f).unwrap().to_string_lossy().into_owned();
        let mut s = Session::boot(None, std::slice::from_ref(&f)).expect("boot");
        s.call("(define quits 0) (set! *ex-quit* (lambda () (set! quits (+ quits 1))))");
        s.call("(focus-scene-editor) (%editor-set-vi 1 1)");
        let ex = |s: &mut Session, line: &str| {
            s.text(":");
            assert_eq!(app::typing(), Some(app::MINIBUFFER), ": opens the minibuffer");
            for c in line.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
        };
        let quits = |s: &mut Session| s.eval_to_string("quits").unwrap();
        let mini = || crate::textpane::lines(app::MINIBUFFER).last().map(|(_, l)| l.clone()).unwrap_or_default();

        ex(&mut s, "3");
        assert_eq!(crate::editorpane::cursor(app::EDITOR).0, 2, ":3 goes to line 3");
        ex(&mut s, "q");
        assert_eq!(quits(&mut s), "1", ":q quits a clean buffer");

        // An edit, as typing makes one (set-text is a load: it stays clean).
        s.call("(%editor-replace 1 0 0 0 0 \"changed \")");
        assert_eq!(s.eval_to_string("(%editor-dirty? 1)").unwrap(), "1");
        ex(&mut s, "q");
        assert_eq!(quits(&mut s), "1", ":q refuses over unsaved changes");
        assert!(mini().contains(":q! quits anyway"), "{}", mini());
        ex(&mut s, "q!");
        assert_eq!(quits(&mut s), "2", ":q! quits anyway");

        ex(&mut s, "e!");
        assert_eq!(crate::editorpane::text(app::EDITOR), "one\ntwo\nthree\n", ":e! reverts to the file");
        s.call("(%editor-replace 1 0 0 0 3 \"saved\")");
        ex(&mut s, "wq");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "saved\ntwo\nthree\n", ":wq writes");
        assert_eq!(quits(&mut s), "3", "and quits");

        // :help — one markdown page: vi's keys, the search in both styles,
        // the replace, the ':' commands, then every key binding.
        ex(&mut s, "help");
        let md = s.eval_to_string("(vi-help-markdown)").unwrap();
        for want in ["## vi: normal mode", "## Search", "| search forward | `C-s` `Cmd-F` | `/` |", "## Replace", ":%s/old/new/c", ":q!"] {
            assert!(md.contains(want), "help has {want}");
        }
        assert!(s.eval_to_string("(buffer-name (%editor-active-buffer 1))").unwrap().contains("*help*"), "it opened");
        // ZZ writes and quits; ZQ quits without writing.
        s.call("(visit-buffer (buffer-by-name \"a.txt\"))");
        s.call("(%editor-replace 1 0 0 0 5 \"zz\")");
        s.text("Z");
        s.text("Z");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "zz\ntwo\nthree\n", "ZZ writes");
        assert_eq!(quits(&mut s), "4", "and quits");
        s.call("(%editor-replace 1 0 0 0 2 \"lost\")");
        s.text("Z");
        s.text("Q");
        assert_eq!(quits(&mut s), "5", "ZQ quits over the change");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "zz\ntwo\nthree\n", "without writing it");
        ex(&mut s, "define ex-ran 42");
        assert_eq!(s.eval_to_string("ex-ran").unwrap(), "42", "anything else is an M-x command");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The search is incremental: each character typed moves the match, a
    /// miss says Failing, C-s at the prompt steps on, RET keeps the match
    /// and turns on the single keys, C-g goes back where it began.
    #[test]
    fn search_follows_the_typing() {
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        s.call("(focus-scene-editor) (%editor-set-text 1 \"cat car\\ncart\") (%editor-goto-line 1 0)");
        let cur = || crate::editorpane::cursor(app::EDITOR);
        let prompt = |s: &mut Session| s.eval_to_string("(%edit :prompt)").unwrap();

        s.key("C-s");
        s.text("c");
        assert_eq!(cur(), (0, 1), "c: the first c");
        s.text("a");
        s.text("r");
        assert_eq!(cur(), (0, 7), "car: the cursor followed the typing to the first car");
        s.text("t");
        assert_eq!(cur(), (1, 4), "cart: on to the next line");
        s.text("x");
        assert!(prompt(&mut s).starts_with("\"Failing"), "a miss says so: {}", prompt(&mut s));
        s.key("backspace");
        assert!(!prompt(&mut s).starts_with("\"Failing"), "{}", prompt(&mut s));
        s.key("C-g");
        assert_eq!(cur(), (0, 0), "C-g: back where the search began");
        assert_eq!(s.eval_to_string("(if *search-mode* 1 0)").unwrap(), "0");

        // @ is a character to search for, not the file picker; TAB
        // completes nothing.
        s.key("C-s");
        s.text("@");
        s.key("tab");
        assert_eq!(s.eval_to_string("(if (%edit :comp) 1 0)").unwrap(), "0", "no completion popup at a search prompt");
        assert_eq!(s.eval_to_string("(%edit :text)").unwrap(), "\"@\"");
        s.key("C-g");

        s.key("C-s");
        s.text("ca");
        s.key("C-s");
        assert_eq!(cur(), (0, 6), "C-s at the prompt: the next ca");
        s.key("return");
        assert_eq!(cur(), (0, 6), "RET keeps the match");
        assert_eq!(s.eval_to_string("(if *search-mode* 1 0)").unwrap(), "1", "and turns on the single keys");
        s.text("n");
        assert_eq!(cur(), (1, 2), "n: the next");
        s.text("q");
    }

    /// The terminal runs the LOGIN shell, not $SHELL: under `nix develop`
    /// $SHELL is a bash without readline, and Tab completed nothing (Kevin,
    /// 2026-11-14). Here: the login shell, then Tab completing in it.
    #[test]
    fn the_terminal_runs_the_login_shell_and_tab_completes() {
        let Some(shell) = super::super::term::login_shell() else {
            eprintln!("no login shell to be found here — skipped");
            return;
        };
        assert!(std::path::Path::new(&shell).exists(), "{shell}");
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        s.call("(term-open)");
        let wait = |pred: &dyn Fn(&[String]) -> bool| {
            let t0 = std::time::Instant::now();
            while t0.elapsed() < std::time::Duration::from_secs(20) {
                if pred(&crate::termpane::screen_text(app::TRANSCRIPT)) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            false
        };
        // Ask the shell its name, once it is up.
        for c in "echo shell-is-$0".chars() {
            s.text(&c.to_string());
        }
        s.key("return");
        let base = std::path::Path::new(&shell).file_name().unwrap().to_string_lossy().into_owned();
        assert!(wait(&|t| t.iter().any(|l| l.contains("shell-is-") && l.contains(&base) && !l.contains("echo"))), "the login shell ({base}) runs, not $SHELL");
        // Tab completes a command name in it (zsh and bash both do).
        for c in "ech".chars() {
            s.text(&c.to_string());
        }
        s.key("tab");
        assert!(wait(&|t| t.iter().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| l.trim_end().ends_with("echo"))), "Tab completed ech to echo");
        s.call("(term-close)");
    }

    /// Every command menu's labels are drawn with each word capitalised
    /// (Kevin, 2026-11-14); a list of names keeps its spelling.
    #[test]
    fn menu_labels_are_drawn_in_title_case() {
        app::resize(1000, 700, 1.0);
        let mut s = Session::boot(None, &[]).expect("boot");
        let get = |s: &mut Session, f: &str| s.eval_to_string(f).unwrap();
        assert_eq!(get(&mut s, r#"(title-case "send C-c to the REPL")"#), "\"Send C-c To The REPL\"");
        assert_eq!(get(&mut s, r#"(title-case "show/hide ignored")"#), "\"Show/Hide Ignored\"");
        assert_eq!(get(&mut s, r#"(title-case "clear + send buffer")"#), "\"Clear + Send Buffer\"");
        // A named menu, as drawn: labels titled, the key column as it was.
        let drawn = get(&mut s, "(%item-labels (hash-table-ref *menus* :files) #t)");
        assert!(drawn.contains("Collapse All") && drawn.contains("Show/Hide Ignored\\ti"), "{drawn}");
        // The data the menu holds is unchanged — commands and tests name it.
        assert!(get(&mut s, "(%item-labels (hash-table-ref *menus* :files))").contains("collapse all"));
        // A list of names posted without TITLE keeps its spelling.
        assert!(get(&mut s, r#"(%item-labels (list (list "a.txt" #f)))"#).contains("a.txt"));
    }
}
