//! Minimal kernel text (docs/modeler-plan.md "Draw primitives"): the first
//! slice of `draw-text`, enough to title the M4 dock placeholders — plus,
//! since M4.5, the monospace face and grid metrics the text panes
//! (`textpane.rs`) lay out with. Labels use vendored IBM Plex Sans;
//! text panes use vendored JetBrains Mono (OFL, assets/JetBrainsMono-OFL.txt)
//! — the terminal aesthetic is the design brief, and monospace makes
//! wrapping trivial character-grid math (the plan's ropey-vs-cosmic-text
//! decision, monospace branch).
//!
//! Rasterization is `swash` (the CoreText-quality gap ab_glyph left): it
//! hints outlines to the pixel grid and hands back an 8-bit coverage bitmap,
//! which [`blend`] composites in sRGB exactly as before. Layout stays our
//! own character-grid math — swash only replaces the glyph → pixels step.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};

use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Angle, Format, Transform, Vector};
use swash::{CacheKey, FontRef, GlyphId};

/// How glyphs are hinted and placed — the knob behind the Zed comparison
/// (`docs/modeler-text-rendering-research.md`). The two axes are not
/// independent in practice, so this offers the three combinations worth
/// looking at rather than a matrix of four.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextMode {
    /// What shipped before 2026-09-14: outlines hinted everywhere, and the
    /// two pipelines disagreeing about placement — the editor (cosmic-text)
    /// putting glyphs on quarter-pixel bins, the panes (this file) snapping
    /// to whole pixels. Kept so the comparison stays runnable.
    Current,
    /// Zed's combination: no hinting, quarter-pixel placement everywhere.
    /// Unhinted outlines are all equally soft, so a line reads uniform —
    /// hinting-plus-subpixel is what makes stem weight vary within a line.
    Zed,
    /// The other way out of that: hinted AND whole-pixel everywhere, so the
    /// editor matches the REPL panes. Uniformly crisp instead of uniformly
    /// soft; spacing jitters by up to half a pixel in exchange.
    Crisp,
    /// Zed's geometry PLUS its weight: unhinted, quarter-pixel, and coverage
    /// pushed through a gamma curve so strokes read heavier. macOS thickens
    /// glyphs by a luminance-dependent amount (CoreGraphics font smoothing,
    /// which Zed reimplements in `glyph_dilation_for_color`) and we don't —
    /// that missing weight, not the hinting, is what reads as "thin" beside
    /// Zed. A curve is an approximation of a real outline dilation: it
    /// darkens the antialiased edge but cannot grow a stem past it.
    ///
    /// **The default.** Kevin's pick from the four-way screenshot A/B
    /// (2026-09-14), confirmed on his display at every scale he tried.
    Smooth,
    /// Resolve per display at draw time — [`TextMode::Smooth`] at scale
    /// >= 1.5, [`TextMode::Crisp`] below it. Briefly the default, on the
    /// > theory that unhinted text loses where the em is small: at 13px glyphs
    /// > the unhinted modes swing about three times as many pixels as at 26px.
    /// > Kevin then looked at it and preferred `:smooth` at every scale — a
    /// > pixel diff says how much changed, never whether it improved, and this
    /// > mode mistook the one for the other. Kept, not default, because if a
    /// > genuinely low-DPI screen (the Linux box) ever does read worse, this is
    /// > the fix a config file can't be: `init.scm` is evaluated before the
    /// > window exists, so it cannot know the scale, and resolving per draw
    /// > follows the window between displays. Any explicit mode overrides it.
    Auto,
}

static MODE: AtomicU8 = AtomicU8::new(3); // :smooth — see TextMode::Smooth

/// The text-rendering mode in effect.
pub fn mode() -> TextMode {
    match MODE.load(Ordering::Relaxed) {
        1 => TextMode::Zed,
        2 => TextMode::Crisp,
        3 => TextMode::Smooth,
        4 => TextMode::Auto,
        _ => TextMode::Current,
    }
}

/// The mode actually drawing: [`mode`] with [`TextMode::Auto`] resolved
/// against the display's current scale. Every policy question below asks
/// this, never [`mode`].
pub fn effective() -> TextMode {
    match mode() {
        TextMode::Auto => {
            if crate::host::ui_scale() >= 1.5 {
                TextMode::Smooth
            } else {
                TextMode::Crisp
            }
        }
        m => m,
    }
}

