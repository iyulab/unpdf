//! Table detection using text position analysis (Stream mode algorithm).
//!
//! Inspired by Camelot's Stream mode, this module detects tables by analyzing
//! text alignment patterns without relying on graphical lines.

use std::collections::BTreeMap;

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
    /// Confidence score (0.0 - 1.0) for this table detection
    pub confidence: f32,
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
            max_columns: 10,
            y_tolerance_factor: 0.4,
            min_alignment_ratio: 0.3,
            min_column_gap: 20.0, // Increased from 15 to prevent splitting within cells
        }
    }
}

/// Check if any span in the slice contains CJK characters.
fn has_cjk_text(spans: &[TextSpan]) -> bool {
    spans.iter().any(|s| {
        s.text.chars().any(|c| {
            matches!(c,
                // CJK Radicals Supplement through CJK Unified Ideographs (covers CJK, Kana, Bopomofo, etc.)
                '\u{2E80}'..='\u{9FFF}' |
                // CJK Compatibility Ideographs
                '\u{F900}'..='\u{FAFF}' |
                // Hangul Syllables
                '\u{AC00}'..='\u{D7AF}' |
                // Halfwidth and Fullwidth Forms
                '\u{FF00}'..='\u{FFEF}'
            )
        })
    })
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

    /// Return the effective minimum column gap, adjusted upward for CJK text.
    ///
    /// CJK characters are fullwidth (~font_size wide), so gaps between characters
    /// within a single cell can look like column separators with the default threshold.
    fn effective_min_column_gap(&self, spans: &[TextSpan]) -> f32 {
        if has_cjk_text(spans) {
            let median_font = if spans.is_empty() {
                12.0
            } else {
                let mut sizes: Vec<f32> = spans.iter().map(|s| s.font_size).collect();
                sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                sizes[sizes.len() / 2]
            };
            // CJK chars are fullwidth (~font_size wide), so require larger gaps
            (median_font * 1.5).max(self.config.min_column_gap)
        } else {
            self.config.min_column_gap
        }
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

        // Step 3: Find table regions — contiguous rows aligned with the page-wide
        // columns, plus runs of multi-span rows those columns miss. A table that fills
        // a small part of the page never contributes enough edges to page-wide
        // columns; its own rows are judged on their own columns in step 4.
        let mut table_regions = if columns.len() >= self.config.min_columns {
            self.find_table_regions(&rows, &columns)
        } else {
            log::debug!(
                "TableDetector: not enough page-wide columns ({} < {})",
                columns.len(),
                self.config.min_columns
            );
            Vec::new()
        };
        let mut local_regions = std::collections::HashSet::new();
        for local in self.find_multi_span_runs(&rows) {
            let overlaps = table_regions
                .iter()
                .any(|&(s, e)| local.0 <= e && s <= local.1);
            if !overlaps {
                table_regions.push(local);
                local_regions.insert(local);
            }
        }
        table_regions.sort_unstable();
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
                    log::debug!("TableDetector: skipping region — detected as list pattern");
                    continue;
                }

                // Reject 2-column page layouts that look like tables.
                // A 2-column document layout has many rows and each row's text
                // approaches the full column width, whereas a 2-column table
                // has shorter cell content.
                if Self::is_multicolumn_layout(&table_rows, &table_columns, right_x) {
                    log::debug!("TableDetector: skipping region — looks like 2-column page layout");
                    continue;
                }

                // Reject text columns set side by side: their lines run on into
                // the next row, which a table cell does not do.
                let cells = Self::column_texts(&table_rows, &table_columns);

                // A region found only as a run of multi-span rows has no page-wide
                // alignment behind it, so it must look like a grid on its own.
                if local_regions.contains(&(start_row, end_row)) && !Self::reads_as_grid(&cells) {
                    log::debug!("TableDetector: skipping local region — not grid-like");
                    continue;
                }

                if Self::columns_read_as_prose(&cells) {
                    log::debug!("TableDetector: skipping region — columns flow as prose");
                    continue;
                }

                // Reject a table of contents: entries against page numbers that
                // never go down.
                if Self::is_table_of_contents(&cells) {
                    log::debug!("TableDetector: skipping region — table of contents");
                    continue;
                }

                // Reject sparse tables: if any column is occupied by fewer than
                // 25% of rows (and the region has > 5 rows), the structure is
                // likely a single text column with occasional indented spans,
                // not a real table.
                if Self::is_sparse_misdetection(&table_rows, &table_columns) {
                    log::debug!("TableDetector: skipping region — sparse column occupancy");
                    continue;
                }

                // Compute confidence before marking spans as used
                let confidence = Self::table_confidence(&table_rows, table_columns.len());
                log::debug!(
                    "TableDetector: region [{start_row}..{end_row}] confidence={:.2}",
                    confidence
                );

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
                    confidence,
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
        let mut edge_counts: BTreeMap<i32, usize> = BTreeMap::new();
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

        // Merge close edges — use a CJK-aware gap threshold
        let all_spans: Vec<TextSpan> = rows.iter().flat_map(|r| r.spans.iter().cloned()).collect();
        let min_gap = self.effective_min_column_gap(&all_spans);
        let mut merged_edges: Vec<f32> = Vec::new();
        for edge in column_edges {
            if merged_edges.is_empty() {
                merged_edges.push(edge);
            } else {
                let last = *merged_edges.last().unwrap();
                if edge - last >= min_gap {
                    merged_edges.push(edge);
                }
            }
        }

        log::debug!("TableDetector: merged column edges = {:?}", merged_edges);

        Self::keep_whitespace_channels(rows, merged_edges)
    }

    /// Simpler column detection for when few rows have multiple spans.
    fn detect_columns_simple(&self, rows: &[TableRowData]) -> Vec<f32> {
        if rows.is_empty() {
            return vec![];
        }

        let mut edge_counts: BTreeMap<i32, usize> = BTreeMap::new();
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

        let all_spans: Vec<TextSpan> = rows.iter().flat_map(|r| r.spans.iter().cloned()).collect();
        let min_gap = self.effective_min_column_gap(&all_spans);
        let mut merged_edges: Vec<f32> = Vec::new();
        for edge in column_edges {
            if merged_edges.is_empty() {
                merged_edges.push(edge);
            } else {
                let last = *merged_edges.last().unwrap();
                if edge - last >= min_gap {
                    merged_edges.push(edge);
                }
            }
        }

        Self::keep_whitespace_channels(rows, merged_edges)
    }

    /// Keep only the edges no row's text runs across.
    ///
    /// Text starting at the same x on several rows is not yet a column: in justified
    /// prose, runs broken at word or style boundaries line up by chance. A column
    /// boundary is a vertical channel of whitespace — the stream-mode criterion of
    /// Tabula and Camelot — so an edge that a row's run crosses is not one. A single
    /// crossing row is tolerated (a spanning title or a merged cell); prose, where
    /// nearly every row runs through, is not. Runs without a measured or estimated
    /// width cannot cross anything, which keeps the previous behaviour for them.
    ///
    /// A header is the other place a table's text crosses its channels: group labels
    /// span the columns they head, and a header set over several lines (or with
    /// subscripts, which form rows of their own) crosses on each of them. So the rows
    /// of a crossing run at the very top — at most [`MAX_HEADER_ROWS`], and fewer than
    /// half the rows — are not counted, as long as rows below them keep the channel.
    fn keep_whitespace_channels(rows: &[TableRowData], edges: Vec<f32>) -> Vec<f32> {
        // Edges are bucketed to 5 pt, so a run starting at an edge may sit up to 2.5 pt
        // either side of it; only a run clearly through the edge counts as crossing.
        const MARGIN: f32 = 3.0;
        const MAX_HEADER_ROWS: usize = 4;
        let crosses = |row: &TableRowData, edge: f32| {
            row.spans
                .iter()
                .any(|s| s.width > 0.0 && s.x < edge - MARGIN && s.x + s.width > edge + MARGIN)
        };
        let header_rows = rows
            .iter()
            .take_while(|row| edges.iter().any(|&edge| crosses(row, edge)))
            .count();
        let header_rows = if header_rows <= MAX_HEADER_ROWS && header_rows * 2 < rows.len() {
            header_rows
        } else {
            0
        };
        let body = &rows[header_rows..];
        let allowed = (body.len() / 10).max(1);
        edges
            .iter()
            .copied()
            .filter(|&edge| body.iter().filter(|row| crosses(row, edge)).count() <= allowed)
            .collect()
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

    /// Runs of at least `min_rows` consecutive rows that each hold two or more spans —
    /// the rows a table occupies, whatever the rest of the page looks like.
    fn find_multi_span_runs(&self, rows: &[TableRowData]) -> Vec<(usize, usize)> {
        let mut runs = Vec::new();
        let mut start: Option<usize> = None;
        for (i, row) in rows.iter().enumerate() {
            if row.spans.len() >= self.config.min_columns {
                start.get_or_insert(i);
            } else if let Some(s) = start.take() {
                if i - s >= self.config.min_rows {
                    runs.push((s, i - 1));
                }
            }
        }
        if let Some(s) = start {
            if rows.len() - s >= self.config.min_rows {
                runs.push((s, rows.len() - 1));
            }
        }
        runs
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

    /// Compute confidence score (0.0 - 1.0) for a detected table.
    fn table_confidence(rows: &[TableRowData], num_columns: usize) -> f32 {
        if rows.is_empty() || num_columns < 2 {
            return 0.0;
        }

        let mut score = 1.0_f32;

        // Penalize very few rows
        if rows.len() < 3 {
            score *= 0.7;
        }

        // Penalize excessive columns (likely false detection)
        if num_columns > 6 {
            score *= 0.6;
        }
        if num_columns > 8 {
            score *= 0.5;
        }

        // Check column occupancy: how many cells are actually filled
        let total_cells = rows.len() * num_columns;
        let filled_cells: usize = rows.iter().map(|r| r.spans.len().min(num_columns)).sum();
        let occupancy = filled_cells as f32 / total_cells as f32;
        if occupancy < 0.3 {
            score *= 0.5; // Sparse table is likely not a real table
        }

        score.clamp(0.0, 1.0)
    }

    /// Detect sparse misdetections — regions where the table column structure is
    /// suspiciously imbalanced. Real tables tend to have most columns filled by
    /// most rows; if one or more columns are nearly always empty, the detector
    /// likely picked up an indented continuation line as a "second column".
    fn is_sparse_misdetection(rows: &[TableRowData], columns: &[f32]) -> bool {
        if rows.len() <= 5 || columns.len() < 2 {
            return false;
        }

        let mut col_occupancy = vec![0usize; columns.len()];
        for row in rows {
            for span in &row.spans {
                let mut col_idx = 0;
                for (i, &cs) in columns.iter().enumerate() {
                    if span.x >= cs - 5.0 {
                        col_idx = i;
                    } else {
                        break;
                    }
                }
                col_occupancy[col_idx] += 1;
            }
        }

        let row_count = rows.len();
        let min_required = (row_count as f32 * 0.25).ceil() as usize;
        let any_sparse = col_occupancy.iter().any(|c| *c < min_required);
        log::debug!(
            "TableDetector: sparse check rows={} cols={} occupancy={:?} min_required={} sparse={}",
            row_count,
            columns.len(),
            col_occupancy,
            min_required,
            any_sparse
        );
        any_sparse
    }

    /// The text of each row, split into the region's columns (same assignment rule as
    /// [`Self::is_sparse_misdetection`]). Runs that land in one cell are joined by a space.
    fn column_texts(rows: &[TableRowData], columns: &[f32]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| {
                let mut cells = vec![String::new(); columns.len()];
                for span in &row.spans {
                    let col = columns
                        .iter()
                        .rposition(|&cs| span.x >= cs - 5.0)
                        .unwrap_or(0);
                    let text = span.text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    if !cells[col].is_empty() {
                        cells[col].push(' ');
                    }
                    cells[col].push_str(text);
                }
                cells
            })
            .collect()
    }

    /// Whether cells look like a grid's: at least three rows, three quarters of the
    /// cells filled, and short cells — at most four words on average. Measured on
    /// born-digital pages, tables found this way average one to three and a half words
    /// a cell; the text columns, reference lists and diagram labels that also come as
    /// runs of multi-span rows average five to eleven, or leave a third of their cells
    /// empty.
    fn reads_as_grid(cells: &[Vec<String>]) -> bool {
        let total = cells.iter().map(Vec::len).sum::<usize>();
        let filled: Vec<&String> = cells.iter().flatten().filter(|c| !c.is_empty()).collect();
        if cells.len() < 3 || total == 0 || filled.len() * 4 < total * 3 {
            return false;
        }
        let words: usize = filled.iter().map(|c| c.split_whitespace().count()).sum();
        words <= filled.len() * 4
    }

    /// Whether the region's columns read as running text rather than as cells.
    ///
    /// For each pair of vertically adjacent non-empty cells in a column, the upper one
    /// *runs on* into the lower one when it ends in a hyphen, or ends without closing
    /// punctuation while the lower one starts in lowercase. A table cell is complete in
    /// itself; a line of a text column is not. Measured on born-digital pages, real
    /// tables stay at or below a third of their pairs running on, and text columns laid
    /// side by side sit above half.
    fn columns_read_as_prose(cells: &[Vec<String>]) -> bool {
        let num_columns = cells.first().map_or(0, Vec::len);
        let mut pairs = 0usize;
        let mut run_on = 0usize;
        for col in 0..num_columns {
            for window in cells.windows(2) {
                let (upper, lower) = (window[0][col].as_str(), window[1][col].as_str());
                if upper.is_empty() || lower.is_empty() {
                    continue;
                }
                pairs += 1;
                let ends_closed = upper.ends_with(['.', ':', ';', '!', '?', ')', ']']);
                let starts_lower = lower.chars().next().is_some_and(char::is_lowercase);
                if upper.ends_with('-') || (!ends_closed && starts_lower) {
                    run_on += 1;
                }
            }
        }
        pairs >= 2 && run_on * 2 > pairs
    }

    /// Whether the region is a table of contents: the last column holds page
    /// references — arabic numbers that never decrease, or roman numerals for front
    /// matter — on most rows, and the entries before them are text.
    fn is_table_of_contents(cells: &[Vec<String>]) -> bool {
        let Some(last) = cells.first().map(|r| r.len().saturating_sub(1)) else {
            return false;
        };
        if last == 0 {
            return false;
        }
        let is_roman = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| matches!(c.to_ascii_lowercase(), 'i' | 'v' | 'x' | 'l' | 'c'))
        };
        let mut pages: Vec<u32> = Vec::new();
        let mut referenced_rows = 0usize;
        for row in cells {
            let page = row[last].as_str();
            if page.is_empty() {
                continue;
            }
            if let Ok(n) = page.parse::<u32>() {
                pages.push(n);
            } else if !is_roman(page) {
                return false;
            }
            if row[..last].iter().all(String::is_empty) {
                return false;
            }
            referenced_rows += 1;
        }
        pages.len() >= 3
            && referenced_rows * 4 >= cells.len() * 3
            && pages.windows(2).all(|w| w[0] <= w[1])
    }

    /// Check if the detected table actually represents a multi-column page layout.
    ///
    /// A 2-column document layout (newspaper, academic paper) presents at the span
    /// level identically to a 2-column table: both have spans aligned to two X edges.
    /// The distinguishing signal is *fill ratio*: in a 2-col page layout each row's
    /// text approaches the full column width, while table cells contain short content.
    ///
    /// Heuristic (all must hold to classify as page layout):
    ///   1. exactly 2 columns
    ///   2. row count is large (> 12)
    ///   3. estimated text width per cell ≥ 60% of the column width on average
    fn is_multicolumn_layout(rows: &[TableRowData], columns: &[f32], _right_x: f32) -> bool {
        // Need at least 2 columns and a substantial number of rows (page-scale extent).
        if columns.len() < 2 || rows.len() < 8 {
            return false;
        }

        let all_spans: Vec<TextSpan> = rows.iter().flat_map(|r| r.spans.iter().cloned()).collect();
        let cjk = has_cjk_text(&all_spans);
        let factor = if cjk { 1.0 } else { 0.55 };

        // Compute the actual right extent using estimated span widths
        // (TextSpan.width is often 0.0 — fall back to char-based estimate).
        let est_span_right = |span: &TextSpan| -> f32 {
            let w = if span.width > 0.0 {
                span.width
            } else {
                span.text.chars().count() as f32 * span.font_size * factor
            };
            span.x + w
        };
        let right_extent = all_spans.iter().map(est_span_right).fold(0.0_f32, f32::max);

        // Compute per-column widths
        let mut col_widths: Vec<f32> = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            let w = if i + 1 < columns.len() {
                columns[i + 1] - columns[i]
            } else {
                right_extent - columns[i]
            };
            col_widths.push(w);
        }
        log::debug!(
            "TableDetector: multicolumn entered rows={} cols={} right_extent={:.0} col_widths={:?}",
            rows.len(),
            columns.len(),
            right_extent,
            col_widths
        );

        // For each column, accumulate fill ratios from rows that have content there.
        let mut per_col_ratios: Vec<Vec<f32>> = vec![Vec::new(); columns.len()];

        for row in rows {
            // Per-column max span width on this row
            let mut per_col_max = vec![0.0f32; columns.len()];
            let mut per_col_chars = vec![0usize; columns.len()];

            for span in &row.spans {
                // Find the column this span belongs to (closest col_start <= span.x)
                let mut col_idx = 0;
                for (i, &cs) in columns.iter().enumerate() {
                    if span.x >= cs - 5.0 {
                        col_idx = i;
                    } else {
                        break;
                    }
                }
                let w_est = if span.width > 0.0 {
                    span.width
                } else {
                    span.text.chars().count() as f32 * span.font_size * factor
                };
                per_col_chars[col_idx] += span.text.chars().count();
                if w_est > per_col_max[col_idx] {
                    per_col_max[col_idx] = w_est;
                }
            }

            for (i, chars) in per_col_chars.iter().enumerate() {
                if *chars > 0 {
                    per_col_ratios[i].push((per_col_max[i] / col_widths[i]).min(2.0));
                }
            }
        }

        // Count columns whose used rows have high average fill (page-layout-like).
        let avg = |v: &[f32]| -> f32 {
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f32>() / v.len() as f32
            }
        };
        let mut layout_cols = 0usize;
        for (i, ratios) in per_col_ratios.iter().enumerate() {
            let a = avg(ratios);
            log::debug!(
                "TableDetector: multicolumn col {} — width={:.0} rows_with_text={}, avg_fill={:.2}",
                i,
                col_widths[i],
                ratios.len(),
                a
            );
            // Narrow columns (<80pt) are likely indent edges, not real column starts.
            // Skip them but don't disqualify the layout as a whole.
            if col_widths[i] < 80.0 {
                continue;
            }
            if ratios.len() >= 4 && a >= 0.6 {
                layout_cols += 1;
            }
        }

        // ≥ 2 columns each filled tightly across many rows ⇒ page layout.
        let is_layout = layout_cols >= 2;
        log::debug!(
            "TableDetector: multicolumn check rows={} cols={} layout_cols={} => {}",
            rows.len(),
            columns.len(),
            layout_cols,
            if is_layout {
                "REJECT-AS-LAYOUT"
            } else {
                "keep-as-table"
            }
        );

        is_layout
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
        "-" | "–"
            | "—"
            | "•"
            | "·"
            | "*"
            | "○"
            | "▪"
            | "◦"
            | "▸"
            | "▹"
            | "►"
            | "■"
            | "●"
            | "※"
            | "□"
            | "◆"
            | "◇"
            | "▶"
            | "▷"
            | "☞"
            | "➤"
            | "➜"
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
    // Use chars().count() instead of len() — len() counts bytes, not characters,
    // so a single multi-byte UTF-8 char (e.g. 'α' = 2 bytes) would pass len()==2
    // but produce only 1 element in the chars vec, causing index-out-of-bounds.
    let chars: Vec<char> = cleaned.chars().collect();
    if chars.len() == 2 && chars[0].is_alphabetic() && (chars[1] == '.' || chars[1] == ')') {
        return true;
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
            confidence: 1.0,
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
        assert!(
            tables.is_empty(),
            "Numbered list should not be detected as a table"
        );
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
        assert!(
            tables.is_empty(),
            "Bullet list should not be detected as a table"
        );
        assert_eq!(remaining.len(), 6);
    }

    fn make_span_w(text: &str, x: f32, y: f32, font_size: f32) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            x,
            y,
            width: 0.0,
            font_size,
            font_name: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
        }
    }

    fn measured(text: &str, x: f32, y: f32, width: f32) -> TextSpan {
        TextSpan {
            width,
            ..make_span_w(text, x, y, 12.0)
        }
    }

    #[test]
    fn justified_prose_whose_runs_line_up_by_chance_is_not_a_table() {
        // Six justified lines, each broken into runs at word boundaries. Three rows
        // happen to start a run at x=200 and three at x=320 — but on every other row a
        // run passes straight through those x positions, so there is no whitespace
        // channel there and no column.
        let detector = TableDetector::new();
        let mut spans = Vec::new();
        for i in 0..6 {
            let y = 700.0 - i as f32 * 14.0;
            if i % 2 == 0 {
                spans.push(measured("the plastids are surrounded", 72.0, y, 124.0));
                spans.push(measured("by three membranes and in", 200.0, y, 116.0));
                spans.push(measured("the remaining lines by four", 320.0, y, 120.0));
            } else {
                spans.push(measured(
                    "a nucleomorph, remnants of the original",
                    72.0,
                    y,
                    180.0,
                ));
                spans.push(measured(
                    "algal nucleus located between the",
                    256.0,
                    y,
                    180.0,
                ));
            }
        }
        let (tables, remaining) = detector.detect(spans);
        assert!(
            tables.is_empty(),
            "prose must not become a table: {tables:?}"
        );
        assert_eq!(remaining.len(), 15);
    }

    #[test]
    fn a_table_with_one_spanning_row_keeps_its_columns() {
        // A real three-column table whose title row spans every column: one crossing
        // row is tolerated.
        let detector = TableDetector::new();
        let mut spans = vec![measured(
            "Timescale of photosynthesis stages",
            72.0,
            720.0,
            300.0,
        )];
        for i in 0..5 {
            let y = 700.0 - i as f32 * 14.0;
            spans.push(measured("Stage", 72.0, y, 30.0));
            spans.push(measured("Event", 200.0, y, 30.0));
            spans.push(measured("Site", 320.0, y, 24.0));
        }
        let (tables, _) = detector.detect(spans);
        assert_eq!(tables.len(), 1, "the table must still be found");
        assert!(
            tables[0].columns.len() >= 3,
            "columns: {:?}",
            tables[0].columns
        );
    }

    #[test]
    fn test_two_column_layout_not_detected_as_table() {
        let detector = TableDetector::new();
        // Simulate a 2-column page layout: ~80-char lines in each column,
        // 20 rows. Should NOT be detected as a table.
        let mut spans = Vec::new();
        let line_text = "Lorem ipsum dolor sit amet consectetuer adipiscing elit ut purus elit"; // ~70 chars
        for i in 0..20 {
            let y = 700.0 - (i as f32) * 14.0;
            spans.push(make_span_w(line_text, 70.0, y, 11.0));
            spans.push(make_span_w(line_text, 320.0, y, 11.0));
        }
        let (tables, _remaining) = detector.detect(spans);
        assert!(
            tables.is_empty(),
            "Two-column layout should not be detected as a table, got {} tables",
            tables.len()
        );
    }

    /// Two text columns of a page set side by side: each column's lines run on into
    /// the next row (no closing punctuation, next line starts lowercase or the line
    /// ends in a hyphen). Table cells are independent of the cell below them.
    #[test]
    fn side_by_side_prose_columns_are_not_a_table() {
        let detector = TableDetector::new();
        let left = [
            "the model is first trained on",
            "a large corpus of web text and",
            "then adapted to the target do-",
            "main with a smaller learning",
            "rate, which keeps the general",
            "knowledge it acquired earlier.",
        ];
        let right = [
            "whereas the second approach",
            "starts from scratch and relies",
            "on curated data alone; it is",
            "slower but avoids inheriting",
            "the biases of the larger cor-",
            "pus that the first one uses.",
        ];
        let mut spans = Vec::new();
        for i in 0..6 {
            let y = 700.0 - i as f32 * 14.0;
            spans.push(measured(left[i], 72.0, y, 150.0));
            spans.push(measured(right[i], 320.0, y, 150.0));
        }
        let (tables, remaining) = detector.detect(spans);
        assert!(
            tables.is_empty(),
            "prose columns must not become a table: {tables:?}"
        );
        assert_eq!(remaining.len(), 12);
    }

    /// A table of contents lines entries up against page numbers, but it is a list of
    /// references in reading order, not a grid: its page column never goes down.
    #[test]
    fn table_of_contents_is_not_a_table() {
        let detector = TableDetector::new();
        let entries = [
            ("Executive Summary", "4"),
            ("Legal Framework", "6"),
            ("Election Administration", "11"),
            ("Campaign", ""),
            ("Media Freedom", "25"),
            ("Recommendations", "39"),
        ];
        let mut spans = Vec::new();
        for (i, (title, page)) in entries.iter().enumerate() {
            let y = 700.0 - i as f32 * 14.0;
            spans.push(measured(title, 72.0, y, 120.0));
            if !page.is_empty() {
                spans.push(measured(page, 400.0, y, 10.0));
            }
        }
        let (tables, _) = detector.detect(spans);
        assert!(
            tables.is_empty(),
            "a table of contents is not a table: {tables:?}"
        );
    }

    /// A small table on a page of single-column text: its column edges recur on only a
    /// handful of the page's rows, too few to count as page-wide columns, but within
    /// its own rows they are on every one.
    #[test]
    fn a_small_table_among_body_text_is_found() {
        let detector = TableDetector::new();
        let mut spans = Vec::new();
        let mut y = 760.0;
        for i in 0..14 {
            spans.push(measured(
                &format!("Body line {i} of the procedure that runs across the page"),
                72.0,
                y,
                400.0,
            ));
            y -= 14.0;
        }
        for row in [
            ["Tube", "Water", "Glucose", "Yeast"],
            ["1", "8 ml", "6 ml", "0 ml"],
            ["2", "12 ml", "0 ml", "2 ml"],
            ["3", "6 ml", "6 ml", "2 ml"],
            ["4", "2 ml", "6 ml", "6 ml"],
        ] {
            for (j, cell) in row.iter().enumerate() {
                spans.push(measured(cell, 72.0 + j as f32 * 110.0, y, 30.0));
            }
            y -= 14.0;
        }
        for i in 0..6 {
            spans.push(measured(
                &format!("More body text {i} after the table"),
                72.0,
                y,
                400.0,
            ));
            y -= 14.0;
        }
        let (tables, _) = detector.detect(spans);
        assert_eq!(tables.len(), 1, "{tables:?}");
        assert_eq!(tables[0].columns.len(), 4);
        assert_eq!(tables[0].rows.len(), 5);
    }

    /// A header set over two lines whose group labels span several columns crosses
    /// the column channels — on its own rows only; the body below keeps them clear.
    /// Coordinates are a real page's: the header's subscripts (`r` with `t` below it)
    /// each make a row of their own, so more than one row crosses each channel.
    #[test]
    fn a_multi_line_spanning_header_keeps_the_body_columns() {
        let detector = TableDetector::new();
        let sized = |text: &str, x: f32, y: f32, width: f32, size: f32| TextSpan {
            width,
            ..make_span_w(text, x, y, size)
        };
        let mut spans = vec![
            sized("Observed returns on the firm's ", 112.6, 649.2, 140.3, 9.0),
            sized(
                "Observed returns on a potential new investment ",
                277.8,
                649.2,
                221.1,
                9.0,
            ),
            sized("Time ", 58.0, 644.0, 24.8, 9.0),
            sized("t ", 82.9, 644.0, 6.0, 9.0),
            sized("p ", 225.2, 643.1, 6.8, 7.2),
            sized("j ", 419.3, 643.1, 3.9, 7.2),
            sized("portfolio over time ", 133.0, 638.8, 85.7, 9.0),
            sized("r", 218.7, 638.8, 3.6, 9.0),
            sized("for the firm's ", 352.9, 638.8, 59.9, 9.0),
            sized("r", 412.8, 638.8, 3.6, 9.0),
            sized("t", 222.3, 637.6, 2.9, 7.2),
            sized("t", 416.4, 637.6, 2.9, 7.2),
        ];
        for (i, (year, a, b)) in [
            ("2012 ", "10% ", "7% "),
            ("2013 ", "6% ", "8% "),
            ("2014 ", "7% ", "5% "),
            ("2015 ", "3% ", "2% "),
            ("2016 ", "5% ", "3% "),
        ]
        .iter()
        .enumerate()
        {
            let y = 620.8 - i as f32 * 18.0;
            spans.push(sized(year, 58.0, y, 21.7, 9.0));
            spans.push(sized(a, 175.2, y, 15.1, 9.0));
            spans.push(sized(b, 380.8, y, 15.0, 9.0));
        }
        let (tables, _) = detector.detect(spans);
        assert_eq!(tables.len(), 1, "{tables:?}");
        assert_eq!(tables[0].columns.len(), 3, "{:?}", tables[0].columns);
    }

    /// Two columns of references side by side also come as a run of two-span rows, and
    /// their entries close with a period, so they do not read as running text — but
    /// their cells are far longer than a grid's.
    #[test]
    fn side_by_side_reference_entries_are_not_a_table() {
        let detector = TableDetector::new();
        let mut spans = Vec::new();
        for i in 0..14 {
            spans.push(measured(
                &format!("Body line {i} of the discussion that runs across the page"),
                72.0,
                760.0 - i as f32 * 14.0,
                440.0,
            ));
        }
        let left = [
            "Jack Rae, Sebastian Borgeaud, Trevor Cai.",
            "Millican Jordan Hoffmann and Francis Song.",
            "Scaling language models: methods and analysis.",
            "Alec Radford, Jeffrey Wu, Rewon Child et al.",
        ];
        let right = [
            "Nazneen Rajani, Kashif Rasul, Younes Belkada.",
            "Shengyi Huang, Leandro von Werra et al.",
            "Zephyr: direct distillation of alignment.",
            "Hugo Touvron, Louis Martin, Kevin Stone et al.",
        ];
        for i in 0..4 {
            let y = 550.0 - i as f32 * 14.0;
            spans.push(measured(left[i], 72.0, y, 210.0));
            spans.push(measured(right[i], 300.0, y, 210.0));
        }
        let (tables, _) = detector.detect(spans);
        assert!(tables.is_empty(), "{tables:?}");
    }

    /// A count column that happens to be sorted is still data when it descends.
    #[test]
    fn a_ranked_count_table_stays_a_table() {
        let detector = TableDetector::new();
        let rows = [
            ("Alpha", "17,266"),
            ("Beta", "9,835"),
            ("Gamma", "711"),
            ("Delta", "46"),
        ];
        let mut spans = Vec::new();
        for (i, (name, count)) in rows.iter().enumerate() {
            let y = 700.0 - i as f32 * 14.0;
            spans.push(measured(name, 72.0, y, 40.0));
            spans.push(measured(count, 300.0, y, 30.0));
        }
        let (tables, _) = detector.detect(spans);
        assert_eq!(tables.len(), 1);
    }

    #[test]
    fn test_real_two_column_table_still_detected() {
        let detector = TableDetector::new();
        // Real 2-col table: short cell content, balanced occupancy
        let spans = vec![
            make_span_w("Name", 50.0, 100.0, 11.0),
            make_span_w("Age", 200.0, 100.0, 11.0),
            make_span_w("Alice", 50.0, 85.0, 11.0),
            make_span_w("30", 200.0, 85.0, 11.0),
            make_span_w("Bob", 50.0, 70.0, 11.0),
            make_span_w("25", 200.0, 70.0, 11.0),
            make_span_w("Carol", 50.0, 55.0, 11.0),
            make_span_w("40", 200.0, 55.0, 11.0),
        ];
        let (tables, _remaining) = detector.detect(spans);
        assert_eq!(tables.len(), 1, "Real 2-col table should still be detected");
    }

    #[test]
    fn test_sparse_misdetection_rejected() {
        let detector = TableDetector::new();
        // 10 rows with col_0 fully occupied, col_1 occupied only twice.
        // Should be rejected as sparse misdetection.
        let mut spans = Vec::new();
        for i in 0..10 {
            let y = 200.0 - (i as f32) * 14.0;
            spans.push(make_span_w("Some text content", 50.0, y, 11.0));
            if i == 2 || i == 7 {
                spans.push(make_span_w("note", 200.0, y, 11.0));
            }
        }
        let (tables, _remaining) = detector.detect(spans);
        assert!(
            tables.is_empty(),
            "Sparse 2-col misdetection should be rejected, got {} tables",
            tables.len()
        );
    }

    #[test]
    fn test_is_list_marker() {
        // Numbered markers
        assert!(is_list_marker("1."));
        assert!(is_list_marker("12."));
        assert!(is_list_marker("1)"));
        assert!(is_list_marker("1 .")); // with space
        assert!(is_list_marker("3")); // bare number

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
