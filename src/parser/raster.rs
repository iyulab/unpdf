//! Page rasterization (feature `raster`).
//!
//! A page is painted from the same operator stream text extraction reads —
//! [`form_xobject::page_operations`](super::form_xobject::page_operations), forms interpreted
//! in place — so the page rendered is the page extracted: the same page tree, box, rotation
//! and damaged-stream recovery. What this cannot paint yet is counted in [`RasterGaps`], the
//! way extraction counts what it could not read: the rest of the page is still painted.

use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, IntSize, LineCap, LineJoin,
    LinearGradient, Mask, MaskType, Paint, Path, PathBuilder, Pattern, Pixmap, PixmapPaint, Point,
    RadialGradient, Rect, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::rc::Rc;

use super::backend::{
    CodecParms, ContentOp, GlyphAdvance, ImageColorSpace, ObjectId, PageBox, PageId, PdfBackend,
    PdfValue, RawXObject, ResourceScope, Shading, ShadingGeometry, SoftMask, SoftMaskEntry,
    TilingPattern, TintSpace, Type3Font,
};
use super::cmyk::cmyk_to_rgb_unit;
use super::form_xobject;
use super::png_encode::{self, PngColorType};
use super::raster_text::LoadedFont;
use super::text_state::{TextMatrix, TextParams};
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
    /// Text runs whose paint or text clipping could not be fully applied: the font's program
    /// is not embedded and no standard face stands in for it, or is one this does not read, or a code
    /// selects no glyph in it (in a Type 3 font: no glyph procedure). Glyphs wholly outside
    /// the raster or its clip are excluded when their declared bounds are known.
    pub text_runs: u32,
    /// Images not painted: a codec this does not decode, a color
    /// space it does not convert, or data that did not decode.
    pub images: u32,
    /// Inline images (`BI … EI`) not painted: a dictionary that does not read. One that reads
    /// but is in a codec or color space this does not decode is counted in `images`.
    pub inline_images: u32,
    /// Shadings (`sh`) and pattern colors not painted: mesh shadings (types 4–7), function
    /// shadings (type 1), a PostScript calculator function, and a tiling pattern nested more
    /// than three deep. Axial and radial shadings and tiling patterns are painted.
    pub shadings: u32,
    /// Soft masks (`/SMask`) not applied as set: the mask group did not decode or is nested
    /// past three deep (what follows is painted unmasked), or its `/TR` transfer function,
    /// which this does not apply, was set (the mask is applied without it).
    pub soft_masks: u32,
    /// Content streams of the page that could not be decoded; whatever they held is missing.
    pub undecodable_content_streams: u32,
}

