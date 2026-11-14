//! **A movie in an editor buffer** — the view-only image buffer's moving
//! twin (Kevin, 2026-11-14: "click on an mp4 in the file tree … play the
//! movie in the editor window, similar to what we do with png files").
//!
//! Decoding is the `ffmpeg` PROGRAM, not its libraries: it reads every
//! container and codec there is, links nothing into the build, and is on
//! PATH wherever ray makes its movies. A reader thread takes its raw RGB
//! frames off a pipe, scaled down to at most [`MAX_W`]×[`MAX_H`], and keeps
//! them ALL — so a scrub is a lookup, not a seek. A clip longer than
//! [`MAX_BYTES`] of frames keeps its first part and says so on the modeline;
//! ray's renders are seconds long. No sound.
//!
//! The [`Player`] is the buffer's state: which frame shows, whether it runs,
//! and a press in progress (a click toggles, a drag scrubs). Time advances
//! in [`Player::tick`], which the host's event loop calls through
//! `editorpane::video_tick` and sleeps until the deadline it answers.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The largest frame kept: a movie is shown in a pane, never bigger than
/// a laptop's screen, and a 4K clip would cost 25 MB a frame.
const MAX_W: u32 = 1280;
const MAX_H: u32 = 720;
/// What the frames of one clip may cost in memory.
const MAX_BYTES: usize = 1 << 30;
/// How far the pointer moves before a press is a scrub, not a click.
const DRAG_SLOP: f64 = 4.0;

/// What the reader thread has decoded so far.
#[derive(Default)]
pub struct Clip {
    pub w: u32,
    pub h: u32,
    pub fps: f32,
    /// RGB24 frames, row-major, `w*h*3` bytes each.
    pub frames: Vec<Vec<u8>>,
    /// The container's frame count, when it says (else 0).
    pub expected: usize,
    /// The decoder has finished (or failed).
    pub done: bool,
    /// The clip ran past [`MAX_BYTES`] and only its start is kept.
    pub truncated: bool,
    pub error: Option<String>,
}

/// A movie buffer's state.
pub struct Player {
    pub clip: Arc<Mutex<Clip>>,
    pub frame: usize,
    pub playing: bool,
    pub looping: bool,
    /// When the shown frame went up, and the frame time carried past it.
    last: Instant,
    /// A press in the pane: where, the frame showing then, and whether it
    /// has moved far enough to be a scrub.
    press: Option<(f64, usize, bool)>,
    /// The pane's width at the last draw: a drag across it spans the clip.
    pub pane_w: f64,
    child: Option<Child>,
}

impl Drop for Player {
    fn drop(&mut self) {
        // A buffer killed mid-decode stops its ffmpeg.
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Is `path` a movie this can play — by its suffix, as the image buffer's
/// dispatch goes.
pub fn is_movie(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [".mp4", ".mov", ".m4v", ".webm", ".mkv", ".avi"].iter().any(|e| p.ends_with(e))
}

/// Width, height, frame rate and frame count of `path`'s first video stream.
fn probe(path: &str) -> Result<(u32, u32, f32, usize), String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=width,height,r_frame_rate,nb_frames")
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(path)
        .output()
        .map_err(|e| format!("ffprobe: {e} (a movie needs ffmpeg on PATH)"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix('='))).unwrap_or("");
    let w: u32 = field("width").parse().map_err(|_| "no video stream".to_string())?;
    let h: u32 = field("height").parse().map_err(|_| "no video stream".to_string())?;
    let fps = match field("r_frame_rate").split_once('/') {
        Some((n, d)) => n.parse::<f32>().unwrap_or(24.0) / d.parse::<f32>().unwrap_or(1.0).max(1.0),
        None => field("r_frame_rate").parse().unwrap_or(24.0),
    };
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 24.0 };
    Ok((w, h, fps, field("nb_frames").parse().unwrap_or(0)))
}

