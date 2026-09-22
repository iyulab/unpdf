//! PDF document parser.

use std::io::Read;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use crate::detect::detect_format_from_path;
use crate::error::{Error, Result};
use crate::model::{
    Block, Document, InlineContent, ListInfo, OutlineItem, Page, Paragraph, Resource, ResourceType,
    TextRun, TextStyle,
};

use super::backend::{PdfBackend, RawBackend, RawXObject};
use super::options::{ErrorMode, ParseOptions};

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

        // Decryption is attempted inside RawDocument::load_with_password(): the empty
        // password first, then `options.password` if one was given. If we get here, the PDF
        // is usable (either not encrypted, or decrypted).
        let backend: Box<dyn PdfBackend> = Box::new(RawBackend::load_file_with_password(
            path,
            options.password.as_deref(),
        )?);

        Ok(Self { backend, options })
    }

    /// Parse a PDF from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::from_bytes_with_options(data, ParseOptions::default())
    }

    /// Parse a PDF from bytes with custom options.
    pub fn from_bytes_with_options(data: &[u8], options: ParseOptions) -> Result<Self> {
        let backend: Box<dyn PdfBackend> = Box::new(RawBackend::load_bytes_with_password(
            data,
            options.password.as_deref(),
        )?);
        Ok(Self { backend, options })
    }

    /// Parse a PDF from a reader.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self> {
        Self::from_reader_with_options(reader, ParseOptions::default())
    }

    /// Parse a PDF from a reader with custom options.
    pub fn from_reader_with_options<R: Read>(reader: R, options: ParseOptions) -> Result<Self> {
        let backend: Box<dyn PdfBackend> = Box::new(RawBackend::load_reader_with_password(
            reader,
            options.password.as_deref(),
        )?);
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
                if self.options.extract_resources {
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

        // Before anything else looks at the resources -- the AI pass below most of all, since
        // it bills per image -- collapse entries whose bytes are identical. A shared logo would
        // otherwise be captioned once per page it appears on.
        if self.options.extract_resources {
            super::dedup::collapse_identical_resources(&mut document);
        }

        #[cfg(feature = "ai")]
        if let Some(ai_config) = &self.options.ai {
            crate::ai_wiring::apply(&mut document, ai_config);
            // `effective_extract_resources()` may have forced image-byte decoding
            // purely so the AI call above had bytes to send. If the caller never
            // asked for the resource inventory itself, drop it now — the IR changes
            // AI made (new Paragraph/Table blocks, filled alt_text) stay regardless,
            // only the raw byte inventory is scoped to `extract_resources`.
            if !self.options.extract_resources {
                for page in &mut document.pages {
                    page.images.clear();
                }
            }
        }

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

    if options.extract_text {
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

        // A content stream that could not be decoded is content this page lost. Lenient
        // keeps what the other streams hold; strict fails the page, exactly as it does
        // when the page's only content stream cannot be decoded.
        let undecodable = analyzer.undecodable_content_streams();
        page.undecodable_content_streams = undecodable;
        if undecodable > 0 {
            if options.error_mode == ErrorMode::Strict {
                return Err(Error::PdfParse(format!(
                    "page {page_num}: {undecodable} content stream(s) could not be decoded"
                )));
            }
            log::warn!("page {page_num}: left out {undecodable} undecodable content stream(s)");
        }
    }

    // 이미지(XObject) 수집 — extract_resources 가 활성화된 경우.
    // 현재는 정확한 Y 좌표 해석이 안 되어 페이지 말미에 순서대로 append.
    // 향후 Phase 2 에서 content-stream 의 Do 연산자 위치 해석으로 interleave 예정.
    // id 는 확장자 포함: `page{N}_{name}.{ext}`. 이 id 를 곧 이미지의
    // 파일명으로도 사용하므로 writer 측에서 별도 suggested_filename 호출 불필요.
    if options.extract_resources {
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
/// existing raw/undecoded-drop path. `Indexed` and `CMYK` colour spaces are a
/// deliberate follow-up rather than an oversight — see [`super::png_encode`].
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
    let mut para = styled_paragraph(block);
    strip_prefix_bytes(&mut para.content, block_list_marker_len(block));
    para.style.list_info = Some(match block.list_item_number {
        Some(n) => ListInfo::numbered(0, n),
        None => ListInfo::bullet(0),
    });
    para
}

/// Byte length of the list marker prefix at the start of a `ListItem` block's
/// plain text. Mirrors `TextBlock::list_item_text`, which is what the marker
/// length is defined against; recomputed here so the styled-run builder can
/// strip the same prefix without re-deriving the block's joined text twice.
fn block_list_marker_len(block: &super::layout::TextBlock) -> usize {
    let full = block.text();
    let stripped = block.list_item_text();
    full.len() - stripped.len()
}

/// Build a `Paragraph` whose inline content preserves per-span bold/italic
/// styling, joining spans and lines with the same spacing the plain-text
/// `TextBlock::text` path would produce. Adjacent runs sharing a style are
/// merged so emphasis markers don't fragment. Falls back to plain text for
/// blocks containing RTL characters: BiDi reordering works on the joined
/// string and can't be represented as independently-styled runs.
fn styled_paragraph(block: &super::layout::TextBlock) -> Paragraph {
    let plain = block.text();
    if super::bidi::contains_rtl(&plain) {
        return Paragraph::with_text(plain);
    }

    let mut runs: Vec<TextRun> = Vec::new();
    for (line_idx, line) in block.lines.iter().enumerate() {
        if line_idx > 0 {
            push_run(&mut runs, " ", false, false);
        }
        for (piece, span_idx) in line.styled_segments() {
            let span = &line.spans[span_idx];
            push_run(&mut runs, &piece, span.is_bold, span.is_italic);
        }
    }

    let mut para = Paragraph::new();
    para.content = runs.into_iter().map(InlineContent::Text).collect();
    para
}

/// Append `text` as a run with the given emphasis, merging into the previous
/// run when it carries the same style so consecutive same-style spans don't
/// produce back-to-back `**a****b**` fragments.
fn push_run(runs: &mut Vec<TextRun>, text: &str, bold: bool, italic: bool) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut() {
        if last.style.bold == bold && last.style.italic == italic {
            last.text.push_str(text);
            return;
        }
    }
    runs.push(TextRun {
        text: text.to_string(),
        style: TextStyle {
            bold,
            italic,
            ..Default::default()
        },
    });
}

/// Remove `n` bytes from the front of a run sequence (the list marker prefix),
/// then trim any whitespace left at the new start. Byte offsets are safe here
/// because the prefix being removed is the marker plus ASCII whitespace, and
/// the runs' concatenated text equals the block's plain text the length was
/// measured against.
fn strip_prefix_bytes(content: &mut Vec<InlineContent>, mut n: usize) {
    while n > 0 && !content.is_empty() {
        let remove_whole = match &content[0] {
            InlineContent::Text(run) => run.text.len() <= n,
            _ => false,
        };
        if remove_whole {
            if let InlineContent::Text(run) = content.remove(0) {
                n -= run.text.len();
            }
        } else if let InlineContent::Text(run) = &mut content[0] {
            run.text = run.text[n..].to_string();
            n = 0;
        } else {
            break;
        }
    }
    if let Some(InlineContent::Text(run)) = content.first_mut() {
        let trimmed = run.text.trim_start().to_string();
        run.text = trimmed;
    }
}

/// Merge consecutive paragraph blocks that share the same visual row
/// (Y within 1.5pt of each other) into a single paragraph. Recovers
/// table-row structure that XY-Cut over-segmented into per-cell blocks.
/// Headings, tables, images, and rule blocks are never merged.
///
/// The merge appends inline content rather than re-joining plain text so any
/// per-span bold/italic styling survives (a bold row label stays bold next to
/// its regular-weight value).
fn merge_same_row_paragraphs(elements: Vec<(f32, Block)>) -> Vec<(f32, Block)> {
    // Tolerance ≈ half of body line height. Table cells in Hancom PDFs
    // frequently sit on slightly offset baselines within the same visual row
    // (header centred vs. body top-aligned). 6pt catches most real rows
    // without merging across line breaks.
    const ROW_Y_TOLERANCE: f32 = 6.0;
    let mut out: Vec<(f32, Block)> = Vec::with_capacity(elements.len());
    for (y, block) in elements {
        // Only plain paragraphs (not headings or list items) are merge candidates.
        let para = match block {
            Block::Paragraph(p)
                if p.style.heading_level.is_none() && p.style.list_info.is_none() =>
            {
                p
            }
            other => {
                out.push((y, other));
                continue;
            }
        };

        if let Some((prev_y, Block::Paragraph(prev_p))) = out.last_mut().map(|(y, b)| (y, b)) {
            if prev_p.style.heading_level.is_none()
                && prev_p.style.list_info.is_none()
                && (*prev_y - y).abs() <= ROW_Y_TOLERANCE
            {
                let needs_gap = {
                    let prev_text = prev_p.plain_text();
                    let cur_text = para.plain_text();
                    !prev_text.ends_with(char::is_whitespace)
                        && !cur_text.starts_with(char::is_whitespace)
                };
                if needs_gap {
                    prev_p.content.push(InlineContent::Text(TextRun {
                        text: " ".to_string(),
                        style: TextStyle::default(),
                    }));
                }
                prev_p.content.extend(para.content);
                continue;
            }
        }

        out.push((y, Block::Paragraph(para)));
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
                            Block::Paragraph(styled_paragraph(&block))
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
                        Block::Paragraph(styled_paragraph(&block))
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
    // Assembled in the test rather than read from disk -- see that module's docs.
    use crate::parser::test_pdf::{pdf, stream};
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
    /// One shared image XObject must cost one resource entry, whatever the page count.
    ///
    /// The key built a few hundred lines above is `page{n}_{name}`, a key space with no way to
    /// say "the same image", so extraction used to emit one entry per page: a logo in a 40-page
    /// document became 40 entries, 40 copies in `page.images`, and 40 VLM calls. The counts are
    /// checked across several page counts because the old behaviour was exactly linear in them,
    /// and a fix that only worked for small documents would still pass a single-size check.
    ///
    /// The fixture is built here rather than in `tests/common/` because what is being measured
    /// is this file's keying decision, and because a `#[cfg(test)]` unit test rides the library
    /// test binary -- which matters on a machine whose application-control policy blocks freshly
    /// built integration-test executables.
    fn shared_logo_pdf(pages: usize) -> Vec<u8> {
        let first_page_obj = 4usize;
        let font_obj = first_page_obj + pages * 2;
        let kids: Vec<String> = (0..pages)
            .map(|i| format!("{} 0 R", first_page_obj + i * 2))
            .collect();

        let mut objects: Vec<Vec<u8>> = vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            format!("<</Type/Pages/Kids[{}]/Count {}>>", kids.join(" "), pages).into_bytes(),
            // Object 3: the single image every page's resource dictionary points at. A
            // `/DCTDecode` stub, because an unfiltered sample has no recognisable format and
            // `convert_resource_xobject` drops it as unsupported before any of this matters.
            stream(
                "<</Type/XObject/Subtype/Image/Width 100/Height 100/ColorSpace/DeviceGray/BitsPerComponent 8/Filter/DCTDecode/Length 10>>",
                &[0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0xFF, 0xD9],
            ),
        ];

        for i in 0..pages {
            let content_obj = first_page_obj + i * 2 + 1;
            objects.push(
                format!(
                    "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Resources<</XObject<</Logo 3 0 R>>/Font<</F1 {font_obj} 0 R>>>>/Contents {content_obj} 0 R>>"
                )
                .into_bytes(),
            );
            let content = format!(
                "q 100 0 0 40 20 780 cm /Logo Do Q\nBT /F1 12 Tf 72 700 Td (Page {}) Tj ET\n",
                i + 1
            );
            objects.push(stream(
                &format!("<</Length {}>>", content.len()),
                content.as_bytes(),
            ));
        }
        objects.push(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec());

        pdf(objects, 1)
    }

    #[test]
    fn one_shared_image_becomes_one_resource_entry_per_page() {
        for pages in [1usize, 2, 8, 40] {
            let pdf = shared_logo_pdf(pages);
            let options = ParseOptions {
                extract_resources: true,
                ..Default::default()
            };
            let doc = PdfParser::from_bytes_with_options(&pdf, options)
                .expect("the fixture parses")
                .parse()
                .expect("the fixture parses");

            let images: Vec<_> = doc
                .resources
                .values()
                .filter(|r| !r.data.is_empty())
                .collect();
            let distinct: std::collections::HashSet<&[u8]> =
                images.iter().map(|r| r.data.as_slice()).collect();

            assert_eq!(
                distinct.len(),
                1,
                "pages={pages}: the document holds exactly one image, whatever extraction did"
            );
            assert_eq!(
                images.len(),
                1,
                "pages={pages}: one image in the file is one entry out, however many pages drew it"
            );
        }
    }
    #[test]
    fn the_surviving_reference_is_the_first_occurrence_in_reading_order() {
        // Which occurrence keeps the id has to be a property of the document, not of how many
        // threads parsed it -- `parse_single_page` runs on a parallel path in `stream.rs`.
        let pdf = shared_logo_pdf(6);
        let options = ParseOptions {
            extract_resources: true,
            ..Default::default()
        };

        let ids: Vec<String> = (0..3)
            .map(|_| {
                let doc = PdfParser::from_bytes_with_options(&pdf, options.clone())
                    .expect("the fixture parses")
                    .parse()
                    .expect("the fixture parses");
                doc.pages
                    .iter()
                    .flat_map(|p| p.elements.iter())
                    .find_map(|b| match b {
                        Block::Image { resource_id, .. } => Some(resource_id.clone()),
                        _ => None,
                    })
                    .expect("the logo is drawn on page 1")
            })
            .collect();

        assert!(
            ids[0].starts_with("page1_"),
            "the first page's occurrence should win, got {}",
            ids[0]
        );
        assert!(
            ids.iter().all(|id| *id == ids[0]),
            "the winner must not depend on the run: {ids:?}"
        );
    }

    #[test]
    fn every_page_still_reports_its_image_and_points_at_a_resource_that_exists() {
        // Collapsing the inventory must not cost a page its picture: each page keeps its own
        // Image block, and the id that block carries has to resolve.
        let pages = 5;
        let pdf = shared_logo_pdf(pages);
        let options = ParseOptions {
            extract_resources: true,
            ..Default::default()
        };
        let doc = PdfParser::from_bytes_with_options(&pdf, options)
            .expect("the fixture parses")
            .parse()
            .expect("the fixture parses");

        let known: std::collections::HashSet<&str> = doc
            .pages
            .iter()
            .flat_map(|p| p.images.iter())
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(known.len(), 1, "one picture, one surviving entry");

        for page in &doc.pages {
            let blocks: Vec<&str> = page
                .elements
                .iter()
                .filter_map(|b| match b {
                    Block::Image { resource_id, .. } => Some(resource_id.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                blocks.len(),
                1,
                "page {} lost its image block to deduplication",
                page.number
            );
            assert!(
                known.contains(blocks[0]),
                "page {} points at {}, which no longer exists",
                page.number,
                blocks[0]
            );
        }
    }

    #[test]
    fn images_that_merely_look_alike_are_kept_apart() {
        // The whole pass turns on byte equality. A fixture whose pages carry *different*
        // pictures must come through untouched, or deduplication is silently losing content.
        let pdf = two_distinct_images_pdf();
        let options = ParseOptions {
            extract_resources: true,
            ..Default::default()
        };
        let doc = PdfParser::from_bytes_with_options(&pdf, options)
            .expect("the fixture parses")
            .parse()
            .expect("the fixture parses");

        let images: Vec<_> = doc
            .resources
            .values()
            .filter(|r| !r.data.is_empty())
            .collect();
        assert_eq!(images.len(), 2, "two different pictures stay two resources");
    }
    /// Two pages, each drawing a *different* image -- the control for the deduplication pass.
    fn two_distinct_images_pdf() -> Vec<u8> {
        let jpeg = |tail: u8| -> Vec<u8> {
            stream(
                "<</Type/XObject/Subtype/Image/Width 100/Height 100/ColorSpace/DeviceGray/BitsPerComponent 8/Filter/DCTDecode/Length 10>>",
                &[0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, tail, 0xFF, 0xD9],
            )
        };

        let objects: Vec<Vec<u8>> = vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[5 0 R 7 0 R]/Count 2>>".to_vec(),
            jpeg(0x46),
            jpeg(0x47),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Resources<</XObject<</Pic 3 0 R>>>>/Contents 6 0 R>>".to_vec(),
            stream("<</Length 34>>", b"q 100 0 0 40 20 780 cm /Pic Do Q\n\n"),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Resources<</XObject<</Pic 4 0 R>>>>/Contents 8 0 R>>".to_vec(),
            stream("<</Length 34>>", b"q 100 0 0 40 20 780 cm /Pic Do Q\n\n"),
        ];

        pdf(objects, 1)
    }
}