/// Set the text-rendering mode; the next frame draws under it (nothing is
/// cached across the change — glyphs rasterize per draw, and cosmic-text
/// keys its cache on the flags we set).
pub fn set_mode(m: TextMode) {
    MODE.store(match m {
        TextMode::Current => 0,
        TextMode::Zed => 1,
        TextMode::Crisp => 2,
        TextMode::Smooth => 3,
        TextMode::Auto => 4,
    }, Ordering::Relaxed);
    crate::host::mark_dirty_ui();
}

/// Whether outlines are grid-fit before rasterizing. swash runs real
/// TrueType hinting; CoreText (so Zed) does not hint at all.
pub fn hinting() -> bool {
    !matches!(effective(), TextMode::Zed | TextMode::Smooth)
}

/// The exponent coverage is pushed through before compositing. 1.0 leaves
/// the rasterizer's alpha alone; below 1.0 lifts every partial pixel, which
/// reads as a heavier stroke — our stand-in for the dilation CoreGraphics
/// applies and we don't. Chosen by eye against Zed, not derived.
pub fn coverage_gamma() -> f32 {
    if effective() == TextMode::Smooth { 0.65 } else { 1.0 }
}

/// Apply [`coverage_gamma`] to an 0..=1 coverage value.
pub fn weigh(coverage: f32) -> f32 {
    let g = coverage_gamma();
    if g == 1.0 { coverage } else { coverage.powf(g) }
}

/// Whether glyphs sit on quarter-pixel bins rather than whole pixels.
/// `editor` selects the pipeline, because :current keeps their historical
/// disagreement — which is itself one of the things under test.
pub fn subpixel(editor: bool) -> bool {
    match effective() {
        TextMode::Current => editor,
        TextMode::Zed | TextMode::Smooth => true,
        // Auto resolved above; listing it keeps this exhaustive by hand
        // rather than by a catch-all that would swallow a new mode.
        TextMode::Crisp | TextMode::Auto => false,
    }
}

use crate::framebuffer::Framebuffer;

/// IBM Plex Sans Regular — the UI/label face: menus (linear popup AND the
/// radial pie), completion rows, the status strip, viewport HUDs. This is the
/// face Zed draws its own popups in (it ships IBM Plex Sans behind the
/// `.ZedSans` alias), which is the look this is after; DejaVu Sans read wide
/// and dated beside it. Replaceable live via `set-label-font`, exactly like
/// the mono face. OFL 1.1 — `assets/IBMPlexSans-OFL.txt`.
///
/// ray's PNG annotation still embeds DejaVu Sans (`src/film.rs`), so the
/// workspace keeps that asset; this is a second, smaller vendored face
/// (200 KB vs 760 KB) rather than a replacement.
const LABEL_BYTES: &[u8] = include_bytes!("../../assets/IBMPlexSans-Regular.ttf");

/// JetBrains Mono Regular — the text panes' default terminal face
/// (replaceable live via `set-text-font`).
const MONO_BYTES: &[u8] = include_bytes!("../../assets/JetBrainsMono-Regular.ttf");

/// Owns font bytes plus a stable [`CacheKey`], so a fresh [`FontRef`] can be
/// materialized on demand without minting a new key each time — rebuilding
/// via `FontRef::from_index` in the draw loop would give every call a unique
/// key and thrash the [`ScaleContext`] outline cache instead of hitting it.
struct OwnedFont {
    data: Vec<u8>,
    offset: u32,
    key: CacheKey,
}

impl OwnedFont {
    /// Parse face `index` out of `data` (0 for a plain .ttf/.otf; other
    /// indices pick a face inside a .ttc). `None` if the bytes aren't a font.
    fn new(data: Vec<u8>, index: usize) -> Option<Self> {
        let f = FontRef::from_index(&data, index)?;
        let (offset, key) = (f.offset, f.key);
        Some(Self { data, offset, key })
    }

    /// A borrowed view carrying the *same* cache key every call.
    fn as_ref(&self) -> FontRef<'_> {
        FontRef { data: &self.data, offset: self.offset, key: self.key }
    }
}

