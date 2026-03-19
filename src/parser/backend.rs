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
        let data = std::fs::read(path).map_err(Error::Io)?;
        Self::load_bytes(&data)
    }

    /// Load from an in-memory byte slice.
    pub fn load_bytes(data: &[u8]) -> Result<Self> {
        let doc = RawDocument::load(data)?;
        Ok(Self {
            doc,
            font_resolver: RawFontResolver::new(),
        })
    }

    /// Load from a reader.
    pub fn load_reader<R: std::io::Read>(mut reader: R) -> Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Self::load_bytes(&data)
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

    fn page_fonts(&self, page: PageId) -> Result<Vec<BackendFontInfo>> {
        self.font_resolver.page_fonts(&self.doc, page)
    }

    fn page_content(&self, page_id: PageId) -> Result<Vec<u8>> {
        let page_dict = self
            .doc
            .get_dict(page_id)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let contents = raw_dict_get(page_dict, b"Contents")
            .ok_or_else(|| Error::PdfParse("No Contents in page".to_string()))?;

        let contents = self.doc.resolve(contents);

        match contents {
            RawPdfObject::Reference(n, g) => {
                let obj = self
                    .doc
                    .get_object((*n, *g))
                    .ok_or_else(|| Error::PdfParse("Content stream not found".to_string()))?;
                let resolved = self.doc.resolve(obj);
                if let Some(stream) = resolved.as_stream() {
                    return raw_stream::decompress(stream);
                }
                Err(Error::PdfParse("Invalid content stream".to_string()))
            }
            RawPdfObject::Stream(stream) => raw_stream::decompress(stream),
            RawPdfObject::Array(arr) => {
                let mut content = Vec::new();
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
                    if let Ok(data) = raw_stream::decompress(stream_obj) {
                        content.extend_from_slice(&data);
                        content.push(b' ');
                    }
                }
                Ok(content)
            }
            _ => Err(Error::PdfParse("Invalid content stream".to_string())),
        }
    }

    fn decode_content(&self, data: &[u8]) -> Result<Vec<ContentOp>> {
        raw_content::parse_content_stream(data)
    }

    fn decode_text(&self, page: PageId, font_name: &[u8], bytes: &[u8]) -> String {
        self.font_resolver
            .decode_text(&self.doc, page, font_name, bytes)
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

    fn page_dimensions(&self, page: PageId) -> (f32, f32) {
        if let Some(dims) = self.find_media_box(page) {
            return dims;
        }
        (612.0, 792.0)
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
        let mut xobjects = Vec::new();

        let page_dict = self
            .doc
            .get_dict(page)
            .map_err(|e| Error::PdfParse(e.to_string()))?;

        let resources = match raw_dict_get(page_dict, b"Resources") {
            Some(r) => r,
            None => return Ok(xobjects),
        };

        let res_dict = raw_resolve_dict(&self.doc, resources);
        let res_dict = match res_dict {
            Some(d) => d,
            None => return Ok(xobjects),
        };

        let xobj_entry = match raw_dict_get(res_dict, b"XObject") {
            Some(x) => x,
            None => return Ok(xobjects),
        };

        let xobj_dict = raw_resolve_dict(&self.doc, xobj_entry);
        let xobj_dict = match xobj_dict {
            Some(d) => d,
            None => return Ok(xobjects),
        };

        for (name, obj) in xobj_dict {
            if let Some((n, g)) = obj.as_reference() {
                if let Some(raw_obj) = self.doc.get_object((n, g)) {
                    let resolved = self.doc.resolve(raw_obj);
                    if let Some(stream) = resolved.as_stream() {
                        let dict = &stream.dict;

                        let subtype = raw_dict_get(dict, b"Subtype")
                            .and_then(|s| s.as_name())
                            .map(|n| String::from_utf8_lossy(n).to_string())
                            .unwrap_or_default();

                        if subtype != "Image" {
                            continue;
                        }

                        let filter = raw_dict_get(dict, b"Filter")
                            .and_then(|f| f.as_name())
                            .map(|n| String::from_utf8_lossy(n).to_string());

                        let data = match filter.as_deref() {
                            Some("DCTDecode") | Some("JPXDecode") => stream.raw_data.clone(),
                            _ => raw_stream::decompress(stream).unwrap_or_else(|_| stream.raw_data.clone()),
                        };

                        let width = raw_dict_get(dict, b"Width")
                            .and_then(|w| w.as_i64())
                            .map(|w| w as u32);
                        let height = raw_dict_get(dict, b"Height")
                            .and_then(|h| h.as_i64())
                            .map(|h| h as u32);
                        let bits = raw_dict_get(dict, b"BitsPerComponent")
                            .and_then(|b| b.as_i64())
                            .map(|b| b as u8);

                        let color_space =
                            raw_dict_get(dict, b"ColorSpace").and_then(|cs| match cs {
                                RawPdfObject::Name(n) => {
                                    Some(String::from_utf8_lossy(n).to_string())
                                }
                                RawPdfObject::Array(arr) => arr
                                    .first()
                                    .and_then(|o| o.as_name())
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
        }

        Ok(xobjects)
    }
}

impl RawBackend {
    /// Find MediaBox for a page, walking up the page tree for inherited values.
    fn find_media_box(&self, page_id: PageId) -> Option<(f32, f32)> {
        let dict = self.doc.get_dict(page_id).ok()?;

        if let Some(media_box) = raw_dict_get(dict, b"MediaBox") {
            if let Some(dims) = extract_dimensions_from_array(media_box) {
                return Some(dims);
            }
        }

        // Walk up to parent
        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.find_media_box_in_ancestor(parent_id);
            }
        }

        None
    }

    fn find_media_box_in_ancestor(&self, id: PageId) -> Option<(f32, f32)> {
        let dict = self.doc.get_dict(id).ok()?;

        if let Some(media_box) = raw_dict_get(dict, b"MediaBox") {
            if let Some(dims) = extract_dimensions_from_array(media_box) {
                return Some(dims);
            }
        }

        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.find_media_box_in_ancestor(parent_id);
            }
        }

        None
    }

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

struct RawFontResolver {
    cmap_cache: RefCell<HashMap<PageId, Option<ToUnicodeMap>>>,
}

impl RawFontResolver {
    fn new() -> Self {
        Self {
            cmap_cache: RefCell::new(HashMap::new()),
        }
    }

    fn decode_text(
        &self,
        doc: &RawDocument,
        page: PageId,
        font_name: &[u8],
        bytes: &[u8],
    ) -> String {
        let font_obj_id = self.find_font_dict(doc, page, font_name);
        let mut is_identity_h = false;

        // 1. Try ToUnicode CMap first
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_to_unicode_map(doc, fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
            is_identity_h = self.is_identity_cid_font(doc, fid);
        }

        // 2. (No lopdf encoding fallback — skip to step 3)

        // 3. Try embedded TrueType cmap table (for Identity-H CID fonts without ToUnicode)
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_embedded_cmap(doc, fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
        }

        // For Identity-H/V fonts, simple fallback always produces garbage
        if is_identity_h {
            return String::new();
        }

        // 4. Final fallback
        let simple = decode_text_simple(bytes);
        if is_likely_binary(&simple) {
            String::new()
        } else {
            simple
        }
    }

    /// Find the font dictionary object ID for a given font name on a page.
    fn find_font_dict(
        &self,
        doc: &RawDocument,
        page: PageId,
        font_name: &[u8],
    ) -> Option<PageId> {
        // Try the page's own Resources
        if let Some(fid) = self.find_font_in_resources(doc, page, font_name) {
            return Some(fid);
        }

        // Walk up the Pages tree for inherited Resources
        if let Ok(page_dict) = doc.get_dict(page) {
            if let Some(parent) = raw_dict_get(page_dict, b"Parent") {
                if let Some(parent_id) = parent.as_reference() {
                    return self.find_font_in_ancestor(doc, parent_id, font_name);
                }
            }
        }

        None
    }

    fn find_font_in_resources(
        &self,
        doc: &RawDocument,
        obj_id: PageId,
        font_name: &[u8],
    ) -> Option<PageId> {
        let dict = doc.get_dict(obj_id).ok()?;
        let resources = raw_dict_get(dict, b"Resources")?;
        self.find_font_in_resource_obj(doc, resources, font_name)
    }

    fn find_font_in_resource_obj(
        &self,
        doc: &RawDocument,
        resources: &RawPdfObject,
        font_name: &[u8],
    ) -> Option<PageId> {
        let res_dict = raw_resolve_dict(doc, resources)?;
        let font_obj = raw_dict_get(res_dict, b"Font")?;
        let font_dict = raw_resolve_dict(doc, font_obj)?;
        let font_entry = raw_dict_get(font_dict, font_name)?;
        font_entry.as_reference()
    }

    fn find_font_in_ancestor(
        &self,
        doc: &RawDocument,
        ancestor_id: PageId,
        font_name: &[u8],
    ) -> Option<PageId> {
        let dict = doc.get_dict(ancestor_id).ok()?;

        if let Some(resources) = raw_dict_get(dict, b"Resources") {
            if let Some(fid) = self.find_font_in_resource_obj(doc, resources, font_name) {
                return Some(fid);
            }
        }

        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.find_font_in_ancestor(doc, parent_id, font_name);
            }
        }

        None
    }

    /// Get or parse the ToUnicode CMap for a font.
    fn get_to_unicode_map(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<ToUnicodeMap> {
        {
            let cache = self.cmap_cache.borrow();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_font_to_unicode(doc, font_obj_id);
        self.cmap_cache
            .borrow_mut()
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

    /// Get the CIDFont's object ID from a Type0 font.
    fn get_cid_font_id(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<PageId> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let descendants = raw_dict_get(font_dict, b"DescendantFonts")?;
        let descendants = doc.resolve(descendants);

        let arr = descendants.as_array()?;
        arr.first()?.as_reference()
    }

    /// Get or parse embedded TrueType cmap for Identity-H fonts.
    fn get_embedded_cmap(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<ToUnicodeMap> {
        let cid_font_id = self.get_cid_font_id(doc, font_obj_id)?;

        {
            let cache = self.cmap_cache.borrow();
            if let Some(cached) = cache.get(&cid_font_id) {
                return cached.clone();
            }
        }

        let result = self.parse_embedded_truetype_cmap(doc, font_obj_id);
        self.cmap_cache
            .borrow_mut()
            .insert(cid_font_id, result.clone());
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

        // Get CIDFont from DescendantFonts
        let cid_font_id = self.get_cid_font_id(doc, font_obj_id)?;
        let cid_font_dict = doc.get_dict(cid_font_id).ok()?;

        // Get FontDescriptor
        let fd_ref = raw_dict_get(cid_font_dict, b"FontDescriptor")?
            .as_reference()?;
        let fd_dict = doc.get_dict(fd_ref).ok()?;

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

    /// Collect font info for a page (with inherited resources fallback).
    fn page_fonts(&self, doc: &RawDocument, page: PageId) -> Result<Vec<BackendFontInfo>> {
        if let Some(fonts) = self.collect_fonts_from_page(doc, page) {
            if !fonts.is_empty() {
                return Ok(fonts);
            }
        }

        // Try inherited resources
        if let Ok(page_dict) = doc.get_dict(page) {
            if let Some(parent) = raw_dict_get(page_dict, b"Parent") {
                if let Some(parent_id) = parent.as_reference() {
                    if let Some(fonts) = self.collect_fonts_from_ancestor(doc, parent_id) {
                        return Ok(fonts);
                    }
                }
            }
        }

        Ok(Vec::new())
    }

    fn collect_fonts_from_page(
        &self,
        doc: &RawDocument,
        page: PageId,
    ) -> Option<Vec<BackendFontInfo>> {
        let dict = doc.get_dict(page).ok()?;
        let resources = raw_dict_get(dict, b"Resources")?;
        self.collect_fonts_from_resource_obj(doc, resources)
    }

    fn collect_fonts_from_ancestor(
        &self,
        doc: &RawDocument,
        ancestor_id: PageId,
    ) -> Option<Vec<BackendFontInfo>> {
        let dict = doc.get_dict(ancestor_id).ok()?;

        if let Some(resources) = raw_dict_get(dict, b"Resources") {
            if let Some(fonts) = self.collect_fonts_from_resource_obj(doc, resources) {
                if !fonts.is_empty() {
                    return Some(fonts);
                }
            }
        }

        if let Some(parent) = raw_dict_get(dict, b"Parent") {
            if let Some(parent_id) = parent.as_reference() {
                return self.collect_fonts_from_ancestor(doc, parent_id);
            }
        }

        None
    }

    fn collect_fonts_from_resource_obj(
        &self,
        doc: &RawDocument,
        resources: &RawPdfObject,
    ) -> Option<Vec<BackendFontInfo>> {
        let res_dict = raw_resolve_dict(doc, resources)?;
        let font_obj = raw_dict_get(res_dict, b"Font")?;
        let font_dict = raw_resolve_dict(doc, font_obj)?;

        let mut result = Vec::new();
        for (name, val) in font_dict {
            let font_id = match val.as_reference() {
                Some(r) => r,
                None => continue,
            };
            let fd = match doc.get_dict(font_id) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let base_font = raw_dict_get(fd, b"BaseFont")
                .and_then(|o| o.as_name())
                .map(|n| String::from_utf8_lossy(n).to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            result.push(BackendFontInfo {
                name: name.clone(),
                base_font,
            });
        }

        Some(result)
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

/// Extract a string value from a raw PDF dictionary.
fn raw_get_string(doc: &RawDocument, dict: &RawPdfDict, key: &[u8]) -> Option<String> {
    let obj = raw_dict_get(dict, key)?;
    let obj = doc.resolve(obj);
    match obj {
        RawPdfObject::Str(bytes) => {
            if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
                // UTF-16BE with BOM
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
        RawPdfObject::Name(bytes) => String::from_utf8(bytes.clone()).ok(),
        _ => None,
    }
}

/// Extract (width, height) from a MediaBox array.
fn extract_dimensions_from_array(obj: &RawPdfObject) -> Option<(f32, f32)> {
    let arr = obj.as_array()?;
    if arr.len() >= 4 {
        let width = arr[2].as_f32().unwrap_or(612.0);
        let height = arr[3].as_f32().unwrap_or(792.0);
        Some((width, height))
    } else {
        None
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

#[cfg(test)]
mod raw_backend_tests {
    use super::*;

    #[test]
    fn test_raw_backend_pages() {
        let raw = RawBackend::load_file("test-files/basic/trivial.pdf").unwrap();
        let pages = raw.pages();
        assert!(!pages.is_empty());
    }

    #[test]
    fn test_raw_backend_page_content() {
        let raw = RawBackend::load_file("test-files/basic/trivial.pdf").unwrap();
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let content = raw.page_content(first_page).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn test_raw_backend_decode_content() {
        let raw = RawBackend::load_file("test-files/basic/trivial.pdf").unwrap();
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let content = raw.page_content(first_page).unwrap();
        let ops = raw.decode_content(&content).unwrap();
        assert!(!ops.is_empty());
    }

    #[test]
    fn test_raw_backend_metadata() {
        let raw = RawBackend::load_file("test-files/basic/trivial.pdf").unwrap();
        let meta = raw.metadata();
        assert!(!meta.version.is_empty());
    }

    #[test]
    fn test_raw_backend_page_dimensions() {
        let raw = RawBackend::load_file("test-files/basic/trivial.pdf").unwrap();
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let (w, h) = raw.page_dimensions(first_page);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn test_raw_backend_korean_pages() {
        let raw = RawBackend::load_file("test-files/cjk/korean-test.pdf").unwrap();
        assert!(!raw.pages().is_empty());
    }
}
