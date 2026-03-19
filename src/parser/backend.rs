//! PDF backend abstraction layer.
//!
//! Provides a trait-based interface for PDF operations, isolating
//! the concrete PDF library (lopdf) from the layout analysis logic.

use std::collections::{BTreeMap, HashMap};

use crate::error::{Error, Result};

use super::font::{
    is_likely_binary, parse_to_unicode_cmap, parse_truetype_cmap_table, ToUnicodeMap,
};

/// Page identifier: (object number, generation number).
pub type PageId = (u32, u16);

/// Font information returned by the backend.
#[derive(Debug, Clone)]
pub struct BackendFontInfo {
    /// Font resource name (key in the page's font dictionary).
    pub name: Vec<u8>,
    /// Base font name (e.g., "Helvetica-Bold").
    pub base_font: String,
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

/// A raw XObject (image) extracted from a PDF page.
#[derive(Debug, Clone)]
pub struct RawXObject {
    pub name: String,
    pub subtype: String,
    pub data: Vec<u8>,
    pub filter: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits_per_component: Option<u8>,
    pub color_space: Option<String>,
}

/// Abstract interface for PDF document access.
///
/// Implementations provide page enumeration, font info, content stream
/// decoding, and text decoding — without exposing any concrete PDF library types.
pub trait PdfBackend {
    /// Return all pages as (page_number → PageId).
    fn pages(&self) -> BTreeMap<u32, PageId>;

    /// Return font info for a given page.
    fn page_fonts(&self, page: PageId) -> Result<Vec<BackendFontInfo>>;

    /// Return the raw (decompressed) content stream bytes for a page.
    fn page_content(&self, page: PageId) -> Result<Vec<u8>>;

    /// Parse raw content stream bytes into a sequence of operations.
    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>>;

    /// Decode a text byte sequence using the font's encoding on the given page.
    /// Falls back to simple decoding if the font or encoding is unavailable.
    fn decode_text(&self, page: PageId, font_name: &[u8], bytes: &[u8]) -> String;

    /// Return raw metadata (version, info dict fields, encryption status).
    fn metadata(&self) -> PdfMetadataRaw;

    /// Return page dimensions (width, height) in points.
    /// Falls back to Letter size (612, 792) if MediaBox is absent.
    fn page_dimensions(&self, page: PageId) -> (f32, f32);

    /// Return the document outline (bookmarks) as a tree.
    /// Implementations must handle cycle detection and depth limits.
    fn outline(&self) -> Result<Vec<RawOutlineItem>>;

    /// Return XObjects (images) from a page.
    fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>>;
}

// Re-export decode_text_simple as pub for external consumers.
pub use super::font::decode_text_simple;

// ---------------------------------------------------------------------------
// LopdfBackend — concrete implementation backed by lopdf
// ---------------------------------------------------------------------------

use lopdf::{Document as LopdfDocument, Object, ObjectId};
use std::cell::RefCell;

/// Concrete [`PdfBackend`] backed by `lopdf::Document`.
pub struct LopdfBackend {
    doc: LopdfDocument,
    /// Cache of parsed ToUnicode CMaps per font object ID.
    cmap_cache: RefCell<HashMap<ObjectId, Option<ToUnicodeMap>>>,
}

impl LopdfBackend {
    /// Load from a file path.
    pub fn load_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let doc = LopdfDocument::load(path).map_err(|e| match e {
            lopdf::Error::Decryption(_) => Error::Encrypted,
            _ => Error::from(e),
        })?;
        Ok(Self {
            doc,
            cmap_cache: RefCell::new(HashMap::new()),
        })
    }

    /// Load from an in-memory byte slice.
    pub fn load_bytes(data: &[u8]) -> Result<Self> {
        let doc = LopdfDocument::load_mem(data).map_err(|e| match e {
            lopdf::Error::Decryption(_) => Error::Encrypted,
            _ => Error::from(e),
        })?;
        Ok(Self {
            doc,
            cmap_cache: RefCell::new(HashMap::new()),
        })
    }