thread_local! {
    /// The text panes' monospace face — swappable at runtime (the prelude's
    /// `set-text-font` loads any local .ttf/.otf/.ttc; system fonts like
    /// Menlo are usable without vendoring them).
    static MONO: RefCell<OwnedFont> = RefCell::new(
        OwnedFont::new(MONO_BYTES.to_vec(), 0).expect("vendored mono font is valid"),
    );

    /// THE GLYPH OF LAST RESORT for the mono face, and never swapped.
    ///
    /// The fontdb holds ONE face on purpose — no system-font discovery, no
    /// machine-dependent fallback — and the cost of that is a character the
    /// chosen face lacks drawing as tofu. Monaco, which is what
    /// `~/.config/modeler/init.scm` picks on macOS, has no U+03BB: the
    /// REPL prompt went in as `scheme λ > ` and came out as `scheme ▯ > `
    /// (Kevin, 2026-10-22, who asked whether the other surfaces could stay
    /// in Monaco — they can, and this is how).
    ///
    /// This is NOT the fallback that decision ruled out. It is the face
    /// already compiled into the binary, so it costs no discovery, and it
    /// is the same on every machine — the two things the original note was
    /// protecting. It is consulted per GLYPH and only when the chosen face
    /// has none, so a user's font still draws everything it can.
    static MONO_FALLBACK: RefCell<OwnedFont> = RefCell::new(
        OwnedFont::new(MONO_BYTES.to_vec(), 0).expect("vendored mono font is valid"),
    );

    /// The proportional UI/label face — swappable the same way (the prelude's
    /// `set-label-font`). Was a `OnceLock`; menus wanting a different face
    /// than the vendored default is exactly why it is mutable now.
    static LABEL: RefCell<OwnedFont> = RefCell::new(
        OwnedFont::new(LABEL_BYTES.to_vec(), 0).expect("vendored label font is valid"),
    );

    /// swash's outline/scratch cache — reused across every glyph draw (it is
    /// the perf-critical piece; recreating it per call would defeat caching).
    static SCALE: RefCell<ScaleContext> = RefCell::new(ScaleContext::new());
}

/// Parse face `index` out of the font file at `path`. A leading `~/` expands
/// to $HOME (`fs::read` would take it literally). Shared by both face
/// setters so they agree on path handling and error wording.
fn load_face(path: &str, index: u32) -> Result<OwnedFont, String> {
    let expanded;
    let path = if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        expanded = format!("{home}/{rest}");
        &expanded
    } else {
        path
    };
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    OwnedFont::new(bytes, index as usize).ok_or_else(|| format!("cannot parse {path}"))
}

/// Replace the monospace face from a font file on disk; `index` picks a
/// face inside a .ttc collection. On error the current face stays.
pub fn set_mono_font(path: &str, index: u32) -> Result<(), String> {
    let font = load_face(path, index)?;
    MONO.with(|m| *m.borrow_mut() = font);
    // The editor pane shapes with its own copy of the face — keep it in step
    // so a set-text-font never leaves the two text surfaces mismatched.
    crate::editorpane::set_font(mono_font_bytes());
    crate::host::mark_dirty();
    Ok(())
}

/// Replace the proportional UI/label face — menus, completion rows, the
/// status strip, viewport HUDs. On error the current face stays. Unlike the
/// mono face this has no editor-pane twin to keep in step (cosmic-text shapes
/// code, not chrome), so it is just the swap plus a repaint.
///
/// Geometry note: popup and pie boxes are measured ONCE at show time
/// (`textpane::overlay_geometry` / `show_pie`), so a swap while a menu is up
/// leaves that menu's boxes sized for the old face. Harmless — popups are
/// transient — but it is why this marks dirty rather than trying to re-lay-out.
pub fn set_label_font(path: &str, index: u32) -> Result<(), String> {
    let font = load_face(path, index)?;
    LABEL.with(|l| *l.borrow_mut() = font);
    crate::host::mark_dirty();
    Ok(())
}

/// The current mono face's raw bytes — what the editor pane's private
/// fontdb loads, so both text surfaces draw the same font.
pub fn mono_font_bytes() -> Vec<u8> {
    MONO.with(|m| m.borrow().data.clone())
}