impl RasterGaps {
    /// Count what `other` left unpainted as left unpainted here too.
    fn absorb(&mut self, other: &RasterGaps) {
        self.text_runs += other.text_runs;
        self.images += other.images;
        self.inline_images += other.inline_images;
        self.shadings += other.shadings;
        self.soft_masks += other.soft_masks;
        self.undecodable_content_streams += other.undecodable_content_streams;
    }

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
    /// Text runs painted in a standard face standing in for a font the PDF does not embed
    /// (feature `standard-fonts`): readable, but not the page's own typeface. Not a gap —
    /// the text is there.
    pub substituted_text_runs: u32,
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

/// Rasterize page `page_num` of the document `backend` reads, charging `budget` as
/// [`PdfParser::render_page_with`](super::PdfParser::render_page_with) says.
pub(crate) fn render_page(
    backend: &dyn PdfBackend,
    page_num: u32,
    options: &RasterOptions,
    budget: &mut dyn FnMut(u64) -> ControlFlow<()>,
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

    if budget(u64::from(px_w) * u64::from(px_h)).is_break() {
        return Err(Error::RasterStopped);
    }
    let mut pixmap = Pixmap::new(px_w, px_h)
        .ok_or_else(|| Error::Render(format!("cannot allocate {px_w} x {px_h} pixels")))?;
    pixmap.fill(tiny_skia::Color::WHITE);

    let painted = form_xobject::page_operations(backend, page)?;
    let mut painter = Painter::new(backend, page, pixmap, base, budget);
    painter.gaps.undecodable_content_streams = painted.undecodable_streams as u32;
    for op in &painted.ops {
        painter.apply(op);
    }
    painter.end_groups();
    if painter.stopped {
        return Err(Error::RasterStopped);
    }

    let Painter {
        pixmap,
        gaps,
        substituted_text_runs,
        ..
    } = painter;
    // The page was filled opaque white first, so every pixel is opaque and premultiplied
    // equals straight color.
    Ok(RasteredPage {
        width: px_w,
        height: px_h,
        rgba: pixmap.take(),
        gaps,
        substituted_text_runs,
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
    /// A pattern, with the space an uncolored pattern's color is given in.
    Pattern(Option<Box<PaintSpace>>),
    /// A `Separation` space: a tint its transform maps into an alternate space.
    Spot(TintSpace),
    /// A space this does not convert (`DeviceN` of several colorants, `Lab`): its components
    /// are read as a tint, 0 white to 1 black.
    Tint(usize),
}

impl PaintSpace {
    fn color(&self, values: &[f32]) -> Option<Rgb> {
        let v = |i: usize| values.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        Some(match self {
            PaintSpace::Gray => Rgb(v(0), v(0), v(0)),
            PaintSpace::Rgb => Rgb(v(0), v(1), v(2)),
            PaintSpace::Cmyk => {
                let [r, g, b] = cmyk_to_rgb_unit(v(0), v(1), v(2), v(3));
                Rgb(r, g, b)
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
            PaintSpace::Pattern(base) => return base.as_ref()?.color(values),
            PaintSpace::Spot(space) => {
                let tints: Vec<f32> = (0..space.components()).map(v).collect();
                let (alternate, components) = space.alternate_color(&tints)?;
                PaintSpace::of_image_space(alternate).color(&components)?
            }
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
            ImageColorSpace::Tint(space) => PaintSpace::Spot(space.clone()),
        }
    }
}

/// A transparency group painted on a layer of its own: the pixmap underneath it, and the
/// alpha, blend mode and soft mask it is composited onto that with when it ends.
struct GroupLayer {
    under: Pixmap,
    alpha: f32,
    blend: BlendMode,
    soft_mask: Option<Rc<Mask>>,
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
    /// How what is painted combines with what is under it (`/BM`, ISO 32000-1 §11.3.5).
    blend: BlendMode,
    /// The soft mask (`/SMask`, §11.6.5.2) in effect, as a coverage mask over the pixmap, and
    /// the clip multiplied by it -- what painting is masked by while a soft mask is in effect.
    soft_mask: Option<Rc<Mask>>,
    masked_clip: Option<Rc<Mask>>,
    /// Whether the current fill/stroke color is a pattern.
    fill_pattern: bool,
    stroke_pattern: bool,
    /// What the current fill/stroke pattern paints with; `None` when it is a pattern this
    /// does not paint.
    fill_pattern_paint: Option<Rc<PatternPaint>>,
    stroke_pattern_paint: Option<Rc<PatternPaint>>,
    /// The default space of the content being painted → device pixels: what a pattern's
    /// matrix is relative to (ISO 32000-1 §8.7.3.1) — the page's, or, inside a form, the form's
    /// space as it was entered.
    pattern_base: Transform,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter: f32,
    dash: Option<(Rc<[f32]>, f32)>,
    /// The clip, as a coverage mask; `None` is the whole page. Shared by the states saved
    /// with it, so saving the state copies none of it.
    clip: Option<Rc<Mask>>,
    /// The text state (ISO 32000-1 §9.3) — part of the graphics state, saved by `q`.
    text: TextState,
}

/// The text state: the font `Tf` selects and the parameters around it.
#[derive(Clone, Default)]
struct TextState {
    /// The font resource `Tf` names, and the form whose resources it is looked up in.
    font: Option<(Option<ObjectId>, Vec<u8>)>,
    params: TextParams,
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
            blend: BlendMode::SourceOver,
            soft_mask: None,
            masked_clip: None,
            fill_pattern: false,
            stroke_pattern: false,
            fill_pattern_paint: None,
            stroke_pattern_paint: None,
            pattern_base: base,
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

/// A font resource: the form whose resources name it (`None`: the page's), and its name.
type FontResource = (Option<ObjectId>, Vec<u8>);

struct Painter<'a> {
    backend: &'a dyn PdfBackend,
    page: PageId,
    /// The form whose operations are being painted (`None`: the page's own).
    current_form: Option<ObjectId>,
    pixmap: Pixmap,
    state: GState,
    stack: Vec<GState>,
    /// The path under construction, in user space.
    path: PathBuilder,
    /// A clip set by `W`/`W*`, applied when the path is next painted or ended.
    pending_clip: Option<FillRule>,
    /// The outlines, in device space, of the glyphs the open text object shows in a clipping
    /// mode (4 to 7): they become the clip at `ET` (ISO 32000-1 §9.3.6, Table 106).
    text_clip: Option<PathBuilder>,
    /// The text matrix and text line matrix, inside `BT … ET`.
    text_matrix: TextMatrix,
    /// Fonts loaded so far, by the resource that names them; `None` when unpaintable.
    fonts: HashMap<FontResource, Option<LoadedFont>>,
    /// Type 3 fonts looked up so far, by the same key; `None` for a font of any other type.
    type3: HashMap<FontResource, Option<Rc<Type3Font>>>,
    /// How many Type 3 glyph procedures are running, one inside another.
    glyph_depth: usize,
    /// How many tiling pattern cells are being painted, one inside another: this painter's.
    pattern_depth: usize,
    /// Tiling pattern cells rendered so far, by where and in what color they are painted.
    tiles: HashMap<TileKey, Option<Rc<PatternPaint>>>,
    /// The transparency groups being painted, innermost last: for one painted on a layer of
    /// its own, the pixmap underneath it and the alpha it is composited with; `None` for one
    /// painted in place.
    groups: Vec<Option<GroupLayer>>,
    /// Painting a shape that takes its color from outside: a Type 3 glyph procedure that
    /// began with `d1`, or the cell of an uncolored tiling pattern. Its own color operators
    /// are ignored.
    uncolored: bool,
    gaps: RasterGaps,
    substituted_text_runs: u32,
    /// Charged, before each step of painting, the work the step does, in pixels.
    budget: &'a mut dyn FnMut(u64) -> ControlFlow<()>,
    /// The budget stopped the painting: nothing more is painted.
    stopped: bool,
}

/// A tiling pattern cell as painted: the content it was named from (form and name), its
/// pattern space → device pixels, and the color of an uncolored one -- as bits, to be a key.
type TileKey = (Option<ObjectId>, Vec<u8>, [u32; 6], [u32; 3]);

/// What a pattern color paints with, placed on the device.
enum PatternPaint {
    /// A shading pattern: its shading, and pattern space → device pixels.
    Shading(Shading, Transform),
    /// A tiling pattern: one cell, rendered, and its pixels → device pixels. The cell repeats.
    Tile(Pixmap, Transform),
}

/// The paint for one drawing operation. It owns what it paints with, so drawing with it does
/// not hold the painter borrowed.
enum PaintSource {
    Ready(Paint<'static>),
    /// A tiling pattern's cell, with its pixels → the space the drawing is in, its alpha and
    /// how it is blended.
    Tile(Rc<PatternPaint>, Transform, f32, BlendMode),
}

impl PaintSource {
    fn paint(&self) -> Paint<'_> {
        match self {
            PaintSource::Ready(paint) => paint.clone(),
            PaintSource::Tile(tile, transform, alpha, blend_mode) => {
                let PatternPaint::Tile(pixmap, _) = &**tile else {
                    unreachable!("a tile source holds a tile");
                };
                Paint {
                    shader: Pattern::new(
                        pixmap.as_ref(),
                        SpreadMode::Repeat,
                        FilterQuality::Bilinear,
                        alpha.clamp(0.0, 1.0),
                        *transform,
                    ),
                    anti_alias: true,
                    blend_mode: *blend_mode,
                    ..Paint::default()
                }
            }
        }
    }
}

impl<'a> Painter<'a> {
    /// A painter for `page` drawing into `pixmap`, user space placed on it by `base`, charging
    /// `budget`.
    fn new(
        backend: &'a dyn PdfBackend,
        page: PageId,
        pixmap: Pixmap,
        base: Transform,
        budget: &'a mut dyn FnMut(u64) -> ControlFlow<()>,
    ) -> Self {
        Painter {
            backend,
            page,
            current_form: None,
            pixmap,
            state: GState::new(base),
            stack: Vec::new(),
            path: PathBuilder::new(),
            pending_clip: None,
            text_clip: None,
            text_matrix: TextMatrix::default(),
            fonts: HashMap::new(),
            type3: HashMap::new(),
            glyph_depth: 0,
            pattern_depth: 0,
            tiles: HashMap::new(),
            groups: Vec::new(),
            uncolored: false,
            gaps: RasterGaps::default(),
            substituted_text_runs: 0,
            budget,
            stopped: false,
        }
    }

    /// Charge the budget `pixels` of work, at least one, before the step that does it: `false`,
    /// the step not taken, once it has stopped the painting.
    fn admit(&mut self, pixels: u64) -> bool {
        if !self.stopped {
            self.stopped = (self.budget)(pixels.max(1)).is_break();
        }
        !self.stopped
    }

    /// How many pixels of the pixmap `bounds`, in device pixels, reaches: all of them when the
    /// bounds are not known.
    fn pixels_within(&self, bounds: Option<Rect>) -> u64 {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let span = |from: f32, to: f32, len: u32| {
            (to.ceil().min(len as f32) - from.floor().max(0.0)).max(0.0) as u64
        };
        match bounds {
            Some(b) => span(b.left(), b.right(), w) * span(b.top(), b.bottom(), h),
            None => u64::from(w) * u64::from(h),
        }
    }

    /// [`Self::admit`] every pixel of the pixmap: a step over a whole layer or mask.
    fn admit_page(&mut self) -> bool {
        self.admit(self.pixels_within(None))
    }

    /// [`Self::admit`] the [`Self::path_work`] of a step that rasterizes `path`.
    fn admit_path(&mut self, path: &Path, transform: Transform) -> bool {
        self.admit(self.path_work(path, transform))
    }

    /// What rasterizing `path`, in the space `transform` maps to device pixels, walks: the
    /// pixels of the pixmap its bounds reach, and its edges.
    fn path_work(&self, path: &Path, transform: Transform) -> u64 {
        self.pixels_within(path.bounds().transform(transform))
            .saturating_add(self.edge_work(path, transform))
    }

    /// [`Self::admit`] what stroking `path` with `stroke`, in the space `transform` maps to
    /// device pixels, walks: the pixels its bounds reach, its edges, and at each point two
    /// outline edges, a join's or a cap's, across as many more rows as the line is wide where
    /// the CTM stretches it most, which each tile copies.
    fn admit_stroke(&mut self, path: &Path, stroke: &Stroke, transform: Transform) -> bool {
        let width = self.state.line_width * max_stretch(&self.state.ctm);
        let reach = (width.max(1.0).ceil() as u64).saturating_add(self.tiles());
        let outline = (path.points().len() as u64)
            .saturating_mul(2)
            .saturating_mul(reach);
        let work = self
            .pixels_within(stroke_bounds(path, stroke, transform))
            .saturating_add(self.edge_work(path, transform))
            .saturating_add(outline);
        self.admit(work)
    }

    /// What tiny-skia walks for `path`'s edges beside the pixels it covers: for each two
    /// consecutive points, the columns and rows of the pixmap between them, plus one; and every
    /// point once for each tile it draws the path in, a tile spanning up to 8191 pixels a side.
    fn edge_work(&self, path: &Path, transform: Transform) -> u64 {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let device = |&p: &Point| {
            let mut p = p;
            transform.map_point(&mut p);
            p
        };
        let span =
            |a: f32, b: f32, len: u32| (a.max(b).min(len as f32) - a.min(b).max(0.0)).max(0.0);
        let walked: f32 = path
            .points()
            .windows(2)
            .map(|pair| {
                let (a, b) = (device(&pair[0]), device(&pair[1]));
                span(a.x, b.x, w) + span(a.y, b.y, h) + 1.0
            })
            .sum();
        let points = path.points().len() as u64;
        (walked as u64).saturating_add(points.saturating_mul(self.tiles()))
    }

    /// How many tiles tiny-skia splits a drawing on the pixmap into: one, or more past 8191
    /// pixels a side.
    fn tiles(&self) -> u64 {
        const TILE_SIDE: u32 = 8191;
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        u64::from(w.div_ceil(TILE_SIDE)) * u64::from(h.div_ceil(TILE_SIDE))
    }
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
        if !self.admit(1) {
            return;
        }
        // A form's operations follow the `q`, `cm` and clip that place it, so where they start
        // the CTM is the form's space. (Leaving a form, the `Q` that follows restores the outer
        // state, this included.)
        if op.form != self.current_form {
            self.current_form = op.form;
            self.state.pattern_base = self.state.ctm;
        }
        // An uncolored glyph's procedure, or an uncolored pattern's cell, sets no color of its
        // own (ISO 32000-1 §9.6.5, §8.7.3.3).
        if self.uncolored
            && matches!(
                op.operator.as_str(),
                "g" | "G" | "rg" | "RG" | "k" | "K" | "cs" | "CS" | "sc" | "scn" | "SC" | "SCN"
            )
        {
            return;
        }
        let n = |i: usize| op.operands.get(i).and_then(number);
        let all = || op.operands.iter().filter_map(number).collect::<Vec<f32>>();
        // The text state and the text position, read as text extraction reads them.
        self.state.text.params.apply(op);
        self.text_matrix.apply(op, &self.state.text.params);
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
                let items = match op.operands.first() {
                    Some(PdfValue::Array(items)) => items.as_slice(),
                    _ => &[],
                };
                if !self.admit(items.len() as u64) {
                    return;
                }
                let array: Vec<f32> = items.iter().filter_map(number).collect();
                let phase = n(1).unwrap_or(0.0);
                self.state.dash = (!array.is_empty() && array.iter().any(|&v| v > 0.0))
                    .then(|| (array.into(), phase));
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
                        if let Some(mode) = gs.blend_mode {
                            self.state.blend = blend_mode(mode);
                        }
                        match &gs.soft_mask {
                            Some(SoftMaskEntry::None) => {
                                self.state.soft_mask = None;
                                self.refresh_masked_clip();
                            }
                            Some(SoftMaskEntry::Mask(mask)) => {
                                self.state.soft_mask = self.render_soft_mask(mask).map(Rc::new);
                                self.refresh_masked_clip();
                            }
                            None => {}
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
                    // Full tint.
                    PaintSpace::Spot(space) => vec![1.0; space.components()],
                    PaintSpace::Pattern(_) => Vec::new(),
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
                if self.state.fill_pattern {
                    self.state.fill_pattern_paint = self.pattern_named(op, self.state.fill);
                }
            }
            "SC" | "SCN" => {
                let space = self.state.stroke_space.clone();
                self.set_stroke(space, &all());
                if self.state.stroke_pattern {
                    self.state.stroke_pattern_paint = self.pattern_named(op, self.state.stroke);
                }
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
            "BI" => {
                let image = match op.operands.first() {
                    Some(PdfValue::Str(body)) => {
                        self.backend.inline_image(op.scope(self.page), body)
                    }
                    _ => None,
                };
                match image {
                    Some(image) => self.draw_image(&image),
                    None => self.gaps.inline_images += 1,
                }
            }
            form_xobject::BEGIN_GROUP => self.begin_group(),
            form_xobject::END_GROUP => self.end_group(),
            // Type 3 glyph metrics: `d1` marks the glyph uncolored.
            "d1" if self.glyph_depth > 0 => self.uncolored = true,
            "sh" => {
                let shading = match op.operands.first() {
                    Some(PdfValue::Name(name)) => self.backend.shading(op.scope(self.page), name),
                    _ => None,
                };
                match shading {
                    Some(shading) => self.paint_shading(&shading),
                    None => self.gaps.shadings += 1,
                }
            }

            // Text: the state and position were applied above; `'` and `"` have moved to
            // the next line, and `"` set its spacing.
            "Tf" => {
                if let Some(PdfValue::Name(name)) = op.operands.first() {
                    self.state.text.font = Some((op.form, name.clone()));
                }
            }
            "BT" => self.text_clip = None,
            "ET" => {
                if let Some(path) = self.text_clip.take() {
                    let path = path.finish();
                    self.clip_in(path.as_ref(), FillRule::Winding, Transform::identity());
                }
            }
            "Tj" | "'" => self.show(op.operands.get(..1).unwrap_or(&[])),
            "TJ" => {
                if let Some(PdfValue::Array(items)) = op.operands.first() {
                    self.show(items);
                }
            }
            "\"" => self.show(op.operands.get(2..3).unwrap_or(&[])),
            _ => {}
        }
    }

    /// Paint the strings and position adjustments of a text-showing operator, advancing the
    /// text matrix as text extraction does.
    fn show(&mut self, items: &[PdfValue]) {
        let Some(key) = self.state.text.font.clone() else {
            return;
        };
        let text = self.state.text.params.clone();
        let scope = ResourceScope {
            page: self.page,
            form: key.0,
        };
        if !self.type3.contains_key(&key) {
            let font = self.backend.type3_font(scope, &key.1).map(Rc::new);
            self.type3.insert(key.clone(), font);
        }
        if let Some(font) = self.type3.get(&key).cloned().flatten() {
            self.show_type3(&font, scope, &key.1, &text, items);
            return;
        }
        if !self.fonts.contains_key(&key) {
            let loaded = self
                .backend
                .font_program(scope, &key.1)
                .and_then(LoadedFont::load);
            self.fonts.insert(key.clone(), loaded);
        }
        let vertical = self.backend.is_vertical_font(scope, &key.1);
        let bounds = self.backend.font_bounds(scope, &key.1);
        // Mode 3 paints nothing (an OCR layer over a scan): nothing is missing.
        let invisible = text.render_mode == 3;
        let mut missing = false;
        let mut substituted = false;
        let mut unpainted_color = false;
        // Taken out of the cache while its glyphs are painted, put back after.
        let mut loaded = self.fonts.remove(&key).flatten();

        'run: for item in items {
            let codes = if let PdfValue::Str(bytes) = item {
                bytes.len() as u64
            } else {
                1
            };
            if !self.admit(codes) {
                break 'run;
            }
            let bytes = match item {
                PdfValue::Str(bytes) => bytes,
                PdfValue::Integer(_) | PdfValue::Real(_) => {
                    self.text_matrix
                        .advance(text.adjustment(number(item).unwrap_or(0.0), vertical));
                    continue;
                }
                _ => continue,
            };
            if (4..=7).contains(&text.render_mode) && !bytes.is_empty() {
                self.text_clip.get_or_insert_with(PathBuilder::new);
            }
            let advances = self.backend.glyph_advances(scope, &key.1, bytes);
            let Some(font) = loaded.as_mut() else {
                match &advances {
                    Some(advances) => {
                        let start = self.text_matrix.clone();
                        for measured in advances {
                            if self.stopped {
                                break 'run;
                            }
                            missing |= !invisible
                                && self.missing_glyph_visible(bounds, &text, measured, true);
                            self.text_matrix.advance(text.glyph_advance(measured));
                        }
                        self.text_matrix = start;
                        self.text_matrix.advance(text.advance_of(advances));
                    }
                    None => {
                        missing |= !invisible
                            && !bytes.is_empty()
                            && self.visible_bounds(None, (4..=7).contains(&text.render_mode))
                    }
                }
                continue;
            };
            let width = font.code_bytes();
            for (i, chunk) in bytes.chunks(width).enumerate() {
                if self.stopped {
                    break 'run;
                }
                let code = chunk.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b));
                let gid = font.glyph(code);
                // The width extraction measures with; the program's own when the font
                // dictionary declares none.
                let measured = match advances.as_ref().and_then(|a| a.get(i)) {
                    Some(g) => *g,
                    None => {
                        let units = gid.and_then(|g| font.program_advance(g)).unwrap_or(0.0);
                        GlyphAdvance {
                            width: units * font.glyph_to_text.sx * 1000.0,
                            vertical: None,
                            is_word_space: width == 1 && code == 32,
                        }
                    }
                };
                let w0 = measured.width;
                match gid {
                    // Mode 7 paints nothing but clips, so its outlines are still taken.
                    Some(gid) if text.render_mode != 3 => {
                        let mut glyph_space = font.glyph_to_text;
                        if font.stand_in {
                            // A stand-in face is drawn to the width the font gives the
                            // glyph, so the line keeps the length the page set.
                            let own = font.program_advance(gid).unwrap_or(0.0)
                                * font.glyph_to_text.sx
                                * 1000.0;
                            if own > 0.0 && w0 > 0.0 {
                                let fit = (w0 / own).clamp(0.5, 2.0);
                                glyph_space = glyph_space.post_scale(fit, 1.0);
                            }
                            substituted = true;
                        }
                        if let Some(outline) = font.outline(gid).cloned() {
                            let transform = self
                                .glyph_transform(&text, &measured)
                                .pre_concat(glyph_space);
                            unpainted_color |=
                                !self.paint_glyph(&outline, transform, text.render_mode);
                        }
                    }
                    Some(_) => {}
                    None => {
                        missing |= !invisible
                            && code != 32
                            && self.missing_glyph_visible(bounds, &text, &measured, true)
                    }
                }
                self.text_matrix.advance(text.glyph_advance(&measured));
            }
        }
        self.fonts.insert(key, loaded);
        if missing {
            self.gaps.text_runs += 1;
        }
        // Text in a pattern this does not paint, once per run.
        if unpainted_color {
            self.gaps.shadings += 1;
        }
        if substituted {
            self.substituted_text_runs += 1;
        }
    }

    fn glyph_transform(&self, text: &TextParams, measured: &GlyphAdvance) -> Transform {
        let origin = text.origin_offset(measured);
        let to_text = Transform::from_row(
            text.size * text.horizontal_scale,
            0.0,
            0.0,
            text.size,
            0.0,
            text.rise,
        );
        let [a, b, c, d, e, f] = self.text_matrix.tm;
        self.state
            .ctm
            .pre_concat(Transform::from_row(a, b, c, d, e, f))
            .pre_concat(Transform::from_translate(origin.tx, origin.ty))
            .pre_concat(to_text)
    }

    fn missing_glyph_visible(
        &mut self,
        bounds: Option<PageBox>,
        text: &TextParams,
        measured: &GlyphAdvance,
        follows_render_mode: bool,
    ) -> bool {
        let bounds = bounds
            .and_then(|b| Rect::from_ltrb(b.llx, b.lly, b.urx, b.ury))
            .and_then(|b| b.transform(self.glyph_transform(text, measured)))
            .and_then(|b| {
                let reach = if follows_render_mode && matches!(text.render_mode, 1 | 2 | 5 | 6) {
                    let width = self.state.line_width * transform_scale(&self.state.ctm);
                    width.max(1.0) / 2.0 * Stroke::default().miter_limit
                } else {
                    0.0
                } + 1.0;
                b.outset(reach, reach)
            });
        self.visible_bounds(
            bounds,
            follows_render_mode && (4..=7).contains(&text.render_mode),
        )
    }

    fn visible_bounds(&mut self, bounds: Option<Rect>, text_clipping: bool) -> bool {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let (x0, y0, x1, y1) = bounds.map_or((0, 0, w, h), |b| {
            (
                b.left().floor().clamp(0.0, w as f32) as u32,
                b.top().floor().clamp(0.0, h as f32) as u32,
                b.right().ceil().clamp(0.0, w as f32) as u32,
                b.bottom().ceil().clamp(0.0, h as f32) as u32,
            )
        });
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        let mask = if text_clipping {
            self.state.clip.as_ref()
        } else {
            self.state.masked_clip.as_ref().or(self.state.clip.as_ref())
        }
        .map(Rc::clone);
        let Some(mask) = mask else { return true };
        if !self.admit(u64::from(x1 - x0) * u64::from(y1 - y0)) {
            return false;
        }
        (y0..y1).any(|y| {
            let start = y as usize * w as usize;
            mask.data()[start + x0 as usize..start + x1 as usize]
                .iter()
                .any(|&coverage| coverage != 0)
        })
    }

    /// Show strings in a Type 3 font: each code's glyph procedure runs where the glyph goes,
    /// then the text position advances by the glyph's width.
    fn show_type3(
        &mut self,
        font: &Type3Font,
        scope: super::backend::ResourceScope,
        font_name: &[u8],
        text: &TextParams,
        items: &[PdfValue],
    ) {
        let bounds = self.backend.font_bounds(scope, font_name);
        let mut missing = false;
        for item in items {
            let codes = if let PdfValue::Str(bytes) = item {
                bytes.len() as u64
            } else {
                1
            };
            if !self.admit(codes) {
                return;
            }
            let bytes = match item {
                PdfValue::Str(bytes) => bytes,
                PdfValue::Integer(_) | PdfValue::Real(_) => {
                    self.text_matrix
                        .advance(text.adjustment(number(item).unwrap_or(0.0), false));
                    continue;
                }
                _ => continue,
            };
            let advances = self.backend.glyph_advances(scope, font_name, bytes);
            for (i, &code) in bytes.iter().enumerate() {
                if self.stopped {
                    return;
                }
                let measured =
                    advances
                        .as_ref()
                        .and_then(|a| a.get(i).copied())
                        .unwrap_or(GlyphAdvance {
                            width: 0.0,
                            vertical: None,
                            is_word_space: code == 32,
                        });
                match font.glyphs.get(&code) {
                    Some(ops) => self.run_glyph(ops, font.matrix, text, &measured),
                    None => {
                        missing |=
                            code != 32 && self.missing_glyph_visible(bounds, text, &measured, false)
                    }
                }
                self.text_matrix.advance(text.glyph_advance(&measured));
            }
        }
        if missing {
            self.gaps.text_runs += 1;
        }
    }

    /// Run one Type 3 glyph procedure with glyph space placed at the current text position.
    /// What the procedure does to the graphics, text and path state stays inside it.
    fn run_glyph(
        &mut self,
        ops: &[ContentOp],
        matrix: [f32; 6],
        text: &TextParams,
        measured: &GlyphAdvance,
    ) {
        // A glyph that shows text in a Type 3 font that shows text ... ends here.
        const MAX_GLYPH_DEPTH: usize = 4;
        if self.glyph_depth >= MAX_GLYPH_DEPTH {
            return;
        }
        let [ma, mb, mc, md, me, mf] = matrix;
        let glyph_space = self
            .glyph_transform(text, measured)
            .pre_concat(Transform::from_row(ma, mb, mc, md, me, mf));

        let saved_state = self.state.clone();
        let saved_depth = self.stack.len();
        let saved_path = std::mem::replace(&mut self.path, PathBuilder::new());
        let saved_clip = self.pending_clip.take();
        let saved_text = self.text_matrix.clone();
        let saved_form = self.current_form;
        let saved_uncolored = std::mem::replace(&mut self.uncolored, false);
        self.state.ctm = glyph_space;
        self.glyph_depth += 1;
        for op in ops {
            self.apply(op);
        }
        self.glyph_depth -= 1;
        self.stack.truncate(saved_depth);
        self.state = saved_state;
        self.path = saved_path;
        self.pending_clip = saved_clip;
        self.text_matrix = saved_text;
        self.current_form = saved_form;
        self.uncolored = saved_uncolored;
    }

    /// A transparency group begins (ISO 32000-1 §11.6.6). A group painted with an alpha below
    /// one, with a blend mode or under a soft mask goes on a layer of its own, which is
    /// composited with that alpha, blend mode and mask when it ends -- so overlapping things
    /// inside it do not show through one another, and an alpha the group sets inside itself
    /// does not undo the one it is painted with. Inside, the alpha starts at one, the blend
    /// mode at Normal and the soft mask at none (a soft mask the group's own content sets
    /// does not undo the one it is painted with either). A
    /// fully opaque, normally blended group is painted in place, which is the same picture;
    /// so is one nested past `MAX_GROUP_LAYERS`, to bound the memory layers take.
    fn begin_group(&mut self) {
        const MAX_GROUP_LAYERS: usize = 4;
        let alpha = self.state.fill_alpha;
        let blend = self.state.blend;
        let soft_mask = self.state.soft_mask.clone();
        let layered = self.groups.iter().filter(|g| g.is_some()).count();
        let own_layer = (alpha < 1.0 || blend != BlendMode::SourceOver || soft_mask.is_some())
            && layered < MAX_GROUP_LAYERS;
        let layer = (own_layer && self.admit_page())
            .then(|| Pixmap::new(self.pixmap.width(), self.pixmap.height()))
            .flatten();
        match layer {
            Some(layer) => {
                let under = std::mem::replace(&mut self.pixmap, layer);
                self.groups.push(Some(GroupLayer {
                    under,
                    alpha,
                    blend,
                    soft_mask,
                }));
                self.state.fill_alpha = 1.0;
                self.state.stroke_alpha = 1.0;
                self.state.blend = BlendMode::SourceOver;
                // The soft mask applies to the group's result, not to what is painted inside.
                self.state.soft_mask = None;
                self.refresh_masked_clip();
            }
            None => self.groups.push(None),
        }
    }

    /// The innermost transparency group ends: its layer, if it has one, is composited onto
    /// what is underneath with the group's alpha.
    fn end_group(&mut self) {
        if let Some(Some(GroupLayer {
            under,
            alpha,
            blend,
            soft_mask,
        })) = self.groups.pop()
        {
            let group = std::mem::replace(&mut self.pixmap, under);
            if !self.admit_page() {
                return;
            }
            self.pixmap.draw_pixmap(
                0,
                0,
                group.as_ref(),
                &PixmapPaint {
                    opacity: alpha,
                    blend_mode: blend,
                    ..PixmapPaint::default()
                },
                Transform::identity(),
                soft_mask.as_deref(),
            );
        }
    }

    /// End every group still open, as the content's end ends them.
    fn end_groups(&mut self) {
        while !self.groups.is_empty() {
            self.end_group();
        }
    }

    /// Paint one glyph outline in text render mode `mode` (fill, stroke, or both). `false` when
    /// part of it was not painted: its color is a pattern this does not paint.
    fn paint_glyph(&mut self, outline: &Path, transform: Transform, mode: i64) -> bool {
        if (4..=7).contains(&mode) && self.admit(outline.points().len() as u64) {
            if let Some(device) = outline.clone().transform(transform) {
                self.text_clip
                    .get_or_insert_with(PathBuilder::new)
                    .push_path(&device);
            }
        }
        let fill = matches!(mode, 0 | 2 | 4 | 6);
        let stroke = matches!(mode, 1 | 2 | 5 | 6);
        let mut painted = true;
        if fill {
            match self.current_paint(false, transform) {
                Some(source) => {
                    if self.admit_path(outline, transform) {
                        self.pixmap.fill_path(
                            outline,
                            &source.paint(),
                            FillRule::Winding,
                            transform,
                            active_clip(&self.state),
                        );
                    }
                }
                None => painted = false,
            }
        }
        if stroke {
            // The line width is in user space; the glyph path is in glyph space. Stroke in
            // device space with the width the CTM gives it.
            let Some(source) = self.current_paint(true, Transform::identity()) else {
                return false;
            };
            let paint = source.paint();
            if let Some(device) = outline.clone().transform(transform) {
                let width = self.state.line_width * transform_scale(&self.state.ctm);
                let stroke = Stroke {
                    width: if width < 1.0 { 0.0 } else { width },
                    ..Stroke::default()
                };
                if self.admit_stroke(&device, &stroke, Transform::identity()) {
                    self.pixmap.stroke_path(
                        &device,
                        &paint,
                        &stroke,
                        Transform::identity(),
                        active_clip(&self.state),
                    );
                }
            }
        }
        painted
    }

    fn space_named(&self, op: &ContentOp, name: &[u8]) -> PaintSpace {
        if let Some(base) = self.backend.named_pattern_space(op.scope(self.page), name) {
            return PaintSpace::Pattern(base.map(|b| Box::new(PaintSpace::of_image_space(&b))));
        }
        match name {
            b"DeviceGray" | b"G" | b"CalGray" => PaintSpace::Gray,
            b"DeviceRGB" | b"RGB" | b"CalRGB" => PaintSpace::Rgb,
            b"DeviceCMYK" | b"CMYK" => PaintSpace::Cmyk,
            b"Pattern" => PaintSpace::Pattern(None),
            _ => match self.backend.named_color_space(op.scope(self.page), name) {
                Some(space) => PaintSpace::of_image_space(&space),
                // `Separation`, `DeviceN`, `Lab`, or a `Pattern` space with a base: read as
                // a tint, so separations still come out dark where they are inked.
                None => PaintSpace::Tint(1),
            },
        }
    }

    fn set_fill(&mut self, space: PaintSpace, values: &[f32]) {
        self.state.fill_pattern = matches!(space, PaintSpace::Pattern(_));
        self.state.fill_pattern_paint = None;
        if let Some(color) = space.color(values) {
            self.state.fill = color;
        }
        self.state.fill_space = space;
    }

    fn set_stroke(&mut self, space: PaintSpace, values: &[f32]) {
        self.state.stroke_pattern = matches!(space, PaintSpace::Pattern(_));
        self.state.stroke_pattern_paint = None;
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
                match self.current_paint(false, self.state.ctm) {
                    Some(source) => {
                        if self.admit_path(path, self.state.ctm) {
                            self.pixmap.fill_path(
                                path,
                                &source.paint(),
                                rule,
                                self.state.ctm,
                                active_clip(&self.state),
                            );
                        }
                    }
                    None => self.gaps.shadings += 1,
                }
            }
            if stroke {
                match self.current_paint(true, self.state.ctm) {
                    Some(source) => self.stroke(path, &source.paint()),
                    None => self.gaps.shadings += 1,
                }
            }
        }
        if let Some(rule) = self.pending_clip.take() {
            self.clip(path.as_ref(), rule);
        }
    }

    /// What the pattern `scn`/`SCN` names (its last operand) paints with, placed by its matrix
    /// relative to the default space of the content being painted. `color` is the color given
    /// with it, which an uncolored tiling pattern paints in.
    fn pattern_named(&mut self, op: &ContentOp, color: Rgb) -> Option<Rc<PatternPaint>> {
        let Some(PdfValue::Name(name)) = op.operands.last() else {
            return None;
        };
        let scope = op.scope(self.page);
        let place = |[a, b, c, d, e, f]: [f32; 6]| {
            self.state
                .pattern_base
                .pre_concat(Transform::from_row(a, b, c, d, e, f))
        };
        if let Some((shading, matrix)) = self.backend.shading_pattern(scope, name) {
            return Some(Rc::new(PatternPaint::Shading(shading, place(matrix))));
        }
        let pattern = self.backend.tiling_pattern(scope, name)?;
        let to_device = place(pattern.matrix);
        let t = to_device;
        let key = (
            op.form,
            name.clone(),
            [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f32::to_bits),
            [color.0, color.1, color.2].map(f32::to_bits),
        );
        if let Some(tile) = self.tiles.get(&key) {
            return tile.clone();
        }
        let tile = self
            .render_tile(&pattern, to_device, color)
            .map(|(pixmap, to_device)| Rc::new(PatternPaint::Tile(pixmap, to_device)));
        self.tiles.insert(key, tile.clone());
        tile
    }

    /// Paint one cell of a tiling pattern into a pixmap of its own, at the resolution the
    /// device shows it with, and say how its pixels land on the device.
    ///
    /// The cell is `x_step` by `y_step` from the `/BBox`'s lower left corner, clipped to the
    /// `/BBox`, as pdf.js paints it: what a cell paints past its step is cut off rather than
    /// overlapping the next. The cell's resolution is capped, as pdf.js caps it (3000 pixels a
    /// side); a coarser cell is still repeated at its exact step.
    fn render_tile(
        &mut self,
        pattern: &TilingPattern,
        to_device: Transform,
        color: Rgb,
    ) -> Option<(Pixmap, Transform)> {
        const MAX_PATTERN_DEPTH: usize = 3;
        const MAX_TILE_SIDE: f32 = 3000.0;
        if self.pattern_depth >= MAX_PATTERN_DEPTH {
            return None;
        }
        let Some(ops) = &pattern.ops else {
            self.gaps.undecodable_content_streams += 1;
            return None;
        };
        let (x_step, y_step) = (pattern.x_step.abs(), pattern.y_step.abs());
        if !(x_step.is_finite() && y_step.is_finite() && x_step > 0.0 && y_step > 0.0) {
            return None;
        }
        // Device pixels per pattern unit along each axis of pattern space.
        let along_x = to_device.sx.hypot(to_device.ky);
        let along_y = to_device.kx.hypot(to_device.sy);
        let side = |len: f32| {
            if len.is_finite() {
                len.ceil().clamp(1.0, MAX_TILE_SIDE)
            } else {
                1.0
            }
        };
        let (w, h) = (side(x_step * along_x), side(y_step * along_y));
        // The cell's pixels: pattern space scaled to fill them exactly, y down.
        let (kx, ky) = (w / x_step, h / y_step);
        let (x0, y0) = (pattern.bbox.llx, pattern.bbox.lly);
        let cell = Transform::from_row(kx, 0.0, 0.0, -ky, -x0 * kx, (y0 + y_step) * ky);
        let tile_to_device = to_device.pre_concat(cell.invert()?);

        if !self.admit(u64::from(w as u32) * u64::from(h as u32)) {
            return None;
        }
        let pixmap = Pixmap::new(w as u32, h as u32)?;
        let mut tile = Painter::new(self.backend, self.page, pixmap, cell, &mut *self.budget);
        tile.current_form = Some(pattern.id);
        tile.pattern_depth = self.pattern_depth + 1;
        tile.glyph_depth = self.glyph_depth;
        if pattern.uncolored {
            tile.uncolored = true;
            tile.state.fill = color;
            tile.state.stroke = color;
        }
        // The cell clips what it paints (ISO 32000-1 §8.7.3.3).
        let b = &pattern.bbox;
        let bbox = Rect::from_ltrb(
            b.llx.min(b.urx),
            b.lly.min(b.ury),
            b.llx.max(b.urx),
            b.lly.max(b.ury),
        );
        if let Some(bbox) = bbox {
            tile.clip(Some(&PathBuilder::from_rect(bbox)), FillRule::Winding);
        }
        let cell_ops = form_xobject::content_operations(self.backend, self.page, ops, pattern.id);
        tile.gaps.undecodable_content_streams += cell_ops.undecodable_streams as u32;
        for op in &cell_ops.ops {
            tile.apply(op);
        }
        tile.end_groups();
        if tile.stopped {
            self.stopped = true;
            return None;
        }
        self.gaps.absorb(&tile.gaps);
        self.substituted_text_runs += tile.substituted_text_runs;
        Some((tile.pixmap, tile_to_device))
    }

    /// The paint for filling (`stroke` false) or stroking with the current color, drawn with
    /// `path_transform` (the CTM for a path, the glyph's transform for a glyph). That transform
    /// applies to a pattern's shader too, so the shader is placed by its inverse followed by
    /// the pattern's own placement on the device. `None` when the color is a pattern this does
    /// not paint.
    fn current_paint(&self, stroke: bool, path_transform: Transform) -> Option<PaintSource> {
        let s = &self.state;
        let (pattern, pattern_paint, color, alpha) = if stroke {
            (
                s.stroke_pattern,
                &s.stroke_pattern_paint,
                s.stroke,
                s.stroke_alpha,
            )
        } else {
            (s.fill_pattern, &s.fill_pattern_paint, s.fill, s.fill_alpha)
        };
        let blend = s.blend;
        if !pattern {
            let mut paint = solid(color, alpha);
            paint.blend_mode = blend;
            return Some(PaintSource::Ready(paint));
        }
        let pattern_paint = pattern_paint.as_ref()?;
        let to_path = path_transform.invert()?;
        match &**pattern_paint {
            PatternPaint::Shading(shading, to_device) => {
                shading_paint(shading, to_path.pre_concat(*to_device), alpha).map(|mut paint| {
                    paint.blend_mode = blend;
                    PaintSource::Ready(paint)
                })
            }
            PatternPaint::Tile(_, to_device) => Some(PaintSource::Tile(
                pattern_paint.clone(),
                to_path.pre_concat(*to_device),
                alpha,
                blend,
            )),
        }
    }

    /// `sh`: paint a shading over the clip — or over its `/BBox` — in the current user space,
    /// which is the shading's space.
    fn paint_shading(&mut self, shading: &Shading) {
        // The rectangle painted and the transform it is drawn with — which the shader is
        // drawn with too: in user space the shader needs no transform of its own, in device
        // pixels it needs the CTM.
        let (rect, transform, shader_transform) = match shading.bbox {
            Some([x0, y0, x1, y1]) => (
                Rect::from_ltrb(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)),
                self.state.ctm,
                Transform::identity(),
            ),
            None => {
                let (w, h) = (self.pixmap.width() as f32, self.pixmap.height() as f32);
                (
                    Rect::from_xywh(0.0, 0.0, w, h),
                    Transform::identity(),
                    self.state.ctm,
                )
            }
        };
        let paint = shading_paint(shading, shader_transform, self.state.fill_alpha).map(|mut p| {
            p.blend_mode = self.state.blend;
            p
        });
        let (Some(rect), Some(paint)) = (rect, paint) else {
            self.gaps.shadings += 1;
            return;
        };
        if !self.admit_path(&PathBuilder::from_rect(rect), transform) {
            return;
        }
        self.pixmap
            .fill_rect(rect, &paint, transform, active_clip(&self.state));
    }

    fn stroke(&mut self, path: &Path, paint: &Paint) {
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
        if let Some((array, phase)) = self.state.dash.clone() {
            if !self.admit(2 * array.len() as u64) {
                return;
            }
            let mut array = array.to_vec();
            if array.len() % 2 == 1 {
                array.extend_from_within(..);
            }
            let wide = (self.state.line_width * max_stretch(&self.state.ctm)).max(1.0) as u64;
            let dash = wide.saturating_mul(2).saturating_add(4);
            let laid = dashes(path, &array).saturating_mul(dash);
            if !self.admit(laid.saturating_mul(self.tiles())) {
                return;
            }
            stroke.dash = StrokeDash::new(array, phase);
        }
        if !self.admit_stroke(path, &stroke, self.state.ctm) {
            return;
        }
        self.pixmap.stroke_path(
            path,
            paint,
            &stroke,
            self.state.ctm,
            active_clip(&self.state),
        );
    }

    /// Intersect the clip with `path` (no path: an empty clip).
    fn clip(&mut self, path: Option<&Path>, rule: FillRule) {
        self.clip_in(path, rule, self.state.ctm);
    }

    /// [`Self::clip`] with `path` in the space `transform` maps to device pixels.
    fn clip_in(&mut self, path: Option<&Path>, rule: FillRule, transform: Transform) {
        let edges = path.map_or(0, |path| self.edge_work(path, transform));
        if !self.admit(self.pixels_within(None).saturating_add(edges)) {
            return;
        }
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Some(path) = path else {
            self.state.clip = Mask::new(w, h).map(Rc::new);
            self.refresh_masked_clip();
            return;
        };
        match &mut self.state.clip {
            Some(mask) => Rc::make_mut(mask).intersect_path(path, rule, true, transform),
            None => {
                if let Some(mut mask) = Mask::new(w, h) {
                    mask.fill_path(path, rule, true, transform);
                    self.state.clip = Some(Rc::new(mask));
                }
            }
        }
        self.refresh_masked_clip();
    }

    /// Recompute what painting is masked by: the clip multiplied by the soft mask in effect
    /// (nothing, when there is none).
    fn refresh_masked_clip(&mut self) {
        if self.state.soft_mask.is_some() && !self.admit_page() {
            return;
        }
        self.state.masked_clip = self.state.soft_mask.as_ref().map(|soft| {
            let mut combined = (**soft).clone();
            if let Some(clip) = &self.state.clip {
                for (m, c) in combined.data_mut().iter_mut().zip(clip.data()) {
                    *m = ((u16::from(*m) * u16::from(*c) + 127) / 255) as u8;
                }
            }
            Rc::new(combined)
        });
    }

    /// The soft mask `mask` makes, as coverage over the pixmap: its group painted with the
    /// coordinate system in effect now (ISO 32000-1 §11.6.5.2), over its backdrop for a
    /// luminosity mask, then read as luminosity or alpha. `None` -- and what follows painted
    /// unmasked -- when the group cannot be painted.
    fn render_soft_mask(&mut self, mask: &SoftMask) -> Option<Mask> {
        const MAX_MASK_DEPTH: usize = 3;
        if self.pattern_depth >= MAX_MASK_DEPTH {
            self.gaps.soft_masks += 1;
            return None;
        }
        let group = &mask.group;
        let Some(ops) = group
            .content
            .as_deref()
            .and_then(|data| self.backend.decode_content(data).ok())
        else {
            self.gaps.soft_masks += 1;
            return None;
        };
        let m = group.matrix;
        let to_device = self
            .state
            .ctm
            .pre_concat(Transform::from_row(m[0], m[1], m[2], m[3], m[4], m[5]));
        if !self.admit_page() {
            return None;
        }
        let mut pixmap = Pixmap::new(self.pixmap.width(), self.pixmap.height())?;
        if mask.luminosity {
            let [r, g, b] = mask.backdrop;
            if let Some(backdrop) =
                Color::from_rgba(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), 1.0)
            {
                pixmap.fill(backdrop);
            }
        }
        let mut sub = Painter::new(
            self.backend,
            self.page,
            pixmap,
            to_device,
            &mut *self.budget,
        );
        sub.current_form = Some(group.id);
        sub.pattern_depth = self.pattern_depth + 1;
        sub.glyph_depth = self.glyph_depth;
        if let Some(b) = &group.bbox {
            let rect = Rect::from_ltrb(
                b.llx.min(b.urx),
                b.lly.min(b.ury),
                b.llx.max(b.urx),
                b.lly.max(b.ury),
            );
            if let Some(rect) = rect {
                sub.clip(Some(&PathBuilder::from_rect(rect)), FillRule::Winding);
            }
        }
        let content = form_xobject::content_operations(self.backend, self.page, &ops, group.id);
        sub.gaps.undecodable_content_streams += content.undecodable_streams as u32;
        for op in &content.ops {
            sub.apply(op);
        }
        sub.end_groups();
        if sub.stopped {
            self.stopped = true;
            return None;
        }
        self.gaps.absorb(&sub.gaps);
        self.substituted_text_runs += sub.substituted_text_runs;
        if mask.transfer_function {
            self.gaps.soft_masks += 1;
        }
        Some(Mask::from_pixmap(
            sub.pixmap.as_ref(),
            if mask.luminosity {
                MaskType::Luminance
            } else {
                MaskType::Alpha
            },
        ))
    }

    fn draw_image(&mut self, image: &RawXObject) {
        let unit = Rect::from_ltrb(0.0, 0.0, 1.0, 1.0).expect("the unit square is a rect");
        let drawn = self.path_work(&PathBuilder::from_rect(unit), self.state.ctm);
        if !self.admit(drawn.saturating_add(decode_work(image))) {
            return;
        }
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
            blend_mode: self.state.blend,
            quality: FilterQuality::Bilinear,
        };
        self.pixmap.draw_pixmap(
            0,
            0,
            pixmap.as_ref(),
            &paint,
            transform,
            active_clip(&self.state),
        );
    }

    /// An image as a premultiplied pixmap: its samples, its soft mask as alpha, or — a
    /// stencil mask — the fill color where the stencil marks.
    fn image_pixmap(&self, image: &RawXObject) -> Option<Pixmap> {
        let (w, h) = (image.width?, image.height?);
        if w == 0 || h == 0 || !png_encode::decodable(w, h, 4) {
            return None;
        }
        let mut rgba: Vec<u8> = if image.image_mask {
            let samples = plain_samples(image, w, h)?;
            // 0 marks, unless `/Decode [1 0]` inverts it.
            let marks_on_one = image
                .decode
                .as_deref()
                .is_some_and(|d| d.first() == Some(&1.0));
            let (_, bits) =
                png_encode::image_pixels(w, h, 1, &ImageColorSpace::Gray, None, &samples)?;
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

        // The image's soft mask gives each pixel's opacity. It covers the same unit square at
        // a resolution of its own (ISO 32000-1 §11.6.5.3), so each pixel takes the mask sample
        // its centre falls on.
        if let Some((mw, mh, alpha)) = image.smask.as_deref().and_then(mask_alpha) {
            let pixels = rgba.as_chunks_mut::<4>().0;
            for y in 0..h as usize {
                let row = (2 * y + 1) * mh as usize / (2 * h as usize) * mw as usize;
                for x in 0..w as usize {
                    let col = (2 * x + 1) * mw as usize / (2 * w as usize);
                    pixels[y * w as usize + x][3] = alpha[row + col];
                }
            }
        }
        // Color key masking: a pixel whose samples all lie in the key's ranges is not painted.
        if let Some(hits) = color_key_hits(image, w, h) {
            for (px, hit) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(hits) {
                if hit {
                    px[3] = 0;
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

/// An image's soft mask as `(width, height, opacity per pixel)`. A JPEG decodes to RGB
/// whatever its components, so a gray mask coded as one has equal channels: the first stands
/// for the pixel.
fn mask_alpha(smask: &RawXObject) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = (smask.width?, smask.height?);
    if w == 0 || h == 0 || !png_encode::decodable(w, h, 4) {
        return None;
    }
    let alpha = match decode_samples(smask, w, h)? {
        (PngColorType::Gray, samples) => samples,
        (PngColorType::Rgb, samples) => samples.as_chunks::<3>().0.iter().map(|p| p[0]).collect(),
    };
    (alpha.len() == w as usize * h as usize).then_some((w, h, alpha))
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
        Some("DCTDecode") => {
            // A gray JPEG in a one-component space (gray, or a tint) holds that space's
            // samples, `/Decode` included -- not brightnesses.
            if let Some(color) = image
                .color
                .as_ref()
                .filter(|c| c.components() == 1 && !matches!(c, ImageColorSpace::Indexed { .. }))
            {
                if let Some(samples) = decode_jpeg_luma(&image.data, w, h) {
                    return png_encode::image_pixels(
                        w,
                        h,
                        8,
                        color,
                        image.decode.as_deref(),
                        &samples,
                    );
                }
            }
            return decode_jpeg(&image.data, w, h, image.decode.as_deref());
        }
        Some("JPXDecode") => return decode_jpx(image, w, h),
        _ => {}
    }
    let samples = plain_samples(image, w, h)?;
    png_encode::image_pixels(
        w,
        h,
        image.bits_per_component?,
        image.color.as_ref()?,
        image.decode.as_deref(),
        &samples,
    )
}

/// A JPEG 2000 image's pixels. Its codestream says its size and, when the image dictionary
/// gives no `/ColorSpace`, its color space; `/Decode` is ignored (ISO 32000-1 §8.9.5.3). A
/// codestream of another size than the dictionary declares is not drawn, as other decoders
/// fail it, and an alpha channel is dropped (`/SMaskInData` is not read).
fn decode_jpx(image: &RawXObject, w: u32, h: u32) -> Option<(PngColorType, Vec<u8>)> {
    let decoded = super::jpx::decode(&image.data, image.color.as_ref(), |width, height| {
        (width, height) == (w, h)
    })?;
    png_encode::image_pixels(w, h, 8, &decoded.color, None, &decoded.samples)
}

/// The image's samples as an image without a codec stores them: its data, or the 1-bit
/// samples a bi-level codec (CCITT, JBIG2) decodes to. `None` for any other codec.
fn plain_samples(image: &RawXObject, w: u32, h: u32) -> Option<std::borrow::Cow<'_, [u8]>> {
    use std::borrow::Cow;
    match (image.filter.as_deref(), &image.codec_parms) {
        (None, _) => Some(Cow::Borrowed(&image.data)),
        (Some("CCITTFaxDecode"), Some(CodecParms::Ccitt(parms))) => {
            super::ccitt::decode(&image.data, parms, w, h).map(Cow::Owned)
        }
        (Some("JBIG2Decode"), Some(CodecParms::Jbig2 { globals })) => {
            super::jbig2::decode(&image.data, globals.as_deref(), w, h).map(Cow::Owned)
        }
        _ => None,
    }
}

/// Decode a JPEG to gray or RGB pixels of the size the image dictionary declares.
///
/// A four-component JPEG (`DeviceCMYK`) is read as the ink amounts it holds, `/Decode` applied
/// (`[1 0 1 0 1 0 1 0]` is how a PDF says Photoshop's inverted CMYK is to be read), and
/// converted like every other CMYK source. YCCK is first turned into
/// CMYK the way libjpeg does (`C = 255 - R` of the YCbCr triple), so an Adobe-marked file
/// with no `/Decode` -- what many layout programs write -- keeps its light and dark.
fn decode_jpeg(
    data: &[u8],
    w: u32,
    h: u32,
    decode: Option<&[f32]>,
) -> Option<(PngColorType, Vec<u8>)> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    // The dictionary's size bounds what the decoder may allocate: a JPEG whose own header says
    // more is refused before its pixels are, not after.
    if !png_encode::decodable(w, h, 4) {
        return None;
    }
    let options = |out: ColorSpace| {
        DecoderOptions::default()
            .jpeg_set_out_colorspace(out)
            .set_max_width(w as usize)
            .set_max_height(h as usize)
    };
    let mut decoder =
        zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options(ColorSpace::RGB));
    decoder.decode_headers().ok()?;
    let input = decoder.input_colorspace()?;
    let four = matches!(input, ColorSpace::CMYK | ColorSpace::YCCK);
    if four {
        decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options(input));
    }
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    if (u32::from(info.width), u32::from(info.height)) != (w, h) {
        return None;
    }
    let count = w as usize * h as usize;
    if !four {
        return (pixels.len() == count * 3).then_some((PngColorType::Rgb, pixels));
    }
    if pixels.len() != count * 4 {
        return None;
    }
    // `/Decode` pairs per ink; the default `[0 1]` leaves a sample as it is.
    let pair = |i: usize| match decode {
        Some(d) if d.len() >= 8 => (d[2 * i], d[2 * i + 1]),
        _ => (0.0, 1.0),
    };
    let pairs = [pair(0), pair(1), pair(2), pair(3)];
    let mut rgb = Vec::with_capacity(count * 3);
    for px in pixels.as_chunks::<4>().0 {
        let ink: [f32; 4] = if input == ColorSpace::YCCK {
            let (y, cb, cr) = (
                f32::from(px[0]),
                f32::from(px[1]) - 128.0,
                f32::from(px[2]) - 128.0,
            );
            let r = y + 1.402 * cr;
            let g = y - 0.344_136 * cb - 0.714_136 * cr;
            let b = y + 1.772 * cb;
            [255.0 - r, 255.0 - g, 255.0 - b, f32::from(px[3])]
        } else {
            px.map(f32::from)
        };
        let amounts = [0, 1, 2, 3].map(|i| ink_amount(ink[i], pairs[i]));
        let [r, g, b] = cmyk_to_rgb_unit(amounts[0], amounts[1], amounts[2], amounts[3]);
        rgb.extend([r, g, b].map(|v| (v * 255.0).round() as u8));
    }
    Some((PngColorType::Rgb, rgb))
}