    /// Load from a reader.
    pub fn load_reader<R: std::io::Read>(mut reader: R) -> Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Self::load_bytes(&data)
    }

    /// Direct access to the underlying `lopdf::Document`.
    ///
    /// Escape hatch for operations not yet covered by `PdfBackend`
    /// (metadata, outlines, resource extraction, etc.).
    pub fn raw_doc(&self) -> &LopdfDocument {
        &self.doc
    }

    /// Check if the document is encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.doc.is_encrypted()
    }

    /// Get PDF version string.
    pub fn version(&self) -> String {
        self.doc.version.to_string()
    }

    /// Find the font dictionary for a given font name on a page,
    /// walking up the Pages tree for inherited Resources.
    fn find_font_dict(&self, page_id: PageId, font_name: &[u8]) -> Option<ObjectId> {
        // First try the page's own Resources
        if let Some(font_obj_id) = self.find_font_in_resources(page_id, font_name) {
            return Some(font_obj_id);
        }

        // Walk up the Pages tree for inherited Resources
        if let Ok(page_dict) = self.doc.get_dictionary(page_id) {
            if let Ok(Object::Reference(parent_ref)) = page_dict.get(b"Parent") {
                return self.find_font_in_ancestor(*parent_ref, font_name);
            }
        }

        None
    }

    /// Search for a font in the Resources dictionary of a given object (page or pages node).
    fn find_font_in_resources(&self, obj_id: ObjectId, font_name: &[u8]) -> Option<ObjectId> {
        let dict = self.doc.get_dictionary(obj_id).ok()?;
        let resources = dict.get(b"Resources").ok()?;
        self.find_font_in_resource_obj(resources, font_name)
    }

    /// Given a Resources object (possibly a reference), find the named font's object ID.
    fn find_font_in_resource_obj(&self, resources: &Object, font_name: &[u8]) -> Option<ObjectId> {
        let res_dict = match resources {
            Object::Reference(r) => {
                if let Ok(Object::Dictionary(d)) = self.doc.get_object(*r) {
                    d
                } else {
                    return None;
                }
            }
            Object::Dictionary(d) => d,
            _ => return None,
        };

        let font_obj = res_dict.get(b"Font").ok()?;
        let font_dict = match font_obj {
            Object::Reference(r) => {
                if let Ok(Object::Dictionary(d)) = self.doc.get_object(*r) {
                    d
                } else {
                    return None;
                }
            }
            Object::Dictionary(d) => d,
            _ => return None,
        };

        match font_dict.get(font_name).ok()? {
            Object::Reference(r) => Some(*r),
            _ => None,
        }
    }

    /// Walk up the ancestor chain to find inherited font resources.
    fn find_font_in_ancestor(&self, ancestor_id: ObjectId, font_name: &[u8]) -> Option<ObjectId> {
        let dict = match self.doc.get_object(ancestor_id).ok()? {
            Object::Dictionary(d) => d,
            _ => return None,
        };

        // Check this ancestor's Resources
        if let Ok(resources) = dict.get(b"Resources") {
            if let Some(font_id) = self.find_font_in_resource_obj(resources, font_name) {
                return Some(font_id);
            }
        }

        // Keep walking up
        if let Ok(Object::Reference(parent_ref)) = dict.get(b"Parent") {
            return self.find_font_in_ancestor(*parent_ref, font_name);
        }

        None
    }

    /// Get or parse the ToUnicode CMap for a font object.
    fn get_to_unicode_map(&self, font_obj_id: ObjectId) -> Option<ToUnicodeMap> {
        // Check cache
        {
            let cache = self.cmap_cache.borrow();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        // Parse and cache
        let result = self.parse_font_to_unicode(font_obj_id);
        self.cmap_cache
            .borrow_mut()
            .insert(font_obj_id, result.clone());
        result
    }

    /// Parse the ToUnicode CMap from a font dictionary.
    fn parse_font_to_unicode(&self, font_obj_id: ObjectId) -> Option<ToUnicodeMap> {
        let font_dict = match self.doc.get_object(font_obj_id).ok()? {
            Object::Dictionary(d) => d,
            _ => return None,
        };

        let to_unicode = font_dict.get(b"ToUnicode").ok()?;
        let stream = match to_unicode {
            Object::Reference(r) => match self.doc.get_object(*r).ok()? {
                Object::Stream(s) => s,
                _ => return None,
            },
            Object::Stream(s) => s,
            _ => return None,
        };

        // Try decompressed first (for compressed streams), fall back to raw content
        let data = safe_decompress(stream);

        parse_to_unicode_cmap(&data)
    }

    /// Get or parse the embedded TrueType cmap table for a font.
    fn get_embedded_cmap(&self, font_obj_id: ObjectId) -> Option<ToUnicodeMap> {
        // We reuse cmap_cache — the key is the CIDFont's ObjectId (not the Type0 font's).
        // To avoid collisions, we look up the descendant font ID and use that as key.
        let cid_font_id = self.get_cid_font_id(font_obj_id)?;

        // Check cache
        {
            let cache = self.cmap_cache.borrow();
            if let Some(cached) = cache.get(&cid_font_id) {
                return cached.clone();
            }
        }

        let result = self.parse_embedded_truetype_cmap(font_obj_id);
        self.cmap_cache
            .borrow_mut()
            .insert(cid_font_id, result.clone());
        result
    }

    /// Get the CIDFont's object ID from a Type0 font.
    fn get_cid_font_id(&self, font_obj_id: ObjectId) -> Option<ObjectId> {
        let font_dict = match self.doc.get_object(font_obj_id).ok()? {
            Object::Dictionary(d) => d,
            _ => return None,
        };

        let descendants = font_dict.get(b"DescendantFonts").ok()?;
        let arr = match descendants {
            Object::Array(a) => a,
            Object::Reference(r) => match self.doc.get_object(*r).ok()? {
                Object::Array(a) => a,
                _ => return None,
            },
            _ => return None,
        };

        arr.first()?.as_reference().ok()
    }

    /// Parse the TrueType cmap table from an embedded font to build a GID→Unicode map.
    ///
    /// Path: Type0 Font → DescendantFonts[0] (CIDFont) → FontDescriptor → FontFile2 → cmap table.
    /// For Identity-H encoding, content stream bytes are GIDs, so reversing the cmap gives us
    /// a GID→Unicode mapping we can use like a ToUnicode CMap.
    fn parse_embedded_truetype_cmap(&self, font_obj_id: ObjectId) -> Option<ToUnicodeMap> {
        let font_dict = match self.doc.get_object(font_obj_id).ok()? {
            Object::Dictionary(d) => d,
            _ => return None,
        };

        // Check this is a Type0 font with Identity-H encoding
        let encoding = font_dict
            .get(b"Encoding")
            .ok()
            .and_then(|e| e.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).to_string())?;

        if encoding != "Identity-H" && encoding != "Identity-V" {
            return None;
        }

        // Get CIDFont from DescendantFonts
        let cid_font_id = self.get_cid_font_id(font_obj_id)?;
        let cid_font_dict = match self.doc.get_object(cid_font_id).ok()? {
            Object::Dictionary(d) => d,
            _ => return None,
        };

        // Get FontDescriptor
        let fd_ref = cid_font_dict
            .get(b"FontDescriptor")
            .ok()?
            .as_reference()
            .ok()?;
        let fd_dict = match self.doc.get_object(fd_ref).ok()? {
            Object::Dictionary(d) => d,
            _ => return None,
        };

        // Get FontFile2 (TrueType) or FontFile3 (CFF/OpenType)
        let font_stream = if let Ok(ff2) = fd_dict.get(b"FontFile2") {
            let ff2_ref = ff2.as_reference().ok()?;
            match self.doc.get_object(ff2_ref).ok()? {
                Object::Stream(s) => s,
                _ => return None,
            }
        } else {
            return None; // Only TrueType (FontFile2) supported for now
        };

        let font_data = safe_decompress(font_stream);
        parse_truetype_cmap_table(&font_data)
    }

    /// Check if a font uses Identity-H or Identity-V CID encoding.
    fn is_identity_cid_font(&self, font_obj_id: ObjectId) -> bool {
        let font_dict = match self.doc.get_object(font_obj_id).ok() {
            Some(Object::Dictionary(d)) => d,
            _ => return false,
        };

        font_dict
            .get(b"Encoding")
            .ok()
            .and_then(|e| e.as_name().ok())
            .map(|n| n == b"Identity-H" || n == b"Identity-V")
            .unwrap_or(false)
    }
}