/// The monospace grid at `px` tall: (column advance, row height). Every
/// text-pane layout computation — wrap columns, cursor x, row count — uses
/// exactly these two numbers.
pub fn mono_metrics(px: f32) -> (f32, f32) {
    MONO.with(|m| {
        let of = m.borrow();
        let f = of.as_ref();
        let advance = f.glyph_metrics(&[]).scale(px).advance_width(f.charmap().map('m'));
        // swash reports descent as a positive distance below the baseline
        // (it negates the font's descender), so the three add up.
        let m = f.metrics(&[]).scale(px);
        (advance, (m.ascent + m.descent + m.leading) * 1.08)
    })
}

/// Rasterize glyph `gid` from `font` at `px` tall and composite it via
/// [`blend`], with its origin at (`pen`, `baseline`). Hinting and horizontal
/// placement follow [`mode`]: whole-pixel snapping (crisp, spacing jitters)
/// or quarter-pixel bins carried into the rasterizer as a [`Render::offset`]
/// (even spacing, softer stems) — see [`draw_mono_line`]. Clips at the
/// buffer edges.
///
/// `emph` reaches the rasterizer as an OUTLINE operation, never as a pass
/// over the finished bitmap: a sheared alpha map stair-steps and a dilated
/// one blurs, while a sheared or fattened outline is antialiased by the
/// rasterizer like any other curve. Hinting is dropped for both, because
/// hinting snaps stems to the pixel grid and these move them off it again.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    fb: &mut Framebuffer,
    font: FontRef,
    px: f32,
    gid: GlyphId,
    pen: f32,
    baseline: f32,
    ink: u32,
    alpha: f32,
    emph: Emphasis,
) {
    // Four bins, Zed's count (SUBPIXEL_VARIANTS_X) and cosmic-text's, so the
    // two pipelines quantize alike. A fraction that rounds to a whole pixel
    // carries into the origin instead of the offset.
    let (origin_x, offset_x) = if subpixel(false) {
        let mut frac = (pen.fract() * 4.0).round() / 4.0;
        let mut base = pen.floor();
        if frac >= 1.0 {
            base += 1.0;
            frac = 0.0;
        }
        (base, frac)
    } else {
        (pen.round(), 0.0)
    };
    let image = SCALE.with(|c| {
        let mut ctx = c.borrow_mut();
        let mut scaler =
            ctx.builder(font).size(px).hint(hinting() && emph == Emphasis::Regular).build();
        Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .offset(Vector::new(offset_x, 0.0))
            .transform((emph == Emphasis::Italic).then(|| {
                // x' = x + tan(SLANT_DEG)·y, in the outline's y-up space: the
                // top leans RIGHT, the way a Latin italic leans.
                Transform::skew(Angle::from_degrees(SLANT_DEG), Angle::from_degrees(0.0))
            }))
            .embolden(if emph == Emphasis::Bold { px * BOLD_FRACTION } else { 0.0 })
            .render(&mut scaler, gid)
    });
    let Some(image) = image else { return };
    let (w, h) = (image.placement.width as i32, image.placement.height as i32);
    if w == 0 || h == 0 {
        return;
    }
    // Placement is relative to the origin: `left` offsets the pen, `top` is
    // the height above the baseline (up-positive), and rows descend from
    // there. So the bitmap's top-left in buffer space is (pen+left, base-top).
    let ox = origin_x as i32 + image.placement.left;
    let oy = baseline.round() as i32 - image.placement.top;
    for row in 0..h {
        let by = oy + row;
        if by < 0 || by >= fb.height as i32 {
            continue;
        }
        for col in 0..w {
            let cov = image.data[(row * w + col) as usize];
            if cov == 0 {
                continue;
            }
            let bx = ox + col;
            if bx < 0 || bx >= fb.width as i32 {
                continue;
            }
            let idx = (by as u32 * fb.width + bx as u32) as usize;
            fb.pixels[idx] = blend(fb.pixels[idx], ink, weigh(cov as f32 / 255.0) * alpha);
        }
    }
}

