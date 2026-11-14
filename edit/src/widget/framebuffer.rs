//! The kernel-owned CPU framebuffer (docs/modeler-plan.md, "Window &
//! framebuffer"). Everything that ever appears on screen — viewport raster,
//! script-drawn overlays, blitted render frames — is composited into this one
//! pixel buffer, which the main loop hands to softbuffer to present. Pixels
//! are `0x00RRGGBB` u32s, softbuffer's native layout, so presenting is a
//! straight copy with no per-pixel conversion.

/// An axis-aligned pixel rectangle within the framebuffer — the pane
/// geometry unit (docs/modeler-plan.md M4). Panes tile the one buffer;
/// every draw entry point below is scoped to a rect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x as f64
            && py >= self.y as f64
            && px < (self.x + self.w) as f64
            && py < (self.y + self.h) as f64
    }

    pub fn aspect(&self) -> f32 {
        self.w as f32 / self.h.max(1) as f32
    }

    /// The intersection with a `w`×`h` buffer — draw loops iterate this so
    /// a stale layout can never index out of bounds.
    pub fn clamped(&self, w: u32, h: u32) -> Rect {
        let x = self.x.min(w);
        let y = self.y.min(h);
        Rect {
            x,
            y,
            w: (self.x + self.w).min(w) - x,
            h: (self.y + self.h).min(h) - y,
        }
    }
}

pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
        }
    }

    /// Drop-and-reallocate on window resize; contents are stale either way
    /// because the next redraw repaints every pixel.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.pixels = vec![0; (width * height) as usize];
    }

    /// Dump the composited buffer straight to a PNG — the exact pixels
    /// softbuffer would present, no tone-mapping (these are already display
    /// values, unlike ray's HDR `Film`). Powers `--screenshot`: headless UI
    /// capture for review. `0x00RRGGBB` u32 → RGB8, row-major.
    pub fn save_png<P: AsRef<std::path::Path>>(&self, path: P) -> image::ImageResult<()> {
        let mut img = image::RgbImage::new(self.width.max(1), self.height.max(1));
        for (i, px) in self.pixels.iter().enumerate() {
            let (x, y) = (i as u32 % self.width, i as u32 / self.width);
            let rgb = image::Rgb([(px >> 16) as u8, (px >> 8) as u8, *px as u8]);
            img.put_pixel(x, y, rgb);
        }
        img.save(path)
    }

    /// Save one RECT as a PNG — a single pane rather than the whole window.
    ///
    /// What a hybrid render writes: its picture is composited into the pane, so
    /// the pane IS the image. [`save_png`](Self::save_png) would bring the
    /// docks, the status strip and every other viewport along with it.
    pub fn save_png_rect<P: AsRef<std::path::Path>>(
        &self,
        rect: Rect,
        path: P,
    ) -> image::ImageResult<()> {
        let r = rect.clamped(self.width, self.height);
        let mut img = image::RgbImage::new(r.w.max(1), r.h.max(1));
        for y in 0..r.h {
            let row = (r.y + y) as usize * self.width as usize + r.x as usize;
            for x in 0..r.w {
                let px = self.pixels[row + x as usize];
                img.put_pixel(x, y, image::Rgb([(px >> 16) as u8, (px >> 8) as u8, px as u8]));
            }
        }
        img.save(path)
    }

    /// Flood a rect with one color (pane backgrounds, dock placeholders).
    pub fn fill_rect(&mut self, rect: Rect, ink: u32) {
        let r = rect.clamped(self.width, self.height);
        for y in r.y..r.y + r.h {
            let row = (y * self.width + r.x) as usize;
            self.pixels[row..row + r.w as usize].fill(ink);
        }
    }

    /// A 1px border just inside a rect (pane edges; accent = focus).
    pub fn stroke_rect(&mut self, rect: Rect, ink: u32) {
        let r = rect.clamped(self.width, self.height);
        if r.w == 0 || r.h == 0 {
            return;
        }
        for x in r.x..r.x + r.w {
            self.pixels[(r.y * self.width + x) as usize] = ink;
            self.pixels[((r.y + r.h - 1) * self.width + x) as usize] = ink;
        }
        for y in r.y..r.y + r.h {
            self.pixels[(y * self.width + r.x) as usize] = ink;
            self.pixels[(y * self.width + r.x + r.w - 1) as usize] = ink;
        }
    }

    /// Anti-aliased rounded rectangle: `fill` inside, a ~1px `border`
    /// ring, corners alpha-blended over whatever is behind (a signed
    /// distance to the rounded boundary drives coverage). The popup
    /// overlay's box; later menus/gizmos share it.
    pub fn round_rect(&mut self, rect: Rect, radius: f32, fill: u32, border: u32) {
        let r = rect.clamped(self.width, self.height);
        if r.w == 0 || r.h == 0 {
            return;
        }
        let radius = radius.clamp(0.0, (r.w.min(r.h) as f32) / 2.0);
        let (hw, hh) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
        let (cx, cy) = (r.x as f32 + hw, r.y as f32 + hh);
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let dx = ((x as f32 + 0.5 - cx).abs() - (hw - radius)).max(0.0);
                let dy = ((y as f32 + 0.5 - cy).abs() - (hh - radius)).max(0.0);
                let d = (dx * dx + dy * dy).sqrt() - radius;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let ink = if d > -1.5 { border } else { fill };
                let idx = (y * self.width + x) as usize;
                self.pixels[idx] = crate::text::blend(self.pixels[idx], ink, cov);
            }
        }
    }

    /// The M1 test pattern, drawn rect-local (an empty scene shows it per 3D
    /// pane since M4): a red-over-x, green-over-y gradient with a white
    /// center crosshair. The gradient proves per-pixel addressing and the
    /// buffer's orientation (red grows rightward, green downward); the
    /// crosshair proves width/height indexing isn't transposed. `tint`
    /// multiplies every channel — M2's visible proof that a REPL eval reached
    /// the paint path ([1,1,1] = the plain pattern).
    pub fn fill_test_pattern(&mut self, rect: Rect, tint: [f32; 3]) {
        let r = rect.clamped(self.width, self.height);
        if r.w == 0 || r.h == 0 {
            return;
        }
        let ch = |v: u32, t: f32| ((v as f32 * t.clamp(0.0, 1.0)) as u32).min(255);
        for y in 0..r.h {
            for x in 0..r.w {
                let red = ch(x * 255 / r.w, tint[0]);
                let g = ch(y * 255 / r.h, tint[1]);
                let b = ch(0x40, tint[2]);
                self.pixels[((r.y + y) * self.width + r.x + x) as usize] =
                    (red << 16) | (g << 8) | b;
            }
        }
        let cross = (ch(255, tint[0]) << 16) | (ch(255, tint[1]) << 8) | ch(255, tint[2]);
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        for x in r.x..r.x + r.w {
            self.pixels[(cy * self.width + x) as usize] = cross;
        }
        for y in r.y..r.y + r.h {
            self.pixels[(y * self.width + cx) as usize] = cross;
        }
    }
}

