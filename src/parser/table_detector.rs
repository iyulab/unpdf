//! Table detection using text position analysis (Stream mode algorithm).
//!
//! Inspired by Camelot's Stream mode, this module detects tables by analyzing
//! text alignment patterns without relying on graphical lines.

use std::collections::HashMap;

use crate::model::{Table, TableCell, TableRow};

use super::layout::TextSpan;

/// A detected table region with its content.
#[derive(Debug, Clone)]
pub struct DetectedTable {
    /// Starting Y coordinate (top of table, in PDF coords)
    pub top_y: f32,
    /// Ending Y coordinate (bottom of table)
    pub bottom_y: f32,
    /// Left X boundary
    pub left_x: f32,
    /// Right X boundary
    pub right_x: f32,
    /// Detected column boundaries (X coordinates)
    pub columns: Vec<f32>,
    /// Rows of text spans grouped by Y position
    pub rows: Vec<TableRowData>,
}

/// A row of text spans in a table.
#[derive(Debug, Clone)]
pub struct TableRowData {
    /// Y position of this row
    pub y: f32,
    /// Spans in this row, sorted by X
    pub spans: Vec<TextSpan>,
}

/// Table detector configuration.
#[derive(Debug, Clone)]
pub struct TableDetectorConfig {
    /// Minimum number of rows to consider as table
    pub min_rows: usize,
    /// Minimum number of columns to consider as table
    pub min_columns: usize,
    /// Maximum number of columns (above this, likely word-level splitting)
    pub max_columns: usize,
    /// Y tolerance for grouping spans into rows (fraction of font size)
    pub y_tolerance_factor: f32,
    /// Minimum column alignment ratio (0.0-1.0)
    pub min_alignment_ratio: f32,
    /// Minimum gap between columns (points)
    pub min_column_gap: f32,
}

impl Default for TableDetectorConfig {
    fn default() -> Self {
        Self {
            min_rows: 2,
            min_columns: 2,
            max_columns: 6,
            y_tolerance_factor: 0.4,
            min_alignment_ratio: 0.3, // Lowered from 0.5 to detect more tables
            min_column_gap: 15.0,     // Increased to avoid false positives
        }
    }
}

/// Detects tables in a list of text spans.
pub struct TableDetector {
    config: TableDetectorConfig,
}

impl TableDetector {
    /// Create a new table detector with default configuration.
    pub fn new() -> Self {
        Self {
            config: TableDetectorConfig::default(),
        }
    }

    /// Create a new table detector with custom configuration.
    pub fn with_config(config: TableDetectorConfig) -> Self {
        Self { config }
    }

