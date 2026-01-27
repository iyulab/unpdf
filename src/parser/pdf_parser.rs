//! PDF document parser using lopdf.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use lopdf::Document as LopdfDocument;

use crate::detect::detect_format_from_path;
use crate::error::{Error, Result};
use crate::model::{
    Document, Metadata, Outline, OutlineItem, Page, Paragraph, Resource, ResourceType,
};

use super::options::{ErrorMode, ExtractMode, ParseOptions};

/// PDF document parser.
pub struct PdfParser {
    doc: LopdfDocument,
    options: ParseOptions,
}

impl PdfParser {
    /// Open a PDF file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_options(path, ParseOptions::default())
    }

    /// Open a PDF file with custom options.
    pub fn open_with_options<P: AsRef<Path>>(path: P, options: ParseOptions) -> Result<Self> {
        let path = path.as_ref();

        // Verify it's a PDF
        detect_format_from_path(path)?;

        // Load document
        let doc = LopdfDocument::load(path).map_err(|e| match e {
            lopdf::Error::Decryption(_) => Error::Encrypted,
            _ => Error::from(e),
        })?;

        // Note: Password-protected PDFs are not yet supported in lopdf 0.34
        // TODO: Add password support when lopdf adds this feature
        if options.password.is_some() && doc.is_encrypted() {
            log::warn!("Password was provided but lopdf 0.34 doesn't support decryption");
        }

        Ok(Self { doc, options })
    }

    /// Parse a PDF from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::from_bytes_with_options(data, ParseOptions::default())
    }

    /// Parse a PDF from bytes with custom options.
    pub fn from_bytes_with_options(data: &[u8], options: ParseOptions) -> Result<Self> {
        let doc = LopdfDocument::load_mem(data).map_err(|e| match e {
            lopdf::Error::Decryption(_) => Error::Encrypted,
            _ => Error::from(e),
        })?;

        // Note: Password-protected PDFs are not yet supported in lopdf 0.34
        if options.password.is_some() && doc.is_encrypted() {
            log::warn!("Password was provided but lopdf 0.34 doesn't support decryption");
        }

        Ok(Self { doc, options })
    }

    /// Parse a PDF from a reader.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self> {
        Self::from_reader_with_options(reader, ParseOptions::default())
    }

    /// Parse a PDF from a reader with custom options.
    pub fn from_reader_with_options<R: Read>(mut reader: R, options: ParseOptions) -> Result<Self> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        Self::from_bytes_with_options(&data, options)
    }

    /// Parse the document and return a structured Document.
    pub fn parse(&self) -> Result<Document> {
        let mut document = Document::new();

        // Extract metadata
        document.metadata = self.extract_metadata()?;

        // Extract pages
        let page_ids = self.doc.get_pages();
        let total_pages = page_ids.len() as u32;
        document.metadata.page_count = total_pages;

        for (page_num, _page_id) in page_ids.iter() {
            let page_num = *page_num;

            // Check page selection
            if !self.options.pages.includes(page_num) {
                continue;
            }

            let page = self.parse_page(page_num)?;
            document.add_page(page);
        }

        // Extract outline (bookmarks) if available
        if let Ok(outline) = self.extract_outline() {
            if !outline.is_empty() {
                document.outline = Some(outline);
            }
        }

        // Extract resources (images) if requested
        if self.options.extract_resources && self.options.extract_mode != ExtractMode::StructureOnly
        {
            if let Ok(resources) = self.extract_resources() {
                document.resources = resources;
            }
        }

        Ok(document)
    }

    /// Extract document metadata.
    fn extract_metadata(&self) -> Result<Metadata> {
        let mut metadata = Metadata::with_version(self.doc.version.to_string());

        // Try to get document info dictionary
        if let Ok(info) = self.doc.trailer.get(b"Info") {
            if let Ok(info_ref) = info.as_reference() {
                if let Ok(info_dict) = self.doc.get_dictionary(info_ref) {
                    metadata.title = get_string_from_dict(info_dict, b"Title");
                    metadata.author = get_string_from_dict(info_dict, b"Author");
                    metadata.subject = get_string_from_dict(info_dict, b"Subject");
                    metadata.keywords = get_string_from_dict(info_dict, b"Keywords");
                    metadata.creator = get_string_from_dict(info_dict, b"Creator");
                    metadata.producer = get_string_from_dict(info_dict, b"Producer");

                    // Parse dates
                    if let Some(date_str) = get_string_from_dict(info_dict, b"CreationDate") {
                        metadata.created = parse_pdf_date(&date_str);
                    }
                    if let Some(date_str) = get_string_from_dict(info_dict, b"ModDate") {
                        metadata.modified = parse_pdf_date(&date_str);
                    }
                }
            }
        }

        // Check if encrypted
        metadata.encrypted = self.doc.is_encrypted();

        Ok(metadata)
    }

    /// Parse a single page.
    fn parse_page(&self, page_num: u32) -> Result<Page> {
        // Get page dimensions
        let (width, height) = self.get_page_dimensions(page_num)?;
        let mut page = Page::new(page_num, width, height);

        // Extract text content
        if self.options.extract_mode != ExtractMode::StructureOnly {
            match self.extract_page_text(page_num) {
                Ok(text) => {
                    if !text.trim().is_empty() {
                        // For now, add as a single paragraph
                        // TODO: Implement proper layout analysis
                        page.add_paragraph(Paragraph::with_text(text));
                    }
                }
                Err(e) => {
                    if self.options.error_mode == ErrorMode::Strict {
                        return Err(e);
                    }
                    // In lenient mode, skip this page's text
                    log::warn!("Failed to extract text from page {}: {}", page_num, e);
                }
            }
        }

        Ok(page)
    }

    /// Get page dimensions.
    fn get_page_dimensions(&self, page_num: u32) -> Result<(f32, f32)> {
        let pages = self.doc.get_pages();
        let page_id = pages
            .get(&page_num)
            .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;

        if let Ok(page_dict) = self.doc.get_dictionary(*page_id) {
            if let Ok(media_box) = page_dict.get(b"MediaBox") {
                if let Ok(array) = media_box.as_array() {
                    if array.len() >= 4 {
                        let width = array[2].as_float().unwrap_or(612.0);
                        let height = array[3].as_float().unwrap_or(792.0);
                        return Ok((width, height));
                    }
                }
            }
        }

        // Default to Letter size
        Ok((612.0, 792.0))
    }

    /// Extract text from a page.
    fn extract_page_text(&self, page_num: u32) -> Result<String> {
        self.doc
            .extract_text(&[page_num])
            .map_err(|e| Error::TextExtract(format!("Page {}: {}", page_num, e)))
    }

    /// Extract document outline (bookmarks).
    fn extract_outline(&self) -> Result<Outline> {
        let mut outline = Outline::new();

        // Get outline root from catalog
        if let Ok(catalog) = self.doc.catalog() {
            if let Ok(outlines) = catalog.get(b"Outlines") {
                if let Ok(outlines_ref) = outlines.as_reference() {
                    if let Ok(outlines_dict) = self.doc.get_dictionary(outlines_ref) {
                        // Get first outline item
                        if let Ok(first) = outlines_dict.get(b"First") {
                            if let Ok(first_ref) = first.as_reference() {
                                self.extract_outline_items(first_ref, 0, &mut outline.items)?;
                            }
                        }
                    }
                }
            }
        }

        Ok(outline)
    }

    /// Recursively extract outline items.
    fn extract_outline_items(
        &self,
        item_ref: lopdf::ObjectId,
        level: u8,
        items: &mut Vec<OutlineItem>,
    ) -> Result<()> {
        if let Ok(item_dict) = self.doc.get_dictionary(item_ref) {
            // Get title
            let title = get_string_from_dict(item_dict, b"Title").unwrap_or_default();

            // Get destination page (simplified)
            let page = self.get_outline_destination(item_dict);

            let mut outline_item = OutlineItem::new(title, page, level);

            // Process children (First)
            if let Ok(first) = item_dict.get(b"First") {
                if let Ok(first_ref) = first.as_reference() {
                    self.extract_outline_items(first_ref, level + 1, &mut outline_item.children)?;
                }
            }

            items.push(outline_item);

            // Process siblings (Next)
            if let Ok(next) = item_dict.get(b"Next") {
                if let Ok(next_ref) = next.as_reference() {
                    self.extract_outline_items(next_ref, level, items)?;
                }
            }
        }

        Ok(())
    }

    /// Get destination page from outline item.
    fn get_outline_destination(&self, item_dict: &lopdf::Dictionary) -> Option<u32> {
        // Try Dest first
        if let Ok(dest) = item_dict.get(b"Dest") {
            return self.resolve_destination(dest);
        }

        // Try A (action) dictionary
        if let Ok(action) = item_dict.get(b"A") {
            if let Ok(action_ref) = action.as_reference() {
                if let Ok(action_dict) = self.doc.get_dictionary(action_ref) {
                    if let Ok(dest) = action_dict.get(b"D") {
                        return self.resolve_destination(dest);
                    }
                }
            }
        }

        None
    }

    /// Resolve a destination to a page number.
    fn resolve_destination(&self, dest: &lopdf::Object) -> Option<u32> {
        let pages = self.doc.get_pages();

        // Destination can be an array or a name
        if let Ok(dest_array) = dest.as_array() {
            if let Some(first) = dest_array.first() {
                if let Ok(page_ref) = first.as_reference() {
                    // Find page number from reference
                    for (num, id) in pages.iter() {
                        if *id == page_ref {
                            return Some(*num);
                        }
                    }
                }
            }
        }

        None
    }

    /// Extract embedded resources (images).
    fn extract_resources(&self) -> Result<HashMap<String, Resource>> {
        let mut resources = HashMap::new();

        for (page_num, page_id) in self.doc.get_pages() {
            if let Ok(page_resources) = self.extract_page_resources(page_id) {
                for (id, resource) in page_resources {
                    let key = format!("page{}_{}", page_num, id);
                    resources.insert(key, resource);
                }
            }
        }

        Ok(resources)
    }

    /// Extract resources from a page.
    fn extract_page_resources(&self, page_id: lopdf::ObjectId) -> Result<Vec<(String, Resource)>> {
        let mut resources = Vec::new();

        if let Ok(page_dict) = self.doc.get_dictionary(page_id) {
            if let Ok(res) = page_dict.get(b"Resources") {
                let res_dict = match res {
                    lopdf::Object::Reference(r) => self.doc.get_dictionary(*r).ok(),
                    lopdf::Object::Dictionary(d) => Some(d),
                    _ => None,
                };

                if let Some(res_dict) = res_dict {
                    // Extract XObjects (images)
                    if let Ok(xobjects) = res_dict.get(b"XObject") {
                        let xobj_dict = match xobjects {
                            lopdf::Object::Reference(r) => self.doc.get_dictionary(*r).ok(),
                            lopdf::Object::Dictionary(d) => Some(d),
                            _ => None,
                        };

                        if let Some(xobj_dict) = xobj_dict {
                            for (name, obj) in xobj_dict.iter() {
                                if let Ok(obj_ref) = obj.as_reference() {
                                    if let Ok(resource) = self.extract_xobject(obj_ref) {
                                        let name_str = String::from_utf8_lossy(name).to_string();
                                        resources.push((name_str, resource));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(resources)
    }

    /// Extract an XObject (image).
    fn extract_xobject(&self, obj_ref: lopdf::ObjectId) -> Result<Resource> {
        let stream = self
            .doc
            .get_object(obj_ref)
            .map_err(|e| Error::ImageExtract(e.to_string()))?;

        if let lopdf::Object::Stream(stream) = stream {
            let dict = &stream.dict;

            // Check if it's an image
            if let Ok(subtype) = dict.get(b"Subtype") {
                match subtype.as_name_str() {
                    Ok("Image") => {}
                    _ => return Err(Error::ImageExtract("Not an image XObject".to_string())),
                }
            }

            // Get image properties
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

            // Get filter to determine format
            let filter = dict
                .get(b"Filter")
                .ok()
                .and_then(|f| f.as_name_str().ok())
                .unwrap_or("");

            let (mime_type, data) = match filter {
                "DCTDecode" => {
                    // JPEG - data can be used directly
                    ("image/jpeg".to_string(), stream.content.clone())
                }
                "FlateDecode" | "LZWDecode" | "" => {
                    // Need to decode and convert to PNG
                    // For now, store raw data
                    let decoded = stream
                        .decompressed_content()
                        .unwrap_or_else(|_| stream.content.clone());
                    ("application/octet-stream".to_string(), decoded)
                }
                "JPXDecode" => ("image/jp2".to_string(), stream.content.clone()),
                _ => (
                    "application/octet-stream".to_string(),
                    stream.content.clone(),
                ),
            };

            let mut resource = Resource::new(data, mime_type, ResourceType::Image);

            if let (Some(w), Some(h)) = (width, height) {
                resource = resource.with_dimensions(w, h);
            }

            if let Some(b) = bits {
                resource = resource.with_bits_per_component(b);
            }

            // Get color space
            if let Ok(cs) = dict.get(b"ColorSpace") {
                let cs_name = match cs {
                    lopdf::Object::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
                    lopdf::Object::Array(arr) => arr
                        .first()
                        .and_then(|o| o.as_name_str().ok())
                        .map(String::from),
                    _ => None,
                };
                if let Some(cs_name) = cs_name {
                    resource = resource.with_color_space(cs_name);
                }
            }

            return Ok(resource);
        }

        Err(Error::ImageExtract("Invalid XObject".to_string()))
    }

    /// Get the number of pages.
    pub fn page_count(&self) -> u32 {
        self.doc.get_pages().len() as u32
    }

    /// Check if the document is encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.doc.is_encrypted()
    }

    /// Get PDF version.
    pub fn version(&self) -> String {
        self.doc.version.to_string()
    }
}

/// Helper to get a string from a PDF dictionary.
fn get_string_from_dict(dict: &lopdf::Dictionary, key: &[u8]) -> Option<String> {
    dict.get(key).ok().and_then(|obj| {
        match obj {
            lopdf::Object::String(bytes, _) => {
                // Try UTF-16BE first (PDF standard for Unicode)
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
                    // Try as Latin-1 or UTF-8
                    String::from_utf8(bytes.clone())
                        .ok()
                        .or_else(|| Some(bytes.iter().map(|&b| b as char).collect()))
                }
            }
            lopdf::Object::Name(bytes) => String::from_utf8(bytes.clone()).ok(),
            _ => None,
        }
    })
}

/// Parse a PDF date string (D:YYYYMMDDHHmmSSOHH'mm').
fn parse_pdf_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let s = s.strip_prefix("D:")?;

    // At minimum we need YYYY
    if s.len() < 4 {
        return None;
    }

    let year: i32 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(4..6).and_then(|m| m.parse().ok()).unwrap_or(1);
    let day: u32 = s.get(6..8).and_then(|d| d.parse().ok()).unwrap_or(1);
    let hour: u32 = s.get(8..10).and_then(|h| h.parse().ok()).unwrap_or(0);
    let minute: u32 = s.get(10..12).and_then(|m| m.parse().ok()).unwrap_or(0);
    let second: u32 = s.get(12..14).and_then(|s| s.parse().ok()).unwrap_or(0);

    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, minute, second))
        .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn test_parse_pdf_date() {
        let date = parse_pdf_date("D:20240115103045").unwrap();
        assert_eq!(date.year(), 2024);
        assert_eq!(date.month(), 1);
        assert_eq!(date.day(), 15);
    }

    #[test]
    fn test_parse_pdf_date_minimal() {
        let date = parse_pdf_date("D:2024").unwrap();
        assert_eq!(date.year(), 2024);
        assert_eq!(date.month(), 1);
        assert_eq!(date.day(), 1);
    }
}
