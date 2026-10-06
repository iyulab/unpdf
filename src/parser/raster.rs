//! Page rasterization (feature `raster`).
//!
//! A page is painted from the same operator stream text extraction reads —
//! [`form_xobject::page_operations`](super::form_xobject::page_operations), forms interpreted
//! in place — so the page rendered is the page extracted: the same page tree, box, rotation
//! and damaged-stream recovery. What this cannot paint yet is counted in [`RasterGaps`], the
//! way extraction counts what it could not read: the rest of the page is still painted.

use tiny_skia::{
    FillRule, FilterQuality, IntSize, LineCap, LineJoin, Mask, Paint, Path, PathBuilder, Pixmap,
    PixmapPaint, Stroke, StrokeDash, Transform,
};

use std::collections::HashMap;

use super::backend::{
    ContentOp, ImageColorSpace, ObjectId, PageId, PdfBackend, PdfValue, RawXObject,
};
use super::png_encode::{self, PngColorType};
use super::raster_text::LoadedFont;
use crate::error::{Error, Result};

/// How to rasterize a page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterOptions {
    /// Resolution in dots per inch; a page point is `dpi / 72` pixels. Default 150.
    pub dpi: f32,
    /// Which of the page's boxes is painted. Default [`PageRegion::Crop`].
    pub region: PageRegion,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            dpi: 150.0,
            region: PageRegion::Crop,
        }
    }
}

/// The region of a page a raster covers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PageRegion {
    /// The crop box — what a viewer shows, and what a printed page holds.
    #[default]
    Crop,
    /// The whole media box, including what the crop box cuts away.
    Media,
}

/// What a rasterized page could not show, by reason. All zero means everything the page's
/// content asks for was painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RasterGaps {
    /// Text runs not painted, or painted only in part: the font's program is not embedded,
    /// or is one this does not read (Type 1, Type 3), or a code selects no glyph in it.
    pub text_runs: u32,
    /// Images not painted: a codec this does not decode (JPEG 2000, JBIG2, CCITT), a color
    /// space it does not convert, or data that did not decode.
    pub images: u32,
    /// Inline images (`BI … EI`) not painted.
    pub inline_images: u32,
    /// Shadings (`sh`, or a shading pattern used as a color) not painted.
    pub shadings: u32,
    /// Content streams of the page that could not be decoded; whatever they held is missing.
    pub undecodable_content_streams: u32,
}

impl RasterGaps {
    /// Whether anything the page asked for was left unpainted.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A rasterized page: `width × height` pixels, opaque RGBA, row by row from the top-left of
/// the page as displayed (its `/Rotate` applied).
#[derive(Debug, Clone)]
pub struct RasteredPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub gaps: RasterGaps,
}

impl RasteredPage {
    /// The page as a PNG (8-bit RGB).
    pub fn to_png(&self) -> Vec<u8> {
        let rgb: Vec<u8> = self
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        png_encode::encode(self.width, self.height, PngColorType::Rgb, &rgb)
            .expect("an RGB buffer of width * height pixels encodes")
    }
}

/// Pages larger than this many pixels are refused rather than allocated.
const MAX_PIXELS: u64 = 200_000_000;

