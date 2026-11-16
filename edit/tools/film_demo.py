#!/usr/bin/env python3
"""A demo film of the standalone editor, driven by itself (Kevin,
2026-11-15: "make a demo movie of the editor").

    python3 edit/tools/film_demo.py OUT.mp4 IMAGE.png [--fps 12]

`edit` runs in film mode (`EDIT_FILM_PORT`, edit/src/film.rs): hidden, at
a fixed frame size, taking forms on a socket. Input is injected through
the same code a real event takes, at a human pace, and each frame is the
editor's own framebuffer, with the caption in a band of its own beneath
the editor so it never covers what is shown. The film's clock is the
demo's INTENDED time — the sum of its pauses — so it plays at the pace it
was written for, not at the pace a machine carrying a camera managed (the
reasoning is ray's modeler/tools/demo_loft.py, whose films this follows).

The film opens a small project it writes afresh each run, so every take
starts from the same files: shapes.scm, scene.scm (a ray scene), crate/ (a
Rust crate), README.md, and IMAGE.png as images/fallingwater.png.
"""

import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

EDIT = Path(__file__).resolve().parents[2] / "target" / "release" / "edit"
PORT = 4131


PROJECT = {
    "shapes.scm": """;;; shapes.scm \u2014 a little Scheme program to show the editor on.

(define (square x)
  "X times itself."
  (* x x))

(define (cube x)
  "X three times over."
  (* x (square x)))

(define (area w h)
  "A rectangle's area: W wide, H high."
  (* w h))

(define (volumes sizes)
  "Each of SIZES, cubed."
  (map cube sizes))

(define sizes '(1 2 3 4))

(define (scale-all k xs)
  "Every X in XS, K times bigger."
  (map (lambda (x) (* k x)) xs))
""",
    "scene.scm": """;;; scene.scm \u2014 a ray scene: its verbs live in ray's image, so it runs in ray.

(clear-scene)
(set-camera :at [0 2 7] :look-at [0 1 0])
(add-sphere :at [0 1 0] :radius 1 :name :ball)
""",
    "crate/Cargo.toml": """[package]
name = "shapes"
version = "0.1.0"
edition = "2021"
""",
    "crate/src/main.rs": """/// X three times over.
fn cube(x: u32) -> u32 {
    x * x * x
}

fn main() {
    let sizes = vec![1, 2, 3, 4];
    for s in &sizes {
        println!("{s} cubed is {}", cube(*s));
    }
}
""",
    "README.md": """# shapes

A small project to show **edit** on: a Scheme program (`shapes.scm`), a ray
scene (`scene.scm`), a Rust crate (`crate/`) and a picture (`images/`).

- `C-M-x` sends a definition to the REPL
- `TAB` completes, `M-.` jumps, `M-?` lists the uses
""",
}


def make_project(image):
    """The project, written afresh into a new temporary directory."""
    root = Path(tempfile.mkdtemp(prefix="edit-film-")) / "shapes"
    for rel, text in PROJECT.items():
        f = root / rel
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(text)
    (root / "images").mkdir(exist_ok=True)
    (root / "images" / "fallingwater.png").write_bytes(Path(image).read_bytes())
    return root


def sq(s):
    """A Scheme string literal."""
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


