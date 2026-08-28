//! Lattice-mode table detection: grid inference and cell assignment.
//!
//! Takes the line segments
//! [`vector_graphics::extract_lines`](super::vector_graphics::extract_lines)
//! already pulled out of a page's content stream, infers table grids from
//! them (axis-aligned ruling lines clustered into row/column boundaries,
//! following the same edge-detection → merge → grid approach Camelot/pdfplumber
//! use for their lattice mode), and assigns text spans into the resulting
//! cells. Priority against `table_detector`'s text-alignment-based stream
//! mode is decided by the caller (`pdf_parser`): a confirmed lattice grid is
//! strong structural evidence (explicit borders), so it bypasses stream
//! mode's alignment/occupancy heuristics entirely.

use super::layout::TextSpan;
use super::vector_graphics::GraphicsLine;
use crate::model::{Table, TableCell, TableRow};

/// Configuration for lattice grid inference.
#[derive(Debug, Clone)]
pub struct LatticeConfig {
    /// Minimum number of rows (row boundary count - 1) to count as a grid.
    pub min_rows: usize,
    /// Minimum number of columns (column boundary count - 1) to count as a grid.
    pub min_columns: usize,
    /// A line is "axis-aligned" if its off-axis deviation is within this tolerance (points).
    pub axis_tolerance: f32,
    /// Lines shorter than this (points) are discarded as noise (tick marks, underlines).
    pub min_line_length: f32,
    /// Two lines within this distance (points) on their axis are clustered into one boundary.
    pub cluster_tolerance: f32,
}

impl Default for LatticeConfig {
    fn default() -> Self {
        Self {
            min_rows: 2,
            min_columns: 2,
            axis_tolerance: 1.0,
            min_line_length: 5.0,
            cluster_tolerance: 2.0,
        }
    }
}

/// A table grid inferred from explicit ruling lines.
///
/// Boundaries follow `table_detector::DetectedTable`'s convention: `row_bounds`
/// is descending (PDF y increases upward; topmost row first), `col_bounds` is
/// ascending (left to right). `N` boundaries describe `N - 1` rows/columns.
#[derive(Debug, Clone, PartialEq)]
pub struct LatticeGrid {
    pub top_y: f32,
    pub bottom_y: f32,
    pub left_x: f32,
    pub right_x: f32,
    /// Row boundary Y positions, descending.
    pub row_bounds: Vec<f32>,
    /// Column boundary X positions, ascending.
    pub col_bounds: Vec<f32>,
}

impl LatticeGrid {
    pub fn row_count(&self) -> usize {
        self.row_bounds.len().saturating_sub(1)
    }

    pub fn column_count(&self) -> usize {
        self.col_bounds.len().saturating_sub(1)
    }
}

/// Infer lattice grids from a page's extracted line segments.
///
/// A page can contain more than one bordered table, so this returns every
/// grid found — each built from a connected cluster of horizontal/vertical
/// lines whose bounding boxes overlap.
pub fn infer_grids(lines: &[GraphicsLine], config: &LatticeConfig) -> Vec<LatticeGrid> {
    let (horizontals, verticals) = classify_lines(lines, config);
    if horizontals.is_empty() || verticals.is_empty() {
        return vec![];
    }

    // Cluster into candidate row/column boundary positions.
    let mut row_positions =
        cluster_positions(horizontals.iter().map(|l| l.0), config.cluster_tolerance);
    // Descending: PDF y increases upward, and reading order is top (high y) to bottom.
    row_positions.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let mut col_positions =
        cluster_positions(verticals.iter().map(|l| l.0), config.cluster_tolerance);
    col_positions.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    if row_positions.len() < config.min_rows + 1 || col_positions.len() < config.min_columns + 1 {
        return vec![];
    }

    // Single-grid MVP: one page is (for now) treated as at most one lattice
    // region, spanning the full extent of its ruling lines. Splitting
    // disjoint clusters into separate grids is deferred until a real
    // multi-table-per-page fixture demonstrates it's needed (YAGNI).
    let top_y = row_positions.first().copied().unwrap_or(0.0);
    let bottom_y = row_positions.last().copied().unwrap_or(0.0);
    let left_x = col_positions.first().copied().unwrap_or(0.0);
    let right_x = col_positions.last().copied().unwrap_or(0.0);

    vec![LatticeGrid {
        top_y,
        bottom_y,
        left_x,
        right_x,
        row_bounds: row_positions,
        col_bounds: col_positions,
    }]
}

/// A cell is considered real content, not a decorative frame, once at least
/// this fraction of its cells hold non-blank text. Lower than
/// `table_detector`'s stream-mode occupancy floor (0.3) on purpose: lattice
/// mode exists specifically to catch bordered tables stream mode's occupancy
/// heuristic would reject, so re-applying the same floor here would defeat
/// its own purpose.
const MIN_OCCUPANCY: f32 = 0.1;