/// Rasterize page `page_num` of the document `backend` reads.
pub(crate) fn render_page(
    backend: &dyn PdfBackend,
    page_num: u32,
    options: &RasterOptions,
) -> Result<RasteredPage> {
    let pages = backend.pages();
    let page = *pages
        .get(&page_num)
        .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;
    if !(options.dpi.is_finite() && options.dpi > 0.0) {
        return Err(Error::Render(format!(
            "dpi must be positive, got {}",
            options.dpi
        )));
    }

    let page_box = match options.region {
        PageRegion::Crop => backend.crop_box(page),
        PageRegion::Media => backend.page_box(page),
    };
    let rotation = backend.page_rotation(page);
    let scale = options.dpi / 72.0;
    let (w, h) = (page_box.width() * scale, page_box.height() * scale);
    let (px_w, px_h) = match rotation {
        90 | 270 => (h, w),
        _ => (w, h),
    };
    let (px_w, px_h) = (px_w.round().max(1.0) as u32, px_h.round().max(1.0) as u32);
    if u64::from(px_w) * u64::from(px_h) > MAX_PIXELS {
        return Err(Error::Render(format!(
            "page {page_num} at {} dpi would be {px_w} x {px_h} pixels",
            options.dpi
        )));
    }

    // User space → the unrotated page, top-left origin, in pixels → the page as displayed.
    let unrotated = Transform::from_row(
        scale,
        0.0,
        0.0,
        -scale,
        -page_box.llx * scale,
        page_box.ury * scale,
    );
    let rotate = match rotation {
        90 => Transform::from_row(0.0, 1.0, -1.0, 0.0, h, 0.0),
        180 => Transform::from_row(-1.0, 0.0, 0.0, -1.0, w, h),
        270 => Transform::from_row(0.0, -1.0, 1.0, 0.0, 0.0, w),
        _ => Transform::identity(),
    };
    let base = rotate.pre_concat(unrotated);

    let mut pixmap = Pixmap::new(px_w, px_h)
        .ok_or_else(|| Error::Render(format!("cannot allocate {px_w} x {px_h} pixels")))?;
    pixmap.fill(tiny_skia::Color::WHITE);

    let painted = super::form_xobject::page_operations(backend, page)?;
    let mut painter = Painter {
        backend,
        page,
        pixmap,
        state: GState::new(base),
        stack: Vec::new(),
        path: PathBuilder::new(),
        pending_clip: None,
        tm: Transform::identity(),
        tlm: Transform::identity(),
        fonts: HashMap::new(),
        gaps: RasterGaps {
            undecodable_content_streams: painted.undecodable_streams as u32,
            ..RasterGaps::default()
        },
    };
    for op in &painted.ops {
        painter.apply(op);
    }

    let Painter { pixmap, gaps, .. } = painter;
    // The page was filled opaque white first, so every pixel is opaque and premultiplied
    // equals straight color.
    Ok(RasteredPage {
        width: px_w,
        height: px_h,
        rgba: pixmap.take(),
        gaps,
    })
}

/// A paint color: RGB in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgb(f32, f32, f32);

impl Rgb {
    const BLACK: Rgb = Rgb(0.0, 0.0, 0.0);
}

/// The color space colors are set in (`cs`/`CS`, implied by `g`/`rg`/`k`).
#[derive(Debug, Clone, PartialEq)]
enum PaintSpace {
    Gray,
    Rgb,
    Cmyk,
    Indexed(ImageColorSpace),
    /// A pattern — painting with it is a gap.
    Pattern,
    /// A space this does not convert (`Separation`, `DeviceN`, `Lab`): its components are
    /// read as a tint, 0 white to 1 black.
    Tint(usize),
}

impl PaintSpace {
    fn color(&self, values: &[f32]) -> Option<Rgb> {
        let v = |i: usize| values.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        Some(match self {
            PaintSpace::Gray => Rgb(v(0), v(0), v(0)),
            PaintSpace::Rgb => Rgb(v(0), v(1), v(2)),
            PaintSpace::Cmyk => {
                let k = 1.0 - v(3);
                Rgb((1.0 - v(0)) * k, (1.0 - v(1)) * k, (1.0 - v(2)) * k)
            }
            PaintSpace::Indexed(space) => {
                let ImageColorSpace::Indexed {
                    base,
                    hival,
                    lookup,
                } = space
                else {
                    return None;
                };
                let index = (values.first()?.round().max(0.0) as usize).min(*hival as usize);
                let n = base.components();
                let entry = lookup.get(index * n..index * n + n)?;
                let comps: Vec<f32> = entry.iter().map(|&b| f32::from(b) / 255.0).collect();
                PaintSpace::of_image_space(base).color(&comps)?
            }
            PaintSpace::Pattern => return None,
            PaintSpace::Tint(n) => {
                let tint = (0..*n).map(v).fold(0.0f32, f32::max);
                Rgb(1.0 - tint, 1.0 - tint, 1.0 - tint)
            }
        })
    }

    fn of_image_space(space: &ImageColorSpace) -> PaintSpace {
        match space {
            ImageColorSpace::Gray => PaintSpace::Gray,
            ImageColorSpace::Rgb => PaintSpace::Rgb,
            ImageColorSpace::Cmyk => PaintSpace::Cmyk,
            indexed @ ImageColorSpace::Indexed { .. } => PaintSpace::Indexed(indexed.clone()),
        }
    }
}

/// The graphics state (ISO 32000-1 §8.4) as far as painting needs it.
#[derive(Clone)]
struct GState {
    /// User space → device pixels.
    ctm: Transform,
    fill: Rgb,
    stroke: Rgb,
    fill_space: PaintSpace,
    stroke_space: PaintSpace,
    fill_alpha: f32,
    stroke_alpha: f32,
    /// Whether the current fill/stroke color is a pattern (not painted).
    fill_pattern: bool,
    stroke_pattern: bool,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter: f32,
    dash: Option<(Vec<f32>, f32)>,
    /// The clip, as a coverage mask; `None` is the whole page.
    clip: Option<Mask>,
    /// The text state (ISO 32000-1 §9.3) — part of the graphics state, saved by `q`.
    text: TextState,
}