class Edit:
    def __init__(self, project, size="1600x1000", scale="1.25", ray=None):
        env = dict(os.environ, EDIT_FILM_PORT=str(PORT), EDIT_FILM_SIZE=size, EDIT_FILM_SCALE=scale)
        self.proc = subprocess.Popen([str(EDIT), "shapes.scm"], cwd=project, env=env,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(200):
            try:
                self.sock = socket.create_connection(("127.0.0.1", PORT), timeout=60)
                break
            except OSError:
                time.sleep(0.05)
        self._read()
        if ray:
            self.eval(f"(set! *ray-program* {sq(ray)})")

    def _read(self):
        data = ""
        while "edit>" not in data:
            chunk = self.sock.recv(1 << 16).decode(errors="replace")
            if not chunk:
                break
            data += chunk
        return data.replace("edit>", "").strip()

    def eval(self, form):
        self.sock.sendall((form + "\n\n").encode())
        reply = self._read()
        if reply.startswith("error:"):
            print(f"  [eval error] {form[:70]} -> {reply[:100]}", file=sys.stderr)
        return reply

    def value(self, form):
        r = self.eval(form)
        return r[3:] if r.startswith("=> ") else r

    def close(self):
        try:
            self.sock.close()
        finally:
            self.proc.terminate()


class Recorder:
    """Frames into ffmpeg, one per 1/fps of the demo's intended clock."""

    def __init__(self, ed, out, fps):
        self.ed, self.fps, self.t, self.written, self.last = ed, fps, 0.0, 0, None
        self.dir = Path(tempfile.mkdtemp(prefix="edit-film-"))
        self.ff = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", str(fps), "-i", "-",
             "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-tune", "animation",
             "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", str(out)],
            stdin=subprocess.PIPE)

    def grab(self):
        due = int(self.t * self.fps)
        if self.written >= due:
            return
        while self.written < due - 1 and self.last:
            self.ff.stdin.write(self.last)
            self.written += 1
        f = self.dir / "f.png"
        self.ed.eval(f"(%screenshot-raw {sq(str(f))})")
        data = f.read_bytes() if f.exists() else self.last
        self.last = data
        if data:
            self.ff.stdin.write(data)
            self.written += 1

    def finish(self):
        due = int(self.t * self.fps)
        while self.written < due and self.last:
            self.ff.stdin.write(self.last)
            self.written += 1
        self.ff.stdin.close()
        self.ff.wait()


class Hand:
    """The keyboard and the pointer, at a human pace."""

    def __init__(self, ed, rec):
        self.ed, self.rec, self.at = ed, rec, (800.0, 500.0)

    def nap(self, seconds):
        # Real time passes too: the pie's hold timer, rust-analyzer and the
        # separate scm all run on their own clocks.
        step = 1.0 / self.rec.fps
        left = seconds
        while left > 1e-9:
            d = min(step, left)
            time.sleep(d)
            self.rec.t += d
            left -= d
            self.rec.grab()

    def say(self, text, hold=0.0):
        self.ed.eval(f"(caption {sq(text)})")
        self.nap(hold)

    def key(self, name, after=0.35):
        self.ed.eval(f"(%inject-key {sq(name)})")
        self.nap(after)

    def keys(self, *names, after=0.3):
        for n in names:
            self.key(n, after)

    def type(self, text, cps=14):
        for ch in text:
            self.ed.eval("(%inject-key " + sq({"\n": "return", " ": " "}.get(ch, ch)) + ")")
            self.nap(1.0 / cps)

    def move(self, x, y, seconds=0.45):
        x0, y0 = self.at
        n = max(2, int(seconds * self.rec.fps))
        for i in range(1, n + 1):
            e = i / n
            e = e * e * (3 - 2 * e)
            self.ed.eval(f"(%inject-mouse \"move\" {x0 + (x - x0) * e:.1f} {y0 + (y - y0) * e:.1f})")
            self.nap(seconds / n)
        self.at = (x, y)

    def click(self, button=0, after=0.5):
        self.ed.eval(f"(%inject-mouse \"press\" {button})")
        self.nap(0.08)
        self.ed.eval(f"(%inject-mouse \"release\" {button})")
        self.nap(after)

    def wheel(self, notches, after=0.25):
        for _ in range(abs(notches)):
            self.ed.eval(f"(%inject-mouse \"wheel\" {-1 if notches > 0 else 1})")
            self.nap(after)

    def icon(self, which):
        x, y = (float(v) for v in self.ed.value(f"(%film-icon {sq(which)})").strip("()").split())
        self.move(x, y)
        self.click()


def rect(ed, pane):
    return [int(float(v)) for v in ed.value(f"(%pane-rect {pane})").strip("()").split()]


def tree_to(h, ed, name):
    """Walk the file tree's selection to the row named NAME and open it —
    down/up as a person would, the count asked of the tree. The tree is
    clicked first, below its rows: opening a file gives the keyboard to the
    editor, so the arrows would move the editor's cursor instead."""
    x, y, w, hh = rect(ed, 4)
    if int(ed.value("(or (%typing-pane) (%focused-pane) 0)")) != 4:
        h.move(x + w * 0.5, y + hh - 30, 0.35)
        h.click(after=0.25)
    there = int(ed.value(
        "(let loop ((rs (vector->list *files-rows*)) (i 0))"
        "  (cond ((null? rs) -1)"
        f"        ((equal? (files-base (car (car rs))) {sq(name)}) i)"
        "        (else (loop (cdr rs) (+ i 1)))))"))
    if there < 0:
        raise SystemExit(f"no row {name} in the tree")
    here = int(ed.value("*files-sel*"))
    for _ in range(abs(there - here)):
        h.key("down" if there > here else "up", 0.18)
    h.key("return", 0.6)