/// Draw one monospace line with its top-left at (x, y): each char advances
/// exactly one grid column, so callers can convert columns to pixels with
/// `mono_metrics` and never measure. Clips at the buffer edges.
///
/// Crispness (the CoreText gap, narrowed): every glyph's pen position and
/// the baseline are ROUNDED to the pixel grid — a fractional advance would
/// smear each char through a different subpixel offset — and swash hints
/// the outline before coverage is composited in sRGB ([`blend`]).
pub fn draw_mono_line(fb: &mut Framebuffer, x: f32, y: f32, px: f32, ink: u32, text: &str) {
    let advance = mono_metrics(px).0;
    MONO.with(|m| {
        let of = m.borrow();
        let f = of.as_ref();
        let baseline = (y + f.metrics(&[]).scale(px).ascent).round();
        let cmap = f.charmap();
        for (i, ch) in text.chars().enumerate() {
            let pen = (x + i as f32 * advance).round();
            if pen > fb.width as f32 {
                break;
            }
            let gid = cmap.map(ch);
            // GLYPH 0 IS `.notdef`, the tofu box. A face the user chose may
            // simply not have the character — so borrow that one glyph from
            // the vendored face rather than draw a box, keeping THIS face
            // for everything it can draw. The baseline and the pen stay the
            // primary's, so the character grid does not shift.
            if gid == 0 && ch != ' ' {
                MONO_FALLBACK.with(|b| {
                    let ob = b.borrow();
                    let bf = ob.as_ref();
                    let bgid = bf.charmap().map(ch);
                    if bgid != 0 {
                        draw_glyph(fb, bf, px, bgid, pen, baseline, ink, 1.0, Emphasis::Regular);
                    }
                });
                continue;
            }
            draw_glyph(fb, f, px, gid, pen, baseline, ink, 1.0, Emphasis::Regular);
        }
    });
}

/// A hover tip's footprint for `text`: (w, h) in physical px.
pub fn tip_size(text: &str, scale: f32) -> (f32, f32) {
    let px = 12.0 * scale;
    (measure_label(px, text) + 14.0 * scale, label_height(px) + 6.0 * scale)
}

/// Hover help: `text` in a glass tag with its top-left at (x, y), clipped
/// to `clip`, at `fade` (0..1, for one that fades in). Every tip in edit
/// and the modeler is this object — the strip's icons, the modeler's
/// toggles and its toolbox (Kevin, 2026-11-14). Returns the tag's rect.
pub fn draw_tip_at(
    fb: &mut Framebuffer,
    x: f32,
    y: f32,
    text: &str,
    scale: f32,
    fade: f32,
    clip: crate::framebuffer::Rect,
) -> crate::framebuffer::Rect {
    let theme = crate::theme::get();
    let px = 12.0 * scale;
    let (w, h) = tip_size(text, scale);
    let tag = crate::framebuffer::Rect {
        x: x.max(0.0) as u32,
        y: y.max(0.0) as u32,
        w: w.ceil() as u32,
        h: h.ceil() as u32,
    };
    crate::framebuffer::glass_rect(fb, tag, 8.0 * scale, theme.popup_bg, 0.88 * fade, 0.10 * fade, clip);
    let top = label_center_top(px, tag.y as f32, tag.h as f32);
    draw_label(fb, (tag.x as f32 + 7.0 * scale) as i32, top as i32, px, theme.menu_ink, fade, text);
    tag
}

/// [`draw_tip_at`] centred above `anchor` (up is toward the panes), kept
/// inside the buffer — a status-strip icon's tip.
pub fn draw_tip(fb: &mut Framebuffer, anchor: crate::framebuffer::Rect, text: &str, scale: f32) {
    let (w, h) = tip_size(text, scale);
    let x = (anchor.x as f32 + anchor.w as f32 / 2.0 - w / 2.0)
        .min(fb.width as f32 - w - 4.0 * scale)
        .max(0.0);
    let y = (anchor.y as f32 - 6.0 * scale - h).max(0.0);
    let full = crate::framebuffer::Rect { x: 0, y: 0, w: fb.width, h: fb.height };
    draw_tip_at(fb, x, y, text, scale, 1.0, full);
}