/// Text state parameters and the font they select.
#[derive(Clone)]
struct TextState {
    /// The font resource `Tf` names, and the form whose resources it is looked up in.
    font: Option<(Option<ObjectId>, Vec<u8>)>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
    /// `Tz / 100`.
    horizontal_scale: f32,
    leading: f32,
    rise: f32,
    render_mode: i64,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            font: None,
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
        }
    }
}

impl GState {
    fn new(base: Transform) -> Self {
        Self {
            ctm: base,
            fill: Rgb::BLACK,
            stroke: Rgb::BLACK,
            fill_space: PaintSpace::Gray,
            stroke_space: PaintSpace::Gray,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            fill_pattern: false,
            stroke_pattern: false,
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: None,
            clip: None,
            text: TextState::default(),
        }
    }
}

struct Painter<'a> {
    backend: &'a dyn PdfBackend,
    page: PageId,
    pixmap: Pixmap,
    state: GState,
    stack: Vec<GState>,
    /// The path under construction, in user space.
    path: PathBuilder,
    /// A clip set by `W`/`W*`, applied when the path is next painted or ended.
    pending_clip: Option<FillRule>,
    /// The text matrix and text line matrix, inside `BT … ET`.
    tm: Transform,
    tlm: Transform,
    /// Fonts loaded so far, by the resource that names them; `None` when unpaintable.
    fonts: HashMap<(Option<ObjectId>, Vec<u8>), Option<LoadedFont>>,
    gaps: RasterGaps,
}

fn number(v: &PdfValue) -> Option<f32> {
    match v {
        PdfValue::Integer(i) => Some(*i as f32),
        PdfValue::Real(r) => Some(*r),
        _ => None,
    }
}