/// A one-pixel 2D line, DDA-stepped, clipped to `rect`: the 3D panes' HUD
/// strokes and the marking menu's drag stroke.
pub fn draw_line_2d(x0: f32, y0: f32, x1: f32, y1: f32, ink: u32, fb: &mut Framebuffer, rect: Rect) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil() as usize;
    let w = fb.width as usize;
    let (lx, ly) = (rect.x as f32, rect.y as f32);
    let (hx, hy) = ((rect.x + rect.w) as f32, (rect.y + rect.h) as f32);
    let inv = 1.0 / steps.max(1) as f32;
    for s in 0..=steps {
        let t = s as f32 * inv;
        let x = x0 + (x1 - x0) * t;
        let y = y0 + (y1 - y0) * t;
        if x < lx || y < ly || x >= hx || y >= hy {
            continue;
        }
        fb.pixels[y as usize * w + x as usize] = ink;
    }
}

/// An anti-aliased rounded rect blended at `alpha` ([`Framebuffer::round_rect`]
/// fills opaquely; the glass needs the scene through it), with a 1px ring
/// of white at `border_alpha` — the ~10% hairline. Clipped to `clip`.
///
/// Lives here, not in `toolbox`, because two unrelated things draw glass: the
/// toolbox panel and the text pane's popups. `toolbox` is gated on the
/// `scripting` feature (it dispatches script forms and has no reason to exist
/// without a VM; the gate was spelled `janet` when this was written),
/// while the text pane is not — so while this function sat there, a pure
/// `cargo build` failed on three `crate::toolbox::glass_rect` calls that the
/// feature-gated module never compiled. A painter has no opinion about
/// scripting; it belongs beside the framebuffer it paints.
pub fn glass_rect(fb: &mut Framebuffer, r: Rect, radius: f32, fill: u32, alpha: f32, border_alpha: f32, clip: Rect) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let radius = radius.clamp(0.0, (r.w.min(r.h) as f32) / 2.0);
    let (hw, hh) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
    let (cx, cy) = (r.x as f32 + hw, r.y as f32 + hh);
    let (x0, y0) = (r.x.max(clip.x), r.y.max(clip.y));
    let (x1, y1) = ((r.x + r.w).min(clip.x + clip.w).min(fb.width), (r.y + r.h).min(clip.y + clip.h).min(fb.height));
    let w = fb.width as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = ((x as f32 + 0.5 - cx).abs() - (hw - radius)).max(0.0);
            let dy = ((y as f32 + 0.5 - cy).abs() - (hh - radius)).max(0.0);
            let d = (dx * dx + dy * dy).sqrt() - radius;
            let cov = (0.5 - d).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let idx = y as usize * w + x as usize;
            let (ink, a) = if d > -1.5 && border_alpha > 0.0 {
                (0x00ff_ffff, border_alpha)
            } else {
                (fill, alpha)
            };
            fb.pixels[idx] = crate::text::blend(fb.pixels[idx], ink, cov * a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_reallocates_to_the_new_pixel_count() {
        let mut fb = Framebuffer::new(4, 3);
        assert_eq!(fb.pixels.len(), 12);
        fb.resize(7, 5);
        assert_eq!((fb.width, fb.height, fb.pixels.len()), (7, 5, 35));
    }

    fn full(fb: &Framebuffer) -> Rect {
        Rect { x: 0, y: 0, w: fb.width, h: fb.height }
    }

    #[test]
    fn test_pattern_orientation_and_crosshair() {
        let mut fb = Framebuffer::new(100, 50);
        fb.fill_test_pattern(full(&fb), [1.0, 1.0, 1.0]);
        let px = |x: u32, y: u32| fb.pixels[(y * fb.width + x) as usize];
        // Red grows rightward along the top row, green downward on the left
        // column (avoiding the center crosshair rows/cols).
        assert!(px(90, 0) >> 16 > px(10, 0) >> 16, "red should grow with x");
        assert!(
            (px(0, 40) >> 8) & 0xff > (px(0, 10) >> 8) & 0xff,
            "green should grow with y"
        );
        // The crosshair: full-white center row and column.
        assert_eq!(px(3, 25), 0x00ff_ffff);
        assert_eq!(px(50, 44), 0x00ff_ffff);
    }

    #[test]
    fn tint_scales_channels() {
        let mut fb = Framebuffer::new(100, 50);
        fb.fill_test_pattern(full(&fb), [1.0, 0.0, 0.0]);
        let px = |x: u32, y: u32| fb.pixels[(y * fb.width + x) as usize];
        // Green and blue killed everywhere; red survives; crosshair pure red.
        assert_eq!(px(90, 10) & 0x0000_ffff, 0, "green/blue should be zeroed");
        assert!(px(90, 10) >> 16 > 0, "red should survive a red tint");
        assert_eq!(px(50, 25), 0x00ff_0000, "crosshair should tint too");
    }

    #[test]
    fn rect_draws_stay_inside_their_rect() {
        let mut fb = Framebuffer::new(20, 10);
        let r = Rect { x: 5, y: 2, w: 8, h: 4 };
        fb.fill_rect(r, 0x00aa_bbcc);
        fb.stroke_rect(r, 0x0011_2233);
        fn px(fb: &Framebuffer, x: u32, y: u32) -> u32 {
            fb.pixels[(y * fb.width + x) as usize]
        }
        assert_eq!(px(&fb, 4, 3), 0, "left of the rect must be untouched");
        assert_eq!(px(&fb, 13, 3), 0, "right of the rect must be untouched");
        assert_eq!(px(&fb, 5, 2), 0x0011_2233, "corner carries the border");
        assert_eq!(px(&fb, 12, 5), 0x0011_2233, "opposite corner too");
        assert_eq!(px(&fb, 7, 3), 0x00aa_bbcc, "interior carries the fill");
        // A pattern scoped to the rect leaves the outside black as well.
        fb.fill_test_pattern(r, [1.0, 1.0, 1.0]);
        assert_eq!(px(&fb, 4, 3), 0);
        assert_eq!(px(&fb, 9, 4), 0x00ff_ffff, "crosshair sits at the rect center");
        // A rect hanging off the buffer clamps instead of panicking.
        fb.fill_rect(Rect { x: 15, y: 5, w: 50, h: 50 }, 0x1);
        assert_eq!(px(&fb, 19, 9), 0x1);
    }
}