impl PdfBackend for LopdfBackend {
    fn pages(&self) -> BTreeMap<u32, PageId> {
        self.doc.get_pages()
    }

    fn page_fonts(&self, page: PageId) -> Result<Vec<BackendFontInfo>> {
        let lopdf_fonts = self
            .doc
            .get_page_fonts(page)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let mut result = Vec::with_capacity(lopdf_fonts.len());
        for (name, font_dict) in &lopdf_fonts {
            let base_font = font_dict
                .get(b"BaseFont")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            result.push(BackendFontInfo {
                name: name.clone(),
                base_font,
            });
        }

        // If lopdf found no fonts, try inherited resources
        if result.is_empty() {
            if let Ok(page_dict) = self.doc.get_dictionary(page) {
                if let Ok(Object::Reference(parent_ref)) = page_dict.get(b"Parent") {
                    if let Some(fonts) = self.collect_fonts_from_ancestor(*parent_ref) {
                        return Ok(fonts);
                    }
                }
            }
        }

        Ok(result)
    }

    fn page_content(&self, page_id: PageId) -> Result<Vec<u8>> {
        let page_dict = self
            .doc
            .get_dictionary(page_id)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let contents = page_dict
            .get(b"Contents")
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        match contents {
            Object::Reference(r) => {
                if let Ok(Object::Stream(s)) = self.doc.get_object(*r) {
                    return Ok(safe_decompress(s));
                }
                Err(Error::PdfParse("Invalid content stream".to_string()))
            }
            Object::Array(arr) => {
                let mut content = Vec::new();
                for obj in arr {
                    if let Object::Reference(r) = obj {
                        if let Ok(Object::Stream(s)) = self.doc.get_object(*r) {
                            let data = safe_decompress(s);
                            content.extend_from_slice(&data);
                            content.push(b' ');
                        }
                    }
                }
                Ok(content)
            }
            _ => Err(Error::PdfParse("Invalid content stream".to_string())),
        }
    }

    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>> {
        let content =
            lopdf::content::Content::decode(data).map_err(|e| Error::PdfParse(e.to_string()))?;

        Ok(content
            .operations
            .into_iter()
            .map(|op| ContentOp {
                operator: op.operator,
                operands: op.operands.iter().map(convert_object).collect(),
            })
            .collect())
    }

