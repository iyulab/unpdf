//! PDF backend abstraction layer.
//!
//! Provides a trait-based interface for PDF operations, isolating
//! the concrete PDF library (lopdf) from the layout analysis logic.

use std::collections::{BTreeMap, HashMap};

use crate::error::{Error, Result};

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
}

/// Simple text decoding fallback when no encoding is available.
pub fn decode_text_simple(bytes: &[u8]) -> String {
    // Try UTF-16BE first (BOM marker)
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
        return String::from_utf16(&utf16).unwrap_or_default();
    }

    // Try UTF-8
    if let Ok(s) = String::from_utf8(bytes.to_vec()) {
        return s;
    }

    // Fallback: Latin-1
    bytes.iter().map(|&b| b as char).collect()
}

// ---------------------------------------------------------------------------
// ToUnicode CMap parser
// ---------------------------------------------------------------------------

/// Parsed ToUnicode CMap: maps character codes to Unicode strings.
#[derive(Debug, Clone)]
struct ToUnicodeMap {
    /// Bytes per character code (1 or 2). Determined from codespace range.
    code_width: usize,
    /// Character code → Unicode string mapping.
    mappings: HashMap<u32, String>,
}

impl ToUnicodeMap {
    /// Decode a byte sequence using this CMap.
    fn decode(&self, bytes: &[u8]) -> String {
        let mut result = String::new();
        let mut i = 0;
        while i < bytes.len() {
            if self.code_width == 2 && i + 1 < bytes.len() {
                let code = u32::from(bytes[i]) << 8 | u32::from(bytes[i + 1]);
                if let Some(s) = self.mappings.get(&code) {
                    result.push_str(s);
                }
                // If unmapped, skip silently (common for space/control chars)
                i += 2;
            } else {
                let code = u32::from(bytes[i]);
                if let Some(s) = self.mappings.get(&code) {
                    result.push_str(s);
                }
                i += 1;
            }
        }
        result
    }
}

/// Parse a hex string like "0048" into a u32 value.
fn parse_hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

/// Decode a hex string into a Unicode string.
/// The hex represents UTF-16BE code units (e.g., "0048" → "H", "D800DC00" → surrogate pair).
fn hex_to_unicode(hex: &str) -> Option<String> {
    if hex.len() % 4 != 0 && hex.len() == 2 {
        // Single-byte mapping: treat as direct code point
        let cp = u32::from_str_radix(hex, 16).ok()?;
        return char::from_u32(cp).map(|c| c.to_string());
    }

    // Parse as UTF-16BE code units (each 4 hex digits = one u16)
    let mut units = Vec::new();
    let mut i = 0;
    while i + 3 < hex.len() {
        let val = u16::from_str_radix(&hex[i..i + 4], 16).ok()?;
        units.push(val);
        i += 4;
    }
    String::from_utf16(&units).ok()
}

