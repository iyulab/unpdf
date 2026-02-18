//! PDF backend abstraction layer.
//!
//! Provides a trait-based interface for PDF operations, isolating
//! the concrete PDF library (lopdf) from the layout analysis logic.

use std::collections::BTreeMap;

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
// LopdfBackend — concrete implementation backed by lopdf
// ---------------------------------------------------------------------------

use lopdf::{Document as LopdfDocument, Object};

/// Concrete [`PdfBackend`] backed by `lopdf::Document`.
pub struct LopdfBackend {
    doc: LopdfDocument,
}

impl LopdfBackend {
    /// Load from a file path.
    pub fn load_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let doc = LopdfDocument::load(path).map_err(|e| match e {
            lopdf::Error::Decryption(_) => Error::Encrypted,
            _ => Error::from(e),
        })?;
        Ok(Self { doc })
    }

    /// Load from an in-memory byte slice.
    pub fn load_bytes(data: &[u8]) -> Result<Self> {
        let doc = LopdfDocument::load_mem(data).map_err(|e| match e {
            lopdf::Error::Decryption(_) => Error::Encrypted,
            _ => Error::from(e),
        })?;
        Ok(Self { doc })
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
        if let Ok(lopdf_fonts) = self.doc.get_page_fonts(page) {
            if let Some(font_dict) = lopdf_fonts.get(font_name) {
                if let Ok(enc) = font_dict.get_font_encoding(&self.doc) {
                    if let Ok(text) = LopdfDocument::decode_text(&enc, bytes) {
                        return text;
                    }
                }
            }
        }
        decode_text_simple(bytes)
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
}