/// Draw `text` with its top-left near (x, y), `px` tall, blending `ink`
/// over whatever the buffer holds. `alpha` scales the whole label's opacity
/// (1.0 = solid; the viewport HUD draws at a fraction so the scene shows
/// through). Clips at the buffer edges.
pub fn draw_label(fb: &mut Framebuffer, x: i32, y: i32, px: f32, ink: u32, alpha: f32, text: &str) {
    draw_label_emph(fb, x, y, px, ink, alpha, text, Emphasis::Regular);
}

/// A second voice in a line of UI text, drawn from the ONE vendored face by
/// transforming its outlines — the modeler has no bold or italic font asset.
///
/// NOTHING ASKS FOR A SECOND VOICE TODAY, and the variants are kept all the
/// same. Both were built for the options strip's object name and both were
/// rejected on sight (Kevin, 2026-09-30: "the italics works, but I don't like
/// it", then "I don't like the bold either"), and what the strip uses instead
/// is colour — see `optionstrip::draw`. The verdict there was about THAT
/// line, though, not about the mechanism: a synthesized weight is an
/// imitation of a face that is not vendored, and it reads as one at 13px
/// beside real text. Somewhere that wants a quiet second voice and cannot
/// spend a colour may still want this, and the hard part — doing it on the
/// outline rather than over the finished bitmap — is done and tested.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
#[allow(dead_code)] // kept deliberately — see above
pub enum Emphasis {
    #[default]
    Regular,
    /// A synthetic oblique, sheared [`SLANT_DEG`].
    Italic,
    /// A faux bold: the outline dilated by [`BOLD_FRACTION`] of the size.
    Bold,
}

/// How far [`Emphasis::Italic`] leans, in degrees. 12° is the shallow end of
/// what real italic faces use (IBM Plex Sans Italic is ~10°, Helvetica
/// Oblique 12°): enough to read as a different voice in a 13px strip, not
/// enough to look like a mistake.
///
/// A shear is not a true italic — an italic face redraws letterforms, giving
/// `a` a single storey and `f` a descender — and at this size that difference
/// is below the pixel grid, which is why no second face is vendored for it.
const SLANT_DEG: f32 = 12.0;

/// How far [`Emphasis::Bold`] dilates the outline, as a fraction of the size.
/// 1/24 em is FreeType's own faux-bold strength and lands at about half a
/// pixel at 13px — the smallest step that still reads as weight rather than
/// as a blurry regular.
///
/// The same caveat as the slant: a real bold face is redrawn, not dilated,
/// with its own proportions and spacing. Dilation keeps the ADVANCES of the
/// regular, so bold text occupies exactly the width `measure_label` says —
/// convenient here, and the giveaway at display sizes.
const BOLD_FRACTION: f32 = 1.0 / 24.0;

/// [`draw_label`] in a second voice — the general form, which [`draw_label`]
/// itself calls with [`Emphasis::Regular`].
#[allow(clippy::too_many_arguments)]
pub fn draw_label_emph(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    px: f32,
    ink: u32,
    alpha: f32,
    text: &str,
    emph: Emphasis,
) {
    LABEL.with(|l| {
        let of = l.borrow();
        let f = of.as_ref();
        let gm = f.glyph_metrics(&[]).scale(px);
        let cmap = f.charmap();
        let baseline = (y as f32 + f.metrics(&[]).scale(px).ascent).round();
        let mut pen = x as f32;
        for ch in text.chars() {
            let gid = cmap.map(ch);
            draw_glyph(fb, f, px, gid, pen, baseline, ink, alpha, emph);
            pen += gm.advance_width(gid);
        }
    });
}

/// How far past its advance width an emphasized label's last glyph reaches,
/// at `px` tall. Neither transform touches an ADVANCE — so [`measure_label`]
/// is still right about where the next word starts — but a slanted ascender
/// overhangs to the right and a dilated outline grows on every side, and a
/// caller packing something flush against the end has to leave this much
/// room or the tail is clipped.
#[allow(dead_code)] // the other half of the kept facility — see [`Emphasis`]
pub fn label_emph_overhang(px: f32, emph: Emphasis) -> f32 {
    match emph {
        Emphasis::Regular => 0.0,
        Emphasis::Italic => {
            let ascent = LABEL.with(|l| l.borrow().as_ref().metrics(&[]).scale(px).ascent);
            ascent * SLANT_DEG.to_radians().tan()
        }
        Emphasis::Bold => px * BOLD_FRACTION,
    }
}

