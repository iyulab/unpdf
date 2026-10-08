//! PDF backend abstraction layer.
//!
//! Provides a trait-based interface for PDF operations, isolating
//! the concrete PDF parser from the layout analysis logic.

use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use crate::error::{Error, Result};
use crate::model::{FieldType, FieldValue, FormField};

use super::core14::StandardFont;
use super::encoding::{
    build_encoding_map, decode_with_encoding_map, glyph_name_to_unicode, BaseEncoding,
};
use super::font::{
    is_likely_binary, parse_to_unicode_cmap, parse_truetype_cmap_table, ToUnicodeMap,
};
use super::glyph_metrics::{expand_w_array, CidCoding, FontMetrics, WEntry};
use super::sanitize::sanitize_extracted_text;
use super::text_string::{decode_text_string, decode_text_string_lossy};

/// Page identifier: (object number, generation number).
pub type PageId = (u32, u16);

/// An indirect object's identifier: (object number, generation number).
pub type ObjectId = (u32, u16);

/// Where the names a content stream uses (`/F1 Tf`, `/Im1 Do`) are looked up.
///
/// A page's own content resolves names in the page's `/Resources`. A Form XObject painted on
/// the page is a content stream of its own and resolves them in the form's `/Resources`
/// (ISO 32000-1 §8.10.1) -- the same name can mean a different font there. A form without
/// `/Resources` takes the page's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceScope {
    /// The page being painted.
    pub page: PageId,
    /// The Form XObject whose content is being interpreted; `None` for the page's own content.
    pub form: Option<ObjectId>,
}

impl ResourceScope {
    /// The page's own content.
    pub fn page(page: PageId) -> Self {
        Self { page, form: None }
    }

    /// The content of `form`, painted on `page`.
    pub fn form(page: PageId, form: ObjectId) -> Self {
        Self {
            page,
            form: Some(form),
        }
    }
}

impl From<PageId> for ResourceScope {
    fn from(page: PageId) -> Self {
        Self::page(page)
    }
}

/// Font information returned by the backend.
/// A page's box in default user space: the rectangle `[llx lly urx ury]` of its
/// `/MediaBox` (ISO 32000-1 §14.11.2), corners normalized so `llx <= urx`, `lly <= ury`.
///
/// The origin is part of the box. A page whose box does not start at `(0, 0)` places its
/// content relative to `(llx, lly)`, so a box reduced to width and height mislocates it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageBox {
    pub llx: f32,
    pub lly: f32,
    pub urx: f32,
    pub ury: f32,
}

impl PageBox {
    /// US Letter at the origin — the box assumed when a page declares none.
    pub const LETTER: PageBox = PageBox {
        llx: 0.0,
        lly: 0.0,
        urx: 612.0,
        ury: 792.0,
    };

    pub fn width(&self) -> f32 {
        self.urx - self.llx
    }

    pub fn height(&self) -> f32 {
        self.ury - self.lly
    }

    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }
}

#[derive(Debug, Clone)]
pub struct BackendFontInfo {
    /// Font resource name (key in the page's font dictionary).
    pub name: Vec<u8>,
    /// Base font name (e.g., "Helvetica-Bold").
    pub base_font: String,
    /// Whether the font declares itself bold — by its descriptor's `/FontWeight` or
    /// `/Flags` ForceBold, or its embedded program's own weight — whatever its name says.
    pub bold: bool,
    /// Whether the font declares itself italic — by its descriptor's `/ItalicAngle` or
    /// `/Flags` Italic — whatever its name says.
    pub italic: bool,
}

/// A value from a PDF content stream operand.
#[derive(Debug, Clone)]
pub enum PdfValue {
    Integer(i64),
    Real(f32),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<PdfValue>),
    Other,
}

/// A single operation from a PDF content stream.
#[derive(Debug, Clone)]
pub struct ContentOp {
    pub operator: String,
    pub operands: Vec<PdfValue>,
    /// The Form XObject whose content stream this operation comes from, once a page's
    /// operations have had their forms expanded; `None` for the page's own content. Names
    /// among the operands resolve in [`ResourceScope`] `{ page, form }`.
    pub form: Option<ObjectId>,
}

impl ContentOp {
    /// An operation from the page's own content.
    pub fn new(operator: impl Into<String>, operands: Vec<PdfValue>) -> Self {
        Self {
            operator: operator.into(),
            operands,
            form: None,
        }
    }

    /// The scope the operation's names resolve in, on `page`.
    pub fn scope(&self, page: PageId) -> ResourceScope {
        ResourceScope {
            page,
            form: self.form,
        }
    }
}

/// What a `Do` operator paints, resolved in its [`ResourceScope`].
#[derive(Debug, Clone)]
pub enum PaintedXObject {
    /// A Form XObject: a content stream painted as part of the page.
    Form(FormXObject),
    /// An image XObject.
    Image,
    /// Anything else (a PostScript XObject, an unrecognised subtype).
    Other,
}

/// A Form XObject, ready to be interpreted in place of the `Do` that paints it.
#[derive(Debug, Clone)]
pub struct FormXObject {
    /// The form's object id -- its [`ResourceScope::form`].
    pub id: ObjectId,
    /// `/Matrix`: form space to the user space of the `Do` (identity when absent).
    pub matrix: [f32; 6],
    /// `/BBox`: the rectangle, in form space, that clips what the form paints (§8.10.1).
    /// `None` when absent or malformed — the form is then not clipped.
    pub bbox: Option<PageBox>,
    /// The decoded content stream, or `None` when it could not be decoded.
    pub content: Option<Vec<u8>>,
}

/// Raw metadata from the PDF backend.
#[derive(Debug, Clone, Default)]
pub struct PdfMetadataRaw {
    pub version: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
    pub encrypted: bool,
}

/// A raw outline (bookmark) item from the PDF.
#[derive(Debug, Clone)]
pub struct RawOutlineItem {
    pub title: String,
    pub page: Option<u32>,
    pub level: u8,
    pub children: Vec<RawOutlineItem>,
}

/// An image's color space, as far as converting its samples to pixels needs it.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageColorSpace {
    /// One component: `DeviceGray`, `CalGray`, or an `ICCBased` profile with `/N 1`.
    Gray,
    /// Three components: `DeviceRGB`, `CalRGB`, or `ICCBased` with `/N 3`.
    Rgb,
    /// Four components: `DeviceCMYK`, or `ICCBased` with `/N 4`.
    Cmyk,
    /// One component, an index into `lookup`: `hival + 1` entries of `base.components()`
    /// bytes each.
    Indexed {
        base: Box<ImageColorSpace>,
        hival: u8,
        lookup: Vec<u8>,
    },
}

impl ImageColorSpace {
    /// Components per sample.
    pub fn components(&self) -> usize {
        match self {
            ImageColorSpace::Gray | ImageColorSpace::Indexed { .. } => 1,
            ImageColorSpace::Rgb => 3,
            ImageColorSpace::Cmyk => 4,
        }
    }
}

/// [`RawXObject::filter`] of an image whose filter chain could not be decoded.
pub const UNDECODED_IMAGE: &str = "undecoded";

/// A raw XObject (image) extracted from a PDF page.
#[derive(Debug, Clone)]
pub struct RawXObject {
    pub name: String,
    pub subtype: String,
    pub data: Vec<u8>,
    /// The image codec `data` is still encoded in (`DCTDecode`, `JPXDecode`, ...), or
    /// [`UNDECODED_IMAGE`]; `None` when `data` is the image's samples, every filter of
    /// its chain applied.
    pub filter: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits_per_component: Option<u8>,
    /// The color space's name (`ICCBased` resolved to its device equivalent).
    pub color_space: Option<String>,
    /// The color space as converting samples needs it; `None` when it is not one this crate
    /// converts (`Lab`, `Separation`, `DeviceN`, ...).
    pub color: Option<ImageColorSpace>,
    /// `/Decode`: how each component's raw sample maps onto its range.
    pub decode: Option<Vec<f32>>,
    /// `/ImageMask true`: a stencil — one bit per sample, painted in the current fill color
    /// where the sample is 0 (1 with `/Decode [1 0]`), left untouched elsewhere.
    pub image_mask: bool,
    /// `/SMask`: a grayscale image giving each pixel's opacity.
    pub smask: Option<Box<RawXObject>>,
}

/// The format of an embedded font program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFormat {
    /// `/FontFile2`: a TrueType font.
    TrueType,
    /// `/FontFile3` `/Type1C` or `/CIDFontType0C`: a bare CFF font.
    Cff,
    /// `/FontFile3` `/OpenType`: an OpenType font (TrueType or CFF outlines).
    OpenType,
    /// `/FontFile`: a Type 1 font program (cleartext part, then the `eexec`-encrypted one).
    Type1,
}

/// How a font's codes select glyphs in its program (ISO 32000-1 §9.6.6, §9.7.4.2).
#[derive(Debug, Clone, PartialEq)]
pub enum GlyphSelection {
    /// A composite font under `Identity-H`/`-V`: two-byte codes are CIDs, and `cid_to_gid`
    /// maps a CID to a glyph index (`None`: the CID is the index — `/CIDToGIDMap /Identity`,
    /// or a CFF font, whose charset maps CIDs itself).
    Cid { cid_to_gid: Option<Vec<u16>> },
    /// A simple font: one-byte codes.
    Simple {
        /// What the font's `/Encoding` says each code is, if it has one.
        chars: Option<HashMap<u8, char>>,
        /// The glyph names `/Differences` gives codes.
        names: HashMap<u8, String>,
        /// `/Flags` bit 3: the font's glyphs are outside the standard Latin set, so codes
        /// select glyphs directly rather than through character names.
        symbolic: bool,
    },
}

/// A font's embedded glyph program, and how its codes select glyphs — what painting the
/// font's text needs.
#[derive(Debug, Clone)]
pub struct FontProgram {
    pub format: FontFormat,
    pub data: Vec<u8>,
    pub selection: GlyphSelection,
    /// The font embeds no program and `data` is a standard face standing in for it
    /// (feature `standard-fonts`): its glyphs are fitted to the font's own widths.
    pub stand_in: bool,
}

/// What a page's rasterizer needs from an `/ExtGState` resource (ISO 32000-1 §8.4.5).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExtGState {
    /// `/LW`.
    pub line_width: Option<f32>,
    /// `/CA`: stroking opacity.
    pub stroke_alpha: Option<f32>,
    /// `/ca`: non-stroking opacity.
    pub fill_alpha: Option<f32>,
}

/// A page's decoded content, together with what decoding it had to drop.
///
/// A page's content may be split across several streams. When some of them cannot be
/// decoded the rest still carry content, so the bytes are kept -- but the loss is
/// reported, so a strict caller can fail the page instead of presenting it as complete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageContent {
    /// The decoded content of every stream that could be decoded, in order.
    pub data: Vec<u8>,
    /// Content streams of the page that could not be decoded and were left out of `data`.
    pub undecodable_streams: usize,
}

impl PageContent {
    /// Content decoded without losing anything.
    pub fn complete(data: Vec<u8>) -> Self {
        Self {
            data,
            undecodable_streams: 0,
        }
    }

    /// A page's single content stream, which could not be decoded.
    fn one_undecodable_stream() -> Self {
        Self {
            data: Vec::new(),
            undecodable_streams: 1,
        }
    }
}

/// What a backend had to drop while reading the document structure.
///
/// Reported alongside the extracted text so a caller can tell "this document says
/// what it says" from "this is what survived a damaged file". The default is the
/// intact case: nothing dropped, nothing to warn about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocumentIntegrity {
    /// Page count the document declares (root `Pages` `/Count`), when readable.
    pub declared_page_count: Option<u32>,
    /// Pages the document structure actually yielded, before any page selection.
    pub found_page_count: u32,
    /// Page-tree nodes that could not be used. Non-zero means the page set is
    /// incomplete; it is not a count of lost pages.
    pub unresolved_page_nodes: usize,
    /// Objects the xref table pointed at that could not be loaded.
    pub skipped_object_count: usize,
}

impl DocumentIntegrity {
    /// Whether pages are known to be missing from the output.
    ///
    /// Two independent signals, because neither covers the other: an unusable page-tree
    /// node is a loss the parser observed directly, while a declared count above the
    /// pages found catches losses that left the tree structurally walkable (a truncated
    /// `Kids` array, say). A declared count *below* what was found is not a loss — some
    /// writers simply understate `/Count`.
    pub fn pages_incomplete(&self) -> bool {
        self.unresolved_page_nodes > 0
            || self
                .declared_page_count
                .is_some_and(|declared| declared > self.found_page_count)
    }
}

