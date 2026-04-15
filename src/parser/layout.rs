//! Layout analysis for PDF documents.
//!
//! This module provides text extraction with position and font information,
//! enabling proper heading detection, paragraph separation, and structure analysis.

use std::collections::HashMap;

use super::backend::{get_number_from_value, PdfBackend, PdfValue};
use crate::error::{Error, Result};

/// A text span with position and style information.
#[derive(Debug, Clone)]
pub struct TextSpan {
    /// The text content
    pub text: String,
    /// X position (left edge)
    pub x: f32,
    /// Y position (baseline)
    pub y: f32,
    /// Width of the text
    pub width: f32,
    /// Font size in points
    pub font_size: f32,
    /// Font name (e.g., "Helvetica-Bold")
    pub font_name: String,
    /// Whether the font appears to be bold
    pub is_bold: bool,
    /// Whether the font appears to be italic
    pub is_italic: bool,
}

impl TextSpan {
    /// Create a new text span.
    pub fn new(text: String, x: f32, y: f32, font_size: f32, font_name: String) -> Self {
        let is_bold = font_name.to_lowercase().contains("bold")
            || font_name.to_lowercase().contains("black")
            || font_name.to_lowercase().contains("heavy");
        let is_italic = font_name.to_lowercase().contains("italic")
            || font_name.to_lowercase().contains("oblique");

        Self {
            text,
            x,
            y,
            width: 0.0, // Will be calculated later if needed
            font_size,
            font_name,
            is_bold,
            is_italic,
        }
    }

    /// Get the bottom Y coordinate (approximate, based on font size).
    pub fn bottom(&self) -> f32 {
        self.y - self.font_size * 0.2 // Approximate descender
    }

    /// Get the top Y coordinate (approximate, based on font size).
    pub fn top(&self) -> f32 {
        self.y + self.font_size * 0.8 // Approximate ascender
    }
}

/// A text line composed of multiple spans on the same baseline.
#[derive(Debug, Clone)]
pub struct TextLine {
    /// The spans in this line, sorted by X position
    pub spans: Vec<TextSpan>,
    /// Y position (baseline)
    pub y: f32,
    /// Leftmost X position
    pub x: f32,
    /// Dominant font size in this line
    pub font_size: f32,
    /// Whether this line appears to be a heading
    pub is_heading: bool,
    /// Detected heading level (1-6, or 0 for non-heading)
    pub heading_level: u8,
}

impl TextLine {
    /// Create a new text line from spans.
    pub fn from_spans(mut spans: Vec<TextSpan>) -> Self {
        if spans.is_empty() {
            return Self {
                spans: vec![],
                y: 0.0,
                x: 0.0,
                font_size: 0.0,
                is_heading: false,
                heading_level: 0,
            };
        }

        // Sort spans by X position
        spans.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

        // Calculate dominant font size (weighted by text length)
        let total_chars: usize = spans.iter().map(|s| s.text.len()).sum();
        let weighted_size: f32 = spans
            .iter()
            .map(|s| s.font_size * s.text.len() as f32)
            .sum();
        let font_size = if total_chars > 0 {
            weighted_size / total_chars as f32
        } else {
            spans[0].font_size
        };

        let y = spans[0].y;
        let x = spans[0].x;

        Self {
            spans,
            y,
            x,
            font_size,
            is_heading: false,
            heading_level: 0,
        }
    }

    /// Get the combined text of all spans with appropriate spacing.
    ///
    /// Inserts spaces between spans based on their X coordinate gaps.
    /// For CJK characters, no space is inserted between adjacent characters.
    pub fn text(&self) -> String {
        if self.spans.is_empty() {
            return String::new();
        }

        if self.spans.len() == 1 {
            return self.spans[0].text.clone();
        }

        let mut result = String::new();

        for (i, span) in self.spans.iter().enumerate() {
            if i == 0 {
                result.push_str(&span.text);
                continue;
            }

            let prev_span = &self.spans[i - 1];

            // Calculate gap between end of previous span and start of current span
            let prev_end = prev_span.x + prev_span.width;
            let gap = span.x - prev_end;

            // Estimate average character width from current span
            let char_count = span.text.chars().count();
            let avg_char_width = if char_count > 0 && span.width > 0.0 {
                span.width / char_count as f32
            } else {
                span.font_size * 0.5 // Fallback: assume half of font size
            };

            // Check if we need to insert a space
            // Gap threshold: if gap is more than 20% of average char width, insert space
            let space_threshold = avg_char_width * 0.2;

            // Get last char of previous span and first char of current span
            let prev_last_char = prev_span.text.chars().last();
            let curr_first_char = span.text.chars().next();

            let should_insert_space = if gap > space_threshold {
                // Check if both characters are CJK (no space needed between CJK chars)
                let prev_is_cjk = prev_last_char
                    .map(is_spaceless_script_char)
                    .unwrap_or(false);
                let curr_is_cjk = curr_first_char
                    .map(is_spaceless_script_char)
                    .unwrap_or(false);

                // Don't insert space between CJK characters
                !(prev_is_cjk && curr_is_cjk)
            } else {
                false
            };

            // Also check if previous span ends with space or current starts with space
            let prev_ends_with_space =
                prev_span.text.ends_with(' ') || prev_span.text.ends_with('\u{00A0}');
            let curr_starts_with_space =
                span.text.starts_with(' ') || span.text.starts_with('\u{00A0}');

            if should_insert_space && !prev_ends_with_space && !curr_starts_with_space {
                result.push(' ');
            }

            result.push_str(&span.text);
        }

        // Apply BiDi reordering for RTL scripts (Arabic, Hebrew, etc.)
        if super::bidi::contains_rtl(&result) {
            result = super::bidi::reorder_bidi(&result);
        }

        result
    }

    /// Check if the line is predominantly bold.
    pub fn is_bold(&self) -> bool {
        let bold_chars: usize = self
            .spans
            .iter()
            .filter(|s| s.is_bold)
            .map(|s| s.text.len())
            .sum();
        let total_chars: usize = self.spans.iter().map(|s| s.text.len()).sum();
        total_chars > 0 && bold_chars as f32 / total_chars as f32 > 0.5
    }

    /// Check if the line appears to be uppercase.
    pub fn is_uppercase(&self) -> bool {
        let text = self.text();
        let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
        !letters.is_empty() && letters.iter().all(|c| c.is_uppercase())
    }
}

