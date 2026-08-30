//! PDF document parser.

use std::io::Read;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use crate::detect::detect_format_from_path;
use crate::error::{Error, Result};
use crate::model::{
    Block, Document, ListInfo, OutlineItem, Page, Paragraph, Resource, ResourceType,
};

use super::backend::{PdfBackend, RawBackend, RawXObject};
use super::options::{ErrorMode, ExtractMode, ParseOptions};

/// PDF document parser.
pub struct PdfParser {
    backend: Box<dyn PdfBackend>,
    options: ParseOptions,
}

impl PdfParser {
    /// Open a PDF file.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_options(path, ParseOptions::default())
    }

    /// Open a PDF file with custom options.
    #[cfg(not(target_arch = "wasm32"))]
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
                                // The unsupported-image quality signal is counted once, from
                                // `parse_single_page`'s pass over the same XObjects (below) —
                                // not duplicated here.
                                let (resource, _unsupported) = convert_resource_xobject(
                                    xobj,
                                    self.options.min_image_dimension,
                                );
                                if let Some(r) = resource {
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
        // One analyzer per page: the text paths below share its font statistics and
        // its record of whether an unreadable OCR layer was dropped.
        let mut analyzer = super::layout::LayoutAnalyzer::new(backend)
            .with_ocr_suppression(options.suppress_low_confidence_ocr);

        match extract_page_with_tables_fn(&mut analyzer, page_num) {
            Ok(blocks) if !blocks.is_empty() => {
                for block in blocks {
                    page.add_block(block);
                }
            }
            _ => {
                fallback_text_extraction_fn(&analyzer, &mut page, page_num, options)?;
            }
        }

        page.ocr_text_suppressed = analyzer.ocr_text_suppressed();
        page.suppressed_text_runs = analyzer.suppressed_text_runs();
        let (text_ops, image_ops) = analyzer.page_op_counts();
        page.text_op_count = text_ops;
        page.image_op_count = image_ops;
    }

    // 이미지(XObject) 수집 — extract_resources 가 활성화된 경우.
    // 현재는 정확한 Y 좌표 해석이 안 되어 페이지 말미에 순서대로 append.
    // 향후 Phase 2 에서 content-stream 의 Do 연산자 위치 해석으로 interleave 예정.
    // id 는 확장자 포함: `page{N}_{name}.{ext}`. 이 id 를 곧 이미지의
    // 파일명으로도 사용하므로 writer 측에서 별도 suggested_filename 호출 불필요.
    if options.extract_resources && options.extract_mode != ExtractMode::StructureOnly {
        let pages = backend.pages();
        if let Some(page_id) = pages.get(&page_num) {
            if let Ok(xobjects) = backend.page_xobjects(*page_id) {
                for xobj in xobjects {
                    let base_id = format!("page{}_{}", page_num, xobj.name);
                    let (resource, unsupported) =
                        convert_resource_xobject(xobj, options.min_image_dimension);
                    if unsupported {
                        page.unsupported_image_count += 1;
                    }
                    if let Some(resource) = resource {
                        let id = resource.suggested_filename(&base_id);
                        let mut img_block = Block::image(id.clone());
                        if let Block::Image {
                            width: bw,
                            height: bh,
                            ..
                        } = &mut img_block
                        {
                            *bw = resource.width.map(|w| w as f32);
                            *bh = resource.height.map(|h| h as f32);
                        }
                        page.add_block(img_block);
                        page.images.push((id, resource));
                    }
                }
            }
        }
    }

    Ok(page)
}

/// Free-function version of `PdfParser::convert_xobject` so `parse_single_page`
/// (and other `run_stream` consumers) can use it without needing `&self`.
pub(crate) fn convert_xobject_pub(xobj: RawXObject) -> Option<Resource> {
    let RawXObject {
        data,
        filter,
        width,
        height,
        bits_per_component,
        color_space,
        ..
    } = xobj;

    let (data, mime_type) = match filter.as_deref() {
        Some("DCTDecode") => (data, "image/jpeg"),
        Some("JPXDecode") => (data, "image/jp2"),
        Some("FlateDecode") => {
            match reencode_flate_image_as_png(
                &data,
                width,
                height,
                bits_per_component,
                color_space.as_deref(),
            ) {
                Some(png) => (png, "image/png"),
                None => (data, "application/octet-stream"),
            }
        }
        _ => (data, "application/octet-stream"),
    };

    let mut resource = Resource::new(data, mime_type.to_string(), ResourceType::Image);
    if let (Some(w), Some(h)) = (width, height) {
        resource = resource.with_dimensions(w, h);
    }
    if let Some(b) = bits_per_component {
        resource = resource.with_bits_per_component(b);
    }
    if let Some(cs) = color_space {
        resource = resource.with_color_space(cs);
    }
    Some(resource)
}