/// Why a decoder discarded a text run instead of returning what it decoded.
///
/// Both reasons are deliberate policy: emitting mojibake would be worse than emitting
/// nothing. But a discarded run is content the document had and the output does not,
/// so the reason travels with the (empty) result — see [`DecodedText`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSuppression {
    /// A composite (Type0/CID) font whose codes could not be resolved to characters.
    ///
    /// The bytes are CID codes; interpreting them as single-byte characters is
    /// categorically wrong rather than merely lossy, so no threshold is consulted —
    /// every such run is discarded.
    CompositeUnresolved,
    /// A simple font whose fallback decode came out as binary noise.
    ///
    /// Judged by control-character density, so unlike [`Self::CompositeUnresolved`]
    /// this one is a heuristic and can in principle be wrong in either direction.
    BinaryDensity,
}

/// The outcome of decoding one text run.
///
/// `text` is empty whenever `suppressed` is set; the two fields are separate because
/// an empty decode and a discarded decode mean opposite things to a caller trying to
/// report extraction completeness.
#[derive(Debug, Clone, Default)]
pub struct DecodedText {
    /// The decoded text, or empty if nothing was decoded or the run was discarded.
    pub text: String,
    /// Set when the decoder discarded a run it could not read.
    pub suppressed: Option<TextSuppression>,
}

impl DecodedText {
    /// A successful decode (possibly of an empty run).
    pub fn text(text: String) -> Self {
        Self {
            text,
            suppressed: None,
        }
    }

    /// A run the decoder discarded, with the reason.
    pub fn suppressed(reason: TextSuppression) -> Self {
        Self {
            text: String::new(),
            suppressed: Some(reason),
        }
    }
}

/// One glyph's contribution to the text position, as the font declares it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphAdvance {
    /// Horizontal displacement in thousandths of text space (the `w0` of
    /// ISO 32000-1 §9.4.4).
    pub width: f32,
    /// Whether word spacing (`Tw`) applies: only a single-byte code 32 qualifies,
    /// never a multi-byte code, whatever it maps to.
    pub is_word_space: bool,
}

/// Abstract interface for PDF document access.
///
/// Implementations provide page enumeration, font info, content stream
/// decoding, and text decoding — without exposing any concrete PDF library types.
pub trait PdfBackend: Send + Sync {
    /// Return all pages as (page_number → PageId).
    fn pages(&self) -> BTreeMap<u32, PageId>;

    /// Return the fonts the names in `scope` can refer to.
    fn page_fonts(&self, scope: ResourceScope) -> Result<Vec<BackendFontInfo>>;

    /// Return the raw (decompressed) content stream bytes for a page.
    ///
    /// Fails when none of the page's content streams can be decoded. A page split across
    /// several streams may still lose some of them here without failing --
    /// [`page_content_with_losses`](Self::page_content_with_losses) reports those.
    fn page_content(&self, page: PageId) -> Result<Vec<u8>>;

    /// Like [`page_content`](Self::page_content), but also reports content streams that
    /// could not be decoded.
    ///
    /// Defaults to "nothing dropped" so a backend that cannot tell partial decoding apart
    /// does not claim a loss it did not observe -- the same default as [`integrity`](Self::integrity).
    fn page_content_with_losses(&self, page: PageId) -> Result<PageContent> {
        self.page_content(page).map(PageContent::complete)
    }

    /// Parse raw content stream bytes into a sequence of operations.
    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>>;

    /// Decode a text byte sequence using the encoding of the font `font_name` names in
    /// `scope`. Falls back to simple decoding if the font or encoding is unavailable.
    ///
    /// Implementations must return text: no C0/C1 control characters other than
    /// `\n`, `\r` and `\t`. PDF string literals may legally contain control bytes,
    /// and reporting one back as text corrupts every output format and cannot cross
    /// the C ABI at all. Pass results through
    /// [`sanitize_extracted_text`](super::sanitize::sanitize_extracted_text) —
    /// on the way out, so any decode-quality judgement still sees the raw density.
    ///
    /// A decoder that gives up on a run must say so via
    /// [`DecodedText::suppressed`] rather than returning a bare empty string:
    /// "this run was discarded" and "this run held no text" look identical to the
    /// caller otherwise, and the difference is the whole of the caller's diagnostic.
    fn decode_text(&self, scope: ResourceScope, font_name: &[u8], bytes: &[u8]) -> DecodedText;

    /// The advance of each code in `bytes`, as the font `font_name` names in `scope`
    /// declares it.
    ///
    /// `None` when the font's widths are not available to this backend (no `/Widths`,
    /// a CMap other than Identity, a font it cannot resolve): the caller then has no
    /// measured extent for the run and falls back to estimating one. Defaults to
    /// `None` so a backend that cannot read widths never claims a measurement.
    fn glyph_advances(
        &self,
        _scope: ResourceScope,
        _font_name: &[u8],
        _bytes: &[u8],
    ) -> Option<Vec<GlyphAdvance>> {
        None
    }

    /// Return raw metadata (version, info dict fields, encryption status).
    fn metadata(&self) -> PdfMetadataRaw;

    /// The page's box (its `/MediaBox`, inherited from the Pages tree when the page does
    /// not set one). Falls back to [`PageBox::LETTER`] when no usable box is found.
    fn page_box(&self, page: PageId) -> PageBox;

    /// Return page dimensions (width, height) in points — the size of [`page_box`].
    ///
    /// [`page_box`]: PdfBackend::page_box
    fn page_dimensions(&self, page: PageId) -> (f32, f32) {
        let b = self.page_box(page);
        (b.width(), b.height())
    }

    /// The page's visible region: its `/CropBox` (inherited when the page does not set one)
    /// clipped to its box — what a viewer shows. Defaults to [`page_box`] when there is no
    /// crop box, or it does not overlap the page box.
    ///
    /// [`page_box`]: PdfBackend::page_box
    fn crop_box(&self, page: PageId) -> PageBox {
        self.page_box(page)
    }

    /// The page's `/Rotate` (inherited when the page does not set one), normalized to
    /// 0, 90, 180 or 270: how far clockwise the page is turned for display. Defaults to 0.
    fn page_rotation(&self, _page: PageId) -> u16 {
        0
    }

    /// Return the document outline (bookmarks) as a tree.
    /// Implementations must handle cycle detection and depth limits.
    fn outline(&self) -> Result<Vec<RawOutlineItem>>;

    /// Return the image XObjects a page's resources hold, including those of the Form
    /// XObjects it paints. An image reached through a form is named `{form}_{image}`.
    fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>>;

    /// What the XObject `name` is, in `scope`. `None` when the name does not resolve.
    ///
    /// Defaults to `None` -- every `Do` stays opaque -- so a backend that cannot read
    /// XObjects never claims to know what one paints.
    fn xobject(&self, _scope: ResourceScope, _name: &[u8]) -> Option<PaintedXObject> {
        None
    }

    /// The image XObject `name` refers to in `scope`, decoded as far as its filters go —
    /// what a rasterizer paints for `Do`. `None` when the name is not an image.
    fn image_xobject(&self, _scope: ResourceScope, _name: &[u8]) -> Option<RawXObject> {
        None
    }

    /// The `/ExtGState` resource `name` refers to in `scope`.
    fn ext_gstate(&self, _scope: ResourceScope, _name: &[u8]) -> Option<ExtGState> {
        None
    }

    /// The embedded program of the font `font_name` refers to in `scope`, with how its codes
    /// select glyphs. `None` for a font with no program this reads (not embedded, Type 3)
    /// or a composite font under a CMap other than `Identity-H`/`-V`.
    fn font_program(&self, _scope: ResourceScope, _font_name: &[u8]) -> Option<FontProgram> {
        None
    }

    /// The `/ColorSpace` resource `name` refers to in `scope`, as far as converting colors
    /// in it needs (`cs`/`CS` with a named space).
    fn named_color_space(&self, _scope: ResourceScope, _name: &[u8]) -> Option<ImageColorSpace> {
        None
    }

    /// Extract AcroForm fields from the document.
    fn acroform_fields(&self) -> Vec<FormField> {
        vec![]
    }

    /// Report what reading the document structure had to drop.
    ///
    /// Defaults to "nothing dropped" so a backend that cannot distinguish partial
    /// recovery does not claim damage it did not observe.
    fn integrity(&self) -> DocumentIntegrity {
        DocumentIntegrity::default()
    }
}

// Re-export decode_text_simple as pub for external consumers.
pub use super::font::decode_text_simple;

/// Helper: extract a number from a [`PdfValue`].
pub fn get_number_from_value(val: &PdfValue) -> Option<f32> {
    match val {
        PdfValue::Integer(i) => Some(*i as f32),
        PdfValue::Real(r) => Some(*r),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// RawBackend — concrete implementation backed by custom parser
// ---------------------------------------------------------------------------

use super::raw::content as raw_content;
use super::raw::stream as raw_stream;
use super::raw::tokenizer::{
    dict_get as raw_dict_get, PdfDict as RawPdfDict, PdfObject as RawPdfObject,
};
use super::raw::RawDocument;

/// Concrete [`PdfBackend`] backed by the custom `RawDocument` parser.
pub struct RawBackend {
    doc: RawDocument,
    font_resolver: RawFontResolver,
}

impl RawBackend {
    /// Load from a file path.
    pub fn load_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        Self::load_file_with_password(path, None)
    }

    /// Load from a file path, offering `password` to an encrypted document.
    pub fn load_file_with_password<P: AsRef<std::path::Path>>(
        path: P,
        password: Option<&str>,
    ) -> Result<Self> {
        let data = std::fs::read(path).map_err(Error::Io)?;
        Self::load_bytes_with_password(&data, password)
    }

    /// Load from an in-memory byte slice.
    pub fn load_bytes(data: &[u8]) -> Result<Self> {
        Self::load_bytes_with_password(data, None)
    }

    /// Load from an in-memory byte slice, offering `password` to an encrypted document.
    pub fn load_bytes_with_password(data: &[u8], password: Option<&str>) -> Result<Self> {
        let doc = RawDocument::load_with_password(data, password)?;
        Ok(Self {
            doc,
            font_resolver: RawFontResolver::new(),
        })
    }

    /// Load from a reader.
    pub fn load_reader<R: std::io::Read>(reader: R) -> Result<Self> {
        Self::load_reader_with_password(reader, None)
    }

    /// Load from a reader, offering `password` to an encrypted document.
    pub fn load_reader_with_password<R: std::io::Read>(
        mut reader: R,
        password: Option<&str>,
    ) -> Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Self::load_bytes_with_password(&data, password)
    }

    /// Check if the document is encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.doc.is_encrypted()
    }
}

impl PdfBackend for RawBackend {
    fn pages(&self) -> BTreeMap<u32, PageId> {
        self.doc.pages()
    }

    fn integrity(&self) -> DocumentIntegrity {
        let scan = self.doc.scan_page_tree();
        DocumentIntegrity {
            declared_page_count: self.doc.declared_page_count(),
            found_page_count: scan.pages.len() as u32,
            unresolved_page_nodes: scan.unresolved_nodes,
            skipped_object_count: self.doc.skipped_object_count(),
        }
    }

    fn page_fonts(&self, scope: ResourceScope) -> Result<Vec<BackendFontInfo>> {
        Ok(self.font_resolver.page_fonts(&self.doc, scope))
    }

    fn page_content(&self, page_id: PageId) -> Result<Vec<u8>> {
        // Without the loss count an empty result would read as an empty page, so losing
        // every stream is an error on this path.
        let content = self.page_content_with_losses(page_id)?;
        if content.data.is_empty() && content.undecodable_streams > 0 {
            return Err(Error::PdfParse(format!(
                "none of the page's {} content stream(s) could be decoded",
                content.undecodable_streams
            )));
        }
        Ok(content.data)
    }

