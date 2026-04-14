//! PDF backend abstraction layer.
//!
//! Provides a trait-based interface for PDF operations, isolating
//! the concrete PDF parser from the layout analysis logic.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::model::{FieldType, FieldValue, FormField};

use super::encoding::{build_encoding_map, decode_with_encoding_map, BaseEncoding};
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
pub trait PdfBackend: Send + Sync {
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

    /// Extract AcroForm fields from the document.
    fn acroform_fields(&self) -> Vec<FormField> {
        vec![]
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
                            _ => raw_stream::decompress(stream)
                                .unwrap_or_else(|_| stream.raw_data.clone()),
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

    fn acroform_fields(&self) -> Vec<FormField> {
        self.extract_acroform_fields()
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
            .map(|s| String::from_utf8_lossy(s).to_string());

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
                .map(|s| FieldValue::Text(String::from_utf8_lossy(s).to_string())),
            FieldType::Checkbox | FieldType::RadioButton => {
                obj.as_name().map(|n| FieldValue::Boolean(n != b"Off"))
            }
            FieldType::Dropdown | FieldType::ListBox => {
                if let Some(s) = obj.as_str_bytes() {
                    Some(FieldValue::Choice(String::from_utf8_lossy(s).to_string()))
                } else if let Some(arr) = obj.as_array() {
                    let choices: Vec<String> = arr
                        .iter()
                        .filter_map(|o| o.as_str_bytes())
                        .map(|s| String::from_utf8_lossy(s).to_string())
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
    cmap_cache: Mutex<HashMap<PageId, Option<ToUnicodeMap>>>,
    encoding_cache: Mutex<HashMap<PageId, Option<HashMap<u8, char>>>>,
    cid_system_info_cache: Mutex<HashMap<PageId, Option<(String, String)>>>,
}

impl RawFontResolver {
    fn new() -> Self {
        Self {
            cmap_cache: Mutex::new(HashMap::new()),
            encoding_cache: Mutex::new(HashMap::new()),
            cid_system_info_cache: Mutex::new(HashMap::new()),
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

        // 2. Try embedded TrueType cmap table (for Identity-H CID fonts without ToUnicode)
        if let Some(fid) = font_obj_id {
            if let Some(cmap) = self.get_embedded_cmap(doc, fid) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
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
                            return decoded;
                        }
                    }
                }
            }
        }

        // 4. Try encoding dictionary (BaseEncoding + Differences)
        if let Some(fid) = font_obj_id {
            if let Some(enc_map) = self.get_encoding_map(doc, fid) {
                let decoded = decode_with_encoding_map(bytes, &enc_map);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
        }

        // For Identity-H/V fonts, simple fallback always produces garbage
        if is_identity_h {
            return String::new();
        }

        // 5. Final fallback
        let simple = decode_text_simple(bytes);
        if is_likely_binary(&simple) {
            String::new()
        } else {
            simple
        }
    }

    /// Find the font dictionary object ID for a given font name on a page.
    fn find_font_dict(&self, doc: &RawDocument, page: PageId, font_name: &[u8]) -> Option<PageId> {
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
            let cache = self.cmap_cache.lock().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_font_to_unicode(doc, font_obj_id);
        self.cmap_cache
            .lock()
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

    /// Get the CIDFont's object ID from a Type0 font.
    fn get_cid_font_id(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<PageId> {
        let font_dict = doc.get_dict(font_obj_id).ok()?;
        let descendants = raw_dict_get(font_dict, b"DescendantFonts")?;
        let descendants = doc.resolve(descendants);

        let arr = descendants.as_array()?;
        arr.first()?.as_reference()
    }

    /// Extract CIDSystemInfo (Registry, Ordering) from a CIDFont.
    fn get_cid_system_info(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<(String, String)> {
        let cid_font_id = self.get_cid_font_id(doc, font_obj_id)?;
        let cid_font_dict = doc.get_dict(cid_font_id).ok()?;

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
            let cache = self.cid_system_info_cache.lock().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }
        let result = self.get_cid_system_info(doc, font_obj_id);
        self.cid_system_info_cache
            .lock()
            .unwrap()
            .insert(font_obj_id, result.clone());
        result
    }

    /// Get or parse embedded TrueType cmap for Identity-H fonts.
    fn get_embedded_cmap(&self, doc: &RawDocument, font_obj_id: PageId) -> Option<ToUnicodeMap> {
        let cid_font_id = self.get_cid_font_id(doc, font_obj_id)?;

        {
            let cache = self.cmap_cache.lock().unwrap();
            if let Some(cached) = cache.get(&cid_font_id) {
                return cached.clone();
            }
        }

        let result = self.parse_embedded_truetype_cmap(doc, font_obj_id);
        self.cmap_cache
            .lock()
            .unwrap()
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
        let fd_ref = raw_dict_get(cid_font_dict, b"FontDescriptor")?.as_reference()?;
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

    /// Get or parse the encoding map for a font.
    fn get_encoding_map(
        &self,
        doc: &RawDocument,
        font_obj_id: PageId,
    ) -> Option<HashMap<u8, char>> {
        {
            let cache = self.encoding_cache.lock().unwrap();
            if let Some(cached) = cache.get(&font_obj_id) {
                return cached.clone();
            }
        }

        let result = self.parse_encoding_dict(doc, font_obj_id);
        self.encoding_cache
            .lock()
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
                let base = raw_dict_get(dict, b"BaseEncoding")
                    .and_then(|b| b.as_name())
                    .and_then(BaseEncoding::from_name);

                let differences = self.parse_differences(doc, dict);
                Some(build_encoding_map(base, &differences))
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
                        let base = raw_dict_get(dict, b"BaseEncoding")
                            .and_then(|b| b.as_name())
                            .and_then(BaseEncoding::from_name);
                        let differences = self.parse_differences(doc, dict);
                        Some(build_encoding_map(base, &differences))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
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
    use std::path::Path;

    /// Skip the test if the PDF fixture (gitignored under `test-files/`)
    /// is unavailable, e.g., on CI. Returns the loaded backend or `None`.
    fn try_load(rel: &str) -> Option<RawBackend> {
        if !Path::new(rel).exists() {
            eprintln!("skipping: fixture not present at {}", rel);
            return None;
        }
        RawBackend::load_file(rel).ok()
    }

    #[test]
    fn test_raw_backend_pages() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        assert!(!pages.is_empty());
    }

    #[test]
    fn test_raw_backend_page_content() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let content = raw.page_content(first_page).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn test_raw_backend_decode_content() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let content = raw.page_content(first_page).unwrap();
        let ops = raw.decode_content(&content).unwrap();
        assert!(!ops.is_empty());
    }

    #[test]
    fn test_raw_backend_metadata() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let meta = raw.metadata();
        assert!(!meta.version.is_empty());
    }

    #[test]
    fn test_raw_backend_page_dimensions() {
        let Some(raw) = try_load("test-files/basic/trivial.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let (w, h) = raw.page_dimensions(first_page);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn test_raw_backend_korean_pages() {
        let Some(raw) = try_load("test-files/cjk/korean-test.pdf") else {
            return;
        };
        assert!(!raw.pages().is_empty());
    }

    #[test]
    fn test_iphone_korean_text_decode() {
        let Some(raw) = try_load("test-files/realworld/iphone-info.pdf") else {
            return;
        };
        let pages = raw.pages();
        let first_page = *pages.values().next().unwrap();
        let decoded = raw.decode_text(first_page, b"T1_1", &[31, 30, 29, 28, 27]);
        assert!(
            decoded.contains('사') && decoded.contains('서'),
            "Korean text should be decoded: got {:?}",
            decoded
        );
    }
}