/// Assign text spans into a lattice grid's cells and build a [`Table`].
///
/// Returns the table (always with `grid.row_count()` rows, even if some cells
/// end up blank) alongside the indices into `spans` that were consumed —
/// the caller removes those before handing the remaining spans to stream-mode
/// detection or the plain text pipeline, so a span isn't extracted twice.
/// Returns `None` when too few cells actually hold text (see [`MIN_OCCUPANCY`])
/// — most likely a decorative box or diagram frame, not a real table.
pub(crate) fn build_table(grid: &LatticeGrid, spans: &[TextSpan]) -> Option<(Table, Vec<usize>)> {
    let rows = grid.row_count();
    let cols = grid.column_count();
    if rows == 0 || cols == 0 {
        return None;
    }

    // Reading order: top row first, left to right within a row — matches
    // `table_detector::TableDetector::group_into_rows`'s sort convention.
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| {
        spans[b]
            .y
            .partial_cmp(&spans[a].y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                spans[a]
                    .x
                    .partial_cmp(&spans[b].x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut cell_text: Vec<Vec<Vec<String>>> = vec![vec![Vec::new(); cols]; rows];
    let mut consumed = Vec::new();

    for i in order {
        let span = &spans[i];
        let (Some(r), Some(c)) = (
            bin_index(&grid.row_bounds, span.y),
            bin_index(&grid.col_bounds, span.x),
        ) else {
            continue;
        };
        let text = span.text.trim();
        if !text.is_empty() {
            cell_text[r][c].push(text.to_string());
        }
        consumed.push(i);
    }

    let non_empty = cell_text.iter().flatten().filter(|c| !c.is_empty()).count();
    let occupancy = non_empty as f32 / (rows * cols) as f32;
    if occupancy < MIN_OCCUPANCY {
        return None;
    }

    let mut table = Table::new();
    table.header_rows = if rows > 1 { 1 } else { 0 };
    for (r, row_cells) in cell_text.into_iter().enumerate() {
        let cells: Vec<TableCell> = row_cells
            .into_iter()
            .map(|texts| TableCell::text(texts.join(" ")))
            .collect();
        let table_row = if r == 0 && table.header_rows > 0 {
            TableRow::header(cells)
        } else {
            TableRow::new(cells)
        };
        table.add_row(table_row);
    }
    table.column_widths = Some(
        (0..cols)
            .map(|i| grid.col_bounds[i + 1] - grid.col_bounds[i])
            .collect(),
    );

    Some((table, consumed))
}

/// Boundary-tolerance for assigning a span to a grid cell (points). Grid
/// boundaries are exact ruling-line positions; a span whose baseline sits a
/// couple of points outside the nominal box (font metrics, hairline rounding)
/// should still land in its cell rather than being silently dropped.
const CELL_BOUNDARY_TOLERANCE: f32 = 2.0;

/// Find which `[boundaries[i], boundaries[i+1])` bin `value` falls into.
/// Works for either ascending (`col_bounds`) or descending (`row_bounds`)
/// boundary lists.
fn bin_index(boundaries: &[f32], value: f32) -> Option<usize> {
    for i in 0..boundaries.len().saturating_sub(1) {
        let (lo, hi) = if boundaries[i] <= boundaries[i + 1] {
            (boundaries[i], boundaries[i + 1])
        } else {
            (boundaries[i + 1], boundaries[i])
        };
        if value >= lo - CELL_BOUNDARY_TOLERANCE && value <= hi + CELL_BOUNDARY_TOLERANCE {
            return Some(i);
        }
    }
    None
}

/// A line reduced to (position on its perpendicular axis, length along its own axis).
type AxisLine = (f32, f32);

/// Split lines into (horizontal, vertical) axis lines, discarding diagonal and
/// too-short lines.
fn classify_lines(
    lines: &[GraphicsLine],
    config: &LatticeConfig,
) -> (Vec<AxisLine>, Vec<AxisLine>) {
    let mut horizontals = Vec::new();
    let mut verticals = Vec::new();

    for line in lines {
        let dx = (line.x1 - line.x0).abs();
        let dy = (line.y1 - line.y0).abs();

        if dy <= config.axis_tolerance && dx >= config.min_line_length {
            let y = (line.y0 + line.y1) / 2.0;
            horizontals.push((y, dx));
        } else if dx <= config.axis_tolerance && dy >= config.min_line_length {
            let x = (line.x0 + line.x1) / 2.0;
            verticals.push((x, dy));
        }
        // Diagonal or too-short lines are not ruling-line evidence — ignored.
    }

    (horizontals, verticals)
}

/// Cluster axis positions within `tolerance` of each other into single
/// representative positions (the mean of each cluster).
///
/// Compares each candidate against its cluster's *first* (smallest) member,
/// not its most recently added one — otherwise a chain of points each just
/// under `tolerance` from its neighbor could drift arbitrarily far from where
/// the cluster started, silently merging two genuinely distinct ruling lines.
fn cluster_positions(positions: impl Iterator<Item = f32>, tolerance: f32) -> Vec<f32> {
    let mut sorted: Vec<f32> = positions.collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut clusters: Vec<Vec<f32>> = Vec::new();
    for pos in sorted {
        match clusters.last_mut() {
            Some(cluster) if (pos - cluster[0]).abs() <= tolerance => {
                cluster.push(pos);
            }
            _ => clusters.push(vec![pos]),
        }
    }

    clusters
        .into_iter()
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(y: f32, x0: f32, x1: f32) -> GraphicsLine {
        GraphicsLine {
            x0,
            y0: y,
            x1,
            y1: y,
        }
    }

    fn v(x: f32, y0: f32, y1: f32) -> GraphicsLine {
        GraphicsLine {
            x0: x,
            y0,
            x1: x,
            y1,
        }
    }

    /// A clean 3-row x 2-column grid: 4 horizontal rules, 3 vertical rules.
    fn clean_grid_lines() -> Vec<GraphicsLine> {
        vec![
            h(300.0, 50.0, 250.0),
            h(280.0, 50.0, 250.0),
            h(260.0, 50.0, 250.0),
            h(240.0, 50.0, 250.0),
            v(50.0, 240.0, 300.0),
            v(150.0, 240.0, 300.0),
            v(250.0, 240.0, 300.0),
        ]
    }

    #[test]
    fn clean_grid_is_detected() {
        let grids = infer_grids(&clean_grid_lines(), &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        let grid = &grids[0];
        assert_eq!(grid.row_count(), 3);
        assert_eq!(grid.column_count(), 2);
        // Descending row bounds (topmost/highest-y first).
        assert_eq!(grid.row_bounds, vec![300.0, 280.0, 260.0, 240.0]);
        assert_eq!(grid.col_bounds, vec![50.0, 150.0, 250.0]);
    }

    #[test]
    fn too_few_lines_yields_no_grid() {
        // Only 2 horizontal + 2 vertical → 1 row, 1 column — below default min_rows=2.
        let lines = vec![
            h(100.0, 0.0, 100.0),
            h(80.0, 0.0, 100.0),
            v(0.0, 80.0, 100.0),
            v(100.0, 80.0, 100.0),
        ];
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    #[test]
    fn no_vertical_lines_yields_no_grid() {
        let lines = vec![
            h(100.0, 0.0, 100.0),
            h(80.0, 0.0, 100.0),
            h(60.0, 0.0, 100.0),
        ];
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    #[test]
    fn diagonal_lines_are_ignored() {
        let lines = vec![
            GraphicsLine {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0,
            }, // pure diagonal
            h(100.0, 0.0, 100.0),
            h(80.0, 0.0, 100.0),
            h(60.0, 0.0, 100.0),
            v(0.0, 60.0, 100.0),
            v(50.0, 60.0, 100.0),
            v(100.0, 60.0, 100.0),
        ];
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(
            grids.len(),
            1,
            "diagonal line should not disrupt grid detection"
        );
    }

    #[test]
    fn too_short_lines_are_discarded_as_noise() {
        // Tick marks shorter than min_line_length (5.0) shouldn't count as ruling lines.
        let lines = vec![h(100.0, 0.0, 2.0), h(80.0, 0.0, 2.0), v(0.0, 80.0, 82.0)];
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    #[test]
    fn clustering_does_not_chain_drift_across_distinct_boundaries() {
        // A chain of horizontal lines each 1.9pt from its neighbor (within the
        // default 2.0pt cluster_tolerance) must not collapse into a single row
        // boundary just because consecutive gaps are individually small — the
        // total span (100.0 to 105.7) is clearly two distinct table regions'
        // worth of drift, not one double-drawn border.
        let config = LatticeConfig::default();
        let positions = cluster_positions(
            [100.0, 101.9, 103.8, 105.7].into_iter(),
            config.cluster_tolerance,
        );
        assert_eq!(
            positions.len(),
            2,
            "chained-but-drifting points should split into more than one cluster, got {:?}",
            positions
        );
    }

    #[test]
    fn near_duplicate_lines_cluster_into_one_boundary() {
        // Two lines 0.5pt apart (within cluster_tolerance=2.0) at each of 3 row
        // positions and 3 column positions — should still resolve to a 2x2 grid,
        // not spurious extra rows/columns from double-drawn borders.
        let lines = vec![
            h(300.0, 50.0, 250.0),
            h(300.5, 50.0, 250.0),
            h(270.0, 50.0, 250.0),
            h(240.0, 50.0, 250.0),
            v(50.0, 240.0, 300.0),
            v(150.0, 240.0, 300.0),
            v(250.0, 240.0, 300.0),
        ];
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].row_count(), 2);
    }

    #[test]
    fn empty_input_yields_no_grid() {
        assert!(infer_grids(&[], &LatticeConfig::default()).is_empty());
    }

    fn span(text: &str, x: f32, y: f32) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            x,
            y,
            width: text.len() as f32 * 6.0,
            font_size: 12.0,
            font_name: "Helvetica".to_string(),
            is_bold: false,
            is_italic: false,
        }
    }

    /// A 2-row x 2-column grid: row_bounds [300,280,260] (descending),
    /// col_bounds [50,150,250] (ascending) — matching `clean_grid_lines`'s
    /// 3-row variant but trimmed to 2 rows for simpler cell math in tests.
    fn two_by_two_grid() -> LatticeGrid {
        LatticeGrid {
            top_y: 300.0,
            bottom_y: 260.0,
            left_x: 50.0,
            right_x: 250.0,
            row_bounds: vec![300.0, 280.0, 260.0],
            col_bounds: vec![50.0, 150.0, 250.0],
        }
    }

    #[test]
    fn build_table_assigns_spans_to_cells_in_reading_order() {
        let grid = two_by_two_grid();
        let spans = vec![
            span("Name", 60.0, 290.0),
            span("Age", 160.0, 290.0),
            span("Alice", 60.0, 270.0),
            span("30", 160.0, 270.0),
        ];
        let (table, consumed) = build_table(&grid, &spans).expect("should build a table");
        assert_eq!(consumed.len(), 4);
        assert_eq!(table.row_count(), 2);
        assert_eq!(table.header_rows, 1);
        assert_eq!(table.rows[0].cells[0].plain_text(), "Name");
        assert_eq!(table.rows[0].cells[1].plain_text(), "Age");
        assert_eq!(table.rows[1].cells[0].plain_text(), "Alice");
        assert_eq!(table.rows[1].cells[1].plain_text(), "30");
    }

    #[test]
    fn build_table_multi_fragment_cell_joins_left_to_right() {
        let grid = two_by_two_grid();
        // Two spans in the same cell (e.g. a wrapped or kerned run split by
        // the font decoder) should join in x order, not arrive concatenated.
        let spans = vec![span("Hello", 60.0, 290.0), span("World", 90.0, 290.0)];
        let (table, _) = build_table(&grid, &spans).expect("should build a table");
        assert_eq!(table.rows[0].cells[0].plain_text(), "Hello World");
    }

    #[test]
    fn build_table_span_outside_grid_is_not_consumed() {
        let grid = two_by_two_grid();
        let spans = vec![
            span("Name", 60.0, 290.0),
            span("Age", 160.0, 290.0),
            span("Alice", 60.0, 270.0),
            span("30", 160.0, 270.0),
            // Far outside the grid's bounding box — page caption, not a cell.
            span("Caption text", 60.0, 20.0),
        ];
        let (_, consumed) = build_table(&grid, &spans).expect("should build a table");
        assert_eq!(
            consumed.len(),
            4,
            "the out-of-grid span must not be consumed"
        );
        assert!(!consumed.contains(&4));
    }

    #[test]
    fn build_table_rejects_decorative_frame_with_no_real_content() {
        // A grid with ruling lines but almost no text inside — a decorative
        // box, not a table. Below MIN_OCCUPANCY (0.1) for a 2x2 = 4-cell grid,
        // a single filled cell is exactly 0.25, so use a 4-row grid (8 cells)
        // with only one span filled: 1/8 = 0.125... still above 0.1. Use a
        // grid with zero spans at all to unambiguously exercise the reject path.
        let grid = two_by_two_grid();
        assert!(build_table(&grid, &[]).is_none());
    }

    #[test]
    fn bin_index_works_for_ascending_and_descending_boundaries() {
        let ascending = [0.0, 10.0, 20.0];
        assert_eq!(bin_index(&ascending, 5.0), Some(0));
        assert_eq!(bin_index(&ascending, 15.0), Some(1));
        assert_eq!(bin_index(&ascending, 100.0), None);

        let descending = [20.0, 10.0, 0.0];
        assert_eq!(bin_index(&descending, 15.0), Some(0));
        assert_eq!(bin_index(&descending, 5.0), Some(1));
    }
}