    fn page_content_with_losses(&self, page_id: PageId) -> Result<PageContent> {
        let page_dict = self
            .doc
            .get_dict(page_id)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        // `/Contents` is optional: a page without it is empty by definition, not damaged,
        // so there is nothing to fail and nothing lost to count.
        let Some(contents) = raw_dict_get(page_dict, b"Contents") else {
            return Ok(PageContent::complete(Vec::new()));
        };

        let contents = self.doc.resolve(contents);

        // A page's only content stream failing to decode is the same loss as one part of a
        // content array failing (below), so it is counted the same way rather than failing
        // here. A strict caller fails the page on the count; a lenient one keeps the page and
        // reports it -- which an error at this point would have left it unable to do.
        match contents {
            RawPdfObject::Reference(n, g) => {
                let obj = self
                    .doc
                    .get_object((*n, *g))
                    .ok_or_else(|| Error::PdfParse("Content stream not found".to_string()))?;
                let resolved = self.doc.resolve(obj);
                if let Some(stream) = resolved.as_stream() {
                    return Ok(raw_stream::decompress(stream).map_or_else(
                        |_| PageContent::one_undecodable_stream(),
                        PageContent::complete,
                    ));
                }
                Err(Error::PdfParse("Invalid content stream".to_string()))
            }
            RawPdfObject::Stream(stream) => Ok(raw_stream::decompress(stream).map_or_else(
                |_| PageContent::one_undecodable_stream(),
                PageContent::complete,
            )),
            RawPdfObject::Array(arr) => {
                let mut content = Vec::new();
                let mut undecodable_streams = 0;
                for item in arr {
                    let resolved = self.doc.resolve(item);
                    let stream_obj = match resolved {
                        RawPdfObject::Stream(s) => s,
                        RawPdfObject::Reference(n, g) => {
                            if let Some(obj) = self.doc.get_object((*n, *g)) {
                                let obj = self.doc.resolve(obj);
                                match obj.as_stream() {
                                    Some(s) => s,
                                    None => continue,
                                }
                            } else {
                                continue;
                            }
                        }
                        _ => continue,
                    };
                    // A part that cannot be decoded is left out and counted, not hidden:
                    // the other parts still carry content, but the page is incomplete.
                    match raw_stream::decompress(stream_obj) {
                        Ok(data) => {
                            content.extend_from_slice(&data);
                            content.push(b' ');
                        }
                        Err(_) => undecodable_streams += 1,
                    }
                }
                Ok(PageContent {
                    data: content,
                    undecodable_streams,
                })
            }
            _ => Err(Error::PdfParse("Invalid content stream".to_string())),
        }
    }

    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>> {
        raw_content::parse_content_stream(data)
    }

    fn decode_text(&self, scope: ResourceScope, font_name: &[u8], bytes: &[u8]) -> DecodedText {
        // Sanitising here — at the outermost return, not inside the decode paths —
        // is deliberate: the inner resolver judges suspect decodes by control-character
        // density, and that evidence must survive until after it has decided.
        let decoded = self
            .font_resolver
            .decode_text(&self.doc, scope, font_name, bytes);
        DecodedText {
            text: sanitize_extracted_text(decoded.text),
            suppressed: decoded.suppressed,
        }
    }

    fn glyph_advances(
        &self,
        scope: ResourceScope,
        font_name: &[u8],
        bytes: &[u8],
    ) -> Option<Vec<GlyphAdvance>> {
        self.font_resolver
            .glyph_advances(&self.doc, scope, font_name, bytes)
    }

    fn metadata(&self) -> PdfMetadataRaw {
        let trailer = self.doc.trailer();
        let mut meta = PdfMetadataRaw {
            version: self.doc.version.clone(),
            encrypted: self.doc.is_encrypted(),
            ..Default::default()
        };

        if let Some(info_ref) = raw_dict_get(trailer, b"Info") {
            if let Some((n, g)) = info_ref.as_reference() {
                if let Ok(info_dict) = self.doc.get_dict((n, g)) {
                    meta.title = raw_get_string(&self.doc, info_dict, b"Title");
                    meta.author = raw_get_string(&self.doc, info_dict, b"Author");
                    meta.subject = raw_get_string(&self.doc, info_dict, b"Subject");
                    meta.keywords = raw_get_string(&self.doc, info_dict, b"Keywords");
                    meta.creator = raw_get_string(&self.doc, info_dict, b"Creator");
                    meta.producer = raw_get_string(&self.doc, info_dict, b"Producer");
                    meta.creation_date = raw_get_string(&self.doc, info_dict, b"CreationDate");
                    meta.mod_date = raw_get_string(&self.doc, info_dict, b"ModDate");
                }
            }
        }

        meta
    }

    fn page_box(&self, page: PageId) -> PageBox {
        inherited_page_attr(&self.doc, page, b"MediaBox")
            .and_then(|obj| page_box_from_array(&self.doc, obj))
            .unwrap_or(PageBox::LETTER)
    }

    fn crop_box(&self, page: PageId) -> PageBox {
        let media = self.page_box(page);
        inherited_page_attr(&self.doc, page, b"CropBox")
            .and_then(|obj| page_box_from_array(&self.doc, obj))
            .and_then(|crop| {
                let clipped = PageBox {
                    llx: crop.llx.max(media.llx),
                    lly: crop.lly.max(media.lly),
                    urx: crop.urx.min(media.urx),
                    ury: crop.ury.min(media.ury),
                };
                (clipped.width() > 0.0 && clipped.height() > 0.0).then_some(clipped)
            })
            .unwrap_or(media)
    }

    fn page_rotation(&self, page: PageId) -> u16 {
        let degrees = inherited_page_attr(&self.doc, page, b"Rotate")
            .map(|obj| self.doc.resolve(obj))
            .and_then(|obj| obj.as_f32())
            .unwrap_or(0.0) as i64;
        // `/Rotate` must be a multiple of 90; anything else is read as the nearest one.
        let quarter = ((degrees as f64 / 90.0).round() as i64).rem_euclid(4);
        (quarter * 90) as u16
    }

    fn outline(&self) -> Result<Vec<RawOutlineItem>> {
        const MAX_DEPTH: u8 = 64;
        let mut items = Vec::new();
        let mut visited = std::collections::HashSet::new();

        let catalog = self.doc.catalog()?;
        if let Some(outlines_obj) = raw_dict_get(catalog, b"Outlines") {
            let outlines_obj = self.doc.resolve(outlines_obj);
            let outlines_dict = match outlines_obj {
                RawPdfObject::Dict(d) => Some(d),
                RawPdfObject::Reference(n, g) => self.doc.get_dict((*n, *g)).ok(),
                _ => None,
            };

            if let Some(outlines_dict) = outlines_dict {
                if let Some(first) = raw_dict_get(outlines_dict, b"First") {
                    if let Some(first_ref) = first.as_reference() {
                        self.collect_outline_items(
                            first_ref,
                            0,
                            MAX_DEPTH,
                            &mut items,
                            &mut visited,
                        );
                    }
                }
            }
        }

        Ok(items)
    }

    fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>> {
        // Fail on a page that is not there, as before; a page without resources has no images.
        self.doc
            .get_dict(page)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let mut xobjects = Vec::new();
        let mut walk = XObjectWalk::default();
        self.collect_images(ResourceScope::page(page), "", 0, &mut walk, &mut xobjects);
        Ok(xobjects)
    }

    fn xobject(&self, scope: ResourceScope, name: &[u8]) -> Option<PaintedXObject> {
        let id = resource_chain(&self.doc, scope)
            .into_iter()
            .find_map(|res| named_resource(&self.doc, res, b"XObject", name))?;
        let stream = self.doc.resolve(self.doc.get_object(id)?).as_stream()?;
        let subtype = raw_dict_get(&stream.dict, b"Subtype").and_then(|s| s.as_name());
        Some(match subtype {
            Some(b"Form") => PaintedXObject::Form(FormXObject {
                id,
                matrix: raw_dict_get(&stream.dict, b"Matrix")
                    .and_then(|m| matrix_from(&self.doc, m))
                    .unwrap_or(IDENTITY_MATRIX),
                bbox: raw_dict_get(&stream.dict, b"BBox")
                    .and_then(|b| page_box_from_array(&self.doc, b)),
                content: raw_stream::decompress(stream).ok(),
            }),
            Some(b"Image") => PaintedXObject::Image,
            _ => PaintedXObject::Other,
        })
    }

    fn image_xobject(&self, scope: ResourceScope, name: &[u8]) -> Option<RawXObject> {
        let id = resource_chain(&self.doc, scope)
            .into_iter()
            .find_map(|res| named_resource(&self.doc, res, b"XObject", name))?;
        let stream = self.doc.resolve(self.doc.get_object(id)?).as_stream()?;
        let subtype = raw_dict_get(&stream.dict, b"Subtype").and_then(|s| s.as_name());
        if subtype != Some(b"Image".as_slice()) {
            return None;
        }
        Some(self.read_image(stream, String::from_utf8_lossy(name).into_owned(), 0))
    }

    fn ext_gstate(&self, scope: ResourceScope, name: &[u8]) -> Option<ExtGState> {
        let dict = resource_chain(&self.doc, scope)
            .into_iter()
            .find_map(|res| {
                let sub = raw_resolve_dict(&self.doc, raw_dict_get(res, b"ExtGState")?)?;
                raw_resolve_dict(&self.doc, raw_dict_get(sub, name)?)
            })?;
        let number = |key: &[u8]| {
            raw_dict_get(dict, key)
                .map(|v| self.doc.resolve(v))
                .and_then(|v| v.as_f32())
        };
        Some(ExtGState {
            line_width: number(b"LW"),
            stroke_alpha: number(b"CA"),
            fill_alpha: number(b"ca"),
        })
    }

    fn font_program(&self, scope: ResourceScope, font_name: &[u8]) -> Option<FontProgram> {
        self.font_resolver.font_program(&self.doc, scope, font_name)
    }

    fn named_color_space(&self, scope: ResourceScope, name: &[u8]) -> Option<ImageColorSpace> {
        resource_chain(&self.doc, scope)
            .into_iter()
            .find_map(|res| {
                let sub = raw_resolve_dict(&self.doc, raw_dict_get(res, b"ColorSpace")?)?;
                resolve_image_color_space(&self.doc, raw_dict_get(sub, name)?, 0)
            })
    }

    fn acroform_fields(&self) -> Vec<FormField> {
        self.extract_acroform_fields()
    }
}

/// What [`RawBackend::collect_images`] has already seen on one page.
#[derive(Default)]
struct XObjectWalk {
    /// Forms already descended into -- a form that paints itself (or an ancestor) is entered once.
    forms: std::collections::HashSet<ObjectId>,
    /// Images already listed -- one image reached by several paths is listed once.
    images: std::collections::HashSet<ObjectId>,
}

/// Forms nested deeper than this are not descended into. Real documents nest a handful of
/// levels; the cap bounds a hostile chain of distinct forms.
pub(crate) const MAX_FORM_DEPTH: usize = 32;

impl RawBackend {
    /// Read an image XObject stream: its samples (the lossless filters applied), geometry,
    /// color space, and — `depth` bounding the chain — its soft mask.
    fn read_image(
        &self,
        stream: &super::raw::tokenizer::PdfStream,
        name: String,
        depth: usize,
    ) -> RawXObject {
        let dict = &stream.dict;
        // The lossless filters are applied whatever the chain (`[/ASCII85Decode
        // /DCTDecode]` hands on the JPEG, `[/ASCII85Decode /FlateDecode]` the
        // samples); `filter` names the image codec still to apply, `None` when `data`
        // is samples. A chain that does not decode leaves the stream's bytes, marked
        // with the reason so they are never read as samples.
        let (data, filter) = match raw_stream::decode(stream) {
            Ok(decoded) => (decoded.data, decoded.codec.map(str::to_string)),
            Err(_) => (stream.raw_data.clone(), Some(UNDECODED_IMAGE.to_string())),
        };
        let int = |key: &[u8]| {
            raw_dict_get(dict, key)
                .map(|v| self.doc.resolve(v))
                .and_then(|v| v.as_i64())
        };
        let image_mask = matches!(
            raw_dict_get(dict, b"ImageMask").map(|v| self.doc.resolve(v)),
            Some(RawPdfObject::Bool(true))
        );
        let color_space_entry = raw_dict_get(dict, b"ColorSpace");
        let smask = (depth == 0)
            .then(|| raw_dict_get(dict, b"SMask"))
            .flatten()
            .and_then(|v| self.doc.resolve(v).as_stream())
            .map(|mask| Box::new(self.read_image(mask, format!("{name}_smask"), depth + 1)));
        RawXObject {
            name,
            subtype: "Image".to_string(),
            data,
            filter,
            width: int(b"Width").map(|w| w as u32),
            height: int(b"Height").map(|h| h as u32),
            bits_per_component: if image_mask {
                Some(1)
            } else {
                int(b"BitsPerComponent").map(|b| b as u8)
            },
            color_space: color_space_entry.and_then(|cs| resolve_color_space_name(&self.doc, cs)),
            color: color_space_entry.and_then(|cs| resolve_image_color_space(&self.doc, cs, 0)),
            decode: raw_dict_get(dict, b"Decode").and_then(|d| numbers_from(&self.doc, d)),
            image_mask,
            smask,
        }
    }

