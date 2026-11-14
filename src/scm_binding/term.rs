//! The terminal pane's prims (docs/editor-crate-plan.md, E5): one call each
//! into `termpane`, keyed by the pane that shows the terminal.

use scm::Val;

use super::{val_num, val_text, val_textv};

prim! {
    /// `(%term-send id text)` — write TEXT to pane ID's program, as if typed.
    /// 1 when there is a terminal to write to.
    "%term-send", fn term_send(id: num, text: text) {
        Out::Num(if crate::termpane::send(id as u32, text.as_bytes()) { 1.0 } else { 0.0 })
    }
}

prim! {
    /// `(%term-key id key)` — send a key by the keymap's name for it
    /// ("return", "C-c", "up") as the bytes a terminal sends. 1 when sent.
    "%term-key", fn term_key(id: num, key: text) {
        Out::Num(if crate::termpane::key(id as u32, &key) { 1.0 } else { 0.0 })
    }
}

prim! {
    /// `(%term-close id)` — stop pane ID's program and drop its terminal.
    "%term-close", fn term_close(id: num) {
        crate::termpane::close(id as u32);
        Out::Nil
    }
}

prim! {
    /// `(%term-open? id)` — 1 when pane ID shows a terminal.
    "%term-open?", fn term_is_open(id: num) {
        Out::Num(if crate::termpane::is_open(id as u32) { 1.0 } else { 0.0 })
    }
}

prim! {
    /// `(%term-exited? id)` — 1 when pane ID's program has exited.
    "%term-exited?", fn term_exited(id: num) {
        Out::Num(if crate::termpane::exited(id as u32) { 1.0 } else { 0.0 })
    }
}

prim! {
    /// `(%term-screen id)` — the screen as text, a string per row.
    "%term-screen", fn term_screen(id: num) {
        Out::rows(crate::termpane::screen_text(id as u32).into_iter().map(Out::Text))
    }
}

prim! {
    /// `(%term-scroll id rows)` — scroll the view through the history
    /// (positive = back in time).
    "%term-scroll", fn term_scroll(id: num, rows: num) {
        crate::termpane::scroll(id as u32, rows as i32);
        Out::Nil
    }
}

prim! {
    /// `(%term-selection id)` — the text selected in pane ID's terminal (a
    /// mouse drag), or #f.
    "%term-selection", fn term_selection(id: num) {
        match crate::termpane::selection_text(id as u32) {
            Some(s) => Out::Text(s),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%term-clear-selection id)`.
    "%term-clear-selection", fn term_clear_selection(id: num) {
        crate::termpane::clear_selection(id as u32);
        Out::Nil
    }
}

prim! {
    /// `(%term-paste id text)` — paste TEXT into the program: as one
    /// bracketed paste when it asked for that, else line by line. 1 when sent.
    "%term-paste", fn term_paste(id: num, text: text) {
        Out::Num(if crate::termpane::paste(id as u32, &text) { 1.0 } else { 0.0 })
    }
}

/// The user's LOGIN shell, from the account database — not `$SHELL`, which a
/// tool can replace: under `nix develop` it is the build environment's bash,
/// compiled without readline, so the terminal had no Tab completion, history
/// or line editing (Kevin, 2026-11-14). macOS keeps accounts in Directory
/// Services (`dscl`); elsewhere `getent passwd`, then /etc/passwd.
pub fn login_shell() -> Option<String> {
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).ok()?;
    let run = |prog: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(prog).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let shell = if cfg!(target_os = "macos") {
        run("/usr/bin/dscl", &[".", "-read", &format!("/Users/{user}"), "UserShell"])
            .and_then(|o| o.split_whitespace().nth(1).map(str::to_string))
    } else {
        let line = run("getent", &["passwd", &user]).or_else(|| {
            std::fs::read_to_string("/etc/passwd")
                .ok()?
                .lines()
                .find(|l| l.starts_with(&format!("{user}:")))
                .map(str::to_string)
        });
        line.and_then(|l| l.trim().rsplit(':').next().map(str::to_string))
    };
    shell.filter(|s| !s.is_empty() && std::path::Path::new(s).exists())
}

prim! {
    /// `(%login-shell)` — the user's login shell (zsh, bash, …) from the
    /// account database, or #f; the terminal's default program.
    "%login-shell", fn login_shell_prim() {
        match login_shell() {
            Some(s) => Out::Text(s),
            None => Out::Nil,
        }
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    // `(%term-open id program args cwd)` — run PROGRAM with ARGS (a list of
    // strings) in a terminal shown by pane ID, in CWD (#f: the editor's
    // own). Raises with the reason when it cannot start.
    interp.define_fn("%term-open", |args: &[Val]| {
        let id = val_num(args, 0) as u32;
        let program = val_text(args, 1).ok_or("expected a program name")?;
        let argv = val_textv(args, 2);
        let cwd = val_text(args, 3);
        crate::termpane::open(id, &program, &argv, cwd.as_deref().map(std::path::Path::new))?;
        Ok(Val::Bool(true))
    })?;
    Ok(vec![
        "%term-open",
        term_send::install(interp)?,
        term_key::install(interp)?,
        term_close::install(interp)?,
        term_is_open::install(interp)?,
        term_exited::install(interp)?,
        term_screen::install(interp)?,
        term_scroll::install(interp)?,
        term_selection::install(interp)?,
        term_clear_selection::install(interp)?,
        term_paste::install(interp)?,
        login_shell_prim::install(interp)?,
    ])
}