/// A text block (paragraph, heading, etc.).
#[derive(Debug, Clone)]
pub struct TextBlock {
    /// The lines in this block
    pub lines: Vec<TextLine>,
    /// Block type
    pub block_type: BlockType,
    /// Heading level (1-6 for headings, 0 otherwise)
    pub heading_level: u8,
}

/// A detected column in the page layout.
#[derive(Debug, Clone)]
pub struct Column {
    /// Left boundary X coordinate
    pub left: f32,
    /// Right boundary X coordinate
    pub right: f32,
    /// Column index (0 = leftmost)
    pub index: usize,
}

impl Column {
    /// Check if an X coordinate falls within this column.
    pub fn contains(&self, x: f32) -> bool {
        x >= self.left && x <= self.right
    }

    /// Check if a span belongs to this column.
    pub fn contains_span(&self, span: &TextSpan) -> bool {
        // A span belongs to a column if its left edge is within the column
        // or if its center point is within the column
        let center = span.x + span.width / 2.0;
        self.contains(span.x) || self.contains(center)
    }
}

/// Type of text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// A heading (H1-H6)
    Heading,
    /// A regular paragraph
    Paragraph,
    /// A list item
    ListItem,
    /// Unknown or unclassified
    Unknown,
}

impl TextBlock {
    /// Create a new text block.
    pub fn new(lines: Vec<TextLine>, block_type: BlockType) -> Self {
        Self {
            lines,
            block_type,
            heading_level: 0,
        }
    }

    /// Get the combined text of all lines.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Check if the block is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() || self.text().trim().is_empty()
    }
}

/// Layout analyzer for extracting structured text from PDF pages.
pub struct LayoutAnalyzer<'a> {
    backend: &'a dyn PdfBackend,
    /// Font size statistics for the document
    font_stats: FontStatistics,
}

/// Font statistics for heading detection.
#[derive(Debug, Clone, Default)]
pub struct FontStatistics {
    /// Body text font size (most common)
    pub body_size: f32,
    /// Font sizes larger than body (potential headings)
    pub heading_sizes: Vec<f32>,
    /// All observed font sizes with frequency
    pub size_histogram: HashMap<i32, usize>,
}

impl FontStatistics {
    /// Add a font size observation.
    pub fn add_size(&mut self, size: f32) {
        let key = (size * 10.0) as i32; // Round to 0.1 precision
        *self.size_histogram.entry(key).or_insert(0) += 1;
    }

    /// Calculate body size and heading sizes.
    pub fn analyze(&mut self) {
        if self.size_histogram.is_empty() {
            self.body_size = 12.0;
            return;
        }

        // Find the most common font size (body text)
        let (body_key, _) = self
            .size_histogram
            .iter()
            .max_by_key(|(_, count)| *count)
            .unwrap();
        self.body_size = *body_key as f32 / 10.0;

        // Find sizes larger than body (potential headings)
        let mut larger_sizes: Vec<f32> = self
            .size_histogram
            .keys()
            .filter(|k| **k as f32 / 10.0 > self.body_size + 0.5)
            .map(|k| *k as f32 / 10.0)
            .collect();
        larger_sizes.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        self.heading_sizes = larger_sizes;
    }

    /// Get heading level for a font size (1-6, or 0 for body text).
    ///
    /// Conservative: requires font size ≥ body + 2.5 to qualify, or body + 1.5
    /// combined with bold. Caps at level 4 to avoid `#####` spam.
    pub fn get_heading_level(&self, font_size: f32, is_bold: bool) -> u8 {
        let strong_threshold = self.body_size + 2.5;
        let bold_threshold = self.body_size + 1.5;

        let qualifies = font_size >= strong_threshold
            || (is_bold && font_size >= bold_threshold);
        if !qualifies {
            return 0;
        }

        // Rank within distinct heading-size tiers. We dedupe `heading_sizes`
        // on the fly so that sizes clustered within 2pt count as one tier —
        // Hancom docs have many discrete sizes that would otherwise produce
        // erratic H-level assignment.
        let mut tier = 0u8;
        let mut last_tier_size: Option<f32> = None;
        for &heading_size in &self.heading_sizes {
            let is_new_tier = match last_tier_size {
                None => true,
                Some(prev) => (prev - heading_size).abs() >= 2.0,
            };
            if is_new_tier {
                tier = tier.saturating_add(1);
                last_tier_size = Some(heading_size);
            }
            if font_size >= heading_size - 0.5 {
                return tier.min(4);
            }
        }
        4
    }
}

impl<'a> LayoutAnalyzer<'a> {
    /// Create a new layout analyzer.
    pub fn new(backend: &'a dyn PdfBackend) -> Self {
        Self {
            backend,
            font_stats: FontStatistics::default(),
        }
    }

    /// Get mutable reference to font statistics (for external use).
    pub fn font_stats_mut(&mut self) -> &mut FontStatistics {
        &mut self.font_stats
    }

    /// Public wrapper for group_spans_into_lines.
    pub fn group_spans_into_lines_pub(&self, spans: Vec<TextSpan>) -> Vec<TextLine> {
        self.group_spans_into_lines(spans)
    }

    /// Public wrapper for detect_headings.
    pub fn detect_headings_pub(&self, lines: Vec<TextLine>) -> Vec<TextLine> {
        self.detect_headings(lines)
    }

    /// Public wrapper for group_lines_into_blocks.
    pub fn group_lines_into_blocks_pub(&self, lines: Vec<TextLine>) -> Vec<TextBlock> {
        self.group_lines_into_blocks(lines)
    }

    /// Extract text spans from a page with position and font information.
    pub fn extract_page_spans(&self, page_num: u32) -> Result<Vec<TextSpan>> {
        let pages = self.backend.pages();
        let page_id = pages
            .get(&page_num)
            .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;

        // Build font info map
        let backend_fonts = self.backend.page_fonts(*page_id)?;
        let mut fonts = HashMap::new();
        for fi in &backend_fonts {
            fonts.insert(
                fi.name.clone(),
                FontInfo {
                    name: fi.base_font.clone(),
                },
            );
        }

        let content = self.backend.page_content(*page_id)?;
        self.parse_operations(&content, &fonts, *page_id)
    }