    /// List the images in `scope`'s XObject resources, descending into its forms.
    fn collect_images(
        &self,
        scope: ResourceScope,
        prefix: &str,
        depth: usize,
        walk: &mut XObjectWalk,
        out: &mut Vec<RawXObject>,
    ) {
        // Listing (unlike looking up one name) uses only the innermost resources: a form's
        // resources are its own, and a page's inherited ones are the page's.
        let Some(res) = resource_chain(&self.doc, scope).into_iter().next() else {
            return;
        };
        let Some(xobj_dict) =
            raw_dict_get(res, b"XObject").and_then(|x| raw_resolve_dict(&self.doc, x))
        else {
            return;
        };

        for (name, obj) in xobj_dict {
            let Some(id) = obj.as_reference() else {
                continue;
            };
            let Some(raw_obj) = self.doc.get_object(id) else {
                continue;
            };
            let Some(stream) = self.doc.resolve(raw_obj).as_stream() else {
                continue;
            };
            let dict = &stream.dict;
            let label = format!("{prefix}{}", String::from_utf8_lossy(name));

            let subtype = raw_dict_get(dict, b"Subtype")
                .and_then(|s| s.as_name())
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap_or_default();

            if subtype == "Form" {
                if depth < MAX_FORM_DEPTH && walk.forms.insert(id) {
                    let inner = ResourceScope::form(scope.page, id);
                    self.collect_images(inner, &format!("{label}_"), depth + 1, walk, out);
                }
                continue;
            }
            if subtype != "Image" || !walk.images.insert(id) {
                continue;
            }

            out.push(self.read_image(stream, label, 0));
        }
    }
}

const IDENTITY_MATRIX: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// A six-number matrix array (`/Matrix [a b c d e f]`).
fn matrix_from(doc: &RawDocument, obj: &RawPdfObject) -> Option<[f32; 6]> {
    let RawPdfObject::Array(items) = doc.resolve(obj) else {
        return None;
    };
    if items.len() != 6 {
        return None;
    }
    let mut m = [0.0f32; 6];
    for (slot, item) in m.iter_mut().zip(items) {
        *slot = match doc.resolve(item) {
            RawPdfObject::Integer(i) => *i as f32,
            RawPdfObject::Real(r) => *r as f32,
            _ => return None,
        };
    }
    Some(m)
}

/// The program a font's (or CIDFont's) descriptor embeds, in a format this reads:
/// `/FontFile` (Type 1), `/FontFile2` (TrueType) or `/FontFile3` with `/Subtype` `/Type1C`,
/// `/CIDFontType0C` (bare CFF) or `/OpenType`.
fn embedded_font_program(
    doc: &RawDocument,
    font_dict: &RawPdfDict,
) -> Option<(FontFormat, Vec<u8>)> {
    let descriptor = raw_resolve_dict(doc, raw_dict_get(font_dict, b"FontDescriptor")?)?;
    let stream_at = |key: &[u8]| {
        raw_dict_get(descriptor, key)
            .map(|v| doc.resolve(v))
            .and_then(|v| v.as_stream())
    };
    let read = |stream: &super::raw::tokenizer::PdfStream| {
        raw_stream::decompress(stream).unwrap_or_else(|_| stream.raw_data.clone())
    };
    if let Some(stream) = stream_at(b"FontFile2") {
        return Some((FontFormat::TrueType, read(stream)));
    }
    if let Some(stream) = stream_at(b"FontFile") {
        return Some((FontFormat::Type1, read(stream)));
    }
    let stream = stream_at(b"FontFile3")?;
    let format = match raw_dict_get(&stream.dict, b"Subtype").and_then(|s| s.as_name()) {
        Some(b"OpenType") => FontFormat::OpenType,
        Some(b"Type1C") | Some(b"CIDFontType0C") => FontFormat::Cff,
        _ => return None,
    };
    Some((format, read(stream)))
}

/// Whether a font program's weight name is a bold one: Bold, Semibold, Demibold, Extrabold,
/// Black, Heavy and the like — not Medium, Book or Regular.
fn weight_name_is_bold(weight: &str) -> bool {
    let w = weight.to_ascii_lowercase();
    ["bold", "black", "heavy", "demi"]
        .iter()
        .any(|t| w.contains(t))
}

#[cfg(test)]
mod weight_tests {
    use super::*;

    #[test]
    fn bold_weight_names() {
        for w in [
            "Bold",
            "Semibold",
            "DemiBold",
            "Demi",
            "ExtraBold",
            "Black",
            "Heavy",
        ] {
            assert!(weight_name_is_bold(w), "{w}");
        }
        for w in ["Regular", "Medium", "Book", "Light", "Roman"] {
            assert!(!weight_name_is_bold(w), "{w}");
        }
    }

    #[test]
    fn the_os2_weight_class_is_found_by_table_tag() {
        // An sfnt with two table records; OS/2 at offset 48, usWeightClass 700.
        let mut sfnt = vec![0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        sfnt.extend(b"head");
        sfnt.extend([0; 4]);
        sfnt.extend(40u32.to_be_bytes());
        sfnt.extend(4u32.to_be_bytes());
        sfnt.extend(b"OS/2");
        sfnt.extend([0; 4]);
        sfnt.extend(48u32.to_be_bytes());
        sfnt.extend(6u32.to_be_bytes());
        sfnt.extend([0; 4]);
        sfnt.extend([0, 3, 0, 0, 2, 188]);
        assert_eq!(os2_weight_class(&sfnt), Some(700));
        assert_eq!(os2_weight_class(b"nope"), None);
    }
}

/// A TrueType or OpenType program's OS/2 `usWeightClass` (400 regular, 700 bold).
fn os2_weight_class(data: &[u8]) -> Option<u16> {
    let u16_at = |at: usize| Some(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]));
    let u32_at = |at: usize| {
        let b = data.get(at..at + 4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let tables = usize::from(u16_at(4)?);
    (0..tables).find_map(|i| {
        let record = 12 + 16 * i;
        (data.get(record..record + 4)? == b"OS/2")
            .then(|| u16_at(usize::try_from(u32_at(record + 8)?).ok()? + 4))
            .flatten()
    })
}

/// A page attribute the page may inherit from its Pages-tree ancestors (ISO 32000-1
/// §7.7.3.4 — `/MediaBox`, `/CropBox`, `/Rotate`, `/Resources`): the page's own value, or
/// the nearest ancestor's. The walk stops at a repeated node or after 64 levels, so a
/// `/Parent` cycle in a damaged file ends the search instead of the process.
fn inherited_page_attr<'d>(
    doc: &'d RawDocument,
    page: PageId,
    key: &[u8],
) -> Option<&'d RawPdfObject> {
    const MAX_TREE_DEPTH: usize = 64;
    let mut node = Some(page);
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = node {
        if seen.len() >= MAX_TREE_DEPTH || !seen.insert(id) {
            return None;
        }
        let dict = doc.get_dict(id).ok()?;
        if let Some(value) = raw_dict_get(dict, key) {
            return Some(value);
        }
        node = raw_dict_get(dict, b"Parent").and_then(|p| p.as_reference());
    }
    None
}

/// A rectangle array `[x1 y1 x2 y2]` as a [`PageBox`], its numbers resolved through
/// indirect references. `None` for anything else, or a box with no area.
fn page_box_from_array(doc: &RawDocument, obj: &RawPdfObject) -> Option<PageBox> {
    let arr = doc.resolve(obj).as_array()?;
    let n = |i: usize| arr.get(i).map(|v| doc.resolve(v)).and_then(|v| v.as_f32());
    let (x1, y1, x2, y2) = (n(0)?, n(1)?, n(2)?, n(3)?);
    let page_box = PageBox {
        llx: x1.min(x2),
        lly: y1.min(y2),
        urx: x1.max(x2),
        ury: y1.max(y2),
    };
    (page_box.width() > 0.0 && page_box.height() > 0.0).then_some(page_box)
}

/// The resource dictionaries the names in `scope` resolve through, innermost first: the
/// form's own `/Resources`, then the page's, then each Pages-tree ancestor's -- resources a
/// page inherits (ISO 32000-1 §7.7.3.4).
///
/// Looking a name up walks the whole chain, so a form or page that omits a resource its
/// content uses still finds it further out -- the lenient reading every viewer applies.
fn resource_chain(doc: &RawDocument, scope: ResourceScope) -> Vec<&RawPdfDict> {
    const MAX_TREE_DEPTH: usize = 64;
    let resources_of = |id: ObjectId| {
        let dict = doc.get_dict(id).ok()?;
        raw_dict_get(dict, b"Resources").and_then(|r| raw_resolve_dict(doc, r))
    };

    let mut chain = Vec::new();
    if let Some(res) = scope.form.and_then(resources_of) {
        chain.push(res);
    }
    let mut node = Some(scope.page);
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = node {
        if seen.len() >= MAX_TREE_DEPTH || !seen.insert(id) {
            break;
        }
        if let Some(res) = resources_of(id) {
            chain.push(res);
        }
        node = doc
            .get_dict(id)
            .ok()
            .and_then(|d| raw_dict_get(d, b"Parent"))
            .and_then(|p| p.as_reference());
    }
    chain
}

/// The object `name` refers to in the `category` sub-dictionary (`/Font`, `/XObject`) of
/// `resources`.
fn named_resource(
    doc: &RawDocument,
    resources: &RawPdfDict,
    category: &[u8],
    name: &[u8],
) -> Option<ObjectId> {
    let sub = raw_resolve_dict(doc, raw_dict_get(resources, category)?)?;
    raw_dict_get(sub, name)?.as_reference()
}