/// The proportional (label) face's row height at `px` — the popup's menu
/// rows lay out with this.
pub fn label_height(px: f32) -> f32 {
    let m = LABEL.with(|l| l.borrow().as_ref().metrics(&[]).scale(px));
    (m.ascent + m.descent + m.leading) * 1.12
}

/// The `y` (line-box top) to pass a draw fn so the text's CAP band is
/// vertically centered in a box `box_h` tall starting at `box_top`. Centering
/// the full line box reads high — the descent+leading below the baseline is
/// empty for status/UI text, so the ink floats up. Caps span
/// `[baseline-cap, baseline]`, baseline = `y + ascent`; solve for `y`.
fn center_top(m: swash::Metrics, box_top: f32, box_h: f32) -> f32 {
    let cap = if m.cap_height > 0.0 { m.cap_height } else { m.ascent * 0.72 };
    box_top + (box_h + cap) * 0.5 - m.ascent
}

/// [`center_top`] for the monospace face ([`draw_mono_line`]).
pub fn mono_center_top(px: f32, box_top: f32, box_h: f32) -> f32 {
    MONO.with(|m| center_top(m.borrow().as_ref().metrics(&[]).scale(px), box_top, box_h))
}

/// [`center_top`] for the proportional label face ([`draw_label`]).
pub fn label_center_top(px: f32, box_top: f32, box_h: f32) -> f32 {
    LABEL.with(|l| center_top(l.borrow().as_ref().metrics(&[]).scale(px), box_top, box_h))
}

/// The advance width of `text` at `px` tall — what right-aligned placement
/// (the HUD's :top-right/:bottom-right) needs; the seed of `measure-text`.
/// How far below a line's top its baseline sits, in the mono face at `px`.
pub fn mono_ascent(px: f32) -> f32 {
    MONO.with(|m| m.borrow().as_ref().metrics(&[]).scale(px).ascent)
}

/// The same for the label face — so a label set beside mono text can share
/// its baseline (the file tree's names after its mono markers).
pub fn label_ascent(px: f32) -> f32 {
    LABEL.with(|l| l.borrow().as_ref().metrics(&[]).scale(px).ascent)
}

pub fn measure_label(px: f32, text: &str) -> f32 {
    LABEL.with(|l| {
        let of = l.borrow();
        let f = of.as_ref();
        let gm = f.glyph_metrics(&[]).scale(px);
        let cmap = f.charmap();
        text.chars().map(|ch| gm.advance_width(cmap.map(ch))).sum()
    })
}

