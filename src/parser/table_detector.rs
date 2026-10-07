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
    /// Indices, into the spans the rows were grouped from, of every span this row
    /// holds — including marks folded into a span's text — so a table consumes
    /// exactly what it took, whatever became of the text.
    pub sources: Vec<usize>,
}

/// Which reading-order group each span falls in — XY-Cut over the spans a table detector
/// is given, the same partition the page's text is later read in.
struct ColumnContext {
    groups: Vec<Vec<usize>>,
    group_of: Vec<usize>,
    config: super::xycut::XyCutConfig,
}

impl ColumnContext {
    fn of(spans: &[TextSpan]) -> Self {
        let blocks = super::layout::xycut_blocks(spans);
        let config = super::layout::xycut_config(spans);
        let groups = super::xycut::xycut_partition(&blocks, &config).groups;
        let mut group_of = vec![0; spans.len()];
        for (g, members) in groups.iter().enumerate() {
            for &i in members {
                group_of[i] = g;
            }
        }
        ColumnContext {
            groups,
            group_of,
            config,
        }
    }

    /// The boundary between two reading flows that `rows` reach across, if they do: the
    /// rows draw on two reading-order groups or more, and those groups, taken whole, divide
    /// into two flows side by side — two columns of running text, or text and a strip of
    /// letters stacked in its margin ([`super::xycut::reading_boundary`]). A region read as
    /// two flows is never also taken for one table. A table XY-Cut splits at a wide channel
    /// between its own columns does not qualify: its groups hold cells, not lines of text.
    fn boundary_straddled_by(&self, rows: &[TableRowData], spans: &[TextSpan]) -> Option<f32> {
        let touched: std::collections::BTreeSet<usize> = rows
            .iter()
            .flat_map(|r| r.sources.iter())
            .map(|&i| self.group_of[i])
            .collect();
        if touched.len() < 2 {
            return None;
        }
        let members: Vec<TextSpan> = touched
            .iter()
            .flat_map(|&g| self.groups[g].iter().map(|&i| spans[i].clone()))
            .collect();
        let blocks = super::layout::xycut_blocks(&members);
        super::xycut::reading_boundary(&blocks, &self.config)
    }
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

        // The page's columns, as reading order will see them: a table region that draws
        // its rows from two columns of running text is judged against those whole columns,
        // not the few lines it happened to catch.
        let column_context = ColumnContext::of(&spans);

        // Step 4: Convert regions to detected tables
        let mut detected_tables = Vec::new();
        let mut used_span_indices: std::collections::HashSet<usize> =
            std::collections::HashSet::new();