/// Which pixels the image's `/Mask` color key hides, `None` when it has none (or the samples
/// are not plain ones: a JPEG's are masked after decoding, which no producer writes).
fn color_key_hits(image: &RawXObject, w: u32, h: u32) -> Option<Vec<bool>> {
    let key = image.color_key.as_deref()?;
    let bpc = usize::from(image.bits_per_component?);
    let components = key.len() / 2;
    if components == 0 || key.len() % 2 != 0 || !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
        return None;
    }
    let samples = plain_samples(image, w, h)?;
    let row_bytes = (w as usize * components * bpc).div_ceil(8);
    let sample = |row: &[u8], index: usize| -> Option<i64> {
        let bit = index * bpc;
        let byte = *row.get(bit / 8)?;
        Some(match bpc {
            16 => i64::from(byte) << 8 | i64::from(*row.get(bit / 8 + 1)?),
            8 => i64::from(byte),
            _ => i64::from((byte >> (8 - bpc - bit % 8)) & ((1 << bpc) - 1)),
        })
    };
    let mut hits = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h as usize {
        let row = samples.get(y * row_bytes..(y + 1) * row_bytes)?;
        for x in 0..w as usize {
            let mut inside = true;
            for c in 0..components {
                let v = sample(row, x * components + c)?;
                inside &= key[2 * c] <= v && v <= key[2 * c + 1];
            }
            hits.push(inside);
        }
    }
    Some(hits)
}