/// Per-channel lerp of `ink` over `bg` by coverage `a` — in sRGB space,
/// deliberately: linear-light compositing is physically right but fattens
/// light-on-dark text into a blur (tried 2026-09-05, looked worse — fonts
/// and terminals are tuned around sRGB blending).
pub fn blend(bg: u32, ink: u32, a: f32) -> u32 {
    let a = a.clamp(0.0, 1.0);
    let ch = |shift: u32| {
        let b = ((bg >> shift) & 0xff) as f32;
        let i = ((ink >> shift) & 0xff) as f32;
        (((b + (i - b) * a) as u32) & 0xff) << shift
    };
    ch(16) | ch(8) | ch(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_land_where_asked_and_clip_at_edges() {
        let mut fb = Framebuffer::new(120, 30);
        draw_label(&mut fb, 4, 4, 14.0, 0x00ff_ffff, 1.0, "outliner");
        let lit = fb.pixels.iter().filter(|&&p| p > 0x40_4040).count();
        assert!(lit > 40, "a word should light real pixels, got {lit}");
        // Everything stays inside the top-left region we asked for.
        assert!(
            fb.pixels
                .iter()
                .enumerate()
                .all(|(i, &p)| p == 0 || (i as u32 % fb.width) < 80),
            "glyphs must not stray far right of the pen"
        );
        // Off-buffer coordinates clip instead of panicking.
        draw_label(&mut fb, -50, -50, 14.0, 0x00ff_ffff, 1.0, "clip");
        draw_label(&mut fb, 115, 25, 14.0, 0x00ff_ffff, 1.0, "clip");
    }

    #[test]
    fn alpha_dims_the_whole_label() {
        let mut solid = Framebuffer::new(60, 20);
        let mut dim = Framebuffer::new(60, 20);
        draw_label(&mut solid, 2, 2, 14.0, 0x00ff_ffff, 1.0, "persp");
        draw_label(&mut dim, 2, 2, 14.0, 0x00ff_ffff, 0.4, "persp");
        let max = |fb: &Framebuffer| fb.pixels.iter().map(|&p| p >> 16).max().unwrap();
        assert!(max(&solid) > 0xf0, "solid label should reach near-full ink");
        let dm = max(&dim);
        assert!(dm > 0x30 && dm < 0x90, "0.4 alpha should cap coverage, got {dm:#x}");
    }

    #[test]
    fn label_font_swaps_at_runtime_and_survives_bad_loads() {
        // The label face was a OnceLock until menus wanted a different face
        // than the vendored default; this pins the swap it grew.
        let before = (label_height(14.0), measure_label(14.0, "Extrude"));
        assert!(set_label_font("/no/such/font.ttf", 0).is_err());
        assert_eq!(
            (label_height(14.0), measure_label(14.0, "Extrude")),
            before,
            "a bad load keeps the face"
        );
        // The crate's other vendored face, the mono one: its advances
        // differ, which proves the face actually changed.
        let other = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/JetBrainsMono-Regular.ttf");
        assert!(set_label_font(other, 0).is_ok());
        assert_ne!(
            measure_label(14.0, "Extrude"),
            before.1,
            "label advances follow the face"
        );
        // Restore the default so sibling tests in this thread see Plex.
        let plex = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/IBMPlexSans-Regular.ttf");
        assert!(set_label_font(plex, 0).is_ok());
        assert_eq!(measure_label(14.0, "Extrude"), before.1, "restored to Plex");
    }

    #[test]
    fn mono_font_swaps_at_runtime_and_survives_bad_loads() {
        let before = mono_metrics(14.0);
        assert!(set_mono_font("/no/such/font.ttf", 0).is_err());
        assert_eq!(mono_metrics(14.0), before, "a bad load keeps the face");
        // Any parseable font swaps in; the vendored Plex (proportional)
        // differs in metrics, which proves the face actually changed.
        let other = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/IBMPlexSans-Regular.ttf");
        assert!(set_mono_font(other, 0).is_ok());
        assert_ne!(mono_metrics(14.0).0, before.0, "metrics follow the face");
    }

    #[test]
    fn blend_lerps_channels() {
        assert_eq!(blend(0x00000000, 0x00ff8040, 1.0), 0x00ff8040);
        assert_eq!(blend(0x00102030, 0x00ff8040, 0.0), 0x00102030);
        let half = blend(0x00000000, 0x00ff0000, 0.5);
        assert!((half >> 16) > 0x70 && (half >> 16) < 0x90);
    }

    /// :auto is a property of the SCREEN, so it has to be re-read per draw —
    /// not decided once at boot, when the window (and its scale) doesn't
    /// exist yet.
    #[test]
    fn auto_mode_follows_the_display_scale() {
        let restore = (mode(), crate::host::ui_scale());
        set_mode(TextMode::Auto);

        crate::host::testing::set_ui_scale(2.0);
        assert_eq!(effective(), TextMode::Smooth, "HiDPI gets the unhinted, weighted look");
        assert!(!hinting());
        assert!(subpixel(false), "…including the panes, not just the editor");
        assert!(coverage_gamma() < 1.0);

        crate::host::testing::set_ui_scale(1.0);
        assert_eq!(effective(), TextMode::Crisp, "1x keeps hinting and whole pixels");
        assert!(hinting());
        assert!(!subpixel(true), "…including the editor, not just the panes");
        assert_eq!(coverage_gamma(), 1.0);

        // An explicit mode outranks the display.
        set_mode(TextMode::Zed);
        crate::host::testing::set_ui_scale(1.0);
        assert_eq!(effective(), TextMode::Zed);

        set_mode(restore.0);
        crate::host::testing::set_ui_scale(restore.1);
    }
}