/// Re-encode an already-inflated `/FlateDecode` image XObject's raw scanlines as PNG.
///
/// Stage-1 scope: 8-bit `DeviceGray`/`DeviceRGB` only (`backend::resolve_color_space_name`
/// already folds `ICCBased` down to its device-equivalent by component count before this
/// runs). `None` means "not eligible", not "encoding failed" — the caller falls back to the
/// existing raw/undecoded-drop path. See
/// `claudedocs/unpdf/issues/ISSUE-unpdf-20260828-123513-flatedecode-images-unconditionally-dropped.md`
/// in the umbrella repo for the staged-rollout rationale and the follow-up scope (Indexed/CMYK).
fn reencode_flate_image_as_png(
    data: &[u8],
    width: Option<u32>,
    height: Option<u32>,
    bits_per_component: Option<u8>,
    color_space: Option<&str>,
) -> Option<Vec<u8>> {
    if bits_per_component != Some(8) {
        return None;
    }
    let color_type = match color_space {
        Some("DeviceGray") | Some("CalGray") => super::png_encode::PngColorType::Gray,
        Some("DeviceRGB") | Some("CalRGB") => super::png_encode::PngColorType::Rgb,
        _ => return None,
    };
    super::png_encode::encode(width?, height?, color_type, data)
}

/// Convert an XObject into a resource for the document's resource inventory, applying the
/// filtering `extract_resources` consumers rely on: unsupported raw/undecoded image formats
/// (most Markdown/GetResourceData consumers can't render them) and images below
/// `min_image_dimension` (decorative logos, rule lines, tracking pixels) are dropped. `None`
/// means the XObject was filtered out, not that conversion failed.
///
/// The single gate for both [`PdfParser::parse`]'s resource collection and
/// [`parse_single_page`]'s inline-block collection — they must apply identical filtering, or
/// `ParseOptions::min_image_dimension`'s documented default silently stops applying to
/// whichever caller's copy of the filter drifts from the other's.
///
/// Returns `(resource, unsupported)`: `unsupported` is `true` only when the XObject was a
/// recognized image that couldn't be materialized (raw/undecoded) — not for a non-image or a
/// below-`min_image_dimension` drop, neither of which is a format-support gap worth a quality
/// warning.
fn convert_resource_xobject(
    xobj: RawXObject,
    min_image_dimension: u32,
) -> (Option<Resource>, bool) {
    let resource = match convert_xobject_pub(xobj) {
        Some(r) => r,
        None => return (None, false),
    };
    if !resource.is_image() {
        return (None, false);
    }
    let ext = resource.extension();
    if ext == "raw" || ext == "bin" {
        return (None, true);
    }
    // Decorative-image cutoff applies only when both dimensions are known — measured is
    // conservative, unmeasured is kept as-is.
    if min_image_dimension > 0 {
        if let (Some(w), Some(h)) = (resource.width, resource.height) {
            if w < min_image_dimension || h < min_image_dimension {
                return (None, false);
            }
        }
    }
    (Some(resource), false)
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

/// Build the `Paragraph` for a `BlockType::ListItem` block: the marker-stripped
/// text, carrying an ordered or unordered `ListInfo` per the block's detected
/// marker. Nesting level is always 0 — `layout::detect_list_marker` reads a
/// single line's text and has no indentation model to derive one from.
fn list_item_paragraph(block: &super::layout::TextBlock) -> Paragraph {
    let mut para = Paragraph::with_text(block.list_item_text());
    para.style.list_info = Some(match block.list_item_number {
        Some(n) => ListInfo::numbered(0, n),
        None => ListInfo::bullet(0),
    });
    para
}

/// Merge consecutive paragraph blocks that share the same visual row
/// (Y within 1.5pt of each other) into a single paragraph. Recovers
/// table-row structure that XY-Cut over-segmented into per-cell blocks.
/// Headings, tables, images, and rule blocks are never merged.
fn merge_same_row_paragraphs(elements: Vec<(f32, Block)>) -> Vec<(f32, Block)> {
    // Tolerance ≈ half of body line height. Table cells in Hancom PDFs
    // frequently sit on slightly offset baselines within the same visual row
    // (header centred vs. body top-aligned). 6pt catches most real rows
    // without merging across line breaks.
    const ROW_Y_TOLERANCE: f32 = 6.0;
    let mut out: Vec<(f32, Block)> = Vec::with_capacity(elements.len());
    for (y, block) in elements {
        let Block::Paragraph(p) = &block else {
            out.push((y, block));
            continue;
        };
        if p.style.heading_level.is_some() || p.style.list_info.is_some() {
            out.push((y, block));
            continue;
        }
        // Can we merge into the previous?
        if let Some((prev_y, Block::Paragraph(prev_p))) = out.last_mut().map(|(y, b)| (y, b)) {
            if (*prev_y - y).abs() <= ROW_Y_TOLERANCE
                && prev_p.style.heading_level.is_none()
                && prev_p.style.list_info.is_none()
            {
                let prev_text = prev_p.plain_text();
                let cur_text = p.plain_text();
                let needs_gap = !prev_text.ends_with(char::is_whitespace)
                    && !cur_text.starts_with(char::is_whitespace);
                let mut combined = prev_text;
                if needs_gap {
                    combined.push(' ');
                }
                combined.push_str(&cur_text);
                *prev_p = Paragraph::with_text(combined);
                continue;
            }
        }
        out.push((y, block));
    }
    out
}

/// Render a table row detected with low confidence as plain paragraph text.
///
/// A row misdetected as a low-confidence table can still be a TOC line (title/
/// leader/page-number spans look table-row-ish to the detector) — dot leaders
/// are normalized the same way the `TextLine` path does, so this fallback
/// doesn't leak raw dot runs into paragraph text.
fn low_confidence_row_text(row: &super::table_detector::TableRowData) -> String {
    super::layout::normalize_dot_leaders(row.spans.clone())
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("  ")
}

fn extract_page_with_tables_fn(
    analyzer: &mut super::layout::LayoutAnalyzer,
    page_num: u32,
) -> Result<Vec<Block>> {
    let (mut spans, lattice_grids) = analyzer.extract_page_spans_and_lattice_grids(page_num)?;

    // Apply header/footer filter before table detection so page numbers
    // in margins don't end up as spurious table rows or body paragraphs.
    analyzer.filter_spans_for_page(&mut spans, page_num);

    if spans.is_empty() {
        return Ok(vec![]);
    }

    // Lattice mode first: explicit ruling lines are direct structural
    // evidence, so a confirmed grid is accepted outright — it doesn't need
    // stream mode's alignment/occupancy heuristics, which exist only to
    // *guess* structure from text position when no such evidence exists.
    // Spans a lattice table consumes are removed before stream-mode
    // detection runs, so the same content isn't extracted twice.
    let mut lattice_tables: Vec<(f32, crate::model::Table)> = Vec::new();
    let mut lattice_consumed = std::collections::HashSet::new();
    for grid in &lattice_grids {
        if let Some((table, consumed)) = super::lattice::build_table(grid, &spans) {
            lattice_tables.push((grid.top_y, table));
            lattice_consumed.extend(consumed);
        }
    }
    let spans: Vec<super::layout::TextSpan> = if lattice_consumed.is_empty() {
        spans
    } else {
        spans
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !lattice_consumed.contains(i))
            .map(|(_, s)| s)
            .collect()
    };

    let table_detector = super::table_detector::TableDetector::new();
    let (detected_tables, remaining_spans) = table_detector.detect(spans.clone());

    let mut blocks: Vec<Block> = Vec::new();

    if !lattice_tables.is_empty() || !detected_tables.is_empty() {
        log::debug!(
            "Detected {} lattice + {} stream tables on page {}",
            lattice_tables.len(),
            detected_tables.len(),
            page_num
        );

        let mut elements: Vec<(f32, Block)> = Vec::new();

        for (top_y, table) in lattice_tables {
            elements.push((top_y, Block::Table(table)));
        }

        const TABLE_CONFIDENCE_THRESHOLD: f32 = 0.4;
        for detected in &detected_tables {
            if detected.confidence < TABLE_CONFIDENCE_THRESHOLD {
                log::debug!(
                    "Table at y={} has low confidence ({:.2}), converting to paragraphs",
                    detected.top_y,
                    detected.confidence
                );
                for row in &detected.rows {
                    let text = low_confidence_row_text(row);
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
            let a = &mut *analyzer;
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
                            Block::Paragraph(list_item_paragraph(&block))
                        }
                    };
                    elements.push((y_pos, para_block));
                }
            }
        }

        elements.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let merged = merge_same_row_paragraphs(elements);
        blocks = merged.into_iter().map(|(_, block)| block).collect();
    } else {
        let text_blocks = analyzer.extract_page_blocks(page_num)?;
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
                        Block::Paragraph(list_item_paragraph(&block))
                    }
                };
                blocks.push(para_block);
            }
        }
    }

    Ok(blocks)
}

