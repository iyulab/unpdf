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

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::rc::Rc;

use super::backend::{
    CodecParms, ContentOp, ExtGState, GlyphAdvance, ImageColorSpace, ObjectId, PageBox, PageId,
    PdfBackend, PdfValue, RawXObject, ResourceScope, Shading, ShadingGeometry, SoftMaskEntry,
    SoftMaskRef, TilingPattern, TintSpace, Type3Font,
};
use super::cmyk::cmyk_to_rgb_unit;
use super::form_xobject;
use super::lab::lab_to_rgb_unit;
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

/// A page's size as displayed: the width and height, in points (1/72 inch), of the region a
/// raster covers with the page's `/Rotate` applied — the box [`PdfParser::render_page`] paints.
///
/// [`PdfParser::render_page`]: super::PdfParser::render_page
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    fn displayed(page_box: PageBox, rotation: u16) -> Self {
        let (width, height) = (page_box.width(), page_box.height());
        match rotation {
            90 | 270 => Self {
                width: height,
                height: width,
            },
            _ => Self { width, height },
        }
    }

    /// The pixels `render_page` paints this size in at `dpi`, as it sizes its bitmap with this
    /// function: each side is `points * (dpi / 72)` in `f32`, rounded half away from zero and at
    /// least one. That can be a pixel off exact arithmetic (396 points at 93 dpi paint 511
    /// pixels, not 512), so `72 * edge / points` is only a candidate `dpi` for a side of `edge`
    /// pixels, to check here before painting. `render_page` refuses a page of more than
    /// 200,000,000 pixels.
    pub fn pixels(&self, dpi: f32) -> (u32, u32) {
        let scale = dpi / 72.0;
        let side = |points: f32| (points * scale).round().max(1.0) as u32;
        (side(self.width), side(self.height))
    }
}

/// What a rasterized page could not show, by reason. All zero means everything the page's
/// content asks for was painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RasterGaps {
    /// Text runs whose paint or text clipping could not be fully applied: the font's program
    /// is not embedded and no standard face stands in for it, or is one this does not read, or a code
    /// selects no glyph in it (in a Type 3 font: no glyph procedure) or one with no outline
    /// whose text does not decode or has a character that is neither whitespace nor
    /// default-ignorable, or a Type 3 glyph procedure is nested more than four deep, or the
    /// text is shown before any `Tf`. Glyphs wholly outside the raster or its clip are excluded
    /// when their declared bounds are known.
    pub text_runs: u32,
    /// Images not painted: a codec this does not decode, a color
    /// space it does not convert, or data that did not decode, the soft mask's included.
    pub images: u32,
    /// Inline images (`BI … EI`) not painted: a dictionary that does not read. One that reads
    /// but is in a codec or color space this does not decode is counted in `images`.
    pub inline_images: u32,
    /// Shadings (`sh`) and pattern colors not painted: mesh shadings (types 4–7), function
    /// shadings (type 1), a PostScript calculator function, and a tiling pattern nested more
    /// than three deep. Axial and radial shadings and tiling patterns are painted. Text in
    /// such a pattern, or in a tiling pattern whose cell was not painted in full, is painted in
    /// black and counted here, not in `text_runs`.
    pub shadings: u32,
    /// Soft masks (`/SMask`) not applied as set: the mask group did not decode or is nested
    /// past three deep (what follows is painted unmasked), or its `/TR` transfer function,
    /// which this does not apply, was set (the mask is applied without it).
    pub soft_masks: u32,
    /// Content streams of the page that could not be decoded, and Form XObjects nested more
    /// than 32 deep; whatever they held is missing.
    pub undecodable_content_streams: u32,
    /// Fills and strokes (each path a Type 3 glyph paints among them), runs of text in other
    /// fonts and stencil masks painted in a color this does not convert: a color space it does
    /// not read (a `DeviceN` or `Separation` whose transform or alternate space is not one it
    /// reads, a resource that is missing), or a color its space gives no value for. Each is
    /// painted in the shade of its ink, dark where the tint is high.
    pub colors: u32,
    /// Annotations whose normal appearance (`/AP /N`) was not painted: a stream that is
    /// missing, does not decode or has no `/BBox`, an `/AP` with states and no `/AS`, a
    /// malformed `/Rect`, a `/BM` that is not a name or names a blend mode other than Normal,
    /// or an annotation past the first 4096 of the page, which is counted unread; and those
    /// painted without what they hold of a knockout group or soft mask, an isolated group past
    /// four nested group layers, a group that is not isolated painted under an alpha, blend
    /// mode or soft mask, or a form nested more than 32 deep. Isolated groups otherwise blend
    /// with nothing beneath them. Of the first 4096, an annotation that is hidden (Hidden,
    /// NoView, or Invisible with a type the standard does not define) or a pop-up, one with no
    /// appearance, and a state `/AS` selects no stream for are not counted. NoZoom and NoRotate
    /// are not applied.
    pub annotations: u32,
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
        self.colors += other.colors;
        self.annotations += other.annotations;
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

fn page_id(backend: &dyn PdfBackend, page_num: u32) -> Result<PageId> {
    let pages = backend.pages();
    pages
        .get(&page_num)
        .copied()
        .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))
}

fn region_box(backend: &dyn PdfBackend, page: PageId, region: PageRegion) -> PageBox {
    match region {
        PageRegion::Crop => backend.crop_box(page),
        PageRegion::Media => backend.page_box(page),
    }
}

/// The size of page `page_num` as [`render_page`] paints `region` of it.
pub(crate) fn page_size(
    backend: &dyn PdfBackend,
    page_num: u32,
    region: PageRegion,
) -> Result<PageSize> {
    let page = page_id(backend, page_num)?;
    let size = PageSize::displayed(
        region_box(backend, page, region),
        backend.page_rotation(page),
    );
    if !(size.width.is_finite() && size.height.is_finite()) {
        return Err(Error::Render(format!("page {page_num} has no finite size")));
    }
    Ok(size)
}