    /// Detect tables in the given spans.
    ///
    /// Returns detected tables and the spans that were NOT part of tables.
    pub fn detect(&self, spans: Vec<TextSpan>) -> (Vec<DetectedTable>, Vec<TextSpan>) {
        log::debug!("TableDetector: starting with {} spans", spans.len());

        if spans.len() < self.config.min_rows * self.config.min_columns {
            log::debug!(
                "TableDetector: not enough spans ({} < {})",
                spans.len(),
                self.config.min_rows * self.config.min_columns
            );
            return (vec![], spans);
        }

        // Step 1: Group spans into rows by Y position
        let rows = self.group_into_rows(&spans);
        log::debug!("TableDetector: grouped into {} rows", rows.len());

        if rows.len() < self.config.min_rows {
            log::debug!(
                "TableDetector: not enough rows ({} < {})",
                rows.len(),
                self.config.min_rows
            );
            return (vec![], spans);
        }

        // Step 2: Detect column boundaries from text edges
        let columns = self.detect_columns(&rows);
        log::debug!(
            "TableDetector: detected {} columns at positions: {:?}",
            columns.len(),
            columns
        );

        if columns.len() < self.config.min_columns {
            log::debug!(
                "TableDetector: not enough columns ({} < {})",
                columns.len(),
                self.config.min_columns
            );
            return (vec![], spans);
        }

        // Step 3: Find table regions (contiguous rows with consistent column alignment)
        let table_regions = self.find_table_regions(&rows, &columns);
        log::debug!("TableDetector: found {} table regions", table_regions.len());

        if table_regions.is_empty() {
            log::debug!("TableDetector: no table regions found");
            return (vec![], spans);
        }

        // Step 4: Convert regions to detected tables
        let mut detected_tables = Vec::new();
        let mut used_span_indices: std::collections::HashSet<usize> =
            std::collections::HashSet::new();

        for (start_row, end_row) in table_regions {
            let table_rows: Vec<TableRowData> = rows[start_row..=end_row].to_vec();

            if table_rows.is_empty() {
                continue;
            }

            // Calculate table boundaries
            let top_y = table_rows.first().map(|r| r.y).unwrap_or(0.0);
            let bottom_y = table_rows.last().map(|r| r.y).unwrap_or(0.0);
            let left_x = table_rows
                .iter()
                .flat_map(|r| r.spans.iter())
                .map(|s| s.x)
                .min_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(0.0);
            let right_x = table_rows
                .iter()
                .flat_map(|r| r.spans.iter())
                .map(|s| s.x + s.width)
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(0.0);

            // Re-detect columns for this specific table region
            let table_columns = self.detect_columns(&table_rows);

            if table_columns.len() >= self.config.min_columns {
                // Reject tables with too many columns (likely word-level splitting)
                if table_columns.len() > self.config.max_columns {
                    log::debug!(
                        "TableDetector: skipping region — too many columns ({} > {})",
                        table_columns.len(),
                        self.config.max_columns
                    );
                    continue;
                }

                // Check if this is actually a list pattern, not a real table
                if self.is_list_pattern(&table_rows, &table_columns) {
                    log::debug!(
                        "TableDetector: skipping region — detected as list pattern"
                    );
                    continue;
                }

                // Mark spans as used
                for row in &table_rows {
                    for span in &row.spans {
                        // Find index in original spans
                        for (i, orig_span) in spans.iter().enumerate() {
                            if (orig_span.x - span.x).abs() < 0.1
                                && (orig_span.y - span.y).abs() < 0.1
                                && orig_span.text == span.text
                            {
                                used_span_indices.insert(i);
                            }
                        }
                    }
                }

                detected_tables.push(DetectedTable {
                    top_y,
                    bottom_y,
                    left_x,
                    right_x,
                    columns: table_columns,
                    rows: table_rows,
                });
            }
        }

        // Return unused spans
        let unused_spans: Vec<TextSpan> = spans
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !used_span_indices.contains(i))
            .map(|(_, span)| span)
            .collect();