    fn decode_text(&self, page: PageId, font_name: &[u8], bytes: &[u8]) -> String {
        let font_obj_id = self.find_font_dict(page, font_name);
        let mut is_identity_h = false;

        // 1. Try ToUnicode CMap first (most reliable for CID/composite fonts)
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_to_unicode_map(fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
            is_identity_h = self.is_identity_cid_font(fid);
        }

        // 2. Try lopdf's built-in font encoding
        if let Ok(lopdf_fonts) = self.doc.get_page_fonts(page) {
            if let Some(font_dict) = lopdf_fonts.get(font_name) {
                // Also detect Identity-H from lopdf's font dict
                // (covers cases where find_font_dict fails)
                if !is_identity_h {
                    is_identity_h = font_dict
                        .get(b"Encoding")
                        .ok()
                        .and_then(|e| e.as_name().ok())
                        .map(|n| n == b"Identity-H" || n == b"Identity-V")
                        .unwrap_or(false);
                }
                // For Identity-H CID fonts, lopdf's encoding will decode
                // glyph IDs as if they were character codes, producing garbage.
                // Skip this step for such fonts.
                if !is_identity_h {
                    if let Ok(enc) = font_dict.get_font_encoding(&self.doc) {
                        if let Ok(text) = LopdfDocument::decode_text(&enc, bytes) {
                            return text;
                        }
                    }
                }
            }
        }

        // 3. Try embedded TrueType cmap table (for Identity-H CID fonts without ToUnicode)
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_embedded_cmap(fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
        }

        // For Identity-H/V fonts, simple fallback always produces garbage because
        // the bytes are 2-byte glyph IDs, not character codes.
        if is_identity_h {
            return String::new();
        }

        // 4. Final fallback — decode as simple text but filter out likely binary garbage
        let simple = decode_text_simple(bytes);
        if is_likely_binary(&simple) {
            String::new()
        } else {
            simple
        }
    }

