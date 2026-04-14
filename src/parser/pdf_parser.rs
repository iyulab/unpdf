//! PDF document parser.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use crate::detect::detect_format_from_path;
use crate::error::{Error, Result};
use crate::model::{Block, Document, OutlineItem, Page, Paragraph, Resource, ResourceType};

use super::backend::{PdfBackend, RawBackend, RawXObject};
use super::options::{ErrorMode, ExtractMode, ParseOptions};

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

        // Decryption (empty password) is attempted inside RawDocument::load().
        // If we get here, the PDF is usable (either not encrypted, or decrypted).
        let backend: Box<dyn PdfBackend> = Box::new(RawBackend::load_file(path)?);

        Ok(Self { backend, options })
    }

    /// Parse a PDF from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::from_bytes_with_options(data, ParseOptions::default())
    }

    /// Parse a PDF from bytes with custom options.
    pub fn from_bytes_with_options(data: &[u8], options: ParseOptions) -> Result<Self> {
        let backend: Box<dyn PdfBackend> = Box::new(RawBackend::load_bytes(data)?);
        Ok(Self { backend, options })
    }

    /// Parse a PDF from a reader.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self> {
        Self::from_reader_with_options(reader, ParseOptions::default())
    }

    /// Parse a PDF from a reader with custom options.
    pub fn from_reader_with_options<R: Read>(reader: R, options: ParseOptions) -> Result<Self> {
        let backend: Box<dyn PdfBackend> = Box::new(RawBackend::load_reader(reader)?);
        Ok(Self { backend, options })
    }

    /// Parse the document and return a structured Document.
    ///
    /// Internally routes through the streaming pipeline (`run_stream`) with
    /// rayon parallel page parsing. The public signature is unchanged.
    pub fn parse(&self) -> Result<Document> {
        use std::ops::ControlFlow;

        use super::stream::{run_stream, PageStreamOptions, ParseEvent};

        let opts: PageStreamOptions = (&self.options).into();

        let mut document = Document::new();
        let mut err_out: Option<Error> = None;

        // Snapshot page map so we can do resource extraction inside the handler.
        let page_ids = self.backend.pages();

        let quality = run_stream(&*self.backend, &opts, |ev| match ev {
            ParseEvent::DocumentStart {
                metadata,
                outline,
                form_fields,
                ..
            } => {
                document.metadata = metadata;
                document.outline = outline;
                document.form_fields = form_fields;
                ControlFlow::Continue(())
            }
            ParseEvent::PageParsed(page) => {
                if self.options.extract_resources
                    && self.options.extract_mode != ExtractMode::StructureOnly
                {
                    if let Some(page_id) = page_ids.get(&page.number) {
                        if let Ok(xobjects) = self.backend.page_xobjects(*page_id) {
                            for xobj in xobjects {
                                let key = format!("page{}_{}", page.number, xobj.name);
                                if let Some(r) = Self::convert_xobject(xobj) {
                                    document.resources.insert(key, r);
                                }
                            }
                        }
                    }
                }
                document.add_page(page);
                ControlFlow::Continue(())
            }
            ParseEvent::PageFailed { page, error } => {
                log::warn!("page {} failed: {}", page, error);
                if self.options.error_mode == ErrorMode::Strict && err_out.is_none() {
                    err_out = Some(error);
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(())
            }
            ParseEvent::Progress { .. } | ParseEvent::DocumentEnd { .. } => {
                ControlFlow::Continue(())
            }
        })?;

        if let Some(e) = err_out {
            return Err(e);
        }

        let mut final_q = quality;
        final_q.encrypted = document.metadata.encrypted;
        document.extraction_quality = final_q;

        Ok(document)
    }

    /// Extract embedded resources (images).
    #[allow(dead_code)]
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

    /// Stream pages in `page_num` ASC order via the provided callback.
    ///
    /// The callback receives `ParseEvent::DocumentStart`, then `PageParsed` /
    /// `PageFailed` / `Progress` events, and finally `DocumentEnd`. Return
    /// `ControlFlow::Break(())` from the callback to terminate early.
    ///
    /// Memory stays bounded because the pipeline consumes pages as the callback
    /// drains them — unlike [`PdfParser::parse`], the whole document is never
    /// materialized. Intended for very large PDFs.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::ops::ControlFlow;
    /// use unpdf::{PdfParser, PageStreamOptions, ParseEvent};
    ///
    /// let parser = PdfParser::open("large.pdf")?;
    /// parser.for_each_page(PageStreamOptions::default(), |ev| {
    ///     if let ParseEvent::PageParsed(page) = ev {
    ///         println!("page {}: {} blocks", page.number, page.elements.len());
    ///     }
    ///     ControlFlow::Continue(())
    /// })?;
    /// # Ok::<(), unpdf::Error>(())
    /// ```
    pub fn for_each_page<F>(
        &self,
        opts: super::stream::PageStreamOptions,
        f: F,
    ) -> Result<crate::model::ExtractionQuality>
    where
        F: FnMut(super::stream::ParseEvent) -> std::ops::ControlFlow<()>,
    {
        super::stream::run_stream(&*self.backend, &opts, f)
    }
}

// ---------------------------------------------------------------------------
// Module-level free functions (backend-agnostic page parsing)
// ---------------------------------------------------------------------------