/// The samples of a one-component JPEG, `None` for any other.
fn decode_jpeg_luma(data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    if !png_encode::decodable(w, h, 1) {
        return None;
    }
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::Luma)
        .set_max_width(w as usize)
        .set_max_height(h as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options);
    decoder.decode_headers().ok()?;
    if decoder.input_colorspace()? != ColorSpace::Luma {
        return None;
    }
    let samples = decoder.decode().ok()?;
    let info = decoder.info()?;
    ((u32::from(info.width), u32::from(info.height)) == (w, h)
        && samples.len() == w as usize * h as usize)
        .then_some(samples)
}

/// An ink's amount in `0..=1` from its 8-bit sample (as decoded, possibly out of range for a
/// converted YCCK) and `/Decode` pair.
fn ink_amount(sample: f32, (lo, hi): (f32, f32)) -> f32 {
    (lo + sample.clamp(0.0, 255.0) / 255.0 * (hi - lo)).clamp(0.0, 1.0)
}

/// A paint whose shader is `shading`, its shading space mapped by `transform` into the space
/// the paint is drawn in. `None` when its colors do not convert or its geometry is degenerate.
///
/// The colors are the shading's samples as gradient stops. Where the shading does not extend
/// past an end, a transparent stop sits exactly at that end and the first color just inside
/// it, so padding past the end paints nothing.
fn shading_paint(shading: &Shading, transform: Transform, alpha: f32) -> Option<Paint<'static>> {
    let space = PaintSpace::of_image_space(&shading.color);
    let colors: Option<Vec<Rgb>> = shading.samples.iter().map(|s| space.color(s)).collect();
    let colors = colors?;
    let last = colors.len().checked_sub(1)?;
    // How far inside an end its first color starts when the end is not extended.
    const EDGE: f32 = 1.0 / 4096.0;
    let color = |c: Rgb| Color::from_rgba(c.0, c.1, c.2, alpha.clamp(0.0, 1.0));
    let mut stops = Vec::with_capacity(colors.len() + 2);
    if !shading.extend.0 {
        stops.push(GradientStop::new(0.0, Color::TRANSPARENT));
    }
    for (i, &c) in colors.iter().enumerate() {
        let mut pos = i as f32 / last.max(1) as f32;
        if i == 0 && !shading.extend.0 {
            pos = EDGE;
        }
        if i == last && !shading.extend.1 {
            pos = 1.0 - EDGE;
        }
        stops.push(GradientStop::new(pos, color(c)?));
    }
    if !shading.extend.1 {
        stops.push(GradientStop::new(1.0, Color::TRANSPARENT));
    }
    let shader: Shader<'static> = match shading.geometry {
        ShadingGeometry::Axial { from, to } => LinearGradient::new(
            Point::from_xy(from.0, from.1),
            Point::from_xy(to.0, to.1),
            stops,
            SpreadMode::Pad,
            transform,
        )?,
        ShadingGeometry::Radial { from, to } => RadialGradient::new(
            Point::from_xy(from.0, from.1),
            from.2,
            Point::from_xy(to.0, to.1),
            to.2,
            stops,
            SpreadMode::Pad,
            transform,
        )?,
    };
    Some(Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    })
}