    fn metadata(&self) -> PdfMetadataRaw {
        let mut meta = PdfMetadataRaw {
            version: self.doc.version.to_string(),
            encrypted: self.doc.is_encrypted(),
            ..Default::default()
        };

        if let Ok(info) = self.doc.trailer.get(b"Info") {
            if let Ok(info_ref) = info.as_reference() {
                if let Ok(info_dict) = self.doc.get_dictionary(info_ref) {
                    meta.title = Self::get_string_from_lopdf_dict(info_dict, b"Title");
                    meta.author = Self::get_string_from_lopdf_dict(info_dict, b"Author");
                    meta.subject = Self::get_string_from_lopdf_dict(info_dict, b"Subject");
                    meta.keywords = Self::get_string_from_lopdf_dict(info_dict, b"Keywords");
                    meta.creator = Self::get_string_from_lopdf_dict(info_dict, b"Creator");
                    meta.producer = Self::get_string_from_lopdf_dict(info_dict, b"Producer");
                    meta.creation_date =
                        Self::get_string_from_lopdf_dict(info_dict, b"CreationDate");
                    meta.mod_date = Self::get_string_from_lopdf_dict(info_dict, b"ModDate");
                }
            }
        }

        meta
    }

    fn page_dimensions(&self, page: PageId) -> (f32, f32) {
        if let Ok(page_dict) = self.doc.get_dictionary(page) {
            if let Ok(media_box) = page_dict.get(b"MediaBox") {
                if let Ok(array) = media_box.as_array() {
                    if array.len() >= 4 {
                        let width = array[2].as_float().unwrap_or(612.0);
                        let height = array[3].as_float().unwrap_or(792.0);
                        return (width, height);
                    }
                }
            }
        }
        (612.0, 792.0)
    }