impl Painter<'_> {
    fn apply(&mut self, op: &ContentOp) {
        let n = |i: usize| op.operands.get(i).and_then(number);
        let all = || op.operands.iter().filter_map(number).collect::<Vec<f32>>();
        match op.operator.as_str() {
            // Graphics state.
            "q" => self.stack.push(self.state.clone()),
            "Q" => {
                if let Some(saved) = self.stack.pop() {
                    self.state = saved;
                }
            }
            "cm" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    self.state.ctm = self
                        .state
                        .ctm
                        .pre_concat(Transform::from_row(a, b, c, d, e, f));
                }
            }
            "w" => {
                if let Some(w) = n(0) {
                    self.state.line_width = w.max(0.0);
                }
            }
            "J" => {
                self.state.cap = match n(0).map(|v| v as i64) {
                    Some(1) => LineCap::Round,
                    Some(2) => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.state.join = match n(0).map(|v| v as i64) {
                    Some(1) => LineJoin::Round,
                    Some(2) => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => {
                if let Some(m) = n(0) {
                    self.state.miter = m.max(1.0);
                }
            }
            "d" => {
                let array: Vec<f32> = match op.operands.first() {
                    Some(PdfValue::Array(items)) => items.iter().filter_map(number).collect(),
                    _ => Vec::new(),
                };
                let phase = n(1).unwrap_or(0.0);
                self.state.dash =
                    (!array.is_empty() && array.iter().any(|&v| v > 0.0)).then_some((array, phase));
            }
            "gs" => {
                if let Some(PdfValue::Name(name)) = op.operands.first() {
                    if let Some(gs) = self.backend.ext_gstate(op.scope(self.page), name) {
                        if let Some(lw) = gs.line_width {
                            self.state.line_width = lw.max(0.0);
                        }
                        if let Some(a) = gs.stroke_alpha {
                            self.state.stroke_alpha = a.clamp(0.0, 1.0);
                        }
                        if let Some(a) = gs.fill_alpha {
                            self.state.fill_alpha = a.clamp(0.0, 1.0);
                        }
                    }
                }
            }

            // Color.
            "g" => self.set_fill(PaintSpace::Gray, &all()),
            "G" => self.set_stroke(PaintSpace::Gray, &all()),
            "rg" => self.set_fill(PaintSpace::Rgb, &all()),
            "RG" => self.set_stroke(PaintSpace::Rgb, &all()),
            "k" => self.set_fill(PaintSpace::Cmyk, &all()),
            "K" => self.set_stroke(PaintSpace::Cmyk, &all()),
            "cs" | "CS" => {
                let space = match op.operands.first() {
                    Some(PdfValue::Name(name)) => self.space_named(op, name),
                    _ => PaintSpace::Gray,
                };
                // Setting a space resets the color to its initial value: black, or the
                // first palette entry.
                let initial = match &space {
                    PaintSpace::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
                    PaintSpace::Gray | PaintSpace::Tint(_) | PaintSpace::Indexed(_) => vec![0.0],
                    PaintSpace::Rgb => vec![0.0, 0.0, 0.0],
                    PaintSpace::Pattern => Vec::new(),
                };
                if op.operator == "cs" {
                    self.set_fill(space, &initial);
                } else {
                    self.set_stroke(space, &initial);
                }
                // `Tint` spaces start at full tint (black) — tint 1, read as 1 - 1 = 0.
                if let PaintSpace::Tint(_) = self.state.fill_space {
                    if op.operator == "cs" {
                        self.state.fill = Rgb::BLACK;
                    }
                }
            }
            "sc" | "scn" => {
                let space = self.state.fill_space.clone();
                self.set_fill(space, &all());
            }
            "SC" | "SCN" => {
                let space = self.state.stroke_space.clone();
                self.set_stroke(space, &all());
            }

            // Path construction (user space).
            "m" => {
                if let (Some(x), Some(y)) = (n(0), n(1)) {
                    self.path.move_to(x, y);
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (n(0), n(1)) {
                    self.path.line_to(x, y);
                }
            }
            "c" => {
                if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x3), Some(y3)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    self.path.cubic_to(x1, y1, x2, y2, x3, y3);
                }
            }
            "v" => {
                if let (Some(x2), Some(y2), Some(x3), Some(y3), Some(cur)) =
                    (n(0), n(1), n(2), n(3), self.path.last_point())
                {
                    self.path.cubic_to(cur.x, cur.y, x2, y2, x3, y3);
                }
            }
            "y" => {
                if let (Some(x1), Some(y1), Some(x3), Some(y3)) = (n(0), n(1), n(2), n(3)) {
                    self.path.cubic_to(x1, y1, x3, y3, x3, y3);
                }
            }
            "h" => self.path.close(),
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (n(0), n(1), n(2), n(3)) {
                    // A rectangle is a closed subpath of its own; negative sizes are allowed.
                    self.path.move_to(x, y);
                    self.path.line_to(x + w, y);
                    self.path.line_to(x + w, y + h);
                    self.path.line_to(x, y + h);
                    self.path.close();
                }
            }

            // Clipping: takes effect when the path is next painted or ended.
            "W" => self.pending_clip = Some(FillRule::Winding),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),

            // Path painting.
            "S" => self.paint(false, None, true),
            "s" => {
                self.path.close();
                self.paint(false, None, true)
            }
            "f" | "F" => self.paint(false, Some(FillRule::Winding), false),
            "f*" => self.paint(false, Some(FillRule::EvenOdd), false),
            "B" => self.paint(false, Some(FillRule::Winding), true),
            "B*" => self.paint(false, Some(FillRule::EvenOdd), true),
            "b" => self.paint(true, Some(FillRule::Winding), true),
            "b*" => self.paint(true, Some(FillRule::EvenOdd), true),
            "n" => self.paint(false, None, false),

            // External objects.
            "Do" => {
                if let Some(PdfValue::Name(name)) = op.operands.first() {
                    // Forms were interpreted in place; a `Do` left is an image, or a kind of
                    // XObject nothing paints (PostScript).
                    if let Some(image) = self.backend.image_xobject(op.scope(self.page), name) {
                        self.draw_image(&image);
                    }
                }
            }
            "BI" => self.gaps.inline_images += 1,
            "sh" => self.gaps.shadings += 1,

            // Text objects and positioning.
            "BT" => {
                self.tm = Transform::identity();
                self.tlm = Transform::identity();
            }
            "Tf" => {
                if let Some(PdfValue::Name(name)) = op.operands.first() {
                    self.state.text.font = Some((op.form, name.clone()));
                }
                if let Some(size) = n(1) {
                    self.state.text.size = size;
                }
            }
            "Tc" => self.state.text.char_spacing = n(0).unwrap_or(0.0),
            "Tw" => self.state.text.word_spacing = n(0).unwrap_or(0.0),
            "Tz" => self.state.text.horizontal_scale = n(0).unwrap_or(100.0) / 100.0,
            "TL" => self.state.text.leading = n(0).unwrap_or(0.0),
            "Ts" => self.state.text.rise = n(0).unwrap_or(0.0),
            "Tr" => {
                if let Some(mode) = n(0) {
                    self.state.text.render_mode = mode as i64;
                }
            }
            "Td" | "TD" => {
                if let (Some(tx), Some(ty)) = (n(0), n(1)) {
                    if op.operator == "TD" {
                        self.state.text.leading = -ty;
                    }
                    self.next_line(tx, ty);
                }
            }
            "Tm" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    self.tlm = Transform::from_row(a, b, c, d, e, f);
                    self.tm = self.tlm;
                }
            }
            "T*" => self.next_line(0.0, -self.state.text.leading),
            "Tj" => self.show(op, op.operands.get(..1).unwrap_or(&[])),
            "TJ" => {
                if let Some(PdfValue::Array(items)) = op.operands.first() {
                    self.show(op, items);
                }
            }
            "'" => {
                self.next_line(0.0, -self.state.text.leading);
                self.show(op, op.operands.get(..1).unwrap_or(&[]));
            }
            "\"" => {
                if let (Some(aw), Some(ac)) = (n(0), n(1)) {
                    self.state.text.word_spacing = aw;
                    self.state.text.char_spacing = ac;
                }
                self.next_line(0.0, -self.state.text.leading);
                self.show(op, op.operands.get(2..3).unwrap_or(&[]));
            }
            _ => {}
        }
    }

    /// Move to the start of the next line, offset `(tx, ty)` from the current one.
    fn next_line(&mut self, tx: f32, ty: f32) {
        self.tlm = self.tlm.pre_concat(Transform::from_translate(tx, ty));
        self.tm = self.tlm;
    }

    /// Paint the strings and position adjustments of a text-showing operator, advancing the
    /// text matrix as text extraction does.
    fn show(&mut self, op: &ContentOp, items: &[PdfValue]) {
        let text = self.state.text.clone();
        let Some(key) = text.font.clone() else {
            return;
        };
        let scope = op.scope(self.page);
        if !self.fonts.contains_key(&key) {
            let loaded = self
                .backend
                .font_program(scope, &key.1)
                .and_then(LoadedFont::load);
            self.fonts.insert(key.clone(), loaded);
        }
        // Mode 3 paints nothing (an OCR layer over a scan): nothing is missing.
        let invisible = matches!(text.render_mode, 3 | 7);
        let mut missing = false;
        // Taken out of the cache while its glyphs are painted, put back after.
        let mut loaded = self.fonts.remove(&key).flatten();

        for item in items {
            let bytes = match item {
                PdfValue::Str(bytes) => bytes,
                PdfValue::Integer(_) | PdfValue::Real(_) => {
                    let adjust = number(item).unwrap_or(0.0);
                    let tx = -adjust / 1000.0 * text.size * text.horizontal_scale;
                    self.tm = self.tm.pre_concat(Transform::from_translate(tx, 0.0));
                    continue;
                }
                _ => continue,
            };
            let advances = self.backend.glyph_advances(scope, &key.1, bytes);
            let Some(font) = loaded.as_mut() else {
                missing |= !invisible;
                // Still move past the run, so what follows lands where it should.
                if let Some(advances) = &advances {
                    let run: f32 = advances
                        .iter()
                        .map(|g| advance(&text, g.width, g.is_word_space))
                        .sum();
                    self.tm = self.tm.pre_concat(Transform::from_translate(run, 0.0));
                }
                continue;
            };
            let width = font.code_bytes();
            for (i, chunk) in bytes.chunks(width).enumerate() {
                let code = chunk.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b));
                let gid = font.glyph(code);
                // The width extraction measures with; the program's own when the font
                // dictionary declares none.
                let (w0, word) = match advances.as_ref().and_then(|a| a.get(i)) {
                    Some(g) => (g.width, g.is_word_space),
                    None => {
                        let units = gid.and_then(|g| font.program_advance(g)).unwrap_or(0.0);
                        (
                            units * font.glyph_to_text.sx * 1000.0,
                            width == 1 && code == 32,
                        )
                    }
                };
                match gid {
                    Some(gid) if !invisible => {
                        let glyph_space = font.glyph_to_text;
                        if let Some(outline) = font.outline(gid).cloned() {
                            let to_text = Transform::from_row(
                                text.size * text.horizontal_scale,
                                0.0,
                                0.0,
                                text.size,
                                0.0,
                                text.rise,
                            );
                            let transform = self
                                .state
                                .ctm
                                .pre_concat(self.tm)
                                .pre_concat(to_text)
                                .pre_concat(glyph_space);
                            self.paint_glyph(&outline, transform, text.render_mode);
                        }
                    }
                    Some(_) => {}
                    None => missing |= !invisible && code != 32,
                }
                let tx = advance(&text, w0, word);
                self.tm = self.tm.pre_concat(Transform::from_translate(tx, 0.0));
            }
        }
        self.fonts.insert(key, loaded);
        if missing {
            self.gaps.text_runs += 1;
        }
    }

    /// Paint one glyph outline in text render mode `mode` (fill, stroke, or both).
    fn paint_glyph(&mut self, outline: &Path, transform: Transform, mode: i64) {
        let fill = matches!(mode, 0 | 2 | 4 | 6);
        let stroke = matches!(mode, 1 | 2 | 5 | 6);
        if fill && !self.state.fill_pattern {
            let paint = solid(self.state.fill, self.state.fill_alpha);
            self.pixmap.fill_path(
                outline,
                &paint,
                FillRule::Winding,
                transform,
                self.state.clip.as_ref(),
            );
        }
        if stroke && !self.state.stroke_pattern {
            let paint = solid(self.state.stroke, self.state.stroke_alpha);
            // The line width is in user space; the glyph path is in glyph space. Stroke in
            // device space with the width the CTM gives it.
            if let Some(device) = outline.clone().transform(transform) {
                let width = self.state.line_width * transform_scale(&self.state.ctm);
                let stroke = Stroke {
                    width: if width < 1.0 { 0.0 } else { width },
                    ..Stroke::default()
                };
                self.pixmap.stroke_path(
                    &device,
                    &paint,
                    &stroke,
                    Transform::identity(),
                    self.state.clip.as_ref(),
                );
            }
        }
    }

    fn space_named(&self, op: &ContentOp, name: &[u8]) -> PaintSpace {
        match name {
            b"DeviceGray" | b"G" | b"CalGray" => PaintSpace::Gray,
            b"DeviceRGB" | b"RGB" | b"CalRGB" => PaintSpace::Rgb,
            b"DeviceCMYK" | b"CMYK" => PaintSpace::Cmyk,
            b"Pattern" => PaintSpace::Pattern,
            _ => match self.backend.named_color_space(op.scope(self.page), name) {
                Some(space) => PaintSpace::of_image_space(&space),
                // `Separation`, `DeviceN`, `Lab`, or a `Pattern` space with a base: read as
                // a tint, so separations still come out dark where they are inked.
                None => PaintSpace::Tint(1),
            },
        }
    }

    fn set_fill(&mut self, space: PaintSpace, values: &[f32]) {
        self.state.fill_pattern = space == PaintSpace::Pattern;
        if let Some(color) = space.color(values) {
            self.state.fill = color;
        }
        self.state.fill_space = space;
    }

    fn set_stroke(&mut self, space: PaintSpace, values: &[f32]) {
        self.state.stroke_pattern = space == PaintSpace::Pattern;
        if let Some(color) = space.color(values) {
            self.state.stroke = color;
        }
        self.state.stroke_space = space;
    }

    /// Paint the current path — fill with `fill`'s rule, stroke if `stroke` — then apply a
    /// pending clip and start a new path.
    fn paint(&mut self, close: bool, fill: Option<FillRule>, stroke: bool) {
        if close {
            self.path.close();
        }
        let builder = std::mem::replace(&mut self.path, PathBuilder::new());
        let path = builder.finish();
        if let Some(path) = &path {
            if let Some(rule) = fill {
                if self.state.fill_pattern {
                    self.gaps.shadings += 1;
                } else {
                    let paint = solid(self.state.fill, self.state.fill_alpha);
                    self.pixmap.fill_path(
                        path,
                        &paint,
                        rule,
                        self.state.ctm,
                        self.state.clip.as_ref(),
                    );
                }
            }
            if stroke {
                if self.state.stroke_pattern {
                    self.gaps.shadings += 1;
                } else {
                    self.stroke(path);
                }
            }
        }
        if let Some(rule) = self.pending_clip.take() {
            self.clip(path.as_ref(), rule);
        }
    }

    fn stroke(&mut self, path: &Path) {
        let paint = solid(self.state.stroke, self.state.stroke_alpha);
        // Width 0 is the thinnest line the device can show; a line thinner than a pixel is
        // drawn one pixel wide so it does not vanish.
        let scale = transform_scale(&self.state.ctm);
        let width = if self.state.line_width * scale < 1.0 {
            0.0 // tiny-skia: a hairline
        } else {
            self.state.line_width
        };
        let mut stroke = Stroke {
            width,
            miter_limit: self.state.miter,
            line_cap: self.state.cap,
            line_join: self.state.join,
            dash: None,
        };
        if let Some((array, phase)) = &self.state.dash {
            let mut array = array.clone();
            if array.len() % 2 == 1 {
                array.extend_from_within(..);
            }
            stroke.dash = StrokeDash::new(array, *phase);
        }
        self.pixmap.stroke_path(
            path,
            &paint,
            &stroke,
            self.state.ctm,
            self.state.clip.as_ref(),
        );
    }

    /// Intersect the clip with `path` (no path: an empty clip).
    fn clip(&mut self, path: Option<&Path>, rule: FillRule) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Some(path) = path else {
            self.state.clip = Mask::new(w, h);
            return;
        };
        match &mut self.state.clip {
            Some(mask) => mask.intersect_path(path, rule, true, self.state.ctm),
            None => {
                if let Some(mut mask) = Mask::new(w, h) {
                    mask.fill_path(path, rule, true, self.state.ctm);
                    self.state.clip = Some(mask);
                }
            }
        }
    }

    fn draw_image(&mut self, image: &RawXObject) {
        let Some(pixmap) = self.image_pixmap(image) else {
            self.gaps.images += 1;
            return;
        };
        // How many device pixels the image's width and height land on. An image drawn much
        // smaller than its samples is averaged down first: sampling it directly drops the
        // thin strokes of a scanned page's text.
        let ctm = self.state.ctm;
        let target = (ctm.sx.hypot(ctm.ky), ctm.kx.hypot(ctm.sy));
        let pixmap = downsample(pixmap, target);
        let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
        // Image space: the unit square, its first row at the top (y = 1).
        let to_unit = Transform::from_row(1.0 / w, 0.0, 0.0, -1.0 / h, 0.0, 1.0);
        let transform = self.state.ctm.pre_concat(to_unit);
        let paint = PixmapPaint {
            opacity: self.state.fill_alpha,
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        self.pixmap.draw_pixmap(
            0,
            0,
            pixmap.as_ref(),
            &paint,
            transform,
            self.state.clip.as_ref(),
        );
    }

    /// An image as a premultiplied pixmap: its samples, its soft mask as alpha, or — a
    /// stencil mask — the fill color where the stencil marks.
    fn image_pixmap(&self, image: &RawXObject) -> Option<Pixmap> {
        let (w, h) = (image.width?, image.height?);
        if w == 0 || h == 0 {
            return None;
        }
        let mut rgba: Vec<u8> = if image.image_mask {
            if image.filter.is_some() {
                return None;
            }
            // 0 marks, unless `/Decode [1 0]` inverts it.
            let marks_on_one = image
                .decode
                .as_deref()
                .is_some_and(|d| d.first() == Some(&1.0));
            let (_, bits) =
                png_encode::image_pixels(w, h, 1, &ImageColorSpace::Gray, None, &image.data)?;
            let Rgb(r, g, b) = self.state.fill;
            let c = [r, g, b].map(|v| (v * 255.0).round() as u8);
            bits.iter()
                .flat_map(|&v| {
                    let marked = (v == 0) != marks_on_one;
                    if marked {
                        [c[0], c[1], c[2], 255]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
                .collect()
        } else {
            let (color_type, pixels) = decode_samples(image, w, h)?;
            match color_type {
                PngColorType::Gray => pixels.iter().flat_map(|&v| [v, v, v, 255]).collect(),
                PngColorType::Rgb => pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|p| [p[0], p[1], p[2], 255])
                    .collect(),
            }
        };
        if rgba.len() != w as usize * h as usize * 4 {
            return None;
        }

        // A soft mask of the image's own size gives each pixel's opacity.
        if let Some(smask) = image.smask.as_deref() {
            if smask.width == Some(w) && smask.height == Some(h) {
                if let Some((PngColorType::Gray, alpha)) = decode_samples(smask, w, h) {
                    for (px, a) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(alpha) {
                        px[3] = a;
                    }
                }
            }
        }
        // Premultiply.
        for px in rgba.as_chunks_mut::<4>().0 {
            let a = u16::from(px[3]);
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
        Pixmap::from_vec(rgba, IntSize::from_wh(w, h)?)
    }
}

/// Average `pixmap` down by whole factors while it is at least twice the `(width, height)`
/// in device pixels it will be drawn at, so each output pixel is the mean of the samples it
/// covers.
fn downsample(pixmap: Pixmap, target: (f32, f32)) -> Pixmap {
    let (w, h) = (pixmap.width(), pixmap.height());
    let fx = ((w as f32 / target.0.max(1.0)).floor() as u32).max(1);
    let fy = ((h as f32 / target.1.max(1.0)).floor() as u32).max(1);
    if fx < 2 && fy < 2 {
        return pixmap;
    }
    let (nw, nh) = (w.div_ceil(fx), h.div_ceil(fy));
    let src = pixmap.data();
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for oy in 0..nh {
        for ox in 0..nw {
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in oy * fy..((oy + 1) * fy).min(h) {
                let row = (y * w) as usize * 4;
                for x in ox * fx..((ox + 1) * fx).min(w) {
                    let i = row + x as usize * 4;
                    for c in 0..4 {
                        sum[c] += u32::from(src[i + c]);
                    }
                    count += 1;
                }
            }
            let o = (oy * nw + ox) as usize * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] + count / 2) / count) as u8;
            }
        }
    }
    // Averages of premultiplied pixels stay premultiplied.
    IntSize::from_wh(nw, nh)
        .and_then(|size| Pixmap::from_vec(out, size))
        .unwrap_or(pixmap)
}

