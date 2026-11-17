//! **edit's per-key cost, measured** (Kevin, 2026-11-16): the three
//! operations compared against Neovim — scrolling (`j` ×1000, page-down
//! ×100), typing (600 characters at line 3000), and `/define` + Enter ×200
//! — each key fed as `bin/edit.rs` feeds it (Escape through the session,
//! printable keys as text, the rest as descriptors) and the whole frame
//! drawn after it, at the window's size and scale. The copy to the window
//! is not in it, as iTerm's drawing is not in Neovim's numbers. The user's
//! init.scm loads (the font); the file tree is hidden unless BENCH_TREE is
//! set, since Neovim has none.
//!
//!     cargo run --release -p edit --example bench_ops -- FILE OUT [W H SCALE]
//!         OUT gets one line of seconds per operation, the cursor after each
//!         phase, and OUT.text the buffer at the end — check the keys did
//!         what they should before reading the times.
//!     … -- FILE - W H SCALE --geometry   rows and columns on screen
//!     … -- FILE - W H SCALE --idle       a frame with nothing changed
//!     BENCH_NODRAW=1 …                    the keys alone, no frame
//!     BENCH_NO_NUMBERS=1 …                with the line-number column off
//!
//! `edit/tools/bench/run.sh FILE OUTDIR` runs this and the Neovim side
//! (`nvim_bench.vim`, in a hidden 57×244 terminal — 1920×1080 at 2× in
//! Monaco 13 — with a timer-driven insert, since feedkeys 'x!' never
//! returns in Insert mode) and prints them side by side.

use std::time::Instant;

use edit::app;
use edit::framebuffer::Framebuffer;
use edit::scm_binding::standalone::Session;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = args.get(1).expect("FILE").clone();
    let out = args.get(2).expect("OUT").clone();
    let num = |i: usize, d: f32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (w, h, scale) = (num(3, 1920.0), num(4, 1080.0), num(5, 2.0));
    let (pw, ph) = ((w * scale) as u32, (h * scale) as u32);
    app::resize(pw, ph, scale);
    let init = std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config/edit/init.scm"));
    let mut s = Session::boot(init.as_deref(), std::slice::from_ref(&file)).expect("boot");
    let mut fb = Framebuffer::new(pw, ph);
    // Neovim has no file tree: the editor takes the window's width.
    if std::env::var_os("BENCH_TREE").is_none() {
        app::show_files(false);
    }
    app::draw(&mut fb);
    // BENCH_NO_NUMBERS: the line-number column off, to see its share.
    if std::env::var_os("BENCH_NO_NUMBERS").is_some() {
        edit::editorpane::set_line_numbers(edit::editorpane::LineNumbers::Off);
        app::draw(&mut fb);
    }
    if std::env::args().any(|a| a == "--idle") {
        // A frame with nothing changed: what the draw costs by itself.
        let mut xs: Vec<f64> = (0..200).map(|_| { let t = Instant::now(); app::draw(&mut fb); t.elapsed().as_secs_f64() * 1000.0 }).collect();
        xs.sort_by(f64::total_cmp);
        println!("idle frame median {:.2} ms p95 {:.2}", xs[100], xs[190]);
        let mut f2 = Framebuffer::new(pw, ph);
        let t = Instant::now();
        for _ in 0..200 { f2.fill_rect(edit::framebuffer::Rect { x: 0, y: 0, w: pw, h: ph }, 0x00282c34); }
        println!("just clearing {pw}x{ph}: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
        return;
    }
    if std::env::args().any(|a| a == "--geometry") {
        // vi's L: the bottom row on screen.
        s.text("L");
        app::draw(&mut fb);
        let rows = edit::editorpane::cursor(1).0 + 1;
        let r = app::rect(app::EDITOR).unwrap();
        let adv = edit::text::mono_metrics(edit::textpane::font_px() * scale).0;
        println!("rows {rows} pane {}x{} px  cols ~{}", r.w, r.h, ((r.w as f32 - 16.0 * scale) / adv) as u32);
        return;
    }

    fn press(s: &mut Session, k: &str) {
        // As bin/edit.rs: Escape asks the session first, printable keys
        // travel as text, the rest as descriptors.
        if k == "escape" {
            if !s.escape() {
                s.key("escape");
            }
        } else if k.chars().count() == 1 {
            s.text(k)
        } else {
            s.key(k)
        }
    }
    fn run(report: &mut String, s: &mut Session, fb: &mut Framebuffer, name: &str, keys: &[&str]) {
        let mut times = Vec::new();
        for k in keys {
            let t = Instant::now();
            for part in k.split('\u{1}') {
                press(s, part);
            }
            // BENCH_NODRAW: the key's own cost, the frame left out.
            if std::env::var_os("BENCH_NODRAW").is_none() {
                app::draw(fb);
            }
            times.push(t.elapsed().as_secs_f64());
        }
        report.push_str(name);
        for t in times {
            report.push_str(&format!(" {t:.6}"));
        }
        report.push('\n');
    }
    let mut report = String::new();

    // 1. scrolling: j from the top, then page-down
    press(&mut s, "g");
    press(&mut s, "g");
    app::draw(&mut fb);
    run(&mut report, &mut s, &mut fb, "scroll-j", &["j"; 1000]);
    report.push_str(&format!("after-j-cursor {:?}\n", edit::editorpane::cursor(1)));
    run(&mut report, &mut s, &mut fb, "page-down", &["pagedown"; 100]);
    report.push_str(&format!("after-page-cursor {:?}\n", edit::editorpane::cursor(1)));
    // 2. inserting: a new line at 3000, 600 characters
    for k in ["3", "0", "0", "0", "G", "o"] {
        press(&mut s, k);
    }
    app::draw(&mut fb);
    let chars: Vec<String> = (0..600)
        .map(|i| if i % 60 == 59 { "return".into() } else if i % 7 == 6 { " ".into() } else { char::from(b'a' + (i % 26) as u8).to_string() })
        .collect();
    let refs: Vec<&str> = chars.iter().map(String::as_str).collect();
    run(&mut report, &mut s, &mut fb, "insert", &refs);
    report.push_str(&format!("after-insert-cursor {:?}\n", edit::editorpane::cursor(1)));
    press(&mut s, "escape");
    press(&mut s, "g");
    press(&mut s, "g");
    app::draw(&mut fb);
    // 3. search: / define return, as one op ('\u{1}' separates its keys)
    let search = ["/", "d", "e", "f", "i", "n", "e", "return"].join("\u{1}");
    run(&mut report, &mut s, &mut fb, "search", &vec![search.as_str(); 200]);
    report.push_str(&format!("frame {pw}x{ph} scale {scale}\n"));
    // What the keys did, to check they did it.
    let id = 1;
    report.push_str(&format!("cursor {:?}\n", edit::editorpane::cursor(id)));
    std::fs::write(format!("{out}.text"), edit::editorpane::text(id)).expect("write text");
    std::fs::write(&out, report).expect("write OUT");
}