/// Resolve an image XObject's `/ColorSpace` entry to a name.
///
/// `/ICCBased` is the common case for Office-exported images (an embedded sRGB/greyscale/CMYK
/// ICC profile) but carries no component count of its own in the name — that lives on the
/// referenced profile stream's `/N` entry. Consumers that gate on device color space (the PNG
/// re-encoder among them) would otherwise see every `ICCBased` image as unrecognized, so this
/// resolves `/N` (1/3/4 components) to the equivalent `Device*` name up front.
/// Resolve an image's `/ColorSpace` to the structure converting its samples needs.
///
/// `depth` bounds the recursion an `Indexed` base or an `ICCBased` `/Alternate` takes.
fn resolve_image_color_space(
    doc: &RawDocument,
    cs: &RawPdfObject,
    depth: usize,
) -> Option<ImageColorSpace> {
    if depth > 4 {
        return None;
    }
    let by_components = |n: i64| match n {
        1 => Some(ImageColorSpace::Gray),
        3 => Some(ImageColorSpace::Rgb),
        4 => Some(ImageColorSpace::Cmyk),
        _ => None,
    };
    match doc.resolve(cs) {
        RawPdfObject::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Some(ImageColorSpace::Gray),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(ImageColorSpace::Rgb),
            b"DeviceCMYK" | b"CMYK" => Some(ImageColorSpace::Cmyk),
            _ => None,
        },
        RawPdfObject::Array(arr) => {
            let family = arr
                .first()
                .map(|o| doc.resolve(o))
                .and_then(|o| o.as_name())?;
            match family {
                b"CalGray" => Some(ImageColorSpace::Gray),
                b"CalRGB" => Some(ImageColorSpace::Rgb),
                b"ICCBased" => {
                    let profile = doc.resolve(arr.get(1)?).as_stream()?;
                    raw_dict_get(&profile.dict, b"N")
                        .and_then(|n| doc.resolve(n).as_i64())
                        .and_then(by_components)
                        .or_else(|| {
                            raw_dict_get(&profile.dict, b"Alternate")
                                .and_then(|alt| resolve_image_color_space(doc, alt, depth + 1))
                        })
                }
                b"Indexed" | b"I" => {
                    let base = resolve_image_color_space(doc, arr.get(1)?, depth + 1)?;
                    if matches!(base, ImageColorSpace::Indexed { .. }) {
                        return None; // ISO 32000-1 §8.6.6.3: the base cannot be Indexed
                    }
                    let hival = doc.resolve(arr.get(2)?).as_i64()?.clamp(0, 255) as u8;
                    let lookup = match doc.resolve(arr.get(3)?) {
                        RawPdfObject::Str(bytes) => bytes.clone(),
                        RawPdfObject::Stream(stream) => raw_stream::decompress(stream).ok()?,
                        _ => return None,
                    };
                    Some(ImageColorSpace::Indexed {
                        base: Box::new(base),
                        hival,
                        lookup,
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// An array of numbers (`/Decode [1 0 1 0]`).
fn numbers_from(doc: &RawDocument, obj: &RawPdfObject) -> Option<Vec<f32>> {
    let RawPdfObject::Array(items) = doc.resolve(obj) else {
        return None;
    };
    items
        .iter()
        .map(|item| match doc.resolve(item) {
            RawPdfObject::Integer(i) => Some(*i as f32),
            RawPdfObject::Real(r) => Some(*r as f32),
            _ => None,
        })
        .collect()
}

fn resolve_color_space_name(doc: &RawDocument, cs: &RawPdfObject) -> Option<String> {
    match cs {
        RawPdfObject::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
        RawPdfObject::Array(arr) => {
            let first_name = arr
                .first()
                .and_then(|o| o.as_name())
                .map(|n| String::from_utf8_lossy(n).to_string())?;
            if first_name == "ICCBased" {
                if let Some(profile) = arr.get(1).map(|second| doc.resolve(second)) {
                    if let Some(stream) = profile.as_stream() {
                        if let Some(components) =
                            raw_dict_get(&stream.dict, b"N").and_then(|n| n.as_i64())
                        {
                            return Some(
                                match components {
                                    1 => "DeviceGray",
                                    3 => "DeviceRGB",
                                    4 => "DeviceCMYK",
                                    _ => return Some(first_name),
                                }
                                .to_string(),
                            );
                        }
                    }
                }
            }
            Some(first_name)
        }
        _ => None,
    }
}

impl RawBackend {
    /// Extract AcroForm fields from the document.
    fn extract_acroform_fields(&self) -> Vec<FormField> {
        let catalog = match self.doc.catalog() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let acroform = match raw_dict_get(catalog, b"AcroForm") {
            Some(obj) => self.doc.resolve(obj),
            None => return vec![],
        };

        let acroform_dict = match acroform {
            RawPdfObject::Dict(d) => d,
            RawPdfObject::Reference(n, g) => match self.doc.get_dict((*n, *g)) {
                Ok(d) => d,
                Err(_) => return vec![],
            },
            _ => return vec![],
        };

        let fields = match raw_dict_get(acroform_dict, b"Fields") {
            Some(obj) => self.doc.resolve(obj),
            None => return vec![],
        };

        let field_refs = match fields {
            RawPdfObject::Array(arr) => arr,
            RawPdfObject::Reference(n, g) => match self.doc.get_object((*n, *g)) {
                Some(obj) => match self.doc.resolve(obj).as_array() {
                    Some(arr) => arr,
                    None => return vec![],
                },
                None => return vec![],
            },
            _ => return vec![],
        };

        let mut result = Vec::new();
        for field_ref in field_refs {
            if let Some(id) = field_ref.as_reference() {
                self.traverse_field_tree(id, String::new(), None, &mut result);
            }
        }
        result
    }

    fn traverse_field_tree(
        &self,
        field_id: PageId,
        parent_name: String,
        inherited_ft: Option<Vec<u8>>,
        result: &mut Vec<FormField>,
    ) {
        let dict = match self.doc.get_dict(field_id) {
            Ok(d) => d,
            Err(_) => return,
        };

        // Build qualified name
        let partial_name = raw_dict_get(dict, b"T")
            .and_then(|o| o.as_str_bytes())
            .map(sanitize_field_string);

        let qualified_name = match &partial_name {
            Some(name) if parent_name.is_empty() => name.clone(),
            Some(name) => format!("{}.{}", parent_name, name),
            None => parent_name.clone(),
        };

        // Get field type (may be inherited from parent)
        let ft = raw_dict_get(dict, b"FT")
            .and_then(|o| o.as_name())
            .map(|n| n.to_vec())
            .or(inherited_ft.clone());

        // Check for Kids (non-terminal field)
        if let Some(kids) = raw_dict_get(dict, b"Kids") {
            let kids = self.doc.resolve(kids);
            if let Some(kids_arr) = kids.as_array() {
                for kid in kids_arr {
                    if let Some(kid_id) = kid.as_reference() {
                        self.traverse_field_tree(
                            kid_id,
                            qualified_name.clone(),
                            ft.clone(),
                            result,
                        );
                    }
                }
                return;
            }
        }

        // Terminal field — extract value
        let ft_bytes = match &ft {
            Some(ft) => ft.as_slice(),
            None => return,
        };

        let ff = raw_dict_get(dict, b"Ff")
            .and_then(|o| o.as_i64())
            .unwrap_or(0) as u32;

        let field_type = match ft_bytes {
            b"Tx" => FieldType::Text,
            b"Btn" => {
                if ff & (1 << 16) != 0 {
                    FieldType::RadioButton
                } else if ff & (1 << 17) != 0 {
                    FieldType::PushButton
                } else {
                    FieldType::Checkbox
                }
            }
            b"Ch" => {
                if ff & (1 << 17) != 0 {
                    FieldType::Dropdown
                } else {
                    FieldType::ListBox
                }
            }
            b"Sig" => FieldType::Signature,
            _ => return,
        };

        let value = self.extract_field_value(dict, &field_type);
        let default_value =
            raw_dict_get(dict, b"DV").and_then(|o| self.pdf_obj_to_field_value(o, &field_type));

        result.push(FormField {
            name: qualified_name,
            field_type,
            value,
            default_value,
        });
    }

    fn extract_field_value(&self, dict: &RawPdfDict, field_type: &FieldType) -> Option<FieldValue> {
        let v = raw_dict_get(dict, b"V")?;
        self.pdf_obj_to_field_value(v, field_type)
    }

    fn pdf_obj_to_field_value(
        &self,
        obj: &RawPdfObject,
        field_type: &FieldType,
    ) -> Option<FieldValue> {
        let obj = self.doc.resolve(obj);
        match field_type {
            FieldType::Text => obj
                .as_str_bytes()
                .map(|s| FieldValue::Text(sanitize_field_string(s))),
            FieldType::Checkbox | FieldType::RadioButton => {
                obj.as_name().map(|n| FieldValue::Boolean(n != b"Off"))
            }
            FieldType::Dropdown | FieldType::ListBox => {
                if let Some(s) = obj.as_str_bytes() {
                    Some(FieldValue::Choice(sanitize_field_string(s)))
                } else if let Some(arr) = obj.as_array() {
                    let choices: Vec<String> = arr
                        .iter()
                        .filter_map(|o| o.as_str_bytes())
                        .map(sanitize_field_string)
                        .collect();
                    Some(FieldValue::Choices(choices))
                } else {
                    None
                }
            }
            FieldType::PushButton | FieldType::Signature => None,
        }
    }

    /// Find MediaBox for a page, walking up the page tree for inherited values.
    /// Collect outline items by following First/Next chain.
    fn collect_outline_items(
        &self,
        item_ref: PageId,
        level: u8,
        max_depth: u8,
        items: &mut Vec<RawOutlineItem>,
        visited: &mut std::collections::HashSet<PageId>,
    ) {
        if !visited.insert(item_ref) || level > max_depth {
            return;
        }

        if let Ok(item_dict) = self.doc.get_dict(item_ref) {
            let title = raw_get_string(&self.doc, item_dict, b"Title").unwrap_or_default();
            let page = self.resolve_outline_dest(item_dict);

            let mut outline_item = RawOutlineItem {
                title,
                page,
                level,
                children: Vec::new(),
            };

            if let Some(first) = raw_dict_get(item_dict, b"First") {
                if let Some(first_ref) = first.as_reference() {
                    self.collect_outline_items(
                        first_ref,
                        level + 1,
                        max_depth,
                        &mut outline_item.children,
                        visited,
                    );
                }
            }

            items.push(outline_item);

            if let Some(next) = raw_dict_get(item_dict, b"Next") {
                if let Some(next_ref) = next.as_reference() {
                    self.collect_outline_items(next_ref, level, max_depth, items, visited);
                }
            }
        }
    }

    /// Resolve an outline destination to a page number.
    fn resolve_outline_dest(&self, item_dict: &RawPdfDict) -> Option<u32> {
        let pages = self.doc.pages();

        // Try Dest
        if let Some(dest) = raw_dict_get(item_dict, b"Dest") {
            let dest = self.doc.resolve(dest);
            if let Some(arr) = dest.as_array() {
                if let Some(first) = arr.first() {
                    if let Some(page_ref) = first.as_reference() {
                        for (num, id) in pages.iter() {
                            if *id == page_ref {
                                return Some(*num);
                            }
                        }
                    }
                }
            }
        }

        // Try A (action) dictionary
        if let Some(action) = raw_dict_get(item_dict, b"A") {
            let action = self.doc.resolve(action);
            let action_dict = match action {
                RawPdfObject::Dict(d) => Some(d),
                RawPdfObject::Reference(n, g) => self.doc.get_dict((*n, *g)).ok(),
                _ => None,
            };

            if let Some(action_dict) = action_dict {
                if let Some(dest) = raw_dict_get(action_dict, b"D") {
                    let dest = self.doc.resolve(dest);
                    if let Some(arr) = dest.as_array() {
                        if let Some(first) = arr.first() {
                            if let Some(page_ref) = first.as_reference() {
                                for (num, id) in pages.iter() {
                                    if *id == page_ref {
                                        return Some(*num);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// RawFontResolver — font resolution for RawBackend
// ---------------------------------------------------------------------------

/// The weight and slant a font declares of itself — see `RawFontResolver::declared_style`.
#[derive(Debug, Clone, Copy, Default)]
struct DeclaredStyle {
    bold: bool,
    italic: bool,
}

struct RawFontResolver {
    cmap_cache: RwLock<HashMap<PageId, Option<ToUnicodeMap>>>,
    /// The `cmap` table of a composite font's embedded TrueType program, by the Type 0
    /// font's object: its CIDFont may be written inline, with no object of its own.
    embedded_cmap_cache: RwLock<HashMap<PageId, Option<ToUnicodeMap>>>,
    encoding_cache: RwLock<HashMap<PageId, Option<HashMap<u8, char>>>>,
    program_encoding_cache: RwLock<HashMap<PageId, Option<HashMap<u8, char>>>>,
    style_cache: RwLock<HashMap<PageId, DeclaredStyle>>,
    cid_system_info_cache: RwLock<HashMap<PageId, Option<(String, String)>>>,
    metrics_cache: RwLock<HashMap<PageId, Option<FontMetrics>>>,
}

impl RawFontResolver {
    fn new() -> Self {
        Self {
            cmap_cache: RwLock::new(HashMap::new()),
            embedded_cmap_cache: RwLock::new(HashMap::new()),
            encoding_cache: RwLock::new(HashMap::new()),
            program_encoding_cache: RwLock::new(HashMap::new()),
            style_cache: RwLock::new(HashMap::new()),
            cid_system_info_cache: RwLock::new(HashMap::new()),
            metrics_cache: RwLock::new(HashMap::new()),
        }
    }

    fn glyph_advances(
        &self,
        doc: &RawDocument,
        scope: ResourceScope,
        font_name: &[u8],
        bytes: &[u8],
    ) -> Option<Vec<GlyphAdvance>> {
        let fid = self.find_font_dict(doc, scope, font_name)?;
        {
            let cache = self.metrics_cache.read().unwrap();
            if let Some(cached) = cache.get(&fid) {
                return cached.as_ref().map(|m| m.advances(bytes));
            }
        }
        let metrics = self.parse_font_metrics(doc, fid);
        let advances = metrics.as_ref().map(|m| m.advances(bytes));
        self.metrics_cache.write().unwrap().insert(fid, metrics);
        advances
    }

    /// Read the advance widths a font dictionary declares (ISO 32000-1 §9.2.4, §9.7.4.3).
    ///
    /// Only fonts whose codes resolve to glyphs without guessing are measured: simple
    /// fonts (one byte per code), and composite fonts in horizontal writing mode under
    /// `Identity-H` (two-byte code = CID) or a predefined CMap whose code → CID table
    /// ships with the crate. Anything else — a vertical CMap, whose advances are `/W2`'s,
    /// or an embedded CMap — returns `None` rather than a width tied to the wrong code.
    fn parse_font_metrics(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<FontMetrics> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let number = |obj: &RawPdfObject| doc.resolve(obj).as_f32();

        if self.is_composite_font(doc, font_obj_id) {
            let cmap = raw_dict_get(font_dict, b"Encoding").and_then(|e| e.as_name())?;
            let coding = if cmap == b"Identity-H" {
                CidCoding::Identity
            } else {
                let cmap = String::from_utf8_lossy(cmap).into_owned();
                let horizontal = cmap.ends_with("-H") || cmap == "H";
                let (_, ordering) = self.get_cid_system_info_cached(doc, font_obj_id)?;
                if !horizontal || !super::predefined_cmap::resolves_cids(&cmap, &ordering) {
                    return None;
                }
                CidCoding::Predefined { cmap, ordering }
            };
            let cid_font = self.cid_font_dict(doc, font_obj_id)?;
            let default_width = raw_dict_get(cid_font, b"DW")
                .and_then(number)
                .unwrap_or(1000.0);
            let entries: Vec<WEntry> = raw_dict_get(cid_font, b"W")
                .and_then(|w| doc.resolve(w).as_array())
                .map(|items| {
                    items
                        .iter()
                        .map_while(|item| match doc.resolve(item) {
                            RawPdfObject::Array(list) => {
                                Some(WEntry::Array(list.iter().filter_map(number).collect()))
                            }
                            other => other.as_f32().map(WEntry::Number),
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Some(FontMetrics::Cid {
                widths: expand_w_array(&entries),
                default_width,
                coding,
            });
        }

        let declared = raw_dict_get(font_dict, b"FirstChar")
            .and_then(number)
            .zip(raw_dict_get(font_dict, b"Widths").and_then(|w| doc.resolve(w).as_array()));
        let Some((first_char, widths)) = declared else {
            return self.standard_font_metrics(doc, font_obj_id);
        };
        let first_char = first_char as u32;
        let widths: Vec<f32> = widths.iter().map(|w| number(w).unwrap_or(0.0)).collect();
        let missing_width = raw_dict_get(font_dict, b"FontDescriptor")
            .and_then(|d| raw_resolve_dict(doc, d))
            .and_then(|d| raw_dict_get(d, b"MissingWidth"))
            .and_then(number)
            .unwrap_or(0.0);

        // Type 3 glyph widths are in glyph space; FontMatrix maps them to text space.
        // Every other simple font is already in thousandths of text space.
        let is_type3 = raw_dict_get(font_dict, b"Subtype")
            .and_then(|s| s.as_name())
            .is_some_and(|n| n == b"Type3");
        let scale = if is_type3 {
            raw_dict_get(font_dict, b"FontMatrix")
                .and_then(|m| doc.resolve(m).as_array())
                .and_then(|m| m.first())
                .and_then(number)
                .map(|a| a * 1000.0)?
        } else {
            1.0
        };

        Some(FontMetrics::Simple {
            first_char,
            widths: widths.into_iter().map(|w| w * scale).collect(),
            missing_width: missing_width * scale,
        })
    }

    /// Widths for a simple font that declares none: one of the standard 14, named
    /// by `/BaseFont` alone (ISO 32000-1 §9.6.2.2). Any other font without
    /// `/Widths` stays unmeasured.
    fn standard_font_metrics(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<FontMetrics> {
        let standard = self.standard_font(doc, font_obj_id)?;
        // The same code → character map the text decoder uses: the font's
        // `/Encoding`, or its built-in one. Symbol and ZapfDingbats ignore it.
        let widths = match self.get_encoding_map(doc, font_obj_id) {
            Some(declared) => standard.widths_by_code(&declared),
            None => standard.widths_by_code(standard.builtin_encoding()),
        };
        Some(FontMetrics::Simple {
            first_char: 0,
            widths,
            missing_width: 0.0,
        })
    }

    /// The standard 14 font a simple font dictionary names, if any. Composite fonts
    /// and Type 3 fonts are never standard fonts, whatever their `/BaseFont` says.
    fn standard_font(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<StandardFont> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let subtype = raw_dict_get(font_dict, b"Subtype").and_then(|s| s.as_name())?;
        if !matches!(subtype, b"Type1" | b"MMType1" | b"TrueType") {
            return None;
        }
        raw_dict_get(font_dict, b"BaseFont")
            .and_then(|n| doc.resolve(n).as_name())
            .and_then(StandardFont::from_base_font)
    }

    fn decode_text(
        &self,
        doc: &RawDocument,
        scope: ResourceScope,
        font_name: &[u8],
        bytes: &[u8],
    ) -> DecodedText {
        let font_obj_id = self.find_font_dict(doc, scope, font_name);
        let mut is_identity_h = false;
        let mut is_composite = false;

        // 1. Try ToUnicode CMap first. For a simple font, a code the CMap leaves
        //    unmapped still has the glyph its `/Encoding` names (ISO 32000-1 §9.10.2
        //    orders the sources per character code, not per string).
        if let Some(fid) = font_obj_id {
            is_identity_h = self.is_identity_cid_font(doc, fid);
            is_composite = self.is_composite_font(doc, fid);
            if let Some(cmap) = self.get_to_unicode_map(doc, fid) {
                let encoding = if !is_composite && cmap.code_width == 1 {
                    self.get_encoding_map(doc, fid)
                } else {
                    None
                };
                let decoded = match &encoding {
                    Some(enc) => cmap.decode_with_fallback(bytes, enc),
                    None => cmap.decode(bytes),
                };
                if !decoded.is_empty() {
                    return DecodedText::text(decoded);
                }
            }
        }

        // 2. Try embedded TrueType cmap table (for Identity-H CID fonts without ToUnicode)
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_embedded_cmap(doc, fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return DecodedText::text(decoded);
                }
            }
        }

        // 3. Try CIDSystemInfo-based CMap resource lookup (for Identity-H CID fonts)
        if is_identity_h {
            if let Some(fid) = font_obj_id {
                if let Some((registry, ordering)) = self.get_cid_system_info_cached(doc, fid) {
                    if let Some(decoded) = crate::parser::cmap_table::decode_with_cid_system_info(
                        &registry, &ordering, bytes,
                    ) {
                        if !decoded.is_empty() {
                            return DecodedText::text(decoded);
                        }
                    }
                }
            }
        }

        // 4. Try a predefined CJK CMap (`/Encoding /KSC-EUC-H` and friends). A Unicode
        //    CMap (`UniKS-UCS2-H`) carries the text itself and reads without the
        //    CIDFont's `/CIDSystemInfo`; a legacy one needs it to resolve its CIDs.
        if is_composite && !is_identity_h {
            if let Some(fid) = font_obj_id {
                if let Some(name) = self.get_encoding_name(doc, fid) {
                    let (registry, ordering) = self
                        .get_cid_system_info_cached(doc, fid)
                        .unwrap_or_default();
                    if let Some(decoded) =
                        crate::parser::predefined_cmap::decode(&name, &registry, &ordering, bytes)
                    {
                        if !decoded.is_empty() {
                            return DecodedText::text(decoded);
                        }
                    }
                }
            }
        }

        // 5. Try encoding dictionary (BaseEncoding + Differences)
        if let Some(fid) = font_obj_id {
            if let Some(enc_map) = self.get_encoding_map(doc, fid) {
                let decoded = decode_with_encoding_map(bytes, &enc_map);
                if !decoded.is_empty() {
                    return DecodedText::text(decoded);
                }
            }
        }

        // For composite (Type0/CID) fonts the content-stream bytes are CID codes, not
        // single-byte character codes. Byte-wise Latin-1 interpretation is categorically
        // wrong for them and only produces mojibake (e.g. `/Encoding /KSC-EUC-H` fonts
        // without ToUnicode), so emit nothing rather than unreadable text.
        if is_identity_h || is_composite {
            return DecodedText::suppressed(TextSuppression::CompositeUnresolved);
        }

        // 5b. A font that embeds its program and gives no `/Encoding` uses the program's
        //     built-in encoding (ISO 32000-1 §9.6.6.1) — TeX's fonts, for one, put
        //     ligatures, quotes and dashes at codes no Latin encoding has there. Before
        //     the binary judgement below: those codes are control characters in Latin-1.
        if let Some(fid) = font_obj_id {
            if let Some(enc_map) = self.program_encoding(doc, fid) {
                let decoded = decode_with_encoding_map(bytes, &enc_map);
                if !decoded.is_empty() {
                    return DecodedText::text(decoded);
                }
            }
        }

        // 6. A standard 14 font without `/Encoding` uses its built-in encoding
        //    (ISO 32000-1 §9.6.6). The control-character judgement below still runs
        //    first: those codes have no glyph in any built-in encoding and would be
        //    dropped, turning the very evidence of a mis-decoded run into clean text.
        let simple = decode_text_simple(bytes);
        if is_likely_binary(&simple) {
            return DecodedText::suppressed(TextSuppression::BinaryDensity);
        }
        if let Some(standard) = font_obj_id.and_then(|fid| self.standard_font(doc, fid)) {
            let decoded = decode_with_encoding_map(bytes, standard.builtin_encoding());
            if !decoded.is_empty() {
                return DecodedText::text(decoded);
            }
        }

        // 7. Final fallback
        DecodedText::text(simple)
    }

    /// The font dictionary `font_name` refers to in `scope`.
    fn find_font_dict(
        &self,
        doc: &RawDocument,
        scope: ResourceScope,
        font_name: &[u8],
    ) -> Option<ObjectId> {
        resource_chain(doc, scope)
            .into_iter()
            .find_map(|res| named_resource(doc, res, b"Font", font_name))
    }

    /// Get or parse the ToUnicode CMap for a font.
    fn get_to_unicode_map(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<ToUnicodeMap> {
        {
            let cache = self.cmap_cache.read().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_font_to_unicode(doc, font_obj_id);
        self.cmap_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    fn parse_font_to_unicode(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<ToUnicodeMap> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let to_unicode = raw_dict_get(font_dict, b"ToUnicode")?;
        let to_unicode = doc.resolve(to_unicode);

        let stream = match to_unicode {
            RawPdfObject::Stream(s) => s,
            RawPdfObject::Reference(n, g) => {
                let obj = doc.get_object((*n, *g))?;
                let resolved = doc.resolve(obj);
                resolved.as_stream()?
            }
            _ => return None,
        };

        let data = raw_stream::decompress(stream).unwrap_or_else(|_| stream.raw_data.clone());
        parse_to_unicode_cmap(&data)
    }

    /// Check if a font uses Identity-H or Identity-V CID encoding.
    fn is_identity_cid_font(&self, doc: &RawDocument, font_obj_id: PageId) -> bool {
        let font_dict = match doc.get_dict(font_obj_id) {
            Ok(d) => d,
            Err(_) => return false,
        };

        raw_dict_get(font_dict, b"Encoding")
            .and_then(|e| e.as_name())
            .map(|n| n == b"Identity-H" || n == b"Identity-V")
            .unwrap_or(false)
    }

    /// Get the font's `/Encoding` when it is a name (a predefined CMap), not a
    /// dictionary or an embedded CMap stream.
    fn get_encoding_name(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<String> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let name = raw_dict_get(font_dict, b"Encoding")?.as_name()?;
        Some(String::from_utf8_lossy(name).into_owned())
    }

    /// Check if a font is a composite (Type0/CID) font.
    ///
    /// Composite fonts address glyphs through CIDs, so their content-stream bytes must
    /// be decoded via a CMap (ToUnicode, embedded cmap, or a predefined CMap). Any
    /// single-byte fallback decoding is meaningless for them.
    fn is_composite_font(&self, doc: &RawDocument, font_obj_id: PageId) -> bool {
        let font_dict = match doc.get_dict(font_obj_id) {
            Ok(d) => d,
            Err(_) => return false,
        };

        let is_type0 = raw_dict_get(font_dict, b"Subtype")
            .and_then(|s| s.as_name())
            .map(|n| n == b"Type0")
            .unwrap_or(false);

        is_type0 || raw_dict_get(font_dict, b"DescendantFonts").is_some()
    }

    /// The descendant CIDFont dictionary of a Type 0 font.
    ///
    /// `/DescendantFonts` is a one-element array (ISO 32000-1 §9.7.6, Table 121) whose
    /// element is the CIDFont dictionary — by reference or written inline, both are
    /// valid and both occur. The array itself may be an indirect object too.
    fn cid_font_dict<'a>(
        &self,
        doc: &'a RawDocument,
        font_obj_id: PageId,
    ) -> Option<&'a RawPdfDict> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let descendants = doc
            .resolve(raw_dict_get(font_dict, b"DescendantFonts")?)
            .as_array()?;
        raw_resolve_dict(doc, descendants.first()?)
    }

    /// Extract CIDSystemInfo (Registry, Ordering) from a CIDFont.
    fn get_cid_system_info(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<(String, String)> {
        let cid_font_dict = self.cid_font_dict(doc, font_obj_id)?;

        let csi = raw_dict_get(cid_font_dict, b"CIDSystemInfo")?;
        let csi = doc.resolve(csi);
        let csi_dict = match csi {
            RawPdfObject::Dict(d) => d,
            RawPdfObject::Reference(n, g) => doc.get_dict((*n, *g)).ok()?,
            _ => return None,
        };

        let registry = raw_dict_get(csi_dict, b"Registry")
            .and_then(|o| o.as_str_bytes())
            .map(|s| String::from_utf8_lossy(s).to_string())?;

        let ordering = raw_dict_get(csi_dict, b"Ordering")
            .and_then(|o| o.as_str_bytes())
            .map(|s| String::from_utf8_lossy(s).to_string())?;

        Some((registry, ordering))
    }

    fn get_cid_system_info_cached(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<(String, String)> {
        {
            let cache = self.cid_system_info_cache.read().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }
        let result = self.get_cid_system_info(doc, font_obj_id);
        self.cid_system_info_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    /// Get or parse embedded TrueType cmap for Identity-H fonts.
    fn get_embedded_cmap(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<ToUnicodeMap> {
        {
            let cache = self.embedded_cmap_cache.read().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_embedded_truetype_cmap(doc, font_obj_id);
        self.embedded_cmap_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    fn parse_embedded_truetype_cmap(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<ToUnicodeMap> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;

        // Check Identity-H/V encoding
        let encoding = raw_dict_get(font_dict, b"Encoding")
            .and_then(|e| e.as_name())
            .map(|n| String::from_utf8_lossy(n).to_string())?;

        if encoding != "Identity-H" && encoding != "Identity-V" {
            return None;
        }

        let cid_font_dict = self.cid_font_dict(doc, font_obj_id)?;
        let fd_dict = raw_resolve_dict(doc, raw_dict_get(cid_font_dict, b"FontDescriptor")?)?;

        // Get FontFile2 (TrueType)
        let ff2 = raw_dict_get(fd_dict, b"FontFile2")?;
        let ff2 = doc.resolve(ff2);
        let font_stream = match ff2 {
            RawPdfObject::Stream(s) => s,
            RawPdfObject::Reference(n, g) => {
                let obj = doc.get_object((*n, *g))?;
                let resolved = doc.resolve(obj);
                resolved.as_stream()?
            }
            _ => return None,
        };

        let font_data =
            raw_stream::decompress(font_stream).unwrap_or_else(|_| font_stream.raw_data.clone());
        parse_truetype_cmap_table(&font_data)
    }

    /// The embedded program of a font, and how its codes select glyphs.
    fn font_program(
        &self,
        doc: &RawDocument,
        scope: ResourceScope,
        font_name: &[u8],
    ) -> Option<FontProgram> {
        let fid = self.find_font_dict(doc, scope, font_name)?;
        let font_dict = doc.get_dict(fid).ok()?;
        if self.is_composite_font(doc, fid) {
            let identity = raw_dict_get(font_dict, b"Encoding")
                .and_then(|e| e.as_name())
                .is_some_and(|n| n == b"Identity-H" || n == b"Identity-V");
            if !identity {
                return None;
            }
            let cid_font = self.cid_font_dict(doc, fid)?;
            let (format, data) = embedded_font_program(doc, cid_font)?;
            let cid_to_gid = raw_dict_get(cid_font, b"CIDToGIDMap")
                .map(|m| doc.resolve(m))
                .and_then(|m| m.as_stream())
                .map(|stream| {
                    let bytes = raw_stream::decompress(stream).unwrap_or_default();
                    bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&pair| u16::from_be_bytes(pair))
                        .collect()
                });
            return Some(FontProgram {
                format,
                data,
                selection: GlyphSelection::Cid { cid_to_gid },
                stand_in: false,
            });
        }

        let flags = raw_dict_get(font_dict, b"FontDescriptor")
            .and_then(|d| raw_resolve_dict(doc, d))
            .and_then(|d| raw_dict_get(d, b"Flags"))
            .map(|f| doc.resolve(f))
            .and_then(|f| f.as_i64());
        let (format, data, stand_in) = match embedded_font_program(doc, font_dict) {
            Some((format, data)) => (format, data, false),
            None => (FontFormat::Cff, self.stand_in(doc, fid, flags)?, true),
        };
        let symbolic = flags.is_some_and(|flags| flags & 4 != 0);
        let names = raw_dict_get(font_dict, b"Encoding")
            .and_then(|e| raw_resolve_dict(doc, e))
            .map(|enc| self.parse_differences(doc, enc).into_iter().collect())
            .unwrap_or_default();
        Some(FontProgram {
            format,
            data,
            selection: GlyphSelection::Simple {
                chars: self.get_encoding_map(doc, fid),
                names,
                symbolic,
            },
            stand_in,
        })
    }

    /// A standard face to paint a simple font that embeds no program with: a Type 1, a
    /// TrueType or a Multiple Master font a reader is expected to supply (§9.6.2.2), chosen
    /// by its name, its descriptor's `/Flags` and the style it declares. A Type 3 font draws
    /// its own glyphs and has none.
    #[cfg(feature = "standard-fonts")]
    fn stand_in(&self, doc: &RawDocument, fid: PageId, flags: Option<i64>) -> Option<Vec<u8>> {
        let dict = doc.get_dict(fid).ok()?;
        let subtype = raw_dict_get(dict, b"Subtype").and_then(|s| s.as_name())?;
        if !matches!(subtype, b"Type1" | b"TrueType" | b"MMType1") {
            return None;
        }
        let base_font = raw_dict_get(dict, b"BaseFont")
            .and_then(|n| n.as_name())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let style = self.declared_style(doc, fid);
        super::standard_fonts::stand_in(&base_font, flags, style.bold, style.italic)
            .map(<[u8]>::to_vec)
    }

    #[cfg(not(feature = "standard-fonts"))]
    fn stand_in(&self, _doc: &RawDocument, _fid: PageId, _flags: Option<i64>) -> Option<Vec<u8>> {
        None
    }

    /// Get or parse the encoding map for a font.
    fn get_encoding_map(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<HashMap<u8, char>> {
        {
            let cache = self.encoding_cache.read().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_encoding_dict(doc, font_obj_id);
        self.encoding_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    /// The built-in encoding of the font's embedded program, as characters: what a code is
    /// when the font dictionary gives no `/Encoding`, and the base `/Differences` apply to
    /// when it names no `/BaseEncoding` (ISO 32000-1 §9.6.6.1 — for an embedded program the
    /// implicit base is the program's own encoding, not StandardEncoding). Read for a Type 1
    /// program (`/FontFile`) and a bare CFF one (`/FontFile3 /Type1C`); `None` for any
    /// other font.
    fn program_encoding(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<HashMap<u8, char>> {
        if let Some(cached) = self
            .program_encoding_cache
            .read()
            .unwrap()
            .get(&font_obj_id)
        {
            return cached.clone();
        }
        let result = doc
            .get_dict(font_obj_id)
            .ok()
            .filter(|dict| {
                !self.is_composite_font(doc, font_obj_id)
                    && raw_dict_get(dict, b"FontDescriptor").is_some()
            })
            .and_then(|dict| embedded_font_program(doc, dict))
            .and_then(|(format, data)| match format {
                FontFormat::Type1 => super::type1::builtin_encoding_chars(&data),
                FontFormat::Cff => super::cff::builtin_encoding_chars(&data),
                _ => None,
            });
        self.program_encoding_cache
            .write()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    /// Parse the /Encoding entry from a font dictionary.
    ///
    /// The /Encoding can be:
    /// - A Name (e.g., /WinAnsiEncoding) → use that base encoding directly
    /// - A Dict with /BaseEncoding and /Differences → build a custom encoding map
    fn parse_encoding_dict(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<HashMap<u8, char>> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let encoding_obj = raw_dict_get(font_dict, b"Encoding")?;
        let encoding_obj = doc.resolve(encoding_obj);

        match encoding_obj {
            // Simple name: /WinAnsiEncoding, /MacRomanEncoding, /StandardEncoding
            RawPdfObject::Name(name) => {
                let base = BaseEncoding::from_name(name)?;
                Some(build_encoding_map(Some(base), &[]))
            }
            // Encoding dictionary with optional BaseEncoding and Differences
            RawPdfObject::Dict(dict) => {
                let differences = self.parse_differences(doc, dict);
                Some(self.encoding_over_base(doc, font_obj_id, dict, &differences))
            }
            RawPdfObject::Reference(n, g) => {
                let obj = doc.get_object((*n, *g))?;
                let resolved = doc.resolve(obj);
                match resolved {
                    RawPdfObject::Name(name) => {
                        let base = BaseEncoding::from_name(name)?;
                        Some(build_encoding_map(Some(base), &[]))
                    }
                    RawPdfObject::Dict(dict) => {
                        let differences = self.parse_differences(doc, dict);
                        Some(self.encoding_over_base(doc, font_obj_id, dict, &differences))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// An encoding dictionary's map: its `/Differences` over its `/BaseEncoding` — or, when
    /// it names none, over the font's own encoding (§9.6.6.1): its embedded program's, else a
    /// standard 14 font's built-in one (Symbol's and ZapfDingbats' are their own), else
    /// StandardEncoding.
    fn encoding_over_base(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
        dict: &RawPdfDict,
        differences: &[(u8, String)],
    ) -> HashMap<u8, char> {
        let named = raw_dict_get(dict, b"BaseEncoding")
            .and_then(|b| b.as_name())
            .and_then(BaseEncoding::from_name);
        if named.is_none() {
            let own = self.program_encoding(doc, font_obj_id).or_else(|| {
                self.standard_font(doc, font_obj_id)
                    .map(|standard| standard.builtin_encoding().clone())
            });
            if let Some(mut map) = own {
                for (code, name) in differences {
                    if let Some(ch) = glyph_name_to_unicode(name) {
                        map.insert(*code, ch);
                    }
                }
                return map;
            }
        }
        build_encoding_map(named, differences)
    }

    /// Parse a /Differences array from an encoding dictionary.
    ///
    /// Format: `[code1 /name1 /name2 ... codeN /nameN ...]`
    /// Each integer sets the starting code, and subsequent names map consecutive codes.
    fn parse_differences(&self, doc: &RawDocument, dict: &RawPdfDict) -> Vec<(u8, String)> {
        let mut result = Vec::new();
        let diff_obj = match raw_dict_get(dict, b"Differences") {
            Some(d) => d,
            None => return result,
        };
        let diff_obj = doc.resolve(diff_obj);
        let arr = match diff_obj.as_array() {
            Some(a) => a,
            None => return result,
        };

        let mut current_code: u32 = 0;
        for item in arr {
            let item = doc.resolve(item);
            match item {
                RawPdfObject::Integer(n) => {
                    current_code = *n as u32;
                }
                RawPdfObject::Name(name) => {
                    if current_code <= 255 {
                        let glyph_name = String::from_utf8_lossy(name).to_string();
                        result.push((current_code as u8, glyph_name));
                    }
                    current_code += 1;
                }
                _ => {}
            }
        }

        result
    }

    /// The weight and slant the font declares (§9.8.1), whatever its name says.
    ///
    /// Bold: the descriptor's `/FontWeight` is 600 or more or its `/Flags` sets ForceBold,
    /// or the embedded program names a bold weight — a Type 1 program's `FontInfo /Weight`,
    /// a CFF one's `Weight`, a TrueType or OpenType one's OS/2 weight class. Italic: the
    /// descriptor's `/ItalicAngle` leans (a degree or more — the entry is required, and
    /// upright fonts write 0) or its `/Flags` sets Italic. A composite font answers through
    /// its descendant. Names alone miss the styles a family calls something else (URW's
    /// `-Medi` is its bold, `-ReguItal` its italic; TeX's `CMTI10` names neither).
    fn declared_style(&self, doc: &RawDocument, font_obj_id: PageId) -> DeclaredStyle {
        if let Some(&cached) = self.style_cache.read().unwrap().get(&font_obj_id) {
            return cached;
        }
        let style = (|| -> Option<DeclaredStyle> {
            let mut dict = doc.get_dict(font_obj_id).ok()?;
            if self.is_composite_font(doc, font_obj_id) {
                dict = self.cid_font_dict(doc, font_obj_id)?;
            }
            let descriptor = raw_resolve_dict(doc, raw_dict_get(dict, b"FontDescriptor")?)?;
            let number = |key: &[u8]| {
                raw_dict_get(descriptor, key)
                    .map(|v| doc.resolve(v))
                    .and_then(|v| v.as_f32())
            };
            const ITALIC: i64 = 1 << 6;
            const FORCE_BOLD: i64 = 1 << 18;
            let flags = number(b"Flags").map_or(0, |f| f as i64);
            let italic = flags & ITALIC != 0
                || number(b"ItalicAngle").is_some_and(|angle| angle.abs() >= 1.0);
            let bold = number(b"FontWeight").is_some_and(|w| w >= 600.0)
                || flags & FORCE_BOLD != 0
                || embedded_font_program(doc, dict).is_some_and(|(format, data)| match format {
                    FontFormat::Type1 => super::type1::declared_weight(&data)
                        .is_some_and(|w| weight_name_is_bold(&w)),
                    FontFormat::Cff => {
                        super::cff::declared_weight(&data).is_some_and(|w| weight_name_is_bold(&w))
                    }
                    FontFormat::TrueType | FontFormat::OpenType => {
                        os2_weight_class(&data).is_some_and(|w| w >= 600)
                    }
                });
            Some(DeclaredStyle { bold, italic })
        })()
        .unwrap_or_default();
        self.style_cache.write().unwrap().insert(font_obj_id, style);
        style
    }

    /// The fonts the names in `scope` can refer to -- an inner name hides an outer one.
    fn page_fonts(&self, doc: &RawDocument, scope: ResourceScope) -> Vec<BackendFontInfo> {
        let mut result: Vec<BackendFontInfo> = Vec::new();
        for res in resource_chain(doc, scope) {
            let Some(font_dict) = raw_dict_get(res, b"Font").and_then(|f| raw_resolve_dict(doc, f))
            else {
                continue;
            };
            for (name, val) in font_dict {
                if result.iter().any(|f| &f.name == name) {
                    continue;
                }
                let Some(fd) = val.as_reference().and_then(|id| doc.get_dict(id).ok()) else {
                    continue;
                };
                let base_font = raw_dict_get(fd, b"BaseFont")
                    .and_then(|o| o.as_name())
                    .map(|n| String::from_utf8_lossy(n).to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                let style = val
                    .as_reference()
                    .map(|id| self.declared_style(doc, id))
                    .unwrap_or_default();
                result.push(BackendFontInfo {
                    name: name.clone(),
                    base_font,
                    bold: style.bold,
                    italic: style.italic,
                });
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// RawBackend helper functions
// ---------------------------------------------------------------------------

/// Resolve a PdfObject to a dictionary reference (following references).
fn raw_resolve_dict<'a>(doc: &'a RawDocument, obj: &'a RawPdfObject) -> Option<&'a RawPdfDict> {
    let resolved = doc.resolve(obj);
    match resolved {
        RawPdfObject::Dict(d) => Some(d),
        RawPdfObject::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

/// Decode a form field string (name or value) and enforce the text invariant.
///
/// Field strings used to be read one byte at a time with no UTF-16 handling at all,
/// which is worse than it sounds: AcroForm producers write these as UTF-16BE *with* the
/// byte-order mark, and `FE FF` is not valid UTF-8, so lossy decoding turned the mark
/// itself into two U+FFFD. Measured on a real form, 55 of 59 field strings carried the
/// mark, and their names came back as `\u{FFFD}\u{FFFD}topmostSubform[0]…` — one pair
/// per path segment. The interleaved NULs were removed by the sanitiser, which recovered
/// the ASCII by accident and hid the rest.
///
/// So the decode has to happen before the sanitiser sees the string, and it has to
/// handle both the BOM-carrying and BOM-less forms — see [`decode_text_string`].
fn sanitize_field_string(bytes: &[u8]) -> String {
    sanitize_extracted_text(decode_text_string_lossy(bytes))
}

/// Extract a string value from a raw PDF dictionary.
///
/// The result is sanitised ([`sanitize_extracted_text`]) because these strings are
/// reported as text — document metadata and outline titles. A NUL here is worse than
/// on a page: [`unpdf_get_title`](crate::ffi) can only answer "no title" for a string
/// it cannot transport, so the value would vanish without even an error.
fn raw_get_string(doc: &RawDocument, dict: &RawPdfDict, key: &[u8]) -> Option<String> {
    let obj = raw_dict_get(dict, key)?;
    let obj = doc.resolve(obj);
    let decoded = match obj {
        // A string that announces its encoding with a byte-order mark and then fails to
        // decode stays `None` rather than becoming mojibake: the caller reports "no
        // title", which is honest about the loss. See `text_string` for the rest.
        RawPdfObject::Str(bytes) => decode_text_string(bytes),
        RawPdfObject::Name(bytes) => String::from_utf8(bytes.clone()).ok(),
        _ => None,
    };
    decoded.map(sanitize_extracted_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_number_from_value() {
        assert_eq!(get_number_from_value(&PdfValue::Integer(42)), Some(42.0));
        assert_eq!(
            get_number_from_value(&PdfValue::Real(std::f32::consts::PI)),
            Some(std::f32::consts::PI)
        );
        assert_eq!(get_number_from_value(&PdfValue::Other), None);
    }

    /// Field strings are decoded first and sanitised second: the NULs a damaged
    /// single-byte string carries are removed rather than turning it into UTF-16.
    #[test]
    fn field_strings_drop_stray_nuls_without_being_read_as_utf16() {
        assert_eq!(sanitize_field_string(b"CHAP\0TER"), "CHAPTER");
        assert_eq!(sanitize_field_string(b"HELLO\0WORLD\0"), "HELLOWORLD");
    }
}

#[cfg(test)]
mod raw_backend_tests {
    use super::*;
    // Assembled in the test rather than read from disk -- see that module's docs.
    use crate::parser::test_pdf::{one_page_pdf, ONE_PAGE_CONTENT};

    fn backend() -> RawBackend {
        RawBackend::load_bytes(&one_page_pdf()).expect("a well-formed PDF loads")
    }

    #[test]
    fn pages_are_enumerated_from_one() {
        let pages = backend().pages();
        assert_eq!(pages.len(), 1);
        assert!(pages.contains_key(&1));
    }

    #[test]
    fn a_page_content_stream_is_returned_as_written() {
        let raw = backend();
        let content = raw.page_content(raw.pages()[&1]).unwrap();
        assert_eq!(content.trim_ascii_end(), ONE_PAGE_CONTENT);
    }

    #[test]
    fn a_content_stream_decodes_into_its_operators_in_order() {
        let raw = backend();
        let content = raw.page_content(raw.pages()[&1]).unwrap();
        let operators: Vec<String> = raw
            .decode_content(&content)
            .unwrap()
            .into_iter()
            .map(|op| op.operator)
            .collect();
        assert_eq!(operators, ["BT", "Tf", "Td", "Tj", "ET"]);
    }

    /// A page whose only content stream claims `/FlateDecode` over bytes no decoder accepts.
    fn undecodable_backend() -> RawBackend {
        use crate::parser::test_pdf::{pdf, stream};
        let bytes = pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Contents 4 0 R>>".to_vec(),
                // No zlib stream begins 0xFF 0xFF.
                stream("<</Length 16/Filter/FlateDecode>>", &[0xFF; 16]),
            ],
            1,
        );
        RawBackend::load_bytes(&bytes).expect("the document structure is intact")
    }

    #[test]
    fn an_undecodable_only_stream_is_counted_rather_than_an_error() {
        let raw = undecodable_backend();
        let content = raw.page_content_with_losses(raw.pages()[&1]).unwrap();
        assert_eq!(content.data, Vec::<u8>::new());
        assert_eq!(content.undecodable_streams, 1);
    }

    /// The plain accessor has no loss count to carry, so it must not turn "nothing could be
    /// decoded" into an empty page.
    #[test]
    fn page_content_fails_when_no_stream_could_be_decoded() {
        let raw = undecodable_backend();
        assert!(raw.page_content(raw.pages()[&1]).is_err());
    }

    /// `/Contents` is optional -- its absence is an empty page, and nothing was lost.
    #[test]
    fn a_page_without_contents_is_empty_rather_than_an_error() {
        use crate::parser::test_pdf::pdf;
        let raw = RawBackend::load_bytes(&pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>".to_vec(),
            ],
            1,
        ))
        .expect("the document structure is intact");
        let page = raw.pages()[&1];

        assert_eq!(
            raw.page_content_with_losses(page).unwrap(),
            PageContent::complete(Vec::new())
        );
        assert_eq!(raw.page_content(page).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn metadata_reports_the_header_version() {
        assert_eq!(backend().metadata().version, "1.4");
    }

    #[test]
    fn page_dimensions_come_from_the_media_box() {
        let raw = backend();
        assert_eq!(raw.page_dimensions(raw.pages()[&1]), (595.0, 842.0));
    }

    /// A box is its four corners, in any order, off the origin or not; the size is the
    /// difference of the corners, not the upper-right corner.
    #[test]
    fn the_page_box_keeps_its_origin() {
        use crate::parser::test_pdf::pdf;
        let raw = RawBackend::load_bytes(&pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[695 942 100 100]>>".to_vec(),
            ],
            1,
        ))
        .unwrap();
        let page = raw.pages()[&1];
        assert_eq!(
            raw.page_box(page),
            PageBox {
                llx: 100.0,
                lly: 100.0,
                urx: 695.0,
                ury: 942.0
            }
        );
        assert_eq!(raw.page_dimensions(page), (595.0, 842.0));
    }

    /// A `/Parent` cycle ends the search for an inherited attribute; it does not recurse
    /// until the stack runs out.
    #[test]
    fn a_parent_cycle_ends_the_inherited_lookup() {
        use crate::parser::test_pdf::pdf;
        let raw = RawBackend::load_bytes(&pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1/Parent 4 0 R>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[2 0 R]/Parent 2 0 R>>".to_vec(),
            ],
            1,
        ))
        .unwrap();
        let page = raw.pages()[&1];
        assert_eq!(raw.page_box(page), PageBox::LETTER);
        assert_eq!(raw.page_rotation(page), 0);
    }

    /// A page without `/Resources` inherits its parent's (ISO 32000-1 §7.7.3.4) -- for its
    /// images as much as for its fonts, which were already looked up that way.
    #[test]
    fn a_page_lists_the_images_of_resources_it_inherits() {
        use crate::parser::test_pdf::{pdf, stream};
        let bytes = pdf(
            vec![
                b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
                b"<</Type/Pages/Kids[3 0 R]/Count 1/Resources<</XObject<</Im0 4 0 R>>>>>>".to_vec(),
                b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>".to_vec(),
                stream(
                    "<</Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray\
                      /BitsPerComponent 8/Length 1>>",
                    &[0x80],
                ),
            ],
            1,
        );
        let raw = RawBackend::load_bytes(&bytes).unwrap();
        let images = raw.page_xobjects(raw.pages()[&1]).unwrap();
        assert_eq!(
            images.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["Im0"]
        );
    }
}