fn fallback_text_extraction_fn(
    analyzer: &super::layout::LayoutAnalyzer,
    page: &mut Page,
    page_num: u32,
    options: &ParseOptions,
) -> Result<()> {
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

    use crate::parser::layout::TextSpan;
    use crate::parser::table_detector::TableRowData;

    fn span_at(text: &str, x: f32, width: f32) -> TextSpan {
        TextSpan {
            width,
            ..TextSpan::new(text.to_string(), x, 500.0, 12.0, "Helvetica".into())
        }
    }

    #[test]
    fn test_low_confidence_row_text_normalizes_toc_dot_leader() {
        // A TOC line ("Chapter 1 .......... 6") is exactly the multi-span shape
        // that can make a table detector misclassify it as a low-confidence
        // table row — the fallback join must still normalize its dot leader.
        let row = TableRowData {
            y: 500.0,
            spans: vec![
                span_at("Chapter 1", 100.0, 54.0),
                span_at("....................", 160.0, 120.0),
                span_at("6", 290.0, 6.0),
            ],
        };

        assert_eq!(low_confidence_row_text(&row), "Chapter 1  (p.6)");
    }

    #[test]
    fn test_low_confidence_row_text_leaves_ordinary_rows_untouched() {
        let row = TableRowData {
            y: 500.0,
            spans: vec![span_at("Name", 100.0, 30.0), span_at("Value", 200.0, 30.0)],
        };

        assert_eq!(low_confidence_row_text(&row), "Name  Value");
    }
}