        (detected_tables, unused_spans)
    }

    /// Group spans into rows by Y position.
    fn group_into_rows(&self, spans: &[TextSpan]) -> Vec<TableRowData> {
        if spans.is_empty() {
            return vec![];
        }

        // Sort by Y (descending for PDF coords) then X
        let mut sorted_spans = spans.to_vec();
        sorted_spans.sort_by(|a, b| {
            let y_cmp = b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal);
            if y_cmp == std::cmp::Ordering::Equal {
                a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                y_cmp
            }
        });

        let mut rows: Vec<TableRowData> = Vec::new();
        let mut current_row_spans: Vec<TextSpan> = Vec::new();
        let mut current_y: Option<f32> = None;

        for span in sorted_spans {
            let y_tolerance = span.font_size * self.config.y_tolerance_factor;

            match current_y {
                Some(y) if (span.y - y).abs() <= y_tolerance => {
                    current_row_spans.push(span);
                }
                _ => {
                    if !current_row_spans.is_empty() {
                        let avg_y = current_row_spans.iter().map(|s| s.y).sum::<f32>()
                            / current_row_spans.len() as f32;
                        rows.push(TableRowData {
                            y: avg_y,
                            spans: std::mem::take(&mut current_row_spans),
                        });
                    }
                    current_y = Some(span.y);
                    current_row_spans.push(span);
                }
            }
        }

        // Don't forget the last row
        if !current_row_spans.is_empty() {
            let avg_y =
                current_row_spans.iter().map(|s| s.y).sum::<f32>() / current_row_spans.len() as f32;
            rows.push(TableRowData {
                y: avg_y,
                spans: current_row_spans,
            });
        }

        rows
    }

    /// Detect column boundaries from text edges.
    ///
    /// Uses a more sophisticated approach:
    /// 1. For each row, collect X positions where text starts
    /// 2. Find X positions that align across multiple rows
    /// 3. Additionally, detect columns by looking at per-row span count consistency
    fn detect_columns(&self, rows: &[TableRowData]) -> Vec<f32> {
        if rows.is_empty() {
            return vec![];
        }

        // Approach 1: Look at rows with multiple spans (likely table rows)
        let multi_span_rows: Vec<&TableRowData> =
            rows.iter().filter(|r| r.spans.len() >= 2).collect();

        log::debug!(
            "TableDetector: {} rows have 2+ spans",
            multi_span_rows.len()
        );

        if multi_span_rows.len() < self.config.min_rows {
            // Not enough multi-span rows, fall back to simpler detection
            return self.detect_columns_simple(rows);
        }

        // Collect all left edges from multi-span rows
        let mut edge_counts: HashMap<i32, usize> = HashMap::new();
        let bucket_size = 5.0; // Group X positions within 5pt

        for row in &multi_span_rows {
            // Use a set to count each bucket only once per row
            let mut row_buckets: std::collections::HashSet<i32> = std::collections::HashSet::new();
            for span in &row.spans {
                let bucket = (span.x / bucket_size).round() as i32;
                row_buckets.insert(bucket);
            }
            for bucket in row_buckets {
                *edge_counts.entry(bucket).or_insert(0) += 1;
            }
        }

        // Find edges that appear in a good portion of multi-span rows
        let min_occurrences =
            (multi_span_rows.len() as f32 * self.config.min_alignment_ratio) as usize;
        let min_occurrences = min_occurrences.max(2);

        log::debug!(
            "TableDetector: min_occurrences = {}, edge_counts = {:?}",
            min_occurrences,
            edge_counts
        );

        let mut column_edges: Vec<f32> = edge_counts
            .iter()
            .filter(|(_, count)| **count >= min_occurrences)
            .map(|(bucket, _)| *bucket as f32 * bucket_size)
            .collect();

        column_edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Merge close edges
        let mut merged_edges: Vec<f32> = Vec::new();
        for edge in column_edges {
            if merged_edges.is_empty() {
                merged_edges.push(edge);
            } else {
                let last = *merged_edges.last().unwrap();
                if edge - last >= self.config.min_column_gap {
                    merged_edges.push(edge);
                }
            }
        }

        log::debug!("TableDetector: merged column edges = {:?}", merged_edges);

        merged_edges
    }

    /// Simpler column detection for when few rows have multiple spans.
    fn detect_columns_simple(&self, rows: &[TableRowData]) -> Vec<f32> {
        if rows.is_empty() {
            return vec![];
        }

        let mut edge_counts: HashMap<i32, usize> = HashMap::new();
        let bucket_size = 5.0;

        for row in rows {
            for span in &row.spans {
                let bucket = (span.x / bucket_size).round() as i32;
                *edge_counts.entry(bucket).or_insert(0) += 1;
            }
        }

        let min_occurrences = (rows.len() as f32 * self.config.min_alignment_ratio) as usize;
        let min_occurrences = min_occurrences.max(2);

        let mut column_edges: Vec<f32> = edge_counts
            .iter()
            .filter(|(_, count)| **count >= min_occurrences)
            .map(|(bucket, _)| *bucket as f32 * bucket_size)
            .collect();

        column_edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let mut merged_edges: Vec<f32> = Vec::new();
        for edge in column_edges {
            if merged_edges.is_empty() {
                merged_edges.push(edge);
            } else {
                let last = *merged_edges.last().unwrap();
                if edge - last >= self.config.min_column_gap {
                    merged_edges.push(edge);
                }
            }
        }

        merged_edges
    }

    /// Find contiguous row regions that form tables.
    fn find_table_regions(&self, rows: &[TableRowData], columns: &[f32]) -> Vec<(usize, usize)> {
        if rows.is_empty() || columns.len() < self.config.min_columns {
            return vec![];
        }

        let mut regions: Vec<(usize, usize)> = Vec::new();
        let mut current_start: Option<usize> = None;
        let mut consecutive_table_rows = 0;

        for (i, row) in rows.iter().enumerate() {
            // Check if this row has good column alignment
            let alignment_score = self.calculate_alignment_score(row, columns);

            if alignment_score >= self.config.min_alignment_ratio {
                if current_start.is_none() {
                    current_start = Some(i);
                }
                consecutive_table_rows += 1;
            } else {
                // End of a potential table region
                if let Some(start) = current_start {
                    if consecutive_table_rows >= self.config.min_rows {
                        regions.push((start, i - 1));
                    }
                }
                current_start = None;
                consecutive_table_rows = 0;
            }
        }

        // Check the last region
        if let Some(start) = current_start {
            if consecutive_table_rows >= self.config.min_rows {
                regions.push((start, rows.len() - 1));
            }
        }

        regions
    }

    /// Calculate how well a row aligns with the detected columns.
    fn calculate_alignment_score(&self, row: &TableRowData, columns: &[f32]) -> f32 {
        if row.spans.is_empty() || columns.is_empty() {
            return 0.0;
        }

        let tolerance = 5.0; // 5pt tolerance for alignment

        let aligned_spans = row
            .spans
            .iter()
            .filter(|span| columns.iter().any(|col| (span.x - col).abs() <= tolerance))
            .count();

        aligned_spans as f32 / row.spans.len() as f32
    }

    /// Convert a detected table to the model Table type.
    pub fn to_table_model(&self, detected: &DetectedTable) -> Table {
        let mut table = Table::new();

        // First row is treated as header
        table.header_rows = if detected.rows.len() > 1 { 1 } else { 0 };

        // Store column widths for reference
        let columns = &detected.columns;

        for (row_idx, row_data) in detected.rows.iter().enumerate() {
            // Create a cell content vector for each column
            let mut cell_contents: Vec<Vec<String>> = vec![Vec::new(); columns.len()];

            // Assign each span to exactly one column (the closest one)
            for span in &row_data.spans {
                let span_x = span.x;

                // Find the column this span belongs to
                // Use the span's left edge to determine column assignment
                let col_idx = self.find_column_for_span(span_x, columns, detected.right_x);

                if col_idx < cell_contents.len() {
                    cell_contents[col_idx].push(span.text.trim().to_string());
                }
            }

            // Build cells from collected content
            let cells: Vec<TableCell> = cell_contents
                .into_iter()
                .map(|contents| {
                    let text = contents.join(" ");
                    TableCell::text(text)
                })
                .collect();

            let table_row = if row_idx == 0 && table.header_rows > 0 {
                TableRow::header(cells)
            } else {
                TableRow::new(cells)
            };

            table.add_row(table_row);
        }

        // Calculate column widths
        let widths: Vec<f32> = (0..columns.len())
            .map(|i| {
                if i + 1 < columns.len() {
                    columns[i + 1] - columns[i]
                } else {
                    detected.right_x - columns[i]
                }
            })
            .collect();
        table.column_widths = Some(widths);

        table
    }

    /// Find which column a span belongs to based on its X position.
    fn find_column_for_span(&self, span_x: f32, columns: &[f32], right_x: f32) -> usize {
        if columns.is_empty() {
            return 0;
        }

        // Find the column where span_x falls within [col_start, col_end)
        for (i, &col_start) in columns.iter().enumerate() {
            let col_end = columns.get(i + 1).copied().unwrap_or(right_x + 100.0);

            // Span belongs to this column if its X is >= col_start and < col_end
            // Allow some tolerance (10pt) for spans slightly before column start
            if span_x >= col_start - 10.0 && span_x < col_end - 10.0 {
                return i;
            }
        }

        // If no exact match, find the closest column
        let mut min_dist = f32::MAX;
        let mut closest_col = 0;

        for (i, &col_start) in columns.iter().enumerate() {
            let dist = (span_x - col_start).abs();
            if dist < min_dist {
                min_dist = dist;
                closest_col = i;
            }
        }

        closest_col
    }

    /// Check if detected table rows actually represent a numbered or bulleted list.
    ///
    /// When a PDF has a numbered list like "1. Item", the number and text often
    /// become separate spans at different X positions, which looks like a multi-column
    /// table to the detector. This method catches that false positive.
    fn is_list_pattern(&self, rows: &[TableRowData], columns: &[f32]) -> bool {
        if columns.len() < 2 || rows.is_empty() {
            return false;
        }

        let mut bullet_count = 0;
        let mut number_count = 0;

        for row in rows {
            if row.spans.is_empty() {
                continue;
            }

            // Check the leftmost span in this row
            let first_span = row
                .spans
                .iter()
                .min_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

            if let Some(span) = first_span {
                let text = span.text.trim();
                if is_bullet_marker(text) {
                    bullet_count += 1;
                } else if is_number_marker(text) {
                    number_count += 1;
                }
            }
        }

        let bullet_ratio = bullet_count as f32 / rows.len() as f32;
        let total_ratio = (bullet_count + number_count) as f32 / rows.len() as f32;
        log::debug!(
            "TableDetector: list markers: bullets={}, numbers={}, total rows={}, bullet_ratio={:.2}, total_ratio={:.2}",
            bullet_count,
            number_count,
            rows.len(),
            bullet_ratio,
            total_ratio
        );

        // Bullet markers (•, -, etc.) are almost never real table data
        if bullet_ratio >= 0.5 {
            return true;
        }

        // For numbered markers, only reject 2-column tables to avoid
        // false-negatives on real tables with numbered first columns
        if columns.len() == 2 && total_ratio >= 0.5 {
            return true;
        }

        false
    }
}