/// The kept size: within MAX_W×MAX_H, aspect kept, even (ffmpeg's scaler
/// wants even sides for most pixel formats).
fn kept_size(w: u32, h: u32) -> (u32, u32) {
    let s = (MAX_W as f32 / w as f32).min(MAX_H as f32 / h as f32).min(1.0);
    let even = |v: f32| ((v.round() as u32) / 2 * 2).max(2);
    (even(w as f32 * s), even(h as f32 * s))
}

impl Player {
    /// Start decoding `path`; playing from the first frame once it arrives.
    /// An error (no ffmpeg, not a movie) is the clip's, shown on the pane.
    pub fn open(path: &str) -> Player {
        let clip = Arc::new(Mutex::new(Clip::default()));
        let mut player = Player {
            clip: clip.clone(),
            frame: 0,
            playing: true,
            looping: true,
            last: Instant::now(),
            press: None,
            pane_w: 1.0,
            child: None,
        };
        let (w, h, fps, expected) = match probe(path) {
            Ok(p) => p,
            Err(e) => {
                let mut c = clip.lock().unwrap_or_else(|e| e.into_inner());
                c.error = Some(e);
                c.done = true;
                return player;
            }
        };
        let (kw, kh) = kept_size(w, h);
        {
            let mut c = clip.lock().unwrap_or_else(|e| e.into_inner());
            (c.w, c.h, c.fps, c.expected) = (kw, kh, fps, expected);
        }
        let child = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-an", "-f", "rawvideo", "-pix_fmt", "rgb24", "-vf"])
            .arg(format!("scale={kw}:{kh}"))
            .arg("-")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let mut c = clip.lock().unwrap_or_else(|e| e.into_inner());
                c.error = Some(format!("ffmpeg: {e} (a movie needs ffmpeg on PATH)"));
                c.done = true;
                return player;
            }
        };
        let Some(mut out) = child.stdout.take() else {
            clip.lock().unwrap_or_else(|e| e.into_inner()).done = true;
            return player;
        };
        player.child = Some(child);
        std::thread::spawn(move || {
            let size = (kw * kh * 3) as usize;
            let cap = (MAX_BYTES / size).max(1);
            loop {
                let mut buf = vec![0u8; size];
                if out.read_exact(&mut buf).is_err() {
                    break;
                }
                let mut c = clip.lock().unwrap_or_else(|e| e.into_inner());
                c.frames.push(buf);
                if c.frames.len() >= cap {
                    c.truncated = true;
                    break;
                }
                drop(c);
                crate::wake::wake();
            }
            clip.lock().unwrap_or_else(|e| e.into_inner()).done = true;
            crate::wake::wake();
        });
        player
    }

    /// (frames decoded, decoding finished, frame rate).
    fn status(&self) -> (usize, bool, f32) {
        let c = self.clip.lock().unwrap_or_else(|e| e.into_inner());
        (c.frames.len(), c.done, c.fps)
    }

    /// Advance by the time passed. Returns (whether the shown frame changed,
    /// when to come back) — `None` while nothing runs.
    pub fn tick(&mut self, now: Instant) -> (bool, Option<Instant>) {
        if !self.playing || self.press.is_some_and(|p| p.2) {
            self.last = now;
            return (false, None);
        }
        let (have, done, fps) = self.status();
        let step = Duration::from_secs_f32(1.0 / fps);
        if have == 0 {
            // Still waiting for the first frame; the decoder wakes us.
            self.last = now;
            return (false, if done { None } else { Some(now + step) });
        }
        let mut changed = false;
        while now.duration_since(self.last) >= step {
            self.last += step;
            if self.frame + 1 < have {
                self.frame += 1;
                changed = true;
            } else if !done {
                // Caught up with the decoder: wait for it, not ahead of it.
                self.last = now;
                break;
            } else if self.looping {
                self.frame = 0;
                changed = true;
            } else {
                self.playing = false;
                return (changed, None);
            }
        }
        (changed, Some(self.last + step))
    }

    pub fn toggle(&mut self) {
        let (have, done, _) = self.status();
        // Play at the end of a clip that does not loop starts it again.
        if !self.playing && done && self.frame + 1 >= have {
            self.frame = 0;
        }
        self.playing = !self.playing;
        self.last = Instant::now();
    }

    /// Show frame `f`, clamped to what has been decoded.
    pub fn seek(&mut self, f: isize) {
        let have = self.status().0;
        self.frame = f.clamp(0, have.saturating_sub(1) as isize) as usize;
        self.last = Instant::now();
    }

    pub fn step(&mut self, by: isize) {
        self.playing = false;
        self.seek(self.frame as isize + by);
    }

    /// A press at window x.
    pub fn press(&mut self, x: f64) {
        self.press = Some((x, self.frame, false));
    }

    /// The pointer, held, at window x over a pane `width` wide: past the
    /// slop it scrubs — the pane's width spans the whole clip.
    pub fn drag(&mut self, x: f64, width: f64) {
        let Some((x0, f0, moved)) = self.press else { return };
        let moved = moved || (x - x0).abs() > DRAG_SLOP;
        self.press = Some((x0, f0, moved));
        if moved {
            let total = self.status().0.max(1) as f64;
            self.seek(f0 as isize + ((x - x0) / width.max(1.0) * total).round() as isize);
        }
    }

    /// The press ends: one that never moved was a click, which toggles.
    pub fn release(&mut self) {
        if let Some((_, _, moved)) = self.press.take() {
            if !moved {
                self.toggle();
            }
            self.last = Instant::now();
        }
    }

    /// The frame to show, as (w, h, RGB24) — cloned out from under the lock
    /// only as far as the draw needs: the draw holds it for one blit.
    pub fn with_frame<R>(&self, f: impl FnOnce(u32, u32, &[u8]) -> R) -> Option<R> {
        let c = self.clip.lock().unwrap_or_else(|e| e.into_inner());
        let frame = c.frames.get(self.frame.min(c.frames.len().saturating_sub(1)))?;
        Some(f(c.w, c.h, frame))
    }

    /// The modeline's right side: play state, frame, time, and how the
    /// decode is going.
    pub fn modeline(&self) -> String {
        let c = self.clip.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = &c.error {
            return format!("cannot play: {e}");
        }
        let n = c.frames.len();
        let total = if c.done { n } else { c.expected.max(n) };
        let t = |f: usize| {
            let s = f as f32 / c.fps;
            format!("{}:{:04.1}", (s / 60.0) as u32, s % 60.0)
        };
        let state = if self.playing { "▶" } else { "❚❚" };
        let mut s = format!(
            "{state} {}/{} · {} / {} · {:.0} fps",
            (self.frame + 1).min(total.max(1)),
            total,
            t(self.frame),
            t(total),
            c.fps
        );
        if !c.done {
            s.push_str(" · decoding");
        } else if c.truncated {
            s.push_str(" · first part only");
        }
        if !self.looping {
            s.push_str(" · once");
        }
        s
    }

    /// Where the frame stands in the clip, 0..1, for the progress bar.
    pub fn progress(&self) -> f32 {
        let c = self.clip.lock().unwrap_or_else(|e| e.into_inner());
        let total = if c.done { c.frames.len() } else { c.expected.max(c.frames.len()) };
        if total <= 1 { 0.0 } else { self.frame as f32 / (total - 1) as f32 }
    }

    /// Decoded so far / expected, for the bar's buffered part.
    pub fn buffered(&self) -> f32 {
        let c = self.clip.lock().unwrap_or_else(|e| e.into_inner());
        if c.done { 1.0 } else if c.expected == 0 { 0.0 } else { (c.frames.len() as f32 / c.expected as f32).min(1.0) }
    }

    pub fn error(&self) -> Option<String> {
        self.clip.lock().unwrap_or_else(|e| e.into_inner()).error.clone()
    }

    /// Block until the decoder finishes (tests).
    #[doc(hidden)]
    pub fn wait_decoded(&self, limit: Duration) -> bool {
        let t0 = Instant::now();
        while t0.elapsed() < limit {
            if self.status().1 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Decoded frames (tests).
    #[doc(hidden)]
    pub fn decoded(&self) -> usize {
        self.status().0
    }
}
