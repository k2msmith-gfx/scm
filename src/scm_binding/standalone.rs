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
    // `(%git-root dir)` — the top of the git checkout DIR is in, or #f.
    interp.define_fn("%git-root", |args| {
        let Some(dir) = val_text(args, 0) else { return Ok(Val::Bool(false)) };
        let out = std::process::Command::new("git")
            .args(["-C", &dir, "rev-parse", "--show-toplevel"])
            .stderr(std::process::Stdio::null())
            .output();
        Ok(match out {
            Ok(o) if o.status.success() => Val::Str(String::from_utf8_lossy(&o.stdout).trim().to_string()),
            _ => Val::Bool(false),
        })
    })?;
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
        let dir = super::shell_dir();
        let load = |interp: &mut scm::Interp, name: &str| -> Result<(), String> {
            let path = dir.join(format!("{name}.scm"));
            let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            interp.eval_named(&src, &path.to_string_lossy()).map(|_| ()).map_err(|e| format!("{name}: {e}"))
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
        session.call(&format!("(edit-open-panel (list {}))", names.join(" ")));
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

    /// Every form `host/prelude.scm` copies from ray's prelude is still
    /// ray's text, byte for byte — so a fix to one there turns this red
    /// until it is carried here.
    #[test]
    fn the_host_prelude_is_rays_verbatim() {
        let dir = super::super::shell_dir();
        let ours = std::fs::read_to_string(dir.join("host/prelude.scm")).expect("host prelude");
        let rays = std::fs::read_to_string(dir.join("../../scripts/scm/prelude.scm")).expect("ray's prelude");
        let mut copied = 0;
        for form in ours.split("\n\n").filter(|f| f.starts_with("(define")) {
            // slurp and spit are the modeler's, written here; not ray's.
            if form.starts_with("(define (slurp") || form.starts_with("(define (spit") {
                continue;
            }
            assert!(rays.contains(form.trim_end()), "not ray's text any more:\n{form}");
            copied += 1;
        }
        assert_eq!(copied, 11, "the eleven definitions 00-base.scm says a host supplies");
    }

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
        let mut s = Session::boot(None, &[file.clone()]).expect("boot");
        // A Scheme file sends to a separate scm by default
        // (`files_default_to_a_separate_scm`); this test is about the
        // editor, so it keeps the file in the editor's image.
        s.call("(run-in-editor-image)");
        assert_eq!(app::typing(), Some(app::EDITOR), "the editor has the keyboard");
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
        let mut s = Session::boot(None, &[a.clone()]).expect("boot");
        let rows = |s: &mut Session| -> Vec<String> {
            let n: usize = s.eval_to_string("(%pane-row-count 4)").unwrap().parse().unwrap();
            (0..n)
                .map(|i| {
                    let t = s.eval_to_string(&format!("(%pane-row-text 4 {i})")).unwrap();
                    t.trim_matches('"').replace('\u{E000}', "").split_whitespace().collect::<Vec<_>>().join(" ")
                })
                .collect()
        };
        assert_eq!(s.eval_to_string("*files-root*").unwrap(), format!("\"{root}\""), "rooted at the checkout");
        let r = rows(&mut s);
        let base = std::path::Path::new(&root).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(r, vec![format!("▾ {base}"), "▸ src".into(), "a.scm".into()], "folders first; target/ and *.log hidden");
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
}