/// Rasterize page `page_num` of the document `backend` reads, charging `budget` as
/// [`PdfParser::render_page_with`](super::PdfParser::render_page_with) says.
pub(crate) fn render_page(
    backend: &dyn PdfBackend,
    page_num: u32,
    options: &RasterOptions,
    budget: &mut dyn FnMut(u64) -> ControlFlow<()>,
) -> Result<RasteredPage> {
    let page = page_id(backend, page_num)?;
    if !(options.dpi.is_finite() && options.dpi > 0.0) {
        return Err(Error::Render(format!(
            "dpi must be positive, got {}",
            options.dpi
        )));
    }

    let page_box = region_box(backend, page, options.region);
    let rotation = backend.page_rotation(page);
    let scale = options.dpi / 72.0;
    let (w, h) = (page_box.width() * scale, page_box.height() * scale);
    let (px_w, px_h) = PageSize::displayed(page_box, rotation).pixels(options.dpi);
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

    let shared = Shared::default();
    let painted = form_xobject::page_operations_with(backend, page, &shared.forms)?;
    let mut painter = Painter::new(backend, page, pixmap, base, budget);
    painter.shared = shared;
    painter.gaps.undecodable_content_streams = painted.undecodable_streams as u32;
    for op in &painted.ops {
        painter.apply(op);
    }
    painter.end_groups();
    painter.paint_annotations(base);
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
    Lab {
        white: [f32; 3],
        range: [f32; 4],
    },
    Indexed(ImageColorSpace),
    /// A pattern, with the space an uncolored pattern's color is given in.
    Pattern(Option<Box<PaintSpace>>),
    /// A `Separation` or `DeviceN` space: a tint per colorant its transform maps into an
    /// alternate space.
    Spot(TintSpace),
    /// A space this does not read: every color in it is counted in [`RasterGaps::colors`].
    Unconverted,
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
            PaintSpace::Lab { white, range } => {
                let component = |i: usize| values.get(i).copied().unwrap_or(0.0);
                let [r, g, b] =
                    lab_to_rgb_unit(*white, *range, [component(0), component(1), component(2)]);
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
                PaintSpace::of_image_space(base).color(&base.palette_entry(entry))?
            }
            PaintSpace::Pattern(base) => return base.as_ref()?.color(values),
            PaintSpace::Spot(space) => {
                let tints: Vec<f32> = (0..space.components()).map(v).collect();
                let (alternate, components) = space.alternate_color(&tints)?;
                PaintSpace::of_image_space(alternate).color(&components)?
            }
            PaintSpace::Unconverted => return None,
        })
    }

    /// The color `values` stand for, and whether this converted it. One it cannot is the shade
    /// of its strongest component, 0 white to 1 black; a pattern space without a base this
    /// reads, given no components, keeps `previous`.
    fn resolve(&self, values: &[f32], previous: Rgb) -> (Rgb, bool) {
        if values.is_empty() && matches!(self, PaintSpace::Pattern(None)) {
            return (previous, true);
        }
        match self.color(values) {
            Some(color) => (color, false),
            None => {
                let tint = values
                    .iter()
                    .fold(0.0f32, |tint, &v| tint.max(v.clamp(0.0, 1.0)));
                (Rgb(1.0 - tint, 1.0 - tint, 1.0 - tint), true)
            }
        }
    }

    fn of_image_space(space: &ImageColorSpace) -> PaintSpace {
        match space {
            ImageColorSpace::Gray => PaintSpace::Gray,
            ImageColorSpace::Rgb => PaintSpace::Rgb,
            ImageColorSpace::Cmyk => PaintSpace::Cmyk,
            ImageColorSpace::Lab { white, range } => PaintSpace::Lab {
                white: *white,
                range: *range,
            },
            indexed @ ImageColorSpace::Indexed { .. } => PaintSpace::Indexed(indexed.clone()),
            ImageColorSpace::Tint(space) => PaintSpace::Spot(space.clone()),
        }
    }

    /// The samples of a tint transform converting one color in this space reads.
    fn samples_read(&self) -> u64 {
        match self {
            PaintSpace::Spot(space) => space.samples_read(),
            PaintSpace::Indexed(space) => space.pixel_work(),
            PaintSpace::Pattern(Some(base)) => base.samples_read(),
            _ => 0,
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
    fill_unconverted: bool,
    stroke_unconverted: bool,
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
            fill_unconverted: false,
            stroke_unconverted: false,
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

/// A resource by the form whose resources name it (`None`: the page's) and its name.
type Named = (Option<ObjectId>, Vec<u8>);

/// An `/ExtGState` resource, its soft mask's group left unread.
type GraphicsState = (ExtGState, Option<SoftMaskRef>);

/// What the painters of a page read once and share: its Form XObjects, and the `/ExtGState` and
/// tiling pattern resources they name.
#[derive(Clone, Default)]
struct Shared {
    forms: form_xobject::Forms,
    gstates: Rc<RefCell<HashMap<Named, Option<Rc<GraphicsState>>>>>,
    patterns: Rc<RefCell<HashMap<ObjectId, Option<Rc<TilingPattern>>>>>,
    shading_patterns: Rc<RefCell<HashMap<ObjectId, Option<Rc<ShadingPattern>>>>>,
    shadings: Rc<RefCell<HashMap<ObjectId, Option<Rc<Shading>>>>>,
}

/// A shading pattern: its shading, and its matrix.
type ShadingPattern = (Shading, [f32; 6]);

/// What `read` gives, read once and kept in `kept` by object id `id`; read every time when
/// there is no id.
fn read_once<T>(
    kept: &RefCell<HashMap<ObjectId, Option<Rc<T>>>>,
    id: Option<ObjectId>,
    read: impl FnOnce() -> Option<T>,
) -> Option<Rc<T>> {
    let Some(id) = id else {
        return read().map(Rc::new);
    };
    if let Some(value) = kept.borrow().get(&id) {
        return value.clone();
    }
    let value = read().map(Rc::new);
    kept.borrow_mut().insert(id, value.clone());
    value
}

/// A font as a painter keeps it loaded: by its dictionary's object id, so every form naming
/// it shares it, or by the resource that names it when there is no id.
#[derive(Clone, PartialEq, Eq, Hash)]
enum FontId {
    Object(ObjectId),
    Named(FontResource),
}

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
    /// Fonts loaded so far, by [`FontId`]; `None` when unpaintable.
    fonts: HashMap<FontId, Option<LoadedFont>>,
    /// Type 3 fonts looked up so far, by the same key; `None` for a font of any other type.
    type3: HashMap<FontId, Option<Rc<Type3Font>>>,
    /// By the same key as `fonts`, whether a code whose glyph has no outline is
    /// [blank](PdfBackend::blank_code).
    blank_codes: HashMap<FontId, HashMap<u32, bool>>,
    /// Color spaces looked up so far, by the form whose resources name them (`None`: the
    /// page's) and their name: a sampled tint transform is read from the file once, not at
    /// every `cs`.
    spaces: HashMap<(Option<ObjectId>, Vec<u8>), PaintSpace>,
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
    /// What the page's painters read once.
    shared: Shared,
    /// Painting an annotation's appearance: what it cannot paint as it should is left out
    /// rather than painted as the page's content is.
    in_appearance: bool,
    /// Something of the appearance being painted was left out.
    cut: bool,
    /// How many transparency groups deep the operators being skipped are: those of a group an
    /// appearance cannot paint as it should.
    skipped_groups: usize,
}

/// A tiling pattern cell as painted: the content it was named from (form and name), its
/// pattern space → device pixels, and the color of an uncolored one -- as bits, to be a key.
type TileKey = (Option<ObjectId>, Vec<u8>, [u32; 6], [u32; 3]);

/// What a pattern color paints with, placed on the device.
enum PatternPaint {
    /// A shading pattern: its shading, the colors of its samples, and pattern space → device
    /// pixels.
    Shading(Shading, Vec<Rgb>, Transform),
    /// A tiling pattern: one cell, rendered, its pixels → device pixels, whether it is
    /// uncolored (painted in the color given with it), and — last — whether the cell was
    /// painted in full (`false` when it left something unpainted). The cell repeats.
    Tile(Pixmap, Transform, bool, bool),
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
    /// `false` for a tiling pattern whose cell left something unpainted.
    fn complete(&self) -> bool {
        match self {
            PaintSource::Ready(_) => true,
            PaintSource::Tile(tile, ..) => {
                matches!(**tile, PatternPaint::Tile(.., true))
            }
        }
    }

    fn paint(&self) -> Paint<'_> {
        match self {
            PaintSource::Ready(paint) => paint.clone(),
            PaintSource::Tile(tile, transform, alpha, blend_mode) => {
                let PatternPaint::Tile(pixmap, ..) = &**tile else {
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
            blank_codes: HashMap::new(),
            spaces: HashMap::new(),
            glyph_depth: 0,
            pattern_depth: 0,
            tiles: HashMap::new(),
            groups: Vec::new(),
            uncolored: false,
            gaps: RasterGaps::default(),
            substituted_text_runs: 0,
            budget,
            stopped: false,
            shared: Shared::default(),
            in_appearance: false,
            cut: false,
            skipped_groups: 0,
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
        if self.skipped_groups > 0 {
            match op.operator.as_str() {
                form_xobject::BEGIN_GROUP => self.skipped_groups += 1,
                form_xobject::END_GROUP => self.skipped_groups -= 1,
                _ => {}
            }
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
                    if let Some(read) = self.ext_gstate(op, name) {
                        let (gs, mask) = &*read;
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
                        let mask = mask.or_else(|| match &gs.soft_mask {
                            Some(SoftMaskEntry::Mask(read)) => {
                                let forms = &self.shared.forms;
                                forms.mask_group(self.backend, self.page, &read.group);
                                Some(SoftMaskRef {
                                    luminosity: read.luminosity,
                                    group: read.group.id,
                                    backdrop: read.backdrop,
                                    transfer_function: read.transfer_function,
                                })
                            }
                            _ => None,
                        });
                        if let Some(mask) = mask {
                            self.state.soft_mask = self.render_soft_mask(&mask).map(Rc::new);
                            self.refresh_masked_clip();
                        } else if let Some(SoftMaskEntry::None) = gs.soft_mask {
                            self.state.soft_mask = None;
                            self.refresh_masked_clip();
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
                    PaintSpace::Gray | PaintSpace::Indexed(_) => vec![0.0],
                    PaintSpace::Rgb | PaintSpace::Lab { .. } => vec![0.0, 0.0, 0.0],
                    // Full tint.
                    PaintSpace::Spot(space) => vec![1.0; space.components()],
                    PaintSpace::Unconverted => vec![1.0],
                    PaintSpace::Pattern(_) => Vec::new(),
                };
                if op.operator == "cs" {
                    self.set_fill(space, &initial);
                } else {
                    self.set_stroke(space, &initial);
                }
            }
            "sc" | "scn" => {
                let space = self.state.fill_space.clone();
                self.set_fill(space, &all());
                if self.state.fill_pattern && !self.stopped {
                    self.state.fill_pattern_paint = self.pattern_named(op, self.state.fill);
                }
            }
            "SC" | "SCN" => {
                let space = self.state.stroke_space.clone();
                self.set_stroke(space, &all());
                if self.state.stroke_pattern && !self.stopped {
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
                    let (backend, scope) = (self.backend, op.scope(self.page));
                    if let Some(image) =
                        backend.image_xobject_with(scope, name, &mut |work| self.admit(work))
                    {
                        self.draw_image(&image);
                    }
                }
            }
            "BI" => {
                let image = match op.operands.first() {
                    Some(PdfValue::Str(body)) => {
                        let (backend, scope) = (self.backend, op.scope(self.page));
                        backend.inline_image_with(scope, body, &mut |work| self.admit(work))
                    }
                    _ => None,
                };
                match image {
                    Some(image) => self.draw_image(&image),
                    None => self.gaps.inline_images += 1,
                }
            }
            form_xobject::BEGIN_GROUP => {
                let set = |key: &[u8]| {
                    self.in_appearance
                        && op
                            .operands
                            .iter()
                            .any(|v| matches!(v, PdfValue::Name(name) if name == key))
                };
                if set(b"K") || !self.begin_group(set(b"I")) {
                    self.cut = true;
                    self.skipped_groups = 1;
                }
            }
            form_xobject::END_GROUP => self.end_group(),
            form_xobject::FORM_TOO_DEEP => {
                if self.in_appearance {
                    self.cut = true;
                } else {
                    self.gaps.undecodable_content_streams += 1;
                }
            }
            // Type 3 glyph metrics: `d1` marks the glyph uncolored.
            "d1" if self.glyph_depth > 0 => self.uncolored = true,
            "sh" => {
                let shading = match op.operands.first() {
                    Some(PdfValue::Name(name)) => {
                        let (backend, scope) = (self.backend, op.scope(self.page));
                        let shared = self.shared.clone();
                        read_once(&shared.shadings, backend.shading_id(scope, name), || {
                            backend.shading_with(scope, name, &mut |work| self.admit(work))
                        })
                    }
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

    /// The `/ExtGState` resource `name` names where `op` is, read once for the page, a soft
    /// mask's group left for [`Self::render_soft_mask`] to read.
    fn ext_gstate(&self, op: &ContentOp, name: &[u8]) -> Option<Rc<GraphicsState>> {
        let key = (op.form, name.to_vec());
        if let Some(gs) = self.shared.gstates.borrow().get(&key) {
            return gs.clone();
        }
        let gs = self
            .backend
            .ext_gstate_ref(op.scope(self.page), name)
            .map(Rc::new);
        self.shared.gstates.borrow_mut().insert(key, gs.clone());
        gs
    }

    /// [`Self::apply`] `op`, breaking once the painting has stopped.
    fn paint_op(&mut self, op: &ContentOp) -> ControlFlow<()> {
        self.apply(op);
        if self.stopped {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    /// Paint the page's annotation appearances over its content, one at a time, each from the
    /// initial graphics state.
    fn paint_annotations(&mut self, base: Transform) {
        if self.stopped {
            return;
        }
        let read = self.backend.annotation_appearances(self.page);
        let mut appearances =
            form_xobject::Appearances::new(self.backend, self.page, &self.shared.forms);
        let mut cut = 0;
        self.in_appearance = true;
        for appearance in &read.appearances {
            self.state = GState::new(base);
            self.stack.clear();
            self.path = PathBuilder::new();
            self.pending_clip = None;
            self.text_clip = None;
            self.text_matrix = TextMatrix::default();
            self.tiles.clear();
            self.cut = false;
            let painted = appearances.paint(appearance, &mut |op| self.paint_op(&op));
            self.end_groups();
            cut += u32::from(self.cut);
            if painted.is_break() {
                break;
            }
        }
        self.gaps.annotations = read.unpainted + appearances.unpainted + cut;
        self.gaps.undecodable_content_streams += appearances.undecodable_streams() as u32;
    }

    /// Paint the strings and position adjustments of a text-showing operator, advancing the
    /// text matrix as text extraction does.
    fn show(&mut self, items: &[PdfValue]) {
        let Some(key) = self.state.text.font.clone() else {
            self.show_without_font(items);
            return;
        };
        let text = self.state.text.params.clone();
        let scope = ResourceScope {
            page: self.page,
            form: key.0,
        };
        let id = match self.backend.font_id(scope, &key.1) {
            Some(id) => FontId::Object(id),
            None => FontId::Named(key.clone()),
        };
        if !self.type3.contains_key(&id) {
            let font = self.backend.type3_font(scope, &key.1).map(Rc::new);
            self.type3.insert(id.clone(), font);
        }
        if let Some(font) = self.type3.get(&id).cloned().flatten() {
            self.show_type3(&font, scope, &key.1, &text, items);
            return;
        }
        if !self.fonts.contains_key(&id) {
            let loaded = self
                .backend
                .font_program(scope, &key.1)
                .and_then(LoadedFont::load);
            self.fonts.insert(id.clone(), loaded);
        }
        let vertical = self.backend.is_vertical_font(scope, &key.1);
        let bounds = self.backend.font_bounds(scope, &key.1);
        // Mode 3 paints nothing (an OCR layer over a scan): nothing is missing.
        let invisible = text.render_mode == 3;
        let mut missing = false;
        let mut substituted = false;
        let mut unpainted_color = false;
        let mut blank_codes = self.blank_codes.remove(&id).unwrap_or_default();
        let mut unconverted_color = false;
        // Taken out of the cache while its glyphs are painted, put back after.
        let mut loaded = self.fonts.remove(&id).flatten();

        'run: for item in items {
            if !self.admit_shown(item) {
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
                            missing = missing
                                || !invisible
                                    && self.missing_glyph_visible(bounds, &text, measured, true);
                            self.text_matrix.advance(text.glyph_advance(measured));
                        }
                        self.text_matrix = start;
                        self.text_matrix.advance(text.advance_of(advances));
                    }
                    None => {
                        missing = missing
                            || !invisible
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
                            unconverted_color |= self.paints_unconverted_color(text.render_mode);
                        } else if !missing {
                            let blank = *blank_codes.entry(code).or_insert_with(|| {
                                let (blank, decoded) =
                                    self.backend.blank_code(scope, &key.1, chunk);
                                self.admit(decoded as u64) && blank
                            });
                            missing = !blank
                                && self.missing_glyph_visible(bounds, &text, &measured, true);
                        }
                    }
                    Some(_) => {}
                    None => {
                        missing = missing
                            || !invisible
                                && code != 32
                                && self.missing_glyph_visible(bounds, &text, &measured, true)
                    }
                }
                self.text_matrix.advance(text.glyph_advance(&measured));
            }
        }
        self.blank_codes.insert(id.clone(), blank_codes);
        self.fonts.insert(id, loaded);
        if missing {
            self.gaps.text_runs += 1;
        }
        // Text in a pattern this does not paint, once per run.
        if unpainted_color {
            self.gaps.shadings += 1;
        }
        self.gaps.colors += u32::from(unconverted_color);
        if substituted {
            self.substituted_text_runs += 1;
        }
    }

    /// [`Self::admit`] one item of a text-showing operator: a string's codes, or one.
    fn admit_shown(&mut self, item: &PdfValue) -> bool {
        let codes = match item {
            PdfValue::Str(bytes) => bytes.len() as u64,
            _ => 1,
        };
        self.admit(codes)
    }

    /// Text shown before any `Tf` has no font to paint with: one run lost, if anything of it
    /// would show.
    fn show_without_font(&mut self, items: &[PdfValue]) {
        for item in items {
            if !self.admit_shown(item) {
                return;
            }
            if matches!(item, PdfValue::Str(bytes) if !bytes.is_empty()) {
                let mode = self.state.text.params.render_mode;
                if mode != 3 && self.visible_bounds(None, (4..=7).contains(&mode)) {
                    self.gaps.text_runs += 1;
                }
                return;
            }
        }
    }

    fn paints_unconverted_color(&self, mode: i64) -> bool {
        glyph_fills(mode) && self.paints_unconverted(false)
            || glyph_strokes(mode) && self.paints_unconverted(true)
    }

    /// Whether filling (`stroke` false) or stroking paints in a color this did not convert: a
    /// pattern paints in its own colors unless it is an uncolored tiling pattern.
    fn paints_unconverted(&self, stroke: bool) -> bool {
        let s = &self.state;
        let (unconverted, pattern, paint) = if stroke {
            (
                s.stroke_unconverted,
                s.stroke_pattern,
                &s.stroke_pattern_paint,
            )
        } else {
            (s.fill_unconverted, s.fill_pattern, &s.fill_pattern_paint)
        };
        unconverted
            && (!pattern || matches!(paint.as_deref(), Some(PatternPaint::Tile(_, _, true, _))))
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
                let reach = if follows_render_mode && glyph_strokes(text.render_mode) {
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
        scope: ResourceScope,
        font_name: &[u8],
        text: &TextParams,
        items: &[PdfValue],
    ) {
        let bounds = self.backend.font_bounds(scope, font_name);
        let invisible = matches!(text.render_mode, 3 | 7);
        let mut missing = false;
        for item in items {
            if !self.admit_shown(item) {
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
                if !invisible {
                    match font.glyphs.get(&code) {
                        Some(ops) => {
                            if !self.run_glyph(ops, font.matrix, text, &measured) {
                                missing = missing
                                    || self.missing_glyph_visible(bounds, text, &measured, false);
                            }
                        }
                        None => {
                            missing = missing
                                || code != 32
                                    && self.missing_glyph_visible(bounds, text, &measured, false)
                        }
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
    /// What the procedure does to the graphics, text and path state stays inside it. `false`
    /// when it did not run, being nested too deep.
    fn run_glyph(
        &mut self,
        ops: &[ContentOp],
        matrix: [f32; 6],
        text: &TextParams,
        measured: &GlyphAdvance,
    ) -> bool {
        // A glyph that shows text in a Type 3 font that shows text ... ends here.
        const MAX_GLYPH_DEPTH: usize = 4;
        if self.glyph_depth >= MAX_GLYPH_DEPTH {
            return false;
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
        true
    }

    /// A transparency group begins (ISO 32000-1 §11.6.6). A group painted with an alpha below
    /// one, with a blend mode or under a soft mask goes on a layer of its own, which is
    /// composited with that alpha, blend mode and mask when it ends -- so overlapping things
    /// inside it do not show through one another, and an alpha the group sets inside itself
    /// does not undo the one it is painted with. Inside, the alpha starts at one, the blend
    /// mode at Normal and the soft mask at none (a soft mask the group's own content sets
    /// does not undo the one it is painted with either). An `isolated` group goes on a layer
    /// too, so its content blends with nothing under it; `false`, and nothing begun, when it
    /// cannot have one, or, in an appearance, when a group that is not isolated would need one,
    /// which would cut it off from its backdrop. A
    /// fully opaque, normally blended group is painted in place, which is the same picture;
    /// so is one nested past `MAX_GROUP_LAYERS`, to bound the memory layers take.
    fn begin_group(&mut self, isolated: bool) -> bool {
        const MAX_GROUP_LAYERS: usize = 4;
        let alpha = self.state.fill_alpha;
        let blend = self.state.blend;
        let soft_mask = self.state.soft_mask.clone();
        let layered = self.groups.iter().filter(|g| g.is_some()).count();
        let needs_layer = alpha < 1.0 || blend != BlendMode::SourceOver || soft_mask.is_some();
        if self.in_appearance && needs_layer && !isolated {
            return false;
        }
        let own_layer = (isolated || needs_layer) && layered < MAX_GROUP_LAYERS;
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
                true
            }
            None if isolated => false,
            None => {
                self.groups.push(None);
                true
            }
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
    /// part of it was painted in black: its color is a pattern this does not paint.
    fn paint_glyph(&mut self, outline: &Path, transform: Transform, mode: i64) -> bool {
        if (4..=7).contains(&mode) && self.admit(outline.points().len() as u64) {
            if let Some(device) = outline.clone().transform(transform) {
                self.text_clip
                    .get_or_insert_with(PathBuilder::new)
                    .push_path(&device);
            }
        }
        let fill = glyph_fills(mode);
        let stroke = glyph_strokes(mode);
        let mut painted = true;
        if fill {
            let (source, exact) = self.glyph_paint(false, transform);
            painted &= exact;
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
        if stroke {
            // The line width is in user space; the glyph path is in glyph space. Stroke in
            // device space with the width the CTM gives it.
            let (source, exact) = self.glyph_paint(true, Transform::identity());
            painted &= exact;
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

    /// [`Self::current_paint`] for a glyph; where the color is a pattern this does not paint,
    /// or a tiling pattern whose cell left something unpainted, black, and `false`, so the text
    /// stays legible.
    fn glyph_paint(&self, stroke: bool, path_transform: Transform) -> (PaintSource, bool) {
        match self.current_paint(stroke, path_transform) {
            Some(source) if source.complete() => (source, true),
            _ => (self.solid_source(Rgb::BLACK, stroke), false),
        }
    }

    /// A solid `color` with the alpha and blend mode of the fill (`stroke` false) or stroke.
    fn solid_source(&self, color: Rgb, stroke: bool) -> PaintSource {
        let s = &self.state;
        let mut paint = solid(color, if stroke { s.stroke_alpha } else { s.fill_alpha });
        paint.blend_mode = s.blend;
        PaintSource::Ready(paint)
    }

    fn space_named(&mut self, op: &ContentOp, name: &[u8]) -> PaintSpace {
        let key = (op.form, name.to_vec());
        if let Some(space) = self.spaces.get(&key) {
            return space.clone();
        }
        let space = self.read_space(op, name);
        self.spaces.insert(key, space.clone());
        space
    }

    fn read_space(&mut self, op: &ContentOp, name: &[u8]) -> PaintSpace {
        let (backend, scope) = (self.backend, op.scope(self.page));
        let mut admit = |work| self.admit(work);
        if let Some(base) = backend.named_pattern_space_with(scope, name, &mut admit) {
            return PaintSpace::Pattern(base.map(|b| Box::new(PaintSpace::of_image_space(&b))));
        }
        match name {
            b"DeviceGray" | b"G" | b"CalGray" => PaintSpace::Gray,
            b"DeviceRGB" | b"RGB" | b"CalRGB" => PaintSpace::Rgb,
            b"DeviceCMYK" | b"CMYK" => PaintSpace::Cmyk,
            b"Pattern" => PaintSpace::Pattern(None),
            _ => match backend.named_color_space_with(scope, name, &mut admit) {
                Some(space) => PaintSpace::of_image_space(&space),
                None => PaintSpace::Unconverted,
            },
        }
    }

    /// [`Self::admit`] `work` when there is any: `false` once the painting has stopped.
    fn admit_work(&mut self, work: u64) -> bool {
        if work > 0 {
            self.admit(work)
        } else {
            !self.stopped
        }
    }

    fn set_fill(&mut self, space: PaintSpace, values: &[f32]) {
        if !self.admit_work(space.samples_read()) {
            return;
        }
        self.state.fill_pattern = matches!(space, PaintSpace::Pattern(_));
        self.state.fill_pattern_paint = None;
        (self.state.fill, self.state.fill_unconverted) = space.resolve(values, self.state.fill);
        self.state.fill_space = space;
    }

    fn set_stroke(&mut self, space: PaintSpace, values: &[f32]) {
        if !self.admit_work(space.samples_read()) {
            return;
        }
        self.state.stroke_pattern = matches!(space, PaintSpace::Pattern(_));
        self.state.stroke_pattern_paint = None;
        (self.state.stroke, self.state.stroke_unconverted) =
            space.resolve(values, self.state.stroke);
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
                self.gaps.colors += u32::from(self.paints_unconverted(false));
                if let Some(source) = self.path_paint(false, self.state.ctm) {
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
            }
            if stroke {
                self.gaps.colors += u32::from(self.paints_unconverted(true));
                if let Some(source) = self.path_paint(true, self.state.ctm) {
                    self.stroke(path, &source.paint());
                }
            }
        }
        if let Some(rule) = self.pending_clip.take() {
            self.clip(path.as_ref(), rule);
        }
    }

    /// The paint for filling (`stroke` false) or stroking what is drawn with `transform`: the
    /// current path or a stencil mask. In a Type 3 glyph it is [`Self::glyph_paint`]'s;
    /// elsewhere a pattern this does not paint leaves it unpainted. Either fallback is counted
    /// in `shadings`.
    fn path_paint(&mut self, stroke: bool, transform: Transform) -> Option<PaintSource> {
        if self.glyph_depth > 0 {
            let (source, exact) = self.glyph_paint(stroke, transform);
            if !exact {
                self.gaps.shadings += 1;
            }
            return Some(source);
        }
        let source = self.current_paint(stroke, transform);
        if source.is_none() {
            self.gaps.shadings += 1;
        }
        source
    }

    /// What the pattern `scn`/`SCN` names (its last operand) paints with, placed by its matrix
    /// relative to the default space of the content being painted. `color` is the color given
    /// with it, which an uncolored tiling pattern paints in.
    fn pattern_named(&mut self, op: &ContentOp, color: Rgb) -> Option<Rc<PatternPaint>> {
        let Some(PdfValue::Name(name)) = op.operands.last() else {
            return None;
        };
        let (backend, scope, base) = (self.backend, op.scope(self.page), self.state.pattern_base);
        let place =
            |[a, b, c, d, e, f]: [f32; 6]| base.pre_concat(Transform::from_row(a, b, c, d, e, f));
        let id = backend.pattern_id(scope, name);
        let shared = self.shared.clone();
        let shading = read_once(&shared.shading_patterns, id, || {
            backend.shading_pattern_with(scope, name, &mut |work| self.admit(work))
        });
        if let Some(shading) = shading {
            let (shading, matrix) = &*shading;
            let colors = self.shading_colors(shading)?;
            return Some(Rc::new(PatternPaint::Shading(
                shading.clone(),
                colors,
                place(*matrix),
            )));
        }
        let pattern = read_once(&self.shared.patterns, id, || {
            self.backend.tiling_pattern(scope, name)
        })?;
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
        let tile = self.render_tile(&pattern, to_device, color).map(Rc::new);
        self.tiles.insert(key, tile.clone());
        tile
    }

    /// Paint one cell of a tiling pattern into a pixmap of its own, at the resolution the
    /// device shows it with, as a [`PatternPaint::Tile`].
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
    ) -> Option<PatternPaint> {
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
        tile.shared = self.shared.clone();
        tile.in_appearance = self.in_appearance;
        let undecodable = form_xobject::paint_content(
            self.backend,
            self.page,
            &self.shared.forms,
            ops,
            pattern.id,
            &mut |op| tile.paint_op(&op),
        );
        tile.gaps.undecodable_content_streams += undecodable as u32;
        tile.end_groups();
        self.cut |= tile.cut;
        if tile.stopped {
            self.stopped = true;
            return None;
        }
        self.gaps.absorb(&tile.gaps);
        self.substituted_text_runs += tile.substituted_text_runs;
        // A color painted in the shade of its ink (`colors`) is painted; only content left out
        // makes the cell incomplete.
        let left_out = RasterGaps {
            colors: 0,
            ..tile.gaps
        };
        Some(PatternPaint::Tile(
            tile.pixmap,
            tile_to_device,
            pattern.uncolored,
            left_out.is_empty(),
        ))
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
            return Some(self.solid_source(color, stroke));
        }
        let pattern_paint = pattern_paint.as_ref()?;
        let to_path = path_transform.invert()?;
        match &**pattern_paint {
            PatternPaint::Shading(shading, colors, to_device) => {
                shading_paint(shading, colors, to_path.pre_concat(*to_device), alpha).map(
                    |mut paint| {
                        paint.blend_mode = blend;
                        PaintSource::Ready(paint)
                    },
                )
            }
            PatternPaint::Tile(_, to_device, ..) => Some(PaintSource::Tile(
                pattern_paint.clone(),
                to_path.pre_concat(*to_device),
                alpha,
                blend,
            )),
        }
    }

    /// The colors of `shading`'s samples, once the budget admits what converting them reads:
    /// `None` when it does not, or a color does not convert.
    fn shading_colors(&mut self, shading: &Shading) -> Option<Vec<Rgb>> {
        let space = PaintSpace::of_image_space(&shading.color);
        let work = (shading.samples.len() as u64).saturating_mul(space.samples_read());
        if !self.admit_work(work) {
            return None;
        }
        shading.samples.iter().map(|s| space.color(s)).collect()
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
        let alpha = self.state.fill_alpha;
        let paint = self
            .shading_colors(shading)
            .and_then(|colors| shading_paint(shading, &colors, shader_transform, alpha))
            .map(|mut p| {
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
    fn render_soft_mask(&mut self, mask: &SoftMaskRef) -> Option<Mask> {
        const MAX_MASK_DEPTH: usize = 3;
        if self.pattern_depth >= MAX_MASK_DEPTH {
            self.gaps.soft_masks += 1;
            return None;
        }
        let group = self.shared.forms.form(self.backend, self.page, mask.group);
        let Some((group, ops)) = group
            .as_ref()
            .and_then(|group| Some((group, group.ops.as_ref()?)))
        else {
            self.gaps.soft_masks += 1;
            return None;
        };
        if self.in_appearance && group.group.is_some_and(|group| group.knockout) {
            self.cut = true;
            return None;
        }
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
        sub.current_form = Some(mask.group);
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
        sub.shared = self.shared.clone();
        sub.in_appearance = self.in_appearance;
        let undecodable = form_xobject::paint_content(
            self.backend,
            self.page,
            &self.shared.forms,
            ops,
            mask.group,
            &mut |op| sub.paint_op(&op),
        );
        sub.gaps.undecodable_content_streams += undecodable as u32;
        sub.end_groups();
        self.cut |= sub.cut;
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
        if image.image_mask {
            self.gaps.colors += u32::from(self.paints_unconverted(false));
        }
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
        if image.image_mask && self.state.fill_pattern {
            self.draw_stencil_in_pattern(&pixmap, transform);
            return;
        }
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

    /// Paint the fill's pattern, as [`Self::path_paint`] gives it, where `stencil`, drawn with
    /// `transform`, marks: the stencil's coverage on a layer over the pixels it reaches takes
    /// the pattern's colors there.
    fn draw_stencil_in_pattern(&mut self, stencil: &Pixmap, transform: Transform) {
        let (w, h) = (self.pixmap.width() as f32, self.pixmap.height() as f32);
        let (x0, y0, x1, y1) =
            Rect::from_xywh(0.0, 0.0, stencil.width() as f32, stencil.height() as f32)
                .and_then(|r| r.transform(transform))
                .map_or((0.0, 0.0, 0.0, 0.0), |r| {
                    (
                        r.left().floor().clamp(0.0, w),
                        r.top().floor().clamp(0.0, h),
                        r.right().ceil().clamp(0.0, w),
                        r.bottom().ceil().clamp(0.0, h),
                    )
                });
        let place = Transform::from_translate(x0, y0);
        let Some(source) = self.path_paint(false, place) else {
            return;
        };
        let Some(mut layer) = Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32) else {
            return;
        };
        let coverage = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        let to_layer = Transform::from_translate(-x0, -y0).pre_concat(transform);
        layer.draw_pixmap(0, 0, stencil.as_ref(), &coverage, to_layer, None);
        let mut paint = source.paint();
        paint.blend_mode = BlendMode::SourceIn;
        let area = Rect::from_xywh(0.0, 0.0, x1 - x0, y1 - y0).expect("the layer is a rect");
        layer.fill_rect(area, &paint, Transform::identity(), None);
        let onto_page = PixmapPaint {
            blend_mode: self.state.blend,
            ..PixmapPaint::default()
        };
        self.pixmap.draw_pixmap(
            x0 as i32,
            y0 as i32,
            layer.as_ref(),
            &onto_page,
            Transform::identity(),
            active_clip(&self.state),
        );
    }

    /// An image as a premultiplied pixmap: its samples, its soft mask as alpha, or — a
    /// stencil mask — the fill color where the stencil marks. `None` when it or its soft mask
    /// does not decode.
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
        if let Some(smask) = image.smask.as_deref() {
            let (mw, mh, alpha) = mask_alpha(smask)?;
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
            let color = image.color.as_ref()?;
            let decode = image.decode.as_deref();
            let transform = match image.codec_parms {
                Some(CodecParms::Dct { color_transform }) => Some(color_transform),
                _ => None,
            };
            return match color {
                ImageColorSpace::Rgb | ImageColorSpace::Cmyk => {
                    decode_jpeg(&image.data, w, h, color.components(), transform, decode)
                }
                _ => {
                    let components = color.components();
                    let samples = decode_jpeg_samples(&image.data, w, h, components, transform)?;
                    png_encode::image_pixels(w, h, 8, color, decode, &samples)
                }
            };
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

/// Decode a JPEG of `components` components to RGB pixels of the size the image dictionary
/// declares, three of them turned back from YCbCr as [`jpeg_decoding`] says.
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
    components: usize,
    color_transform: Option<bool>,
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
    if usize::from(decoder.info()?.components) != components {
        return None;
    }
    let (out, transformed) = jpeg_decoding(
        data,
        decoder.input_colorspace()?,
        components,
        color_transform,
    )?;
    let four = components == 4;
    if out != ColorSpace::RGB {
        decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options(out));
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
        let ink: [f32; 4] = if transformed {
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

/// The samples of a JPEG of `components` components as the DCT filter gives them: three turned
/// back from YCbCr as [`jpeg_decoding`] says, one or four as they are stored. `None` for a
/// JPEG of another number of components, or of four its encoder turned into YCCK.
fn decode_jpeg_samples(
    data: &[u8],
    w: u32,
    h: u32,
    components: usize,
    color_transform: Option<bool>,
) -> Option<Vec<u8>> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    if !matches!(components, 1 | 3 | 4) || !png_encode::decodable(w, h, components as u32) {
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
    if usize::from(decoder.info()?.components) != components {
        return None;
    }
    let (out, transformed) = jpeg_decoding(
        data,
        decoder.input_colorspace()?,
        components,
        color_transform,
    )?;
    if components == 4 && transformed {
        return None;
    }
    if out != ColorSpace::RGB {
        decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options(out));
    }
    let samples = decoder.decode().ok()?;
    let info = decoder.info()?;
    ((u32::from(info.width), u32::from(info.height)) == (w, h)
        && samples.len() == w as usize * h as usize * components)
        .then_some(samples)
}

/// How to decode `data`, a JPEG of `components` components its decoder reads as `input`: what
/// into, for its samples to come out as stored and three turned back from YCbCr, and whether
/// its encoder turned RGB into YCbCr or CMYK into YCCK (ISO 32000-1 Table 13) -- as its last
/// Adobe marker says, else as `/ColorTransform` says, else for three components only. `None`
/// when its markers cannot be followed to its first scan, when an Adobe marker follows that
/// scan, or when the decoder reads the components otherwise than those markers say, as when
/// it reads three named R, G and B as RGB though a marker calls them YCbCr, or misses a marker.
fn jpeg_decoding(
    data: &[u8],
    input: zune_jpeg::zune_core::colorspace::ColorSpace,
    components: usize,
    color_transform: Option<bool>,
) -> Option<(zune_jpeg::zune_core::colorspace::ColorSpace, bool)> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    let marker = adobe_transform(data)?;
    let read_as_marked = match (components, marker) {
        (3, Some(0)) => matches!(input, ColorSpace::CMYK | ColorSpace::RGB),
        (3, Some(_)) => input == ColorSpace::YCbCr,
        (3, None) => matches!(input, ColorSpace::YCbCr | ColorSpace::RGB),
        (4, Some(2)) => input == ColorSpace::YCCK,
        (4, _) => input == ColorSpace::CMYK,
        _ => true,
    };
    if !read_as_marked {
        return None;
    }
    let transformed =
        components >= 3 && marker.map_or(color_transform.unwrap_or(components == 3), |t| t != 0);
    let out = match components {
        1 => ColorSpace::Luma,
        3 if !transformed && input == ColorSpace::YCbCr => ColorSpace::YCbCr,
        3 => ColorSpace::RGB,
        _ => input,
    };
    Some((out, transformed))
}

/// The transform the last Adobe marker before a JPEG's first scan names, the one its decoder
/// reads: `Some(None)` without one. `None` when the markers cannot be followed to the scan, or
/// an Adobe marker comes after it, which the decoder would read too.
fn adobe_transform(data: &[u8]) -> Option<Option<u8>> {
    let mut transform = None;
    let mut scanned = false;
    let mut at = 2;
    loop {
        if scanned {
            let Some(skip) = data
                .get(at..)?
                .windows(2)
                .position(|pair| pair[0] == 0xFF && !matches!(pair[1], 0x00 | 0xD0..=0xD7 | 0xFF))
            else {
                return Some(transform);
            };
            at += skip;
        }
        match *data.get(at..)? {
            [0xFF, 0xFF, ..] => at += 1,
            [0xFF, 0xD9, ..] => return Some(transform),
            [0xFF, marker, high, low, ..] => {
                let end = at + 2 + usize::from(u16::from_be_bytes([high, low]));
                let segment = data.get(at + 4..end)?;
                if marker == 0xEE && segment.len() >= 12 && segment.starts_with(b"Adobe") {
                    if scanned {
                        return None;
                    }
                    transform = Some(segment[11]);
                }
                scanned |= marker == 0xDA;
                at = end;
            }
            _ => return None,
        }
    }
}

/// An ink's amount in `0..=1` from its 8-bit sample (as decoded, possibly out of range for a
/// converted YCCK) and `/Decode` pair.
fn ink_amount(sample: f32, (lo, hi): (f32, f32)) -> f32 {
    (lo + sample.clamp(0.0, 255.0) / 255.0 * (hi - lo)).clamp(0.0, 1.0)
}

/// A paint whose shader is `shading`, its shading space mapped by `transform` into the space
/// the paint is drawn in. `None` when its geometry is degenerate.
///
/// The colors, those of the shading's samples, are its gradient stops. Where the shading does not extend
/// past an end, a transparent stop sits exactly at that end and the first color just inside
/// it, so padding past the end paints nothing.
fn shading_paint(
    shading: &Shading,
    colors: &[Rgb],
    transform: Transform,
    alpha: f32,
) -> Option<Paint<'static>> {
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

fn glyph_fills(mode: i64) -> bool {
    matches!(mode, 0 | 2 | 4 | 6)
}

fn glyph_strokes(mode: i64) -> bool {
    matches!(mode, 1 | 2 | 5 | 6)
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
/// samples its codec makes -- its declared size, or a JBIG2 page's own when that is larger --
/// and the samples its tint transform reads to convert each pixel, or each value a sample can
/// take when [`png_encode::lut_entries`] converts those instead.
fn decode_work(image: &RawXObject) -> u64 {
    let declared_samples = |x: &RawXObject| {
        x.width
            .zip(x.height)
            .map_or(0, |(w, h)| u64::from(w) * u64::from(h))
    };
    let decoded = |x: &RawXObject| {
        let declared = declared_samples(x);
        let page = match (x.filter.as_deref(), &x.codec_parms) {
            (Some("JBIG2Decode"), Some(CodecParms::Jbig2 { globals })) => {
                super::jbig2::page_samples(&x.data, globals.as_deref())
            }
            _ => 0,
        };
        let space = x.color.as_ref().map_or(0, |color| {
            let bits = match x.filter.as_deref() {
                Some("DCTDecode" | "JPXDecode") => Some(8),
                _ => x.bits_per_component,
            };
            bits.and_then(|bits| png_encode::lut_entries(color, bits))
                .map_or(declared, u64::from)
                .saturating_mul(color.pixel_work())
        });
        declared
            .max(page)
            .saturating_add(x.data.len() as u64)
            .saturating_add(space)
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
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::backend::{
        AnnotationAppearances, BackendFontInfo, DecodedText, FormXObject, PaintedXObject,
        PdfMetadataRaw, RawBackend, RawOutlineItem,
    };
    use super::super::test_pdf::{pdf, stream};
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

    struct CountingForms {
        backend: RawBackend,
        read: AtomicUsize,
    }

    impl CountingForms {
        fn load(objects: Vec<Vec<u8>>) -> Self {
            let bytes = pdf(objects, 1);
            CountingForms {
                backend: RawBackend::load_bytes(&bytes).unwrap(),
                read: Default::default(),
            }
        }

        fn take_read(&self) -> usize {
            self.read.swap(0, Ordering::Relaxed)
        }

        fn count(&self) {
            self.read.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl PdfBackend for CountingForms {
        fn pages(&self) -> BTreeMap<u32, PageId> {
            self.backend.pages()
        }
        fn page_fonts(&self, scope: ResourceScope) -> Result<Vec<BackendFontInfo>> {
            self.backend.page_fonts(scope)
        }
        fn page_content(&self, page: PageId) -> Result<Vec<u8>> {
            self.backend.page_content(page)
        }
        fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>> {
            self.backend.decode_content(data)
        }
        fn decode_text(&self, scope: ResourceScope, font_name: &[u8], bytes: &[u8]) -> DecodedText {
            self.backend.decode_text(scope, font_name, bytes)
        }
        fn metadata(&self) -> PdfMetadataRaw {
            self.backend.metadata()
        }
        fn page_box(&self, page: PageId) -> PageBox {
            self.backend.page_box(page)
        }
        fn outline(&self) -> Result<Vec<RawOutlineItem>> {
            self.backend.outline()
        }
        fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>> {
            self.backend.page_xobjects(page)
        }
        fn annotation_appearances(&self, page: PageId) -> AnnotationAppearances {
            self.backend.annotation_appearances(page)
        }
        fn xobject(&self, scope: ResourceScope, name: &[u8]) -> Option<PaintedXObject> {
            self.count();
            self.backend.xobject(scope, name)
        }
        fn form_xobject_id(&self, scope: ResourceScope, name: &[u8]) -> Option<ObjectId> {
            self.backend.form_xobject_id(scope, name)
        }
        fn tiling_pattern(&self, scope: ResourceScope, name: &[u8]) -> Option<TilingPattern> {
            self.count();
            self.backend.tiling_pattern(scope, name)
        }
        fn pattern_id(&self, scope: ResourceScope, name: &[u8]) -> Option<ObjectId> {
            self.backend.pattern_id(scope, name)
        }
        fn shading_id(&self, scope: ResourceScope, name: &[u8]) -> Option<ObjectId> {
            self.backend.shading_id(scope, name)
        }
        fn font_id(&self, scope: ResourceScope, font_name: &[u8]) -> Option<ObjectId> {
            self.backend.font_id(scope, font_name)
        }
        fn font_program(
            &self,
            scope: ResourceScope,
            font_name: &[u8],
        ) -> Option<super::super::backend::FontProgram> {
            self.count();
            self.backend.font_program(scope, font_name)
        }
        fn shading(&self, scope: ResourceScope, name: &[u8]) -> Option<Shading> {
            let shading = self.backend.shading(scope, name);
            if shading.is_some() {
                self.count();
            }
            shading
        }
        fn shading_pattern(
            &self,
            scope: ResourceScope,
            name: &[u8],
        ) -> Option<(Shading, [f32; 6])> {
            let pattern = self.backend.shading_pattern(scope, name);
            if pattern.is_some() {
                self.count();
            }
            pattern
        }
        fn ext_gstate_ref(
            &self,
            scope: ResourceScope,
            name: &[u8],
        ) -> Option<(ExtGState, Option<SoftMaskRef>)> {
            self.backend.ext_gstate_ref(scope, name)
        }
        fn ext_gstate(&self, scope: ResourceScope, name: &[u8]) -> Option<ExtGState> {
            let gs = self.backend.ext_gstate(scope, name);
            if let Some(ExtGState {
                soft_mask: Some(SoftMaskEntry::Mask(_)),
                ..
            }) = &gs
            {
                self.count();
            }
            gs
        }
        fn form_xobject(&self, id: ObjectId) -> Option<FormXObject> {
            self.count();
            self.backend.form_xobject(id)
        }
    }

    fn form(entries: &str, content: &str) -> Vec<u8> {
        stream(
            &format!(
                "<</Subtype/Form/BBox[0 0 100 100]{entries}/Length {}>>",
                content.len()
            ),
            content.as_bytes(),
        )
    }

    #[test]
    fn annotations_read_no_form_past_the_step_the_budget_refuses() {
        let backend = CountingForms::load(vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[4 0 R 5 0 R]>>".to_vec(),
            b"<</Subtype/Stamp/Rect[0 0 100 100]/AP<</N 6 0 R>>>>".to_vec(),
            b"<</Subtype/Stamp/Rect[0 0 100 100]/AP<</N 9 0 R>>>>".to_vec(),
            form("/Resources<</XObject<</A 7 0 R/B 8 0 R>>>>", "/A Do /B Do"),
            form("", "0 0 100 100 re f"),
            form("", "0 0 100 100 re f"),
            form("", "0 0 100 100 re f"),
        ]);
        let page = backend.pages()[&1];
        let base = Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, 100.0);
        for (most, read) in [(0, 0), (1, 1)] {
            let mut budget = |work: u64| {
                if work > most {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let white = solid(100, 100, [255; 4]);
            let mut painter = Painter::new(&backend, page, white, base, &mut budget);
            painter.paint_annotations(base);
            assert!(painter.stopped);
            assert_eq!(backend.take_read(), read, "{most}");
        }
    }

    #[test]
    fn a_form_the_page_and_its_annotations_paint_is_read_once() {
        let backend = CountingForms::load(vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[4 0 R 4 0 R 5 0 R]\
              /Resources<</XObject<</Fm 6 0 R>>>>/Contents 8 0 R>>"
                .to_vec(),
            b"<</Subtype/Stamp/Rect[10 10 20 20]/AP<</N 6 0 R>>>>".to_vec(),
            b"<</Subtype/Stamp/Rect[50 50 60 60]/AP<</N 6 0 R>>>>".to_vec(),
            form("/Resources<</XObject<</Child 7 0 R>>>>", "/Child Do"),
            form("", "0 0 1 rg 0 0 100 100 re f"),
            stream("<</Length 6>>", b"/Fm Do"),
        ]);
        let options = RasterOptions {
            dpi: 72.0,
            region: PageRegion::Crop,
        };
        let page = render_page(&backend, 1, &options, &mut |_| ControlFlow::Continue(())).unwrap();
        assert!(page.gaps.is_empty(), "{:?}", page.gaps);
        assert_eq!(backend.take_read(), 2);
    }

    fn painting_through(resources: &str, content: &str, painter: Vec<u8>) -> CountingForms {
        CountingForms::load(vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[4 0 R 5 0 R]>>".to_vec(),
            b"<</Subtype/Stamp/Rect[0 0 50 50]/AP<</N 6 0 R>>>>".to_vec(),
            b"<</Subtype/Stamp/Rect[50 50 100 100]/AP<</N 6 0 R>>>>".to_vec(),
            form(&format!("/Resources<<{resources}>>"), content),
            painter,
            form("", "0 0 10 10 re f"),
            form("", "0 0 10 10 re f"),
        ])
    }

    #[test]
    fn forms_in_an_appearance_s_pattern_or_soft_mask_are_read_once_and_not_past_a_refusal() {
        let cell = "/A Do /B Do";
        let pattern = stream(
            &format!(
                "<</PatternType 1/PaintType 1/TilingType 1/BBox[0 0 10 10]/XStep 10/YStep 10\
                 /Resources<</XObject<</A 8 0 R/B 9 0 R>>>>/Length {}>>",
                cell.len()
            ),
            cell.as_bytes(),
        );
        let group = form(
            "/Group<</S/Transparency>>/Resources<</XObject<</A 8 0 R/B 9 0 R>>>>",
            cell,
        );
        let cases = [
            painting_through(
                "/Pattern<</P 7 0 R>>",
                "/Pattern cs /P scn 0 0 100 100 re f",
                pattern,
            ),
            painting_through(
                "/ExtGState<</M<</SMask<</S/Alpha/G 7 0 R>>>>>>",
                "/M gs 0 0 1 rg 0 0 100 100 re f",
                group,
            ),
        ];
        let options = RasterOptions {
            dpi: 72.0,
            region: PageRegion::Crop,
        };
        for (case, backend) in cases.iter().enumerate() {
            render_page(backend, 1, &options, &mut |_| ControlFlow::Continue(())).unwrap();
            assert_eq!(backend.take_read(), 4, "{case}");
            let refused = render_page(backend, 1, &options, &mut |_| {
                if backend.read.load(Ordering::Relaxed) < 3 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            });
            assert!(refused.is_err(), "{case}");
            assert_eq!(backend.take_read(), 3, "{case}");
        }
    }

    #[test]
    fn appearances_that_share_a_soft_mask_read_its_group_once() {
        let masked = form(
            "/Resources<</ExtGState<</M<</SMask<</S/Alpha/G 8 0 R>>>>>>>>",
            "/M gs 0 0 1 rg 0 0 100 100 re f",
        );
        let backend = CountingForms::load(vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[4 0 R 5 0 R]>>".to_vec(),
            b"<</Subtype/Stamp/Rect[0 0 50 50]/AP<</N 6 0 R>>>>".to_vec(),
            b"<</Subtype/Stamp/Rect[50 50 100 100]/AP<</N 7 0 R>>>>".to_vec(),
            masked.clone(),
            masked,
            form("/Group<</S/Transparency>>", "0 0 50 50 re f"),
        ]);
        let options = RasterOptions {
            dpi: 72.0,
            region: PageRegion::Crop,
        };
        render_page(&backend, 1, &options, &mut |_| ControlFlow::Continue(())).unwrap();
        assert_eq!(backend.take_read(), 3);
    }

    #[test]
    fn appearances_that_share_a_tiling_pattern_read_it_once() {
        let patterned = form(
            "/Resources<</Pattern<</P 8 0 R>>>>",
            "/Pattern cs /P scn 0 0 100 100 re f",
        );
        let cell = "0 0 5 5 re f";
        let backend = CountingForms::load(vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[4 0 R 5 0 R]>>".to_vec(),
            b"<</Subtype/Stamp/Rect[0 0 50 50]/AP<</N 6 0 R>>>>".to_vec(),
            b"<</Subtype/Stamp/Rect[50 50 100 100]/AP<</N 7 0 R>>>>".to_vec(),
            patterned.clone(),
            patterned,
            stream(
                &format!(
                    "<</PatternType 1/PaintType 1/TilingType 1/BBox[0 0 10 10]/XStep 10\
                     /YStep 10/Resources<<>>/Length {}>>",
                    cell.len()
                ),
                cell.as_bytes(),
            ),
        ]);
        let options = RasterOptions {
            dpi: 72.0,
            region: PageRegion::Crop,
        };
        render_page(&backend, 1, &options, &mut |_| ControlFlow::Continue(())).unwrap();
        assert_eq!(backend.take_read(), 3);
    }

    fn two_appearances_sharing(resources: &str, content: &str, shared: Vec<Vec<u8>>) -> usize {
        let painting = form(&format!("/Resources<<{resources}>>"), content);
        let mut objects = vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[4 0 R 5 0 R]>>".to_vec(),
            b"<</Subtype/Stamp/Rect[0 0 50 50]/AP<</N 6 0 R>>>>".to_vec(),
            b"<</Subtype/Stamp/Rect[50 50 100 100]/AP<</N 7 0 R>>>>".to_vec(),
            painting.clone(),
            painting,
        ];
        objects.extend(shared);
        let backend = CountingForms::load(objects);
        let options = RasterOptions {
            dpi: 72.0,
            region: PageRegion::Crop,
        };
        render_page(&backend, 1, &options, &mut |_| ControlFlow::Continue(())).unwrap();
        backend.take_read()
    }

    #[test]
    fn appearances_that_share_a_font_load_it_once() {
        let read = two_appearances_sharing(
            "/Font<</F 8 0 R>>",
            "BT /F 12 Tf 10 10 Td (A) Tj ET",
            vec![b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec()],
        );
        assert_eq!(read, 3);
    }

    #[test]
    fn appearances_that_share_a_shading_read_it_once() {
        let read = two_appearances_sharing(
            "/Shading<</Sh 8 0 R>>/Pattern<</P 9 0 R>>",
            "/Sh sh /Pattern cs /P scn 0 0 10 10 re f",
            vec![
                b"<</ShadingType 2/ColorSpace/DeviceRGB/Coords[0 0 100 0]\
                  /Function<</FunctionType 2/Domain[0 1]/C0[1 0 0]/C1[0 0 1]/N 1>>>>"
                    .to_vec(),
                b"<</PatternType 2/Shading 8 0 R>>".to_vec(),
            ],
        );
        assert_eq!(read, 4);
    }

    #[test]
    fn a_resource_a_form_writes_inline_has_no_id_though_the_page_names_one_alike() {
        let shading = "<</ShadingType 2/ColorSpace/DeviceRGB/Coords[0 0 1 0]\
                       /Function<</FunctionType 2/Domain[0 1]/C0[1 0 0]/C1[1 0 0]/N 1>>>>";
        let pattern = format!("<</PatternType 2/Shading {shading}>>");
        let font = "<</Type/Font/Subtype/Type1/BaseFont/Courier>>";
        let backend = RawBackend::load_bytes(&pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]\
                  /Resources<</Shading<</N 4 0 R>>/Pattern<</N 5 0 R>>/Font<</N 6 0 R>>>>>>"
                    .to_vec(),
                shading.as_bytes().to_vec(),
                pattern.clone().into_bytes(),
                font.as_bytes().to_vec(),
                form(
                    &format!(
                        "/Resources<</Shading<</N {shading}>>/Pattern<</N {pattern}>>\
                         /Font<</N {font}>>>>"
                    ),
                    "",
                ),
            ],
            1,
        ))
        .unwrap();
        let page = backend.pages()[&1];
        let ids = |scope: ResourceScope| {
            (
                backend.shading_id(scope, b"N"),
                backend.pattern_id(scope, b"N"),
                backend.font_id(scope, b"N"),
            )
        };
        assert_eq!(
            ids(ResourceScope::page(page)),
            (Some((4, 0)), Some((5, 0)), Some((6, 0)))
        );
        assert_eq!(ids(ResourceScope::form(page, (7, 0))), (None, None, None));
    }

    fn deep_chain(tries: usize) -> CountingForms {
        let mut objects = vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Annots[4 0 R]>>".to_vec(),
            b"<</Subtype/Stamp/Rect[0 0 100 100]/AP<</N 5 0 R>>>>".to_vec(),
            form("/Resources<</XObject<</Fm 6 0 R>>>>", "/Fm Do"),
        ];
        let nested = |entries: String, content: &str| {
            stream(
                &format!("<</Subtype/Form{entries}/Length {}>>", content.len()),
                content.as_bytes(),
            )
        };
        objects.extend((6..36).map(|n| {
            nested(
                format!("/Resources<</XObject<</Fm {} 0 R>>>>", n + 1),
                "/Fm Do",
            )
        }));
        objects.push(nested(
            "/Resources<</XObject<</Fm 37 0 R>>>>".to_string(),
            &"/Fm Do ".repeat(tries),
        ));
        objects.push(nested(String::new(), "0 0 1 rg 0 0 100 100 re f"));
        CountingForms::load(objects)
    }

    fn paint_appearances(
        backend: &CountingForms,
        budget: &mut dyn FnMut(u64) -> ControlFlow<()>,
    ) -> RasterGaps {
        let page = backend.pages()[&1];
        let base = Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, 100.0);
        let white = solid(100, 100, [255; 4]);
        let mut painter = Painter::new(backend, page, white, base, budget);
        painter.paint_annotations(base);
        painter.gaps
    }

    #[test]
    fn a_form_nested_past_the_limit_is_never_read_and_each_try_is_charged() {
        let tries = |tries: usize| {
            let backend = deep_chain(tries);
            let mut charges = 0;
            let gaps = paint_appearances(&backend, &mut |_| {
                charges += 1;
                ControlFlow::Continue(())
            });
            (backend.take_read(), charges, gaps.annotations)
        };
        let ((read_once, once, lost_once), (read_four, four, lost_four)) = (tries(1), tries(4));
        assert_eq!((read_once, read_four), (32, 32));
        assert_eq!(four - once, 3);
        assert_eq!((lost_once, lost_four), (1, 1));

        let backend = deep_chain(4);
        let mut past_the_chain = 0;
        paint_appearances(&backend, &mut |_| {
            if backend.read.load(Ordering::Relaxed) < 32 {
                return ControlFlow::Continue(());
            }
            past_the_chain += 1;
            if past_the_chain < 2 {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        });
        assert_eq!((backend.take_read(), past_the_chain), (32, 2));
    }

    #[test]
    fn a_color_the_budget_refuses_to_read_or_convert_is_not_set() {
        use super::super::backend::RawBackend;
        use super::super::test_pdf::{pdf, stream};
        let content = b"1 0 0 rg /S0 cs /S1 cs /P0 scn";
        let bytes = pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]\
                  /Resources<</ColorSpace<</S0[/DeviceN[/A/B]/DeviceGray 5 0 R]\
                  /S1[/Pattern[/DeviceN[/A/B]/DeviceGray 5 0 R]]>>/Pattern<</P0 6 0 R>>>>\
                  /Contents 4 0 R>>"
                    .to_vec(),
                stream(&format!("<</Length {}>>", content.len()), content),
                stream(
                    "<</FunctionType 0/Domain[0 1 0 1]/Range[0 1]/Size[2 2]/BitsPerSample 8\
                      /Length 4>>",
                    &[0; 4],
                ),
                b"<</PatternType 2/Shading<</ShadingType 2/ColorSpace/DeviceGray\
                  /Coords[0 0 100 0]/Function<</FunctionType 2/Domain[0 1]/C0[0]/C1[1]/N 1>>>>>>"
                    .to_vec(),
            ],
            1,
        );
        let backend = RawBackend::load_bytes(&bytes).unwrap();
        let page = backend.pages()[&1];
        let ops = form_xobject::page_operations(&backend, page).unwrap().ops;
        let (red, black) = (Rgb(1.0, 0.0, 0.0), Rgb(0.0, 0.0, 0.0));
        for (refused_call, fill) in [(3, red), (4, red), (9, black)] {
            let mut calls = 0;
            let mut budget = |_| {
                calls += 1;
                if calls < refused_call {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            };
            let base = Transform::from_row(1.0, 0.0, 0.0, -1.0, 0.0, 100.0);
            let mut painter =
                Painter::new(&backend, page, solid(100, 100, [255; 4]), base, &mut budget);
            for op in &ops {
                painter.apply(op);
            }
            assert!(painter.stopped, "{refused_call}");
            assert_eq!(painter.state.fill, fill, "{refused_call}");
            assert!(painter.state.fill_pattern_paint.is_none(), "{refused_call}");
        }
    }

    #[test]
    fn an_image_near_its_drawn_size_is_left_alone() {
        let small = downsample(solid(10, 10, [9, 9, 9, 255]), (6.0, 6.0));
        assert_eq!((small.width(), small.height()), (10, 10));
    }
}