/// Parse a single page without requiring `&PdfParser`. Enables per-page
/// parallel invocation in `run_stream`.
pub(crate) fn parse_single_page(
    backend: &dyn PdfBackend,
    page_num: u32,
    options: &ParseOptions,
) -> Result<Page> {
    let (width, height) = get_page_dimensions_fn(backend, page_num)?;
    let mut page = Page::new(page_num, width, height);

    if options.extract_mode != ExtractMode::StructureOnly {
        match extract_page_with_tables_fn(backend, page_num) {
            Ok(blocks) if !blocks.is_empty() => {
                for block in blocks {
                    page.add_block(block);
                }
            }
            Ok(_) => {
                fallback_text_extraction_fn(backend, &mut page, page_num, options)?;
            }
            Err(_) => {
                fallback_text_extraction_fn(backend, &mut page, page_num, options)?;
            }
        }
    }

    Ok(page)
}

/// Convert a raw outline item into a model `OutlineItem`. Exposed as
/// `pub(crate)` so `run_stream` can build the document outline.
pub(crate) fn convert_outline_item_pub(raw: super::backend::RawOutlineItem) -> OutlineItem {
    let mut item = OutlineItem::new(raw.title, raw.page, raw.level);
    item.children = raw
        .children
        .into_iter()
        .map(convert_outline_item_pub)
        .collect();
    item
}

fn get_page_dimensions_fn(backend: &dyn PdfBackend, page_num: u32) -> Result<(f32, f32)> {
    let pages = backend.pages();
    let page_id = pages
        .get(&page_num)
        .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;
    Ok(backend.page_dimensions(*page_id))
}

fn extract_page_with_tables_fn(backend: &dyn PdfBackend, page_num: u32) -> Result<Vec<Block>> {
    let analyzer = super::layout::LayoutAnalyzer::new(backend);
    let spans = analyzer.extract_page_spans(page_num)?;

    if spans.is_empty() {
        return Ok(vec![]);
    }

    let table_detector = super::table_detector::TableDetector::new();
    let (detected_tables, remaining_spans) = table_detector.detect(spans.clone());

    let mut blocks: Vec<Block> = Vec::new();

    if !detected_tables.is_empty() {
        log::debug!(
            "Detected {} tables on page {}",
            detected_tables.len(),
            page_num
        );

        let mut elements: Vec<(f32, Block)> = Vec::new();

        const TABLE_CONFIDENCE_THRESHOLD: f32 = 0.4;
        for detected in &detected_tables {
            if detected.confidence < TABLE_CONFIDENCE_THRESHOLD {
                log::debug!(
                    "Table at y={} has low confidence ({:.2}), converting to paragraphs",
                    detected.top_y,
                    detected.confidence
                );
                for row in &detected.rows {
                    let text = row
                        .spans
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>()
                        .join("  ");
                    if !text.trim().is_empty() {
                        elements.push((row.y, Block::Paragraph(Paragraph::with_text(text))));
                    }
                }
            } else {
                let table = table_detector.to_table_model(detected);
                if !table.is_empty() {
                    elements.push((detected.top_y, Block::Table(table)));
                }
            }
        }

        if !remaining_spans.is_empty() {
            let mut a = super::layout::LayoutAnalyzer::new(backend);
            for span in &remaining_spans {
                a.font_stats_mut().add_size(span.font_size);
            }
            a.font_stats_mut().analyze();

            let lines = a.group_spans_into_lines_pub(remaining_spans);
            let lines = a.detect_headings_pub(lines);
            let text_blocks = a.group_lines_into_blocks_pub(lines);

            for block in text_blocks {
                if !block.is_empty() {
                    let text = block.text();
                    let y_pos = block.lines.first().map(|l| l.y).unwrap_or(0.0);
                    let para_block = match block.block_type {
                        super::layout::BlockType::Heading => {
                            let level = block.heading_level.clamp(1, 6);
                            Block::Paragraph(Paragraph::heading(text, level))
                        }
                        super::layout::BlockType::Paragraph | super::layout::BlockType::Unknown => {
                            Block::Paragraph(Paragraph::with_text(text))
                        }
                        super::layout::BlockType::ListItem => {
                            Block::Paragraph(Paragraph::with_text(format!("• {}", text)))
                        }
                    };
                    elements.push((y_pos, para_block));
                }
            }
        }

        elements.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        blocks = elements.into_iter().map(|(_, block)| block).collect();
    } else {
        let mut a = super::layout::LayoutAnalyzer::new(backend);
        let text_blocks = a.extract_page_blocks(page_num)?;
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
                    super::layout::BlockType::Heading => {
                        let level = block.heading_level.clamp(1, 6);
                        Block::Paragraph(Paragraph::heading(text, level))
                    }
                    super::layout::BlockType::Paragraph | super::layout::BlockType::Unknown => {
                        Block::Paragraph(Paragraph::with_text(text))
                    }
                    super::layout::BlockType::ListItem => {
                        Block::Paragraph(Paragraph::with_text(format!("• {}", text)))
                    }
                };
                blocks.push(para_block);
            }
        }
    }

    Ok(blocks)
}

fn fallback_text_extraction_fn(
    backend: &dyn PdfBackend,
    page: &mut Page,
    page_num: u32,
    options: &ParseOptions,
) -> Result<()> {
    let analyzer = super::layout::LayoutAnalyzer::new(backend);
    match analyzer.extract_page_spans(page_num) {
        Ok(spans) if !spans.is_empty() => {
            let lines = analyzer.group_spans_into_lines_pub(spans);
            let text = lines
                .iter()
                .map(|l| l.text())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.trim().is_empty() {
                page.add_paragraph(Paragraph::with_text(text));
            }
        }
        Ok(_) => {}
        Err(e) => {
            if options.error_mode == ErrorMode::Strict {
                return Err(e);
            }
            log::warn!("Failed to extract text from page {}: {}", page_num, e);
        }
    }
    Ok(())
}

/// Parse a PDF date string (D:YYYYMMDDHHmmSSOHH'mm'). Exposed as `pub(crate)` for `run_stream`.
pub(crate) fn parse_pdf_date_pub(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    parse_pdf_date(s)
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