/// An image's samples as 8-bit gray or RGB pixels, decoding the codecs this reads.
fn decode_samples(image: &RawXObject, w: u32, h: u32) -> Option<(PngColorType, Vec<u8>)> {
    match image.filter.as_deref() {
        None => {
            let color = image.color.as_ref()?;
            png_encode::image_pixels(
                w,
                h,
                image.bits_per_component?,
                color,
                image.decode.as_deref(),
                &image.data,
            )
        }
        Some("DCTDecode") => decode_jpeg(&image.data, w, h),
        _ => None,
    }
}

/// Decode a JPEG to gray or RGB pixels of the size the image dictionary declares.
fn decode_jpeg(data: &[u8], w: u32, h: u32) -> Option<(PngColorType, Vec<u8>)> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    if (u32::from(info.width), u32::from(info.height)) != (w, h) {
        return None;
    }
    (pixels.len() == w as usize * h as usize * 3).then_some((PngColorType::Rgb, pixels))
}

/// How far one glyph moves the text position: its width `w0` (thousandths of text space),
/// character spacing, and word spacing for a single-byte space (ISO 32000-1 §9.4.4).
fn advance(text: &TextState, w0: f32, word_space: bool) -> f32 {
    let word = if word_space { text.word_spacing } else { 0.0 };
    (w0 / 1000.0 * text.size + text.char_spacing + word) * text.horizontal_scale
}