/// Check if text is a bullet marker (•, -, etc.).
fn is_bullet_marker(text: &str) -> bool {
    let trimmed = text.trim();
    matches!(
        trimmed,
        "-" | "–" | "—" | "•" | "·" | "*" | "○" | "▪" | "◦" | "▸" | "▹" | "►" | "■" | "●" | "※" | "□" | "◆" | "◇" | "▶" | "▷" | "☞" | "➤" | "➜"
    )
}

/// Check if text is a number-style list marker (1., 2), a., etc.).
fn is_number_marker(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Remove internal whitespace for pattern matching (handles "1 .")
    let cleaned: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();

    // Numbered markers: digits followed by "." or ")" — e.g., "1.", "12.", "1)"
    if let Some(pos) = cleaned.find(|c: char| !c.is_ascii_digit()) {
        let prefix = &cleaned[..pos];
        let suffix = &cleaned[pos..];
        if !prefix.is_empty() && (suffix == "." || suffix == ")") {
            return true;
        }
    }

    // Just a bare number
    if cleaned.parse::<u32>().is_ok() {
        return true;
    }

    // Letter marker: "a.", "B)"
    if cleaned.len() == 2 {
        let chars: Vec<char> = cleaned.chars().collect();
        if chars[0].is_alphabetic() && (chars[1] == '.' || chars[1] == ')') {
            return true;
        }
    }

    false
}