/// Parse a ToUnicode CMap stream into a `ToUnicodeMap`.
fn parse_to_unicode_cmap(data: &[u8]) -> Option<ToUnicodeMap> {
    let text = String::from_utf8_lossy(data);
    let mut mappings = HashMap::new();
    let mut code_width: usize = 2; // default for Identity-H

    // Parse codespace range to determine code width
    if let Some(cs_start) = text.find("begincodespacerange") {
        if let Some(cs_end) = text[cs_start..].find("endcodespacerange") {
            let cs_block = &text[cs_start..cs_start + cs_end];
            // Look for hex strings like <0000> or <00>
            if let Some(first_angle) = cs_block.find('<') {
                if let Some(close_angle) = cs_block[first_angle..].find('>') {
                    let hex_len = close_angle - 1; // length of hex string
                    code_width = hex_len / 2; // 2 hex chars = 1 byte
                    if code_width == 0 {
                        code_width = 1;
                    }
                }
            }
        }
    }

    // Parse beginbfchar sections
    let mut search_pos = 0;
    while let Some(start) = text[search_pos..].find("beginbfchar") {
        let block_start = search_pos + start + "beginbfchar".len();
        if let Some(end) = text[block_start..].find("endbfchar") {
            let block = &text[block_start..block_start + end];
            // Parse lines like: <0003> <0020>
            for line in block.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line
                    .split(['<', '>'])
                    .filter(|s| !s.trim().is_empty())
                    .collect();
                if parts.len() >= 2 {
                    if let Some(code) = parse_hex(parts[0].trim()) {
                        if let Some(unicode_str) = hex_to_unicode(parts[1].trim()) {
                            mappings.insert(code, unicode_str);
                        }
                    }
                }
            }
            search_pos = block_start + end;
        } else {
            break;
        }
    }

    // Parse beginbfrange sections
    search_pos = 0;
    while let Some(start) = text[search_pos..].find("beginbfrange") {
        let block_start = search_pos + start + "beginbfrange".len();
        if let Some(end) = text[block_start..].find("endbfrange") {
            let block = &text[block_start..block_start + end];
            for line in block.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                // Check for array form: <start> <end> [<u1> <u2> ...]
                if line.contains('[') {
                    let parts: Vec<&str> = line
                        .split(['<', '>', '[', ']'])
                        .filter(|s| !s.trim().is_empty())
                        .collect();
                    if parts.len() >= 3 {
                        if let (Some(lo), Some(hi)) =
                            (parse_hex(parts[0].trim()), parse_hex(parts[1].trim()))
                        {
                            for (i, code) in (lo..=hi).enumerate() {
                                if let Some(unicode_str) =
                                    parts.get(2 + i).and_then(|h| hex_to_unicode(h.trim()))
                                {
                                    mappings.insert(code, unicode_str);
                                }
                            }
                        }
                    }
                } else {
                    // Simple form: <start> <end> <dst_start>
                    let parts: Vec<&str> = line
                        .split(['<', '>'])
                        .filter(|s| !s.trim().is_empty())
                        .collect();
                    if parts.len() >= 3 {
                        if let (Some(lo), Some(hi), Some(dst_start)) = (
                            parse_hex(parts[0].trim()),
                            parse_hex(parts[1].trim()),
                            parse_hex(parts[2].trim()),
                        ) {
                            for (i, code) in (lo..=hi).enumerate() {
                                let dst = dst_start + i as u32;
                                if let Some(c) = char::from_u32(dst) {
                                    mappings.insert(code, c.to_string());
                                }
                            }
                        }
                    }
                }
            }
            search_pos = block_start + end;
        } else {
            break;
        }
    }

    if mappings.is_empty() {
        return None;
    }

    Some(ToUnicodeMap {
        code_width,
        mappings,
    })
}

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
        let data = stream
            .decompressed_content()
            .unwrap_or_else(|_| stream.content.clone());

        parse_to_unicode_cmap(&data)
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
                    return s
                        .decompressed_content()
                        .map_err(|e| Error::PdfParse(e.to_string()));
                }
                Err(Error::PdfParse("Invalid content stream".to_string()))
            }
            Object::Array(arr) => {
                let mut content = Vec::new();
                for obj in arr {
                    if let Object::Reference(r) = obj {
                        if let Ok(Object::Stream(s)) = self.doc.get_object(*r) {
                            if let Ok(data) = s.decompressed_content() {
                                content.extend_from_slice(&data);
                                content.push(b' ');
                            }
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
        // 1. Try lopdf's built-in font encoding
        if let Ok(lopdf_fonts) = self.doc.get_page_fonts(page) {
            if let Some(font_dict) = lopdf_fonts.get(font_name) {
                if let Ok(enc) = font_dict.get_font_encoding(&self.doc) {
                    if let Ok(text) = LopdfDocument::decode_text(&enc, bytes) {
                        return text;
                    }
                }
            }
        }

        // 2. Try ToUnicode CMap (handles Identity-H fonts from Typst, etc.)
        if let Some(font_obj_id) = self.find_font_dict(page, font_name) {
            if let Some(cmap) = self.get_to_unicode_map(font_obj_id) {
                let decoded = cmap.decode(bytes);
                if !decoded.is_empty() {
                    return decoded;
                }
            }
        }

        // 3. Final fallback
        decode_text_simple(bytes)
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
    fn test_decode_text_simple_utf8() {
        assert_eq!(decode_text_simple(b"Hello"), "Hello");
    }

    #[test]
    fn test_decode_text_simple_latin1() {
        // 0xE9 = 'é' in Latin-1
        let bytes = vec![0x48, 0x65, 0x6C, 0x6C, 0xE9];
        let text = decode_text_simple(&bytes);
        assert_eq!(text, "Hellé");
    }

    #[test]
    fn test_decode_text_simple_utf16be() {
        // UTF-16BE BOM + "Hi"
        let bytes = vec![0xFE, 0xFF, 0x00, 0x48, 0x00, 0x69];
        assert_eq!(decode_text_simple(&bytes), "Hi");
    }

    #[test]
    fn test_get_number_from_value() {
        assert_eq!(get_number_from_value(&PdfValue::Integer(42)), Some(42.0));
        assert_eq!(get_number_from_value(&PdfValue::Real(3.14)), Some(3.14));
        assert_eq!(get_number_from_value(&PdfValue::Other), None);
    }

    #[test]
    fn test_parse_to_unicode_cmap_bfchar() {
        let cmap = b"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
1 begincodespacerange
<0000> <ffff>
endcodespacerange
3 beginbfchar
<0003> <0020>
<001C> <0039>
<0024> <0041>
endbfchar
endcmap";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        assert_eq!(map.code_width, 2);
        assert_eq!(map.mappings.get(&0x0003), Some(&" ".to_string()));
        assert_eq!(map.mappings.get(&0x001C), Some(&"9".to_string()));
        assert_eq!(map.mappings.get(&0x0024), Some(&"A".to_string()));
    }

    #[test]
    fn test_parse_to_unicode_cmap_bfrange() {
        let cmap = b"1 begincodespacerange
<00> <FF>
endcodespacerange
1 beginbfrange
<20> <7E> <0020>
endbfrange";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        assert_eq!(map.code_width, 1);
        assert_eq!(map.mappings.get(&0x20), Some(&" ".to_string()));
        assert_eq!(map.mappings.get(&0x41), Some(&"A".to_string()));
        assert_eq!(map.mappings.get(&0x7E), Some(&"~".to_string()));
    }

    #[test]
    fn test_to_unicode_map_decode() {
        let cmap = b"1 begincodespacerange
<0000> <ffff>
endcodespacerange
3 beginbfchar
<0003> <0020>
<001C> <0039>
<0024> <0041>
endbfchar";
        let map = parse_to_unicode_cmap(cmap).unwrap();
        // Decode bytes: 0x0003 0x001C 0x0024 → " 9A"
        let result = map.decode(&[0x00, 0x03, 0x00, 0x1C, 0x00, 0x24]);
        assert_eq!(result, " 9A");
    }
}