fn solid(color: Rgb, alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        (color.0 * 255.0).round() as u8,
        (color.1 * 255.0).round() as u8,
        (color.2 * 255.0).round() as u8,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    paint.anti_alias = true;
    paint
}

/// How much `t` scales lengths, on average over its two axes.
fn transform_scale(t: &Transform) -> f32 {
    let sx = t.sx.hypot(t.ky);
    let sy = t.kx.hypot(t.sy);
    ((sx * sy).abs()).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let data: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
        Pixmap::from_vec(data, IntSize::from_wh(w, h).unwrap()).unwrap()
    }

    #[test]
    fn an_image_drawn_much_smaller_is_averaged_down() {
        // Alternating black and white columns, drawn at a quarter of their size.
        let mut data = Vec::new();
        for _ in 0..8 {
            for x in 0..8 {
                let v = if x % 2 == 0 { 0 } else { 255 };
                data.extend([v, v, v, 255]);
            }
        }
        let pixmap = Pixmap::from_vec(data, IntSize::from_wh(8, 8).unwrap()).unwrap();
        let small = downsample(pixmap, (2.0, 2.0));
        assert_eq!((small.width(), small.height()), (2, 2));
        // Each output pixel is the mean of black and white: mid-gray, not one or the other.
        assert!(small
            .data()
            .chunks(4)
            .all(|p| p[0].abs_diff(128) <= 1 && p[3] == 255));
    }

    #[test]
    fn an_image_near_its_drawn_size_is_left_alone() {
        let small = downsample(solid(10, 10, [9, 9, 9, 255]), (6.0, 6.0));
        assert_eq!((small.width(), small.height()), (10, 10));
    }
}