def film(out, image, fps):
    project = make_project(image)
    ray = Path.home() / "devel/ray/target/release/ray"
    ed = Edit(project, ray=str(ray) if ray.exists() else None)
    rec = Recorder(ed, out, fps)
    h = Hand(ed, rec)
    E, FILES = 1, 4

    # ── 1. The panes ─────────────────────────────────────────────────────
    h.say("edit: a Scheme editor in a window of its own, Emacs keys and vi", 3.5)
    h.say("The file tree, the REPL and a terminal are each one click on the strip, bottom right", 1.0)
    h.icon("files"); h.nap(0.8)
    h.icon("files"); h.nap(0.8)
    h.icon("repl"); h.nap(1.6)
    h.icon("terminal"); h.nap(1.5)
    h.type("ls\n", cps=8); h.nap(1.6)
    h.icon("terminal"); h.nap(0.8)
    h.say("Pictures, markdown and movies open in the editor too, from the tree", 0.6)
    tree_to(h, ed, "images"); h.key("right", 0.5)
    tree_to(h, ed, "fallingwater.png"); h.nap(2.6)
    tree_to(h, ed, "README.md"); h.nap(0.8)
    h.keys("C-c", "C-v", after=2.4)

    # ── 2. Languages ─────────────────────────────────────────────────────
    h.say("Each file is read in its own language, and the modeline says which", 0.6)
    tree_to(h, ed, "shapes.scm"); h.nap(1.6)
    tree_to(h, ed, "scene.scm"); h.nap(0.6)
    h.say("A ray scene reads as Scheme (ray): C-c C-b sends it to ray, where its verbs are", 0.8)
    h.keys("C-c", "C-z", after=0.6)
    h.keys("C-x", "o", after=0.4)
    h.keys("C-c", "C-b", after=3.0)
    h.say("Rust: rust-analyzer starts by itself when it is on the PATH", 0.4)
    tree_to(h, ed, "crate"); h.key("right", 0.4)
    tree_to(h, ed, "src"); h.key("right", 0.4)
    tree_to(h, ed, "main.rs"); h.nap(3.5)
    # Type a mistake into main, save, and the compiler's error comes back.
    h.move(*[v + 40 for v in rect(ed, E)[:2]]); h.click(after=0.3)
    h.keys("j", "j", "j", "j", "j", "j", after=0.08)
    h.key("o", 0.3)
    h.type('let n: u32 = "four";', cps=16)
    h.key("escape", 0.2)
    h.say("Saved, cargo check's errors come back underlined; M-g n goes to one and says why", 0.3)
    h.keys("C-x", "C-s", after=0.2)
    for _ in range(40):
        h.nap(0.25)
        if "mismatched" in ed.eval("(%editor-diagnostics 1)"):
            break
    h.nap(0.8)
    h.keys("M-g", "n", after=2.6)
    h.say("TAB completes from rust-analyzer; inside a call, the signature is echoed", 0.3)
    h.key("o", 0.2)
    h.type("let c = cu", cps=12); h.key("tab", 1.6); h.key("return", 0.4)
    h.type("(2", cps=10); h.nap(2.2)
    h.key("escape", 0.2)
    h.keys("u", "u", "u", "u", after=0.15)
    h.keys("C-x", "C-s", after=0.6)

    # ── 3. Menus ─────────────────────────────────────────────────────────
    tree_to(h, ed, "shapes.scm"); h.nap(0.6)
    ex, ey, ew, eh = rect(ed, E)
    h.say("Right-click for the pane's menu", 0.2)
    h.move(ex + ew * 0.55, ey + eh * 0.45)
    h.click(button=2, after=2.6)
    h.key("escape", 0.6)
    h.say("Hold the right button for the marking menu: flick toward a command", 0.2)
    ed.eval('(%inject-mouse "press" 2)'); h.nap(0.9)
    cx, cy = h.at
    for dx, dy in [(70, -40), (80, 40), (-70, 50), (-70, -40)]:
        h.move(cx + dx, cy + dy, 0.35); h.nap(0.45)
    h.move(cx, cy, 0.3)
    ed.eval('(%inject-mouse "release" 2)'); h.nap(0.6)
    h.say("M-x runs any command by name, with completion; C-x C-b lists the buffers", 0.2)
    h.key("M-x", 0.4); h.type("find-ref", cps=10); h.nap(1.6); h.key("C-g", 0.5)
    h.keys("C-x", "C-b", after=2.0); h.key("escape", 0.6)

    # ── 4. Completion and editing ────────────────────────────────────────
    h.say("Back in Scheme: TAB completes a name as you type", 0.2)
    h.key("G", 0.3); h.key("o", 0.2); h.key("return", 0.2)
    h.type("(define (report)", cps=14); h.key("return", 0.3)
    h.type("(for-e", cps=12); h.key("tab", 1.4); h.key("return", 0.3)
    h.type(" (lambda (n) (dis", cps=12); h.key("tab", 1.2); h.key("return", 0.3)
    h.type(" (cube n)))", cps=10); h.nap(1.6)
    h.type(" sizes))", cps=12); h.nap(0.6)
    h.say("Mistakes are underlined before anything runs: a name nothing defines, a wrong count", 0.3)
    h.key("return", 0.2)
    h.type("(dispaly (cube 1 2))", cps=13); h.nap(1.6)
    h.key("escape", 0.3)
    h.keys("M-g", "n", after=2.2)
    h.keys("M-g", "n", after=2.2)
    h.keys("d", "d", after=0.4)
    h.say("M-. jumps to a definition and M-, comes back; M-? lists every use of a name", 0.2)
    h.key("/", 0.2); h.type("map cube", cps=12); h.key("return", 0.3)
    h.key("b", 0.4)
    h.key("M-.", 1.8)
    h.key("M-,", 1.2)
    h.key("M-S-/", 2.8); h.key("C-g", 0.5)
    h.say("M-g i lists what this file defines; typing narrows it", 0.2)
    h.keys("M-g", "i", after=1.4); h.type("vol", cps=8); h.nap(1.0); h.key("return", 1.0)

    # ── 5. SLIME ─────────────────────────────────────────────────────────
    h.say("SLIME's loop: C-M-x sends a definition to a running scm, and the answer lands in the REPL", 0.2)
    h.key("C-c", 0.1); h.key("C-z", 0.8)
    h.key("C-x", 0.1); h.key("o", 0.5)
    h.key("C-M-x", 1.4)
    h.key("/", 0.2); h.type("define sizes", cps=12); h.key("return", 0.3)
    h.key("C-M-x", 1.0)
    h.key("/", 0.2); h.type("define (cube", cps=12); h.key("return", 0.3)
    h.key("C-M-x", 1.0)
    h.key("/", 0.2); h.type("define (square", cps=12); h.key("return", 0.3)
    h.key("C-M-x", 1.0)
    h.key("/", 0.2); h.type("define (area", cps=12); h.key("return", 0.3)
    h.key("C-M-x", 1.4)
    h.say("The REPL is the same scm: what was sent is there, and TAB completes from it", 0.2)
    h.key("C-c", 0.1); h.key("C-z", 0.5)
    h.type("(volumes sizes)", cps=12); h.key("return", 1.6)
    h.type("(squ", cps=10); h.key("tab", 1.6); h.key("return", 0.3)
    h.type(" 12)", cps=10); h.key("return", 1.6)
    h.say("An error says where it was raised; the image lives on", 0.2)
    h.type("(cube 'two)", cps=12); h.key("return", 2.6)
    h.key("C-x", 0.1); h.key("o", 0.5)
    h.say("K (or C-c C-d d) describes the name under the cursor: its signature and docstring", 0.2)
    h.key("/", 0.2); h.type("(area", cps=12); h.key("return", 0.3); h.key("w", 0.3)
    h.key("K", 2.8)

    # ── 6. Close ─────────────────────────────────────────────────────────
    h.say(":help lists every key and command", 0.2)
    h.key(":", 0.3); h.type("help", cps=10); h.key("return", 1.4)
    ex, ey, ew, eh = rect(ed, E)
    h.move(ex + ew * 0.5, ey + eh * 0.5, 0.3)
    h.wheel(8, 0.2); h.nap(1.0)
    h.say("edit: in the scm repository, cargo build --release -p edit", 3.0)
    rec.finish()
    ed.close()
    print(f"{out}: {rec.t:.1f} s of film, {rec.written} frames")


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    fps = int(sys.argv[sys.argv.index("--fps") + 1]) if "--fps" in sys.argv else 12
    film(Path(args[0]).resolve(), Path(args[1]).resolve(), fps)