/// What painting is clipped and masked by: the clip, times the soft mask when there is one.
fn active_clip(state: &GState) -> Option<&Mask> {
    match &state.masked_clip {
        Some(masked) => Some(masked),
        None => state.clip.as_deref(),
    }
}

/// The rasterizer's blend mode for a PDF blend mode (ISO 32000-1 §11.3.5): the separable
/// ones (§11.3.5.2) and the non-separable ones (§11.3.5.3) are the same set tiny-skia has.
fn blend_mode(mode: super::backend::BlendMode) -> BlendMode {
    match mode {
        super::backend::BlendMode::Normal => BlendMode::SourceOver,
        super::backend::BlendMode::Multiply => BlendMode::Multiply,
        super::backend::BlendMode::Screen => BlendMode::Screen,
        super::backend::BlendMode::Overlay => BlendMode::Overlay,
        super::backend::BlendMode::Darken => BlendMode::Darken,
        super::backend::BlendMode::Lighten => BlendMode::Lighten,
        super::backend::BlendMode::ColorDodge => BlendMode::ColorDodge,
        super::backend::BlendMode::ColorBurn => BlendMode::ColorBurn,
        super::backend::BlendMode::HardLight => BlendMode::HardLight,
        super::backend::BlendMode::SoftLight => BlendMode::SoftLight,
        super::backend::BlendMode::Difference => BlendMode::Difference,
        super::backend::BlendMode::Exclusion => BlendMode::Exclusion,
        super::backend::BlendMode::Hue => BlendMode::Hue,
        super::backend::BlendMode::Saturation => BlendMode::Saturation,
        super::backend::BlendMode::Color => BlendMode::Color,
        super::backend::BlendMode::Luminosity => BlendMode::Luminosity,
    }
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

/// How much `t` stretches lengths along the axis it stretches most.
fn max_stretch(t: &Transform) -> f32 {
    t.sx.hypot(t.ky).max(t.kx.hypot(t.sy))
}

/// Where stroking `path` with `stroke` in the space `transform` maps to device pixels can paint:
/// its bounds widened by the farthest a join or a cap reaches past the line, then by the pixel
/// a hairline or an anti-aliased edge covers.
fn stroke_bounds(path: &Path, stroke: &Stroke, transform: Transform) -> Option<Rect> {
    let reach = stroke.width / 2.0 * stroke.miter_limit.max(std::f32::consts::SQRT_2);
    path.bounds()
        .outset(reach, reach)?
        .transform(transform)?
        .outset(1.0, 1.0)
}

/// What drawing `image` decodes, for it and its soft mask: the bytes its filters gave, and the
/// samples its codec makes -- its declared size, or a JBIG2 page's own when that is larger.
fn decode_work(image: &RawXObject) -> u64 {
    let decoded = |x: &RawXObject| {
        let declared = x
            .width
            .zip(x.height)
            .map_or(0, |(w, h)| u64::from(w) * u64::from(h));
        let page = match (x.filter.as_deref(), &x.codec_parms) {
            (Some("JBIG2Decode"), Some(CodecParms::Jbig2 { globals })) => {
                super::jbig2::page_samples(&x.data, globals.as_deref())
            }
            _ => 0,
        };
        declared.max(page).saturating_add(x.data.len() as u64)
    };
    decoded(image).saturating_add(image.smask.as_deref().map_or(0, decoded))
}

/// How many dashes stroking `path` with `pattern`, in the path's space, can lay: the dashes of
/// its whole periods, capped where tiny-skia gives up at a million, and a partial period of every
/// dash for each contour, which starts the pattern again. Its length is taken along its control
/// points, which a curve never exceeds, and each point counts as a contour.
fn dashes(path: &Path, pattern: &[f32]) -> u64 {
    const MAX_DASHES: u64 = 1_000_000;
    let pairs = pattern.len() / 2;
    let interval: f32 = pattern.iter().sum();
    let length: f32 = path.points().windows(2).map(|p| p[0].distance(p[1])).sum();
    let whole = ((length * pairs as f32 / interval).ceil() as u64).min(MAX_DASHES);
    let partial = (pairs as u64).saturating_mul(path.points().len() as u64);
    whole.saturating_add(partial)
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
    fn a_step_the_budget_refuses_is_not_painted() {
        use super::super::backend::RawBackend;
        use super::super::test_pdf::{pdf, stream};
        let content = b"0 0 1 rg 0 0 100 100 re f";
        let bytes = pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Contents 4 0 R>>".to_vec(),
                stream(&format!("<</Length {}>>", content.len()), content),
            ],
            1,
        );
        let backend = RawBackend::load_bytes(&bytes).unwrap();
        let page = backend.pages()[&1];
        let ops = form_xobject::page_operations(&backend, page).unwrap().ops;
        for (refused, painted) in [(u64::MAX, true), (100 * 100, false)] {
            let mut budget = |work: u64| {
                if work >= refused {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let base = Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, 100.0);
            let white = solid(100, 100, [255; 4]);
            let mut painter = Painter::new(&backend, page, white, base, &mut budget);
            for op in &ops {
                painter.apply(op);
            }
            assert_eq!(painter.stopped, !painted);
            let blue = painter.pixmap.pixels().iter().any(|p| p.red() == 0);
            assert_eq!(blue, painted, "{refused}");
        }
    }

    #[test]
    fn an_image_near_its_drawn_size_is_left_alone() {
        let small = downsample(solid(10, 10, [9, 9, 9, 255]), (6.0, 6.0));
        assert_eq!((small.width(), small.height()), (10, 10));
    }
}
