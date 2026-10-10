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

use super::layout::{TextLine, TextSpan};
use super::table_detector::group_into_rows;
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
    /// Two lines within this distance (points) on their axis are clustered into one
    /// boundary, and lines whose ends come this close count as touching. 3pt is
    /// pdfplumber's `snap_tolerance`: a border drawn twice (a background box and the
    /// frame inset inside it) lands a couple of points off, and no real table column
    /// is that narrow.
    pub cluster_tolerance: f32,
}

impl Default for LatticeConfig {
    fn default() -> Self {
        Self {
            min_rows: 2,
            min_columns: 2,
            axis_tolerance: 1.0,
            min_line_length: 5.0,
            cluster_tolerance: 3.0,
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
/// lines that touch or cross one another. Lines that never meet are not
/// evidence of a shared table: a figure frame, a separator rule and a
/// bordered table elsewhere on the page each stay their own cluster, and a
/// cluster that doesn't form at least `min_rows` x `min_columns` cells is
/// dropped. Merging them into one page-spanning grid would turn every span
/// that happens to fall between them into a table cell.
///
/// Grids are returned top-down (highest `top_y` first).
pub fn infer_grids(lines: &[GraphicsLine], config: &LatticeConfig) -> Vec<LatticeGrid> {
    let segments = classify_lines(lines, config);
    let mut grids: Vec<LatticeGrid> = connected_components(&segments, config.cluster_tolerance)
        .into_iter()
        .filter_map(|component| grid_from_component(&component, config))
        .collect();
    grids.sort_by(|a, b| {
        b.top_y
            .partial_cmp(&a.top_y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    grids
}

/// Build a grid from one connected cluster of ruling lines, or `None` if the
/// cluster is too sparse to describe a table (e.g. a single frame rectangle).
fn grid_from_component(component: &[AxisSegment], config: &LatticeConfig) -> Option<LatticeGrid> {
    let x_rules = ruled_positions(component, Axis::Vertical, config);
    let y_rules = ruled_positions(component, Axis::Horizontal, config);
    let mut row_positions = boundaries(component, Axis::Horizontal, &x_rules, config)?;
    let mut col_positions = boundaries(component, Axis::Vertical, &y_rules, config)?;
    // A table ruled only between its cells, with no frame, ends where its rules do.
    let open_rows = open_sides(component, Axis::Vertical, &row_positions);
    let open_columns = open_sides(component, Axis::Horizontal, &col_positions);
    row_positions.extend(open_rows);
    col_positions.extend(open_columns);
    // Descending: PDF y increases upward, and reading order is top (high y) to bottom.
    row_positions.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    col_positions.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let columns = col_positions.len().saturating_sub(1);
    let rows = row_positions.len().saturating_sub(1);
    // A single ruled column is a table when it is a stack of at least three ruled rows (a boxed
    // list of items) as wide as text runs; `build_table` then holds it to one line per cell. Two
    // rows in one column are a box with a title bar.
    let single_column = columns == 1
        && rows >= SINGLE_COLUMN_MIN_ROWS
        && (col_positions[1] - col_positions[0]).abs() >= SINGLE_COLUMN_MIN_WIDTH;
    if rows < config.min_rows || (columns < config.min_columns && !single_column) {
        return None;
    }

    Some(LatticeGrid {
        top_y: *row_positions.first()?,
        bottom_y: *row_positions.last()?,
        left_x: *col_positions.first()?,
        right_x: *col_positions.last()?,
        row_bounds: row_positions,
        col_bounds: col_positions,
    })
}

/// The fewest ruled rows a grid of one column needs to be a table.
const SINGLE_COLUMN_MIN_ROWS: usize = 3;

/// The narrowest a grid of one column may be and still be a table (points). A chart's legend is
/// a small ruled box of short entries -- three rows of one column, like a list of items; the
/// lists that are tables run the width of a text column.
const SINGLE_COLUMN_MIN_WIDTH: f32 = 120.0;

/// How far from a rule a filled area's edge still belongs to that rule: cell shading
/// is commonly inset a few points from the rules around it.
const FILL_SNAP: f32 = 8.0;

/// The boundary positions of one axis of a grid, or `None` when that axis is not ruled.
///
/// Rules decide the boundaries. A filled area's edge adds one only where no rule runs —
/// the band of a shaded row, a frame painted as a filled ring. Near a rule it is the
/// same boundary drawn twice: shading inset from the rules around a cell, taken as
/// boundaries of its own, gave every column an empty neighbour on each side.
///
/// An axis needs at least two ruled boundaries. Filled areas complete a ruled grid, but
/// boundaries from filled areas alone are a figure — the bars of a bar chart between its
/// stroked gridlines.
///
/// And a filled edge is a boundary only where it reaches a rule of the other axis
/// (`across`): one unbroken stretch (pieces at most [`FILL_GAP`] apart) that meets or crosses
/// one. A shaded row does, cell by cell; a highlight drawn behind one line of a cell's text
/// stops short of the rules around the cell, and taking each such edge as a row boundary
/// split a three-line header cell into three rows.
fn boundaries(
    component: &[AxisSegment],
    axis: Axis,
    across: &[f32],
    config: &LatticeConfig,
) -> Option<Vec<f32>> {
    let on_axis = || component.iter().filter(move |s| s.axis == axis);
    let mut positions = cluster_positions(
        on_axis().filter(|s| s.ruled).map(|s| s.pos),
        config.cluster_tolerance,
    );
    if positions.len() < 2 {
        return None;
    }
    let filled = cluster_positions(
        on_axis().filter(|s| !s.ruled).map(|s| s.pos),
        config.cluster_tolerance,
    );
    let ruled = positions.clone();
    let runs_across = |f: f32| {
        let mut extents: Vec<(f32, f32)> = on_axis()
            .filter(|s| !s.ruled && (s.pos - f).abs() <= config.cluster_tolerance)
            .map(|s| (s.lo, s.hi))
            .collect();
        extents.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The unbroken stretches the pieces make.
        let mut stretches: Vec<(f32, f32)> = Vec::new();
        for (lo, hi) in extents {
            match stretches.last_mut() {
                Some(last) if lo - last.1 <= FILL_GAP => last.1 = last.1.max(hi),
                _ => stretches.push((lo, hi)),
            }
        }
        // Only rows need it: a column's filled edge is the side of a shaded row, broken
        // wherever the next row is not shaded, and a highlight's sides fall within
        // [`FILL_SNAP`] of the cell's rules already.
        axis == Axis::Vertical
            || stretches.iter().any(|&(lo, hi)| {
                across
                    .iter()
                    .any(|&r| r >= lo - FILL_TOUCH && r <= hi + FILL_TOUCH)
            })
    };
    positions.extend(
        filled
            .into_iter()
            .filter(|&f| ruled.iter().all(|r| (r - f).abs() > FILL_SNAP) && runs_across(f)),
    );
    Some(positions)
}

/// The widest break a filled edge may have and still be one stretch: a band drawn cell by
/// cell leaves a hairline between cells.
const FILL_GAP: f32 = 3.0;

/// How close a filled stretch must come to a rule to meet it. Highlights behind a cell's text
/// are inset several points from the cell's rules; a shaded cell meets them.
const FILL_TOUCH: f32 = 2.0;

/// How far past a grid's outermost boundary every rule across it must run for the grid to have
/// a row or column there that no rule closes (points) — room for a line of text.
const OPEN_SIDE_MIN: f32 = 12.0;

/// The outer boundaries of a grid drawn without a frame: where the rules of `across` — the
/// axis whose rules run across the `positions` — all run on past the outermost of those
/// positions, the grid's last row or column lies between that position and the rules' ends.
///
/// Rules set only between the cells of a table (booktabs-style rules under the header and
/// between rows, column rules between columns) leave its outer rows and columns open: the
/// first column has no rule on its left, the last row none below it. Each rule still spans the
/// whole table, so their ends bound it. Taken by the rules' own ends — the nearest of them, so
/// every rule reaches the boundary — a side is added only when every rule across it overhangs
/// by at least [`OPEN_SIDE_MIN`]: a framed table's rules stop at the frame.
fn open_sides(component: &[AxisSegment], across: Axis, positions: &[f32]) -> Vec<f32> {
    let rules: Vec<&AxisSegment> = component
        .iter()
        .filter(|s| s.axis == across && s.ruled)
        .collect();
    let (Some(first), Some(last)) = (
        positions.iter().copied().reduce(f32::min),
        positions.iter().copied().reduce(f32::max),
    ) else {
        return Vec::new();
    };
    if rules.is_empty() {
        return Vec::new();
    }
    let mut sides = Vec::new();
    if rules.iter().all(|s| s.lo <= first - OPEN_SIDE_MIN) {
        sides.push(rules.iter().map(|s| s.lo).fold(f32::MIN, f32::max));
    }
    if rules.iter().all(|s| s.hi >= last + OPEN_SIDE_MIN) {
        sides.push(rules.iter().map(|s| s.hi).fold(f32::MAX, f32::min));
    }
    sides
}

/// The positions of the ruled segments of `axis`.
fn ruled_positions(component: &[AxisSegment], axis: Axis, config: &LatticeConfig) -> Vec<f32> {
    cluster_positions(
        component
            .iter()
            .filter(|s| s.axis == axis && s.ruled)
            .map(|s| s.pos),
        config.cluster_tolerance,
    )
}

/// Group segments into clusters of lines that touch or cross (within
/// `tolerance`), following the same "edges that intersect form one table"
/// rule pdfplumber's lattice finder uses.
fn connected_components(segments: &[AxisSegment], tolerance: f32) -> Vec<Vec<AxisSegment>> {
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        let mut node = i;
        while parent[node] != root {
            let next = parent[node];
            parent[node] = root;
            node = next;
        }
        root
    }

    let mut parent: Vec<usize> = (0..segments.len()).collect();
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            if segments[i].touches(&segments[j], tolerance) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }

    let mut groups: std::collections::BTreeMap<usize, Vec<AxisSegment>> =
        std::collections::BTreeMap::new();
    for (i, segment) in segments.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(*segment);
    }
    groups.into_values().collect()
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
/// Baseline tolerance, as a fraction of the font size, for lines inside one cell —
/// the stream-mode detector's default.
const CELL_LINE_TOLERANCE: f32 = 0.4;

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

    let mut cell_spans: Vec<Vec<Vec<TextSpan>>> = vec![vec![Vec::new(); cols]; rows];
    let mut consumed = Vec::new();

    for i in order {
        let span = &spans[i];
        let (Some(r), Some(c)) = (
            bin_index(&grid.row_bounds, span.y),
            bin_index(&grid.col_bounds, span.x),
        ) else {
            continue;
        };
        if !span.text.trim().is_empty() {
            cell_spans[r][c].push(span.clone());
        }
        consumed.push(i);
    }

    // A cell reads line by line, a superscript with the line it marks (`0.31*`,
    // not `* 0.31`) — the same rows stream-mode detection builds.
    let cell_text: Vec<Vec<Vec<String>>> = cell_spans
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| {
                    group_into_rows(&cell, CELL_LINE_TOLERANCE)
                        .into_iter()
                        .map(|line| TextLine::from_spans(line.spans).text().trim().to_string())
                        .filter(|text| !text.is_empty())
                        .collect()
                })
                .collect()
        })
        .collect();

