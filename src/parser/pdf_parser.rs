//! PDF document parser.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use crate::detect::detect_format_from_path;
use crate::error::{Error, Result};
use crate::model::{
    Block, Document, Metadata, Outline, OutlineItem, Page, Paragraph, Resource, ResourceType,
};

use super::backend::{LopdfBackend, PdfBackend, RawOutlineItem, RawXObject};
use super::layout::{BlockType, LayoutAnalyzer};
use super::options::{ErrorMode, ExtractMode, ParseOptions};
use super::table_detector::TableDetector;

/// PDF document parser.
pub struct PdfParser {
    backend: Box<dyn PdfBackend>,
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

        let backend: Box<dyn PdfBackend> = Box::new(LopdfBackend::load_file(path)?);

        if options.password.is_some() && backend.metadata().encrypted {
            log::warn!("Password was provided but PDF decryption is not supported");
        }

        Ok(Self { backend, options })
    }

    /// Parse a PDF from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::from_bytes_with_options(data, ParseOptions::default())
    }

    /// Parse a PDF from bytes with custom options.
    pub fn from_bytes_with_options(data: &[u8], options: ParseOptions) -> Result<Self> {
        let backend: Box<dyn PdfBackend> = Box::new(LopdfBackend::load_bytes(data)?);

        if options.password.is_some() && backend.metadata().encrypted {
            log::warn!("Password was provided but PDF decryption is not supported");
        }

        Ok(Self { backend, options })
    }

    /// Parse a PDF from a reader.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self> {
        Self::from_reader_with_options(reader, ParseOptions::default())
    }

    /// Parse a PDF from a reader with custom options.
    pub fn from_reader_with_options<R: Read>(reader: R, options: ParseOptions) -> Result<Self> {
        let backend: Box<dyn PdfBackend> = Box::new(LopdfBackend::load_reader(reader)?);

        if options.password.is_some() && backend.metadata().encrypted {
            log::warn!("Password was provided but PDF decryption is not supported");
        }

        Ok(Self { backend, options })
    }

    /// Parse the document and return a structured Document.
    pub fn parse(&self) -> Result<Document> {
        let mut document = Document::new();

        // Extract metadata
        document.metadata = self.extract_metadata()?;

        // Extract pages
        let page_ids = self.backend.pages();
        let total_pages = page_ids.len() as u32;
        document.metadata.page_count = total_pages;

        for (page_num, _page_id) in page_ids.iter() {
            let page_num = *page_num;

            // Check page selection
            if !self.options.pages.includes(page_num) {
                continue;
            }

            match self.parse_page(page_num) {
                Ok(page) => document.add_page(page),
                Err(e) => {
                    if self.options.error_mode == ErrorMode::Strict {
                        return Err(e);
                    }
                    log::warn!("Skipping page {}: {}", page_num, e);
                }
            }
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
        let raw = self.backend.metadata();
        let mut metadata = Metadata::with_version(raw.version);
        metadata.title = raw.title;
        metadata.author = raw.author;
        metadata.subject = raw.subject;
        metadata.keywords = raw.keywords;
        metadata.creator = raw.creator;
        metadata.producer = raw.producer;
        metadata.encrypted = raw.encrypted;
        if let Some(date_str) = raw.creation_date {
            metadata.created = parse_pdf_date(&date_str);
        }
        if let Some(date_str) = raw.mod_date {
            metadata.modified = parse_pdf_date(&date_str);
        }
        Ok(metadata)
    }

    /// Parse a single page.
    fn parse_page(&self, page_num: u32) -> Result<Page> {
        // Get page dimensions
        let (width, height) = self.get_page_dimensions(page_num)?;
        let mut page = Page::new(page_num, width, height);

        // Extract text content
        if self.options.extract_mode != ExtractMode::StructureOnly {
            // Try layout-aware extraction with table detection
            match self.extract_page_with_tables(page_num) {
                Ok(blocks) if !blocks.is_empty() => {
                    for block in blocks {
                        page.add_block(block);
                    }
                }
                Ok(_) => {
                    // Layout analysis returned empty, use fallback
                    self.fallback_text_extraction(&mut page, page_num)?;
                }
                Err(_) => {
                    // Layout analysis failed, use fallback
                    self.fallback_text_extraction(&mut page, page_num)?;
                }
            }
        }

        Ok(page)
    }

    /// Extract page content with table detection.
    fn extract_page_with_tables(&self, page_num: u32) -> Result<Vec<Block>> {
        let analyzer = LayoutAnalyzer::new(&*self.backend);

        // Step 1: Extract all text spans
        let spans = analyzer.extract_page_spans(page_num)?;

        if spans.is_empty() {
            return Ok(vec![]);
        }

        // Step 2: Detect tables
        let table_detector = TableDetector::new();
        let (detected_tables, remaining_spans) = table_detector.detect(spans.clone());

        let mut blocks: Vec<Block> = Vec::new();

        // If tables were detected, process them along with remaining text
        if !detected_tables.is_empty() {
            log::debug!(
                "Detected {} tables on page {}",
                detected_tables.len(),
                page_num
            );

            // Collect all elements with their Y position for proper ordering
            let mut elements: Vec<(f32, Block)> = Vec::new();

            // Add detected tables
            for detected in &detected_tables {
                let table = table_detector.to_table_model(detected);
                if !table.is_empty() {
                    elements.push((detected.top_y, Block::Table(table)));
                }
            }

            // Process remaining spans into text blocks
            if !remaining_spans.is_empty() {
                let mut analyzer = LayoutAnalyzer::new(&*self.backend);
                // Manually add font stats from remaining spans
                for span in &remaining_spans {
                    analyzer.font_stats_mut().add_size(span.font_size);
                }
                analyzer.font_stats_mut().analyze();

                let lines = analyzer.group_spans_into_lines_pub(remaining_spans);
                let lines = analyzer.detect_headings_pub(lines);
                let text_blocks = analyzer.group_lines_into_blocks_pub(lines);

                for block in text_blocks {
                    if !block.is_empty() {
                        let text = block.text();
                        let y_pos = block.lines.first().map(|l| l.y).unwrap_or(0.0);

                        let para_block = match block.block_type {
                            BlockType::Heading => {
                                let level = block.heading_level.clamp(1, 6);
                                Block::Paragraph(Paragraph::heading(text, level))
                            }
                            BlockType::Paragraph | BlockType::Unknown => {
                                Block::Paragraph(Paragraph::with_text(text))
                            }
                            BlockType::ListItem => {
                                Block::Paragraph(Paragraph::with_text(format!("• {}", text)))
                            }
                        };
                        elements.push((y_pos, para_block));
                    }
                }
            }

            // Sort by Y position (descending for PDF coords - top to bottom)
            elements.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            blocks = elements.into_iter().map(|(_, block)| block).collect();
        } else {
            // No tables detected, use regular layout analysis
            match self.extract_page_with_layout(page_num) {
                Ok(text_blocks) => {
                    for block in text_blocks {
                        if !block.is_empty() {
                            let text = block.text();
                            log::debug!(
                                "Block type: {:?}, heading_level: {}, text preview: {}",
                                block.block_type,
                                block.heading_level,
                                {
                                    let t = text
                                        .char_indices()
                                        .nth(50)
                                        .map_or(text.as_str(), |(i, _)| &text[..i]);
                                    t
                                }
                            );
                            let para_block = match block.block_type {
                                BlockType::Heading => {
                                    let level = block.heading_level.clamp(1, 6);
                                    Block::Paragraph(Paragraph::heading(text, level))
                                }
                                BlockType::Paragraph | BlockType::Unknown => {
                                    Block::Paragraph(Paragraph::with_text(text))
                                }
                                BlockType::ListItem => {
                                    Block::Paragraph(Paragraph::with_text(format!("• {}", text)))
                                }
                            };
                            blocks.push(para_block);
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok(blocks)
    }

    /// Extract page with layout analysis.
    fn extract_page_with_layout(&self, page_num: u32) -> Result<Vec<super::layout::TextBlock>> {
        let mut analyzer = LayoutAnalyzer::new(&*self.backend);
        analyzer.extract_page_blocks(page_num)
    }

    /// Fallback text extraction without layout analysis.
    fn fallback_text_extraction(&self, page: &mut Page, page_num: u32) -> Result<()> {
        match self.extract_page_text(page_num) {
            Ok(text) => {
                if !text.trim().is_empty() {
                    page.add_paragraph(Paragraph::with_text(text));
                }
            }
            Err(e) => {
                if self.options.error_mode == ErrorMode::Strict {
                    return Err(e);
                }
                log::warn!("Failed to extract text from page {}: {}", page_num, e);
            }
        }
        Ok(())
    }

    /// Get page dimensions.
    fn get_page_dimensions(&self, page_num: u32) -> Result<(f32, f32)> {
        let pages = self.backend.pages();
        let page_id = pages
            .get(&page_num)
            .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;
        Ok(self.backend.page_dimensions(*page_id))
    }

    /// Extract text from a page using layout-aware span extraction.
    fn extract_page_text(&self, page_num: u32) -> Result<String> {
        let analyzer = LayoutAnalyzer::new(&*self.backend);
        let spans = analyzer.extract_page_spans(page_num)?;

        if spans.is_empty() {
            return Ok(String::new());
        }

        let lines = analyzer.group_spans_into_lines_pub(spans);
        Ok(lines
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Extract document outline (bookmarks).
    fn extract_outline(&self) -> Result<Outline> {
        let raw_items = self.backend.outline()?;
        let mut outline = Outline::new();
        outline.items = raw_items
            .into_iter()
            .map(Self::convert_outline_item)
            .collect();
        Ok(outline)
    }

    /// Convert a raw outline item from the backend into a model OutlineItem.
    fn convert_outline_item(raw: RawOutlineItem) -> OutlineItem {
        let mut item = OutlineItem::new(raw.title, raw.page, raw.level);
        item.children = raw
            .children
            .into_iter()
            .map(Self::convert_outline_item)
            .collect();
        item
    }

    /// Extract embedded resources (images).
    fn extract_resources(&self) -> Result<HashMap<String, Resource>> {
        let mut resources = HashMap::new();
        for (page_num, page_id) in self.backend.pages() {
            if let Ok(xobjects) = self.backend.page_xobjects(page_id) {
                for xobj in xobjects {
                    let key = format!("page{}_{}", page_num, xobj.name);
                    if let Some(resource) = Self::convert_xobject(xobj) {
                        resources.insert(key, resource);
                    }
                }
            }
        }
        Ok(resources)
    }

    /// Convert a raw XObject into a model Resource.
    fn convert_xobject(xobj: RawXObject) -> Option<Resource> {
        let mime_type = match xobj.filter.as_deref() {
            Some("DCTDecode") => "image/jpeg",
            Some("JPXDecode") => "image/jp2",
            _ => "application/octet-stream",
        };
        let mut resource = Resource::new(xobj.data, mime_type.to_string(), ResourceType::Image);
        if let (Some(w), Some(h)) = (xobj.width, xobj.height) {
            resource = resource.with_dimensions(w, h);
        }
        if let Some(b) = xobj.bits_per_component {
            resource = resource.with_bits_per_component(b);
        }
        if let Some(cs) = xobj.color_space {
            resource = resource.with_color_space(cs);
        }
        Some(resource)
    }

    /// Get the number of pages.
    pub fn page_count(&self) -> u32 {
        self.backend.pages().len() as u32
    }

    /// Check if the document is encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.backend.metadata().encrypted
    }

    /// Get PDF version.
    pub fn version(&self) -> String {
        self.backend.metadata().version
    }
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