/// Check if a text string looks like a list marker (number, bullet, etc.).
#[cfg(test)]
fn is_list_marker(text: &str) -> bool {
    is_bullet_marker(text) || is_number_marker(text)
}

impl Default for TableDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_span(text: &str, x: f32, y: f32) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            x,
            y,
            width: text.len() as f32 * 6.0, // Approximate width
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
        }
    }

    #[test]
    fn test_group_into_rows() {
        let detector = TableDetector::new();
        let spans = vec![
            make_span("A1", 10.0, 100.0),
            make_span("B1", 60.0, 100.0),
            make_span("A2", 10.0, 85.0),
            make_span("B2", 60.0, 85.0),
        ];

        let rows = detector.group_into_rows(&spans);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].spans.len(), 2);
        assert_eq!(rows[1].spans.len(), 2);
    }

    #[test]
    fn test_detect_columns() {
        let detector = TableDetector::new();
        let rows = vec![
            TableRowData {
                y: 100.0,
                spans: vec![make_span("A1", 10.0, 100.0), make_span("B1", 60.0, 100.0)],
            },
            TableRowData {
                y: 85.0,
                spans: vec![make_span("A2", 10.0, 85.0), make_span("B2", 60.0, 85.0)],
            },
            TableRowData {
                y: 70.0,
                spans: vec![make_span("A3", 10.0, 70.0), make_span("B3", 60.0, 70.0)],
            },
        ];

        let columns = detector.detect_columns(&rows);
        assert_eq!(columns.len(), 2);
    }

    #[test]
    fn test_detect_simple_table() {
        let detector = TableDetector::new();
        let spans = vec![
            // Header row
            make_span("Name", 10.0, 100.0),
            make_span("Age", 60.0, 100.0),
            // Data row 1
            make_span("Alice", 10.0, 85.0),
            make_span("30", 60.0, 85.0),
            // Data row 2
            make_span("Bob", 10.0, 70.0),
            make_span("25", 60.0, 70.0),
        ];

        let (tables, remaining) = detector.detect(spans);
        assert_eq!(tables.len(), 1);
        assert!(remaining.is_empty());

        let table = &tables[0];
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.columns.len(), 2);
    }

    #[test]
    fn test_no_table_single_column() {
        let detector = TableDetector::new();
        let spans = vec![
            make_span("Line 1", 10.0, 100.0),
            make_span("Line 2", 10.0, 85.0),
            make_span("Line 3", 10.0, 70.0),
        ];

        let (tables, remaining) = detector.detect(spans);
        assert!(tables.is_empty());
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn test_table_model_conversion() {
        let detector = TableDetector::new();
        let detected = DetectedTable {
            top_y: 100.0,
            bottom_y: 70.0,
            left_x: 10.0,
            right_x: 100.0,
            columns: vec![10.0, 60.0],
            rows: vec![
                TableRowData {
                    y: 100.0,
                    spans: vec![
                        make_span("Name", 10.0, 100.0),
                        make_span("Age", 60.0, 100.0),
                    ],
                },
                TableRowData {
                    y: 85.0,
                    spans: vec![make_span("Alice", 10.0, 85.0), make_span("30", 60.0, 85.0)],
                },
            ],
        };

        let table = detector.to_table_model(&detected);
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.column_count(), 2);
        assert_eq!(table.header_rows, 1);
    }

    #[test]
    fn test_numbered_list_not_detected_as_table() {
        let detector = TableDetector::new();
        // Simulates a numbered list where number and text are separate spans
        let spans = vec![
            make_span("1.", 50.0, 400.0),
            make_span("장비관리설정", 80.0, 400.0),
            make_span("2.", 50.0, 370.0),
            make_span("Object관리", 80.0, 370.0),
            make_span("3.", 50.0, 340.0),
            make_span("정책관리 및 라우팅", 80.0, 340.0),
            make_span("4.", 50.0, 310.0),
            make_span("VPN", 80.0, 310.0),
            make_span("5.", 50.0, 280.0),
            make_span("운영관리", 80.0, 280.0),
        ];

        let (tables, remaining) = detector.detect(spans);
        assert!(tables.is_empty(), "Numbered list should not be detected as a table");
        assert_eq!(remaining.len(), 10);
    }

    #[test]
    fn test_bullet_list_not_detected_as_table() {
        let detector = TableDetector::new();
        // Simulates a bullet list with "-" markers
        let spans = vec![
            make_span("-", 50.0, 400.0),
            make_span("Management", 80.0, 400.0),
            make_span("-", 50.0, 370.0),
            make_span("Interface/Service Option", 80.0, 370.0),
            make_span("-", 50.0, 340.0),
            make_span("Firmware", 80.0, 340.0),
        ];

        let (tables, remaining) = detector.detect(spans);
        assert!(tables.is_empty(), "Bullet list should not be detected as a table");
        assert_eq!(remaining.len(), 6);
    }

    #[test]
    fn test_is_list_marker() {
        // Numbered markers
        assert!(is_list_marker("1."));
        assert!(is_list_marker("12."));
        assert!(is_list_marker("1)"));
        assert!(is_list_marker("1 ."));  // with space
        assert!(is_list_marker("3"));    // bare number

        // Bullet markers
        assert!(is_list_marker("-"));
        assert!(is_list_marker("•"));
        assert!(is_list_marker("*"));
        assert!(is_list_marker("–"));

        // Letter markers
        assert!(is_list_marker("a."));
        assert!(is_list_marker("B)"));

        // Not markers
        assert!(!is_list_marker("Name"));
        assert!(!is_list_marker("Hello World"));
        assert!(!is_list_marker("Alice"));
        assert!(!is_list_marker(""));
    }
}