    fn outline(&self) -> Result<Vec<RawOutlineItem>> {
        const MAX_DEPTH: u8 = 64;
        let mut items = Vec::new();
        let mut visited = std::collections::HashSet::new();

        if let Ok(catalog) = self.doc.catalog() {
            if let Ok(outlines) = catalog.get(b"Outlines") {
                if let Ok(outlines_ref) = outlines.as_reference() {
                    if let Ok(outlines_dict) = self.doc.get_dictionary(outlines_ref) {
                        if let Ok(first) = outlines_dict.get(b"First") {
                            if let Ok(first_ref) = first.as_reference() {
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
            }
        }

        Ok(items)
    }

    fn page_xobjects(&self, page: PageId) -> Result<Vec<RawXObject>> {
        let mut xobjects = Vec::new();

        let page_dict = self
            .doc
            .get_dictionary(page)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let res = match page_dict.get(b"Resources") {
            Ok(r) => r,
            Err(_) => return Ok(xobjects),
        };

        let res_dict = match res {
            Object::Reference(r) => match self.doc.get_dictionary(*r) {
                Ok(d) => d,
                Err(_) => return Ok(xobjects),
            },
            Object::Dictionary(d) => d,
            _ => return Ok(xobjects),
        };

        let xobj_entry = match res_dict.get(b"XObject") {
            Ok(x) => x,
            Err(_) => return Ok(xobjects),
        };

        let xobj_dict = match xobj_entry {
            Object::Reference(r) => match self.doc.get_dictionary(*r) {
                Ok(d) => d,
                Err(_) => return Ok(xobjects),
            },
            Object::Dictionary(d) => d,
            _ => return Ok(xobjects),
        };

        for (name, obj) in xobj_dict.iter() {
            if let Ok(obj_ref) = obj.as_reference() {
                if let Ok(Object::Stream(stream)) = self.doc.get_object(obj_ref) {
                    let dict = &stream.dict;

                    let subtype = dict
                        .get(b"Subtype")
                        .ok()
                        .and_then(|s| s.as_name().ok())
                        .map(|n| String::from_utf8_lossy(n).to_string())
                        .unwrap_or_default();

                    if subtype != "Image" {
                        continue;
                    }

                    let filter = dict
                        .get(b"Filter")
                        .ok()
                        .and_then(|f| f.as_name().ok())
                        .map(|n| String::from_utf8_lossy(n).to_string());

                    let data = match filter.as_deref() {
                        Some("DCTDecode") | Some("JPXDecode") => stream.content.clone(),
                        _ => safe_decompress(stream),
                    };

                    let width = dict
                        .get(b"Width")
                        .ok()
                        .and_then(|w| w.as_i64().ok())
                        .map(|w| w as u32);
                    let height = dict
                        .get(b"Height")
                        .ok()
                        .and_then(|h| h.as_i64().ok())
                        .map(|h| h as u32);
                    let bits = dict
                        .get(b"BitsPerComponent")
                        .ok()
                        .and_then(|b| b.as_i64().ok())
                        .map(|b| b as u8);

                    let color_space = dict.get(b"ColorSpace").ok().and_then(|cs| match cs {
                        Object::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
                        Object::Array(arr) => arr
                            .first()
                            .and_then(|o| o.as_name().ok())
                            .map(|n| String::from_utf8_lossy(n).to_string()),
                        _ => None,
                    });

                    xobjects.push(RawXObject {
                        name: String::from_utf8_lossy(name).to_string(),
                        subtype,
                        data,
                        filter,
                        width,
                        height,
                        bits_per_component: bits,
                        color_space,
                    });
                }
            }
        }

        Ok(xobjects)
    }
}

impl LopdfBackend {
    /// Collect font info from an ancestor's Resources dictionary.
    fn collect_fonts_from_ancestor(&self, ancestor_id: ObjectId) -> Option<Vec<BackendFontInfo>> {
        let dict = match self.doc.get_object(ancestor_id).ok()? {
            Object::Dictionary(d) => d,
            _ => return None,
        };

        if let Ok(resources) = dict.get(b"Resources") {
            if let Some(fonts) = self.collect_fonts_from_resource_obj(resources) {
                if !fonts.is_empty() {
                    return Some(fonts);
                }
            }
        }

        // Keep walking up
        if let Ok(Object::Reference(parent_ref)) = dict.get(b"Parent") {
            return self.collect_fonts_from_ancestor(*parent_ref);
        }

        None
    }

    /// Extract font info from a Resources object.
    fn collect_fonts_from_resource_obj(&self, resources: &Object) -> Option<Vec<BackendFontInfo>> {
        let res_dict = match resources {
            Object::Reference(r) => match self.doc.get_object(*r).ok()? {
                Object::Dictionary(d) => d,
                _ => return None,
            },
            Object::Dictionary(d) => d,
            _ => return None,
        };

        let font_obj = res_dict.get(b"Font").ok()?;
        let font_dict = match font_obj {
            Object::Reference(r) => match self.doc.get_object(*r).ok()? {
                Object::Dictionary(d) => d,
                _ => return None,
            },
            Object::Dictionary(d) => d,
            _ => return None,
        };

        let mut result = Vec::new();
        for (name, val) in font_dict.as_hashmap() {
            let font_ref = match val {
                Object::Reference(r) => *r,
                _ => continue,
            };
            let fd = match self.doc.get_object(font_ref).ok()? {
                Object::Dictionary(d) => d,
                _ => continue,
            };
            let base_font = fd
                .get(b"BaseFont")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            result.push(BackendFontInfo {
                name: name.clone(),
                base_font,
            });
        }

        Some(result)
    }

    fn get_string_from_lopdf_dict(dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
        dict.get(key).ok().and_then(|obj| match obj {
            Object::String(bytes, _) => {
                if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                    let utf16: Vec<u16> = bytes[2..]
                        .chunks(2)
                        .filter_map(|c| {
                            if c.len() == 2 {
                                Some(u16::from_be_bytes([c[0], c[1]]))
                            } else {
                                None
                            }
                        })
                        .collect();
                    String::from_utf16(&utf16).ok()
                } else {
                    String::from_utf8(bytes.clone())
                        .ok()
                        .or_else(|| Some(bytes.iter().map(|&b| b as char).collect()))
                }
            }
            Object::Name(bytes) => String::from_utf8(bytes.clone()).ok(),
            _ => None,
        })
    }

    fn collect_outline_items(
        &self,
        item_ref: ObjectId,
        level: u8,
        max_depth: u8,
        items: &mut Vec<RawOutlineItem>,
        visited: &mut std::collections::HashSet<ObjectId>,
    ) {
        if !visited.insert(item_ref) || level > max_depth {
            return;
        }

        if let Ok(item_dict) = self.doc.get_dictionary(item_ref) {
            let title = Self::get_string_from_lopdf_dict(item_dict, b"Title").unwrap_or_default();
            let page = self.resolve_outline_dest(item_dict);

            let mut outline_item = RawOutlineItem {
                title,
                page,
                level,
                children: Vec::new(),
            };

            if let Ok(first) = item_dict.get(b"First") {
                if let Ok(first_ref) = first.as_reference() {
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

            if let Ok(next) = item_dict.get(b"Next") {
                if let Ok(next_ref) = next.as_reference() {
                    self.collect_outline_items(next_ref, level, max_depth, items, visited);
                }
            }
        }
    }

    fn resolve_outline_dest(&self, item_dict: &lopdf::Dictionary) -> Option<u32> {
        let pages = self.doc.get_pages();

        // Try Dest
        if let Ok(dest) = item_dict.get(b"Dest") {
            if let Ok(dest_array) = dest.as_array() {
                if let Some(first) = dest_array.first() {
                    if let Ok(page_ref) = first.as_reference() {
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
        if let Ok(action) = item_dict.get(b"A") {
            if let Ok(action_ref) = action.as_reference() {
                if let Ok(action_dict) = self.doc.get_dictionary(action_ref) {
                    if let Ok(dest) = action_dict.get(b"D") {
                        if let Ok(dest_array) = dest.as_array() {
                            if let Some(first) = dest_array.first() {
                                if let Ok(page_ref) = first.as_reference() {
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
        }

        None
    }
}

/// Safely decompress a PDF stream, handling missing `Filter` key.
///
/// Per ISO 32000, the `Filter` key in a stream dictionary is **optional**.
/// Its absence means the stream data is uncompressed (identity encoding).
/// `lopdf` requires it, so we check first and use raw content when absent.
pub(crate) fn safe_decompress(stream: &lopdf::Stream) -> Vec<u8> {
    if stream.dict.get(b"Filter").is_ok() {
        stream
            .decompressed_content()
            .unwrap_or_else(|_| stream.content.clone())
    } else {
        // No Filter = uncompressed (identity encoding per PDF spec)
        stream.content.clone()
    }
}

/// Convert a `lopdf::Object` to [`PdfValue`].
fn convert_object(obj: &Object) -> PdfValue {
    match obj {
        Object::Integer(i) => PdfValue::Integer(*i),
        Object::Real(r) => PdfValue::Real(*r),
        Object::Name(n) => PdfValue::Name(n.clone()),
        Object::String(b, _) => PdfValue::Str(b.clone()),
        Object::Array(arr) => PdfValue::Array(arr.iter().map(convert_object).collect()),
        _ => PdfValue::Other,
    }
}

/// Helper: extract a number from a [`PdfValue`].
pub fn get_number_from_value(val: &PdfValue) -> Option<f32> {
    match val {
        PdfValue::Integer(i) => Some(*i as f32),
        PdfValue::Real(r) => Some(*r),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_number_from_value() {
        assert_eq!(get_number_from_value(&PdfValue::Integer(42)), Some(42.0));
        assert_eq!(get_number_from_value(&PdfValue::Real(3.14)), Some(3.14));
        assert_eq!(get_number_from_value(&PdfValue::Other), None);
    }
}