    let non_empty = cell_text.iter().flatten().filter(|c| !c.is_empty()).count();
    let occupancy = non_empty as f32 / (rows * cols) as f32;
    if occupancy < MIN_OCCUPANCY {
        return None;
    }
    // One column is a table only as a list of items: every cell one line. A column whose
    // cells hold paragraphs is text drawn in boxes, and reads as text.
    if cols == 1 && !cell_text.iter().all(|row| row[0].len() == 1) {
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

/// Most words a column label of a header row standing above a grid holds.
const HEADER_LABEL_MAX_WORDS: usize = 4;

/// How far above a grid's top rule a header row may stand, in multiples of its font size.
const HEADER_ABOVE_MAX_GAP: f32 = 2.0;

/// `grid` with a header row added above its top rule, when the line just above it is one.
///
/// A table ruled under its header but not above it — the rule under the header is its top
/// rule — leaves the header outside the grid. The line nearest above that rule is the header
/// when it stands within [`HEADER_ABOVE_MAX_GAP`] of its font size of the rule and is made of
/// short labels (at most [`HEADER_LABEL_MAX_WORDS`] words) over two columns or more, each
/// label inside one column and all of them inside the grid's width. A caption or a title
/// above a table runs across its columns, or is one piece of text.
pub(crate) fn with_header_above(grid: &LatticeGrid, spans: &[TextSpan]) -> LatticeGrid {
    let within_width = |s: &TextSpan| {
        s.x >= grid.left_x - CELL_BOUNDARY_TOLERANCE
            && s.x + s.width <= grid.right_x + CELL_BOUNDARY_TOLERANCE
    };
    let above: Vec<TextSpan> = spans
        .iter()
        .filter(|s| s.y > grid.top_y + CELL_BOUNDARY_TOLERANCE && !s.text.trim().is_empty())
        .filter(|s| s.x < grid.right_x && s.x + s.width > grid.left_x)
        .cloned()
        .collect();
    let Some(line) = group_into_rows(&above, CELL_LINE_TOLERANCE)
        .into_iter()
        .min_by(|a, b| a.y.total_cmp(&b.y))
    else {
        return grid.clone();
    };
    let size = line
        .spans
        .iter()
        .map(|s| s.font_size)
        .fold(0.0_f32, f32::max);
    let column_of = |s: &TextSpan| {
        let c = bin_index(&grid.col_bounds, s.x)?;
        (s.x + s.width <= grid.col_bounds[c + 1] + CELL_BOUNDARY_TOLERANCE).then_some(c)
    };
    let columns: Option<Vec<usize>> = line.spans.iter().map(column_of).collect();
    let is_header = line.y - grid.top_y <= size * HEADER_ABOVE_MAX_GAP
        && line.spans.iter().all(within_width)
        && columns.is_some_and(|mut cs| {
            let words_fit = (0..grid.column_count()).all(|c| {
                line.spans
                    .iter()
                    .filter(|s| column_of(s) == Some(c))
                    .map(|s| s.text.split_whitespace().count())
                    .sum::<usize>()
                    <= HEADER_LABEL_MAX_WORDS
            });
            cs.sort_unstable();
            cs.dedup();
            cs.len() >= 2 && words_fit
        });
    if !is_header {
        return grid.clone();
    }
    let top = line.y + size;
    let mut row_bounds = vec![top];
    row_bounds.extend(grid.row_bounds.iter().copied());
    LatticeGrid {
        top_y: top,
        row_bounds,
        ..grid.clone()
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// An axis-aligned ruling line: its position on the perpendicular axis and
/// its extent `[lo, hi]` along its own axis.
#[derive(Debug, Clone, Copy)]
struct AxisSegment {
    axis: Axis,
    pos: f32,
    lo: f32,
    hi: f32,
    /// See [`GraphicsLine::ruled`].
    ruled: bool,
}

impl AxisSegment {
    /// Whether two segments meet: a horizontal and a vertical line that cross
    /// or end on each other, or two collinear lines that overlap or abut
    /// (a border drawn in pieces).
    fn touches(&self, other: &AxisSegment, tolerance: f32) -> bool {
        let within = |v: f32, lo: f32, hi: f32| v >= lo - tolerance && v <= hi + tolerance;
        if self.axis == other.axis {
            (self.pos - other.pos).abs() <= tolerance
                && self.lo <= other.hi + tolerance
                && other.lo <= self.hi + tolerance
        } else {
            within(other.pos, self.lo, self.hi) && within(self.pos, other.lo, other.hi)
        }
    }
}

/// Reduce lines to axis-aligned segments, discarding diagonal and too-short lines.
fn classify_lines(lines: &[GraphicsLine], config: &LatticeConfig) -> Vec<AxisSegment> {
    let mut segments = Vec::new();

    for line in lines {
        let dx = (line.x1 - line.x0).abs();
        let dy = (line.y1 - line.y0).abs();

        if dy <= config.axis_tolerance && dx >= config.min_line_length {
            segments.push(AxisSegment {
                axis: Axis::Horizontal,
                pos: (line.y0 + line.y1) / 2.0,
                lo: line.x0.min(line.x1),
                hi: line.x0.max(line.x1),
                ruled: line.ruled,
            });
        } else if dx <= config.axis_tolerance && dy >= config.min_line_length {
            segments.push(AxisSegment {
                axis: Axis::Vertical,
                pos: (line.x0 + line.x1) / 2.0,
                lo: line.y0.min(line.y1),
                hi: line.y0.max(line.y1),
                ruled: line.ruled,
            });
        }
        // Diagonal or too-short lines are not ruling-line evidence — ignored.
    }

    segments
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
            ruled: true,
        }
    }

    fn v(x: f32, y0: f32, y1: f32) -> GraphicsLine {
        GraphicsLine {
            x0: x,
            y0,
            x1: x,
            y1,
            ruled: true,
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

    /// Bars of a chart between stroked gridlines: the gridlines are rows, the bars'
    /// outlines would be the columns — filled areas only, so no table.
    #[test]
    fn a_bar_chart_between_gridlines_is_not_a_grid() {
        let mut lines: Vec<GraphicsLine> = [100.0, 150.0, 200.0, 250.0]
            .iter()
            .map(|&y| h(y, 50.0, 400.0))
            .collect();
        for (x, top) in [
            (80.0, 230.0),
            (160.0, 190.0),
            (240.0, 245.0),
            (320.0, 160.0),
        ] {
            for edge in [
                v(x, 100.0, top),
                v(x + 30.0, 100.0, top),
                h(top, x, x + 30.0),
            ] {
                lines.push(GraphicsLine {
                    ruled: false,
                    ..edge
                });
            }
        }
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    /// Rows marked by shading bands, columns by stroked rules: still a table.
    #[test]
    fn shaded_rows_with_ruled_columns_are_a_grid() {
        let mut lines = vec![
            h(300.0, 50.0, 250.0),
            h(240.0, 50.0, 250.0),
            v(50.0, 240.0, 300.0),
            v(150.0, 240.0, 300.0),
            v(250.0, 240.0, 300.0),
        ];
        for edge in [h(280.0, 50.0, 250.0), h(260.0, 50.0, 250.0)] {
            lines.push(GraphicsLine {
                ruled: false,
                ..edge
            });
        }
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].row_count(), 3);
    }

    /// Cell shading inset a few points from the rules around it is the same boundary,
    /// not an empty column beside each real one.
    #[test]
    fn shading_inset_from_the_rules_adds_no_boundaries() {
        let mut lines = vec![
            h(300.0, 50.0, 250.0),
            h(270.0, 50.0, 250.0),
            h(240.0, 50.0, 250.0),
            v(50.0, 240.0, 300.0),
            v(150.0, 240.0, 300.0),
            v(250.0, 240.0, 300.0),
        ];
        // A shaded header cell inset 4pt from its rules on every side.
        for edge in [
            h(296.0, 54.0, 146.0),
            h(274.0, 54.0, 146.0),
            v(54.0, 274.0, 296.0),
            v(146.0, 274.0, 296.0),
        ] {
            lines.push(GraphicsLine {
                ruled: false,
                ..edge
            });
        }
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].row_count(), 2);
        assert_eq!(grids[0].column_count(), 2);
    }

    #[test]
    fn a_highlight_behind_a_line_of_cell_text_adds_no_row() {
        // A 2x2 grid; the top cells' three text lines each have a shaded band behind them,
        // inset from the cell's rules.
        let mut lines = vec![
            h(300.0, 50.0, 250.0),
            h(240.0, 50.0, 250.0),
            h(200.0, 50.0, 250.0),
            v(50.0, 200.0, 300.0),
            v(150.0, 200.0, 300.0),
            v(250.0, 200.0, 300.0),
        ];
        // ... and the same in the top-right cell, so the bands together span most of the width.
        for top in [296.0, 282.0, 268.0] {
            for edge in [
                h(top, 55.0, 145.0),
                h(top - 13.0, 55.0, 145.0),
                h(top, 155.0, 245.0),
                h(top - 13.0, 155.0, 245.0),
            ] {
                lines.push(GraphicsLine {
                    ruled: false,
                    ..edge
                });
            }
        }
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].row_count(), 2, "{:?}", grids[0].row_bounds);
    }

    #[test]
    fn a_shaded_band_across_the_table_still_completes_the_grid() {
        // Rules above and below; the band between them is drawn as a filled area only.
        let mut lines = vec![
            h(300.0, 50.0, 250.0),
            h(240.0, 50.0, 250.0),
            v(50.0, 240.0, 300.0),
            v(150.0, 240.0, 300.0),
            v(250.0, 240.0, 300.0),
        ];
        lines.push(GraphicsLine {
            ruled: false,
            ..h(270.0, 50.0, 250.0)
        });
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids[0].row_count(), 2, "{:?}", grids[0].row_bounds);
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

    /// A boxed list: one column ruled into `rows` rows, 20 pt each, from y = 300 down.
    fn one_column(rows: usize) -> Vec<GraphicsLine> {
        let bottom = 300.0 - 20.0 * rows as f32;
        let mut lines: Vec<GraphicsLine> = (0..=rows)
            .map(|r| h(300.0 - 20.0 * r as f32, 100.0, 400.0))
            .collect();
        lines.push(v(100.0, bottom, 300.0));
        lines.push(v(400.0, bottom, 300.0));
        lines
    }

    #[test]
    fn one_ruled_column_of_three_rows_is_a_grid_but_of_two_is_a_box() {
        let grids = infer_grids(&one_column(3), &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!((grids[0].row_count(), grids[0].column_count()), (3, 1));
        // A title bar over a body: a box, not a table.
        assert!(infer_grids(&one_column(2), &LatticeConfig::default()).is_empty());
        // A chart legend: three rows, but a narrow box.
        let mut legend = vec![
            h(300.0, 100.0, 160.0),
            h(280.0, 100.0, 160.0),
            h(260.0, 100.0, 160.0),
        ];
        legend.extend([
            h(240.0, 100.0, 160.0),
            v(100.0, 240.0, 300.0),
            v(160.0, 240.0, 300.0),
        ]);
        assert!(infer_grids(&legend, &LatticeConfig::default()).is_empty());
    }

    #[test]
    fn one_column_is_a_table_only_when_every_cell_is_one_line() {
        let grid = &infer_grids(&one_column(3), &LatticeConfig::default())[0];
        let items = vec![
            span("#1: Recycle", 110.0, 285.0),
            span("#2: Reuse", 110.0, 265.0),
            span("#3: Reduce", 110.0, 245.0),
        ];
        let (table, _) = build_table(grid, &items).expect("a boxed list of items is a table");
        assert_eq!(table.to_csv(), "#1: Recycle\r\n#2: Reuse\r\n#3: Reduce\r\n");

        // The same box with a paragraph in a cell is text drawn in boxes.
        let mut prose = items.clone();
        prose.push(span("and a second line of the same cell", 110.0, 274.0));
        assert!(build_table(grid, &prose).is_none());
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
                ruled: true,
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
        // default 3.0pt cluster_tolerance) must not collapse into a single row
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
        // Two lines 0.5pt apart (within cluster_tolerance=3.0) at each of 3 row
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

    /// The four edges of a rectangle, as `re … f`/`re … S` produces them.
    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<GraphicsLine> {
        vec![h(y0, x0, x1), h(y1, x0, x1), v(x0, y0, y1), v(x1, y0, y1)]
    }

    #[test]
    fn disjoint_line_clusters_are_not_merged_into_one_grid() {
        // A figure frame on the left and a separator rule far below it. The two
        // never touch, so they cannot be borders of the same table — merged, their
        // extents would span every line of text between them.
        let mut lines = rect(46.0, 467.0, 174.0, 633.0);
        lines.push(h(100.0, 179.0, 356.0));
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    #[test]
    fn a_figure_frame_drawn_as_nested_rectangles_is_not_a_grid() {
        // A thumbnail container as a browser prints it: a background box, a
        // caption box stacked under it, and the image border inset by ~2pt. The
        // near-coincident edges are one border drawn several times, not
        // 2pt-wide table columns.
        let mut lines = rect(44.0, 465.0, 177.0, 636.0);
        lines.extend(rect(44.0, 418.0, 177.0, 465.0));
        lines.extend(rect(46.0, 467.0, 175.0, 634.0));
        lines.extend(rect(46.0, 467.0, 174.0, 633.0));
        assert!(infer_grids(&lines, &LatticeConfig::default()).is_empty());
    }

    #[test]
    fn two_separate_tables_yield_two_grids_top_down() {
        let mut lines = clean_grid_lines();
        // A second 2x2 table well below the first one.
        lines.extend([
            h(100.0, 50.0, 250.0),
            h(80.0, 50.0, 250.0),
            h(60.0, 50.0, 250.0),
            v(50.0, 60.0, 100.0),
            v(150.0, 60.0, 100.0),
            v(250.0, 60.0, 100.0),
        ]);
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 2);
        assert_eq!(grids[0].row_bounds, vec![300.0, 280.0, 260.0, 240.0]);
        assert_eq!(grids[1].row_bounds, vec![100.0, 80.0, 60.0]);
    }

    #[test]
    fn a_table_border_drawn_in_pieces_is_one_grid() {
        // Each cell stroked separately: collinear pieces that abut end to end
        // still belong to the same ruling line.
        let lines = vec![
            h(300.0, 50.0, 150.0),
            h(300.0, 150.0, 250.0),
            h(270.0, 50.0, 150.0),
            h(270.0, 150.0, 250.0),
            h(240.0, 50.0, 150.0),
            h(240.0, 150.0, 250.0),
            v(50.0, 270.0, 300.0),
            v(50.0, 240.0, 270.0),
            v(150.0, 270.0, 300.0),
            v(150.0, 240.0, 270.0),
            v(250.0, 270.0, 300.0),
            v(250.0, 240.0, 270.0),
        ];
        let grids = infer_grids(&lines, &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].row_count(), 2);
        assert_eq!(grids[0].column_count(), 2);
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
            width_measured: false,
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
        // Two spans in the same cell join in x order, with a space where the page
        // leaves a word gap and none where the runs touch (a kerned run split by
        // the font decoder) — the same joining as a line of body text.
        let spans = vec![span("Hello", 60.0, 290.0), span("World", 94.0, 290.0)];
        let (table, _) = build_table(&grid, &spans).expect("should build a table");
        assert_eq!(table.rows[0].cells[0].plain_text(), "Hello World");

        let spans = vec![span("Hel", 60.0, 290.0), span("lo", 78.0, 290.0)];
        let (table, _) = build_table(&grid, &spans).expect("should build a table");
        assert_eq!(table.rows[0].cells[0].plain_text(), "Hello");
    }

    /// A superscript sits on a higher baseline than the value it marks; read by
    /// baseline alone it came first (`* 0.31`).
    #[test]
    fn build_table_reads_a_superscript_after_its_value() {
        let grid = two_by_two_grid();
        let star = TextSpan {
            font_size: 7.0,
            width: 4.0,
            width_measured: true,
            ..span("*", 84.0, 294.0)
        };
        let spans = vec![span("0.31", 60.0, 290.0), star, span("Next", 160.0, 290.0)];
        let (table, _) = build_table(&grid, &spans).expect("should build a table");
        assert_eq!(table.rows[0].cells[0].plain_text(), "0.31*");
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

    /// Rules only between the cells — under the header, between rows, between columns —
    /// and no frame: the outer columns and the last row are bounded by the rules' ends.
    fn open_border_lines() -> Vec<GraphicsLine> {
        vec![
            h(300.0, 20.0, 420.0),
            h(260.0, 20.0, 420.0),
            v(120.0, 210.0, 300.0),
            v(270.0, 210.0, 300.0),
        ]
    }

    #[test]
    fn a_table_ruled_only_between_its_cells_ends_where_its_rules_do() {
        let grids = infer_grids(&open_border_lines(), &LatticeConfig::default());
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].row_bounds, vec![300.0, 260.0, 210.0]);
        assert_eq!(grids[0].col_bounds, vec![20.0, 120.0, 270.0, 420.0]);
    }

    /// The labels over a table ruled under its header but not above it are its header row.
    #[test]
    fn labels_just_above_the_top_rule_are_the_header_row() {
        let grid = &infer_grids(&open_border_lines(), &LatticeConfig::default())[0];
        let spans = vec![
            span("Name", 130.0, 306.0),
            span("Value", 280.0, 306.0),
            span("Alpha", 30.0, 280.0),
            span("one", 130.0, 280.0),
            span("two", 280.0, 280.0),
        ];
        let grid = with_header_above(grid, &spans);
        assert_eq!(grid.row_count(), 3);
        let (table, consumed) = build_table(&grid, &spans).unwrap();
        assert_eq!(consumed.len(), 5);
        assert_eq!(table.rows[0].cells[1].plain_text(), "Name");
        assert_eq!(table.rows[0].cells[2].plain_text(), "Value");
    }

    /// A caption above a table runs across its columns: it stays out of the table.
    #[test]
    fn a_caption_above_the_top_rule_is_not_a_header_row() {
        let grid = &infer_grids(&open_border_lines(), &LatticeConfig::default())[0];
        let spans = vec![
            span("Table 1. Results of the trial", 30.0, 306.0),
            span("Alpha", 30.0, 280.0),
        ];
        assert_eq!(with_header_above(grid, &spans), *grid);
    }
}