    /// Extract structured text blocks from a page.
    pub fn extract_page_blocks(&mut self, page_num: u32) -> Result<Vec<TextBlock>> {
        // Get page dimensions for header/footer filtering
        let pages = self.backend.pages();
        let page_id = pages
            .get(&page_num)
            .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;
        let (_page_width, page_height) = self.backend.page_dimensions(*page_id);

        let mut spans = self.extract_page_spans(page_num)?;

        // Filter out page numbers / running headers from top/bottom margins
        filter_header_footer_spans(&mut spans, page_height);

        // Update font statistics
        for span in &spans {
            self.font_stats.add_size(span.font_size);
        }
        self.font_stats.analyze();

        // Group spans into lines
        let lines = self.group_spans_into_lines(spans);

        // Detect headings
        let lines = self.detect_headings(lines);

        // Group lines into blocks (paragraphs)
        let blocks = self.group_lines_into_blocks(lines);

        Ok(blocks)
    }

    /// Parse content stream operations into text spans.
    ///
    /// Delegates content decoding and text decoding to the backend,
    /// keeping layout.rs free from concrete PDF library types.
    fn parse_operations(
        &self,
        content: &[u8],
        fonts: &HashMap<Vec<u8>, FontInfo>,
        page_id: super::backend::PageId,
    ) -> Result<Vec<TextSpan>> {
        let operations = self.backend.decode_content(content)?;

        let mut spans = Vec::new();
        let mut current_font = String::new();
        let mut current_font_name: Vec<u8> = Vec::new();
        let mut current_font_size: f32 = 12.0;
        let mut text_matrix = TextMatrix::default();
        let mut in_text_block = false;

        for op in &operations {
            match op.operator.as_str() {
                "BT" => {
                    in_text_block = true;
                    text_matrix = TextMatrix::default();
                }
                "ET" => {
                    in_text_block = false;
                }
                "Tf" => {
                    if op.operands.len() >= 2 {
                        if let PdfValue::Name(font_name) = &op.operands[0] {
                            current_font_name = font_name.clone();
                            if let Some(info) = fonts.get(font_name.as_slice()) {
                                current_font = info.name.clone();
                            } else {
                                current_font =
                                    String::from_utf8_lossy(font_name.as_slice()).to_string();
                            }
                        }
                        current_font_size = get_number_from_value(&op.operands[1]).unwrap_or(12.0);
                    }
                }
                "Td" | "TD" => {
                    if op.operands.len() >= 2 {
                        let tx = get_number_from_value(&op.operands[0]).unwrap_or(0.0);
                        let ty = get_number_from_value(&op.operands[1]).unwrap_or(0.0);
                        text_matrix.translate(tx, ty);
                    }
                }
                "Tm" => {
                    if op.operands.len() >= 6 {
                        text_matrix.set(
                            get_number_from_value(&op.operands[0]).unwrap_or(1.0),
                            get_number_from_value(&op.operands[1]).unwrap_or(0.0),
                            get_number_from_value(&op.operands[2]).unwrap_or(0.0),
                            get_number_from_value(&op.operands[3]).unwrap_or(1.0),
                            get_number_from_value(&op.operands[4]).unwrap_or(0.0),
                            get_number_from_value(&op.operands[5]).unwrap_or(0.0),
                        );
                    }
                }
                "T*" => {
                    text_matrix.next_line();
                }
                "Tj" | "TJ" => {
                    if in_text_block {
                        let text = if op.operator == "TJ" {
                            // TJ: array of strings and positioning adjustments
                            // Numbers indicate kerning/spacing adjustments in 1/1000 text space units
                            // Large negative values (like -200 to -300) often indicate word spaces
                            if let Some(PdfValue::Array(arr)) = op.operands.first() {
                                let mut combined = String::new();

                                for item in arr {
                                    match item {
                                        PdfValue::Str(bytes) => {
                                            combined.push_str(&self.backend.decode_text(
                                                page_id,
                                                &current_font_name,
                                                bytes,
                                            ));
                                        }
                                        PdfValue::Integer(n) => {
                                            let adjustment = -(*n as f32);
                                            maybe_insert_space_tj(&mut combined, adjustment);
                                        }
                                        PdfValue::Real(n) => {
                                            let adjustment = -n;
                                            maybe_insert_space_tj(&mut combined, adjustment);
                                        }
                                        _ => {}
                                    }
                                }
                                combined
                            } else {
                                String::new()
                            }
                        } else {
                            // Tj: single string
                            if let Some(PdfValue::Str(bytes)) = op.operands.first() {
                                self.backend.decode_text(page_id, &current_font_name, bytes)
                            } else {
                                String::new()
                            }
                        };

                        if !text.trim().is_empty() {
                            let (x, y) = text_matrix.get_position();
                            let effective_size = current_font_size * text_matrix.get_scale();
                            spans.push(TextSpan::new(
                                text,
                                x,
                                y,
                                effective_size,
                                current_font.clone(),
                            ));
                        }
                    }
                }
                "'" | "\"" => {
                    text_matrix.next_line();
                    if in_text_block {
                        let text_idx = if op.operator == "\"" { 2 } else { 0 };
                        if let Some(PdfValue::Str(bytes)) = op.operands.get(text_idx) {
                            let text = self.backend.decode_text(page_id, &current_font_name, bytes);

                            if !text.trim().is_empty() {
                                let (x, y) = text_matrix.get_position();
                                let effective_size = current_font_size * text_matrix.get_scale();
                                spans.push(TextSpan::new(
                                    text,
                                    x,
                                    y,
                                    effective_size,
                                    current_font.clone(),
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(spans)
    }

    /// Detect columns in a page based on vertical gap (gutter) detection.
    ///
    /// This looks for vertical empty spaces between text regions to identify
    /// column boundaries. Returns columns sorted from left to right.
    fn detect_columns(&self, spans: &[TextSpan]) -> Vec<Column> {
        if spans.is_empty() {
            return vec![];
        }

        // Find minimum and maximum X to determine page extent
        let min_x = spans
            .iter()
            .map(|s| s.x)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);
        let max_x = spans
            .iter()
            .map(|s| s.x + s.width)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        let page_width = max_x - min_x;

        // Don't detect columns if page is too narrow
        if page_width < 250.0 {
            return vec![Column {
                left: min_x - 10.0,
                right: max_x + 10.0,
                index: 0,
            }];
        }

        // Divide page into vertical slices and count spans in each
        let slice_width = 3.0; // Finer slices for better precision
        let num_slices = ((page_width / slice_width) as usize) + 1;
        let mut slice_occupancy = vec![0usize; num_slices];

        // Count how many spans occupy each slice
        for span in spans {
            let start_slice = ((span.x - min_x) / slice_width) as usize;
            let end_slice = (((span.x + span.width) - min_x) / slice_width) as usize;

            for slot in slice_occupancy
                .iter_mut()
                .take(end_slice.min(num_slices - 1) + 1)
                .skip(start_slice)
            {
                *slot += 1;
            }
        }

        // Find the largest gap (sequence of empty slices) in the middle 70% of the page
        // Extended from 50% to catch more gutters
        let search_start = num_slices * 15 / 100; // Start at 15%
        let search_end = num_slices * 85 / 100; // End at 85%

        let mut best_gap_start = 0;
        let mut best_gap_len = 0;
        let mut best_gap_center_dist = f32::MAX; // Distance from center

        let page_center = num_slices / 2;
        let mut current_gap_start = 0;
        let mut current_gap_len = 0;

        for (i, &occupancy) in slice_occupancy
            .iter()
            .enumerate()
            .take(search_end)
            .skip(search_start)
        {
            if occupancy == 0 {
                if current_gap_len == 0 {
                    current_gap_start = i;
                }
                current_gap_len += 1;
            } else {
                if current_gap_len > 0 {
                    let gap_center = current_gap_start + current_gap_len / 2;
                    let center_dist = (gap_center as i32 - page_center as i32).abs() as f32;

                    // Prefer gaps that are:
                    // 1. Larger (more confident it's a gutter)
                    // 2. Closer to center (more likely to be a column separator)
                    let current_gap_width = current_gap_len as f32 * slice_width;

                    if current_gap_width >= 10.0 {
                        // Minimum 10pt gap
                        // Score: gap_width * (1 - center_distance_ratio)
                        let best_gap_width = best_gap_len as f32 * slice_width;

                        // Prefer larger gaps, or similar-sized gaps closer to center
                        if current_gap_width > best_gap_width * 1.5
                            || (current_gap_width >= best_gap_width * 0.7
                                && center_dist < best_gap_center_dist)
                        {
                            best_gap_start = current_gap_start;
                            best_gap_len = current_gap_len;
                            best_gap_center_dist = center_dist;
                        }
                    }
                }
                current_gap_len = 0;
            }
        }

        // Check the last gap
        if current_gap_len > 0 {
            let gap_center = current_gap_start + current_gap_len / 2;
            let center_dist = (gap_center as i32 - page_center as i32).abs() as f32;
            let current_gap_width = current_gap_len as f32 * slice_width;
            let best_gap_width = best_gap_len as f32 * slice_width;

            if current_gap_width >= 10.0
                && (current_gap_width > best_gap_width * 1.5
                    || (current_gap_width >= best_gap_width * 0.7
                        && center_dist < best_gap_center_dist))
            {
                best_gap_start = current_gap_start;
                best_gap_len = current_gap_len;
            }
        }

        // Convert gap to actual X coordinates
        let gap_width = best_gap_len as f32 * slice_width;

        log::debug!(
            "Best gap: width={:.1}pt at x={:.1}, page_width={:.1}",
            gap_width,
            min_x + best_gap_start as f32 * slice_width,
            page_width
        );

        // Require a minimum gap width for column detection (at least 12 points)
        if gap_width < 12.0 {
            log::debug!("Gap too small (< 12pt), treating as single column");
            return vec![Column {
                left: min_x - 10.0,
                right: max_x + 10.0,
                index: 0,
            }];
        }

        // Calculate gutter center
        let gutter_center =
            min_x + (best_gap_start as f32 + best_gap_len as f32 / 2.0) * slice_width;

        // Validate that both columns have reasonable width (at least 80 points each)
        let left_col_width = gutter_center - min_x;
        let right_col_width = max_x - gutter_center;

        log::debug!(
            "Column widths: left={:.1}, right={:.1}",
            left_col_width,
            right_col_width
        );

        if left_col_width < 80.0 || right_col_width < 80.0 {
            log::debug!("Column too narrow, treating as single column");
            return vec![Column {
                left: min_x - 10.0,
                right: max_x + 10.0,
                index: 0,
            }];
        }

        // Validate that both columns have spans
        let left_spans = spans
            .iter()
            .filter(|s| s.x + s.width / 2.0 < gutter_center)
            .count();
        let right_spans = spans
            .iter()
            .filter(|s| s.x + s.width / 2.0 >= gutter_center)
            .count();

        log::debug!(
            "Spans: left={}, right={}, total={}",
            left_spans,
            right_spans,
            spans.len()
        );

        // Both columns should have at least 10% of spans
        let min_spans = spans.len() / 10;
        if left_spans < min_spans.max(2) || right_spans < min_spans.max(2) {
            log::debug!("Spans too imbalanced, treating as single column");
            return vec![Column {
                left: min_x - 10.0,
                right: max_x + 10.0,
                index: 0,
            }];
        }

        vec![
            Column {
                left: min_x - 10.0,
                right: gutter_center,
                index: 0,
            },
            Column {
                left: gutter_center,
                right: max_x + 10.0,
                index: 1,
            },
        ]
    }

    /// Group spans into lines based on Y position, using XY-Cut for layout segmentation.
    ///
    /// Uses the recursive XY-Cut algorithm to detect multi-column layouts and
    /// other complex structures. Each segmented region is processed independently
    /// as a single-column block.
    fn group_spans_into_lines(&self, spans: Vec<TextSpan>) -> Vec<TextLine> {
        if spans.is_empty() {
            return vec![];
        }

        // Convert spans to XY-cut blocks
        let blocks: Vec<super::xycut::Block> = spans
            .iter()
            .map(|s| super::xycut::Block {
                x: s.x,
                y: s.y,
                width: s.width,
                height: s.font_size,
            })
            .collect();

        // Determine gap thresholds based on median font size. These are
        // intentionally large so XY-Cut only fires on true multi-column
        // layouts — not on intra-table cell gaps or bulleted list
        // indentation, which previously fragmented pages into dozens of
        // groups on Hancom-produced PDFs.
        let median_font = median_font_size(&spans);
        let min_x_gap = (median_font * 5.0).max(60.0);
        let min_y_gap = (median_font * 3.0).max(36.0);

        let groups = super::xycut::xycut_segment(&blocks, min_x_gap, min_y_gap);

        log::debug!(
            "XY-Cut segmented {} spans into {} groups (median_font={:.1}, min_x_gap={:.1}, min_y_gap={:.1})",
            spans.len(),
            groups.len(),
            median_font,
            min_x_gap,
            min_y_gap,
        );

        if groups.len() <= 1 {
            // Single column — use simple grouping
            return self.group_spans_into_lines_single_column(spans);
        }

        // Multi-column: process each group independently
        let mut all_lines = Vec::new();
        for group in &groups {
            // Match spans to this group's blocks by position
            let group_spans: Vec<TextSpan> = spans
                .iter()
                .filter(|s| {
                    group
                        .iter()
                        .any(|b| (s.x - b.x).abs() < 1.0 && (s.y - b.y).abs() < 1.0)
                })
                .cloned()
                .collect();
            let lines = self.group_spans_into_lines_single_column(group_spans);
            all_lines.extend(lines);
        }
        all_lines
    }

    /// Group spans into lines using the legacy column-detection approach.
    ///
    /// This method uses `detect_columns()` to find a single gutter and split
    /// spans into columns. Kept as fallback; the primary path now uses XY-Cut
    /// via `group_spans_into_lines()`.
    #[allow(dead_code)]
    fn group_spans_into_lines_legacy_columns(&self, spans: Vec<TextSpan>) -> Vec<TextLine> {
        if spans.is_empty() {
            return vec![];
        }

        // Detect columns first
        let columns = self.detect_columns(&spans);

        log::debug!("Detected {} columns", columns.len());
        for col in &columns {
            log::debug!(
                "  Column {}: left={:.1}, right={:.1}",
                col.index,
                col.left,
                col.right
            );
        }

        // If single column, use simple Y-based grouping
        if columns.len() <= 1 {
            return self.group_spans_into_lines_single_column(spans);
        }

        // Multi-column layout: process each column separately, then interleave
        let mut column_lines: Vec<Vec<TextLine>> = vec![Vec::new(); columns.len()];

        // Assign spans to columns
        let mut column_spans: Vec<Vec<TextSpan>> = vec![Vec::new(); columns.len()];
        for span in spans {
            // Find which column this span belongs to
            let col_idx = columns
                .iter()
                .position(|c| c.contains_span(&span))
                .unwrap_or(0);
            column_spans[col_idx].push(span);
        }

        log::debug!(
            "Spans per column: {:?}",
            column_spans.iter().map(|v| v.len()).collect::<Vec<_>>()
        );

        // Group each column's spans into lines
        for (col_idx, col_spans) in column_spans.into_iter().enumerate() {
            column_lines[col_idx] = self.group_spans_into_lines_single_column(col_spans);
        }

        // Interleave lines from columns by Y position (top to bottom reading order)
        // First, collect all lines with their column index
        let mut all_lines: Vec<(usize, TextLine)> = Vec::new();
        for (col_idx, lines) in column_lines.into_iter().enumerate() {
            for line in lines {
                all_lines.push((col_idx, line));
            }
        }

        // Read each column sequentially: all lines from column 0, then column 1, etc.
        // Within each column, maintain top-to-bottom order (Y descending).
        all_lines.sort_by(|(col_a, line_a), (col_b, line_b)| {
            let col_cmp = col_a.cmp(col_b);
            if col_cmp != std::cmp::Ordering::Equal {
                col_cmp
            } else {
                line_b
                    .y
                    .partial_cmp(&line_a.y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        });

        all_lines.into_iter().map(|(_, line)| line).collect()
    }

    /// Simple Y-based line grouping for single-column layout.
    fn group_spans_into_lines_single_column(&self, spans: Vec<TextSpan>) -> Vec<TextLine> {
        if spans.is_empty() {
            return vec![];
        }

        // Sort spans by Y (descending, since PDF Y is bottom-up) then X
        let mut spans = spans;
        spans.sort_by(|a, b| {
            let y_cmp = b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal);
            if y_cmp == std::cmp::Ordering::Equal {
                a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                y_cmp
            }
        });

        // Merge adjacent single/few-character spans that likely form words.
        // This fixes the "w arranty", "M ac B ook" splitting caused by
        // per-character text rendering in some PDFs.
        spans = merge_fragmented_spans(spans);

        let mut lines: Vec<TextLine> = Vec::new();
        let mut current_line_spans: Vec<TextSpan> = Vec::new();
        let mut current_y: Option<f32> = None;

        for span in spans {
            let y_tolerance = span.font_size * 0.3; // Allow 30% of font size variance

            if let Some(y) = current_y {
                if (span.y - y).abs() <= y_tolerance {
                    // Same line
                    current_line_spans.push(span);
                } else {
                    // New line
                    if !current_line_spans.is_empty() {
                        lines.push(TextLine::from_spans(std::mem::take(
                            &mut current_line_spans,
                        )));
                    }
                    current_y = Some(span.y);
                    current_line_spans.push(span);
                }
            } else {
                current_y = Some(span.y);
                current_line_spans.push(span);
            }
        }

        // Don't forget the last line
        if !current_line_spans.is_empty() {
            lines.push(TextLine::from_spans(current_line_spans));
        }

        lines
    }

    /// Detect headings based on font size hierarchy.
    fn detect_headings(&self, mut lines: Vec<TextLine>) -> Vec<TextLine> {
        // Snapshot each line's font size so neighbour lookups aren't polluted
        // by mutations inside the loop.
        let sizes: Vec<f32> = lines.iter().map(|l| l.font_size).collect();
        let body_size = self.font_stats.body_size;

        for (i, line) in lines.iter_mut().enumerate() {
            let visible_chars: usize = line
                .text()
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
                .count();
            if visible_chars < 3 {
                continue;
            }

            // List / bullet marker exclusion — never promote a line that
            // begins with a common bullet/list glyph. These are inline
            // enumerations inside body content, regardless of font size.
            let trimmed = line.text();
            let trimmed = trimmed.trim_start();
            if let Some(first) = trimmed.chars().next() {
                const BULLETS: &[char] =
                    &['-', '*', '·', '•', '◦', '▶', '▷', '◎', '☞', '※'];
                if BULLETS.contains(&first) {
                    continue;
                }
            }

            let level = self
                .font_stats
                .get_heading_level(line.font_size, line.is_bold() || line.is_uppercase());
            if level == 0 {
                continue;
            }

            // Neighbour-context suppression — if both prev and next lines
            // share the same font size (within 0.5pt), this line is part of
            // a body run, not a standalone heading. Prevents mid-paragraph
            // or table-cell promotion on Hancom docs where body font varies
            // by a few pts across cells.
            let prev_size = if i > 0 { Some(sizes[i - 1]) } else { None };
            let next_size = if i + 1 < sizes.len() {
                Some(sizes[i + 1])
            } else {
                None
            };
            let same = |a: f32, b: f32| (a - b).abs() < 0.5;
            // Neighbour-context suppression — if EITHER adjacent line
            // shares the same font size (±0.5pt), this line is likely part
            // of a sibling run (table column, list). Only promote when
            // the line sits alone within its font-size cohort.
            let matches_prev = prev_size.map_or(false, |p| same(p, line.font_size));
            let matches_next = next_size.map_or(false, |n| same(n, line.font_size));
            if (matches_prev || matches_next) && line.font_size < body_size + 6.0 {
                continue;
            }

            line.is_heading = true;
            line.heading_level = level;
        }
        lines
    }

    /// Group lines into blocks (paragraphs) based on spacing.
    fn group_lines_into_blocks(&self, lines: Vec<TextLine>) -> Vec<TextBlock> {
        if lines.is_empty() {
            return vec![];
        }

        let mut blocks: Vec<TextBlock> = Vec::new();
        let mut current_block_lines: Vec<TextLine> = Vec::new();

        // Calculate average line spacing
        let avg_spacing = self.calculate_avg_line_spacing(&lines);

        for (i, line) in lines.into_iter().enumerate() {
            if i == 0 {
                current_block_lines.push(line);
                continue;
            }

            let prev_line = current_block_lines.last().unwrap();

            // Check if this should start a new block
            let should_break = self.should_break_block(prev_line, &line, avg_spacing);

            if should_break {
                // Create block from current lines
                if !current_block_lines.is_empty() {
                    let block_type = if current_block_lines.iter().any(|l| l.is_heading) {
                        BlockType::Heading
                    } else {
                        BlockType::Paragraph
                    };
                    let mut block =
                        TextBlock::new(std::mem::take(&mut current_block_lines), block_type);
                    if block_type == BlockType::Heading {
                        block.heading_level = block
                            .lines
                            .iter()
                            .filter(|l| l.is_heading)
                            .map(|l| l.heading_level)
                            .min()
                            .unwrap_or(0);
                    }
                    blocks.push(block);
                }
            }

            current_block_lines.push(line);
        }

        // Don't forget the last block
        if !current_block_lines.is_empty() {
            let block_type = if current_block_lines.iter().any(|l| l.is_heading) {
                BlockType::Heading
            } else {
                BlockType::Paragraph
            };
            let mut block = TextBlock::new(current_block_lines, block_type);
            if block_type == BlockType::Heading {
                block.heading_level = block
                    .lines
                    .iter()
                    .filter(|l| l.is_heading)
                    .map(|l| l.heading_level)
                    .min()
                    .unwrap_or(0);
            }
            blocks.push(block);
        }

        blocks
    }

    /// Calculate average line spacing.
    fn calculate_avg_line_spacing(&self, lines: &[TextLine]) -> f32 {
        if lines.len() < 2 {
            return 12.0; // Default
        }

        let spacings: Vec<f32> = lines
            .windows(2)
            .map(|w| (w[0].y - w[1].y).abs())
            .filter(|s| *s > 0.1) // Filter out very small spacings
            .collect();

        if spacings.is_empty() {
            return 12.0;
        }

        spacings.iter().sum::<f32>() / spacings.len() as f32
    }

    /// Determine if a new block should start.
    fn should_break_block(
        &self,
        prev_line: &TextLine,
        curr_line: &TextLine,
        avg_spacing: f32,
    ) -> bool {
        // Heading always starts a new block, UNLESS the previous line is
        // also a heading of the same level sitting close by (within ~2x
        // line-height). This merges decorative stacked titles on covers —
        // e.g. "스마트\n제조혁신\n통합공고" stays one heading block.
        if curr_line.is_heading {
            // Merge consecutive heading lines that are spatially close and
            // at similar font size (≤ 2pt delta). Level difference is
            // tolerated — decorative stacked titles often vary font size
            // per word. The block picks up the minimum (most prominent)
            // level via existing `block.heading_level = ...min()` logic.
            if prev_line.is_heading
                && (prev_line.font_size - curr_line.font_size).abs() <= 2.0
            {
                let gap = (prev_line.y - curr_line.y).abs();
                let bigger = prev_line.font_size.max(curr_line.font_size);
                let close = gap <= (bigger * 2.5).max(avg_spacing * 2.0);
                if close {
                    return false;
                }
            }
            return true;
        }

        // After a heading, start new block
        if prev_line.is_heading {
            return true;
        }

        // Large spacing indicates new paragraph
        let spacing = (prev_line.y - curr_line.y).abs();
        if spacing > avg_spacing * 1.5 {
            return true;
        }

        // Significant font size change (only break on >=2pt difference —
        // smaller changes are common in superscripts / mixed-font Korean
        // text and shouldn't fragment paragraphs).
        if (prev_line.font_size - curr_line.font_size).abs() >= 2.0 {
            return true;
        }

        // Significant left margin change (indentation) — raised from 20pt
        // to 40pt so minor indent variation within a Hancom bullet list
        // doesn't start a new block per line.
        if (prev_line.x - curr_line.x).abs() > 40.0 {
            return true;
        }

        false
    }
}

/// Filter out header/footer text spans (page numbers, running headers).
///
/// Removes spans in the top/bottom margin that contain only numbers or short
/// page-number patterns (e.g. "- 3 -", "Page 5", "2 / 10").
fn filter_header_footer_spans(spans: &mut Vec<TextSpan>, page_height: f32) {
    if spans.is_empty() || page_height <= 0.0 {
        return;
    }

    // Define margin regions: top/bottom 5% of page height.
    // PDF Y axis is bottom-up: Y=0 is at the bottom of the page.
    let margin = page_height * 0.05;
    let top_threshold = page_height - margin; // Near the top edge
    let bottom_threshold = margin; // Near the bottom edge

    spans.retain(|span| {
        let in_header = span.y >= top_threshold;
        let in_footer = span.y <= bottom_threshold;

        if !in_header && !in_footer {
            return true; // Keep spans that are not in the margins
        }

        let text = span.text.trim();
        if text.is_empty() {
            return false; // Remove empty spans in margins
        }

        // Keep the span unless it looks like a bare page number
        let is_page_num = text.chars().all(|c| c.is_ascii_digit()) || is_page_number_pattern(text);

        !is_page_num
    });
}

/// Return `true` if `text` matches a common page-number decoration pattern.
///
/// Recognised patterns:
/// - `"- N -"` / `"– N –"` / `"— N —"` (hyphen/dash-surrounded numbers)
/// - `"Page N"` / `"page N"`
/// - `"N / M"` or `"N of M"` (fraction-style)
fn is_page_number_pattern(text: &str) -> bool {
    let text = text.trim();

    // Pattern: "- N -" or "– N –" or "— N —"
    for dash in &['-', '–', '—'] {
        let dash_str = dash.to_string();
        if let Some(inner) = text.strip_prefix(dash_str.as_str()) {
            if let Some(inner) = inner.trim().strip_suffix(dash_str.as_str()) {
                if inner.trim().chars().all(|c| c.is_ascii_digit()) {
                    return true;
                }
            }
        }
    }

    // Pattern: "Page N" or "page N"
    if let Some(rest) = text
        .strip_prefix("Page ")
        .or_else(|| text.strip_prefix("page "))
    {
        if rest.trim().chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }

    // Pattern: "N / M" or "N of M"
    // Split on whitespace and '/', keep non-empty tokens
    let tokens: Vec<&str> = text
        .split(|c: char| c == '/' || c.is_ascii_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.len() == 3
        && tokens[0].chars().all(|c| c.is_ascii_digit())
        && (tokens[1] == "of" || tokens[1] == "/")
        && tokens[2].chars().all(|c| c.is_ascii_digit())
    {
        return true;
    }
    // "N / M" where slash is surrounded by spaces → tokens = ["N", "M"] after filtering
    if tokens.len() == 2
        && tokens[0].chars().all(|c| c.is_ascii_digit())
        && tokens[1].chars().all(|c| c.is_ascii_digit())
        && text.contains('/')
    {
        return true;
    }

    false
}

/// Font information.
#[derive(Debug, Clone)]
struct FontInfo {
    name: String,
}

/// Text matrix for tracking position in content stream.
#[derive(Debug, Clone)]
struct TextMatrix {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32, // X translation
    f: f32, // Y translation
    line_y: f32,
}

impl Default for TextMatrix {
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
            line_y: 0.0,
        }
    }
}

impl TextMatrix {
    fn set(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.a = a;
        self.b = b;
        self.c = c;
        self.d = d;
        self.e = e;
        self.f = f;
        self.line_y = f;
    }

    fn translate(&mut self, tx: f32, ty: f32) {
        self.e += tx * self.a + ty * self.c;
        self.f += tx * self.b + ty * self.d;
        if ty != 0.0 {
            self.line_y = self.f;
        }
    }

    fn next_line(&mut self) {
        // Default line leading (could be set by TL operator)
        self.f -= 12.0 * self.d;
        self.line_y = self.f;
    }

    fn get_position(&self) -> (f32, f32) {
        (self.e, self.f)
    }

    fn get_scale(&self) -> f32 {
        // Return the vertical scale factor
        (self.a * self.a + self.c * self.c).sqrt()
    }
}

/// Insert a space into `text` if it doesn't already end with one and the
/// last character is not from a spaceless script (CJK/Japanese).
/// Insert a space in TJ array based on kerning adjustment, with script-aware thresholds.
///
/// TJ adjustments are in 1/1000 text space units. The threshold for inserting a space
/// varies by script:
/// - Latin: 200 units (~33% of typical char width ~600)
/// - Hangul (Korean): 500 units (~50% of typical char width 1000)
///   Korean uses word spaces, but kerning between syllables is typically 100-300 units.
/// - CJK (Chinese/Japanese): never insert spaces (handled by is_spaceless_script_char)
fn maybe_insert_space_tj(text: &mut String, adjustment: f32) {
    if text.is_empty() || text.ends_with(' ') || text.ends_with('\u{00A0}') {
        return;
    }

    if let Some(last_char) = text.chars().last() {
        if is_spaceless_script_char(last_char) {
            return;
        }

        let threshold = if is_hangul_char(last_char) {
            500.0
        } else {
            200.0
        };
        if adjustment > threshold {
            text.push(' ');
        }
    }
}

/// Check if a character is a Hangul (Korean) syllable or jamo.
fn is_hangul_char(c: char) -> bool {
    let code = c as u32;
    // Hangul Syllables
    (0xAC00..=0xD7AF).contains(&code)
    // Hangul Jamo
    || (0x1100..=0x11FF).contains(&code)
    // Hangul Compatibility Jamo
    || (0x3130..=0x318F).contains(&code)
    // Hangul Jamo Extended-A/B
    || (0xA960..=0xA97F).contains(&code)
    || (0xD7B0..=0xD7FF).contains(&code)
}

/// Check if a character is a CJK (Chinese/Japanese/Korean) character.
///
/// CJK characters typically don't need spaces between them.
/// Compute the median font size from a slice of spans.
fn median_font_size(spans: &[TextSpan]) -> f32 {
    if spans.is_empty() {
        return 12.0;
    }
    let mut sizes: Vec<f32> = spans.iter().map(|s| s.font_size).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sizes[sizes.len() / 2]
}

/// Check if character is from a script that doesn't use word spaces.
/// Chinese and Japanese don't use spaces between words, but Korean does.
fn is_spaceless_script_char(c: char) -> bool {
    let code = c as u32;

    // CJK Unified Ideographs (Chinese characters, used in Chinese/Japanese)
    (0x4E00..=0x9FFF).contains(&code)
    // CJK Unified Ideographs Extension A
    || (0x3400..=0x4DBF).contains(&code)
    // CJK Unified Ideographs Extension B-F
    || (0x20000..=0x2A6DF).contains(&code)
    || (0x2A700..=0x2B73F).contains(&code)
    || (0x2B740..=0x2B81F).contains(&code)
    || (0x2B820..=0x2CEAF).contains(&code)
    || (0x2CEB0..=0x2EBEF).contains(&code)
    // Hiragana (Japanese)
    || (0x3040..=0x309F).contains(&code)
    // Katakana (Japanese)
    || (0x30A0..=0x30FF).contains(&code)
    // NOTE: Hangul (Korean) is NOT included - Korean uses word spaces like English
    // CJK Symbols and Punctuation
    || (0x3000..=0x303F).contains(&code)
}

/// Merge adjacent fragmented spans that likely form words.
///
/// Some PDFs render text character-by-character with separate Tj operations,
/// creating many single-character spans with width=0. This function merges
/// consecutive short spans (≤3 chars) with width=0 that are:
/// - On the same baseline (same Y within tolerance)
/// - Using the same font at the same size
/// - Positioned sequentially without large gaps
///
/// Only merges into width=0 spans — when the previous span also has width=0 or
/// is itself a recently-merged fragment. This prevents merging normal
/// multi-character spans that happen to be adjacent.
fn merge_fragmented_spans(spans: Vec<TextSpan>) -> Vec<TextSpan> {
    if spans.len() < 2 {
        return spans;
    }

    let mut result: Vec<TextSpan> = Vec::with_capacity(spans.len());
    // Track which result spans were created by merging (started as width=0)
    let mut was_fragment: Vec<bool> = Vec::with_capacity(spans.len());

    for span in spans {
        let is_fragment = span.text.chars().count() <= 3 && span.width <= 0.0;

        let should_merge =
            if let Some((prev, prev_was_frag)) = result.last().zip(was_fragment.last()) {
                // Only merge if BOTH are fragments (or prev was already merged from fragments)
                if !is_fragment || !prev_was_frag {
                    false
                } else {
                    // Same baseline (Y within tolerance)
                    let y_tolerance = span.font_size * 0.3;
                    let same_y = (prev.y - span.y).abs() <= y_tolerance;

                    // Same font and size
                    let same_font = prev.font_name == span.font_name
                        && (prev.font_size - span.font_size).abs() < 0.1;

                    if !same_y || !same_font {
                        false
                    } else {
                        // Estimate character width from font size
                        let est_char_width = prev.font_size * 0.6;

                        // Estimate where the previous span ends
                        let prev_end = if prev.width > 0.0 {
                            prev.x + prev.width
                        } else {
                            prev.x + est_char_width * prev.text.chars().count() as f32
                        };

                        let gap = span.x - prev_end;

                        // Merge if gap is small enough to not be a word space.
                        // Character-to-character gap within a word: 0 to ~0.3 * char_width
                        // Word space gap: ~0.4 * char_width or more
                        gap < est_char_width * 0.4 && gap > -est_char_width * 0.5
                    }
                }
            } else {
                false
            };

        if should_merge {
            let prev = result.last_mut().unwrap();
            // Update width to cover the merged extent
            let new_end = span.x + span.font_size * 0.6 * span.text.chars().count() as f32;
            prev.width = new_end - prev.x;
            prev.text.push_str(&span.text);
        } else {
            was_fragment.push(is_fragment);
            result.push(span);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_statistics() {
        let mut stats = FontStatistics::default();
        // Simulate body text (most common)
        for _ in 0..100 {
            stats.add_size(12.0);
        }
        // Simulate headings
        for _ in 0..5 {
            stats.add_size(18.0);
        }
        for _ in 0..3 {
            stats.add_size(24.0);
        }

        stats.analyze();

        assert!((stats.body_size - 12.0).abs() < 0.1);
        assert_eq!(stats.get_heading_level(12.0, false), 0);
        assert!(stats.get_heading_level(18.0, false) > 0);
        assert!(stats.get_heading_level(24.0, false) > 0);
    }

    #[test]
    fn test_text_span_bold_detection() {
        let span = TextSpan::new(
            "Test".to_string(),
            0.0,
            0.0,
            12.0,
            "Helvetica-Bold".to_string(),
        );
        assert!(span.is_bold);
        assert!(!span.is_italic);

        let span2 = TextSpan::new(
            "Test".to_string(),
            0.0,
            0.0,
            12.0,
            "Helvetica-Oblique".to_string(),
        );
        assert!(!span2.is_bold);
        assert!(span2.is_italic);
    }

    #[test]
    fn test_merge_fragmented_spans_single_chars() {
        // Simulate per-character rendering: "Hello" as 5 separate spans
        let spans: Vec<TextSpan> = "Hello"
            .chars()
            .enumerate()
            .map(|(i, c)| TextSpan {
                text: c.to_string(),
                x: 100.0 + i as f32 * 6.0,
                y: 500.0,
                width: 0.0, // width=0 is the fragmentation signal
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                is_bold: false,
                is_italic: false,
            })
            .collect();

        let merged = merge_fragmented_spans(spans);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Hello");
    }

    #[test]
    fn test_merge_fragmented_spans_preserves_normal() {
        // Normal multi-character spans should not be merged unnecessarily
        let spans = vec![
            TextSpan {
                text: "Hello".to_string(),
                x: 100.0,
                y: 500.0,
                width: 30.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                is_bold: false,
                is_italic: false,
            },
            TextSpan {
                text: "World".to_string(),
                x: 145.0, // gap indicates word space
                y: 500.0,
                width: 30.0,
                font_size: 12.0,
                font_name: "Helvetica".to_string(),
                is_bold: false,
                is_italic: false,
            },
        ];

        let merged = merge_fragmented_spans(spans);
        // Should not merge because fragmentation threshold is not met
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_column_contains() {
        let col = Column {
            left: 100.0,
            right: 200.0,
            index: 0,
        };
        assert!(col.contains(100.0));
        assert!(col.contains(150.0));
        assert!(col.contains(200.0));
        assert!(!col.contains(99.0));
        assert!(!col.contains(201.0));
    }

    #[test]
    fn test_column_contains_span() {
        let col = Column {
            left: 100.0,
            right: 200.0,
            index: 0,
        };

        // Span fully inside column
        let span1 = TextSpan::new(
            "Test".to_string(),
            120.0,
            0.0,
            12.0,
            "Helvetica".to_string(),
        );
        let span1 = TextSpan {
            width: 50.0,
            ..span1
        };
        assert!(col.contains_span(&span1));

        // Span center inside column
        let span2 = TextSpan::new("Test".to_string(), 90.0, 0.0, 12.0, "Helvetica".to_string());
        let span2 = TextSpan {
            width: 40.0,
            ..span2
        }; // center at 110
        assert!(col.contains_span(&span2));

        // Span completely outside
        let span3 = TextSpan::new(
            "Test".to_string(),
            250.0,
            0.0,
            12.0,
            "Helvetica".to_string(),
        );
        let span3 = TextSpan {
            width: 30.0,
            ..span3
        };
        assert!(!col.contains_span(&span3));
    }
}