        for (start_row, end_row) in table_regions {
            let table_rows: Vec<TableRowData> = rows[start_row..=end_row].to_vec();

            if table_rows.is_empty() {
                continue;
            }

            // Lines of running text are not rows of cells, however their words happen to
            // line up: a justified line stretches its word spaces past any fixed cell gap.
            if Self::rows_read_as_running_text(&table_rows) {
                log::debug!(
                    "TableDetector: skipping region [{start_row}..{end_row}] — its rows are lines of running text"
                );
                continue;
            }

            // Text set in two flows side by side is no grid, however its lines happen to line
            // up — the lines of one column fall between those of the other, a margin tab's
            // letters fall beside them, and a justified line's widened word gaps pass for cell
            // boundaries. When the region reaches across a boundary reading order splits flows
            // at, each side is searched for tables of its own.
            if let Some(boundary) = column_context.boundary_straddled_by(&table_rows, &spans) {
                log::debug!(
                    "TableDetector: region [{start_row}..{end_row}] reaches across a reading boundary at x={boundary:.0} — detecting each side on its own"
                );
                let sources: Vec<usize> = table_rows
                    .iter()
                    .flat_map(|r| r.sources.iter().copied())
                    .collect();
                for left in [true, false] {
                    let picks: Vec<usize> = sources
                        .iter()
                        .copied()
                        .filter(|&i| (spans[i].x + spans[i].width / 2.0 < boundary) == left)
                        .collect();
                    let (tables, _) =
                        self.detect(picks.iter().map(|&i| spans[i].clone()).collect());
                    for mut table in tables {
                        for row in &mut table.rows {
                            for source in &mut row.sources {
                                *source = picks[*source];
                            }
                            used_span_indices.extend(row.sources.iter().copied());
                        }
                        detected_tables.push(table);
                    }
                }
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

                // A table puts values side by side: its rows hold two cells or more.
                // Lines that each fill one column — the staggered lines of two text
                // columns, whose heights never pair up — make a region of one-cell rows.
                if Self::rows_with_several_cells(&cells) < self.config.min_rows {
                    log::debug!("TableDetector: skipping region — no row holds two cells");
                    continue;
                }

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

                // Mark spans as used — by the index each row carries, not by
                // matching text: a span whose marks were folded into it no longer
                // reads as it did on the page.
                for row in &table_rows {
                    used_span_indices.extend(row.sources.iter().copied());
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

    /// Whether most of `rows` that hold the region's main text are lines of running text —
    /// see [`is_running_text_row`]. Rows of smaller or larger print only (a margin tab's
    /// letters stacked beside the text, a chart's labels) say nothing either way.
    fn rows_read_as_running_text(rows: &[TableRowData]) -> bool {
        let mut sizes: Vec<f32> = rows
            .iter()
            .flat_map(|r| r.spans.iter().map(|s| s.font_size))
            .collect();
        sizes.sort_by(f32::total_cmp);
        let Some(&size) = sizes.get(sizes.len() / 2) else {
            return false;
        };
        let main: Vec<&TableRowData> = rows
            .iter()
            .filter(|r| {
                r.spans
                    .iter()
                    .any(|s| (s.font_size - size).abs() <= size * 0.25)
            })
            .collect();
        let running = main.iter().filter(|r| is_running_text_row(r)).count();
        running >= 2 && running * 2 > main.len()
    }

    /// Group spans into rows by Y position.
    fn group_into_rows(&self, spans: &[TextSpan]) -> Vec<TableRowData> {
        group_into_rows(spans, self.config.y_tolerance_factor)
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
            if separated_runs(row) >= self.config.min_columns {
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

    /// How many rows hold text in two cells or more.
    fn rows_with_several_cells(cells: &[Vec<String>]) -> usize {
        cells
            .iter()
            .filter(|row| row.iter().filter(|c| !c.trim().is_empty()).count() >= 2)
            .count()
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

/// Group spans into rows by baseline, top row first and left to right within a
/// row, with superscripts and subscripts kept in the row they mark
/// ([`attach_script_rows`]). Shared by stream-mode detection and by the cells of a
/// ruled table, so both read a row the same way.
pub(crate) fn group_into_rows(spans: &[TextSpan], y_tolerance_factor: f32) -> Vec<TableRowData> {
    if spans.is_empty() {
        return vec![];
    }

    // Sort by Y (descending for PDF coords) then X
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&spans[a], &spans[b]);
        let y_cmp = b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal);
        if y_cmp == std::cmp::Ordering::Equal {
            a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            y_cmp
        }
    });

    // A row is one line: its spans read left to right, whatever fraction of a point their
    // baselines differ by — an OCR layer lifts each word by its own text rise.
    fn finish(mut indices: Vec<usize>, spans: &[TextSpan]) -> TableRowData {
        indices.sort_by(|&a, &b| spans[a].x.total_cmp(&spans[b].x));
        let row_spans: Vec<TextSpan> = indices.iter().map(|&i| spans[i].clone()).collect();
        let avg_y = row_spans.iter().map(|s| s.y).sum::<f32>() / row_spans.len() as f32;
        TableRowData {
            y: avg_y,
            spans: row_spans,
            sources: indices,
        }
    }

    let mut rows: Vec<TableRowData> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_y: Option<f32> = None;

    for i in order {
        let span = &spans[i];
        let y_tolerance = span.font_size * y_tolerance_factor;

        match current_y {
            Some(y) if (span.y - y).abs() <= y_tolerance => current.push(i),
            _ => {
                if !current.is_empty() {
                    rows.push(finish(std::mem::take(&mut current), spans));
                }
                current_y = Some(span.y);
                current.push(i);
            }
        }
    }

    // Don't forget the last row
    if !current.is_empty() {
        rows.push(finish(current, spans));
    }

    attach_script_rows(rows)
}

/// How many runs of text a row holds once spans set a word space apart are joined.
///
/// A span is not a cell: OCR text layers and some producers write one span per word, so a
/// caption line ("Table 4. Interactive canopy cover ...") arrives as several spans with
/// word spaces between them, and counted span by span every such line looked like a table
/// row. A gap wider than [`CELL_GAP_EM`] of the font size separates cells; a word space
/// (a quarter to a third of the size) does not, and a tight table column gap (two-thirds
/// of the size, measured) still does.
fn separated_runs(row: &TableRowData) -> usize {
    let mut spans: Vec<&TextSpan> = row.spans.iter().collect();
    spans.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let mut runs = 0;
    let mut previous_end: Option<f32> = None;
    for span in spans {
        let separated = match previous_end {
            None => true,
            Some(end) => span.x - end > span.font_size * CELL_GAP_EM,
        };
        if separated {
            runs += 1;
        }
        let end = span.x + span.width;
        previous_end = Some(previous_end.map_or(end, |p| p.max(end)));
    }
    runs
}

/// The gap, in multiples of the font size, beyond which two spans are separate cells.
const CELL_GAP_EM: f32 = 0.5;

/// Whether a row is a line of running text: every space in it is a word space.
///
/// A fixed gap cannot tell the two apart — justification stretches a line's word spaces to
/// fill the column, measured at up to two-thirds of the font size, past [`CELL_GAP_EM`].
/// What it cannot do is make one space stand out: it spreads the slack over all of them,
/// while a table row's cells stand apart by more than the word spaces inside them. So a row
/// is running text when it has at least [`RUNNING_TEXT_MIN_SPACES`] spaces (between its
/// spans, or written inside them), none of those between spans wider than the font size or
/// more than [`WORD_SPACE_SPREAD`] times the typical one, and its words are words rather
/// than figures. The line is measured where its main-size text runs: a footnote mark or a
/// superscript inside it belongs to it, a margin tab's letter beyond its end does not.
fn is_running_text_row(row: &TableRowData) -> bool {
    let mut sizes: Vec<f32> = row.spans.iter().map(|s| s.font_size).collect();
    sizes.sort_by(f32::total_cmp);
    let Some(&size) = sizes.get(sizes.len() / 2) else {
        return false;
    };
    if size <= 0.0 {
        return false;
    }
    // The line is where its main-size text runs; a smaller mark inside it (a footnote
    // reference, a superscript) is part of it, a letter beyond its ends (a margin tab) is not.
    let main = |s: &&TextSpan| (s.font_size - size).abs() <= size * 0.25;
    let start = row
        .spans
        .iter()
        .filter(main)
        .map(|s| s.x)
        .fold(f32::MAX, f32::min);
    let end = row
        .spans
        .iter()
        .filter(main)
        .map(|s| s.x + s.width)
        .fold(f32::MIN, f32::max);
    let mut spans: Vec<&TextSpan> = row
        .spans
        .iter()
        .filter(|s| s.x >= start - 0.5 && s.x + s.width <= end + 0.5)
        .collect();
    spans.sort_by(|a, b| a.x.total_cmp(&b.x));

    // Spaces: gaps wider than glyph fitting (a word drawn in pieces leaves gaps of a few
    // hundredths of the size, or overlaps).
    let mut spaces: Vec<f32> = Vec::new();
    for pair in spans.windows(2) {
        let gap = pair[1].x - (pair[0].x + pair[0].width);
        if gap > size * 0.15 {
            spaces.push(gap);
        }
    }
    // Spaces inside a span are word spaces as written; their width is not measured, but
    // they count toward how many the line has.
    let inner_spaces: usize = spans
        .iter()
        .map(|s| s.text.split_whitespace().count().saturating_sub(1))
        .sum();
    if spaces.len() + inner_spaces < RUNNING_TEXT_MIN_SPACES {
        return false;
    }
    let even = if spaces.is_empty() {
        true
    } else {
        spaces.sort_by(f32::total_cmp);
        let typical = spaces[spaces.len() / 2];
        let widest = spaces[spaces.len() - 1];
        widest <= size && widest <= typical * WORD_SPACE_SPREAD
    };
    let words: Vec<&str> = spans
        .iter()
        .flat_map(|s| s.text.split_whitespace())
        .collect();
    let lettered = words
        .iter()
        .filter(|w| w.chars().any(char::is_alphabetic))
        .count();
    even && lettered * 3 >= words.len() * 2
}

/// The fewest spaces a row needs before its spacing says anything: a row of two or three
/// cells has one or two, which are as even as word spaces by construction.
const RUNNING_TEXT_MIN_SPACES: usize = 3;

/// How much wider than the row's typical space its widest may be and still be a word space.
/// Justification keeps a line's spaces within a few percent of each other, a little more
/// after punctuation; measured up to 1.4.
const WORD_SPACE_SPREAD: f32 = 1.6;

/// Fold a row of superscripts or subscripts into the row of text they mark.
///
/// A superscript (`0.31*`, a footnote mark) sits on a baseline raised by about a
/// third of the text size and is set smaller, so row grouping — which compares
/// baselines against a fraction of the font size — gives it a row of its own
/// directly above its line. Its cell then reads the mark first (`* 0.31`), or the
/// table gains a row of nothing but marks.
///
/// A row is a script row of the row next to it only when it looks like marks on
/// that text and nothing else: every span is short (a mark, a footnote number),
/// clearly smaller, on a baseline within the text's line height, and set right
/// against the end of a span of that row. Smaller text in a neighbouring column
/// on a slightly different baseline shares the first three and fails the last —
/// folding it in turns running text into rows of a table.
fn attach_script_rows(rows: Vec<TableRowData>) -> Vec<TableRowData> {
    const MAX_MARK_CHARS: usize = 4;

    fn largest(row: &TableRowData) -> f32 {
        row.spans.iter().map(|s| s.font_size).fold(0.0, f32::max)
    }
    fn is_script_of(script: &TableRowData, text: &TableRowData) -> bool {
        let size = largest(text);
        let ratio = largest(script) / size;
        // Scripts are set at roughly 55-75% of the text size. Text at a fifth of the
        // size of the row next to it is ordinary text beside a display line or a
        // watermark, not a mark on it.
        if !(0.45..=0.8).contains(&ratio) || (script.y - text.y).abs() > size * 0.6 {
            return false;
        }
        // Set against a span of the text row, or against another mark that is (`1•2`).
        let against = |mark: &TextSpan, other: &TextSpan| {
            let end = other.x + other.width;
            mark.x >= end - size * 0.3 && mark.x <= end + size * 0.5
        };
        let anchored = script
            .spans
            .iter()
            .any(|mark| text.spans.iter().any(|base| against(mark, base)));
        anchored
            && script.spans.iter().all(|mark| {
                mark.text.trim().chars().count() <= MAX_MARK_CHARS
                    && (text.spans.iter().any(|base| against(mark, base))
                        || script
                            .spans
                            .iter()
                            .any(|other| !std::ptr::eq(other, mark) && against(mark, other)))
            })
    }

    let mut out: Vec<TableRowData> = Vec::with_capacity(rows.len());
    let mut pending: Option<TableRowData> = None;
    for row in rows {
        // Rows run top to bottom: a superscript row arrives before its text row.
        if let Some(script) = pending.take() {
            if script.y > row.y && is_script_of(&script, &row) {
                let mut row = row;
                fold_marks(&mut row, script);
                pending = Some(row);
                continue;
            }
            out.push(script);
        }
        match out.last_mut() {
            Some(text) if row.y < text.y && is_script_of(&row, text) => fold_marks(text, row),
            _ => pending = Some(row),
        }
    }
    out.extend(pending);
    out
}

/// Append each mark of `script` to the span of `text` it is set against, so the
/// mark reads after its value (`0.31*`) and the row keeps its spans — a mark is
/// part of the token it marks, not a column of its own.
fn fold_marks(text: &mut TableRowData, script: TableRowData) {
    text.sources.extend(script.sources);
    let mut marks = script.spans;
    marks.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    for mark in marks {
        let base = text.spans.iter_mut().min_by(|a, b| {
            let da = (mark.x - (a.x + a.width)).abs();
            let db = (mark.x - (b.x + b.width)).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(base) = base {
            base.text.push_str(mark.text.trim());
            base.width = (mark.x + mark.width).max(base.x + base.width) - base.x;
            base.width_measured &= mark.width_measured;
        }
    }
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
            width_measured: false,
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

    fn sized(text: &str, x: f32, y: f32, size: f32) -> TextSpan {
        TextSpan {
            font_size: size,
            ..make_span(text, x, y)
        }
    }

    /// `0.31*` with the star set as a superscript: one row, the star after the value.
    #[test]
    fn test_group_into_rows_keeps_superscripts_with_their_row() {
        let detector = TableDetector::new();
        let spans = vec![
            sized("IM", 10.0, 400.0, 8.0),
            sized("0.31", 60.0, 400.0, 8.0),
            sized("*", 84.0, 404.0, 5.0),
            sized("0.77", 120.0, 400.0, 8.0),
            sized("**", 144.0, 404.0, 5.0),
            sized("IBE", 10.0, 390.0, 8.0),
            sized("0.30", 60.0, 390.0, 8.0),
            sized("2", 84.0, 387.0, 5.0),
        ];

        let rows = detector.group_into_rows(&spans);
        let texts: Vec<Vec<&str>> = rows
            .iter()
            .map(|r| r.spans.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert_eq!(
            texts,
            vec![vec!["IM", "0.31*", "0.77**"], vec!["IBE", "0.302"],]
        );
    }

    /// Words whose baselines differ by a fraction of a point — an OCR layer lifts each by
    /// its own text rise — read left to right in their row.
    #[test]
    fn test_group_into_rows_reads_a_row_left_to_right() {
        let detector = TableDetector::new();
        let spans = vec![
            make_span("Fifth", 10.0, 400.0),
            make_span("day", 50.0, 400.0),
            make_span("a", 80.0, 400.4),
            make_span("syndrome", 95.0, 400.0),
        ];
        let rows = detector.group_into_rows(&spans);
        assert_eq!(rows.len(), 1);
        let words: Vec<&str> = rows[0].spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(words, ["Fifth", "day", "a", "syndrome"]);
    }

    /// Smaller text in the next column, on a baseline a little above, is running
    /// text of its own — not marks on this row.
    #[test]
    fn test_group_into_rows_keeps_a_neighbouring_columns_line_apart() {
        let detector = TableDetector::new();
        let spans = vec![
            sized("True-crime series looks at", 10.0, 400.0, 9.0),
            sized("grimly chasing", 200.0, 404.5, 7.0),
        ];
        assert_eq!(detector.group_into_rows(&spans).len(), 2);
    }

    /// Body-size cells beside a large watermark glyph are not marks on the watermark.
    #[test]
    fn test_group_into_rows_does_not_fold_text_into_a_watermark() {
        let detector = TableDetector::new();
        let spans = vec![
            sized("A", 100.0, 440.0, 40.0),
            sized("PPO", 124.0, 430.0, 8.0),
            sized("31.1", 160.0, 430.0, 8.0),
        ];
        assert_eq!(detector.group_into_rows(&spans).len(), 2);
    }

    /// Marks set one after another (`1•2`) fold into the token before them and leave
    /// the row's span count alone — a caption with footnote marks is still one cell.
    #[test]
    fn test_group_into_rows_folds_chained_marks_into_their_token() {
        let detector = TableDetector::new();
        let spans = vec![
            sized("season.", 10.0, 400.0, 7.6),
            sized("1", 52.0, 404.5, 5.2),
            sized("2", 58.0, 404.5, 5.2),
        ];
        let rows = detector.group_into_rows(&spans);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spans.len(), 1);
        assert_eq!(rows[0].spans[0].text, "season.12");
    }

    /// A row of smaller text a full line below is its own row, not a subscript.
    #[test]
    fn test_group_into_rows_keeps_a_smaller_row_at_line_spacing() {
        let detector = TableDetector::new();
        let spans = vec![
            sized("A", 10.0, 400.0, 10.0),
            sized("B", 60.0, 400.0, 10.0),
            sized("note", 10.0, 388.0, 7.0),
        ];
        assert_eq!(detector.group_into_rows(&spans).len(), 2);
    }

    /// A caption written one span per word is one run of text, not a table row.
    #[test]
    fn test_word_spans_are_one_run_and_cells_are_separate() {
        let row = |spans: Vec<TextSpan>| TableRowData {
            y: 100.0,
            spans,
            sources: Vec::new(),
        };
        // make_span widths are 6pt per character at 12pt: "Table" ends at 40, a 3pt word
        // space, then "4." ends at 55, another word space, then the caption.
        let caption = row(vec![
            make_span("Table", 10.0, 100.0),
            make_span("4.", 43.0, 100.0),
            make_span("Interactive", 58.0, 100.0),
        ]);
        assert_eq!(separated_runs(&caption), 1);
        let cells = row(vec![
            make_span("Name", 10.0, 100.0),
            make_span("Age", 60.0, 100.0),
            make_span("City", 110.0, 100.0),
        ]);
        assert_eq!(separated_runs(&cells), 3);
    }

    #[test]
    fn test_detect_columns() {
        let detector = TableDetector::new();
        let rows = vec![
            TableRowData {
                y: 100.0,
                spans: vec![make_span("A1", 10.0, 100.0), make_span("B1", 60.0, 100.0)],
                sources: Vec::new(),
            },
            TableRowData {
                y: 85.0,
                spans: vec![make_span("A2", 10.0, 85.0), make_span("B2", 60.0, 85.0)],
                sources: Vec::new(),
            },
            TableRowData {
                y: 70.0,
                spans: vec![make_span("A3", 10.0, 70.0), make_span("B3", 60.0, 70.0)],
                sources: Vec::new(),
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

    /// A mark folded into its value's text is consumed with the table: matching
    /// consumed spans by their text left both the changed span and the mark behind,
    /// and they reappeared as body text after the table.
    #[test]
    fn test_detect_consumes_folded_marks_with_the_table() {
        let detector = TableDetector::new();
        let spans = vec![
            sized("Name", 10.0, 100.0, 12.0),
            sized("Value", 60.0, 100.0, 12.0),
            sized("Alice", 10.0, 85.0, 12.0),
            sized("30", 60.0, 85.0, 12.0),
            sized("*", 72.0, 90.0, 7.0),
            sized("Bob", 10.0, 70.0, 12.0),
            sized("25", 60.0, 70.0, 12.0),
        ];

        let (tables, remaining) = detector.detect(spans);
        assert_eq!(tables.len(), 1);
        assert!(remaining.is_empty(), "left behind: {remaining:?}");
        let texts: Vec<&str> = tables[0].rows[1]
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(texts, vec!["Alice", "30*"]);
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
                    sources: Vec::new(),
                },
                TableRowData {
                    y: 85.0,
                    spans: vec![make_span("Alice", 10.0, 85.0), make_span("30", 60.0, 85.0)],
                    sources: Vec::new(),
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
            width_measured: false,
            font_size,
            font_name: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
        }
    }

    fn measured(text: &str, x: f32, y: f32, width: f32) -> TextSpan {
        TextSpan {
            width,
            width_measured: true,
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

    /// A justified line drawn word by word: `words` set from `x0`, each `word_width` wide,
    /// `space` apart.
    fn justified_line(
        words: &[&str],
        x0: f32,
        y: f32,
        word_width: f32,
        space: f32,
    ) -> Vec<TextSpan> {
        words
            .iter()
            .enumerate()
            .map(|(i, w)| measured(w, x0 + i as f32 * (word_width + space), y, word_width))
            .collect()
    }

    #[test]
    fn justified_lines_whose_word_spaces_pass_a_cell_gap_are_not_a_table() {
        // Word spaces stretched to two-thirds of the font size — wider than a cell gap —
        // and spread evenly, and the word starts of every line coincide.
        let detector = TableDetector::new();
        let mut spans = Vec::new();
        for i in 0..4 {
            let y = 700.0 - i as f32 * 18.0;
            spans.extend(justified_line(
                &["monetary", "policy", "was", "kept", "tight"],
                72.0,
                y,
                40.0,
                8.0,
            ));
        }
        let (tables, remaining) = detector.detect(spans);
        assert!(
            tables.is_empty(),
            "running text must not become a table: {tables:?}"
        );
        assert_eq!(remaining.len(), 20);
    }

    fn row_of(spans: Vec<TextSpan>) -> TableRowData {
        let sources = (0..spans.len()).collect();
        TableRowData {
            y: spans[0].y,
            spans,
            sources,
        }
    }

    #[test]
    fn a_footnote_mark_inside_a_line_is_part_of_it() {
        // A smaller reference mark right after a word, then a word space: measured without
        // the mark, its place would look like a gap twice the font size.
        let mut spans = justified_line(&["will", "likely", "slow"], 72.0, 700.0, 40.0, 5.0);
        let mut mark = make_span_w("12)", 202.0, 702.0, 8.0);
        mark.width = 17.0;
        spans.push(mark);
        spans.extend(justified_line(
            &["but", "policy", "shifts"],
            224.0,
            700.0,
            40.0,
            5.0,
        ));
        assert!(is_running_text_row(&row_of(spans)));
    }

    #[test]
    fn a_margin_letter_beyond_a_line_is_not_part_of_it() {
        let mut spans = justified_line(&["the", "rate", "was", "held"], 72.0, 700.0, 36.0, 6.0);
        let mut tab = make_span_w("T", 300.0, 700.0, 7.0);
        tab.width = 5.0;
        spans.push(tab);
        assert!(is_running_text_row(&row_of(spans)));
    }

    #[test]
    fn a_row_of_cells_is_not_running_text() {
        let row = row_of(vec![
            measured("Gross domestic product", 72.0, 700.0, 130.0),
            measured("2.1 percent", 240.0, 700.0, 64.0),
            measured("rose", 360.0, 700.0, 36.0),
        ]);
        assert!(!is_running_text_row(&row));
    }

    #[test]
    fn a_table_whose_cells_hold_several_words_is_still_a_table() {
        // Word spaces inside the cells, wider gaps between them: the cell gaps stand out.
        let detector = TableDetector::new();
        let mut spans = Vec::new();
        for (i, (a, b, c)) in [
            ("Gross domestic product", "2.1 percent", "rose"),
            ("Consumer price index", "3.2 percent", "fell"),
            ("Policy interest rate", "3.5 percent", "held"),
            ("Household credit growth", "1.0 percent", "slowed"),
        ]
        .iter()
        .enumerate()
        {
            let y = 700.0 - i as f32 * 16.0;
            spans.push(measured(a, 72.0, y, 130.0));
            spans.push(measured(b, 240.0, y, 64.0));
            spans.push(measured(c, 360.0, y, 36.0));
        }
        let (tables, _) = detector.detect(spans);
        assert_eq!(tables.len(), 1, "a real table must survive: {tables:?}");
    }

    #[test]
    fn interleaved_lines_of_two_text_columns_are_not_a_table() {
        // Two columns whose baselines are offset by half a line, each justified and drawn
        // word by word, so rows alternate between the columns. Lines are 180 wide, the
        // gutter 24.
        let detector = TableDetector::new();
        let mut spans = Vec::new();
        for i in 0..10 {
            let left_y = 700.0 - i as f32 * 18.0;
            spans.extend(justified_line(
                &["aa", "bb", "cc", "dd"],
                72.0,
                left_y,
                39.0,
                8.0,
            ));
            spans.extend(justified_line(
                &["ee", "ff", "gg"],
                276.0,
                left_y - 9.0,
                54.0,
                9.0,
            ));
        }
        // A real stretch of misalignment: one line in each column broken by a wide gap.
        spans.push(measured("hh", 72.0, 500.0, 20.0));
        spans.push(measured("ii", 150.0, 500.0, 102.0));
        let total = spans.len();
        let (tables, remaining) = detector.detect(spans);
        assert!(
            tables.is_empty(),
            "two text columns must not become a table: {tables:?}"
        );
        assert_eq!(remaining.len(), total);
    }

    #[test]
    fn a_margin_tab_beside_running_text_does_not_make_it_a_table() {
        // A thumb-index tab set as stacked letters at the margin, between and on the
        // text's lines.
        let detector = TableDetector::new();
        let mut spans = Vec::new();
        for i in 0..6 {
            let y = 700.0 - i as f32 * 18.0;
            spans.extend(justified_line(
                &["the", "rate", "was", "held"],
                72.0,
                y,
                36.0,
                6.0,
            ));
            let mut tab = make_span_w("T", 300.0, y, 7.0);
            tab.width = 5.0;
            spans.push(tab);
            let mut between = make_span_w("A", 300.0, y - 9.0, 7.0);
            between.width = 5.0;
            spans.push(between);
        }
        let (tables, _) = detector.detect(spans);
        assert!(tables.is_empty(), "{tables:?}");
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
            width_measured: true,
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
